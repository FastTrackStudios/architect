//! End-to-end test for `#[architect::actions]` codegen.
//!
//! Mirrors the shape `architect-rpc-derive/tests/smoke.rs` uses: exercise
//! the macro's emitted surface directly (no real REAPER/CLI backend —
//! those are Phase 2/3, in `daw-reaper` and `fts-cli` respectively).
//! This test's `InMemoryActionBackend` stands in for both: it proves the
//! `ActionBackend` seam is enough for a REAPER-style registry (register,
//! look up by id, invoke) and, in `clap_tree_from_actions`, that
//! `ActionMeta::all()` carries everything needed to mechanically build a
//! `category -> group -> action` CLI tree without any per-action
//! hand-wiring.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use architect::action::{ActionBackend, ActionMeta};
use architect::actions;

#[actions(namespace = "SESSION")]
pub trait SetlistActions {
    #[action(
        description = "Scan every open REAPER project tab and rebuild the setlist",
        category = "Setlist",
        group = "Build"
    )]
    fn build_setlist(&self);

    #[action(
        description = "Stamp demo markers/regions (3 songs) into the current project",
        category = "Setlist",
        group = "Demo"
    )]
    fn load_demo_setlist(&self);

    #[action(
        description = "Log every marker/region with position, name, color, and lane",
        category = "Debug",
        group = "Diagnostics"
    )]
    fn dump_ruler_state(&self);
}

#[derive(Default)]
struct FakeSession {
    calls: Mutex<Vec<&'static str>>,
}

impl SetlistActions for FakeSession {
    fn build_setlist(&self) {
        self.calls.lock().unwrap().push("build_setlist");
    }

    fn load_demo_setlist(&self) {
        self.calls.lock().unwrap().push("load_demo_setlist");
    }

    fn dump_ruler_state(&self) {
        self.calls.lock().unwrap().push("dump_ruler_state");
    }
}

/// One registered (metadata, handler) pair — factored out purely to
/// satisfy clippy's `type_complexity` lint on the field below.
type RegisteredAction = (&'static ActionMeta, Arc<dyn Fn() -> Result<(), String> + Send + Sync>);

/// Stand-in for a real host (REAPER named-command table, a CLI
/// dispatcher). Stores `(meta, handler)` and invokes by id — exactly
/// the shape `daw-reaper`'s `register_action_main_thread` or a CLI
/// subcommand handler would wrap.
#[derive(Default)]
struct InMemoryActionBackend {
    registered: Mutex<Vec<RegisteredAction>>,
}

impl ActionBackend for InMemoryActionBackend {
    fn register(&self, meta: &'static ActionMeta, handler: Arc<dyn Fn() -> Result<(), String> + Send + Sync>) {
        self.registered.lock().unwrap().push((meta, handler));
    }
}

impl InMemoryActionBackend {
    fn invoke(&self, id: &str) -> bool {
        let registered = self.registered.lock().unwrap();
        match registered.iter().find(|(meta, _)| meta.id == id) {
            Some((_, handler)) => {
                handler().expect("action handler should succeed in this test");
                true
            }
            None => false,
        }
    }
}

#[test]
fn actions_all_reports_every_decorated_method() {
    let all = SetlistActionsActions::all();
    assert_eq!(all.len(), 3);

    let ids: Vec<_> = all.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        [
            "SESSION_BUILD_SETLIST",
            "SESSION_LOAD_DEMO_SETLIST",
            "SESSION_DUMP_RULER_STATE",
        ]
    );

    let build = all
        .iter()
        .find(|m| m.id == "SESSION_BUILD_SETLIST")
        .unwrap();
    assert_eq!(build.display_name, "Build Setlist");
    assert_eq!(build.category, "Setlist");
    assert_eq!(build.group, "Build");
    assert!(!build.toggleable);
    assert_eq!(build.trait_name, "SetlistActions");
    assert_eq!(build.method_name, "build_setlist");
}

#[test]
fn register_and_invoke_through_backend() {
    let session = Arc::new(FakeSession::default());
    let backend = InMemoryActionBackend::default();

    register_setlist_actions(&backend, session.clone());

    assert_eq!(backend.registered.lock().unwrap().len(), 3);

    assert!(backend.invoke("SESSION_LOAD_DEMO_SETLIST"));
    assert!(backend.invoke("SESSION_BUILD_SETLIST"));
    assert!(!backend.invoke("SESSION_DOES_NOT_EXIST"));

    assert_eq!(
        *session.calls.lock().unwrap(),
        vec!["load_demo_setlist", "build_setlist"]
    );
}

