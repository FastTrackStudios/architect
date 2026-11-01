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
//! Keys: until the keymap engine drives them, a key runs the command whose
//! shortcut it is ([`Commands::handle_key`]), so a shortcut shown in the
//! palette is also the one that works.

// The `rsx!` loops iterate rows collected so a signal's read is let go
// before drawing.
#![allow(clippy::needless_collect)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;

use architect_shell::{ShellTheme, try_use_shell};
use dioxus::prelude::*;
use lucide_dioxus::{Command as CommandIcon, Search};

use crate::fuzzy::fuzzy;
use crate::opener::{Found, OpenIn, Opener, Openers};
use crate::palette::{Command, Query, Ranked, Scope, rank};
use crate::recent::Recents;

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
    version: Signal<u64>,
    serial: CopyValue<u64>,
}

/// Puts a [`Commands`] in context for everything below.
#[must_use = "the commands, to run one or open the palette"]
pub fn use_commands_provider() -> Commands {
    use_context_provider(|| Commands {
        scoped: CopyValue::new(Rc::new(RefCell::new(Vec::new()))),
        lists: CopyValue::new(Vec::new()),
        openers: CopyValue::new(Rc::new(RefCell::new(Openers::new()))),
        recents: Signal::new(Recents::default()),
        overlay: Signal::new(None),
        version: Signal::new(0),
        serial: CopyValue::new(0),
    })
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
        if let (Some(c), Some(serial)) = (commands, serial) {
            c.scoped.read().borrow_mut().retain(|s| s.serial != serial);
            c.bump();
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
        .shortcut(info.shortcut.clone());
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
    fn bump(self) {
        let mut v = self.version;
        let next = v.peek().saturating_add(1);
        v.set(next);
    }

    /// Every command there is now.
    #[must_use]
    pub fn all(&self) -> Vec<Command> {
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
        all
    }

    /// Runs command `id`. False when there is none, or it is disabled.
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
        list.is_some_and(|list| {
            recents.write().touch(id);
            list.run(id).is_ok()
        })
    }

    pub fn show(&self, overlay: Overlay) {
        let mut o = self.overlay;
        o.set(Some(overlay));
    }

    pub fn close(&self) {
        let mut o = self.overlay;
        o.set(None);
    }

    /// A key at the app's root: ⌘K (or ⌘⇧P) the palette, ⌘O quick open,
    /// Esc closes; any other chord runs the command whose shortcut it is.
    /// True when it was used (the caller then stops it going further).
    #[must_use = "true when the key was used: stop it going further"]
    pub fn handle_key(&self, e: &KeyboardEvent) -> bool {
        chord(e).is_some_and(|chord| self.handle_chord(&chord))
    }

    /// A chord as commands show theirs (`⌘K`, `⌃⇧Tab`): what
    /// [`Self::handle_key`] does with a DOM key, for a key heard elsewhere
    /// (the window, [`WindowKeys`]).
    #[must_use = "true when the chord was used"]
    pub fn handle_chord(&self, chord: &str) -> bool {
        match chord {
            "⌘K" | "⌃K" | "⇧⌘P" | "⌃⇧P" => {
                self.show(Overlay::Palette);
                return true;
            }
            "⌘O" | "⌃O" => {
                self.show(Overlay::QuickOpen);
                return true;
            }
            "Esc" if self.overlay.peek().is_some() => {
                self.close();
                return true;
            }
            _ => {}
        }
        let id = self
            .all()
            .into_iter()
            .find(|c| c.enabled && c.shortcut == *chord)
            .map(|c| c.id);
        id.is_some_and(|id| self.run(&id))
    }
}

