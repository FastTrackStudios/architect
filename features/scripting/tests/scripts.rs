//! Scripts end to end, over a small app: tracks in a Loro document, two
//! actions (`TRACKS_SELECTED`, read-only; `TRACKS_ARM`, one undo step a
//! track), and its undo history.
//!
//! The issue's bar: a script "arm every selected track" runs from ⌘K (the
//! palette runs it by its action id, from the list) and undoes as one
//! step, and a model writes and runs it over MCP.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use architect::action::{
    ActionCaller, ActionError, ActionList, ActionMeta, Decision, Effect, PermissionEngine,
    Principal,
};
use architect_permissions::{Action, Resource};
use architect_scripting::actions::Scripting;
use architect_scripting::history::HistoryTransactions;
use architect_scripting::luau::{Limits, Runtime};
use architect_scripting::{ErrorKind, Library, MemoryStore, Scope, Script};
use crdt::History;
use crdt::loro::{LoroDoc, LoroMap, LoroValue};
use facet::Facet;
use futures::executor::block_on;
use serde_json::{Value, json};

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Tracks",
        method_name: id,
        display_name: id,
        description: "a test action",
        category: "Tracks",
        group: "Tracks",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static SELECTED: ActionMeta = meta("TRACKS_SELECTED");
static ARM: ActionMeta = meta("TRACKS_ARM");

#[derive(Facet)]
struct Arm {
    id: String,
    on: bool,
}

const ARM_SELECTED: &str = r#"-- Arm every selected track
local selected = state.query("TRACKS_SELECTED")
for _, id in selected do
  actions.run("TRACKS_ARM", { id = id, on = true })
  print("armed", id)
end
return #selected
"#;

/// The app: four tracks, two of them selected, none armed.
struct App {
    list: ActionList,
    doc: LoroDoc,
    history: Arc<Mutex<History>>,
}

impl App {
    fn new() -> Self {
        let doc = LoroDoc::new();
        let tracks = doc.get_map("tracks");
        for (id, selected) in [
            ("kick", true),
            ("snare", false),
            ("bass", true),
            ("vox", false),
        ] {
            let track = tracks.insert_container(id, LoroMap::new()).expect("track");
            track.insert("selected", selected).expect("selected");
            track.insert("armed", false).expect("armed");
        }
        doc.commit();
        let history = Arc::new(Mutex::new(History::new(&doc, 100)));
        let list = ActionList::new();
        let d = doc.clone();
        list.register(&SELECTED, Effect::ReadOnly, move |(): ()| {
            Ok(flags(&d, "selected")
                .into_iter()
                .filter_map(|(id, on)| on.then_some(id))
                .collect::<Vec<String>>())
        })
        .expect("registered");
        let (d, h) = (doc.clone(), Arc::clone(&history));
        list.register(&ARM, Effect::Mutating, move |a: Arm| {
            let track = d
                .get_map("tracks")
                .get(&a.id)
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_map().ok())
                .ok_or_else(|| format!("no track {}", a.id))?;
            h.lock().unwrap().transact(&d, "Arm track", |_| {
                track.insert("armed", a.on).map_err(|e| e.to_string())
            })
        })
        .expect("registered");
        Self { list, doc, history }
    }

    fn scripting(&self) -> Scripting {
        let scripting = Scripting::new(
            self.list.clone(),
            Library::new(MemoryStore::new()).with_org(MemoryStore::new()),
        )
        .with_transactions(HistoryTransactions::new(
            self.doc.clone(),
            Arc::clone(&self.history),
        ));
        scripting.install().expect("installed");
        scripting
    }

    fn armed(&self) -> Vec<String> {
        flags(&self.doc, "armed")
            .into_iter()
            .filter_map(|(id, on)| on.then_some(id))
            .collect()
    }

    fn steps(&self) -> Vec<String> {
        self.history.lock().unwrap().steps().to_vec()
    }

    fn undo(&self) -> Option<String> {
        self.history.lock().unwrap().undo()
    }
}

fn flags(doc: &LoroDoc, key: &str) -> Vec<(String, bool)> {
    let LoroValue::Map(tracks) = doc.get_map("tracks").get_deep_value() else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = tracks
        .iter()
        .map(|(id, track)| {
            let on = match track {
                LoroValue::Map(m) => matches!(m.get(key), Some(LoroValue::Bool(true))),
                _ => false,
            };
            (id.clone(), on)
        })
        .collect();
    out.sort();
    out
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("json")
}

