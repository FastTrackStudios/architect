//! The Architect app's bottom slots on a phone, as favourites
//! (`architect_shell::Favourites`): Inbox, Vault, Project and Schedule,
//! and Capture as the round button, unless the open note's type declares
//! its own (`architect_shell::resolve_favourites`).
//!
//! The declarations are the loaded extensions' `[[favourites]]`
//! ([`declared`]), each target read here ([`opens`]): `{view = "page"}` is
//! the open note, `{view = "search"}` quick open, `{view = "<view>"}` (or
//! `"<its own id>/<view>"`) a view of the declaring extension's own on the
//! open note, `{action}` an action the app has that the declaring
//! extension may run (its own namespace, or one its capabilities name up
//! to their effect; never a destructive one: a tab's press asks no one),
//! `{place}` a note beside the open one. Anything else (a share page's
//! `player.*` views, another extension's view, an action it may not run,
//! a heading or a folder) is left out. The open note is the Vault's
//! workspace's document on show ([`VaultFocus`]); its type its `type`
//! property, read again whenever the file changes.
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

use std::collections::BTreeMap;

use architect_extensions::favourites::{Declarer, Effect, Target};
use architect_shell::{Action, Declared, Favourites, PlaceOpened, Shows, Tab, icon};
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

/// The note on show in the Vault's workspace: its path in the vault, its
/// `type` property when it has one, and the extension view it is shown in
/// (`<extension>/<view>`), if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenNote {
    pub path: String,
    pub note_type: Option<String>,
    pub view: Option<String>,
}

impl OpenNote {
    /// What is on show, as a declared tab would have opened it.
    #[must_use]
    pub fn shows(&self) -> Shows {
        match self.view.as_deref().and_then(|v| v.split_once('/')) {
            Some((extension, view)) => Shows::View {
                extension: extension.to_owned(),
                view: view.to_owned(),
                note: self.path.clone(),
            },
            None => Shows::Note(self.path.clone()),
        }
    }
}

/// The Vault's workspace as the app's root sees it (in context, written by
/// `crate::vault::VaultPage` while it is mounted): the note on show, the
/// note whose set a `{place}` tab kept (and its type), and the workspace a
/// favourite opens a note in.
#[derive(Clone, Copy)]
pub struct VaultFocus {
    pub note: Signal<Option<OpenNote>>,
    pub opened: Signal<Option<(PlaceOpened, Option<String>)>>,
    pub workspace: Signal<Option<Signal<Workspace>>>,
}

impl VaultFocus {
    /// The root's, provided once.
    pub fn provide() -> Self {
        use_context_provider(|| Self {
            note: Signal::new(None),
            opened: Signal::new(None),
            workspace: Signal::new(None),
        })
    }