/// Proves `ActionMeta::all()` carries enough to mechanically build a
/// `category -> group -> [action]` tree — the shape a CLI's `clap`
/// codegen (Phase 2, in `fts-cli`) or a REAPER menu builder would need.
/// This is a plain grouping demo, not a real `clap::Command` builder —
/// architect doesn't depend on clap.
fn group_actions_by_category_and_group(
    actions: &[ActionMeta],
) -> BTreeMap<&'static str, BTreeMap<&'static str, Vec<&ActionMeta>>> {
    let mut tree: BTreeMap<&'static str, BTreeMap<&'static str, Vec<&ActionMeta>>> =
        BTreeMap::new();
    for action in actions {
        tree.entry(action.category)
            .or_default()
            .entry(action.group)
            .or_default()
            .push(action);
    }
    tree
}

#[test]
fn cli_tree_shape_is_derivable_from_metadata_alone() {
    let tree = group_actions_by_category_and_group(SetlistActionsActions::all());

    // category -> group -> action, exactly a `clap::Command` subcommand
    // tree's shape (`fts session <category:lower> <group:lower> <action>`
    // or flattened, per Phase 2's choice — either is mechanical from here).
    let categories: Vec<_> = tree.keys().copied().collect();
    assert_eq!(categories, ["Debug", "Setlist"]);

    let setlist = &tree["Setlist"];
    let groups: Vec<_> = setlist.keys().copied().collect();
    assert_eq!(groups, ["Build", "Demo"]);
    assert_eq!(setlist["Build"][0].id, "SESSION_BUILD_SETLIST");
    assert_eq!(setlist["Demo"][0].id, "SESSION_LOAD_DEMO_SETLIST");

    let debug = &tree["Debug"];
    assert_eq!(debug["Diagnostics"][0].id, "SESSION_DUMP_RULER_STATE");
}

/// Real `clap::Command` proof-of-concept (Phase 2 will live in
/// `fts-cli`, this just proves the shape is sound before that's built).
///
/// The core design question this module answers: can a feature crate
/// (e.g. `session`) build its own standalone CLI from `ActionMeta::all()`
/// *and* let a separate parent app (e.g. `fts-cli`) mount that exact same
/// command tree as a subcommand group — without either side needing to
/// know in advance which mode it's in?
///
/// The answer is yes, via two mechanically-derived, nesting-agnostic
/// functions:
///
/// - [`build_action_command`] returns a plain `clap::Command` named
///   whatever the caller asks for. It doesn't call `.get_matches()` and
///   it doesn't know if it's the root command or a mounted subtree — the
///   caller decides by either running it directly or `.subcommand()`-ing
///   it into something bigger.
/// - [`resolve_action_id`] takes an `&ArgMatches` scoped to *this*
///   action-tree's own subtree (the root matches when standalone, or
///   `parent_matches.subcommand_matches(name)` when mounted) and resolves
///   which `ActionMeta::id` was requested. It never looks above its own
///   subtree, so it works identically at any nesting depth.
mod clap_tree_from_actions {
    use architect::action::ActionMeta;
    use clap::{ArgMatches, Command};

    use super::SetlistActionsActions;

    /// `build_setlist` -> `build-setlist` (clap subcommand convention).
    /// Leaked to `'static` because `ActionMeta::method_name` is itself
    /// `'static` and `clap::Command` wants `'static`-friendly names — a
    /// real (non-test) implementation would precompute+cache these once
    /// per process instead of leaking per call.
    fn kebab(method_name: &'static str) -> &'static str {
        if method_name.contains('_') {
            method_name.replace('_', "-").leak()
        } else {
            method_name
        }
    }

    /// Build a `clap::Command` named `name` with one subcommand per
    /// action in `actions`, each subcommand grouped under a `--help`
    /// heading per `category` so help output reads as
    /// `category -> actions`, while the subcommand structure itself
    /// stays flat (`<name> <action>`) — matching the real target shape
    /// (`fts session build-setlist`, not `fts session setlist
    /// build-setlist`).
    ///
    /// Returns a bare `Command`: not run, not parsed. The caller decides
    /// standalone (`build_action_command(...).get_matches()`) vs. mounted
    /// (`parent.subcommand(build_action_command(...))`).
    fn build_action_command(name: &'static str, actions: &'static [ActionMeta]) -> Command {
        let mut cmd = Command::new(name).about("Actions generated from ActionMeta::all()");
        for action in actions {
            let heading: &'static str = if action.category.is_empty() {
                "Actions"
            } else {
                action.category
            };
            // `next_help_heading` on the parent sets the heading every
            // subcommand added *after* the call is grouped under, so it
            // has to be set here (on `cmd`), not on the child `Command`.
            cmd = cmd
                .next_help_heading(heading)
                .subcommand(Command::new(kebab(action.method_name)).about(action.description));
        }
        cmd
    }

