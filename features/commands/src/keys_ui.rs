//! Keys through the keymap (feature `ui`): the [`Commands`] side of
//! `architect-input` — chords run through the engine in the live
//! contexts, the which-key panel, the binding editor in the action list
//! window, custom actions, the palette's `@` / `#` / `?` scopes, and an
//! action's arguments asked a step at a time
//! (`docs/content/architecture/actions-everywhere.md` §2–§7).

// The `rsx!` loops iterate rows collected so a signal's read is let go
// before drawing. Items here are shared with `ui` but kept out of the
// crate's `pub use ui::*`, hence `pub(crate)`.
#![allow(clippy::needless_collect, clippy::redundant_pub_crate)]

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use architect_input::{
    InputCommand, InputContext, InputEvent, KeyEvent, KeymapConfig, parse_key_chord,
};
use dioxus::prelude::*;
use serde_json::Value;

use crate::custom::{Condition, CustomAction, Runner, Step};
use crate::keymap::{
    Binding, Clash, EVERYWHERE, Keymap, Layer, PersonKeymap, display_chord, display_keys,
    normalize_keys,
};
use crate::kit::{Frame, Kbd, fill, theme};
use crate::params::{Param, args_json, to_ask};
use crate::ui::{Commands, try_use_commands};
use architect_palette::fuzzy;
use architect_palette::{Command, Query, Ranked, Scope};
use architect_palette::{OpenIn, OpenItem, Opener};

/// What a palette row does when picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Run (or first ask the arguments of) an action.
    Run(String),
    /// Open what an `@` or `#` finder found.
    Open(Scope, OpenItem),
    /// Run what a contributed source's row does (its `[[palette_sources]]`
    /// `run`, given the row's id).
    Item(Command),
}

/// An action's arguments being asked in the palette, a step at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asking {
    pub id: String,
    pub label: String,
    pub params: Vec<Param>,
    /// What was answered so far, in order.
    pub answers: Vec<(String, Value)>,
    /// How far it got (answers skip optional fields left empty).
    pub step: usize,
    /// Why the last answer was not taken.
    pub error: Option<String>,
}

/// A binding being recorded for an action in the action list window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub action: String,
    /// The context it will bind in.
    pub context: String,
    /// The chord pressed, in keymap notation; `None` until one is.
    pub keys: Option<String>,
}

/// A custom action as a palette row.
pub(crate) fn custom_command(custom: &CustomAction) -> Command {
    let steps = custom
        .steps
        .iter()
        .map(|s| s.action.as_str())
        .collect::<Vec<_>>()
        .join(" › ");
    Command::new(custom.id.clone(), custom.name.clone())
        .group("Custom")
        .detail(steps)
}

/// How a context is shown.
fn context_label(context: &str) -> String {
    if context.is_empty() {
        "Everywhere".to_owned()
    } else {
        context.to_owned()
    }
}

thread_local! {
    /// How deep custom actions are running inside each other, so one that
    /// runs itself stops.
    static CUSTOM_DEPTH: Cell<u8> = const { Cell::new(0) };
}

/// How deep custom actions may nest.
const MAX_CUSTOM_DEPTH: u8 = 8;

/// A custom action's steps run against the commands.
struct Over(Commands);

impl Runner for Over {
    fn run(&mut self, action: &str, args: &str) -> Result<(), String> {
        let ran = if args.trim().is_empty() {
            self.0.run(action)
        } else {
            self.0.run_with(action, args)
        };
        if ran {
            Ok(())
        } else {
            Err(format!("{action} did not run"))
        }
    }

    // A toggle that is on: its list says so (`ActionList::set_checked`,
    // an extension's `toggle = true`).
    fn checked(&self, action: &str) -> bool {
        self.0
            .all()
            .iter()
            .any(|c| c.id == action && c.checked == Some(true))
    }

    fn enabled(&self, action: &str) -> bool {
        self.0.all().iter().any(|c| c.id == action && c.enabled)
    }
}

/// The keymap in force with everything it binds, computed once per change.
pub(crate) struct Effective {
    pub keymap: Keymap,
    pub bindings: Vec<Binding>,
    /// The shortcut each bound action shows.
    pub shortcuts: HashMap<String, String>,
}

impl Commands {
    /// The keymap in force: the app's, each command's declared shortcut,
    /// the person's layer (redrawn when any changes). Computed once per
    /// change, not per row.
    pub(crate) fn effective(&self) -> Rc<Effective> {
        let version = (self.version)();
        if let Some((at, effective)) = self.cache.peek().as_ref()
            && *at == version
        {
            return Rc::clone(effective);
        }
        let declared = self.declared();
        let keymap = self.keymap_for(&declared);
        let bindings = keymap.bindings();
        let shortcuts = keymap.shortcuts();
        let effective = Rc::new(Effective {
            keymap,
            bindings,
            shortcuts,
        });
        let mut cache = self.cache;
        cache.set(Some((version, Rc::clone(&effective))));
        effective
    }