    /// The note whose type picks the slots, and its type: the note on show,
    /// or the note a `{place}` tab on show was opened from
    /// (`architect_shell::set_note`).
    #[must_use]
    pub fn set_note(&self) -> Option<(String, Option<String>)> {
        let note = self.note.read().clone()?;
        let opened = self.opened.read().clone();
        Some(match opened {
            Some((o, ty)) if o.place == note.path => (o.from, ty),
            _ => (note.path, note.note_type),
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

impl Opens {
    /// What is on show once it opened, on note `note`.
    #[must_use]
    pub fn shows(&self, note: &str) -> Shows {
        match self {
            Self::Note => Shows::Note(note.to_owned()),
            Self::Path(p) => Shows::Note(p.clone()),
            Self::View { extension, view } => Shows::View {
                extension: extension.clone(),
                view: view.clone(),
                note: note.to_owned(),
            },
            Self::QuickOpen | Self::Action(_) => Shows::Elsewhere,
        }
    }
}

/// What the app has to open a target with: the loaded extensions' views
/// (`(extension, view)`), the actions and commands it can run and how much
/// each changes, the loaded extensions' capabilities, the open note's
/// path, and whether a path is a file of the vault.
pub struct Reach<'a> {
    pub views: &'a [(String, String)],
    pub actions: &'a BTreeMap<String, Effect>,
    pub capabilities: &'a [(String, architect_extensions::manifest::Capabilities)],
    pub note: &'a str,
    pub exists: &'a dyn Fn(&str) -> bool,
}

/// How much a command changes, as a manifest names it.
#[must_use]
pub const fn effect(e: architect_commands::Effect) -> Effect {
    match e {
        architect_commands::Effect::ReadOnly => Effect::ReadOnly,
        architect_commands::Effect::Mutating => Effect::Mutating,
        architect_commands::Effect::Destructive => Effect::Destructive,
    }
}

/// Whether extension `declaring` may name action `id`, which does `e`, as
/// a favourite: `Capabilities::may_run` over its own namespace and its
/// capabilities (as `host::gate` lets any of its actions run), never a
/// destructive one.
fn may_run(declaring: &str, id: &str, e: Effect, reach: &Reach<'_>) -> bool {
    use architect_extensions::manifest::{Reach as Level, action_prefix};
    let level = match e {
        Effect::ReadOnly => Level::ReadOnly,
        Effect::Mutating => Level::Mutating,
        Effect::Destructive => return false,
    };
    reach
        .capabilities
        .iter()
        .find(|(ext, _)| ext == declaring)
        .is_some_and(|(_, c)| c.may_run(&action_prefix(declaring), id, level).is_ok())
}

/// Quick open, as the Command Palette runs it.
pub const QUICK_OPEN: &str = "COMMAND_PALETTE_OPEN_ANYTHING";

/// What `target`, declared by extension `declaring`, opens here; `None`
/// for what the app cannot open or `declaring` may not name (the tab is
/// left out).
#[must_use]
pub fn opens(declaring: &str, target: &Target, reach: &Reach<'_>) -> Option<Opens> {
    match target {
        Target::View(v) if v == "page" => Some(Opens::Note),
        Target::View(v) if v == "search" => reach
            .actions
            .contains_key(QUICK_OPEN)
            .then_some(Opens::QuickOpen),
        Target::View(v) => {
            let (extension, view) = v.split_once('/').unwrap_or((declaring, v));
            let own = Declarer {
                extension: declaring.to_owned(),
                ..Declarer::default()
            };
            (own.may_open_view_of(extension)
                && reach.views.iter().any(|(e, w)| e == extension && w == view))
            .then(|| Opens::View {
                extension: extension.to_owned(),
                view: view.to_owned(),
            })
        }
        Target::Action(a) => {
            let e = *reach.actions.get(a)?;
            may_run(declaring, a, e, reach).then(|| Opens::Action(a.clone()))
        }
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
/// read by [`opens`] and its press made by `run` (once per note, type and
/// target, kept in `presses`, not on every render); a primary action's
/// state is on while `state_on(extension, state)` says so (its own runtime
/// state, source `ext:<id>`).
#[must_use]
pub fn declared(
    decls: &[(String, architect_extensions::favourites::TypeFavourites)],
    ty: &str,
    reach: &Reach<'_>,
    presses: &architect_shell::Presses,
    run: impl Fn(&str, Opens) + Clone + 'static,
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
                    let shows = what.shows(reach.note);
                    let key = format!("{}\0{ty}\0{extension}\0{what:?}", reach.note);
                    let run = run.clone();
                    let by = extension.clone();
                    let press =
                        presses.get(key, || Callback::new(move |()| run(&by, what.clone())));
                    Some((press, shows))
                },
                |state| state_on(extension, state),
            )
        })
        .collect()
}

/// Runs a favourite's declared action `id` as the extension `by` that
/// declared it, never as the app: what it reaches checks whose call it is
/// by that (the playhead's `PLAYHEAD_TOGGLE` then toggles `by`'s own
/// source, the one its face reads, not whichever is active; #660 M3).
///
/// # Errors
///
/// The action's own.
pub async fn run_declared(
    list: &architect::action::ActionList,
    by: &str,
    id: &str,
) -> Result<Vec<u8>, architect::action::ActionError> {
    use architect::action::{ActionCaller, Invocation, InvocationSource, Principal};
    let caller = ActionCaller::new(Principal::extension(by, None))
        .with_invocation(Invocation::new(InvocationSource::App));
    list.run_json_async_with(&caller, id, &[]).await
}

/// Whether runtime state entry (`source`, `key`, `text`) turns extension
/// `extension`'s state `state` on: its own (an extension's state is
/// `ext:<id>`'s), and not empty, `off`, `false` or `0`.
#[must_use]
pub fn state_on(source: &str, key: &str, text: &str, extension: &str, state: &str) -> bool {
    source.strip_prefix("ext:") == Some(extension)
        && key == state
        && !matches!(text.trim(), "" | "off" | "false" | "0")
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

    /// #660 M3: B plays; A's note is open, A's source paused, so A's
    /// primary shows Play. Pressed, it plays A's own source (B pauses: one
    /// thing heard at a time), never just pauses B, the active one.
    #[test]
    fn a_favourites_toggle_moves_its_declarers_own_source() {
        use architect::action::ActionList;
        use architect_extensions::extensions::Extensions;
        use architect_extensions::host::{Host, SharedSections};
        use architect_extensions::install::Installs;
        use architect_playhead::Caller;
        use serde_json::json;

        let tmp = tempfile::tempdir().unwrap();
        let list = ActionList::new();
        let host = Host::new(
            list.clone(),
            SharedSections::new(architect_settings::Registry::new()),
        );
        let ext = Extensions::new(host, Installs::new(tmp.path().join("i")))
            .with_linked(&architect_playhead::Playhead)
            .unwrap();
        ext.install_contribution_actions().unwrap();
        let _ = ext.sync();
        let set = |id: &str| {
            architect_playhead::ask(
                &Caller::Extension(id.to_owned()),
                "PLAYHEAD_SET",
                &json!({"queue": [{"id": "1", "title": id, "duration": 60}]}),
            )
            .unwrap();
        };
        set("lab.a");
        set("lab.b");
        architect_playhead::ask(
            &Caller::Extension("lab.b".into()),
            "PLAYHEAD_PLAY",
            &json!({}),
        )
        .unwrap();
        assert!(!architect_playhead::playing("lab.a"));
        futures::executor::block_on(run_declared(&list, "lab.a", "PLAYHEAD_TOGGLE")).unwrap();
        assert!(
            architect_playhead::playing("lab.a"),
            "A's primary moved another source"
        );
        assert!(!architect_playhead::playing("lab.b"));
        for id in ["lab.a", "lab.b"] {
            architect_playhead::ask(
                &Caller::Extension(id.to_owned()),
                "PLAYHEAD_SET",
                &json!({"queue": []}),
            )
            .unwrap();
        }
    }

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
target = { action = "LAB_COOK_TOGGLE" }
state = "cooking"
on = { label = "Stop", icon = "pause" }
"#;

    fn caps(actions: &[&str], effect: Level) -> architect_extensions::manifest::Capabilities {
        architect_extensions::manifest::Capabilities {
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
            effect,
            ..Default::default()
        }
    }

    use architect_extensions::manifest::Reach as Level;

    /// The app's actions: the recipe extension's own, the player's, the
    /// app's own and a destructive one of its own.
    fn actions() -> BTreeMap<String, Effect> {
        [
            ("LAB_COOK_TOGGLE", Effect::Mutating),
            ("LAB_COOK_BURN", Effect::Destructive),
            ("PLAYER_TOGGLE", Effect::Mutating),
            ("VAULT_DELETE", Effect::Destructive),
            ("APP_SETTINGS", Effect::ReadOnly),
            (QUICK_OPEN, Effect::ReadOnly),
        ]
        .into_iter()
        .map(|(a, e)| (a.to_owned(), e))
        .collect()
    }

    #[test]
    fn a_declared_target_opens_what_the_app_has_and_nothing_else() {
        let views = vec![
            ("lab.cook".to_owned(), "steps".to_owned()),
            ("lab.other".to_owned(), "board".to_owned()),
        ];
        let actions = actions();
        let capabilities = vec![("lab.cook".to_owned(), caps(&[], Level::ReadOnly))];
        let exists = |p: &str| p == "Food/Shopping.md";
        let reach = Reach {
            views: &views,
            actions: &actions,
            capabilities: &capabilities,
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
                // Another extension's view, loaded or not: never.
                None,
                Some(Opens::Path("Food/Shopping.md".into())),
            ]
        );
        assert_eq!(
            opens("lab.cook", &Target::View("lab.other/board".into()), &reach),
            None,
            "another extension's loaded view"
        );
        assert_eq!(
            opens("lab.cook", &Target::View("lab.cook/steps".into()), &reach),
            Some(Opens::View {
                extension: "lab.cook".into(),
                view: "steps".into()
            })
        );
        assert_eq!(
            opens("lab.cook", &Target::Action("NOT_HERE".into()), &reach),
            None
        );
        assert_eq!(
            opens("x", &Target::View("search".into()), &reach),
            Some(Opens::QuickOpen)
        );
        // A heading alone, or a folder: not opened from a tab yet.
        assert_eq!(opens("x", &Target::Place("#steps".into()), &reach), None);
        assert_eq!(opens("x", &Target::Place("Shopping/".into()), &reach), None);
    }