/// A key event as a shortcut's text (`⌘W`, `⌃⇧Tab`), the way commands
/// show theirs: modifiers in the Mac order, then the key.
fn chord(e: &KeyboardEvent) -> Option<String> {
    let m = e.modifiers();
    let key = match e.key() {
        Key::Character(c) if c == " " => "Space".to_owned(),
        Key::Character(c) => c.to_uppercase(),
        Key::Tab => "Tab".to_owned(),
        Key::Enter => "↵".to_owned(),
        Key::Escape => "Esc".to_owned(),
        Key::Backspace => "⌫".to_owned(),
        Key::ArrowLeft => "←".to_owned(),
        Key::ArrowRight => "→".to_owned(),
        Key::ArrowUp => "↑".to_owned(),
        Key::ArrowDown => "↓".to_owned(),
        Key::F1 => "F1".to_owned(),
        _ => return None,
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

/// A row's fill: lit when picked.
const fn fill(on: bool, t: &ShellTheme) -> &'static str {
    if on { t.active } else { "transparent" }
}

fn theme() -> ShellTheme {
    try_use_shell().map(|s| s.theme).unwrap_or_default()
}

/// The overlays, mounted once inside the app (over everything, absolute
/// against the app's root).
#[component]
pub fn CommandLayer() -> Element {
    let commands = try_use_commands();
    let Some(commands) = commands else {
        return rsx! {};
    };
    match (commands.overlay)() {
        Some(Overlay::Palette) => rsx! { CommandPalette {} },
        Some(Overlay::QuickOpen) => rsx! { QuickOpen {} },
        None => rsx! {},
    }
}

/// The overlay's frame: a scrim, and a panel near the top with a field.
#[component]
fn Frame(
    placeholder: &'static str,
    query: Signal<String>,
    on_key: EventHandler<KeyboardEvent>,
    children: Element,
) -> Element {
    let commands = try_use_commands();
    let t = theme();
    let mut query = query;
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: 90; background: rgba(0,0,0,0.45); \
                    display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh;",
            onclick: move |_| {
                if let Some(c) = commands {
                    c.close();
                }
            },
            div {
                style: "width: 620px; max-width: 92%; max-height: 70vh; display: flex; flex-direction: column; \
                        border-radius: 12px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: 0 24px 64px rgba(0,0,0,0.6); overflow: hidden; font-family: {t.font};",
                onclick: move |e| e.stop_propagation(),
                div { style: "flex-shrink: 0; height: 48px; display: flex; align-items: center; gap: 10px; padding: 0 14px; \
                              border-bottom: 1px solid {t.rule};",
                    Search { size: 16, color: t.ink_3 }
                    input {
                        r#type: "text",
                        value: "{query}",
                        style: "flex: 1; min-width: 0; height: 32px; border: none; outline: none; background: transparent; \
                                color: {t.ink}; font-family: {t.font}; font-size: 15px;",
                        "aria-label": placeholder,
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| on_key.call(e),
                        onmounted: move |e| {
                            spawn(async move {
                                let _ = e.data().set_focus(true).await;
                            });
                        },
                    }
                    if query.read().is_empty() {
                        span { style: "position: absolute; margin-left: 26px; color: {t.ink_3}; font-size: 15px; pointer-events: none;",
                            "{placeholder}"
                        }
                    }
                    Kbd { text: "esc" }
                }
                div { style: "flex: 1; min-height: 0; overflow-y: scroll; padding: 6px;",
                    {children}
                }
            }
        }
    }
}

#[component]
fn Kbd(text: String) -> Element {
    let t = theme();
    rsx! {
        span { style: "flex-shrink: 0; padding: 1px 6px; border-radius: 4px; border: 1px solid {t.rule}; \
                       color: {t.ink_3}; font-size: 11px; font-family: {t.font};",
            "{text}"
        }
    }
}

/// A space's width, in ems.
const WORD_SPACE: f32 = 0.27;

