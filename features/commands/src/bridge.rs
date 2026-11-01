//! The commands on an Action List (feature `ui`): what a headless runner,
//! a tour or a model reaches them by — every command (a view's own
//! included) run by id, and the palette's state read back.
//!
//! A view's command holds its signals, so it cannot live on the Action
//! List itself (shared across threads). [`use_command_actions`] puts two
//! actions there instead: [`RUN_ACTION`] hands the id to the window's
//! thread, which runs it as a key would, and answers; [`STATE_ACTION`]
//! reads what the palette shows. Each change of the palette is a
//! `Changed { topic: "commands" }` on the list.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionList, ActionMeta, Effect};
use dioxus::prelude::*;
use facet::Facet;
use futures_util::StreamExt as _;

use crate::ui::{Overlay, try_use_commands};
use crate::{RUN_ACTION, STATE_ACTION};

/// What [`RUN_ACTION`] takes: a command's id, and its arguments as JSON
/// (for one from an Action List that takes any).
#[derive(Debug, Clone, Facet)]
pub struct RunCommand {
    pub id: String,
    #[facet(default)]
    pub args: Option<String>,
}

/// What [`RUN_ACTION`] answers: whether it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Ran {
    pub ran: bool,
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
    description: "Runs any command by its id — a view's own too — as its key would; `args` is JSON for one that takes any.",
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
    let registered = use_hook({
        let list = list.clone();
        let state = state.clone();
        move || {
            let tx = runner.tx();
            let ran = list.register_async(&RUN_META, Effect::Mutating, move |ask: RunCommand| {
                let (answer, heard) = futures_channel::oneshot::channel();
                let sent = tx.unbounded_send((ask, answer)).is_ok();
                async move {
                    if !sent {
                        return Err("the window is gone".to_owned());
                    }
                    let ran = heard.await.map_err(|_| "the window is gone".to_owned())?;
                    Ok(Ran { ran })
                }
            });
            let read = list.register(&STATE_META, Effect::ReadOnly, move |(): ()| {
                Ok(state.lock().unwrap_or_else(PoisonError::into_inner).clone())
            });
            ran.is_ok() && read.is_ok()
        }
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
        }
    });
}