    #[test]
    fn a_favourites_action_is_one_its_extension_may_run() {
        let views = Vec::new();
        let actions = actions();
        let exists = |_: &str| false;
        let run = |caps: Vec<(String, architect_extensions::manifest::Capabilities)>, id: &str| {
            let reach = Reach {
                views: &views,
                actions: &actions,
                capabilities: &caps,
                note: "Food/Soup.md",
                exists: &exists,
            };
            opens("lab.cook", &Target::Action(id.to_owned()), &reach)
        };
        let none = || vec![("lab.cook".to_owned(), caps(&[], Level::ReadOnly))];
        // Its own: yes; the app's or another's, with no capabilities: no.
        assert_eq!(
            run(none(), "LAB_COOK_TOGGLE"),
            Some(Opens::Action("LAB_COOK_TOGGLE".into()))
        );
        assert_eq!(run(none(), "PLAYER_TOGGLE"), None);
        assert_eq!(run(none(), "APP_SETTINGS"), None);
        // A destructive one never, its own or one it may reach.
        assert_eq!(run(none(), "LAB_COOK_BURN"), None);
        let all = || vec![("lab.cook".to_owned(), caps(&["*"], Level::Destructive))];
        assert_eq!(run(all(), "VAULT_DELETE"), None);
        // What its capabilities name, up to their effect.
        let player = |e: Level| vec![("lab.cook".to_owned(), caps(&["PLAYER_"], e))];
        assert_eq!(
            run(player(Level::Mutating), "PLAYER_TOGGLE"),
            Some(Opens::Action("PLAYER_TOGGLE".into()))
        );
        assert_eq!(run(player(Level::ReadOnly), "PLAYER_TOGGLE"), None);
        assert_eq!(run(player(Level::Mutating), "APP_SETTINGS"), None);
        // An extension that is not loaded may run nothing but… nothing.
        assert_eq!(run(Vec::new(), "LAB_COOK_TOGGLE"), None);
    }