    /// Runs custom action `id`. False when there is none, or a step failed.
    pub(crate) fn run_custom(&self, id: &str) -> bool {
        let custom = self
            .keymap
            .peek()
            .custom()
            .iter()
            .find(|c| c.id == id)
            .cloned();
        let Some(custom) = custom else {
            return false;
        };
        let depth = CUSTOM_DEPTH.get();
        if depth >= MAX_CUSTOM_DEPTH {
            return false;
        }
        CUSTOM_DEPTH.set(depth.saturating_add(1));
        let mut recents = self.recents;
        recents.write().touch(id);
        let ran = custom.run(&mut Over(*self)).is_ok();
        CUSTOM_DEPTH.set(depth);
        ran
    }

    /// The live contexts are `path` and every dotted prefix of it
    /// (`session.arrange.item`): the focused view, then the target under
    /// the pointer. Its bindings win over the app's, the innermost first.
    pub fn set_context(&self, path: &str) {
        let mut context = self.context;
        context.write().set_context(path);
    }

    /// The live contexts are exactly `chain`, outermost first.
    pub fn set_contexts<I, S>(&self, chain: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut context = self.context;
        context.write().set_contexts(chain);
    }

    /// The keymap in force, with each command's declared shortcut in it,
    /// for a view that shows it.
    #[must_use]
    pub fn keymap(&self) -> Keymap {
        self.keymap_for(&self.declared())
    }

    /// The person's layer changed elsewhere (another device): keys and
    /// shown shortcuts follow it. Nothing is told back.
    pub fn set_person_keymap(&self, person: PersonKeymap) {
        let mut keymap = self.keymap;
        keymap.write().set_person(person);
        self.bump();
    }

    /// Changes the person's layer and tells the app (to keep and sync it).
    pub fn edit_keymap(&self, edit: impl FnOnce(&mut Keymap)) {
        // Edited with the declared shortcuts in it, so removing one is
        // removing an app binding (an `unbind` the layer keeps).
        let current = self.keymap_for(&self.declared());
        let mut keymap = self.keymap;
        keymap.set(current);
        edit(&mut keymap.write());
        self.bump();
        let person = keymap.peek().person().clone();
        if let Some(on_change) = *self.on_keymap.peek() {
            on_change.call(person);
        }
    }

    /// Rebuilds the engine when the keymap in force changed.
    fn sync_keys(&self) {
        let declared = self.declared();
        let config: KeymapConfig = self.keymap_for(&declared).config();
        if self.built.peek().as_ref() == Some(&config) {
            return;
        }
        let keys = self.keys.peek().clone();
        if keys.reload_config(config.clone()).is_ok() {
            let mut built = self.built;
            built.set(Some(config));
        }
    }

    /// One chord (`⌘W`) through the keymap. True when it was used.
    pub(crate) fn key_chord(&self, chord: &str) -> bool {
        let Some(chord) = normalize_keys(chord)
            .ok()
            .and_then(|keys| parse_key_chord(&keys).ok())
        else {
            return false;
        };
        let (used, _) = self.feed(InputEvent::Key(KeyEvent {
            key: chord.key,
            modifiers: chord.modifiers,
        }));
        used
    }

    /// An input from the app (a pointer gesture over a target, a wheel, a
    /// key the page heard, a MIDI or OSC message) through the keymap in
    /// the live contexts. Actions it binds run; what is left — a gesture
    /// with its data for the app's action, `Unhandled` — comes back.
    #[must_use = "what the app handles itself: a gesture's data, an unbound input"]
    pub fn handle_input(&self, event: InputEvent) -> Vec<InputCommand> {
        self.feed(event).1
    }

    /// A key went up (`⌘W`, `Z`): the end of a held-letter tool, or its
    /// tap when nothing came while it was down. What is left comes back
    /// as from [`Self::handle_input`].
    #[must_use = "what the app handles itself"]
    pub fn handle_key_up(&self, chord: &str) -> Vec<InputCommand> {
        let Some(chord) = normalize_keys(chord)
            .ok()
            .and_then(|keys| parse_key_chord(&keys).ok())
        else {
            return Vec::new();
        };
        let ctx = self.context.peek().clone();
        let keys = self.keys.peek().clone();
        let released = keys.release(&chord, &ctx);
        self.dispatch(released.commands).1
    }

