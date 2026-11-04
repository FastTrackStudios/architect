//! A script's action calls run on the app's Tokio runtime, not on the
//! script's own thread: an action that needs a runtime (a timer, a
//! socket, `tokio::spawn`) works when a Luau script calls it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use architect_scripting::luau::Runtime;
use serde_json::{Value, json};

static NAP: ActionMeta = ActionMeta {
    id: "TEST_NAP",
    trait_name: "Test",
    method_name: "nap",
    display_name: "Nap",
    description: "sleeps on a Tokio timer, then answers from a spawned task",
    category: "Test",
    group: "Test",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// An action that only works inside a Tokio runtime: a timer, then a
/// spawned task's answer.
fn list() -> ActionList {
    let list = ActionList::new();
    list.register_async(&NAP, Effect::ReadOnly, |(): ()| async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let spawned = tokio::spawn(async { "slept".to_owned() });
        spawned.await.map_err(|e| e.to_string())
    })
    .unwrap();
    list
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scripts_action_runs_on_the_apps_runtime() {
    let runtime = Runtime::new(list());
    let report = runtime
        .run(
            &ActionCaller::app(),
            "return actions.run('TEST_NAP')",
            Value::Null,
            "nap",
        )
        .await;
    assert_eq!(report.result, Ok(json!("slept")), "{:?}", report.log);
}

#[tokio::test(flavor = "current_thread")]
async fn a_live_scripts_frame_reaches_the_runtime_too() {
    // A current-thread runtime: the script's thread hands the call back
    // and the caller's runtime drives it while the frame is awaited.
    let runtime = Runtime::new(list());
    let source = "defer(function() print(actions.run('TEST_NAP')) end)";
    let (report, live) = runtime
        .start(&ActionCaller::app(), source, Value::Null, "nap")
        .await;
    assert!(report.result.is_ok(), "{report:?}");
    let frame = live.expect("it started").frame().await;
    assert_eq!(frame.result, Ok(Value::Null), "{frame:?}");
    assert_eq!(frame.log, vec!["slept".to_owned()]);
}

#[test]
fn a_runtime_named_ahead_serves_a_run_started_outside_one() {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let runtime = Runtime::new(list()).with_runtime(tokio.handle().clone());
    let report = futures::executor::block_on(runtime.run(
        &ActionCaller::app(),
        "return actions.run('TEST_NAP')",
        Value::Null,
        "nap",
    ));
    assert_eq!(report.result, Ok(json!("slept")));
}