    #[test]
    fn the_loaded_extensions_declarations_follow_the_open_type() {
        let dom = VirtualDom::new(|| rsx! {});
        dom.in_scope(ScopeId::ROOT, || {
            let views = vec![("lab.cook".to_owned(), "steps".to_owned())];
            let actions = actions();
            let capabilities = vec![("lab.cook".to_owned(), caps(&[], Level::ReadOnly))];
            let exists = |p: &str| p == "Food/Shopping.md";
            let reach = Reach {
                views: &views,
                actions: &actions,
                capabilities: &capabilities,
                note: "Food/Soup.md",
                exists: &exists,
            };
            let decls = vec![("lab.cook".to_owned(), decl(RECIPE))];
            let run = |_: &str, _: Opens| {};
            let set = declared(
                &decls,
                "Recipe",
                &reach,
                &architect_shell::Presses::default(),
                run,
                |_, _| false,
            );
            assert_eq!(set.len(), 1);
            let default = default_set(0, None);
            let open = resolve_favourites(&default, &set, Some("recipe"));
            assert_eq!(open.labels(), ["Recipe", "Steps", "Shopping"]);
            assert_eq!(
                open.primary.as_ref().map(|a| a.label.as_str()),
                Some("Cook")
            );
            // The tab current is the one whose place is on show.
            let on_steps = OpenNote {
                path: "Food/Soup.md".into(),
                note_type: Some("recipe".into()),
                view: Some("lab.cook/steps".into()),
            };
            assert_eq!(open.current(&on_steps.shows()), Some("recipe:steps"));
            let on_page = OpenNote {
                view: None,
                ..on_steps
            };
            assert_eq!(open.current(&on_page.shows()), Some("recipe:page"));
            let on = declared(
                &decls,
                "recipe",
                &reach,
                &architect_shell::Presses::default(),
                run,
                |e, s| e == "lab.cook" && s == "cooking",
            );
            let open = resolve_favourites(&default, &on, Some("recipe"));
            assert_eq!(
                open.primary.as_ref().map(|a| a.label.as_str()),
                Some("Stop")
            );
            // Another type, or none: the default four.
            assert!(
                declared(
                    &decls,
                    "song",
                    &reach,
                    &architect_shell::Presses::default(),
                    run,
                    |_, _| false
                )
                .is_empty()
            );
            assert_eq!(
                resolve_favourites(&default, &[], None).ids(),
                ["inbox", "vault", "project", "schedule"]
            );
        });
    }

