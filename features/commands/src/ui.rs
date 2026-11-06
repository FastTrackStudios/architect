//! The commands' Dioxus half (feature `ui`): the ⌘K palette, ⌘O quick
//! open and the action list window, drawn with the shell's theme, over a
//! [`Commands`] context an app puts at its root.
//!
//! Commands come from two places: the app's Action List (every action
//! with an id, shared with keys, menus, scripts and models), and scoped
//! commands a view registers while it is mounted ([`use_command`]) — a
//! handler that holds the view's signals, which the Action List (shared
//! across threads) cannot.
//!
//! Keys run through `architect-input` ([`Commands::handle_chord`]): the
//! app's keymap, each command's declared shortcut, and the person's
//! layer over both ([`crate::use_keymap`]), in the live contexts
//! ([`Commands::set_context`]). A shortcut shown in the palette is the
//! binding that runs it, and changes when the person rebinds it.

// The `rsx!` loops iterate rows collected so a signal's read is let go
// before drawing. Items shared with `keys_ui` are `pub(crate)`, out of
// the crate's `pub use ui::*`.
#![allow(clippy::needless_collect, clippy::redundant_pub_crate)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;
use architect_input::dioxus::{InputHandle, use_input_processor};
use architect_input::{InputContext, InputProcessor, KeymapConfig, MacroKeys};

use dioxus::prelude::*;
use lucide_dioxus::Command as CommandIcon;

use architect_palette::fuzzy;
use crate::keymap::{Keymap, PersonKeymap};
use crate::keys_ui::{Asking, Recording};
use crate::kit::{fill, theme};
use crate::menu::Pick as MenuPick;
use architect_palette::{Engine, Found, OpenIn, Opener, Openers};
use architect_palette::{Command, Query, Ranked, Scope, rank};
use architect_palette::Recents;

/// A command a view registered, with what it runs.
struct Scoped {
    command: Command,
    run: Rc<dyn Fn()>,
    /// Which registration it is, to remove the right one on drop.
    serial: u64,
}

/// The overlay on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Palette,
    QuickOpen,
}

impl Overlay {
    /// Its name, as `COMMANDS_SHOW` and `[[palette_views]]` write it
    /// (`palette`, `quick-open`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::QuickOpen => "quick-open",
        }
    }

    /// The overlay named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "palette" => Some(Self::Palette),
            "quick-open" => Some(Self::QuickOpen),
            _ => None,
        }
    }
}

/// The view that draws an overlay in the shell's palette slot
/// ([`CommandLayer`]): an extension's (`[[palette_views]]`), or an app's
/// own.
#[derive(Clone)]
pub struct PaletteView {
    pub overlay: Overlay,
    /// The extension that fills it (`command-palette`).
    pub owner: String,
    /// Draws the overlay.
    pub draw: Rc<dyn Fn() -> Element>,
}

impl std::fmt::Debug for PaletteView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaletteView")
            .field("overlay", &self.overlay)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PaletteView {
    fn eq(&self, other: &Self) -> bool {
        self.overlay == other.overlay
            && self.owner == other.owner
            && Rc::ptr_eq(&self.draw, &other.draw)
    }
}

/// What picking a contributed source's row does.
type RunItem = Rc<dyn Fn(&architect_palette::Item)>;

/// The extensions' contributed palette sources (`[[palette_sources]]`),
/// searched by the engine, and what picking each one's row runs.
#[derive(Default)]
pub(crate) struct Contributed {
    pub(crate) engine: Engine,
    pub(crate) runs: std::collections::HashMap<String, RunItem>,
}

