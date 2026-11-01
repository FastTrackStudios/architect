//! The app shell's lab: a made-up app on [`architect_shell`] that
//! `apps/architect-desktop` and `apps/architect-mobile` mount, so the shell
//! can be designed on a phone, a tablet and a desk with `dx serve`'s hot
//! reload — an `rsx!` edit here or in `architect-shell` shows in the running
//! window at once.
//!
//! The app is Task's shape (the shell's reference): Inbox, Tasks, Schedule
//! and Vault as the tabs, Capture as the separate button, projects in the
//! left side and the selection's details in the right. Its own menu (the
//! flask) switches the shell's style live.

mod arrange;
mod extensions;
mod items;
mod pages;
mod screen;
mod settings;
mod vault;

use architect_orgs::{Capabilities, Org, Orgs, Role};
use architect_shell::icon;
use architect_shell::{self as shell, AppShell, ShellStyle};
use dioxus::prelude::*;
use lucide_dioxus::{
    CalendarDays, CheckCheck, Feather, Inbox, Keyboard, ListTodo, NotebookText, Plus, Settings2,
    Undo2, Users,
};

pub use screen::{Device, use_window_chrome, use_window_screen};

/// Where the lab is: a tab's list, one item of it (by its id), a new one
/// being written for it, or the help (over the tab it was asked from).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Tab(&'static str),
    Item(&'static str, usize),
    Capture(&'static str),
    Help(&'static str),
}

impl Route {
    const fn tab(self) -> &'static str {
        match self {
            Self::Tab(t) | Self::Item(t, _) | Self::Capture(t) | Self::Help(t) => t,
        }
    }
}

/// The lab's tabs: the command that goes to each, its label, its id.
const TABS: [(&str, &str, &str); 4] = [
    ("LAB_GO_INBOX", "Go to Inbox", "inbox"),
    ("LAB_GO_TASKS", "Go to Tasks", "tasks"),
    ("LAB_GO_SCHEDULE", "Go to Schedule", "schedule"),
    ("LAB_GO_VAULT", "Go to Vault", "vault"),
];

/// What a Done, Delete or Move changed, for Undo.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Undone {
    /// "Completed", "Deleted", "Moved": the status line's word.
    pub verb: &'static str,
    /// What Edit › Undo calls it: "Done", "Delete", "Move".
    pub action: &'static str,
    /// The whole of it: "Moved “Bass DI is buzzing” to Tasks in Signal".
    pub said: String,
    at: usize,
    /// The item as it was, where it was.
    item: items::Item,
    change: Change,
}

/// What a change did to the item, for Undo and Redo to do back and again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Change {
    /// Done or Delete took it out.
    Removed,
    /// Move filed it here (a tab, a project).
    Moved(&'static str, Option<&'static str>),
    /// Capture wrote it.
    Added,
}

