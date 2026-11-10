//! The Architect app's UI, on [`architect_shell`]: what `apps/desktop`
//! (`architect-desktop`), `apps/mobile` (`architect-mobile`) and the dev
//! sandbox (`apps/sandbox`, `architect-sandbox`) mount, with `dx serve`'s
//! hot reload — an `rsx!` edit here or in `architect-shell` shows in the
//! running window at once.
//!
//! Which app it is comes from its [`distribution`]. The Architect app opens the
//! person's vault (its first run asks for one, `first_run`) in the Vault
//! tab. The sandbox is the old lab, Task's shape (the shell's reference):
//! made-up Inbox, Tasks, Schedule and Vault as the tabs, Capture as the
//! separate button, projects in the left side and the selection's details
//! in the right, the example extensions (Board, Clicker) loaded. Its own
//! menu (the flask) switches the shell's style live.

mod accounts;
mod arrange;
mod capture;
mod confirm;
mod cursors;
mod decks;
pub mod distribution;
mod embedded;
mod extensions;
mod files;
mod first_run;
mod guide;
mod items;
#[cfg(feature = "keyflow-lab")]
mod keyflow;
mod note;
mod note_editor;
mod notes;
mod pages;
mod paint;
pub mod presence;
mod properties;
mod publish;
mod screen;
mod setlists;
mod settings;
mod surfaces;
mod theme;
mod vault;
pub mod vaults;

use architect_orgs::{Capabilities, Org, Orgs, Role};
use architect_shell::icon;
use architect_shell::{self as shell, AppShell, ShellStyle};
use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowLeft, CalendarDays, CheckCheck, CircleQuestionMark, Command as CommandIcon, Feather,
    Inbox, Keyboard, LayoutGrid, ListTodo, NotebookText, Palette, Plus, Search, Settings2, Timer,
    Undo2, Users,
};

pub use distribution::{app_dir, app_dir_or_temp, distribution, sandbox};
pub use guide::builtin_tours;
pub use screen::{Device, use_window_chrome, use_window_screen};

/// `EXT_CHECK` as the lab runs it — its view kinds, process extensions,
/// page files — for `architect-desktop extensions check <folder>` with no
/// window.
#[must_use]
pub fn extension_checker() -> architect_extensions::check::Checker {
    let pages = architect_pages::runner::Pages::new(architect_pages::host::AppHost::new(
        architect::action::ActionList::new(),
    ))
    .with_js()
    .with_luau()
    .with_a2ui();
    architect_extensions::check::Checker::new()
        .with_view_kinds(embedded::kinds())
        .with_page_checker(pages.page_checker())
}

/// The page API's types, written beside the agents' guide for an editor
/// (`architect_control::guide::Guide::file`): `(name, what it is, text)`.
#[must_use]
pub fn page_types() -> [(&'static str, &'static str, String); 2] {
    use architect_pages::codegen::{DTS, LUAU, luau, typescript};
    [
        (
            DTS,
            "the page API's TypeScript types, for a TSX/JS page under `pages/` \
             (`import { h, useState, actions, theme } from \"architect\"`); point your \
             editor or `tsc --noEmit` at it. `PAGES_CHECK {file}` renders a page once.",
            typescript(),
        ),
        (
            LUAU,
            "the page API's Luau types, for a Luau element page under `pages/`.",
            luau(),
        ),
    ]
}

/// In context above [`LabApp`] with no window (a film's run): read and
/// watch the lab's folder ([`app_dir`]) as a window's lab does (its theme
/// files), though nothing of the person's is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabFiles;

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

/// Every tab: the command that goes to each, its label, its id.
const ALL_TABS: [(&str, &str, &str); 4] = [
    ("LAB_GO_INBOX", "Go to Inbox", "inbox"),
    ("LAB_GO_TASKS", "Go to Tasks", "tasks"),
    ("LAB_GO_SCHEDULE", "Go to Schedule", "schedule"),
    ("LAB_GO_VAULT", "Go to Vault", "vault"),
];