/// The commands context: [`use_commands_provider`] at the root.
#[derive(Clone, Copy)]
pub struct Commands {
    scoped: CopyValue<Rc<RefCell<Vec<Scoped>>>>,
    /// The app's Action Lists: every action in them is a command.
    lists: CopyValue<Vec<Arc<ActionList>>>,
    openers: CopyValue<Rc<RefCell<Openers>>>,
    pub recents: Signal<Recents>,
    pub overlay: Signal<Option<Overlay>>,
    /// Bumped when the scoped commands change, so the palette redraws.
    pub(crate) version: Signal<u64>,
    /// Whether the platform shows the menu bar ([`NativeMenus`](crate::NativeMenus)
    /// took it), so the in-window one stands aside.
    global_menu: Signal<bool>,
    serial: CopyValue<u64>,
    /// The app's keymap with the person's layer over it.
    pub(crate) keymap: Signal<Keymap>,
    /// Told when the person's layer changes, to keep and sync it.
    pub(crate) on_keymap: CopyValue<Option<Callback<PersonKeymap>>>,
    /// The keymap engine the keys run through.
    pub(crate) keys: CopyValue<InputHandle>,
    /// The config the engine was last built from.
    pub(crate) built: CopyValue<Option<KeymapConfig>>,
    /// Where keys are going: the live contexts.
    pub(crate) context: CopyValue<InputContext>,
    /// The held-letter key down now, for its tool's hint.
    pub(crate) held: Signal<Option<String>>,
    /// The palette's field.
    pub(crate) query: Signal<String>,
    /// An action's arguments being asked for in the palette.
    pub(crate) asking: Signal<Option<Asking>>,
    /// The one group the palette lists, and its field's prompt
    /// ([`Commands::show_group`]).
    pub(crate) only: Signal<Option<(String, String)>>,
    /// What ⌘O's field is set to next ([`Commands::show_quick_open`]).
    pub(crate) seed: Signal<Option<String>>,
    /// A binding being recorded in the action list window.
    pub(crate) recording: Signal<Option<Recording>>,
    /// The `@` (this page's targets) and `#` (settings) finders.
    pub(crate) targets: CopyValue<Rc<RefCell<Openers>>>,
    pub(crate) settings: CopyValue<Rc<RefCell<Openers>>>,
    /// The keymap in force, at the `version` it was computed for.
    pub(crate) cache: CopyValue<Option<(u64, Rc<crate::keys_ui::Effective>)>>,
    /// The views that draw the overlays (the palette extension's).
    pub(crate) views: Signal<Vec<PaletteView>>,
    /// Keys the extensions bind (`[[keys]]`): `(action, chord)`.
    pub(crate) bound: Signal<Vec<(String, String)>>,
    /// The extensions' contributed sources.
    pub(crate) sources: CopyValue<Rc<RefCell<Contributed>>>,
}

/// Puts a [`Commands`] in context for everything below, and the
/// [`CommandRunner`](architect_shell::CommandRunner) a phone's bar runs
/// its buttons through.
#[must_use = "the commands, to run one or open the palette"]
pub fn use_commands_provider() -> Commands {
    // Not vim: `q` and `@` are keys like any other.
    let keys = use_input_processor(|| InputProcessor::new().with_macro_keys(MacroKeys::Off));
    let commands = use_context_provider(|| Commands {
        scoped: CopyValue::new(Rc::new(RefCell::new(Vec::new()))),
        lists: CopyValue::new(Vec::new()),
        openers: CopyValue::new(Rc::new(RefCell::new(Openers::new()))),
        recents: Signal::new(Recents::default()),
        overlay: Signal::new(None),
        version: Signal::new(0),
        global_menu: Signal::new(false),
        serial: CopyValue::new(0),
        keymap: Signal::new(Keymap::default()),
        on_keymap: CopyValue::new(None),
        keys: CopyValue::new(keys),
        built: CopyValue::new(None),
        context: CopyValue::new(InputContext::new()),
        held: Signal::new(None),
        query: Signal::new(String::new()),
        asking: Signal::new(None),
        only: Signal::new(None),
        seed: Signal::new(None),
        recording: Signal::new(None),
        targets: CopyValue::new(Rc::new(RefCell::new(Openers::new()))),
        settings: CopyValue::new(Rc::new(RefCell::new(Openers::new()))),
        cache: CopyValue::new(None),
        views: Signal::new(Vec::new()),
        bound: Signal::new(Vec::new()),
        sources: CopyValue::new(Rc::new(RefCell::new(Contributed::default()))),
    });
    // A phone's bar buttons run what the palette and the keys run.
    use_context_provider(|| architect_shell::CommandRunner {
        run: Callback::new(move |id: String| commands.run(&id)),
        enabled: Callback::new(move |id: String| {
            commands
                .declared()
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.enabled)
        }),
        checked: Callback::new(move |id: String| {
            commands
                .declared()
                .into_iter()
                .find(|c| c.id == id)
                .and_then(|c| c.checked)
        }),
    });
    commands
}

/// The [`Commands`] above, if any.
#[must_use]
pub fn try_use_commands() -> Option<Commands> {
    try_use_context()
}

