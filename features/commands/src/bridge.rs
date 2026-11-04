//! The commands on an Action List (feature `ui`): what a headless runner,
//! a tour or a model reaches them by — every command (a view's own
//! included) run by id, and the palette's state read back.
//!
//! A view's command holds its signals, so it cannot live on the Action
//! List itself (shared across threads). [`use_command_actions`] puts two
//! actions there instead: [`RUN_ACTION`] hands the id to the window's
//! thread, which runs it as a key would, and answers; [`STATE_ACTION`]
//! reads what the palette shows; [`LIST_ACTION`] lists every command with
//! its effect. Each change of the palette is a
//! `Changed { topic: "commands" }` on the list.
//!
//! A command carries its own [`Effect`]: `COMMANDS_RUN` is not a blanket
//! write. Run by a caller from outside (a model, a peer):
//!
//! - a **destructive** command (or Action List action) waits for a
//!   person's yes when the list asks for confirmations
//!   (`ActionList::require_confirmation`): `COMMANDS_RUN`'s per-call check
//!   (`ActionList::set_confirm`) reads the command's effect;
//! - an Action List action run through it is authorised as itself, so
//!   `COMMANDS_RUN` is no way round the list's authorizer.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use dioxus::prelude::*;
use facet::Facet;
use futures_util::StreamExt as _;

use crate::ui::{Overlay, try_use_commands};
use crate::{LIST_ACTION, RUN_ACTION, SHOW_ACTION, STATE_ACTION};

/// What [`RUN_ACTION`] takes: a command's id, and its arguments as JSON
/// (for one from an Action List that takes any).
#[derive(Debug, Clone, Facet)]
pub struct RunCommand {
    /// The command's id (`WORKSPACE_CLOSE_TAB`); `COMMANDS_LIST` lists
    /// them, each with its effect.
    pub id: String,
    /// Its arguments as JSON, for an Action List action that takes any.
    #[facet(default)]
    pub args: Option<String>,
}

/// What [`SHOW_ACTION`] takes: the overlay, and what its field says.
#[derive(Debug, Clone, Facet)]
pub struct ShowCommands {
    /// `palette` or `quick-open`; `none` closes the one open.
    pub overlay: String,
    #[facet(default)]
    pub query: Option<String>,
}

/// What [`RUN_ACTION`] answers: whether it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Ran {
    pub ran: bool,
}

/// One command, as [`LIST_ACTION`] lists it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct CommandInfo {
    pub id: String,
    pub label: String,
    pub group: String,
    /// `read-only`, `mutating` or `destructive`.
    pub effect: String,
    pub enabled: bool,
    #[facet(default)]
    pub disabled_reason: Option<String>,
}

/// What [`LIST_ACTION`] answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct CommandList {
    pub commands: Vec<CommandInfo>,
}

/// What [`STATE_ACTION`] answers: the overlay on show and the palette's
/// field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct CommandsState {
    /// `palette`, `quick-open`, or empty when neither is open.
    pub overlay: String,
    pub query: String,
}