    fn feed(&self, event: InputEvent) -> (bool, Vec<InputCommand>) {
        self.sync_keys();
        let ctx: InputContext = self.context.peek().clone();
        let keys = self.keys.peek().clone();
        let commands = keys.process_event(event, &ctx);
        self.dispatch(commands)
    }

    /// Runs what the engine asked for; whether anything was used, and
    /// what is the app's to handle.
    fn dispatch(&self, commands: Vec<InputCommand>) -> (bool, Vec<InputCommand>) {
        let mut used = false;
        let mut rest = Vec::new();
        let mut held = self.held;
        for command in commands {
            match command {
                // A key bound to a command that cannot run now (disabled,
                // gone) is not used: it goes on to the page.
                InputCommand::Action(id) | InputCommand::ActionWithArgs { action: id, .. } => {
                    used |= self.run(&id);
                }
                InputCommand::Hold(chord) => {
                    held.set(Some(display_chord(&chord)));
                    used = true;
                }
                InputCommand::Unhold(_) => {
                    held.set(None);
                    used = true;
                }
                InputCommand::Pending { .. }
                | InputCommand::SwitchMode(_)
                | InputCommand::PushMode(_)
                | InputCommand::PopMode => used = true,
                other @ (InputCommand::Gesture { .. }
                | InputCommand::InsertText(_)
                | InputCommand::Unhandled(_)) => rest.push(other),
            }
        }
        (used, rest)
    }

    /// A palette row was picked: its command runs (or first asks its
    /// arguments), what a finder found opens, a contributed row runs its
    /// source's action.
    pub fn pick(&self, pick: &Pick) {
        match pick {
            Pick::Run(id) => {
                let ask = to_ask(self.params_schema(id).as_deref());
                if let Some(params) = ask {
                    let label = self
                        .all()
                        .into_iter()
                        .find(|c| c.id == *id)
                        .map_or_else(|| id.clone(), |c| c.label);
                    let mut asking = self.asking;
                    asking.set(Some(Asking {
                        id: id.clone(),
                        label,
                        params,
                        answers: Vec::new(),
                        step: 0,
                        error: None,
                    }));
                    let mut query = self.query;
                    query.set(String::new());
                } else {
                    self.close();
                    let _ = self.run(id);
                }
            }
            Pick::Open(scope, item) => {
                self.close();
                let finder = match scope {
                    Scope::Settings => self.settings,
                    _ => self.targets,
                };
                let _ = finder.peek().borrow().open(item, OpenIn::Here);
            }
            Pick::Item(item) => {
                self.close();
                let mut recents = self.recents;
                recents
                    .write()
                    .touch(&format!("{}:{}", item.source, item.id));
                let run = self.sources.peek().borrow().runs.get(&item.source).cloned();
                if let Some(run) = run {
                    run(item);
                }
            }
        }
    }

    /// A chord while a binding is being recorded: Esc gives up, any other
    /// is the binding's key.
    pub(crate) fn record_chord(&self, chord: &str) -> bool {
        let mut recording = self.recording;
        if chord == "Esc" {
            recording.set(None);
            return true;
        }
        let Ok(keys) = normalize_keys(chord) else {
            return false;
        };
        if let Some(r) = recording.write().as_mut() {
            r.keys = Some(keys);
        }
        true
    }
}

/// Gives the [`Commands`] above the app's keymap and the person's layer.
///
/// `on_change` keeps the layer (in the person's synced settings, so their
/// keys follow them). Call it once, at the root. When the layer changes
/// on another device, hand it to [`Commands::set_person_keymap`].
pub fn use_keymap(
    app: impl FnOnce() -> KeymapConfig,
    person: impl FnOnce() -> PersonKeymap,
    on_change: Option<Callback<PersonKeymap>>,
) {
    let commands = try_use_commands();
    use_hook(move || {
        if let Some(c) = commands {
            let mut keymap = c.keymap;
            keymap.set(Keymap::new(app()).with_person(person()));
            let mut cb = c.on_keymap;
            cb.set(on_change);
            c.bump();
        }
    });
}

/// Registers a finder for the palette's `@` (this page's targets: a
/// track, a section) or `#` (settings) while the calling component is
/// mounted. Any other scope registers nothing.
pub fn use_palette_opener(scope: Scope, make: impl FnOnce() -> Opener) {
    let commands = try_use_commands();
    let kind = use_hook(move || {
        let c = commands?;
        let finder = match scope {
            Scope::Targets => c.targets,
            Scope::Settings => c.settings,
            Scope::Commands | Scope::Help => return None,
        };
        let opener = make();
        let kind = opener.kind.clone();
        finder.read().borrow_mut().register(opener);
        Some((finder, kind))
    });
    use_drop(move || {
        if let Some((finder, kind)) = &kind {
            finder.read().borrow_mut().remove(kind);
        }
    });
}