/// Registers `command` while the calling component is mounted.
///
/// It is listed in the palette, run by its shortcut, and gone when the
/// component is. Call it
/// every render: the command's label, shortcut and enabled state are kept
/// current; the handler is the first one given.
// By value: a command is built where it is registered (`Command::new(…)`).
#[allow(clippy::needless_pass_by_value)]
pub fn use_command(command: Command, run: impl Fn() + 'static) {
    let commands = try_use_commands();
    let first = command.clone();
    let serial = use_hook(move || {
        let command = first;
        let commands = commands?;
        let mut next = commands.serial;
        let serial = next.cloned().saturating_add(1);
        next.set(serial);
        commands.scoped.read().borrow_mut().push(Scoped {
            command,
            run: Rc::new(run),
            serial,
        });
        commands.bump();
        Some(serial)
    });
    if let (Some(c), Some(serial)) = (commands, serial) {
        let changed = {
            let scoped = c.scoped.read();
            let mut scoped = scoped.borrow_mut();
            scoped
                .iter_mut()
                .find(|s| s.serial == serial)
                .filter(|s| s.command != command)
                .map(|s| s.command = command.clone())
                .is_some()
        };
        if changed {
            c.bump();
        }
    }
    use_drop(move || {
        // The provider may go first (the whole window torn down, or a
        // shell slot drawn above it): then there is nothing to leave.
        if let (Some(c), Some(serial)) = (commands, serial)
            && let Ok(scoped) = c.scoped.try_read()
        {
            scoped.borrow_mut().retain(|s| s.serial != serial);
            drop(scoped);
            if c.version.try_peek().is_ok() {
                c.bump();
            }
        }
    });
}

/// Lists every action of `list` in the palette (and runs them by their
/// shortcuts) while the calling component is mounted — the app's Action
/// List, at its root.
pub fn use_action_list(list: Arc<ActionList>) {
    let commands = try_use_commands();
    let held = use_hook(move || {
        let commands = commands?;
        let mut lists = commands.lists;
        lists.write().push(list.clone());
        commands.bump();
        Some(list)
    });
    use_drop(move || {
        if let (Some(c), Some(list)) = (commands, held) {
            let mut lists = c.lists;
            lists.write().retain(|l| !Arc::ptr_eq(l, &list));
            c.bump();
        }
    });
}

/// A listed action as a palette row.
fn from_info(info: &architect::action::ActionInfo) -> Command {
    let group = if info.group.is_empty() {
        info.category.clone()
    } else {
        info.group.clone()
    };
    let mut c = Command::new(info.id.clone(), info.display_name.clone())
        .group(group)
        .detail(info.description.clone())
        .shortcut(info.shortcut.clone())
        .effect(match info.effect {
            architect::action::Effect::ReadOnly => crate::Effect::ReadOnly,
            architect::action::Effect::Mutating => crate::Effect::Mutating,
            architect::action::Effect::Destructive => crate::Effect::Destructive,
        });
    if let Some(on) = info.checked {
        c = c.checked(on);
    }
    if !info.enabled {
        c = c.disabled(
            info.disabled_reason
                .clone()
                .unwrap_or_else(|| "not now".to_owned()),
        );
    }
    c
}

/// Registers an opener for ⌘O while the calling component is mounted.
pub fn use_opener(make: impl FnOnce() -> Opener) {
    let commands = try_use_commands();
    let kind = use_hook(move || {
        let commands = commands?;
        let opener = make();
        let kind = opener.kind.clone();
        commands.openers.read().borrow_mut().register(opener);
        Some(kind)
    });
    use_drop(move || {
        if let (Some(c), Some(kind)) = (commands, kind) {
            c.openers.read().borrow_mut().remove(&kind);
        }
    });
}

impl Commands {
    pub(crate) fn bump(self) {
        let mut v = self.version;
        let next = v.peek().saturating_add(1);
        v.set(next);
    }

    /// Every command there is now, each shown with the binding that runs
    /// it (the person's, else the app's).
    #[must_use]
    pub fn all(&self) -> Vec<Command> {
        let effective = self.effective();
        self.declared()
            .into_iter()
            .map(|mut c| {
                c.shortcut = effective.shortcuts.get(&c.id).cloned().unwrap_or_default();
                c
            })
            .collect()
    }

    /// The keymap with `commands`' declared shortcuts in it.
    pub(crate) fn keymap_for(&self, commands: &[Command]) -> Keymap {
        let mut keymap = self.keymap.read().clone();
        // A command's own shortcut first, then the keys extensions bind
        // (the Command Palette's ⌘K and ⌘O).
        keymap.set_shortcuts(
            commands
                .iter()
                .filter(|c| !c.shortcut.is_empty())
                .map(|c| (c.id.clone(), c.shortcut.clone()))
                .chain(self.bound.read().iter().cloned()),
        );
        keymap
    }

