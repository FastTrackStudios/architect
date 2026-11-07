//! Decks: one context each, shown in a screenset, with the context's own
//! changes kept as its override (`docs/content/architecture/workspace.md`,
//! "Decks"). The model, its stores across a restart, and the `DECK_*`
//! actions over MCP.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use architect::action::ActionList;
use architect_mcp::Server;
use architect_workspace::control::deck::{HeadlessDecks, ids};
use architect_workspace::home::{ContextFiles, FileStore};
use architect_workspace::store::{self, ContextStore};
use architect_workspace::{
    DeckContext, DeckId, DeckState, DeckTemplate, Decks, FormClass, Load, MemoryContextStore,
    MemoryStore, Pane, Screenset, Screensets, Side, Target, Unchanged, Workspace,
};
use serde_json::{Value, json};

/// "Mixing": the mixer in the center, the song's chart docked right.
fn mixing() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(Pane::new("mixer", "song.mixer", "Mixer"), Target::Tab);
    let _ = w.open(
        Pane::new("chart", "song.chart", "Chart"),
        Target::Dock(Side::Right),
    );
    w
}

/// "Charts + Lyrics": chart and lyrics side by side.
fn charts() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(Pane::new("chart", "song.chart", "Chart"), Target::Tab);
    let _ = w.open(
        Pane::new("lyrics", "song.lyrics", "Lyrics"),
        Target::Split(Side::Right),
    );
    w
}

fn screensets() -> Screensets {
    Screensets::default().with_builtins([
        Screenset::new("Mixing")
            .slot(1)
            .layout(FormClass::Desk, mixing()),
        Screenset::new("Charts + Lyrics")
            .slot(2)
            .layout(FormClass::Desk, charts()),
    ])
}

fn song(id: &str, title: &str) -> DeckContext {
    DeckContext::new("song", id, title).icon("music")
}

fn state() -> DeckState {
    DeckState::new(Decks::default(), screensets(), Workspace::new())
        .with_templates(vec![DeckTemplate::new("song", "mixing")])
}

/// A person's change inside the deck on show: the lyrics pane opened.
fn arrange(st: &mut DeckState) {
    let _ = st.live.open(
        Pane::new("lyrics", "song.lyrics", "Lyrics"),
        Target::Split(Side::Bottom),
    );
    let (form, live) = (st.form, st.live.clone());
    st.screensets.set_working(form, live);
}

fn ids_of(st: &DeckState) -> Vec<String> {
    st.decks
        .decks
        .iter()
        .map(|d| d.context.id.clone())
        .collect()
}

