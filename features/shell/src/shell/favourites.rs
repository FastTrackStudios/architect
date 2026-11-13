//! A phone's bottom slots as favourites: the four tabs and the round
//! button beside them are not fixed but follow what is open.
//!
//! An app gives its default set (the Architect app's: Inbox, Vault,
//! Project, Schedule and Capture) and the sets the types it knows declare;
//! [`resolve_favourites`] picks the one for the open note's type, and the
//! app hands its tabs and its primary action to [`AppShell`] (`tabs`,
//! `separate`). Leaving the type brings the default back. The sidebar
//! (the navigator, in a phone's left drawer) is never a slot: it stays the
//! way to everything, the default set's places among them.
//!
//! The declared sets are the extensions' `[[favourites]]` (a note type's
//! tabs, each `{id, label, icon, target}`, and one primary action; #604's
//! `architect_favourites::TypeFavourites`), turned into [`Declared`] by the
//! app, which resolves each target. TODO(#607): read them from the loaded
//! extensions once #604 lands; until then an app declares none and the
//! default set shows.
//!
//! [`AppShell`]: super::AppShell

use super::{Action, Tab};

/// How many tabs a phone's bar holds; a set's tabs past it are dropped.
pub const SLOTS: usize = 4;

/// A set of favourites: up to [`SLOTS`] tabs and the primary action (the
/// round button beside them).
#[derive(Clone, PartialEq, Default)]
pub struct Favourites {
    pub tabs: Vec<Tab>,
    pub primary: Option<Action>,
}

impl Favourites {
    #[must_use]
    pub const fn new(tabs: Vec<Tab>, primary: Option<Action>) -> Self {
        Self { tabs, primary }
    }

    /// The tabs the bar shows: the first [`SLOTS`].
    #[must_use]
    pub fn slots(&self) -> &[Tab] {
        self.tabs
            .get(..SLOTS.min(self.tabs.len()))
            .unwrap_or_default()
    }

    /// The tabs' ids, in order.
    #[must_use]
    pub fn ids(&self) -> Vec<&'static str> {
        self.slots().iter().map(|t| t.id).collect()
    }
}

/// The favourites a note type declares.
#[derive(Clone, PartialEq)]
pub struct Declared {
    /// The note type (`setlist`): a note whose `type` is it.
    pub note_type: String,
    pub favourites: Favourites,
}

impl Declared {
    #[must_use]
    pub fn new(note_type: impl Into<String>, favourites: Favourites) -> Self {
        Self {
            note_type: note_type.into(),
            favourites,
        }
    }
}

/// The favourites for what is open: the first set declared for the open
/// note's type (`open`), else the default. A declared set with no tabs is
/// none (its type keeps the default).
#[must_use]
pub fn resolve_favourites<'a>(
    default: &'a Favourites,
    declared: &'a [Declared],
    open: Option<&str>,
) -> &'a Favourites {
    open.and_then(|ty| {
        declared
            .iter()
            .find(|d| d.note_type == ty && !d.favourites.tabs.is_empty())
    })
    .map_or(default, |d| &d.favourites)
}

#[cfg(test)]
mod tests {
    // `icon!` casts its closure to the fn pointer, as an app's does.
    #![allow(clippy::as_conversions)]

    // Named, so the prelude's own `Action` does not shadow the shell's.
    use super::{Action, Declared, Favourites, Tab, resolve_favourites};
    use crate::icon;
    use dioxus::prelude::*;
    use lucide_dioxus::{CalendarDays, Feather, FolderKanban, Inbox, ListMusic, NotebookText};

    fn default() -> Favourites {
        Favourites::new(
            vec![
                Tab::new("inbox", "Inbox", icon!(Inbox)),
                Tab::new("vault", "Vault", icon!(NotebookText)),
                Tab::new("project", "Project", icon!(FolderKanban)),
                Tab::new("schedule", "Schedule", icon!(CalendarDays)),
            ],
            Some(Action::new("Capture", icon!(Feather), |()| {})),
        )
    }

    fn setlist() -> Declared {
        Declared::new(
            "setlist",
            Favourites::new(
                vec![
                    Tab::new("setlist", "Setlist", icon!(ListMusic)),
                    Tab::new("charts", "Charts", icon!(ListMusic)),
                    Tab::new("lyrics", "Lyrics", icon!(ListMusic)),
                    Tab::new("arrangement", "Arrangement", icon!(ListMusic)),
                    Tab::new("fifth", "Fifth", icon!(ListMusic)),
                ],
                Some(Action::new("Play", icon!(ListMusic), |()| {})),
            ),
        )
    }

    /// `f` where a button's callback can be made (a Dioxus runtime).
    fn in_runtime(f: impl FnOnce()) {
        let dom = VirtualDom::new(|| rsx! {});
        dom.in_scope(ScopeId::ROOT, f);
    }

    #[test]
    fn the_open_type_picks_the_slots_and_leaving_it_brings_the_default_back() {
        in_runtime(open_type_picks_the_slots);
    }

    fn open_type_picks_the_slots() {
        let (default, declared) = (default(), vec![setlist()]);
        // A plain note, or nothing open: the default four and Capture.
        let plain = resolve_favourites(&default, &declared, Some("note"));
        assert_eq!(plain.ids(), ["inbox", "vault", "project", "schedule"]);
        assert_eq!(
            plain.primary.as_ref().map(|a| a.label.as_str()),
            Some("Capture")
        );
        assert_eq!(
            resolve_favourites(&default, &declared, None).ids(),
            plain.ids()
        );
        // A setlist: its own four (the fifth past the bar) and its action.
        let set = resolve_favourites(&default, &declared, Some("setlist"));
        assert_eq!(set.ids(), ["setlist", "charts", "lyrics", "arrangement"]);
        assert_eq!(set.primary.as_ref().map(|a| a.label.as_str()), Some("Play"));
        // Left again: the default.
        assert_eq!(
            resolve_favourites(&default, &declared, Some("note")).ids(),
            ["inbox", "vault", "project", "schedule"]
        );
    }

    #[test]
    fn a_type_that_declares_no_tabs_keeps_the_default() {
        in_runtime(no_tabs_keeps_the_default);
    }

    fn no_tabs_keeps_the_default() {
        let default = default();
        let declared = vec![Declared::new("empty", Favourites::default())];
        assert_eq!(
            resolve_favourites(&default, &declared, Some("empty")).ids(),
            ["inbox", "vault", "project", "schedule"]
        );
    }
}