    /// Every command, with the shortcut it declares, and the person's
    /// custom actions.
    pub(crate) fn declared(&self) -> Vec<Command> {
        let _ = (self.version)();
        let mut all: Vec<Command> = self
            .scoped
            .read()
            .borrow()
            .iter()
            .map(|s| s.command.clone())
            .collect();
        // A view's own command wins over a listed one of the same id.
        for list in self.lists.read().iter() {
            for info in list.infos() {
                if !all.iter().any(|c| c.id == info.id) {
                    all.push(from_info(&info));
                }
            }
        }
        for custom in self.keymap.read().custom() {
            if !all.iter().any(|c| c.id == custom.id) {
                // As much as its most far-reaching step: a destructive
                // step makes the whole of it destructive.
                let effect = custom
                    .steps
                    .iter()
                    .filter_map(|step| all.iter().find(|c| c.id == step.action))
                    .map(|c| c.effect)
                    .max_by_key(|e| match e {
                        crate::Effect::ReadOnly => 0,
                        crate::Effect::Mutating => 1,
                        crate::Effect::Destructive => 2,
                    })
                    .unwrap_or_default();
                all.push(crate::keys_ui::custom_command(custom).effect(effect));
            }
        }
        all
    }

    /// Runs command `id`. False when there is none, or it is disabled. An
    /// async action from a list (a saved script) is spawned, and counts as
    /// run once started.
    #[must_use = "false when there is no such command, or it cannot run"]
    pub fn run(&self, id: &str) -> bool {
        let found = {
            let scoped = self.scoped.read();
            let scoped = scoped.borrow();
            scoped
                .iter()
                .rev()
                .find(|s| s.command.id == id && s.command.enabled)
                .map(|s| s.run.clone())
        };
        let mut recents = self.recents;
        if let Some(run) = found {
            recents.write().touch(id);
            run();
            return true;
        }
        let list = self
            .lists
            .read()
            .iter()
            .find(|l| l.info(id).is_some_and(|i| i.enabled))
            .cloned();
        if let Some(list) = list {
            recents.write().touch(id);
            if !list.is_async(id) {
                return list.run(id).is_ok();
            }
            // An async action (a saved script, a service call) is awaited
            // off the key's turn; started is all a key can know.
            let id = id.to_owned();
            spawn(async move {
                let _ = list.run_json_async(&id, b"").await;
            });
            return true;
        }
        self.run_custom(id)
    }

    /// The escape hatch ([`crate::keymap::ESCAPE_HATCH`]): the person's
    /// key changes dropped, and [`crate::keymap::ESCAPE_HATCH_ACTION`] run
    /// on whichever list has it (every built-in workspace layout back as
    /// shipped). Its chord runs this before anything else; nothing can
    /// rebind it.
    pub fn escape_hatch(&self) {
        self.edit_keymap(Keymap::reset_all);
        let _ = self.run_with(crate::keymap::ESCAPE_HATCH_ACTION, "{}");
    }

    /// Runs action `id` with `args` (JSON). False when there is no such
    /// action in the app's lists, it is disabled, or it said no. An async
    /// action is spawned, and counts as run once started.
    #[must_use = "false when it did not run"]
    pub fn run_with(&self, id: &str, args: &str) -> bool {
        let list = self
            .lists
            .read()
            .iter()
            .find(|l| l.info(id).is_some_and(|i| i.enabled))
            .cloned();
        list.is_some_and(|list| {
            let mut recents = self.recents;
            recents.write().touch(id);
            if !list.is_async(id) {
                return list.run_json(id, args.as_bytes()).is_ok();
            }
            let id = id.to_owned();
            let args = args.as_bytes().to_vec();
            spawn(async move {
                let _ = list.run_json_async(&id, &args).await;
            });
            true
        })
    }

    /// Runs action `id` (with `args`, JSON, when given) as the app, saying
    /// where it was invoked from: a context menu's target, a status
    /// item, a contributed menu row; its work outlives the component
    /// that asked (a menu row unmounts as it runs). A command registered with
    /// [`use_command`] runs as [`Self::run`] does (it takes no
    /// invocation). False when there is no such action, it is disabled,
    /// or it said no; an async one is spawned and counts as run once
    /// started.
    #[must_use = "false when it did not run"]
    pub fn run_invoked(
        &self,
        id: &str,
        args: Option<&str>,
        invocation: architect::action::Invocation,
    ) -> bool {
        let list = self
            .lists
            .read()
            .iter()
            .find(|l| l.info(id).is_some_and(|i| i.enabled))
            .cloned();
        let Some(list) = list else {
            return self.run(id);
        };
        let mut recents = self.recents;
        recents.write().touch(id);
        let caller = architect::action::ActionCaller::app().with_invocation(invocation);
        let args = args.unwrap_or_default().as_bytes().to_vec();
        if !list.is_async(id) {
            return list.run_json_with(&caller, id, &args).is_ok();
        }
        let id = id.to_owned();
        // Past the caller's scope: a menu row is gone as it runs.
        dioxus::core::spawn_forever(async move {
            let _ = list.run_json_async_with(&caller, &id, &args).await;
        });
        true
    }

