//! Every script in `scripts/`, run against a fresh board: what it returns,
//! what it changed, and the undo step it made.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use architect::action::{ActionCaller, ActionError};
use architect_scripting::luau::Report;
use architect_scripting::{ErrorKind, Script};
use example_scripting::{App, viewer};
use futures::executor::block_on;
use serde_json::{Value, json};

/// The scripts these tests cover, each by one test below: a script added
/// to `scripts/` without a test here fails `every_script_has_a_test`.
const COVERED: [&str; 5] = [
    "broken.luau",
    "burndown.luau",
    "close_tag.luau",
    "inventory.luau",
    "tidy.luau",
];

fn script(name: &str) -> String {
    let path = scripts_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn scripts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts")
}

fn app() -> App {
    let app = App::new().expect("the app starts");
    app.list.validate().expect("a sound list");
    app
}

fn run(app: &App, name: &str, args: Value) -> Report {
    let step = format!("Script: {}", name.trim_end_matches(".luau"));
    block_on(
        app.scripting
            .run(&ActionCaller::app(), &script(name), args, &step),
    )
}

/// Which tasks are done, by id.
fn done(app: &App) -> Vec<String> {
    app.board
        .tasks()
        .into_iter()
        .filter(|t| t.done)
        .map(|t| t.id)
        .collect()
}

