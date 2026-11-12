//! The Architect app's bottom slots on a phone, as favourites
//! (`architect_shell::Favourites`): Inbox, Vault, Project and Schedule,
//! and Capture as the round button, unless the open note's type declares
//! its own (`architect_shell::resolve_favourites`; TODO(#607): the types'
//! declarations, #604's `architect_favourites::TypeFavourites`, and the
//! open note's type).
//!
//! Each favourite opens its real surface ([`Surface`]) in the Vault's
//! workspace, the same one desk and phone alike: the inbox deck (the
//! captures to process, the round button's count), the projects list
//! (`PROJECTS_OPEN`), the calendar's day (`CALENDAR_OPEN`) and the vault's
//! notes (`VAULT_NOTES`, the last note shown). The sidebar's navigator
//! reaches all of them and everything else whatever the slots hold, and
//! the Go menu, ⌘K and ⌃⌘1–4 keep the default four ([`TABS`]).
//!
//! The sandbox keeps its made-up Inbox, Tasks, Schedule, Vault and
//! Embedded (`crate::ALL_TABS`).

use architect_shell::{Action, Declared, Favourites, Tab, icon};
use dioxus::prelude::*;
use lucide_dioxus::{CalendarDays, FolderKanban, Inbox, NotebookText};

/// The Architect app's tabs, in order: the command that goes to each (the
/// Go menu, ⌘K, ⌃⌘1–4), its label, its id.
pub const TABS: [(&str, &str, &str); 4] = [
    ("LAB_GO_INBOX", "Go to Inbox", "inbox"),
    ("LAB_GO_VAULT", "Go to Vault", "vault"),
    ("LAB_GO_PROJECT", "Go to Project", "project"),
    ("LAB_GO_SCHEDULE", "Go to Schedule", "schedule"),
];

/// The default set: the four, the Inbox counting the captures to process,
/// and `capture` as the round button.
#[must_use]
pub fn default_set(to_process: usize, capture: Option<Action>) -> Favourites {
    let inbox = Tab::new("inbox", "Inbox", icon!(Inbox));
    Favourites::new(
        vec![
            if to_process > 0 {
                inbox.badge(to_process)
            } else {
                inbox
            },
            Tab::new("vault", "Vault", icon!(NotebookText)),
            Tab::new("project", "Project", icon!(FolderKanban)),
            Tab::new("schedule", "Schedule", icon!(CalendarDays)),
        ],
        capture,
    )
}

/// The sets the note types declare. TODO(#607): every loaded extension's
/// `[[favourites]]` (#604), each target resolved here; none until then.
#[must_use]
pub const fn declared() -> Vec<Declared> {
    Vec::new()
}

/// What a favourite opens in the Vault's workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// An extension's action, with its arguments (JSON): the projects
    /// list, the calendar's day.
    Action(&'static str, &'static str),
    /// One of the app's commands: the vault's notes.
    Command(&'static str),
    /// The inbox deck (`architect_capture::ui::CaptureUi::inbox`, what
    /// `INBOX_OPEN` opens).
    Inbox,
}

/// What favourite `id` opens; `None` for an id that is not one of the four.
#[must_use]
pub const fn surface(id: &str) -> Option<Surface> {
    match id.as_bytes() {
        b"inbox" => Some(Surface::Inbox),
        b"vault" => Some(Surface::Command("VAULT_NOTES")),
        b"project" => Some(Surface::Action("PROJECTS_OPEN", "{}")),
        // A phone's day, not a desk's week.
        b"schedule" => Some(Surface::Action("CALENDAR_OPEN", r#"{"view":"day"}"#)),
        _ => None,
    }
}

/// What opens a favourite's surface: the app's commands (its actions
/// and commands) and the capture UI (the inbox deck).
#[derive(Clone, Copy)]
pub struct Opener {
    pub commands: architect_commands::Commands,
    pub capture: Option<architect_capture::ui::CaptureUi>,
}

impl Opener {
    /// Opens favourite `id`'s surface.
    pub fn open(&self, id: &str) {
        match surface(id) {
            Some(Surface::Action(action, args)) => {
                let _ = self.commands.run_with(action, args);
            }
            Some(Surface::Command(command)) => {
                let _ = self.commands.run(command);
            }
            Some(Surface::Inbox) => {
                if let Some(capture) = self.capture {
                    capture.inbox();
                }
            }
            None => {}
        }
    }
}

/// Whether tab `id` shows the Vault's workspace (with its surface in it):
/// the Architect app's four.
#[must_use]
pub fn in_vault(id: &str) -> bool {
    !crate::distribution().is_sandbox() && TABS.iter().any(|(_, _, t)| *t == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use architect_shell::resolve_favourites;

    #[test]
    fn the_default_four_in_the_owners_order() {
        let set = default_set(5, None);
        assert_eq!(set.ids(), ["inbox", "vault", "project", "schedule"]);
        assert_eq!(set.slots().first().and_then(|t| t.badge.clone()).as_deref(), Some("5"));
        assert!(default_set(0, None).slots().iter().all(|t| t.badge.is_none()));
        // Each opens a surface of its own.
        for (_, _, id) in TABS {
            assert!(surface(id).is_some(), "{id}");
        }
        assert_eq!(
            surface("schedule"),
            Some(Surface::Action("CALENDAR_OPEN", r#"{"view":"day"}"#))
        );
        assert_eq!(surface("inbox"), Some(Surface::Inbox));
    }

    #[test]
    fn a_declared_type_replaces_the_slots_and_the_four_stay_reachable() {
        let default = default_set(0, None);
        let declared = vec![Declared::new(
            "setlist",
            Favourites::new(vec![Tab::new("setlist", "Setlist", icon!(NotebookText))], None),
        )];
        let open = resolve_favourites(&default, &declared, Some("setlist"));
        assert_eq!(open.ids(), ["setlist"]);
        // The four are no slot now, and still a command each (the Go menu,
        // ⌘K, ⌃⌘1–4) and a surface the navigator's rows open.
        for (command, _, id) in TABS {
            assert!(!open.ids().contains(&id));
            assert!(command.starts_with("LAB_GO_") && surface(id).is_some());
        }
        // Leaving it: the default again.
        assert_eq!(
            resolve_favourites(&default, &declared, None).ids(),
            ["inbox", "vault", "project", "schedule"]
        );
    }
}