static RUN_META: ActionMeta = ActionMeta {
    id: RUN_ACTION,
    trait_name: "",
    method_name: "",
    display_name: "Run a command",
    description: "Runs any command by its id — a view's own too — as its key would; `args` is JSON for one that takes any. Each command has its own effect (COMMANDS_LIST): a destructive one waits for a person's yes.",
    category: "Commands",
    group: "Commands",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static SHOW_META: ActionMeta = ActionMeta {
    id: SHOW_ACTION,
    trait_name: "",
    method_name: "",
    display_name: "Show the palette",
    description: "Opens the command palette (`overlay: palette`) or ⌘O (`quick-open`), its field set to `query` when given: `theme:` asks only ⌘O's theme opener. `none` closes whichever is open.",
    category: "Commands",
    group: "Commands",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static LIST_META: ActionMeta = ActionMeta {
    id: LIST_ACTION,
    trait_name: "",
    method_name: "",
    display_name: "Commands",
    description: "Every command there is now: its id, label, group, effect (read-only, mutating, destructive), and whether it can run (and why not).",
    category: "Commands",
    group: "Commands",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static STATE_META: ActionMeta = ActionMeta {
    id: STATE_ACTION,
    trait_name: "",
    method_name: "",
    display_name: "Command palette state",
    description: "Which overlay is open (palette, quick-open or none) and what the palette's field says.",
    category: "Commands",
    group: "Commands",
    shortcut: "",
    toggleable: false,
    undo: false,
};

type Ask = (RunCommand, futures_channel::oneshot::Sender<bool>);

/// Puts the commands on `list` while the calling component is mounted.
///
/// Under [`crate::use_commands_provider`]: [`RUN_ACTION`] and
/// [`STATE_ACTION`], and a `commands` change event whenever the palette
/// opens, closes or its field changes.
pub fn use_command_actions(list: Arc<ActionList>) {
    let commands = try_use_commands();
    let state = use_hook(|| Arc::new(Mutex::new(CommandsState::default())));
    // Every command and its effect, readable from any thread.
    let known = use_hook(|| Arc::new(Mutex::new(CommandList::default())));
    // The window's thread runs what the list was asked to.
    let runner = use_coroutine(move |mut asks: UnboundedReceiver<Ask>| async move {
        while let Some((ask, answer)) = asks.next().await {
            let ran = commands.is_some_and(|c| match &ask.args {
                Some(args) => c.run_with(&ask.id, args),
                None => c.run(&ask.id),
            });
            let _ = answer.send(ran);
        }
    });
    // The window's thread opens what the list was asked to show.
    let shower = use_coroutine(
        move |mut asks: UnboundedReceiver<ShowCommands>| async move {
            while let Some(ask) = asks.next().await {
                let Some(c) = commands else { continue };
                match (ask.overlay.as_str(), ask.query) {
                    ("none", _) => c.close(),
                    ("quick-open", Some(q)) => c.show_quick_open(&q),
                    ("quick-open", None) => c.show(Overlay::QuickOpen),
                    (_, q) => {
                        c.show(Overlay::Palette);
                        if let Some(q) = q {
                            let mut query = c.query;
                            query.set(q);
                        }
                    }
                }
            }
        },
    );
    let registered = use_hook({
        let list = list.clone();
        let state = state.clone();
        let known = known.clone();
        move || {
            let tx = runner.tx();
            let inner = list.clone();
            let ran = list.register_async_with(
                &RUN_META,
                Effect::Mutating,
                move |caller: &ActionCaller, ask: RunCommand| {
                    let allowed = authorised(&inner, caller, &ask.id);
                    let sent = allowed.is_ok();
                    let (answer, heard) = futures_channel::oneshot::channel();
                    let sent = sent && tx.unbounded_send((ask, answer)).is_ok();
                    async move {
                        allowed?;
                        if !sent {
                            return Err("the window is gone".to_owned());
                        }
                        let ran = heard.await.map_err(|_| "the window is gone".to_owned())?;
                        Ok(Ran { ran })
                    }
                },
            );
            // A destructive command waits for a person's yes, as a
            // destructive action does.
            let effects = known.clone();
            let actions = list.clone();
            list.set_confirm(RUN_ACTION, move |args| {
                effect_of(&effects, &actions, args) == Some(Effect::Destructive)
            });
            let listed = known.clone();
            let all = list.register(&LIST_META, Effect::ReadOnly, move |(): ()| {
                Ok(listed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone())
            });
            let read = list.register(&STATE_META, Effect::ReadOnly, move |(): ()| {
                Ok(state.lock().unwrap_or_else(PoisonError::into_inner).clone())
            });
            let show_tx = shower.tx();
            let show = list.register(&SHOW_META, Effect::Mutating, move |ask: ShowCommands| {
                if !matches!(ask.overlay.as_str(), "palette" | "quick-open" | "none") {
                    return Err(format!(
                        "overlay {:?}: it is `palette`, `quick-open` or `none`",
                        ask.overlay
                    ));
                }
                show_tx
                    .unbounded_send(ask)
                    .map_err(|_| "the window is gone".to_owned())?;
                Ok(Ran { ran: true })
            });
            ran.is_ok() && read.is_ok() && all.is_ok() && show.is_ok()
        }
    });
    // Every command as it is now (a view's come and go).
    use_effect(move || {
        let Some(c) = commands else {
            return;
        };
        let now = CommandList {
            commands: c
                .all()
                .into_iter()
                .map(|cmd| CommandInfo {
                    effect: cmd.effect.name().to_owned(),
                    id: cmd.id,
                    label: cmd.label,
                    group: cmd.group,
                    enabled: cmd.enabled,
                    disabled_reason: cmd.disabled_reason,
                })
                .collect(),
        };
        *known.lock().unwrap_or_else(PoisonError::into_inner) = now;
    });
    // The palette, as it is now.
    let told = list.clone();
    use_effect(move || {
        let Some(c) = commands else {
            return;
        };
        let now = CommandsState {
            overlay: match (c.overlay)() {
                Some(Overlay::Palette) => "palette".to_owned(),
                Some(Overlay::QuickOpen) => "quick-open".to_owned(),
                None => String::new(),
            },
            query: (c.query)(),
        };
        let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
        if *kept != now {
            let detail = now.overlay.clone();
            *kept = now;
            drop(kept);
            told.changed("commands", &detail);
        }
    });
    use_drop(move || {
        if registered {
            list.unregister(RUN_ACTION);
            list.unregister(STATE_ACTION);
            list.unregister(LIST_ACTION);
            list.unregister(SHOW_ACTION);
            // The check holds the list: let it go.
            list.set_confirm(RUN_ACTION, |_| false);
        }
    });
}

/// May `caller` run command `id` through `COMMANDS_RUN`? An Action List
/// action is asked of the list's authorizer as itself; a view's own
/// command, which the authorizer does not know, as `COMMANDS_RUN`.
fn authorised(list: &ActionList, caller: &ActionCaller, id: &str) -> Result<(), String> {
    let Some(who) = caller.principal() else {
        return Ok(());
    };
    if list.info(id).is_none() {
        return Ok(());
    }
    match list.authorize(who, id) {
        architect::action::Decision::Allow => Ok(()),
        architect::action::Decision::Deny { reason } => Err(format!("{id}: not allowed: {reason}")),
    }
}

/// The effect of the command a `COMMANDS_RUN` call names: the command's
/// own, else the Action List action's. `None` when the call names none.
fn effect_of(known: &Mutex<CommandList>, list: &ActionList, args: &[u8]) -> Option<Effect> {
    let ask: RunCommand = facet_json::from_slice(args).ok()?;
    let listed = known
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .commands
        .iter()
        .find(|c| c.id == ask.id)
        .map(|c| match c.effect.as_str() {
            "read-only" => Effect::ReadOnly,
            "destructive" => Effect::Destructive,
            _ => Effect::Mutating,
        });
    listed.or_else(|| list.info(&ask.id).map(|i| i.effect))
}
