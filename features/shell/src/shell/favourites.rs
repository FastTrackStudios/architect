//! A phone's bottom slots as favourites: the four tabs and the round
//! button beside them are not fixed but follow what is open.
//!
//! An app gives its default set (the Architect app's: Inbox, Vault,
//! Project, Schedule and Capture) and the sets the types it knows declare;
//! [`resolve_favourites`] picks the one for the open note's type, and the
//! app hands its tabs and its primary action to [`AppShell`] (`tabs`,
//! `separate`), and a tab's press to [`Favourites::press`] first. Leaving
//! the type brings the default back. The sidebar (the navigator, in a
//! phone's left drawer) is never a slot: it stays the way to everything,
//! the default set's places among them.
//!
//! The declared sets are the extensions' `[[favourites]]`
//! (`architect_favourites::TypeFavourites`: a note type's tabs, each
//! `{id, label, icon, target}`, and one primary action), each made a
//! [`Declared`] by [`declare`] with the app's own reading of each target:
//! what it opens there, or nothing, and a tab the app cannot open is left
//! out, never shown dead. The desktop and phone app (`apps/ui`) and the
//! app in a browser (`apps/web`) both take this one path.
//!
//! [`AppShell`]: super::AppShell

use architect_favourites::{Face, Target, TypeFavourites};
use dioxus::prelude::*;

use super::{Action, Tab, intern, named_icon};

/// How many tabs a phone's bar holds; a set's tabs past it are dropped.
pub const SLOTS: usize = 4;

/// A set of favourites: up to [`SLOTS`] tabs and the primary action.
///
/// The primary action is the round button beside the tabs; `presses` is
/// what a press on a tab of the set's own does (a declared set's: the app
/// routes the default set's ids itself).
#[derive(Clone, PartialEq, Default)]
pub struct Favourites {
    pub tabs: Vec<Tab>,
    pub primary: Option<Action>,
    /// A tab's press, by its id: the tabs [`declare`] made.
    pub presses: Vec<(&'static str, Callback<()>)>,
}

impl Favourites {
    #[must_use]
    pub const fn new(tabs: Vec<Tab>, primary: Option<Action>) -> Self {
        Self {
            tabs,
            primary,
            presses: Vec::new(),
        }
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

    /// The tabs' labels, in order.
    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        self.slots().iter().map(|t| t.label).collect()
    }

    /// Runs tab `id`'s own press: true when the set has one for it (the
    /// app's `on_tab` asks this first, then routes its own ids).
    #[must_use = "false when the app routes the tab itself"]
    pub fn press(&self, id: &str) -> bool {
        match self.presses.iter().find(|(t, _)| *t == id) {
            Some((_, press)) => {
                press.call(());
                true
            }
            None => false,
        }
    }
}

/// The favourites a note type declares.
#[derive(Clone, PartialEq)]
pub struct Declared {
    /// The note type (`recipe`): a note whose `type` is it, any case.
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

    /// Whether it is about notes of type `ty` (any case, spaces trimmed).
    #[must_use]
    pub fn is_for(&self, ty: &str) -> bool {
        self.note_type.trim().eq_ignore_ascii_case(ty.trim())
    }
}

/// The favourites for what is open: the open note's type's, else the default.
///
/// The first set declared for type `open` wins. A declared set with no
/// tabs is none (its type keeps the default); one without a primary action
/// keeps the default's round button.
#[must_use]
pub fn resolve_favourites(
    default: &Favourites,
    declared: &[Declared],
    open: Option<&str>,
) -> Favourites {
    let found = open.and_then(|ty| {
        declared
            .iter()
            .find(|d| d.is_for(ty) && !d.favourites.tabs.is_empty())
    });
    found.map_or_else(
        || default.clone(),
        |d| {
            let mut set = d.favourites.clone();
            if set.primary.is_none() {
                set.primary.clone_from(&default.primary);
            }
            set
        },
    )
}

/// A declared tab's id in the shell: its type's and its own, so it never
/// meets an app's own tab (`inbox`) or another type's.
#[must_use]
pub fn declared_tab_id(note_type: &str, id: &str) -> &'static str {
    intern(&format!("{}:{id}", note_type.trim().to_ascii_lowercase()))
}

/// `decl` as the app can show it, or `None` when it shows no tab of it.
///
/// Each tab's target is read by `open`: what a press does there, or `None`
/// for a target the app cannot open, and the tab is left out; the first
/// [`SLOTS`] of what is left are the tabs. Its primary action is read the
/// same way, showing its second face (`on`) while `state_on` says its
/// `state` is on. A declaration that does not
/// [`check`](TypeFavourites::check) is none.
#[must_use]
pub fn declare(
    decl: &TypeFavourites,
    mut open: impl FnMut(&Target) -> Option<Callback<()>>,
    state_on: impl Fn(&str) -> bool,
) -> Option<Declared> {
    decl.check().ok()?;
    let mut set = Favourites::default();
    for fav in &decl.tabs {
        if set.tabs.len() == SLOTS {
            break;
        }
        let Some(press) = open(&fav.target) else {
            continue;
        };
        let id = declared_tab_id(&decl.note_type, &fav.id);
        set.tabs
            .push(Tab::named(id, intern(&fav.label), intern(&fav.icon)));
        set.presses.push((id, press));
    }
    if set.tabs.is_empty() {
        return None;
    }
    set.primary = decl.primary.as_ref().and_then(|p| {
        let on = p
            .state
            .as_deref()
            .filter(|s| state_on(s))
            .and(p.on.as_ref());
        let (label, icon, target) = match on {
            Some(Face {
                label,
                icon,
                target,
            }) => (label, icon, target.as_ref().unwrap_or(&p.target)),
            None => (&p.label, &p.icon, &p.target),
        };
        let press = open(target)?;
        Some(
            Action::new(label.clone(), named_icon(icon), move |()| press.call(()))
                .glyph(intern(icon)),
        )
    });
    Some(Declared::new(decl.note_type.trim(), set))
}