/// The palette's rows for `@`, `#` and `?`.
pub(crate) fn scope_rows(commands: &Commands, q: &Query) -> Vec<(Ranked, Pick)> {
    match q.scope {
        Scope::Targets | Scope::Settings => {
            let finder = if q.scope == Scope::Targets {
                commands.targets
            } else {
                commands.settings
            };
            let finder = finder.read();
            let finder = finder.borrow();
            finder
                .search(&q.text)
                .into_iter()
                .map(|f| {
                    let group = finder.title(&f.item.kind).unwrap_or_default().to_owned();
                    let row = Ranked {
                        item: Command::new(
                            format!("{}:{}", f.item.kind, f.item.id),
                            f.item.title.clone(),
                        )
                        .group(group)
                        .detail(f.item.detail.clone()),
                        positions: f.positions,
                        score: f.score,
                    };
                    (row, Pick::Open(q.scope, f.item))
                })
                .collect()
        }
        Scope::Help => help_rows(commands, &q.text),
        Scope::Commands => Vec::new(),
    }
}

/// `?⌘W`: what the key does, in each context that binds it.
fn help_rows(commands: &Commands, text: &str) -> Vec<(Ranked, Pick)> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let all = commands.all();
    let effective = commands.effective();
    effective
        .keymap
        .what_does(text)
        .into_iter()
        .map(|b| {
            let found = all.iter().find(|c| c.id == b.action);
            let label = found.map_or_else(|| b.action.clone(), |c| c.label.clone());
            let mut command = Command::new(b.action.clone(), label)
                .group(context_label(&b.context))
                .shortcut(display_keys(&b.keys));
            if let Some(c) = found
                && !c.enabled
            {
                command = command.disabled(c.disabled_reason.clone().unwrap_or_default());
            }
            let row = Ranked {
                item: command,
                positions: Vec::new(),
                score: 0,
            };
            (row, Pick::Run(b.action))
        })
        .collect()
}

/// While a sequence waits for its next key, what can follow; while a
/// held-letter key is down, the gestures it enables. Drawn by
/// [`crate::CommandLayer`].
#[component]
pub fn WhichKeyPanel() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let keys = commands.keys.peek().clone();
    let pending = keys.pending_display();
    let held = (commands.held)();
    if pending.is_none() && held.is_none() {
        return rsx! {};
    }
    let t = theme();
    let ctx = commands.context.peek().clone();
    let labels: HashMap<String, String> = commands
        .all()
        .into_iter()
        .map(|c| (c.id, c.label))
        .collect();
    let name = |id: &String| labels.get(id).cloned().unwrap_or_else(|| id.clone());
    let (title, rows): (String, Vec<(String, String, bool)>) = {
        let processor = keys.processor();
        if let Some(menu) = processor.which_key(&ctx, &HashMap::new()) {
            let typed: Vec<String> = menu.typed.iter().map(display_chord).collect();
            let rows = menu
                .entries
                .iter()
                .map(|e| {
                    let label = e
                        .label
                        .clone()
                        .or_else(|| e.action().map(&name))
                        .unwrap_or_default();
                    (display_chord(&e.chord), label, e.is_group())
                })
                .collect();
            (typed.join(" "), rows)
        } else if let Some(tool) = processor.held_tool(&ctx) {
            let rows = tool
                .gestures
                .iter()
                .map(|(pattern, action)| (pattern.clone(), name(action), false))
                .collect();
            (format!("Holding {}", display_chord(&tool.key)), rows)
        } else {
            return rsx! {};
        }
    };
    rsx! {
        div {
            style: "position: absolute; left: 50%; bottom: 36px; transform: translateX(-50%); z-index: 95; \
                    min-width: 320px; max-width: 80%; padding: 10px 12px; border-radius: 10px; \
                    background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; \
                    font-family: {t.font}; font-size: 12px; color: {t.ink};",
            div { style: "color: {t.ink_3}; margin-bottom: 6px; font-weight: 600;", "{title}" }
            div { style: "display: flex; flex-wrap: wrap; gap: 4px 18px;",
                for (i, (key, label, group)) in rows.into_iter().enumerate() {
                    span { key: "{i}", style: "display: flex; align-items: center; gap: 6px;",
                        Kbd { text: key }
                        span { style: if group { format!("color: {};", t.accent) } else { format!("color: {};", t.ink_2) },
                            if group { "+{label}" } else { "{label}" }
                        }
                    }
                }
            }
        }
    }
}