/// `n` as a count of word spaces.
fn count(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// `text` with the characters at `positions` lit.
#[component]
fn Lit(text: String, positions: Vec<usize>, ink: &'static str, lit: &'static str) -> Element {
    // Runs of lit and unlit characters, so a row is a few spans, not one
    // per character.
    let mut runs: Vec<(bool, String)> = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let on = positions.contains(&i);
        match runs.last_mut() {
            Some((was, s)) if *was == on => s.push(c),
            _ => runs.push((on, c.to_string())),
        }
    }
    // Blitz drops a space where two spans meet: a run's edge spaces become
    // margin, a word space each.
    let runs: Vec<(bool, String, f32, f32)> = runs
        .into_iter()
        .map(|(on, s)| {
            let lead = s.chars().take_while(|c| *c == ' ').count();
            let trail = s.chars().rev().take_while(|c| *c == ' ').count();
            let word = WORD_SPACE;
            (
                on,
                s.trim().to_owned(),
                word * count(lead),
                word * count(trail),
            )
        })
        .collect();
    rsx! {
        span { style: "white-space: nowrap; overflow: hidden;",
            for (i, (on, s, before, after)) in runs.into_iter().enumerate() {
                span {
                    key: "{i}",
                    style: if on {
                        format!("display: inline-block; color: {lit}; font-weight: 650; margin-left: {before}em; margin-right: {after}em;")
                    } else {
                        format!("display: inline-block; color: {ink}; margin-left: {before}em; margin-right: {after}em;")
                    },
                    "{s}"
                }
            }
        }
    }
}

/// ⌘K: every command, matched as typed; ↑↓ to pick, ↵ to run.
#[component]
pub fn CommandPalette() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let q = Query::parse(&query.read());
    let rows: Vec<Ranked> = match q.scope {
        Scope::Commands => rank(&commands.all(), &q.text, &commands.recents.read()),
        _ => Vec::new(),
    };
    let count = rows.len();
    // A new query starts at the top.
    use_effect(move || {
        let _ = query();
        selected.set(0);
    });
    let ids: Vec<(String, bool)> = rows
        .iter()
        .map(|r| (r.command.id.clone(), r.command.enabled))
        .collect();
    let pick = selected().min(count.saturating_sub(1));
    let run_at = move |i: usize| {
        if let Some((id, true)) = ids.get(i) {
            commands.close();
            let _ = commands.run(id);
        }
    };
    let run_key = run_at.clone();
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::ArrowDown => {
            e.prevent_default();
            selected.set((pick.saturating_add(1)).min(count.saturating_sub(1)));
        }
        Key::ArrowUp => {
            e.prevent_default();
            selected.set(pick.saturating_sub(1));
        }
        Key::Enter => run_key(pick),
        Key::Escape => commands.close(),
        _ => {}
    };
    let empty = match q.scope {
        Scope::Commands if q.text.is_empty() => "No commands yet",
        Scope::Commands => "No command matches",
        Scope::Targets => "Nothing on this page to jump to yet",
        Scope::Settings => "Settings come with the settings page",
        Scope::Help => "Type a key to see what it does",
    };
    rsx! {
        Frame { placeholder: "Type a command…", query, on_key,
            if rows.is_empty() {
                div { style: "padding: 18px 12px; color: {t.ink_3}; font-size: 13px;", "{empty}" }
            }
            for (i, row) in rows.into_iter().enumerate() {
                CommandRow {
                    key: "{row.command.id}",
                    row,
                    on: i == pick,
                    onpress: {
                        let run_at = run_at.clone();
                        move |()| run_at(i)
                    },
                }
            }
        }
    }
}

#[component]
fn CommandRow(row: Ranked, on: bool, onpress: EventHandler<()>) -> Element {
    let t = theme();
    let c = row.command;
    let fill = if on { t.active } else { "transparent" };
    let ink = if c.enabled { t.ink } else { t.ink_3 };
    rsx! {
        div {
            class: "lp",
            style: "height: 36px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; \
                    background: {fill}; font-size: 13px; cursor: pointer;",
            onclick: move |_| onpress.call(()),
            span { style: "flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 8px; overflow: hidden;",
                if !c.group.is_empty() {
                    span { style: "flex-shrink: 0; color: {t.ink_3};", "{c.group}:" }
                }
                Lit { text: c.label.clone(), positions: row.positions, ink, lit: t.ink }
                if let Some(why) = c.disabled_reason.clone() {
                    span { style: "color: {t.ink_3}; font-size: 12px; white-space: nowrap;", "— {why}" }
                }
            }
            if !c.shortcut.is_empty() {
                Kbd { text: c.shortcut }
            }
        }
    }
}