    /// Action `id`'s params schema, when an app list has it.
    pub(crate) fn params_schema(&self, id: &str) -> Option<String> {
        self.lists
            .read()
            .iter()
            .find_map(|l| l.info(id))
            .and_then(|i| i.params_schema)
    }

    /// Does what picking a menu row does: runs its command, with the row's
    /// arguments when it has them (a live section's row: those go to the
    /// Action List, which is where an action that takes any lives). False
    /// when there is no such command, it is disabled or it failed.
    #[must_use = "false when there is no such command, or it cannot run"]
    pub fn run_pick(&self, pick: &MenuPick) -> bool {
        let Some(args) = &pick.args else {
            return self.run(&pick.command);
        };
        let list = self
            .lists
            .read()
            .iter()
            .find(|l| l.info(&pick.command).is_some_and(|i| i.enabled))
            .cloned();
        list.is_some_and(|list| {
            let mut recents = self.recents;
            recents.write().touch(&pick.command);
            list.run_json(&pick.command, args.as_bytes()).is_ok()
        })
    }

    /// Where the Action Lists' events have got to: it moves on with each
    /// run, each [`ActionList::changed`] the app reports, each action
    /// registered. A menu bar asks again (`refresh`) when it moved,
    /// so enabled and ticked states follow the events, not a clock.
    #[must_use]
    pub fn heard(&self) -> u64 {
        self.lists
            .peek()
            .iter()
            .fold(0_u64, |sum, l| sum.wrapping_add(l.latest()))
    }

    /// Whether the platform shows the app's menu bar (the macOS one, or a
    /// Linux shell that took the exported one): the in-window bar
    /// ([`InWindowMenus`](crate::InWindowMenus)) draws nothing then.
    #[must_use]
    pub fn has_global_menu(&self) -> bool {
        (self.global_menu)()
    }

    /// Says whether the platform shows the menu bar: what a backend does
    /// once it knows.
    pub fn set_global_menu(&self, shown: bool) {
        let mut g = self.global_menu;
        if *g.peek() != shown {
            g.set(shown);
        }
    }

    /// Everything that reads the commands reads them again.
    pub fn refresh(&self) {
        self.bump();
    }

    pub fn show(&self, overlay: Overlay) {
        let mut o = self.overlay;
        if overlay == Overlay::Palette {
            let mut query = self.query;
            query.set(String::new());
            let mut asking = self.asking;
            asking.set(None);
            let mut only = self.only;
            only.set(None);
        }
        o.set(Some(overlay));
    }

    /// ⌘O asked `query`: `kind:` alone (`theme:`) lists only that
    /// opener's items, the field empty under its title for the person to
    /// narrow; anything else is put in the field.
    pub fn show_quick_open(&self, query: &str) {
        let mut seed = self.seed;
        seed.set(Some(query.to_owned()));
        self.show(Overlay::QuickOpen);
    }

    /// ⌘O's `opener`, from now on (in place of one of its kind); what
    /// [`use_opener`] does for a component's life.
    pub fn register_opener(&self, opener: Opener) {
        self.openers.read().borrow_mut().register(opener);
        self.bump();
    }

    /// ⌘O's opener of `kind`, gone.
    pub fn remove_opener(&self, kind: &str) {
        self.openers.read().borrow_mut().remove(kind);
        self.bump();
    }

    /// The palette on `group`'s commands only (those that can run now),
    /// its field empty with `prompt` in it: a command that asks where or which (⇧⌘M's "Move
    /// to…" lists the `Move` group), the person narrowing it by typing.
    pub fn show_group(&self, group: &str, prompt: &str) {
        self.show(Overlay::Palette);
        let mut only = self.only;
        only.set(Some((group.to_owned(), prompt.to_owned())));
    }

    pub fn close(&self) {
        let mut o = self.overlay;
        o.set(None);
        let mut asking = self.asking;
        asking.set(None);
        let mut only = self.only;
        only.set(None);
    }

    /// The views that draw the overlays from now on: what the
    /// extensions' `[[palette_views]]` say as they load and go (or an
    /// app's own). An overlay no view draws does not show.
    pub fn set_palette_views(&self, views: Vec<PaletteView>) {
        let same = {
            let now = self.views.peek();
            now.len() == views.len()
                && now
                    .iter()
                    .zip(&views)
                    .all(|(a, b)| a.overlay == b.overlay && a.owner == b.owner)
        };
        if !same {
            let mut v = self.views;
            v.set(views);
            // An overlay with no view left closes.
            let open = *self.overlay.peek();
            if open.is_some_and(|o| !self.has_palette(o)) {
                self.close();
            }
        }
    }

    /// Whether a view draws `overlay` (a palette is installed).
    #[must_use]
    pub fn has_palette(&self, overlay: Overlay) -> bool {
        self.views.peek().iter().any(|v| v.overlay == overlay)
    }