/// The palette asking an action's arguments, one field at a time: a
/// choice from a list, anything else typed. ↵ takes the answer; the last
/// one runs the action.
#[component]
pub fn AskPalette() -> Element {
    let mut selected = use_signal(|| 0usize);
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let Some(asking) = commands.asking.read().clone() else {
        return rsx! {};
    };
    let Some(param) = asking.params.get(asking.step).cloned() else {
        return rsx! {};
    };
    let t = theme();
    let query = commands.query;
    let text = query.read().clone();
    let options: Vec<String> = param
        .options()
        .into_iter()
        .filter(|o| fuzzy(&text, o).is_some())
        .collect();
    let count = options.len();
    let pick = selected().min(count.saturating_sub(1));
    let answer = {
        let options = options.clone();
        move |typed: Option<String>| {
            let given = typed.or_else(|| options.get(pick).cloned());
            let given = given.unwrap_or_else(|| query.peek().clone());
            answer_step(&commands, &given);
            let mut selected = selected;
            selected.set(0);
        }
    };
    let answer_key = answer.clone();
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::Named(NamedKey::ArrowDown) => {
            e.prevent_default();
            selected.set(pick.saturating_add(1).min(count.saturating_sub(1)));
        }
        Key::Named(NamedKey::ArrowUp) => {
            e.prevent_default();
            selected.set(pick.saturating_sub(1));
        }
        Key::Named(NamedKey::Enter) => answer_key(None),
        Key::Named(NamedKey::Escape) => commands.close(),
        _ => {}
    };
    let done: Vec<String> = asking
        .answers
        .iter()
        .map(|(_, v)| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
        .collect();
    let trail = std::iter::once(asking.label.clone())
        .chain(done)
        .collect::<Vec<_>>()
        .join(" ▸ ");
    let prompt = param.prompt();
    let optional = if param.required { "" } else { " (optional)" };
    rsx! {
        Frame { placeholder: "Answer…", query, on_key,
            div { style: "padding: 6px 10px 10px; color: {t.ink_2}; font-size: 13px;",
                span { style: "color: {t.ink};", "{trail} ▸ " }
                "{prompt}{optional}"
            }
            if let Some(why) = asking.error {
                div { style: "padding: 0 10px 8px; color: {t.danger}; font-size: 12px;", "{why}" }
            }
            for (i, option) in options.into_iter().enumerate() {
                div {
                    key: "{option}",
                    class: "lp",
                    style: "height: 34px; display: flex; align-items: center; padding: 0 10px; border-radius: 8px; \
                            background: {fill(i == pick, &t)}; font-size: 13px; cursor: pointer; color: {t.ink};",
                    onclick: {
                        let answer = answer.clone();
                        move |_| answer(Some(option.clone()))
                    },
                    "{option}"
                }
            }
        }
    }
}

/// Takes `text` as the answer to the field being asked; after the last
/// one, runs the action with every answer.
fn answer_step(commands: &Commands, text: &str) {
    let mut asking_signal = commands.asking;
    let Some(mut asking) = asking_signal.peek().clone() else {
        return;
    };
    let Some(param) = asking.params.get(asking.step) else {
        return;
    };
    match param.answer(text) {
        Err(why) => asking.error = Some(why),
        Ok(value) => {
            if let Some(value) = value {
                asking.answers.push((param.name.clone(), value));
            }
            asking.step = asking.step.saturating_add(1);
            asking.error = None;
            let mut query = commands.query;
            query.set(String::new());
        }
    }
    if asking.step < asking.params.len() {
        asking_signal.set(Some(asking));
        return;
    }
    commands.close();
    let _ = commands.run_with(&asking.id, &args_json(&asking.answers));
}

/// An action's bindings in the action list window: each with its context
/// and a ×, + to record another (press the chord; what it would collide
/// with is shown before it is bound), ↺ to put the app's back.
#[component]
pub fn BindingEditor(action: String) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let effective = commands.effective();
    let bindings: Vec<Binding> = effective
        .bindings
        .iter()
        .filter(|b| b.action == action)
        .cloned()
        .collect();
    let changed = effective.keymap.is_changed(&action);
    let recording = commands
        .recording
        .read()
        .clone()
        .filter(|r| r.action == action);
    let mut recording_signal = commands.recording;
    let start = {
        let action = action.clone();
        move |_| {
            recording_signal.set(Some(Recording {
                action: action.clone(),
                context: EVERYWHERE.to_owned(),
                keys: None,
            }));
        }
    };
    let reset = move |_| commands.edit_keymap(|k| k.reset(&action));
    rsx! {
        span { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 4px;",
            for b in bindings {
                BindingChip { key: "{b.context}/{b.keys}", binding: b }
            }
            if let Some(r) = recording {
                Recorder { recording: r }
            } else {
                button {
                    class: "lp",
                    title: "Add a binding",
                    style: "height: 20px; padding: 0 6px; border-radius: 4px; border: 1px dashed {t.rule}; \
                            background: transparent; color: {t.ink_3}; font-size: 11px;",
                    onclick: start,
                    "+"
                }
            }
            if changed {
                button {
                    class: "lp",
                    title: "Put the app's bindings back",
                    style: "height: 20px; padding: 0 6px; border-radius: 4px; border: none; \
                            background: transparent; color: {t.accent}; font-size: 12px;",
                    onclick: reset,
                    "↺"
                }
            }
        }
    }
}

