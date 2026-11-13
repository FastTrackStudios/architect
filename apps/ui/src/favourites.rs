//! The Architect app's bottom slots on a phone, as favourites
//! (`architect_shell::Favourites`): Inbox, Vault, Project and Schedule,
//! and Capture as the round button, unless the open note's type declares
//! its own (`architect_shell::resolve_favourites`).
//!
//! The declarations are the loaded extensions' `[[favourites]]`
//! ([`declared`]), each target read here ([`opens`]): `{view = "page"}` is
//! the open note, `{view = "search"}` quick open, `{view = "<extension
//! id>/<view>"}` (or a bare view of the declaring extension's own) that
//! extension's view on the open note, `{action}` an action the app has,
//! `{place}` a note beside the open one. Anything else (a share page's
//! `player.*` views, an action or a view no loaded extension has, a
//! heading or a folder) is left out. The open note is the Vault's
//! workspace's document on show ([`VaultFocus`]); its type its `type`
//! property.
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

use std::collections::BTreeSet;

use architect_extensions::favourites::Target;
use architect_shell::{Action, Declared, Favourites, Tab, icon};
use architect_workspace::Workspace;
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

/// The note on show in the Vault's workspace: its path in the vault and
/// its `type` property, when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenNote {
    pub path: String,
    pub note_type: Option<String>,
}

/// The Vault's workspace as the app's root sees it (in context, written by
/// `crate::vault::VaultPage` while it is mounted): the note on show, and
/// the workspace a favourite opens a note in.
#[derive(Clone, Copy)]
pub struct VaultFocus {
    pub note: Signal<Option<OpenNote>>,
    pub workspace: Signal<Option<Signal<Workspace>>>,
}

impl VaultFocus {
    /// The root's, provided once.
    pub fn provide() -> Self {
        use_context_provider(|| Self {
            note: Signal::new(None),
            workspace: Signal::new(None),
        })
    }
}

/// A note's `type` property in its markdown, trimmed and unquoted.
#[must_use]
pub fn note_type(text: &str) -> Option<String> {
    let (props, _) = crate::note::frontmatter(text);
    props
        .into_iter()
        .find(|(k, _)| k == "type")
        .map(|(_, v)| {
            v.trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_owned()
        })
        .filter(|v| !v.is_empty())
}

/// What a declared favourite opens in the Architect app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opens {
    /// The open note itself (`page`): its pane brought forward.
    Note,
    /// Quick open (`search`).
    QuickOpen,
    /// An extension's view, showing the open note.
    View { extension: String, view: String },
    /// An action, by id.
    Action(String),
    /// Another note of the vault, by its path (a `{place}`).
    Path(String),
}

/// What the app has to open a target with: the loaded extensions' views
/// (`(extension, view)`), the actions and commands it can run, the open
/// note's path, and whether a path is a file of the vault.
pub struct Reach<'a> {
    pub views: &'a [(String, String)],
    pub actions: &'a BTreeSet<String>,
    pub note: &'a str,
    pub exists: &'a dyn Fn(&str) -> bool,
}

/// Quick open, as the Command Palette runs it.
pub const QUICK_OPEN: &str = "COMMAND_PALETTE_OPEN_ANYTHING";

/// What `target`, declared by extension `declaring`, opens here; `None`
/// for what the app cannot open (the tab is left out).
#[must_use]
pub fn opens(declaring: &str, target: &Target, reach: &Reach<'_>) -> Option<Opens> {
    match target {
        Target::View(v) if v == "page" => Some(Opens::Note),
        Target::View(v) if v == "search" => reach
            .actions
            .contains(QUICK_OPEN)
            .then_some(Opens::QuickOpen),
        Target::View(v) => {
            let (extension, view) = v.split_once('/').unwrap_or((declaring, v));
            reach
                .views
                .iter()
                .any(|(e, w)| e == extension && w == view)
                .then(|| Opens::View {
                    extension: extension.to_owned(),
                    view: view.to_owned(),
                })
        }
        Target::Action(a) => reach.actions.contains(a).then(|| Opens::Action(a.clone())),
        Target::Place(p) => {
            let path = architect_extensions::favourites::place_path(reach.note, p)?;
            let md = format!("{path}.md");
            if (reach.exists)(&path) {
                Some(Opens::Path(path))
            } else if (reach.exists)(&md) {
                Some(Opens::Path(md))
            } else {
                None
            }
        }
    }
}