    /// The extension whose view draws `overlay`, if any.
    #[must_use]
    pub fn palette_owner(&self, overlay: Overlay) -> Option<String> {
        self.views
            .read()
            .iter()
            .find(|v| v.overlay == overlay)
            .map(|v| v.owner.clone())
    }

    /// The keys the extensions bind (`[[keys]]`, `(action, chord)`) from
    /// now on, in the keymap after the commands' own shortcuts.
    pub fn set_bound_keys(&self, keys: Vec<(String, String)>) {
        if *self.bound.peek() != keys {
            let mut b = self.bound;
            b.set(keys);
            self.bump();
        }
    }

    /// The keys the extensions bind, `(action, chord)`.
    #[must_use]
    pub fn bound_keys(&self) -> Vec<(String, String)> {
        self.bound.read().clone()
    }

    /// A source of the palette's rows (an extension's `[[palette_sources]]`)
    /// from now on, in place of one with its id: its rows are listed with
    /// the commands once something is typed (or its prefix, `tours:`, is),
    /// and picking one runs `run`.
    pub fn register_source(
        &self,
        source: impl architect_palette::Source + 'static,
        run: impl Fn(&architect_palette::Item) + 'static,
    ) {
        {
            let sources = self.sources.read();
            let mut sources = sources.borrow_mut();
            sources.runs.insert(source.id().to_owned(), Rc::new(run));
            sources.engine.register(source);
        }
        self.bump();
    }

    /// Source `id`, gone from the palette.
    pub fn remove_source(&self, id: &str) {
        {
            let sources = self.sources.read();
            let mut sources = sources.borrow_mut();
            sources.runs.remove(id);
            sources.engine.remove(id);
        }
        self.bump();
    }