#[component]
fn BindingChip(binding: Binding) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let shown = display_keys(&binding.keys);
    let mine = binding.layer == Layer::Person;
    let border = if mine { &*t.accent } else { &*t.rule };
    let context = binding.context.clone();
    let keys = binding.keys.clone();
    rsx! {
        span { style: "display: inline-flex; align-items: center; gap: 4px; height: 20px; padding: 0 2px 0 6px; \
                       border-radius: 4px; border: 1px solid {border}; color: {t.ink_2}; font-size: 11px;",
            "{shown}"
            if !binding.context.is_empty() {
                span { style: "color: {t.ink_3};", "· {binding.context}" }
            }
            button {
                class: "lp",
                title: "Remove this binding",
                style: "border: none; background: transparent; color: {t.ink_3}; font-size: 11px; padding: 0 3px;",
                onclick: move |_| {
                    commands.edit_keymap(|k| {
                        let _ = k.unbind(&context, &keys);
                    });
                },
                "×"
            }
        }
    }
}

/// Recording a binding: press the chord (the window hears it, whatever
/// has the focus), pick the context, see what it collides with, bind.
#[component]
fn Recorder(recording: Recording) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let effective = commands.effective();
    let mut signal = commands.recording;
    // Where it can bind: everywhere, and every context the keymap names.
    let mut contexts: Vec<String> = std::iter::once(EVERYWHERE.to_owned())
        .chain(effective.bindings.iter().map(|b| b.context.clone()))
        .chain(effective.keymap.app().contexts.keys().cloned())
        .collect();
    contexts.sort();
    contexts.dedup();
    let next_context = contexts
        .iter()
        .position(|c| *c == recording.context)
        .and_then(|i| contexts.get(i.saturating_add(1)))
        .or_else(|| contexts.first())
        .cloned()
        .unwrap_or_default();
    let labels: HashMap<String, String> = commands
        .all()
        .into_iter()
        .map(|c| (c.id, c.label))
        .collect();
    let conflicts: Vec<String> = recording
        .keys
        .as_deref()
        .map(|keys| {
            effective
                .keymap
                .conflicts(&recording.action, &recording.context, keys)
        })
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let what = labels
                .get(&c.binding.action)
                .cloned()
                .unwrap_or_else(|| c.binding.action.clone());
            let shown = display_keys(&c.binding.keys);
            let place = context_label(&c.binding.context);
            match c.clash {
                Clash::Same => format!("{shown} is {what} in {place}: it would replace it"),
                Clash::Prefix => format!("{shown} is {what} in {place}: one starts the other"),
                Clash::Outer => format!("{shown} is also {what} in {place}: this wins here"),
                Clash::Inner => format!("{shown} is also {what} in {place}: that wins there"),
            }
        })
        .collect();
    let bind = {
        let r = recording.clone();
        move |_| {
            if let Some(keys) = r.keys.clone() {
                commands.edit_keymap(|k| {
                    let _ = k.bind(&r.action, &r.context, &keys);
                });
            }
            signal.set(None);
        }
    };
    let shown = recording
        .keys
        .as_deref()
        .map_or_else(|| "Press a key…".to_owned(), display_keys);
    let place = context_label(&recording.context);
    rsx! {
        span {
            tabindex: "0",
            style: "display: inline-flex; flex-direction: column; gap: 3px; padding: 4px 6px; border-radius: 6px; \
                    border: 1px solid {t.accent}; outline: none; font-size: 11px; color: {t.ink};",
            // A page without the window's keys (the web) records here.
            onkeydown: move |e: KeyboardEvent| {
                if let Some(chord) = crate::ui::chord(&e) {
                    e.prevent_default();
                    e.stop_propagation();
                    let _ = commands.record_chord(&chord);
                }
            },
            onmounted: move |e| {
                spawn(async move {
                    let _ = e.data().set_focus(true).await;
                });
            },
            span { style: "display: flex; align-items: center; gap: 6px;",
                Kbd { text: shown }
                button {
                    class: "lp",
                    title: "Where it holds",
                    style: "height: 18px; padding: 0 6px; border-radius: 4px; border: 1px solid {t.rule}; \
                            background: transparent; color: {t.ink_2}; font-size: 11px;",
                    onclick: move |_| {
                        if let Some(r) = signal.write().as_mut() {
                            r.context.clone_from(&next_context);
                        }
                    },
                    "{place} ▾"
                }
                button {
                    class: "lp",
                    disabled: recording.keys.is_none(),
                    style: "height: 18px; padding: 0 8px; border-radius: 4px; border: none; \
                            background: {t.accent}; color: {t.accent_ink}; font-size: 11px;",
                    onclick: bind,
                    "Bind"
                }
                button {
                    class: "lp",
                    style: "height: 18px; padding: 0 6px; border-radius: 4px; border: none; \
                            background: transparent; color: {t.ink_3}; font-size: 11px;",
                    onclick: move |_| signal.set(None),
                    "Cancel"
                }
            }
            for (i, line) in conflicts.into_iter().enumerate() {
                span { key: "{i}", style: "color: {t.danger};", "{line}" }
            }
        }
    }
}