    /// Resolve the `ActionMeta::id` requested by `matches`, where
    /// `matches` is scoped to this action tree's own subtree (i.e. it's
    /// either the top-level `ArgMatches` when standalone, or the
    /// `ArgMatches` handed back by `subcommand_matches(name)` on some
    /// parent when mounted). Deliberately takes no notion of "am I
    /// nested" — that's the whole point.
    fn resolve_action_id(
        matches: &ArgMatches,
        actions: &'static [ActionMeta],
    ) -> Option<&'static str> {
        let (sub_name, _sub_matches) = matches.subcommand()?;
        actions
            .iter()
            .find(|a| kebab(a.method_name) == sub_name)
            .map(|a| a.id)
    }

    /// Standalone mode: the feature crate (`session`) runs its own
    /// generated command tree as the whole CLI — no parent involved.
    #[test]
    fn standalone_cli_resolves_action_id() {
        let actions = SetlistActionsActions::all();
        let cmd = build_action_command("session", actions);

        let matches = cmd
            .try_get_matches_from(["session", "build-setlist"])
            .expect("parses as a standalone CLI invocation");

        assert_eq!(
            resolve_action_id(&matches, actions),
            Some("SESSION_BUILD_SETLIST")
        );
    }

    /// Mounted mode: a *separate* parent app (`fts-cli`) builds its own
    /// top-level `Command` and mounts session's exact same generated
    /// tree under its own namespace via plain `.subcommand()`. Nothing
    /// about `build_action_command`'s output changed between this test
    /// and the standalone one above — same function, same return type,
    /// zero special-casing for "am I nested".
    #[test]
    fn mounted_cli_resolves_action_id_through_parent() {
        let actions = SetlistActionsActions::all();

        // A second, wholly separate top-level command — stands in for
        // `fts-cli`'s own `clap::Parser`-derived `Cli` struct's command
        // tree. It has no knowledge of `SetlistActions` beyond mounting
        // the pre-built subtree under the name "session".
        let parent = Command::new("fts")
            .about("FastTrackStudio CLI")
            .subcommand(build_action_command("session", actions));

        let matches = parent
            .try_get_matches_from(["fts", "session", "load-demo-setlist"])
            .expect("parses as `fts session load-demo-setlist`");

        // Dispatch side: the parent hands its "session" submatches down
        // to exactly the same `resolve_action_id` used standalone above.
        // `session`'s dispatch logic never needed to special-case
        // top-level-vs-nested — it only ever sees its own subtree.
        let (name, session_matches) = matches.subcommand().expect("a subcommand was matched");
        assert_eq!(name, "session");
        assert_eq!(
            resolve_action_id(session_matches, actions),
            Some("SESSION_LOAD_DEMO_SETLIST")
        );
    }

    /// Same tree, same resolver, mounted two levels deep
    /// (`fts studio session dump-ruler-state`) — confirms there's no
    /// hidden depth assumption (e.g. "parent is always exactly one level
    /// up").
    #[test]
    fn mounted_cli_resolves_action_id_at_arbitrary_depth() {
        let actions = SetlistActionsActions::all();

        let grandparent = Command::new("fts").subcommand(
            Command::new("studio").subcommand(build_action_command("session", actions)),
        );

        let matches = grandparent
            .try_get_matches_from(["fts", "studio", "session", "dump-ruler-state"])
            .expect("parses as `fts studio session dump-ruler-state`");

        let (_, studio_matches) = matches.subcommand().expect("studio matched");
        let (_, session_matches) = studio_matches.subcommand().expect("session matched");
        assert_eq!(
            resolve_action_id(session_matches, actions),
            Some("SESSION_DUMP_RULER_STATE")
        );
    }
}