/// `n` of `noun`, with its plural: "1 item", "5 items".
pub(crate) fn count<N: std::fmt::Display + PartialEq + From<u8> + Copy>(
    n: N,
    noun: &str,
) -> String {
    if n == N::from(1) {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The lab's state, shared with its pages.
#[derive(Clone, Copy)]
pub struct Lab {
    pub route: Signal<Route>,
    back: Signal<Vec<Route>>,
    pub style: Signal<ShellStyle>,
    /// Whether the app fills the right side.
    pub right: Signal<bool>,
    /// How a desk shows the status items.
    pub status: Signal<shell::StatusStyle>,
    /// Every list.
    pub lists: Signal<items::Lists>,
    /// The project picked on the left: the Tasks list shows only its.
    pub project: Signal<Option<&'static str>>,
    /// The lists oldest first.
    pub oldest: Signal<bool>,
    /// The row just added, marked in its list until the lab moves on.
    pub fresh: Signal<Option<usize>>,
    /// What Undo puts back, the last at the end (⌘Z steps back through
    /// it).
    pub undone: Signal<Vec<Undone>>,
    /// What Undo put back, for Redo (⇧⌘Z); a new change lets it go.
    pub redone: Signal<Vec<Undone>>,
    /// Whether the last change is still said over its list (until the
    /// lab moves to another list).
    pub notice: Signal<bool>,
    /// The rows being completed: checked and fading, then gone.
    pub completing: Signal<Vec<usize>>,
    /// The row the keys act on in a list (the one focused).
    pub selected: Signal<Option<usize>>,

    settings: Option<architect_settings::SettingsHandle>,
}

impl Lab {
    /// Sets a text setting (the lab's signals follow the settings).
    pub fn put(self, key: &str, id: &str) {
        if let Some(s) = self.settings {
            s.set(key, architect_settings::Value::Text(id.to_owned()));
        }
    }

    pub fn put_bool(self, key: &str, on: bool) {
        if let Some(s) = self.settings {
            s.set(key, architect_settings::Value::Bool(on));
        }
    }

    pub fn go(mut self, to: Route) {
        let here = *self.route.peek();
        // A row pressed and its Return heard at once go once.
        if here == to {
            return;
        }
        self.back.write().push(here);
        self.fresh.set(None);
        self.route.set(to);
    }

    /// To a tab's list or an item afresh: no way back, no project picked,
    /// and the last change no longer said over the list (⌘Z still undoes
    /// it).
    pub fn jump(mut self, to: Route) {
        self.back.write().clear();
        self.project.set(None);
        self.fresh.set(None);
        self.notice.set(false);
        self.selected.set(None);
        self.route.set(to);
    }

    /// Keeps `u` for Undo and says it over the list.
    fn did(mut self, u: Undone) {
        let mut undone = self.undone.write();
        undone.push(u);
        let over = undone.len().saturating_sub(UNDO_STEPS);
        undone.drain(..over);
        drop(undone);
        self.redone.write().clear();
        self.notice.set(true);
    }

    /// The ids of the list on show (on an item's page, its list's).
    fn shown_ids(self) -> Vec<usize> {
        let tab = self.route.peek().tab();
        let project = if tab == "tasks" {
            *self.project.peek()
        } else {
            None
        };
        self.lists
            .read()
            .of(tab, project, *self.oldest.peek())
            .iter()
            .map(|i| i.id)
            .collect()
    }

    /// Done on `id`: in a list its circle fills and the row fades before
    /// it goes (the selection already on the next row, so ⌘↵ again
    /// completes that one too); on its own page, at once.
    pub fn complete(mut self, id: usize) {
        if !matches!(*self.route.peek(), Route::Tab(_)) {
            self.remove(id, "Completed");
            return;
        }
        let mut completing = self.completing;
        if completing.peek().contains(&id) {
            return;
        }
        let shown: Vec<usize> = self
            .shown_ids()
            .into_iter()
            .filter(|i| !completing.peek().contains(i))
            .collect();
        completing.write().push(id);
        self.selected.set(items::next_after(&shown, id));
        spawn(async move {
            futures_timer::Delay::new(COMPLETING).await;
            completing.write().retain(|i| *i != id);
            self.remove(id, "Completed");
        });
    }

    /// The Tasks of `project`.
    pub fn show_project(mut self, project: &'static str) {
        self.jump(Route::Tab("tasks"));
        self.project.set(Some(project));
    }

    /// Write a new item for `tab` (Capture, for the Inbox).
    pub fn capture(self, tab: &'static str) {
        self.go(Route::Capture(tab));
    }

    /// Adds `name` to `tab` (in the picked project) and shows it there;
    /// a blank name adds nothing.
    pub fn add(mut self, tab: &'static str, name: &str) {
        let project = if tab == "inbox" {
            None
        } else {
            *self.project.peek()
        };
        let Some(id) = self.lists.write().add(tab, name, project) else {
            return;
        };
        let at = self.lists.peek().position(id).unwrap_or_default();
        let item = self.lists.peek().get(id).cloned();
        if let Some(item) = item {
            let (verb, action) = if tab == "inbox" {
                ("Captured", "Capture")
            } else {
                ("Added", "Add")
            };
            self.did(Undone {
                verb,
                action,
                said: format!("{verb} “{}”", item.name),
                at,
                item,
                change: Change::Added,
            });
        }
        // Back to the list it was written from, the new row marked and
        // selected: the next key acts on what was just written.
        self.back_to(tab);
        self.fresh.set(Some(id));
        self.selected.set(Some(id));
    }

    /// Esc on a capture with text: dropped, but kept for Undo to bring
    /// back into the list (nothing typed is lost to a stray Esc).
    pub fn drop_capture(mut self, tab: &'static str, name: &str) {
        let project = if tab == "inbox" {
            None
        } else {
            *self.project.peek()
        };
        let id = self.lists.write().add(tab, name, project);
        let removed = id.and_then(|id| self.lists.write().remove(id));
        self.back_to(tab);
        if let Some((at, item)) = removed {
            self.did(Undone {
                verb: "Dropped",
                action: "Drop",
                said: format!("Dropped “{}”", item.name),
                at,
                item,
                change: Change::Removed,
            });
        }
    }

    /// To `tab`'s list, from a page written for it.
    fn back_to(mut self, tab: &'static str) {
        let mut back = self.back;
        let from = back.write().pop();
        if from != Some(Route::Tab(tab)) {
            back.write().clear();
        }
        self.route.set(Route::Tab(tab));
    }

    /// Takes the item away (`verb`: "Completed", "Deleted"), for Undo to
    /// put back; from the item's own page, back to its list.
    pub fn remove(mut self, id: usize, verb: &'static str) {
        // The next row, unless the selection has moved on already (a
        // completion fading out).
        let next = if *self.selected.peek() == Some(id) || self.selected.peek().is_none() {
            items::next_after(&self.shown_ids(), id)
        } else {
            *self.selected.peek()
        };
        let removed = self.lists.write().remove(id);
        if let Some((at, item)) = removed {
            let tab = item.tab;
            let said = format!("{verb} “{}”", item.name);
            let action = match verb {
                "Deleted" => "Delete",
                "Dropped" => "Drop",
                _ => "Done",
            };
            self.did(Undone {
                verb,
                action,
                said,
                at,
                item,
                change: Change::Removed,
            });
            // The next row at hand, for the next Done.
            self.selected.set(next);
            if matches!(*self.route.peek(), Route::Item(_, on) if on == id) {
                if self.can_go_back() {
                    self.go_back();
                } else {
                    self.route.set(Route::Tab(tab));
                }
            }
        }
    }

    /// Files the item in `tab`'s list, in `project` (triage), and goes
    /// back to the list it came from, the next one to file at hand, with
    /// Undo.
    pub fn move_item(mut self, id: usize, tab: &'static str, project: Option<&'static str>) {
        let before = self.lists.read().get(id).cloned();
        let at = self.lists.read().position(id);
        let (Some(item), Some(at)) = (before, at) else {
            return;
        };
        let next = items::next_after(&self.shown_ids(), id);
        if !self.lists.write().move_to(id, tab, project) {
            return;
        }
        let from = item.tab;
        // Said as the Move to list says it.
        let to = move_label(tab, project);
        self.did(Undone {
            verb: "Moved",
            action: "Move",
            said: format!("Moved “{}” to {to}", item.name),
            at,
            item,
            change: Change::Moved(tab, project),
        });
        // Triage: the next capture selected, for ⇧⌘M to file.
        self.selected.set(next);
        if matches!(*self.route.peek(), Route::Item(_, on) if on == id) {
            if self.can_go_back() {
                self.go_back();
            } else {
                self.route.set(Route::Tab(from));
            }
        }
    }

    /// Picks the row `by` rows on from the selected one in the list on
    /// show (the first, when none is).
    pub fn step(self, by: isize) {
        let Route::Tab(tab) = *self.route.peek() else {
            return;
        };
        let project = if tab == "tasks" {
            *self.project.peek()
        } else {
            None
        };
        let ids: Vec<usize> = self
            .lists
            .read()
            .of(tab, project, *self.oldest.peek())
            .iter()
            .map(|i| i.id)
            .collect();
        let at = (*self.selected.peek()).and_then(|id| ids.iter().position(|i| *i == id));
        let next = at.map_or(0, |at| {
            at.saturating_add_signed(by)
                .min(ids.len().saturating_sub(1))
        });
        if let Some(id) = ids.get(next) {
            let mut selected = self.selected;
            selected.set(Some(*id));
        }
    }

    /// Opens the list's selected row.
    pub fn open_selected(self) {
        // Read, then let go of the signals before `go` writes the route.
        let here = *self.route.peek();
        let selected = *self.selected.peek();
        if let (Route::Tab(tab), Some(id)) = (here, selected) {
            self.go(Route::Item(tab, id));
        }
    }

    /// The item Done and the keys act on: the page's, or the list's
    /// selected row.
    #[must_use]
    pub fn target(self) -> Option<usize> {
        match *self.route.peek() {
            Route::Item(_, id) => Some(id),
            Route::Tab(_) => *self.selected.peek(),
            Route::Capture(_) | Route::Help(_) => None,
        }
    }

    /// Puts back what the last Done, Delete or Move changed; again, the
    /// one before.
    pub fn undo(mut self) {
        let undone = self.undone.write().pop();
        if let Some(u) = undone {
            let id = u.item.id;
            let mut lists = self.lists.write();
            match u.change {
                Change::Removed => lists.restore(u.at, u.item.clone()),
                Change::Moved(..) => {
                    let _ = lists.remove(id);
                    lists.restore(u.at, u.item.clone());
                }
                Change::Added => {
                    let _ = lists.remove(id);
                }
            }
            drop(lists);
            let back = u.change != Change::Added;
            self.redone.write().push(u);
            self.fresh.set(back.then_some(id));
            self.selected.set(back.then_some(id));
            // The notice said what is now undone: the row's mark says it.
            self.notice.set(false);
        }
    }

    /// Does again what Undo put back.
    pub fn redo(mut self) {
        let redone = self.redone.write().pop();
        let Some(u) = redone else {
            return;
        };
        // Doing it again is a change like any other, but keeps the rest
        // of what Redo has.
        let rest = std::mem::take(&mut *self.redone.write());
        match u.change {
            Change::Moved(tab, project) => self.move_item(u.item.id, tab, project),
            Change::Removed => self.remove(u.item.id, u.verb),
            Change::Added => {
                self.lists.write().restore(u.at, u.item.clone());
                self.fresh.set(Some(u.item.id));
                self.did(u);
            }
        }
        self.redone.set(rest);
    }

    /// The help, over the tab it is asked from.
    pub fn help(self) {
        // Let go of the route before `go` writes it.
        let tab = self.route.peek().tab();
        self.go(Route::Help(tab));
    }

    #[must_use]
    pub fn can_go_back(self) -> bool {
        !self.back.read().is_empty()
    }

    pub fn go_back(mut self) {
        let prev = self.back.write().pop();
        if let Some(prev) = prev {
            self.route.set(prev);
        }
    }
}

/// How many changes Undo keeps.
const UNDO_STEPS: usize = 20;

/// How long a completed row shows its check before it goes.
const COMPLETING: std::time::Duration = std::time::Duration::from_millis(420);

/// The destinations ⇧⌘M offers, in the left side's order, each a
/// command: its id, where it files the item (the tab, the project). Not
/// the Schedule: an event needs a time, which a move cannot ask.
const MOVES: [(&str, &str, Option<&str>); 6] = [
    ("LAB_MOVE_STUDIO", "tasks", Some("Studio")),
    ("LAB_MOVE_HARBOR", "tasks", Some("Harbor & Pine")),
    ("LAB_MOVE_ALBUM", "tasks", Some("Album release")),
    ("LAB_MOVE_SIGNAL", "tasks", Some("Signal")),
    ("LAB_MOVE_ADMIN", "tasks", Some("Admin")),
    ("LAB_MOVE_INBOX", "inbox", None),
];

/// A destination, as the Move to list and the notice say it: "Signal",
/// "Inbox".
const fn move_label(tab: &str, project: Option<&'static str>) -> &'static str {
    match (tab.as_bytes(), project) {
        (_, Some(p)) => p,
        (b"inbox", None) => "Inbox",
        _ => "Schedule",
    }
}

/// Edit › Undo or Redo: "Undo Move", what it changes in its detail; off
/// with nothing to do, or while a field is typed in (its own then).
fn edit_command(
    id: &str,
    verb: &str,
    change: Option<&Undone>,
    typing: bool,
    detail: &str,
    none: &str,
) -> architect_commands::Command {
    let c = match change {
        Some(u) if !typing => architect_commands::Command::new(id, format!("{verb} {}", u.action))
            .detail(format!("{detail} “{}”", u.item.name)),
        Some(_) => architect_commands::Command::new(id, verb).disabled("this page's field has it"),
        None => architect_commands::Command::new(id, verb).disabled(none),
    };
    c.group("Edit")
}

/// The stylesheet: the page's ground, and hover and press feedback
/// (`.lp`; an inset shadow, so it shows over a row's own background).
const CSS: &str = "\
html, body { margin: 0; padding: 0; background: #09090b; }\
button { cursor: pointer; }\
.lp:hover { box-shadow: inset 0 0 0 999px rgba(255,255,255,0.035); }\
.lp:active { background-color: rgba(255,255,255,0.07); }\
";

/// The lab's face, the same on every desk and phone.
///
/// Inter (variable, OFL: `assets/fonts/OFL.txt`), `FastTrackStudio`'s, the
/// first of the shell's stack (`ShellTheme::font`). The apps hand it to
/// Blitz when they launch (an `@font-face` from a `data:` URI does not
/// load).
pub const FONTS: &[&[u8]] = &[include_bytes!("../assets/fonts/Inter-Variable.ttf")];

/// The system's faces and [`FONTS`], for `dioxus_native::Config::with_font_ctx`.
#[must_use]
pub fn font_context() -> dioxus_native::FontContext {
    let mut ctx = dioxus_native::FontContext::default();
    for font in FONTS {
        ctx.collection.register_fonts(font.to_vec().into(), None);
    }
    ctx
}

/// The lab's root. The host publishes the window as a
/// `Signal<shell::Screen>` ([`use_window_screen`]) above it.
#[component]
pub fn LabApp() -> Element {
    let screen = use_context::<Signal<shell::Screen>>();
    // Settings: the lab's sections, kept on this device.
    let store = use_hook(|| {
        architect_settings::DeviceValues::open("architect-lab")
            .ok()
            .map(std::rc::Rc::new)
    });
    let save = store.clone();
    let settings = architect_settings::use_settings_provider(
        settings::registry,
        || architect_settings::ScopedValues {
            device: store.as_ref().map(|s| s.load()).unwrap_or_default(),
            ..Default::default()
        },
        Some(Callback::new(
            move |change: architect_settings::SettingsChange| {
                // The lab's sections are all this device's.
                if change.scope == architect_settings::Scope::Device {
                    if let Some(s) = save.as_ref() {
                        let _ = s.save(&change.values);
                    }
                }
            },
        )),
    );
    let lab = use_context_provider(|| Lab {
        route: Signal::new(Route::Tab("inbox")),
        back: Signal::new(Vec::new()),
        style: Signal::new(ShellStyle::FloatingPill),
        right: Signal::new(true),
        status: Signal::new(shell::StatusStyle::Bar),
        lists: Signal::new(items::Lists::sample()),
        project: Signal::new(None),
        oldest: Signal::new(false),
        fresh: Signal::new(None),
        undone: Signal::new(Vec::new()),
        redone: Signal::new(Vec::new()),
        notice: Signal::new(false),
        completing: Signal::new(Vec::new()),
        selected: Signal::new(None),
        settings: Some(settings),
    });
    // The lab's signals follow the settings.
    {
        let text = |key: &str| {
            architect_settings::use_setting(key)
                .and_then(|v| v.as_text().map(str::to_owned))
                .unwrap_or_default()
        };
        let style = settings::shell_style(&text("workspace.shell"));
        let status = settings::status_style(&text("workspace.status"));
        let right = architect_settings::use_setting("workspace.details")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let (mut a, mut b, mut c) = (lab.style, lab.status, lab.right);
        if *a.peek() != style {
            a.set(style);
        }
        if *b.peek() != status {
            b.set(status);
        }
        if *c.peek() != right {
            c.set(right);
        }
    }
    let metronome = architect_settings::use_setting("workspace.metronome")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Commands: the palette (⌘K), quick open (⌘O), and every command's key.
    let commands = architect_commands::use_commands_provider();
    for (n, (id, label, to)) in TABS.into_iter().enumerate() {
        // ⌃⌘1–4: ⌘1–9 are the Vault's screensets, and ⌥ changes the digit.
        architect_commands::use_command(
            architect_commands::Command::new(id, label)
                .group("Go")
                .shortcut(format!("⌃⌘{}", n.saturating_add(1))),
            move || lab.jump(Route::Tab(to)),
        );
    }
    architect_commands::use_command(
        architect_commands::Command::new("LAB_TOGGLE_DETAILS", "Details")
            .group("View")
            .checked((lab.right)()),
        move || lab.put_bool("workspace.details", !(lab.right)()),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_CAPTURE", "Capture")
            .group("Inbox")
            .shortcut("⇧⌘N"),
        move || lab.capture("inbox"),
    );
    for (id, label, by) in [
        ("LAB_NEXT", "Select the next item", 1),
        ("LAB_PREVIOUS", "Select the previous item", -1),
    ] {
        architect_commands::use_command(
            architect_commands::Command::new(id, label)
                .group("Go")
                .shortcut(if by > 0 { "⌃N" } else { "⌃P" }),
            move || lab.step(by),
        );
    }
    architect_commands::use_command(
        architect_commands::Command::new("LAB_OPEN_SELECTED", "Open the selected item")
            .group("Go")
            .shortcut("⌘↓"),
        move || lab.open_selected(),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_DONE", "Mark done")
            .group("Edit")
            .shortcut("⌘↵"),
        move || {
            if let Some(id) = lab.target() {
                lab.complete(id);
            }
        },
    );
    // Edit › Undo and Redo are the lab's (⌘Z, ⇧⌘Z), named for what they
    // change; on a page with a field to type in (Capture, an item's
    // notes), they stand aside for it.
    let typing = matches!((lab.route)(), Route::Capture(_) | Route::Item(..));
    let last = (lab.undone)().last().cloned();
    let again = (lab.redone)().last().cloned();
    architect_commands::use_command(
        edit_command(
            "LAB_UNDO",
            "Undo",
            last.as_ref(),
            typing,
            "Put back",
            "nothing to undo yet",
        )
        .shortcut("⌘Z"),
        move || lab.undo(),
    );
    architect_commands::use_command(
        edit_command(
            "LAB_REDO",
            "Redo",
            again.as_ref(),
            typing,
            "Again:",
            "nothing undone to redo",
        )
        .shortcut("⇧⌘Z"),
        move || lab.redo(),
    );
    // Triage from the keys: ⇧⌘M asks where, typing narrows it, Return
    // files the item and selects the next.
    let _ = (lab.selected)();
    let target = lab
        .target()
        .and_then(|id| lab.lists.read().get(id).cloned());
    architect_commands::use_command(
        {
            let c = architect_commands::Command::new("LAB_MOVE", "Move to…")
                .group("Edit")
                .shortcut("⇧⌘M");
            if target.is_some() {
                c
            } else {
                c.disabled("select an item first")
            }
        },
        move || {
            let name = lab
                .target()
                .and_then(|id| lab.lists.peek().get(id).map(|i| i.name.clone()))
                .unwrap_or_default();
            commands.show_group("Move to", &format!("Move “{name}” to…"));
        },
    );
    for (id, tab, project) in MOVES {
        let here = target
            .as_ref()
            .is_some_and(|i| i.tab == tab && (tab != "tasks" || i.project == project));
        let c = architect_commands::Command::new(id, move_label(tab, project)).group("Move to");
        architect_commands::use_command(
            match &target {
                None => c.disabled("select an item first"),
                Some(_) if here => c.disabled("it is already there"),
                Some(_) => c,
            },
            move || {
                if let Some(at) = lab.target() {
                    lab.move_item(at, tab, project);
                }
            },
        );
    }
    architect_commands::use_command(
        architect_commands::Command::new("LAB_HELP", "Keys and how the lab works")
            .group("Help")
            .shortcut("⌘/"),
        move || lab.help(),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_SHOW_COMMANDS", "Show every command")
            .group("Help")
            .shortcut("⌘K"),
        move || commands.show(architect_commands::Overlay::Palette),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_OPEN_ANYTHING", "Open…")
            .group("File")
            .shortcut("⌘O"),
        move || commands.show(architect_commands::Overlay::QuickOpen),
    );
    // The menu bar: a view of the commands, labels and keys and all.
    let menus = architect_commands::MenuBar::standard("Architect Lab")
        .menu("File", |m| {
            m.commands(["LAB_CAPTURE", "LAB_OPEN_ANYTHING"])
                .separator()
                .commands([
                    "WORKSPACE_CLOSE_TAB",
                    "WORKSPACE_REOPEN_TAB",
                    "WORKSPACE_CLOSE_OTHER_TABS",
                ])
        })
        .menu("Edit", |m| {
            m.instead(architect_commands::Standard::Undo, "LAB_UNDO")
                .instead(architect_commands::Standard::Redo, "LAB_REDO")
                .separator()
                .commands(["LAB_DONE", "LAB_MOVE"])
        })
        .menu("View", |m| {
            m.command("LAB_TOGGLE_DETAILS")
                .separator()
                .group("Panes")
                .separator()
                .group("Docks")
        })
        .menu("Go", |m| {
            // The tabs, the one showing ticked: a live section, read again
            // whenever the bar is drawn.
            m.section("Tabs", move || {
                let on = (lab.route)().tab();
                TABS.iter()
                    .map(|(id, label, to)| {
                        architect_commands::Live::new(*label, *id).checked(*to == on)
                    })
                    .collect()
            })
            .separator()
            .commands([
                "WORKSPACE_NEXT_TAB",
                "WORKSPACE_PREVIOUS_TAB",
                "WORKSPACE_GO_BACK",
                "WORKSPACE_GO_FORWARD",
            ])
        })
        .menu("Help", |m| m.commands(["LAB_HELP", "LAB_SHOW_COMMANDS"]));
    let route = (lab.route)();
    let inbox_count = lab.lists.read().count("inbox");
    let tabs = vec![
        shell::Tab::new("inbox", "Inbox", icon!(Inbox)).badge(inbox_count),
        shell::Tab::new("tasks", "Tasks", icon!(ListTodo)),
        shell::Tab::new("schedule", "Schedule", icon!(CalendarDays)),
        shell::Tab::new("vault", "Vault", icon!(NotebookText)),
    ];
    // Which workspace the lab shows; `None` is all of them.
    // The person's organizations (the identity server's, here made up):
    // the switcher, and what each role may do.
    let orgs = architect_orgs::use_orgs_provider(
        || {
            Orgs::new(vec![
                Org::new("me", "Personal", Role::Owner)
                    .personal()
                    .color("#047857"),
                Org::new("fts", "FastTrackStudio", Role::Admin)
                    .members(12)
                    .color("#1d4ed8"),
                Org::new("hp", "Harbor & Pine", Role::Viewer)
                    .members(4)
                    .color("#b45309"),
            ])
        },
        Capabilities::new()
            .allow(&Role::Viewer, ["notes.read"])
            .allow(&Role::Member, ["notes.write"])
            .allow(&Role::Admin, ["members.invite"])
            .allow(&Role::Owner, ["org.delete"])
            .inherit(),
        None,
    );
    let workspaces = orgs
        .switcher(Some("All organizations"))
        .action("New organization", icon!(Plus), |()| {})
        .action("Manage organizations", icon!(Settings2), |()| {});
    // A command that is the role's to allow: off with the reason elsewhere.
    let here = orgs
        .current()
        .map_or_else(|| "every organization".to_owned(), |o| o.name);
    let invite =
        architect_commands::Command::new("ORG_INVITE", "Invite people").group("Organization");
    architect_commands::use_command(
        if orgs.can("members.invite") {
            invite
        } else {
            invite.disabled(format!("only an admin can invite to {here}"))
        },
        || {},
    );
    let window_chrome = screen::use_window_chrome();
    let capture = shell::Action::new("Capture", icon!(Feather), move |()| {
        lab.capture("inbox");
    });
    let mut status = vec![
        shell::StatusItem::new("commands", "⌘K Commands")
            .hint("Every command")
            .on_press(move |()| commands.show(architect_commands::Overlay::Palette)),
        shell::StatusItem::new("sync", "Synced")
            .icon(icon!(CheckCheck))
            .hint("Every change is saved and synced"),
        shell::StatusItem::new("people", "3 online").icon(icon!(Users)),
    ];
    // Done, Delete and Move keep their way back on the status line,
    // where the notice over their list is not on show.
    let said_here = (lab.notice)()
        && last
            .as_ref()
            .is_some_and(|u| route == Route::Tab(u.item.tab));
    if let Some(u) = last.as_ref().filter(|_| !said_here) {
        status.insert(
            0,
            shell::StatusItem::new(
                "undo",
                format!(
                    "{} {} {} · Undo ⌘Z",
                    u.verb,
                    if matches!(u.change, Change::Moved(..)) {
                        "from"
                    } else {
                        "in"
                    },
                    pages::title(u.item.tab)
                ),
            )
            .icon(icon!(Undo2))
            .hint(format!("Put back “{}”", u.item.name))
            .on_press(move |()| lab.undo()),
        );
    }
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; overflow: hidden;",
            document::Style { {CSS} }
            // The window's chords (⌘K, ⌘W, …), whatever has the focus.
            architect_commands::WindowKeys {}
            ListKeys {}
            NativeMenus { bar: menus }
            AppShell {
                screen: screen(),
                style: (lab.style)(),
                title: "Architect Lab",
                tabs,
                current: Some(route.tab()),
                on_tab: move |id| lab.jump(Route::Tab(id)),
                separate: capture,
                brand: rsx! { Brand {} },
                account: shell::Account::new("Cody Wright")
                    .detail("cody@fasttrackstudio.app")
                    .presence("#22c55e")
                    .menu(
                        // The lab has no account to leave: only what works.
                        shell::Menu::new()
                            .item("Settings", icon!(Settings2), move |()| settings.show(None))
                            .item("Keys and help", icon!(Keyboard), move |()| {
                                lab.help();
                            }),
                    ),
                workspaces,
                left: rsx! { pages::LeftSide {} },
                // The Vault's panes are its own, the help is the help; the
                // details are the lists'.
                right: if (lab.right)() && route != Route::Tab("vault") && !matches!(route, Route::Help(_)) { Some(rsx! { pages::RightSide {} }) } else { None },
                on_pull: move |()| lab.jump(Route::Tab("vault")),
                window: window_chrome,
                status,
                status_style: (lab.status)(),
                match route {
                    Route::Tab("vault") => rsx! { vault::VaultPage {} },
                    Route::Tab(tab) => rsx! { pages::ListPage { key: "{tab}", tab } },
                    Route::Item(tab, id) => rsx! { pages::ItemPage { tab, id } },
                    Route::Capture(tab) => rsx! { pages::CapturePage { key: "{tab}", tab } },
                    Route::Help(_) => rsx! { pages::HelpPage {} },
                }
            }
            architect_commands::CommandLayer {}
            architect_settings::SettingsPage {}
            extensions::LabExtensions { metronome }
            screen::DeviceChrome {}
        }
    }
}

/// The lab's brand, T3 Code's way: a mark and a two-tone wordmark.
#[component]
fn Brand() -> Element {
    let font = shell::use_shell().theme.font;
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; font-family: {font};",
            span { style: "width: 26px; height: 26px; border-radius: 7px; background: #e4e4e7; display: flex; \
                           align-items: center; justify-content: center; color: #09090b; font-size: 14px; font-weight: 800;",
                "A"
            }
            span { style: "font-size: 19px; font-weight: 750; letter-spacing: -0.03em; color: #e4e4e7;", "Architect" }
            span { style: "font-size: 19px; font-weight: 500; letter-spacing: -0.03em; color: #8f8f99;", "Lab" }
        }
    }
}

