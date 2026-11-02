//! An agent over MCP writes, runs and saves the same scripts a person
//! does: `script_describe`, `script_run`, `script_save`, then the saved
//! script as a tool of its own. A viewer's run is refused part-way.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use architect_mcp::Server;
use example_scripting::{App, agent, viewer};
use serde_json::{Value, json};

fn script(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The model's side: one JSON-RPC message, the reply.
fn rpc(server: &Server, method: &str, params: &Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let reply = server.handle(&request.to_string()).expect("a reply");
    serde_json::from_str(&reply).expect("json")
}

fn tools(server: &Server) -> Vec<String> {
    rpc(server, "tools/list", &json!({}))["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
}

/// Calls `tool`: whether it failed, and its text.
fn call(server: &Server, tool: &str, arguments: &Value) -> (bool, String) {
    let reply = rpc(
        server,
        "tools/call",
        &json!({"name": tool, "arguments": arguments}),
    );
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().expect("text");
    (
        result["isError"].as_bool().expect("isError"),
        text.to_owned(),
    )
}

/// What a run answered, as JSON.
fn report(text: &str) -> Value {
    serde_json::from_str(text).expect("a report")
}

#[test]
fn an_agent_writes_runs_and_saves_a_script() {
    let app = App::new().unwrap();
    let server = app.mcp(agent());
    let offered = tools(&server);
    for tool in ["script_describe", "script_run", "script_save", "tasks_list"] {
        assert!(offered.iter().any(|t| t == tool), "{tool}: {offered:?}");
    }

    // What a script can call, and every action it may run with its schema.
    let (failed, described) = call(&server, "script_describe", &json!({}));
    assert!(!failed);
    assert!(described.contains("state.query(id, args?)"), "{described}");
    assert!(described.contains("TASKS_COMPLETE"), "{described}");

    // `args` is JSON text: what the script reads as `args`.
    let (failed, ran) = call(
        &server,
        "script_run",
        &json!({
            "source": script("close_tag.luau"),
            "args": r#"{"tag":"home"}"#,
            "name": "Close home",
        }),
    );
    assert!(!failed, "{ran}");
    let ran = report(&ran);
    assert_eq!(ran["result"], 2);
    assert_eq!(ran["undo"], "Script: Close home");
    assert_eq!(app.board.steps(), ["Script: Close home"]);

    // A failure names its line, and what it changed is taken back.
    let (failed, broken) = call(
        &server,
        "script_run",
        &json!({"source": script("broken.luau")}),
    );
    assert!(failed);
    assert!(broken.contains("line 9: "), "{broken}");
    assert_eq!(app.board.steps(), ["Script: Close home"]);

    // Saved, it is an action — and so a tool — of its own.
    let (failed, saved) = call(
        &server,
        "script_save",
        &json!({
            "name": "Tidy the board",
            "source": script("tidy.luau"),
            "description": "Clears what is done; adds work when there is none.",
        }),
    );
    assert!(!failed, "{saved}");
    assert_eq!(report(&saved)["id"], "SCRIPT_MY_TIDY_THE_BOARD");
    assert!(
        tools(&server)
            .iter()
            .any(|t| t == "script_my_tidy_the_board")
    );
    let (failed, tidied) = call(&server, "script_my_tidy_the_board", &json!({}));
    assert!(!failed, "{tidied}");
    assert_eq!(report(&tidied)["result"]["removed"], 4);
    assert_eq!(
        app.board.steps(),
        ["Script: Close home", "Script: Tidy the board"]
    );
}

#[test]
fn a_viewer_is_refused_part_way() {
    let app = App::new().unwrap();
    let server = app.mcp(viewer());
    let offered = tools(&server);
    assert!(offered.iter().any(|t| t == "script_run"), "{offered:?}");
    assert!(
        !offered.iter().any(|t| t == "tasks_complete"),
        "a viewer is not offered what it may not run: {offered:?}"
    );

    let (failed, refused) = call(
        &server,
        "script_run",
        &json!({"source": script("close_tag.luau")}),
    );
    assert!(failed);
    assert!(refused.contains("line 11: "), "{refused}");
    assert!(refused.contains("not allowed"), "{refused}");
    assert!(
        refused.contains("ran: TASKS_LIST, TASKS_COMPLETE"),
        "it read the board first: {refused}"
    );
    assert_eq!(app.board.tasks().iter().filter(|t| t.done).count(), 2);
    assert!(app.board.steps().is_empty());

    // Reading is a viewer's to do.
    let (failed, read) = call(
        &server,
        "script_run",
        &json!({"source": script("inventory.luau")}),
    );
    assert!(!failed, "{read}");
    assert_eq!(report(&read)["result"]["open"], 3);
}

/// The board says the same over MCP and over its control socket: every
/// action the agent may run, listed, described and (the read-only ones)
/// run the same on both.
#[test]
fn mcp_and_the_cli_agree() {
    let app = App::new().expect("the board");
    let report = architect_mcp::parity::Parity::new(&app.mcp(agent()))
        .run("TASKS_LIST", None)
        .check()
        .unwrap_or_else(|mismatch| panic!("{mismatch}"));
    assert!(report.actions >= 5, "{report:?}");
}