#[cfg(test)]
mod tests {
    // `icon!` casts its closure to the fn pointer, as an app's does.
    #![allow(clippy::as_conversions)]

    // Named, so the prelude's own `Action` does not shadow the shell's.
    use super::{Action, Declared, Favourites, SLOTS, Tab, declare, resolve_favourites};
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

    fn a_recipe() -> architect_favourites::TypeFavourites {
        toml::from_str(
            r#"
note_type = "Recipe"

[[tabs]]
id = "page"
label = "Page"
icon = "list-music"
target = { view = "page" }

[[tabs]]
id = "elsewhere"
label = "Elsewhere"
icon = "file-music"
target = { view = "player.now" }

[[tabs]]
id = "steps"
label = "Steps"
icon = "list"
target = { view = "demo.cook/steps" }

[[tabs]]
id = "timer"
label = "Timer"
icon = "timer"
target = { action = "COOK_TIMER" }

[[tabs]]
id = "notes"
label = "Notes"
icon = "notebook-text"
target = { place = "Notes/" }

[primary]
id = "cook"
label = "Cook"
icon = "play"
target = { action = "COOK_TOGGLE" }
state = "cooking"
on = { label = "Stop", icon = "pause" }
"#,
        )
        .unwrap()
    }

    /// What this test's app opens: everything but `player.*`.
    fn opens(t: &architect_favourites::Target) -> Option<Callback<()>> {
        use architect_favourites::Target;
        let known = match t {
            Target::View(v) => !v.starts_with("player."),
            Target::Action(_) | Target::Place(_) => true,
        };
        known.then(|| Callback::new(|()| {}))
    }

    #[test]
    fn a_declared_type_shows_what_the_app_can_open_and_drops_the_rest() {
        in_runtime(declared_type_drops_the_unknown);
    }

    fn declared_type_drops_the_unknown() {
        let d = declare(&a_recipe(), opens, |_| false).unwrap();
        // `player.now` is no view of this app's: left out, and the fifth
        // fills the fourth slot.
        assert_eq!(d.favourites.labels(), ["Page", "Steps", "Timer", "Notes"]);
        assert_eq!(d.favourites.ids()[0], "recipe:page");
        assert!(d.favourites.press("recipe:steps"));
        assert!(
            !d.favourites.press("inbox"),
            "the app's own ids are the app's"
        );
        // Its icons by name, from Lucide's table.
        assert_eq!(d.favourites.tabs[0].glyph, Some("list-music"));
        let default = default();
        let declared = vec![d];
        // Any case of the type.
        let set = resolve_favourites(&default, &declared, Some("recipe"));
        assert_eq!(set.labels(), ["Page", "Steps", "Timer", "Notes"]);
        // Not declared: the default and Capture.
        let other = resolve_favourites(&default, &declared, Some("song"));
        assert_eq!(other.ids(), ["inbox", "vault", "project", "schedule"]);
        assert_eq!(
            other.primary.as_ref().map(|a| a.label.as_str()),
            Some("Capture")
        );
    }

    #[test]
    fn more_than_four_tabs_keep_the_first_four() {
        in_runtime(more_than_four);
    }

    fn more_than_four() {
        let all = |_: &architect_favourites::Target| Some(Callback::new(|()| {}));
        let d = declare(&a_recipe(), all, |_| false).unwrap();
        assert_eq!(d.favourites.tabs.len(), SLOTS);
        assert_eq!(
            d.favourites.labels(),
            ["Page", "Elsewhere", "Steps", "Timer"]
        );
        // The fifth is no tab at all: nothing presses it.
        assert!(!d.favourites.press("recipe:notes"));
    }

    #[test]
    fn a_type_whose_targets_the_app_cannot_open_keeps_the_default() {
        in_runtime(no_known_target);
    }

    fn no_known_target() {
        let none = |_: &architect_favourites::Target| None;
        assert!(declare(&a_recipe(), none, |_| false).is_none());
        // A declaration that breaks the manifest's rules is none either.
        let mut bad = a_recipe();
        bad.note_type = " ".into();
        assert!(declare(&bad, opens, |_| false).is_none());
    }

    #[test]
    fn the_primary_shows_its_second_face_while_its_state_is_on() {
        in_runtime(primary_faces);
    }

    fn primary_faces() {
        let off = declare(&a_recipe(), opens, |_| false).unwrap();
        let p = off.favourites.primary.unwrap();
        assert_eq!((p.label.as_str(), p.glyph), ("Cook", Some("play")));
        let on = declare(&a_recipe(), opens, |s| s == "cooking").unwrap();
        let p = on.favourites.primary.unwrap();
        assert_eq!((p.label.as_str(), p.glyph), ("Stop", Some("pause")));
        // A state it does not know: the first face.
        let other = declare(&a_recipe(), opens, |s| s == "playing").unwrap();
        assert_eq!(other.favourites.primary.unwrap().label, "Cook");
        // A primary the app cannot run: the default's round button stays.
        let no_actions = |t: &architect_favourites::Target| {
            (!matches!(t, architect_favourites::Target::Action(_)))
                .then(|| opens(t))
                .flatten()
        };
        let d = declare(&a_recipe(), no_actions, |_| false).unwrap();
        assert!(d.favourites.primary.is_none());
        let default = default();
        let set = resolve_favourites(&default, &[d], Some("Recipe"));
        assert_eq!(
            set.primary.as_ref().map(|a| a.label.as_str()),
            Some("Capture")
        );
    }
}