/// A list's plain keys: ↑ and ↓ select a row, Return opens it. Heard from
/// the window (Blitz moves no focus for the page to hear them), and only
/// over a list with nothing over it, so the palette's field and the
/// capture field keep theirs.
#[component]
fn ListKeys() -> Element {
    use dioxus_native::winit::event::{ElementState, WindowEvent};
    use dioxus_native::winit::keyboard::{Key, ModifiersState, NamedKey};

    let lab = use_context::<Lab>();
    let commands = use_context::<architect_commands::Commands>();
    let mut held = use_signal(ModifiersState::default);
    dioxus_native::use_window_event(move |event, _| match event {
        WindowEvent::ModifiersChanged(m) => held.set(m.state()),
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
            let on_list = matches!(*lab.route.peek(), Route::Tab(t) if t != "vault");
            let over = commands.overlay.peek().is_some()
                || lab.settings.is_some_and(|s| s.open.peek().is_some());
            if !on_list || over || !held.peek().is_empty() {
                return;
            }
            match &event.logical_key {
                Key::Named(NamedKey::ArrowDown) => lab.step(1),
                Key::Named(NamedKey::ArrowUp) => lab.step(-1),
                Key::Named(NamedKey::Enter) => lab.open_selected(),
                _ => {}
            }
        }
        _ => {}
    });
    rsx! {}
}

/// The menu bar: the platform's where there is one (macOS, a Linux shell
/// that takes dbusmenu, iPadOS), else the app's own in the window.
#[component]
fn NativeMenus(bar: architect_commands::MenuBar) -> Element {
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "ios"))]
    return rsx! {
        architect_commands::NativeMenus { bar: bar.clone() }
        architect_commands::InWindowMenus { bar }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "ios")))]
    rsx! { architect_commands::InWindowMenus { bar } }
}