/// The sets the loaded extensions declare for note type `ty`, each target
/// read by [`opens`] and its press made by `run`; a primary action's
/// state is on while `state_on(extension, state)` says so.
#[must_use]
pub fn declared(
    decls: &[(String, architect_extensions::favourites::TypeFavourites)],
    ty: &str,
    reach: &Reach<'_>,
    run: impl Fn(Opens) + Clone + 'static,
    state_on: impl Fn(&str, &str) -> bool,
) -> Vec<Declared> {
    decls
        .iter()
        .filter(|(_, d)| d.is_for(ty))
        .filter_map(|(extension, d)| {
            architect_shell::declare(
                d,
                |target| {
                    let what = opens(extension, target, reach)?;
                    let run = run.clone();
                    Some(Callback::new(move |()| run(what.clone())))
                },
                |state| state_on(extension, state),
            )
        })
        .collect()
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
        assert_eq!(
            set.slots().first().and_then(|t| t.badge.clone()).as_deref(),
            Some("5")
        );
        assert!(
            default_set(0, None)
                .slots()
                .iter()
                .all(|t| t.badge.is_none())
        );
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

    fn decl(text: &str) -> architect_extensions::favourites::TypeFavourites {
        toml::from_str(text).unwrap()
    }

    const RECIPE: &str = r#"
note_type = "recipe"

[[tabs]]
id = "page"
label = "Recipe"
icon = "notebook-text"
target = { view = "page" }

[[tabs]]
id = "now"
label = "Now"
icon = "play"
target = { view = "player.now" }

[[tabs]]
id = "steps"
label = "Steps"
icon = "list"
target = { view = "steps" }

[[tabs]]
id = "other"
label = "Other"
icon = "list"
target = { view = "lab.other/board" }

[[tabs]]
id = "shop"
label = "Shopping"
icon = "shopping-cart"
target = { place = "Shopping" }

[primary]
id = "cook"
label = "Cook"
icon = "play"
target = { action = "COOK_TOGGLE" }
state = "cooking"
on = { label = "Stop", icon = "pause" }
"#;

    #[test]
    fn a_declared_target_opens_what_the_app_has_and_nothing_else() {
        let views = vec![("lab.cook".to_owned(), "steps".to_owned())];
        let actions: BTreeSet<String> = ["COOK_TOGGLE".to_owned()].into();
        let exists = |p: &str| p == "Food/Shopping.md";
        let reach = Reach {
            views: &views,
            actions: &actions,
            note: "Food/Soup.md",
            exists: &exists,
        };
        let d = decl(RECIPE);
        let got: Vec<Option<Opens>> = d
            .tabs
            .iter()
            .map(|t| opens("lab.cook", &t.target, &reach))
            .collect();
        assert_eq!(
            got,
            [
                Some(Opens::Note),
                // A share page's player view: not the app's.
                None,
                // A bare view: the declaring extension's own.
                Some(Opens::View {
                    extension: "lab.cook".into(),
                    view: "steps".into()
                }),
                // Another extension's view that is not loaded.
                None,
                Some(Opens::Path("Food/Shopping.md".into())),
            ]
        );
        assert_eq!(
            opens("lab.cook", &Target::Action("NOT_HERE".into()), &reach),
            None
        );
        assert_eq!(opens("x", &Target::View("search".into()), &reach), None);
        // A heading alone, or a folder: not opened from a tab yet.
        assert_eq!(opens("x", &Target::Place("#steps".into()), &reach), None);
        assert_eq!(opens("x", &Target::Place("Shopping/".into()), &reach), None);
    }

    #[test]
    fn the_loaded_extensions_declarations_follow_the_open_type() {
        let dom = VirtualDom::new(|| rsx! {});
        dom.in_scope(ScopeId::ROOT, || {
            let views = vec![("lab.cook".to_owned(), "steps".to_owned())];
            let actions: BTreeSet<String> = ["COOK_TOGGLE".to_owned()].into();
            let exists = |p: &str| p == "Food/Shopping.md";
            let reach = Reach {
                views: &views,
                actions: &actions,
                note: "Food/Soup.md",
                exists: &exists,
            };
            let decls = vec![("lab.cook".to_owned(), decl(RECIPE))];
            let run = |_: Opens| {};
            let set = declared(&decls, "Recipe", &reach, run, |_, _| false);
            assert_eq!(set.len(), 1);
            let default = default_set(0, None);
            let open = resolve_favourites(&default, &set, Some("recipe"));
            assert_eq!(open.labels(), ["Recipe", "Steps", "Shopping"]);
            assert_eq!(
                open.primary.as_ref().map(|a| a.label.as_str()),
                Some("Cook")
            );
            let on = declared(&decls, "recipe", &reach, run, |e, s| {
                e == "lab.cook" && s == "cooking"
            });
            let open = resolve_favourites(&default, &on, Some("recipe"));
            assert_eq!(
                open.primary.as_ref().map(|a| a.label.as_str()),
                Some("Stop")
            );
            // Another type, or none: the default four.
            assert!(declared(&decls, "song", &reach, run, |_, _| false).is_empty());
            assert_eq!(
                resolve_favourites(&default, &[], None).ids(),
                ["inbox", "vault", "project", "schedule"]
            );
        });
    }

    #[test]
    fn a_notes_type_is_its_type_property() {
        assert_eq!(
            note_type("---\ntype: recipe\ntitle: Soup\n---\nbody").as_deref(),
            Some("recipe")
        );
        assert_eq!(
            note_type("---\ntype: \"recipe\"\n---\n").as_deref(),
            Some("recipe")
        );
        assert_eq!(note_type("no frontmatter"), None);
        assert_eq!(note_type("---\ntitle: x\n---\n"), None);
    }

    #[test]
    fn a_declared_type_replaces_the_slots_and_the_four_stay_reachable() {
        let default = default_set(0, None);
        let declared = vec![Declared::new(
            "setlist",
            Favourites::new(
                vec![Tab::new("setlist", "Setlist", icon!(NotebookText))],
                None,
            ),
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