    /// The contributed sources' ids.
    #[must_use]
    pub fn source_ids(&self) -> Vec<String> {
        self.sources
            .read()
            .borrow()
            .engine
            .ids()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// The palette's field.
    #[must_use]
    pub const fn palette_query(&self) -> Signal<String> {
        self.query
    }

    /// The one group the palette is on, and its field's prompt, while a
    /// command asks where or which ([`Self::show_group`]).
    #[must_use]
    pub fn choice(&self) -> Option<(String, String)> {
        (self.only)()
    }

    /// Whether the palette is asking an action's arguments
    /// ([`crate::kit::AskPalette`] draws that).
    #[must_use]
    pub fn is_asking(&self) -> bool {
        self.asking.read().is_some()
    }

    /// What ⌘O was asked to show ([`Self::show_quick_open`]), once.
    pub fn take_seed(&self) -> Option<String> {
        let mut seed = self.seed;
        let q = seed();
        if q.is_some() {
            seed.set(None);
        }
        q
    }

    /// The palette's rows for `q`, in order, with what picking each does
    /// ([`Self::pick`]): the commands ranked by the engine (only the
    /// group's while on one, in the app's own order when nothing is
    /// typed), the contributed sources' rows merged in once something is
    /// typed, or the `@`, `#` and `?` finders'.
    #[must_use]
    pub fn palette_rows(&self, q: &Query) -> Vec<(Ranked, crate::kit::Pick)> {
        use crate::kit::Pick;
        let only = (self.only)();
        let mut all = self.all();
        // A choice lists only what can be chosen.
        if let Some((group, _)) = &only {
            all.retain(|c| c.group == *group && c.enabled);
        }
        match q.scope {
            // On one group with nothing typed, the app's own order (a
            // list's, as its side shows it), not the alphabet.
            Scope::Commands if only.is_some() && q.text.is_empty() => all
                .into_iter()
                .map(|mut command| {
                    command.group.clear();
                    let pick = Pick::Run(command.id.clone());
                    let row = Ranked {
                        item: command,
                        positions: Vec::new(),
                        score: 0,
                    };
                    (row, pick)
                })
                .collect(),
            Scope::Commands => {
                let recents = self.recents.read();
                let sources = self.sources.read();
                let sources = sources.borrow();
                let (narrowed, _) = sources.engine.narrow(&q.text);
                let mut rows: Vec<(Ranked, Pick)> = if narrowed.is_some() {
                    Vec::new()
                } else {
                    rank(&all, &q.text, &recents)
                        .into_iter()
                        .map(|mut r| {
                            // On one group, its prompt names it: the rows
                            // need not.
                            if only.is_some() {
                                r.item.group.clear();
                            }
                            let pick = Pick::Run(r.item.id.clone());
                            (r, pick)
                        })
                        .collect()
                };
                // The extensions' sources, once something is typed: an
                // empty palette is the commands and the recent ones.
                if only.is_none() && !q.text.is_empty() {
                    let found = sources.engine.search(&q.text, &recents);
                    rows.extend(found.into_iter().map(|mut r| {
                        if r.item.group.is_empty() {
                            r.item.group = sources
                                .engine
                                .title(&r.item.source)
                                .unwrap_or_default()
                                .to_owned();
                        }
                        let pick = Pick::Item(r.item.clone());
                        (r, pick)
                    }));
                    if narrowed.is_none() {
                        rows.sort_by(|(a, _), (b, _)| {
                            b.item.enabled.cmp(&a.item.enabled).then(b.score.cmp(&a.score))
                        });
                    }
                }
                rows
            }
            _ => crate::keys_ui::scope_rows(self, q),
        }
    }

    /// What ⌘O finds for `query` (`kind:` narrows to one opener), best
    /// first.
    #[must_use]
    pub fn open_rows(&self, query: &str) -> Vec<Found> {
        let _ = (self.version)();
        self.openers.read().borrow().search(query)
    }

    /// ⌘O's opener `kind`'s title (`Themes`), when there is one.
    #[must_use]
    pub fn open_title(&self, kind: &str) -> Option<String> {
        self.openers.peek().borrow().title(kind).map(str::to_owned)
    }

    /// Opens what ⌘O found, `open_in` where: the overlay closes, and it is
    /// the most recent.
    pub fn open_found(&self, found: &Found, open_in: OpenIn) {
        self.close();
        let mut recents = self.recents;
        recents
            .write()
            .touch(&format!("{}:{}", found.item.kind, found.item.id));
        let _ = self.openers.read().borrow().open(&found.item, open_in);
    }

    /// A key at the app's root: Esc closes the overlay; any other chord
    /// runs the command whose shortcut it is (⌘K and ⌘O the palette
    /// extension's, bound with `[[keys]]`).
    /// True when it was used (the caller then stops it going further).
    #[must_use = "true when the key was used: stop it going further"]
    pub fn handle_key(&self, e: &KeyboardEvent) -> bool {
        chord(e).is_some_and(|chord| self.handle_chord(&chord))
    }

    /// A chord as commands show theirs (`⌘K`, `⌃⇧Tab`): what
    /// [`Self::handle_key`] does with a DOM key, for a key heard elsewhere
    /// (the window, [`WindowKeys`]).
    ///
    /// Any other chord goes to the keymap: a binding runs its command, a
    /// sequence's first key waits for the next ([`crate::WhichKeyPanel`]
    /// shows what can follow). While the action list window records a
    /// binding, the chord is recorded instead; while the palette asks `?`,
    /// it is the key asked about.
    #[must_use = "true when the chord was used"]
    pub fn handle_chord(&self, chord: &str) -> bool {
        // Before anything a person bound, a recording or an overlay: it
        // always works.
        if crate::keymap::is_escape_hatch(chord) {
            self.escape_hatch();
            return true;
        }
        if self.recording.peek().is_some() {
            return self.record_chord(chord);
        }
        if self.overlay.peek().is_some()
            && chord != "Esc"
            && self.query.peek().trim_start().starts_with('?')
        {
            let mut query = self.query;
            query.set(format!("?{chord}"));
            return true;
        }
        // ⌘K and ⌘O are the palette extension's keys (`[[keys]]`), in
        // the keymap with the rest; Esc closes whichever overlay is open.
        if chord == "Esc" && self.overlay.peek().is_some() {
            self.close();
            return true;
        }
        self.key_chord(chord)
    }
}

/// A key event as a shortcut's text (`⌘W`, `⌃⇧Tab`), the way commands
/// show theirs: modifiers in the Mac order, then the key.
pub(crate) fn chord(e: &KeyboardEvent) -> Option<String> {
    let m = e.modifiers();
    let key = match e.key() {
        Key::Character(c) if c == " " => "Space".to_owned(),
        Key::Character(c) => c.to_uppercase(),
        Key::Named(NamedKey::Tab) => "Tab".to_owned(),
        Key::Named(NamedKey::Enter) => "↵".to_owned(),
        Key::Named(NamedKey::Escape) => "Esc".to_owned(),
        Key::Named(NamedKey::Backspace) => "⌫".to_owned(),
        Key::Named(NamedKey::ArrowLeft) => "←".to_owned(),
        Key::Named(NamedKey::ArrowRight) => "→".to_owned(),
        Key::Named(NamedKey::ArrowUp) => "↑".to_owned(),
        Key::Named(NamedKey::ArrowDown) => "↓".to_owned(),
        Key::Named(NamedKey::F1) => "F1".to_owned(),
        Key::Named(_) => return None,
    };
    // A shifted character arrives as itself (`{` not `[`): the shift is in it.
    let shift = m.shift()
        && !matches!(e.key(), Key::Character(ref c) if !c.chars().all(char::is_alphabetic));
    let mut out = String::new();
    if m.ctrl() {
        out.push('⌃');
    }
    if m.alt() {
        out.push('⌥');
    }
    if shift {
        out.push('⇧');
    }
    if m.meta() {
        out.push('⌘');
    }
    out.push_str(&key);
    Some(out)
}

/// The action list window (REAPER's): every command, searchable, with its
/// group, shortcut and id; run one, see why one cannot run. A workspace
/// view or a settings section.
#[component]
pub fn ActionListView() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let mut filter = use_signal(String::new);
    let mut only_unbound = use_signal(|| false);
    let mut all = commands.all();
    all.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.label.cmp(&b.label)));
    let text = filter.read().clone();
    let rows: Vec<Command> = all
        .into_iter()
        .filter(|c| !only_unbound() || c.shortcut.is_empty())
        .filter(|c| {
            text.trim().is_empty()
                || fuzzy(&text, &c.label).is_some()
                || fuzzy(&text, &c.id).is_some()
                || fuzzy(&text, &c.group).is_some()
                || c.shortcut.to_lowercase().contains(&text.to_lowercase())
        })
        .collect();
    let count = rows.len();
    rsx! {
        div { style: "flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; font-family: {t.font};",
            div { style: "flex-shrink: 0; display: flex; align-items: center; gap: 10px; padding: 10px 14px; \
                          border-bottom: 1px solid {t.rule};",
                CommandIcon { size: 15, color: t.ink_3.clone() }
                input {
                    r#type: "text",
                    value: "{filter}",
                    "aria-label": "Filter actions",
                    style: "flex: 1; min-width: 0; height: 28px; padding: 0 8px; border-radius: 6px; border: 1px solid {t.rule}; \
                            background: {t.desk}; color: {t.ink}; font-size: 13px; outline: none;",
                    oninput: move |e| filter.set(e.value()),
                }
                button {
                    class: "lp",
                    style: "height: 28px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                            background: {fill(only_unbound(), &t)}; color: {t.ink_2}; font-size: 12px;",
                    onclick: move |_| only_unbound.toggle(),
                    "Unbound only"
                }
                span { style: "color: {t.ink_3}; font-size: 12px;", "{count} actions" }
                crate::KeymapTools {}
            }
            div { style: "flex-shrink: 0; display: flex; gap: 10px; padding: 6px 14px; color: {t.ink_3}; font-size: 11px; \
                          font-weight: 600; border-bottom: 1px solid {t.rule};",
                span { style: "flex: 3; min-width: 0;", "Action" }
                span { style: "flex: 2; min-width: 0;", "Group" }
                span { style: "flex: 2; min-width: 0;", "Bindings" }
                span { style: "flex: 3; min-width: 0;", "Id" }
                span { style: "width: 52px;", "" }
            }
            div { style: "flex: 1; min-height: 0; overflow-y: scroll;",
                for c in rows {
                    ActionRow { key: "{c.id}", command: c }
                }
            }
        }
    }
}