#[test]
fn every_script_has_a_test() {
    let mut found: Vec<String> = std::fs::read_dir(scripts_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(found, COVERED);
}

#[test]
fn inventory_reads_and_changes_nothing() {
    let app = app();
    let report = run(&app, "inventory.luau", Value::Null);
    let out = report.result.expect("ran");
    assert_eq!(out["actions"], 4);
    assert_eq!(out["add_takes"], json!(["title"]));
    assert_eq!(out["open"], 3);
    assert_eq!(out["by_tag"], json!({"home": 2, "studio": 1}));
    assert_eq!(
        report.log,
        [
            "TASKS_ADD\tmutating",
            "TASKS_COMPLETE\tmutating",
            "TASKS_LIST\tread_only",
            "TASKS_REMOVE\tdestructive",
        ]
    );
    assert_eq!(report.ran, ["TASKS_LIST"]);
    assert_eq!(report.undo, None, "it changed nothing: no step");
    assert!(app.board.steps().is_empty());
}

#[test]
fn close_tag_branches_loops_and_undoes_as_one_step() {
    let app = app();
    let report = run(&app, "close_tag.luau", json!({"tag": "home"}));
    assert_eq!(report.result, Ok(json!(2)));
    assert_eq!(
        report.log,
        [
            "done:\tBuy milk",
            "done:\tFix the gate",
            "already done:\tCall the plumber"
        ]
    );
    assert_eq!(
        report.ran,
        ["TASKS_LIST", "TASKS_COMPLETE", "TASKS_COMPLETE"]
    );
    assert_eq!(done(&app), ["t1", "t2", "t3", "t5"]);

    // Two actions, each a transaction of its own: one step, one Undo.
    assert_eq!(report.undo.as_deref(), Some("Script: close_tag"));
    assert_eq!(app.board.steps(), ["Script: close_tag"]);
    assert_eq!(app.board.undo().as_deref(), Some("Script: close_tag"));
    assert_eq!(done(&app), ["t3", "t5"]);

    // A tag no open task has: nothing closed, and no step.
    let other = run(&app, "close_tag.luau", json!({"tag": "garden"}));
    assert_eq!((other.result, other.undo), (Ok(json!(0)), None));
}

#[test]
fn tidy_removes_what_is_done_and_adds_when_work_runs_out() {
    let app = app();
    let report = run(&app, "tidy.luau", Value::Null);
    assert_eq!(
        report.result,
        Ok(json!({"removed": 2, "added": "t6"})),
        "{report:?}"
    );
    let titles: Vec<String> = app.board.tasks().into_iter().map(|t| t.title).collect();
    assert_eq!(
        titles,
        [
            "Buy milk",
            "Fix the gate",
            "Review the mix",
            "Plan next week"
        ]
    );
    assert_eq!(app.board.steps(), ["Script: tidy"]);

    // Run again: nothing done to remove, and work to do, so no step.
    let again = run(&app, "tidy.luau", Value::Null);
    assert_eq!(
        (again.result, again.undo),
        (Ok(json!({"removed": 0})), None)
    );
}

#[test]
fn broken_says_its_line_and_takes_back_what_it_changed() {
    let app = app();
    let report = run(&app, "broken.luau", Value::Null);
    let error = report.result.expect_err("it fails");
    assert_eq!(error.kind, ErrorKind::Action);
    assert_eq!(error.line, Some(9), "{error}");
    assert!(error.message.contains("there is no task t99"), "{error}");
    assert_eq!(report.log, ["completed\tBuy milk"]);
    assert_eq!(
        report.ran,
        ["TASKS_LIST", "TASKS_COMPLETE", "TASKS_COMPLETE"]
    );
    assert_eq!(report.undo, None);
    assert_eq!(
        done(&app),
        ["t3", "t5"],
        "t1 was completed, then taken back"
    );
    assert!(app.board.steps().is_empty());

    // Run as an action (`SCRIPT_RUN`), the failure is the action's.
    let failed = block_on(
        app.list.run_json_async(
            "SCRIPT_RUN",
            json!({"source": script("broken.luau")})
                .to_string()
                .as_bytes(),
        ),
    );
    let Err(ActionError::Failed { message, .. }) = failed else {
        panic!("failed: {failed:?}");
    };
    assert!(message.starts_with("line 9: "), "{message}");
    assert!(message.contains("taken back"), "{message}");
}

#[test]
fn burndown_is_live_one_step_a_frame() {
    let app = app();
    let runtime = app.runtime();
    let (first, live) = block_on(runtime.start(
        &ActionCaller::app(),
        &script("burndown.luau"),
        Value::Null,
        "Script: burndown",
    ));
    assert_eq!(first.result, Ok(json!("started")));
    assert_eq!(first.undo, None, "the first run only defers");
    let live = live.expect("it started");

    let mut logs = Vec::new();
    while live.running() {
        let frame = block_on(live.frame());
        assert!(frame.result.is_ok(), "{frame:?}");
        logs.extend(frame.log);
        assert!(logs.len() < 10, "it stops: {logs:?}");
    }
    assert_eq!(
        logs,
        [
            "frame\t1\tdone:\tBuy milk",
            "frame\t2\tdone:\tFix the gate",
            "frame\t3\tdone:\tReview the mix",
            "frame\t4\tnothing left",
        ]
    );
    assert_eq!(done(&app), ["t1", "t2", "t3", "t4", "t5"]);
    assert_eq!(app.board.steps(), ["Script: burndown"; 3]);
    assert_eq!(app.board.undo().as_deref(), Some("Script: burndown"));
    assert_eq!(done(&app), ["t1", "t2", "t3", "t5"], "one frame undone");

    // Run once, there is no `defer`: a live script needs a live run.
    let once = run(&app, "burndown.luau", Value::Null);
    assert_eq!(once.result.map_err(|e| e.kind), Err(ErrorKind::Runtime));
}

#[test]
fn a_saved_script_is_an_action_the_palette_runs() {
    let app = app();
    let id = app
        .scripting
        .save(Script::new("Tidy the board", script("tidy.luau")).described("Tidies the board."))
        .expect("saved");
    assert_eq!(id, "SCRIPT_MY_TIDY_THE_BOARD");

    // ⌘K lists every action on the list and runs the one picked by its id.
    let info = app.list.info(&id).expect("on the list");
    assert_eq!(info.display_name, "Tidy the board");
    assert_eq!(info.description, "Tidies the board.");
    assert_eq!(info.group, "Your scripts");
    assert!(info.params_schema.is_none(), "a key runs it with nothing");
    app.list.validate().expect("a sound list");

    let out: Value =
        serde_json::from_slice(&block_on(app.list.run_json_async(&id, b"")).unwrap()).unwrap();
    assert_eq!(out["result"], json!({"removed": 2, "added": "t6"}));
    assert_eq!(out["undo"], "Script: Tidy the board");
    assert_eq!(app.board.steps(), ["Script: Tidy the board"]);
}

#[test]
fn a_viewer_is_refused_at_the_line_that_changes_the_board() {
    let app = app();
    let vic = ActionCaller::new(viewer());
    let report = block_on(app.scripting.run(
        &vic,
        &script("close_tag.luau"),
        Value::Null,
        "Script: close_tag",
    ));
    let error = report.result.expect_err("refused");
    assert_eq!(error.kind, ErrorKind::Action);
    assert_eq!(error.line, Some(11), "{error}");
    assert!(error.message.contains("not allowed"), "{error}");
    assert_eq!(
        report.ran,
        ["TASKS_LIST", "TASKS_COMPLETE"],
        "it read first"
    );
    assert_eq!(done(&app), ["t3", "t5"]);

    // What a viewer's script sees is what a viewer may run.
    let inventory = block_on(app.scripting.run(
        &vic,
        &script("inventory.luau"),
        Value::Null,
        "Script: inventory",
    ));
    assert_eq!(inventory.result.expect("it only reads")["actions"], 1);
}