#[test]
fn a_saved_script_runs_from_the_palette_and_undoes_as_one_step() {
    let app = App::new();
    let _scripting = app.scripting();
    let saved = block_on(
        app.list.run_json_async(
            "SCRIPT_SAVE",
            json!({
                "name": "Arm every selected track",
                "source": ARM_SELECTED,
                "description": "Arms the selected tracks.",
            })
            .to_string()
            .as_bytes(),
        ),
    )
    .expect("saved");
    let id = "SCRIPT_MY_ARM_EVERY_SELECTED_TRACK";
    assert_eq!(json_of(&saved)["id"], id);

    // ⌘K lists every action on the list, and runs the one picked by id.
    let listed = app
        .list
        .infos()
        .into_iter()
        .find(|i| i.id == id)
        .expect("in ⌘K");
    assert_eq!(listed.display_name, "Arm every selected track");
    assert_eq!(listed.description, "Arms the selected tracks.");
    assert_eq!(listed.group, "Your scripts");
    assert!(listed.params_schema.is_none(), "a key runs it with nothing");
    app.list.validate().expect("a sound list");

    let out = json_of(&block_on(app.list.run_json_async(id, b"")).expect("ran"));
    assert_eq!(out["result"], 2);
    assert_eq!(out["log"], json!(["armed\tbass", "armed\tkick"]));
    assert_eq!(
        out["ran"],
        json!(["TRACKS_SELECTED", "TRACKS_ARM", "TRACKS_ARM"])
    );
    assert_eq!(out["undo"], "Script: Arm every selected track");
    assert_eq!(app.armed(), ["bass", "kick"]);

    // Two actions, each a transaction of its own: one step.
    assert_eq!(app.steps(), ["Script: Arm every selected track"]);
    assert_eq!(
        app.undo().as_deref(),
        Some("Script: Arm every selected track")
    );
    assert!(app.armed().is_empty());
}