/// ⌘O: open anything every opener knows; ↵ here, ⌘↵ in a new tab.
#[component]
pub fn QuickOpen() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let found: Vec<Found> = commands.openers.read().borrow().search(&query.read());
    let count = found.len();
    use_effect(move || {
        let _ = query();
        selected.set(0);
    });
    let pick = selected().min(count.saturating_sub(1));
    let items: Rc<Vec<Found>> = Rc::new(found);
    let open_at = {
        let items = items.clone();
        move |i: usize, open_in: OpenIn| {
            if let Some(f) = items.get(i) {
                commands.close();
                let mut recents = commands.recents;
                recents
                    .write()
                    .touch(&format!("{}:{}", f.item.kind, f.item.id));
                let _ = commands.openers.read().borrow().open(&f.item, open_in);
            }
        }
    };
    let open_key = open_at.clone();
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::ArrowDown => {
            e.prevent_default();
            selected.set((pick.saturating_add(1)).min(count.saturating_sub(1)));
        }
        Key::ArrowUp => {
            e.prevent_default();
            selected.set(pick.saturating_sub(1));
        }
        Key::Enter => {
            let m = e.modifiers();
            let open_in = if m.meta() && m.alt() {
                OpenIn::Split
            } else if m.meta() || m.ctrl() {
                OpenIn::Tab
            } else {
                OpenIn::Here
            };
            open_key(pick, open_in);
        }
        Key::Escape => commands.close(),
        _ => {}
    };
    let titles: Vec<String> = items
        .iter()
        .map(|f| {
            commands
                .openers
                .read()
                .borrow()
                .title(&f.item.kind)
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    rsx! {
        Frame { placeholder: "Open…  (note: to look in one place)", query, on_key,
            if items.is_empty() {
                div { style: "padding: 18px 12px; color: {t.ink_3}; font-size: 13px;", "Nothing to open matches" }
            }
            for (i, f) in items.iter().cloned().enumerate() {
                div {
                    key: "{f.item.kind}:{f.item.id}",
                    class: "lp",
                    style: "height: 40px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; \
                            background: {fill(i == pick, &t)}; font-size: 13px; cursor: pointer;",
                    onclick: {
                        let open_at = open_at.clone();
                        move |_| open_at(i, OpenIn::Here)
                    },
                    span { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden;",
                        Lit { text: f.item.title.clone(), positions: f.positions.clone(), ink: t.ink_2, lit: t.ink }
                        if !f.item.detail.is_empty() {
                            span { style: "color: {t.ink_3}; font-size: 11px; white-space: nowrap; overflow: hidden;", "{f.item.detail}" }
                        }
                    }
                    span { style: "flex-shrink: 0; color: {t.ink_3}; font-size: 11px;",
                        "{titles.get(i).cloned().unwrap_or_default()}"
                    }
                }
            }
            div { style: "display: flex; gap: 14px; padding: 8px 10px 4px; color: {t.ink_3}; font-size: 11px;",
                span { "Return  open" }
                span { "⌘ Return  new tab" }
                span { "⌘⌥ Return  split" }
            }
        }
    }
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
                CommandIcon { size: 15, color: t.ink_3 }
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
            }
            div { style: "flex-shrink: 0; display: flex; gap: 10px; padding: 6px 14px; color: {t.ink_3}; font-size: 11px; \
                          font-weight: 600; border-bottom: 1px solid {t.rule};",
                span { style: "flex: 3; min-width: 0;", "Action" }
                span { style: "flex: 2; min-width: 0;", "Group" }
                span { style: "flex: 1; min-width: 0;", "Shortcut" }
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
    let ink = if command.enabled { t.ink } else { t.ink_3 };
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
            span { style: "flex: 1; min-width: 0;",
                if !command.shortcut.is_empty() {
                    Kbd { text: command.shortcut.clone() }
                }
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