/// The action list window's keymap tools: put every binding back, and
/// make custom actions (a list of actions run in order, a step run only
/// when its test holds, as many times as it says).
#[component]
pub fn KeymapTools() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let mut editing = use_signal(|| None::<CustomAction>);
    let any_changed = !commands.keymap.read().person().rebinds.is_empty();
    let custom: Vec<CustomAction> = commands.keymap.read().custom().to_vec();
    rsx! {
        if any_changed {
            button {
                class: "lp",
                style: "height: 28px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                        background: transparent; color: {t.ink_2}; font-size: 12px;",
                onclick: move |_| commands.edit_keymap(Keymap::reset_all),
                "Reset all"
            }
        }
        button {
            class: "lp",
            style: "height: 28px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                    background: transparent; color: {t.ink_2}; font-size: 12px;",
            onclick: move |_| editing.set(Some(CustomAction::new("", ""))),
            "New custom action"
        }
        for c in custom {
            button {
                key: "{c.id}",
                class: "lp",
                title: "Edit this custom action",
                style: "height: 28px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                        background: transparent; color: {t.ink_3}; font-size: 12px;",
                onclick: move |_| editing.set(Some(c.clone())),
                "{c.name}"
            }
        }
        if let Some(action) = editing() {
            CustomEditor { action, editing }
        }
    }
}

/// `CUSTOM_` and the name in capitals, words joined by `_`.
fn custom_id(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_uppercase)
        .collect();
    format!("CUSTOM_{}", words.join("_"))
}

/// What a step's test reads as.
const fn condition_label(condition: Option<&Condition>) -> &'static str {
    match condition {
        None => "always",
        Some(Condition::Enabled(_)) => "if it can run",
        Some(Condition::Not(_)) => "if it cannot run",
        Some(Condition::Checked(_)) => "if checked",
    }
}

/// The next test in the editor's cycle: always, if it can run, if it
/// cannot.
fn next_condition(step: &Step) -> Option<Condition> {
    match &step.when {
        None => Some(Condition::Enabled(step.action.clone())),
        Some(Condition::Enabled(a)) => {
            Some(Condition::Not(Box::new(Condition::Enabled(a.clone()))))
        }
        Some(_) => None,
    }
}