#[component]
fn ActionRow(command: Command) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let ink = if command.enabled { &*t.ink } else { &*t.ink_3 };
    let id = command.id.clone();
    rsx! {
        div { style: "display: flex; align-items: center; gap: 10px; min-height: 32px; padding: 0 14px; \
                      border-bottom: 1px solid {t.rule}; font-size: 13px;",
            span { style: "flex: 3; min-width: 0; display: flex; flex-direction: column; overflow: hidden; color: {ink};",
                span { style: "white-space: nowrap; overflow: hidden;", "{command.label}" }
                if let Some(why) = command.disabled_reason.clone() {
                    span { style: "color: {t.ink_3}; font-size: 11px;", "{why}" }
                }
            }
            span { style: "flex: 2; min-width: 0; color: {t.ink_2}; white-space: nowrap; overflow: hidden;", "{command.group}" }
            span { style: "flex: 2; min-width: 0;",
                crate::BindingEditor { action: command.id.clone() }
            }
            span { style: "flex: 3; min-width: 0; color: {t.ink_3}; font-family: ui-monospace, Menlo, monospace; \
                           font-size: 11px; white-space: nowrap; overflow: hidden;",
                "{command.id}"
            }
            button {
                class: "lp",
                disabled: !command.enabled,
                style: "width: 52px; height: 24px; border-radius: 6px; border: 1px solid {t.rule}; background: transparent; \
                        color: {ink}; font-size: 12px;",
                onclick: move |_| {
                    let _ = commands.run(&id);
                },
                "Run"
            }
        }
    }
}