#[test]
fn a_model_writes_runs_and_saves_a_script_over_mcp() {
    let app = App::new();
    let _scripting = app.scripting();
    let model = Principal::Service {
        name: "model".into(),
    };
    let caller = ActionCaller::new(model);
    let server = architect_mcp::Server::new("Tracks", "0", app.list.clone())
        .with_tools(architect_mcp::Tools::Both);

    let tools = server.tools_for(&Principal::Anonymous);
    for tool in ["script_run", "script_save", "script_describe"] {
        assert!(
            tools.iter().any(|t| t["name"] == tool),
            "{tool} is offered: {tools:?}"
        );
    }

    let described = block_on(server.call_tool(&caller, "script_describe", json!({}))).unwrap();
    let text = described["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("actions.run(id, args?)"), "{text}");
    assert!(
        text.contains("TRACKS_ARM"),
        "the actions it may run: {text}"
    );

    let ran = block_on(server.call_tool(
        &caller,
        "script_run",
        json!({ "source": ARM_SELECTED, "name": "Arm selected" }),
    ))
    .unwrap();
    assert_eq!(ran["isError"], false, "{ran}");
    let report: Value = serde_json::from_str(ran["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["result"], 2);
    assert_eq!(app.armed(), ["bass", "kick"]);
    assert_eq!(app.steps(), ["Script: Arm selected"]);

    // The generic `run` reaches it too.
    let saved = block_on(server.call_tool(
        &caller,
        "run",
        json!({ "id": "SCRIPT_SAVE_ORG", "args": { "name": "Arm selected", "source": ARM_SELECTED } }),
    ))
    .unwrap();
    assert_eq!(saved["isError"], false, "{saved}");
    assert!(app.list.info("SCRIPT_ORG_ARM_SELECTED").is_some());
}

#[test]
fn a_failure_says_its_line_and_takes_back_what_the_script_changed() {
    let app = App::new();
    let scripting = app.scripting();
    let source = "actions.run(\"TRACKS_ARM\", { id = \"vox\", on = true })\n\
                  print(\"halfway\")\n\
                  actions.run(\"TRACKS_ARM\", { id = \"nope\", on = true })\n";
    let report = block_on(scripting.run(&ActionCaller::app(), source, Value::Null, "Script"));
    let error = report.result.expect_err("failed");
    assert_eq!(error.kind, ErrorKind::Action);
    assert_eq!(error.line, Some(3), "{error}");
    assert!(error.message.contains("no track nope"), "{error}");
    assert_eq!(report.log, ["halfway"]);
    assert!(app.armed().is_empty(), "vox was armed, then taken back");
    assert!(app.steps().is_empty());

    // As an action, the failure is the action's, with the line.
    let failed = block_on(app.list.run_json_async(
        "SCRIPT_RUN",
        json!({ "source": source }).to_string().as_bytes(),
    ));
    let Err(ActionError::Failed { message, .. }) = failed else {
        panic!("failed: {failed:?}");
    };
    assert!(message.starts_with("line 3: "), "{message}");
    assert!(message.contains("taken back"), "{message}");
    assert!(message.contains("halfway"), "the log: {message}");
}

#[test]
fn errors_luau_raises_say_their_line() {
    let app = App::new();
    let scripting = app.scripting();
    let run = |source: &str| {
        block_on(scripting.run(&ActionCaller::app(), source, Value::Null, "Script"))
            .result
            .expect_err("failed")
    };

    let syntax = run("local a = 1\nlocal b = = 2\n");
    assert_eq!(
        (syntax.kind, syntax.line),
        (ErrorKind::Syntax, Some(2)),
        "{syntax}"
    );

    let runtime = run("local a = 1\n\nlocal b = nil + a\n");
    assert_eq!(
        (runtime.kind, runtime.line),
        (ErrorKind::Runtime, Some(3)),
        "{runtime}"
    );
    assert!(runtime.message.contains("arithmetic"), "{runtime}");

    let raised = run("local function f()\n  error(\"no tracks\")\nend\nf()\n");
    assert_eq!(raised.line, Some(2), "{raised}");
    assert_eq!(raised.message, "no tracks");

    let unknown = run("\nactions.run(\"TRACKS_NOPE\")\n");
    assert_eq!(
        (unknown.kind, unknown.line),
        (ErrorKind::Action, Some(2)),
        "{unknown}"
    );

    let changes = run("state.query(\"TRACKS_ARM\", { id = \"kick\", on = true })\n");
    assert!(changes.message.contains("actions.run"), "{changes}");
    assert!(app.armed().is_empty());

    // Saving checks it first, and names the line.
    let refused = scripting.save(Script::new("Broken", "local x =\n"));
    assert!(refused.is_err_and(|e| e.starts_with("line ")));
    assert!(app.list.info("SCRIPT_MY_BROKEN").is_none());
}

#[test]
fn a_script_branches_loops_and_catches_what_fails() {
    let app = App::new();
    let scripting = app.scripting();
    let source = r#"
local armed = 0
for _, track in { "kick", "nope", "vox" } do
  local ok = pcall(actions.run, "TRACKS_ARM", { id = track, on = true })
  if ok then armed += 1 end
end
local found = actions.list("tracks_")
local enabled, why = actions.enabled("TRACKS_NOPE")
return { armed = armed, listed = #found, arm_takes = actions.describe("TRACKS_ARM").params.required, enabled = enabled, why = why, given = args.n }
"#;
    let report = block_on(scripting.run(
        &ActionCaller::app(),
        source,
        json!({ "n": 7 }),
        "Script: Try",
    ));
    let out = report.result.expect("ran");
    assert_eq!(out["armed"], 2);
    assert_eq!(out["listed"], 2);
    assert_eq!(out["arm_takes"], json!(["id", "on"]));
    assert_eq!(out["enabled"], false);
    assert_eq!(out["why"], "no action TRACKS_NOPE");
    assert_eq!(out["given"], 7);
    assert_eq!(app.armed(), ["kick", "vox"]);
    assert_eq!(report.undo.as_deref(), Some("Script: Try"));
}

#[test]
fn a_script_cannot_run_away_or_change_its_globals() {
    let app = App::new();
    let runtime = Runtime::new(app.list.clone()).with_limits(Limits {
        steps: 10_000,
        ..Limits::default()
    });
    let forever =
        block_on(runtime.run(&ActionCaller::app(), "while true do end", Value::Null, "S"));
    let error = forever.result.expect_err("stopped");
    assert_eq!(
        (error.kind, error.line),
        (ErrorKind::Limit, Some(1)),
        "{error}"
    );

    let greedy = block_on(
        Runtime::new(app.list)
            .with_limits(Limits {
                memory: 1024 * 1024,
                ..Limits::default()
            })
            .run(
                &ActionCaller::app(),
                "local t = {}\nfor i = 1, 1e7 do t[i] = string.rep(\"x\", 64) end",
                Value::Null,
                "S",
            ),
    );
    assert_eq!(greedy.result.expect_err("stopped").kind, ErrorKind::Limit);

    let tamper = block_on(runtime.run(&ActionCaller::app(), "actions.run = nil", Value::Null, "S"));
    assert!(tamper.result.is_err(), "the API is read-only");

    // What one run sets, the next does not see.
    let first = block_on(runtime.run(&ActionCaller::app(), "seen = 1", Value::Null, "S"));
    assert!(first.result.is_ok());
    let second = block_on(runtime.run(&ActionCaller::app(), "return seen", Value::Null, "S"));
    assert_eq!(second.result, Ok(Value::Null));
}

/// Lets the model read, not change.
struct ReadOnlyModel;

impl PermissionEngine for ReadOnlyModel {
    fn check(&self, _: &Principal, _: &Resource, _: &Action) -> Decision {
        Decision::Allow
    }

    fn authorize(&self, who: &Principal, action_id: &str) -> Decision {
        match who {
            Principal::Service { name } if name == "model" && action_id == "TRACKS_ARM" => {
                Decision::deny("the model may only read")
            }
            _ => Decision::Allow,
        }
    }
}

#[test]
fn a_script_runs_as_whoever_ran_it() {
    let app = App::new();
    app.list.set_authorizer(ReadOnlyModel);
    let scripting = app.scripting();
    let model = ActionCaller::new(Principal::Service {
        name: "model".into(),
    });
    let report = block_on(scripting.run(&model, ARM_SELECTED, Value::Null, "Script"));
    let error = report.result.expect_err("denied");
    assert_eq!(error.line, Some(4), "{error}");
    assert!(error.message.contains("not allowed"), "{error}");
    assert!(app.armed().is_empty());

    let hidden =
        block_on(scripting.run(&model, "return #actions.list(\"TRACKS\")", Value::Null, "S"));
    assert_eq!(hidden.result, Ok(json!(1)), "it does not see TRACKS_ARM");

    // The app's own door (a key, ⌘K) is trusted.
    let report = block_on(scripting.run(&ActionCaller::app(), ARM_SELECTED, Value::Null, "S"));
    assert!(report.result.is_ok());
    assert_eq!(app.armed(), ["bass", "kick"]);
}

#[test]
fn saving_again_replaces_and_deleting_takes_the_action_away() {
    let app = App::new();
    let scripting = app.scripting();
    let id = scripting
        .save(Script::new("Count", "return 1").described("Counts"))
        .expect("saved");
    assert_eq!(id, "SCRIPT_MY_COUNT");
    scripting
        .save(Script::new("Count", "return 2").described("Counts again"))
        .expect("saved again");
    assert_eq!(app.list.info(&id).unwrap().description, "Counts again");
    let out = json_of(&block_on(app.list.run_json_async(&id, b"")).unwrap());
    assert_eq!(out["result"], 2);

    let listing = json_of(&app.list.run_json("SCRIPT_LIST", b"").unwrap());
    assert_eq!(listing["scripts"][0]["id"], id);
    assert_eq!(listing["scripts"].as_array().map(Vec::len), Some(1));

    let deleted = json_of(
        &app.list
            .run_json("SCRIPT_DELETE", br#"{"name":"count"}"#)
            .unwrap(),
    );
    assert_eq!(deleted["id"], id);
    assert!(app.list.info(&id).is_none());
    assert!(scripting.library().scripts().unwrap().is_empty());

    // A list built later finds what was saved: a script saved for the
    // organization is an action again on the next start.
    let library = Library::new(MemoryStore::new()).with_org(MemoryStore::new());
    library
        .save(Script::new("Shared", "return 3").for_org())
        .unwrap();
    let list = ActionList::new();
    Scripting::new(list.clone(), library).install().unwrap();
    let shared = list.info("SCRIPT_ORG_SHARED").expect("registered at start");
    assert_eq!(shared.group, Scope::Org.group());
}

#[test]
fn without_an_organization_there_is_nothing_to_save_to() {
    let list = ActionList::new();
    Scripting::new(list.clone(), Library::new(MemoryStore::new()))
        .install()
        .unwrap();
    assert!(list.info("SCRIPT_SAVE").is_some());
    assert!(list.info("SCRIPT_SAVE_ORG").is_none());
    list.validate().unwrap();
    list.probe_hostile_args().unwrap();
}