/// One custom action being made or changed.
#[component]
fn CustomEditor(action: CustomAction, editing: Signal<Option<CustomAction>>) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let mut editing = editing;
    let mut find = use_signal(String::new);
    let all = commands.all();
    let labels: HashMap<String, String> = all
        .iter()
        .map(|c| (c.id.clone(), c.label.clone()))
        .collect();
    let found: Vec<(String, String)> = if find.read().trim().is_empty() {
        Vec::new()
    } else {
        all.iter()
            .filter(|c| c.id != action.id && fuzzy(&find.read(), &c.label).is_some())
            .take(6)
            .map(|c| (c.id.clone(), c.label.clone()))
            .collect()
    };
    let is_new = action.id.is_empty();
    let edit = move |f: &dyn Fn(&mut CustomAction)| {
        let mut editing = editing;
        if let Some(a) = editing.write().as_mut() {
            f(a);
        }
    };
    let save = {
        let action = action.clone();
        move |_| {
            let mut saved = action.clone();
            if saved.id.is_empty() {
                saved.id = custom_id(&saved.name);
            }
            if !saved.name.trim().is_empty() && !saved.steps.is_empty() {
                commands.edit_keymap(|k| k.save_custom(saved.clone()));
                editing.set(None);
            }
        }
    };
    let remove = {
        let id = action.id.clone();
        move |_| {
            commands.edit_keymap(|k| k.remove_custom(&id));
            editing.set(None);
        }
    };
    let steps: Vec<(usize, Step, String, &'static str)> = action
        .steps
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, step)| {
            let name = labels
                .get(&step.action)
                .cloned()
                .unwrap_or_else(|| step.action.clone());
            let line = format!("{}. {name}", i.saturating_add(1));
            let when = condition_label(step.when.as_ref());
            (i, step, line, when)
        })
        .collect();
    rsx! {
        div {
            style: "position: absolute; top: 56px; right: 14px; z-index: 40; width: 420px; padding: 12px; \
                    display: flex; flex-direction: column; gap: 8px; border-radius: 10px; background: {t.pill}; \
                    border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; font-size: 12px; color: {t.ink};",
            input {
                r#type: "text",
                value: "{action.name}",
                "aria-label": "Custom action name",
                style: "height: 28px; padding: 0 8px; border-radius: 6px; border: 1px solid {t.rule}; \
                        background: {t.desk}; color: {t.ink}; font-size: 13px; outline: none;",
                oninput: move |e| edit(&|a| a.name = e.value()),
            }
            if action.name.is_empty() {
                span { style: "color: {t.ink_3}; margin-top: -4px;", "Name it (it becomes an action you can bind)" }
            }
            for (i, step, line, when) in steps {
                div { key: "{i}", style: "display: flex; align-items: center; gap: 6px;",
                    span { style: "flex: 1; min-width: 0; overflow: hidden; white-space: nowrap;", "{line}" }
                    button {
                        class: "lp",
                        title: "When it runs",
                        style: "height: 20px; padding: 0 6px; border-radius: 4px; border: 1px solid {t.rule}; \
                                background: transparent; color: {t.ink_2}; font-size: 11px;",
                        onclick: move |_| {
                            let next = next_condition(&step);
                            edit(&|a| {
                                if let Some(s) = a.steps.get_mut(i) {
                                    s.when.clone_from(&next);
                                }
                            });
                        },
                        "{when}"
                    }
                    button {
                        class: "lp",
                        title: "Fewer times",
                        style: "border: none; background: transparent; color: {t.ink_3};",
                        onclick: move |_| edit(&|a| {
                            if let Some(s) = a.steps.get_mut(i) {
                                s.repeat = s.repeat.saturating_sub(1).max(1);
                            }
                        }),
                        "−"
                    }
                    span { "×{step.repeat}" }
                    button {
                        class: "lp",
                        title: "More times",
                        style: "border: none; background: transparent; color: {t.ink_3};",
                        onclick: move |_| edit(&|a| {
                            if let Some(s) = a.steps.get_mut(i) {
                                s.repeat = s.repeat.saturating_add(1);
                            }
                        }),
                        "+"
                    }
                    button {
                        class: "lp",
                        title: "Remove the step",
                        style: "border: none; background: transparent; color: {t.ink_3};",
                        onclick: move |_| edit(&|a| {
                            if i < a.steps.len() {
                                a.steps.remove(i);
                            }
                        }),
                        "✕"
                    }
                }
            }
            input {
                r#type: "text",
                value: "{find}",
                "aria-label": "Add a step",
                style: "height: 26px; padding: 0 8px; border-radius: 6px; border: 1px solid {t.rule}; \
                        background: {t.desk}; color: {t.ink}; font-size: 12px; outline: none;",
                oninput: move |e| find.set(e.value()),
            }
            if find.read().is_empty() {
                span { style: "color: {t.ink_3}; margin-top: -4px;", "Add a step: type an action" }
            }
            for (id, label) in found {
                div {
                    key: "{id}",
                    class: "lp",
                    style: "padding: 4px 8px; border-radius: 6px; cursor: pointer; color: {t.ink_2};",
                    onclick: move |_| {
                        edit(&|a| a.steps.push(Step::new(id.clone())));
                        find.set(String::new());
                    },
                    "{label}"
                }
            }
            div { style: "display: flex; gap: 8px; justify-content: flex-end;",
                if !is_new {
                    button {
                        class: "lp",
                        style: "height: 26px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                                background: transparent; color: {t.danger}; font-size: 12px;",
                        onclick: remove,
                        "Delete"
                    }
                }
                button {
                    class: "lp",
                    style: "height: 26px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; \
                            background: transparent; color: {t.ink_2}; font-size: 12px;",
                    onclick: move |_| editing.set(None),
                    "Cancel"
                }
                button {
                    class: "lp",
                    style: "height: 26px; padding: 0 10px; border-radius: 6px; border: none; \
                            background: {t.accent}; color: {t.accent_ink}; font-size: 12px;",
                    onclick: save,
                    "Save"
                }
            }
        }
    }
}