    #[test]
    fn a_place_on_show_keeps_the_set_of_the_note_it_was_opened_from() {
        let dom = VirtualDom::new(|| rsx! {});
        dom.in_scope(ScopeId::ROOT, || {
            let focus = VaultFocus {
                note: Signal::new(Some(OpenNote {
                    path: "Food/Shopping.md".into(),
                    note_type: Some("list".into()),
                    view: None,
                })),
                opened: Signal::new(Some((
                    PlaceOpened {
                        place: "Food/Shopping.md".into(),
                        from: "Food/Soup.md".into(),
                    },
                    Some("recipe".into()),
                ))),
                workspace: Signal::new(None),
            };
            assert_eq!(
                focus.set_note(),
                Some(("Food/Soup.md".into(), Some("recipe".into())))
            );
            let mut note = focus.note;
            note.set(Some(OpenNote {
                path: "Other.md".into(),
                note_type: None,
                view: None,
            }));
            assert_eq!(focus.set_note(), Some(("Other.md".into(), None)));
        });
    }

    #[test]
    fn a_primarys_state_is_its_extensions_own() {
        assert!(state_on(
            "ext:lab.cook",
            "cooking",
            "1",
            "lab.cook",
            "cooking"
        ));
        assert!(!state_on(
            "ext:lab.cook",
            "cooking",
            "off",
            "lab.cook",
            "cooking"
        ));
        assert!(!state_on(
            "ext:lab.cook",
            "other",
            "1",
            "lab.cook",
            "cooking"
        ));
        // Another's, or the bare id an agent may not write as.
        assert!(!state_on(
            "ext:lab.other",
            "cooking",
            "1",
            "lab.cook",
            "cooking"
        ));
        assert!(!state_on("lab.cook", "cooking", "1", "lab.cook", "cooking"));
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