#[test]
fn two_decks_on_one_screenset_show_different_contexts_each_as_left() {
    let mut st = state();
    let a = st
        .open(
            song("amazing-grace", "Amazing Grace"),
            Load::NewDeck,
            None,
            None,
        )
        .unwrap()
        .unwrap();
    let b = st
        .open(song("slow-river", "Slow River"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    // Both in the type's default screenset, the second on show.
    assert_eq!(st.decks.get(&a).unwrap().screenset, "mixing");
    assert_eq!(st.decks.get(&b).unwrap().screenset, "mixing");
    assert_eq!(st.decks.active.as_ref(), Some(&b));
    assert_eq!(ids_of(&st), ["amazing-grace", "slow-river"]);
    assert!(st.live.same_arrangement(&mixing()));

    // Rearranged inside Slow River: its override, not the screenset's.
    arrange(&mut st);
    let slow_river = st.live.clone();
    st.activate(&a).unwrap().unwrap();
    assert!(
        st.live.same_arrangement(&mixing()),
        "Amazing Grace shows Mixing as saved"
    );
    assert!(st.decks.get(&b).unwrap().modified(FormClass::Desk));
    assert!(!st.decks.get(&a).unwrap().modified(FormClass::Desk));
    assert_eq!(
        st.screensets
            .get("mixing")
            .unwrap()
            .layout_for(FormClass::Desk),
        Some(&mixing()),
        "the screenset is untouched"
    );
    // Back: as it was left.
    st.activate(&b).unwrap().unwrap();
    assert!(st.live.same_arrangement(&slow_river));
    // ⌃Tab round the end.
    st.cycle(1).unwrap().unwrap();
    assert_eq!(st.decks.active.as_ref(), Some(&a));
    assert_eq!(
        st.activate(&a).unwrap(),
        Err((a.clone(), Unchanged::AlreadyActive))
    );
}

#[test]
fn a_context_open_already_comes_forward_and_here_replaces_the_deck_on_show() {
    let mut st = state();
    let a = st
        .open(song("a", "A"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    let b = st
        .open(song("b", "B"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    // A is open: loading it here brings its deck forward.
    assert_eq!(
        st.open(song("a", "A"), Load::Here, None, None).unwrap(),
        Ok(a.clone())
    );
    assert_eq!(st.decks.decks.len(), 2);
    assert_eq!(
        st.open(song("a", "A"), Load::Here, None, None).unwrap(),
        Err((a.clone(), Unchanged::AlreadyOpen))
    );
    // C, loaded here, replaces A on its deck.
    assert_eq!(
        st.open(song("c", "C"), Load::Here, None, None).unwrap(),
        Ok(a)
    );
    assert_eq!(ids_of(&st), ["c", "b"]);
    // A new deck goes after the one on show.
    st.open(song("d", "D"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(ids_of(&st), ["c", "d", "b"]);
    st.move_to(&b, 0).unwrap().unwrap();
    assert_eq!(ids_of(&st), ["b", "c", "d"]);
    // Close the one on show: its right neighbour shows; reopen brings it.
    let d = st.decks.active.clone().unwrap();
    st.close(&d).unwrap().unwrap();
    assert_eq!(ids_of(&st), ["b", "c"]);
    assert_eq!(st.decks.active_deck().unwrap().context.id, "c");
    st.reopen().unwrap();
    assert_eq!(st.decks.active_deck().unwrap().context.id, "d");
    // Duplicate: a second deck on the same context, as it is now.
    arrange(&mut st);
    let copy = st
        .duplicate(&st.decks.active.clone().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(st.decks.get(&copy).unwrap().context.id, "d");
    assert!(st.decks.get(&copy).unwrap().modified(FormClass::Desk));
    // ⌘9 is the last.
    st.activate_nth(0).unwrap().unwrap();
    st.activate_nth(8).unwrap().unwrap();
    assert_eq!(st.decks.active.as_ref(), Some(&copy));
}

#[test]
fn switch_save_as_and_reset_a_decks_screenset() {
    let mut st = state();
    let a = st
        .open(song("a", "A"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    arrange(&mut st);
    st.record();
    assert!(st.decks.get(&a).unwrap().modified(FormClass::Desk));
    // Reset: the screenset as saved.
    st.reset(&a).unwrap().unwrap();
    assert!(st.live.same_arrangement(&mixing()));
    assert_eq!(
        st.reset(&a).unwrap(),
        Err((a.clone(), Unchanged::NotModified))
    );
    // Switch its screenset: the layout follows, the override goes.
    arrange(&mut st);
    st.set_screenset(&a, "charts---lyrics").unwrap().unwrap();
    assert!(st.live.same_arrangement(&charts()));
    assert!(!st.decks.get(&a).unwrap().modified(FormClass::Desk));
    assert_eq!(st.screensets.active.as_deref(), Some("charts---lyrics"));
    // A screenset loaded while it shows (⌘⌥2, WORKSPACES_LOAD) is its own.
    st.live = st.screensets.recall("mixing", FormClass::Desk).unwrap();
    st.record();
    assert_eq!(st.decks.get(&a).unwrap().screenset, "mixing");
    // Save as screenset: its layout a named set it now uses; others can.
    arrange(&mut st);
    let made = st.save_as_screenset(&a, "Mixing + Lyrics").unwrap();
    assert_eq!(made, "mixing---lyrics");
    assert_eq!(st.decks.get(&a).unwrap().screenset, made);
    assert!(!st.decks.get(&a).unwrap().modified(FormClass::Desk));
    let saved = st
        .screensets
        .get(&made)
        .unwrap()
        .layout_for(FormClass::Desk)
        .unwrap()
        .clone();
    assert!(
        saved
            .pane(&architect_workspace::PaneId::new("lyrics"))
            .is_some()
    );
    let b = st
        .open(song("b", "B"), Load::NewDeck, Some(&made), None)
        .unwrap()
        .unwrap();
    assert!(st.live.same_arrangement(&saved));
    assert_eq!(st.decks.get(&b).unwrap().screenset, made);
}

#[test]
fn a_types_default_is_the_persons_then_a_templates_then_the_active() {
    let mut st = state();
    assert_eq!(st.screenset_for("song"), "mixing");
    st.decks.set_default("song", Some("charts---lyrics"));
    assert_eq!(st.screenset_for("song"), "charts---lyrics");
    st.decks.set_default("song", None);
    assert_eq!(st.screenset_for("song"), "mixing");
    // No template for projects: the first screenset.
    assert_eq!(st.screenset_for("project"), "mixing");
    // A template naming a screenset that is not there is passed over.
    st.templates = vec![DeckTemplate::new("song", "gone")];
    assert_eq!(st.screenset_for("song"), "mixing");
}

/// Saves the decks as the app does: the session per device, each deck's
/// screenset and override with its context.
fn keep(st: &DeckState, session: &MemoryStore, contexts: &dyn ContextStore) {
    store::save_session(session, &st.decks);
    for d in &st.decks.decks {
        store::save_context(contexts, d);
    }
}

/// Starts again from what was kept: the decks, the one on show live.
fn restart(session: &MemoryStore, contexts: &dyn ContextStore) -> DeckState {
    let decks = store::load_decks(session, contexts);
    let mut st = DeckState::new(decks, screensets(), Workspace::new())
        .with_templates(vec![DeckTemplate::new("song", "mixing")]);
    st.show();
    st
}

#[test]
fn an_override_survives_a_restart_and_the_session_restores_the_strip() {
    let (session, contexts) = (MemoryStore::new(), MemoryContextStore::new());
    let mut st = state();
    st.open(song("a", "A"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    let b = st
        .open(song("b", "B"), Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    arrange(&mut st);
    st.record();
    let left = st.live.clone();
    keep(&st, &session, &contexts);
    assert_eq!(contexts.len(), 2);

    let mut again = restart(&session, &contexts);
    assert_eq!(ids_of(&again), ["a", "b"]);
    assert_eq!(again.decks.active.as_ref(), Some(&b));
    assert!(again.live.same_arrangement(&left), "B as it was left");
    let a = again.decks.decks[0].id.clone();
    again.activate(&a).unwrap().unwrap();
    assert!(again.live.same_arrangement(&mixing()), "A as saved");

    // Closed and opened again later: the context keeps its layout.
    again.close(&b).unwrap().unwrap();
    keep(&again, &session, &contexts);
    let mut later = restart(&session, &contexts);
    assert_eq!(ids_of(&later), ["a"]);
    let kept = store::load_context(&contexts, &song("b", "B"));
    later
        .open(song("b", "B"), Load::NewDeck, None, kept)
        .unwrap()
        .unwrap();
    assert!(later.live.same_arrangement(&left));
}

#[test]
fn a_contexts_layout_is_kept_in_its_folder_or_under_the_vault() {
    let dir = std::env::temp_dir().join(format!("decks-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = ContextFiles::new(&dir);
    let project = DeckContext::new("project", "Projects/Album", "Album").folder("Projects/Album");
    assert_eq!(
        files.path_for(&project),
        dir.join("Projects/Album/.architect/deck.json")
    );
    assert_eq!(
        files.path_for(&song("Songs/Slow River.md", "Slow River")),
        dir.join(".architect/decks/song/Songs-Slow River-md.json")
    );
    let session = FileStore(dir.join("decks.json"));
    let mut st = state();
    st.open(project, Load::NewDeck, None, None)
        .unwrap()
        .unwrap();
    arrange(&mut st);
    st.record();
    let left = st.live.clone();
    store::save_session(&session, &st.decks);
    for d in &st.decks.decks {
        store::save_context(&files, d);
    }
    assert!(dir.join("Projects/Album/.architect/deck.json").is_file());
    let decks = store::load_decks(&session, &files);
    let mut again = DeckState::new(decks, screensets(), Workspace::new());
    again.show();
    assert!(again.live.same_arrangement(&left));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The app's `resolve`: a project given by its id alone gets its folder.
struct Projects;

impl architect_workspace::control::deck::Kept for Projects {
    fn kept(&self, _: &DeckContext) -> Option<architect_workspace::ContextLayout> {
        None
    }

    fn resolve(&self, mut context: DeckContext) -> DeckContext {
        if context.kind == "project" && context.folder.is_none() {
            context.folder = Some(context.id.clone());
        }
        context
    }
}

#[test]
fn deck_open_resolves_a_context_given_by_its_id() {
    use architect_workspace::control::deck::{DeckOpenArgs, open};
    let mut st = state();
    let a = DeckOpenArgs {
        context: DeckContext::new("project", "Projects/Album", "Album"),
        new: true,
        screenset: None,
    };
    let out = open(&mut st, &Projects, a).unwrap();
    assert!(out.changed);
    let deck = st.decks.active_deck().unwrap();
    assert_eq!(deck.context.folder.as_deref(), Some("Projects/Album"));
}

struct Model {
    server: Server,
    decks: HeadlessDecks,
}

impl Model {
    fn new() -> Self {
        let list = ActionList::new();
        let decks = HeadlessDecks::new(state());
        decks.install(&list).expect("installed");
        Self {
            server: Server::new("decks", "0", list),
            decks,
        }
    }

    fn tool(&self, id: &str, args: &Value) -> (Value, bool) {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "run", "arguments": {"id": id, "args": args}}});
        let reply: Value =
            serde_json::from_str(&self.server.handle(&request.to_string()).expect("a reply"))
                .expect("json");
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text");
        let value = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
        (value, result["isError"].as_bool().unwrap_or(false))
    }

    fn run(&self, id: &str, args: Value) -> Value {
        let (out, failed) = self.tool(id, &args);
        assert!(!failed, "{id} {args} failed: {out}");
        out
    }

    fn fails(&self, id: &str, args: Value) -> String {
        let (out, failed) = self.tool(id, &args);
        assert!(failed, "{id} {args} should fail, gave {out}");
        out.to_string()
    }
}

#[test]
fn the_deck_actions_over_mcp() {
    assert!(ids::ALL.iter().all(|id| id.starts_with(ids::PREFIX)));
    let m = Model::new();
    let song_a = json!({"kind": "song", "id": "a", "title": "A"});
    let song_b = json!({"kind": "song", "id": "b", "title": "B"});
    let out = m.run(ids::OPEN, json!({"context": song_a, "new": true}));
    assert_eq!(out["changed"], true);
    let a = out["deck"].as_str().unwrap().to_owned();
    let out = m.run(ids::OPEN, json!({"context": song_b, "new": true}));
    let b = out["deck"].as_str().unwrap().to_owned();
    assert_eq!(out["list"]["decks"][1]["screenset_name"], "Mixing");
    // Already on show.
    let out = m.run(ids::OPEN, json!({"context": song_b}));
    assert_eq!(
        (out["changed"].clone(), out["reason"].clone()),
        (json!(false), json!("already_open"))
    );
    // Rearranged by the person, then listed as modified.
    m.decks.arrange(|w| {
        let _ = w.open(Pane::new("lyrics", "song.lyrics", "Lyrics"), Target::Tab);
    });
    let list = m.run(ids::LIST, json!({}));
    assert_eq!(list["active"], b.as_str());
    assert_eq!(list["decks"][1]["modified"], true);
    assert_eq!(list["decks"][0]["modified"], false);
    assert_eq!(
        list["defaults"][0],
        json!({"kind": "song", "screenset": "mixing", "by": "app"})
    );
    // Next and previous, round the ends.
    assert_eq!(m.run(ids::NEXT, json!({}))["list"]["active"], a.as_str());
    assert_eq!(m.run(ids::PREV, json!({}))["list"]["active"], b.as_str());
    let out = m.run(ids::MOVE, json!({"deck": b, "index": 0}));
    assert_eq!(out["list"]["decks"][0]["id"], b.as_str());
    // Save B's layout as a screenset; A switched onto it.
    let out = m.run(ids::SAVE_AS_SCREENSET, json!({"name": "Lyrics up"}));
    let made = out["screenset"].as_str().unwrap().to_owned();
    assert_eq!(out["list"]["decks"][0]["modified"], false);
    let out = m.run(ids::SET_SCREENSET, json!({"deck": a, "screenset": made}));
    assert_eq!(out["list"]["decks"][1]["screenset"], made.as_str());
    assert!(
        m.fails(ids::SET_SCREENSET, json!({"screenset": "nope"}))
            .contains("nope")
    );
    let out = m.run(ids::RESET, json!({}));
    assert_eq!(out["reason"], "not_modified");
    // A person's default for songs.
    let out = m.run(
        ids::SET_DEFAULT,
        json!({"kind": "song", "screenset": "charts---lyrics"}),
    );
    assert_eq!(out["list"]["defaults"][0]["by"], "person");
    // Close B; reopen it as it was.
    let out = m.run(ids::CLOSE, json!({"deck": b}));
    assert_eq!(out["list"]["closed"], 1);
    let out = m.run(ids::REOPEN, json!({}));
    assert_eq!(out["list"]["active"], b.as_str());
    let out = m.run(ids::DUPLICATE, json!({}));
    assert_eq!(out["list"]["decks"].as_array().unwrap().len(), 3);
    let out = m.run(ids::ACTIVATE, json!({"index": 0}));
    assert_eq!(out["list"]["active"], a.as_str());
    assert_eq!(m.decks.state().decks.decks.len(), 3);
    let _ = DeckId::new("x");
}

/// What an extension ships with `[[deck_templates]] layout`.
const SONG_LAYOUT: &str = r#"{"windows":[{"id":"main",
  "center":{"split":{"axis":"row","children":[
    {"share":0.62,"node":{"group":{"tabs":[{"pane":"chart","view":"song.chart","title":"Chart"}]}}},
    {"share":0.38,"node":{"group":{"tabs":[{"pane":"lyrics","view":"song.lyrics","title":"Lyrics"}]}}}]}},
  "docks":[{"side":"left","root":{"group":{"tabs":[{"pane":"files","view":"files","title":"Files"}]}}}]}],
  "active":"chart"}"#;

#[test]
fn a_templates_shipped_layout_is_a_built_in_screenset() {
    use architect_workspace::control::{ViewSpec, deck::template_screenset};
    let views = vec![
        ViewSpec::new("song.chart", "Chart"),
        ViewSpec::new("song.lyrics", "Lyrics"),
        ViewSpec::new("files", "Files"),
    ];
    let set = template_screenset("Song", SONG_LAYOUT, views.clone()).unwrap();
    assert_eq!(set.id, "song");
    assert!(set.builtin);
    let w = set.layout_for(FormClass::Desk).unwrap();
    assert_eq!(w.panes.len(), 3);
    assert!(template_screenset("Song", SONG_LAYOUT, views[..1].to_vec()).is_err());
    assert!(template_screenset("Song", "{}", views).is_err());
}