/// The tabs this app shows: the sandbox's four, the Architect app's Vault
/// (its Inbox, Tasks and Schedule come with capture and projects, #352,
/// #344).
fn tabs() -> &'static [(&'static str, &'static str, &'static str)] {
    if crate::distribution().is_sandbox() {
        &ALL_TABS
    } else {
        ALL_TABS.get(3..).unwrap_or_default()
    }
}

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
#[derive(Clone, PartialEq, Eq, Debug)]
enum Change {
    /// Done or Delete took it out.
    Removed,
    /// Move filed it here (a tab, a project).
    Moved(&'static str, Option<&'static str>),
    /// Capture wrote it.
    Added,
    /// Its notes changed to these (the item kept is as it was before).
    Noted(Vec<String>),
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
    /// What the last Undo put back, said until the next change or move.
    pub undid: Signal<Option<String>>,
    /// What Undo put back, for Redo (⇧⌘Z); a new change lets it go.
    pub redone: Signal<Vec<Undone>>,
    /// Whether the last change is still said over its list (until the
    /// lab moves to another list).
    pub notice: Signal<bool>,
    /// Whether a field on the page has text in it: ⌘Z is the field's then.
    pub typing: Signal<bool>,
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
        self.typing.set(false);
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
        self.typing.set(false);
        self.undid.set(None);
        self.route.set(to);
        self.select_first();
    }

    /// A list just shown: its first row selected, so the keys and the
    /// details have something from the start.
    fn select_first(mut self) {
        let first = self.shown_ids().first().copied();
        let on_list = matches!(*self.route.peek(), Route::Tab(t) if t != "vault");
        self.selected.set(first.filter(|_| on_list));
    }

    /// Keeps `u` for Undo and says it over the list.
    fn did(mut self, u: Undone) {
        let mut undone = self.undone.write();
        undone.push(u);
        let over = undone.len().saturating_sub(UNDO_STEPS);
        undone.drain(..over);
        drop(undone);
        self.redone.write().clear();
        self.undid.set(None);
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
        self.select_first();
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
        self.typing.set(false);
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
            match &u.change {
                Change::Removed => lists.restore(u.at, u.item.clone()),
                Change::Noted(_) => lists.set_notes(id, u.item.notes.clone()),
                Change::Moved(..) => {
                    let _ = lists.remove(id);
                    lists.restore(u.at, u.item.clone());
                }
                Change::Added => {
                    let _ = lists.remove(id);
                }
            }
            drop(lists);
            let back = matches!(u.change, Change::Removed | Change::Moved(..));
            let added = u.change == Change::Added;
            self.undid
                .set(Some(format!("Undid {} on “{}”", u.action, u.item.name)));
            self.redone.write().push(u);
            if back || added {
                self.fresh.set(back.then_some(id));
                self.selected.set(back.then_some(id));
            }
            // The notice now says what Undo did, with the way to do it again.
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
        match u.change.clone() {
            Change::Noted(notes) => {
                self.lists.write().set_notes(u.item.id, notes);
                self.did(u);
            }
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

    /// Adds `text` to `id`'s notes, for Undo to take back; whether it
    /// did (not for a blank one).
    #[must_use = "false when nothing was added"]
    pub fn note(self, id: usize, text: &str) -> bool {
        self.change_notes(id, "Added a note", "Note", |lists| lists.note(id, text))
    }

    /// Takes `id`'s note `at` away, for Undo to put back.
    pub fn unnote(self, id: usize, at: usize) {
        let _ = self.change_notes(id, "Removed a note", "Note Removal", |lists| {
            lists.unnote(id, at)
        });
    }

    fn change_notes(
        mut self,
        id: usize,
        verb: &'static str,
        action: &'static str,
        change: impl FnOnce(&mut items::Lists) -> bool,
    ) -> bool {
        let before = self.lists.peek().get(id).cloned();
        let at = self.lists.peek().position(id).unwrap_or_default();
        let Some(before) = before else {
            return false;
        };
        if !change(&mut self.lists.write()) {
            return false;
        }
        let after = self
            .lists
            .peek()
            .get(id)
            .map(|i| i.notes.clone())
            .unwrap_or_default();
        self.did(Undone {
            verb,
            action,
            said: format!("{verb} to “{}”", before.name),
            at,
            item: before,
            change: Change::Noted(after),
        });
        true
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
        self.typing.set(false);
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
    here: &str,
) -> architect_commands::Command {
    let c = match change {
        Some(u) if !typing => {
            // A change made in another tab says where: "Undo Done in Inbox".
            let place = if u.item.tab == here {
                String::new()
            } else {
                format!(" in {}", pages::title(u.item.tab))
            };
            architect_commands::Command::new(id, format!("{verb} {}{place}", u.action))
                .detail(format!("{detail} “{}”", u.item.name))
        }
        Some(_) => architect_commands::Command::new(id, verb).disabled("this page's field has it"),
        None => architect_commands::Command::new(id, verb).disabled(none),
    };
    c.group("Edit")
}

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

/// The lab's own layout (`layouts/main.json`, written read-only on every
/// start): the shell as it is, nothing over it.
const MAIN_LAYOUT: &str = r#"{
  "name": "Architect",
  "chrome": {}
}
"#;

/// The lab's root. The host publishes the window as a
/// `Signal<shell::Screen>` ([`use_window_screen`]) above it.
///
/// Headless (`apps/sandbox/media`, the guide scenarios' runner) it has no
/// window: what needs one (the window's keys, its title bar, the
/// platform's menu bar) is left out, and the host's Action List, given in
/// context, is the lab's.
#[component]
pub fn LabApp() -> Element {
    let screen = use_context::<Signal<shell::Screen>>();
    // Fixed for the component's life, so the hooks below it are called the
    // same way every render.
    let windowed = use_hook(|| {
        try_consume_context::<std::sync::Arc<dyn dioxus_native::winit::window::Window>>().is_some()
    });
    // A phone's bar becomes the editing bar while a field has the focus:
    // the document's text focus, told to the shell.
    use_hook(|| dioxus_native::on_text_focus_change(shell::set_typing));
    let list = use_hook(|| {
        try_consume_context::<std::sync::Arc<architect::action::ActionList>>()
            .unwrap_or_else(|| std::sync::Arc::new(architect::action::ActionList::new()))
    });
    use_context_provider(|| list.clone());
    // `APP_STATUS`: what is on screen and what is wrong, from every part
    // of the lab that reports it (the workspace, the theme, extensions).
    use_hook(|| {
        let _ = architect::action::app_status::register(&list);
    });
    // Themes: one store for the app, its THEME_* actions on the list.
    let files = use_hook(|| windowed || try_consume_context::<LabFiles>().is_some());
    let (theme, appearance) = theme::use_lab_theme(windowed, files, &list);
    // An agent's Delete waits for the person's yes, in the sheet below.
    use_hook(|| confirm::require(&list));
    // Sign-in (#188): the person's accounts, ACCOUNT_* on the list.
    let accounts = accounts::use_app_accounts(&list, files);
    // The lab's vault: a folder of markdown, its VAULT_* actions (only
    // with files: a film's run keeps nothing).
    use_hook(|| {
        if files {
            notes::register(&list);
            // Projects and asset libraries (#347), over the files agent.
            files::register(&list);
        }
    });
    // Capture (#352): the composer, the inbox over the vault, the capture
    // window on a system-wide key, CAPTURE_* and INBOX_* on the list.
    let capture_ui = capture::use_app_capture(&list, files, windowed);
    // An agent's UI (A2UI): its surfaces on a document of their own, the
    // `A2UI_*` actions on the list, the Vault's "Agent UI" view drawing
    // them. Every write a surface's button makes asks first (the sheet).
    let surfaces = use_hook(|| {
        architect_a2ui::Surfaces::new(crdt::CrdtDoc::ephemeral(), architect_a2ui::store::ROOT)
    });
    use_hook(|| {
        let _ = architect_a2ui::actions::register(&list, &surfaces);
        embedded::register(&list, &surfaces);
    });
    architect_a2ui::ui::use_a2ui(surfaces, (*list).clone());
    // Settings: the lab's sections, kept on this device — in a window.
    // Headless, the defaults, kept nowhere: a scenario neither reads nor
    // changes the person's own.
    let store = use_hook(|| {
        windowed
            .then(|| {
                architect_settings::DeviceValues::open(crate::distribution().folder_name()).ok()
            })
            .flatten()
            .map(std::rc::Rc::new)
    });
    // The person's (synced) values, such as Navigator's: kept beside the
    // device's until a synced store holds them.
    let synced_store = use_hook(|| {
        windowed
            .then(|| {
                architect_settings::DeviceValues::open_named(
                    crate::distribution().folder_name(),
                    "settings-synced",
                )
                .ok()
            })
            .flatten()
            .map(std::rc::Rc::new)
    });
    let (save, save_synced) = (store.clone(), synced_store.clone());
    let settings = architect_settings::use_settings_provider(
        settings::registry,
        || {
            let mut device = store.as_ref().map(|s| s.load()).unwrap_or_default();
            let mut synced = synced_store.as_ref().map(|s| s.load()).unwrap_or_default();
            if settings::adopt_synced(&mut device, &mut synced) {
                if let (Some(d), Some(p)) = (store.as_ref(), synced_store.as_ref()) {
                    let _ = (d.save(&device), p.save(&synced));
                }
            }
            architect_settings::ScopedValues {
                device,
                synced,
                ..Default::default()
            }
        },
        Some(Callback::new(
            move |change: architect_settings::SettingsChange| {
                let to = match change.scope {
                    architect_settings::Scope::Device => save.as_ref(),
                    architect_settings::Scope::Synced => save_synced.as_ref(),
                    architect_settings::Scope::Project => None,
                };
                if let Some(s) = to {
                    let _ = s.save(&change.values);
                }
            },
        )),
    );
    // Appearance: architect's section, the theme store's layers.
    architect_settings::use_appearance_section();
    let lab = use_context_provider(|| {
        // The sandbox's made-up lists; the Architect app's are empty (its
        // tabs are the Vault's).
        let sandbox = crate::distribution().is_sandbox();
        let lists = if sandbox {
            items::Lists::sample()
        } else {
            items::Lists::default()
        };
        // The Inbox's first row, selected from the start.
        let first = lists.of("inbox", None, false).first().map(|i| i.id);
        Lab {
            route: Signal::new(Route::Tab(if sandbox { "inbox" } else { "vault" })),
            back: Signal::new(Vec::new()),
            style: Signal::new(ShellStyle::FloatingPill),
            right: Signal::new(true),
            status: Signal::new(shell::StatusStyle::Bar),
            lists: Signal::new(lists),
            project: Signal::new(None),
            oldest: Signal::new(false),
            fresh: Signal::new(None),
            undone: Signal::new(Vec::new()),
            redone: Signal::new(Vec::new()),
            notice: Signal::new(false),
            completing: Signal::new(Vec::new()),
            typing: Signal::new(false),
            undid: Signal::new(None),
            selected: Signal::new(first),
            settings: Some(settings),
        }
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
        // On, motion is reduced; off, the OS's preference holds.
        let reduce = architect_settings::use_setting("appearance.reduce_motion")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        shell::motion::set_reduced_motion(reduce.then_some(true));
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
    // Extensions, and what they add to the window: the shell's chrome
    // (above the shell), a pane's context menu, the menus, their views.
    let ext_ui = extensions::use_lab_extensions(metronome);
    use_context_provider(|| ext_ui);
    // The interface as files (layouts/<name>.json, keys.json under the
    // lab's folder), live: the layout shown fills the shell's slots over
    // what extensions fill, sets the window's look and arrangement.
    let home = use_hook(|| {
        let dir = app_dir_or_temp();
        let home = architect_workspace::home::Home::open(dir, &[("main", MAIN_LAYOUT)]);
        let _ = architect_workspace::home::actions::register(&list, &home);
        // Layouts and keys.json in the extensions manager.
        if let Some(manager) = try_consume_context::<architect_extensions::manager::Manager>() {
            manager.register(architect_extensions::manager::kinds::LayoutKind::new(
                home.clone(),
            ));
        }
        home
    });
    let home_rev = architect_workspace::home::ui::use_home(home.clone());
    architect_workspace::home::ui::use_home_apply(
        home.clone(),
        home_rev,
        architect_theme::try_use_theme().map(|t| t.store()),
        list.clone(),
    );
    let mut chrome = shell::use_chrome();
    let contributed = use_memo(move || ext_ui.chrome());
    let laid_out = {
        let home = home;
        use_memo(move || {
            let _ = home_rev();
            let layout = home.current().unwrap_or_default();
            let owner = format!("layout:{}", home.active());
            architect_workspace::home::ui::chrome_of(&layout, |view| {
                let (extension, v) = view.split_once('/')?;
                let (e, v) = (extension.to_owned(), v.to_owned());
                Some(shell::Filler::new(owner.clone(), view, move || {
                    rsx! { architect_extensions::interface::ExtensionView { ui: ext_ui, extension: e.clone(), view: v.clone(), fill: false } }
                }))
            })
        })
    };
    use_effect(move || chrome.set(laid_out().over(&contributed())));
    // Every command (a view's own too) and the palette's state, on the
    // lab's list; the lab's own state beside them.
    architect_commands::use_command_actions(list.clone());
    guide::use_lab_actions(&list, &lab);
    // Guided tours, from ⌘K (Help) or the Action List (`GUIDE_TOUR_START`):
    // the Architect tour (its place kept in a window's lab), and two of the
    // lab's scenarios.
    let tours = architect_guide::tour::use_tours(list.clone(), use_hook(|| guide::tours(windowed)));
    // Each running extension's tours (`[[tours]]`) beside them, TOURS_LIST
    // and TOUR_START, the manager's "Take the tour", and the invitation
    // after an install.
    guide::use_extension_tours(&list, &tours, ext_ui, windowed);
    // "Take the tour" goes on where the person left it, from the start
    // once they finished it; a window's lab offers it on its first start.
    let take_tour = move || {
        let at = match windowed.then(guide::tour_place) {
            Some(guide::TourPlace::At(step)) => Some(step),
            _ => None,
        };
        let _ = tours.start_at(guide::ARCHITECT_TOUR, at.as_deref());
    };
    architect_commands::use_command(
        architect_commands::Command::new("LAB_TAKE_TOUR", "Take the tour").group("Help"),
        take_tour,
    );
    // The Architect app's first run: no vault yet, so the Vault tab asks
    // for one (`first_run`), and the tour is offered once it is open.
    let mut needs_vault =
        use_signal(move || windowed && !crate::distribution().is_sandbox() && !notes::has_vault());
    use_hook(move || {
        if windowed && !needs_vault() && guide::tour_place() == guide::TourPlace::Never {
            take_tour();
        }
    });
    let vault_opened = use_callback(move |()| {
        needs_vault.set(false);
        if guide::tour_place() == guide::TourPlace::Never {
            take_tour();
        }
    });
    for (id, title) in use_hook(move || tours.offered())
        .into_iter()
        .filter(|(id, _)| id != guide::ARCHITECT_TOUR)
    {
        architect_commands::use_command(
            architect_commands::Command::new(
                format!("LAB_TOUR_{}", id.to_uppercase().replace('-', "_")),
                format!("Tour: {title}"),
            )
            .group("Help"),
            move || {
                let _ = tours.start(&id);
            },
        );
    }
    for (n, &(id, label, to)) in tabs().iter().enumerate() {
        // ⌃⌘1–4: ⌘1–9 are the Vault's screensets, and ⌥ changes the digit.
        architect_commands::use_command(
            architect_commands::Command::new(id, label)
                .group("Go")
                .shortcut(format!("⌃⌘{}", n.saturating_add(1))),
            move || lab.jump(Route::Tab(to)),
        );
    }
    // Keyflow's "New Chart" is a root command, so ⌘K has it from every view.
    #[cfg(feature = "keyflow-lab")]
    {
        let chart_commands = commands.clone();
        architect_commands::use_command(
            architect_commands::Command::new("LAB_NEW_CHART", "New Chart").group("Keyflow"),
            move || {
                lab.jump(Route::Tab("vault"));
                let _ = chart_commands.run_with("WORKSPACES_LOAD", r#"{"name":"Chart"}"#);
            },
        );
    }
    architect_commands::use_command(
        architect_commands::Command::new("LAB_TOGGLE_DETAILS", "Details")
            .group("View")
            .checked((lab.right)()),
        move || lab.put_bool("workspace.details", !(lab.right)()),
    );
    architect_commands::use_command(
        {
            let c = architect_commands::Command::new("LAB_CAPTURE", "Capture")
                .group("Inbox")
                .shortcut("⇧⌘N");
            if crate::distribution().is_sandbox() || capture_ui.is_some() {
                c
            } else {
                c.disabled("capture needs a vault")
            }
        },
        // The Architect app's composer; the sandbox's capture page.
        move || match capture_ui {
            Some(c) => c.compose(""),
            None => lab.capture("inbox"),
        },
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
        {
            let c = architect_commands::Command::new("LAB_GO_BACK", "Go back")
                .group("Go")
                .shortcut("⌘[");
            if lab.can_go_back() {
                c
            } else {
                c.disabled("nowhere to go back to")
            }
        },
        move || lab.go_back(),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_OPEN_SELECTED", "Open the selected item")
            .group("Go")
            .shortcut("⌘↓"),
        move || lab.open_selected(),
    );
    architect_commands::use_command(
        {
            let c = architect_commands::Command::new("LAB_DELETE", "Delete")
                .group("Edit")
                .shortcut("⌘⌫")
                .destructive();
            // A field with text has ⌘⌫ (delete to the line's start).
            if lab.target().is_some() && !(lab.typing)() {
                c
            } else {
                c.disabled("select an item first")
            }
        },
        move || {
            if let Some(id) = lab.target() {
                lab.remove(id, "Deleted");
            }
        },
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
    let typing = (lab.typing)();
    let here_tab = (lab.route)().tab();
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
            here_tab,
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
            here_tab,
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
    // "Show every command" (⌘K) and "Open…" (⌘O) are the Command
    // Palette extension's (COMMAND_PALETTE_OPEN, _OPEN_ANYTHING; #381):
    // turned off, they are gone from the menus and the keys.
    // The menu bar: a view of the commands, labels and keys and all.
    let menus = architect_commands::MenuBar::standard(crate::distribution().name())
        .menu("File", |m| {
            let m = m.command("LAB_CAPTURE");
            // The inbox deck and the capture window, where capture is.
            let m = if capture_ui.is_some() {
                m.commands([
                    architect_capture::actions::ids::INBOX_OPEN,
                    capture::WINDOW_COMMAND,
                ])
            } else {
                m
            };
            let m = m
                .command("COMMAND_PALETTE_OPEN_ANYTHING")
                .separator()
                .commands([
                    "WORKSPACE_CLOSE_TAB",
                    "WORKSPACE_REOPEN_TAB",
                    "WORKSPACE_CLOSE_OTHER_TABS",
                ]);
            // The account's, where the app has accounts.
            if accounts.is_some() {
                m.separator().commands([
                    architect_accounts::actions::ids::SIGN_IN,
                    architect_accounts::actions::ids::SWITCH,
                    architect_accounts::actions::ids::SIGN_OUT,
                ])
            } else {
                m
            }
        })
        .menu("Edit", |m| {
            m.instead(architect_commands::Standard::Undo, "LAB_UNDO")
                .instead(architect_commands::Standard::Redo, "LAB_REDO")
                .separator()
                .commands(["LAB_DONE", "LAB_MOVE", "LAB_DELETE"])
        })
        .menu("View", |m| {
            m.command("LAB_TOGGLE_DETAILS")
                .separator()
                .commands(["LAB_THEME_CHOOSE", "LAB_THEME_SCHEME"])
                .separator()
                .group("Panes")
                .separator()
                .group("Docks")
                .separator()
                .command("WORKSPACES_MANAGE")
        })
        .menu("Go", |m| {
            // The tabs, the one showing ticked: a live section, read again
            // whenever the bar is drawn.
            m.section("Tabs", move || {
                let on = (lab.route)().tab();
                tabs()
                    .iter()
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
        .menu("Help", |m| m.commands(["LAB_HELP", "COMMAND_PALETTE_OPEN"]))
        // What extensions add to each menu.
        .contributed(move |menu| ext_ui.menu_rows(menu));
    let route = (lab.route)();
    let inbox_count = lab.lists.read().count("inbox");
    let all_tabs = [
        {
            let inbox = shell::Tab::new("inbox", "Inbox", icon!(Inbox));
            // A clear Inbox shows no count.
            if inbox_count > 0 {
                inbox.badge(inbox_count)
            } else {
                inbox
            }
        },
        shell::Tab::new("tasks", "Tasks", icon!(ListTodo)),
        shell::Tab::new("schedule", "Schedule", icon!(CalendarDays)),
        shell::Tab::new("vault", "Vault", icon!(NotebookText)),
        // The Vault's "Embedded views" screenset, one tap away: on_tab
        // below loads it into the Vault.
        shell::Tab::new("embedded", "Embedded", icon!(LayoutGrid)),
    ];
    let tabs: Vec<shell::Tab> = if crate::distribution().is_sandbox() {
        all_tabs.into()
    } else {
        all_tabs.into_iter().filter(|t| t.id == "vault").collect()
    };
    // Which workspace the lab shows; `None` is all of them.
    // The person's organizations (the identity server's, here made up):
    // the switcher, and what each role may do.
    let caps = Capabilities::new()
        .allow(&Role::Viewer, ["notes.read"])
        .allow(&Role::Member, ["notes.write"])
        .allow(&Role::Admin, ["members.invite"])
        .allow(&Role::Owner, ["org.delete"])
        .inherit();
    // The Architect app's are the active account's (#349): signed out,
    // the person's own on this device. The sandbox's are made up.
    let orgs = match accounts {
        Some(accounts) => accounts::use_account_orgs(accounts, caps),
        None => architect_orgs::use_orgs_provider(
            || {
                if !crate::distribution().is_sandbox() {
                    return Orgs::new(vec![architect_accounts::device_org()]);
                }
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
            caps,
            None,
        ),
    };
    // The org's default look, under the person's: its settings, when the
    // window's org changes.
    {
        let org = orgs.current().map(|o| o.id.0);
        let mut shown = use_signal(|| None::<Option<String>>);
        if shown.peek().as_ref() != Some(&org) {
            let defaults = theme::org_settings(org.as_deref());
            if let Some(kept) = appearance.as_ref() {
                kept.set_org(defaults.clone());
            }
            settings.set_org(defaults);
            shown.set(Some(org));
        }
    }
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
    let window_chrome = if windowed {
        screen::use_window_chrome()
    } else {
        None
    };
    // Task's capture button (#445): a tap captures; press and slide up
    // starts a timer and opens its page; the captures waiting ride it.
    let to_process = architect_capture::ui::use_to_process(capture_ui);
    let capture = shell::Action::new("Capture", icon!(Feather), move |()| match capture_ui {
        Some(c) => c.compose(""),
        None => lab.capture("inbox"),
    })
    .on_swipe_up("Start a timer", icon!(Timer), move |()| {
        let _ = commands.run_with("TIME_START", "{}");
        let _ = commands.run_with("TIME_OPEN", "{}");
    })
    .with_badge(to_process);
    // The sidebar's foot, above the switcher and the account (bottom to
    // top: Account, Workspace, Settings, Theme).
    let foot = vec![
        shell::Action::new("Theme", icon!(Palette), move |()| {
            let _ = commands.run_with("THEME_PICKER_CHOOSE", "{}");
        }),
        shell::Action::new("Settings", icon!(Settings2), move |()| settings.show(None)),
    ];
    let mut status = vec![
        shell::StatusItem::new("commands", "⌘K Commands")
            .hint("Every command")
            .on_press(move |()| commands.show(architect_commands::Overlay::Palette)),
        shell::StatusItem::new("sync", "Synced")
            .icon(icon!(CheckCheck))
            .hint("Every change is saved and synced"),
        shell::StatusItem::new("people", "3 online").icon(icon!(Users)),
    ];
    // The made-up sync and people are the sandbox's: the Architect app
    // has neither until sign-in and sync (#188, #343).
    if !crate::distribution().is_sandbox() {
        status.retain(|s| s.id == "commands");
    }
    // ⌘K is the Command Palette extension's: off, no hint for it.
    if commands
        .palette_owner(architect_commands::Overlay::Palette)
        .is_none()
    {
        status.retain(|s| s.id != "commands");
    }
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
                    "{} {} {} · {}",
                    u.verb,
                    if matches!(u.change, Change::Moved(..)) {
                        "from"
                    } else {
                        "in"
                    },
                    pages::title(u.item.tab),
                    // Where a field has ⌘Z, the item says only Undo.
                    if typing { "Undo" } else { "Undo ⌘Z" }
                ),
            )
            .icon(icon!(Undo2))
            .hint(format!("Put back “{}”", u.item.name))
            .on_press(move |()| lab.undo()),
        );
    }
    // What the last Undo did, with the way to do it again, where no
    // notice over a list says it.
    let undid = (lab.undid)();
    if let Some(said) = undid.filter(|_| !matches!(route, Route::Tab(_))) {
        status.retain(|s| s.id != "undo");
        status.insert(
            0,
            shell::StatusItem::new("undo", format!("{said} · Redo ⇧⌘Z"))
                .icon(icon!(Undo2))
                .on_press(move |()| lab.redo()),
        );
    }
    // The Account item's own rows, under the switcher's.
    let account_menu = shell::Menu::new()
        .item("Settings", icon!(Settings2), move |()| settings.show(None))
        .item("Keys and help", icon!(Keyboard), move |()| {
            lab.help();
        });
    // The signed-in person and the switcher (#188); the sandbox's made-up
    // person, or no one where a run keeps nothing.
    let account = match accounts {
        Some(accounts) => accounts.shell_account(account_menu),
        None if crate::distribution().is_sandbox() => shell::Account::new("Cody Wright")
            .detail("cody@fasttrackstudio.app")
            .presence("#22c55e")
            .menu(account_menu),
        None => shell::Account::new("Not signed in").menu(account_menu),
    };
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; overflow: hidden;",
            document::Style { {theme::page_css(&theme.resolved().read())} }
            // The window's chords (⌘K, ⌘W, …), whatever has the focus.
            if windowed {
                theme::SystemScheme {}
                architect_commands::WindowKeys {}
                ListKeys {}
                // The Dock icon, its badge and menu, the menu-bar item,
                // background mode and opening at login.
                presence::LabPresence { presence: presence::declare(lab, inbox_count, capture_ui.is_some()), inbox: inbox_count }
            }
            // A column: the in-window menu bar (where there is one) takes
            // its height off the top, the shell the rest. Under
            // architect-ui's root, its CSS variables the theme's (the
            // clicker's `clicker.*` tokens too, for its process view).
            architect_ui::theme::ThemeStoreProvider { css_namespaces: vec!["clicker".to_owned()],
            div { style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; flex-direction: column;",
                NativeMenus { bar: menus, windowed }
                div { style: "position: relative; flex: 1 1 0; min-height: 0;",
                    AppShell {
                        screen: screen(),
                        style: (lab.style)(),
                        title: crate::distribution().name(),
                        tabs,
                        // The help is over a tab, not one of them.
                        current: (!matches!(route, Route::Help(_))).then(|| route.tab()),
                        on_tab: move |id: &'static str| {
                            if ext_ui.is_tab(id) {
                                lab.jump(Route::Tab(id));
                            } else if id == "embedded" {
                                lab.jump(Route::Tab("vault"));
                                let _ = commands.run_with(
                                    "WORKSPACES_LOAD",
                                    r#"{"name":"Embedded views"}"#,
                                );
                            } else {
                                lab.jump(Route::Tab(id));
                            }
                        },
                        separate: (crate::distribution().is_sandbox() || capture_ui.is_some()).then_some(capture),
                        brand: rsx! { Brand {} },
                        account,
                        workspaces,
                        foot,
                        // The sandbox's made-up projects; the Architect app's
                        // sidebar is the Vault's navigator.
                        left: crate::distribution().is_sandbox().then(|| rsx! { pages::LeftSide {} }),
                        // The details are the lists': the Vault's panes are its own,
                        // an item's page is its details, the help is the help.
                        right: if (lab.right)() && matches!(route, Route::Tab(t) if t != "vault" && !ext_ui.is_tab(t)) { Some(rsx! { pages::RightSide {} }) } else { None },
                        on_pull: move |()| lab.jump(Route::Tab("vault")),
                        window: window_chrome,
                        status,
                        status_style: (lab.status)(),
                        // A phone's action bar (a page adds its own: the
                        // Vault, its tabs).
                        actions: vec![
                            shell::BarAction::new("LAB_GO_BACK", "Go back", icon!(ArrowLeft)),
                            shell::BarAction::new("COMMAND_PALETTE_OPEN_ANYTHING", "Open…", icon!(Search)),
                            shell::BarAction::new("LAB_UNDO", "Undo", icon!(Undo2)),
                            shell::BarAction::new("COMMAND_PALETTE_OPEN", "Commands", icon!(CommandIcon)),
                            shell::BarAction::new("APP_SETTINGS", "Settings", icon!(Settings2)),
                            shell::BarAction::new("LAB_HELP", "Help", icon!(CircleQuestionMark)),
                        ],
                        // Sidebar: Rail / One column / Two columns, on ⌘K.
                        SidebarCommands {}
                        match route {
                            Route::Tab("vault") if needs_vault() => rsx! { first_run::FirstRun { on_open: vault_opened } },
                            Route::Tab("vault") => rsx! { vault::VaultPage {} },
                            Route::Tab(tab) if ext_ui.is_tab(tab) => rsx! { extensions::ExtensionTab { key: "{tab}", ui: ext_ui, id: tab } },
                            Route::Tab(tab) => rsx! { pages::ListPage { key: "{tab}", tab } },
                            Route::Item(tab, id) => rsx! { pages::ItemPage { tab, id } },
                            Route::Capture(tab) => rsx! { pages::CapturePage { key: "{tab}", tab } },
                            Route::Help(_) => rsx! { pages::HelpPage {} },
                        }
                    }
                }
            }
            }
            architect_commands::CommandLayer {}
            architect_commands::ConfirmSheet {}
            architect_accounts::ui::AccountSheet {}
            architect_settings::SettingsPage {}
            architect_extensions::interface::ExtensionOverlays { ui: ext_ui }
            if let Some(capture) = capture_ui {
                architect_capture::ui::CaptureLayer { capture }
            }
            architect_guide::tour::TourLayer {}
            screen::DeviceChrome {}
        }
    }
}

/// The shell's sidebar layouts on the palette (inside the shell, whose
/// layout they set).
#[component]
fn SidebarCommands() -> Element {
    architect_commands::use_sidebar_commands();
    rsx! {}
}

/// The app's brand, T3 Code's way: a mark and a wordmark (two-tone in the
/// sandbox). A placeholder until the brand's own assets come.
#[component]
fn Brand() -> Element {
    let t = shell::use_shell().theme;
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; font-family: {t.font};",
            span { style: "width: 26px; height: 26px; border-radius: 7px; background: {t.ink}; display: flex; \
                           align-items: center; justify-content: center; color: {t.desk}; font-size: 14px; font-weight: 800;",
                "A"
            }
            span { style: "font-size: 19px; font-weight: 750; letter-spacing: -0.03em; color: {t.ink};", "Architect" }
            if crate::distribution().is_sandbox() {
                span { style: "font-size: 19px; font-weight: 500; letter-spacing: -0.03em; color: {t.ink_3};", "Sandbox" }
            }
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
fn NativeMenus(bar: architect_commands::MenuBar, windowed: bool) -> Element {
    if !windowed {
        return rsx! { architect_commands::InWindowMenus { bar } };
    }
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "ios"))]
    return rsx! {
        architect_commands::NativeMenus { bar: bar.clone() }
        architect_commands::InWindowMenus { bar }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "ios")))]
    rsx! { architect_commands::InWindowMenus { bar } }
}

#[cfg(test)]
mod layers {
    /// The app's root stacks extensions' overlays and the tour as
    /// siblings: an extension's prompt, dialog or menu is over the tour's
    /// card, so a step that brings one up can be answered (#314).
    #[test]
    fn prompts_are_over_the_tour() {
        const {
            assert!(
                architect_extensions::host_ui_view::PROMPT_LAYER
                    > architect_guide::tour::TOUR_LAYER
            );
        }
    }
}
