//! The lab's Action List from the command line and over MCP, headless:
//! the same set and the same answers on both (`architect_mcp::parity`),
//! and the CLI listing, describing and running its actions.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_control::cli::{ActionsArgs, Headless as _};
use architect_desktop::HeadlessLab;
use architect_mcp::parity::Parity;
use clap::Parser as _;
use serde_json::Value;

/// Points the lab's folder at a fresh temporary one for this test binary,
/// so the headless lab never writes into the person's own
/// `~/.config/architect-lab` (its built-in pages, the agents' guide).
fn isolated() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("a temporary lab folder"));
    // SAFETY: set before the lab (and any of its threads) start; every
    // test in this binary sets the same value.
    unsafe { std::env::set_var("ARCHITECT_LAB_CONFIG_DIR", dir.path()) };
}


#[derive(clap::Parser)]
struct Top {
    #[command(flatten)]
    args: ActionsArgs,
}

fn actions(line: &str) -> (u8, String, String) {
    isolated();
    let words = std::iter::once("actions").chain(line.split(' '));
    let args = Top::try_parse_from(words).expect("parses").args;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = architect_desktop::cli().execute(&args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("utf-8"),
        String::from_utf8(err).expect("utf-8"),
    )
}

/// A surface of more than ten catalog components (and one outside it).
fn a2ui_surface() -> Value {
    serde_json::json!({ "messages": [
        { "createSurface": { "surfaceId": "lab-form", "catalogId": architect_a2ui::CATALOG_ID,
          "dataModel": { "name": "Ada", "subscribe": true, "volume": 30, "plan": ["free"], "tab": 0,
                         "items": [ { "title": "One" }, { "title": "Two" } ] },
          "components": [
            { "id": "root", "component": "Column", "children": ["title", "card", "tabs", "list", "row", "x"] },
            { "id": "title", "component": "Text", "variant": "h2", "text": "Sign up" },
            { "id": "card", "component": "Card", "child": "fields" },
            { "id": "fields", "component": "Column", "children": ["name", "subscribe", "volume", "plan", "rule"] },
            { "id": "name", "component": "TextField", "label": "Name", "value": { "path": "/name" } },
            { "id": "subscribe", "component": "CheckBox", "label": "Subscribe", "value": { "path": "/subscribe" } },
            { "id": "volume", "component": "Slider", "label": "Volume", "value": { "path": "/volume" } },
            { "id": "plan", "component": "ChoicePicker", "label": "Plan", "value": { "path": "/plan" },
              "options": [ { "label": "Free", "value": "free" }, { "label": "Pro", "value": "pro" } ] },
            { "id": "rule", "component": "Divider" },
            { "id": "tabs", "component": "Tabs", "activeTab": { "path": "/tab" },
              "tabs": [ { "title": "One", "child": "t1" }, { "title": "Two", "child": "t2" } ] },
            { "id": "t1", "component": "Text", "text": "First tab" },
            { "id": "t2", "component": "Text", "text": "Second tab" },
            { "id": "list", "component": "List", "children": { "componentId": "item", "path": "/items" } },
            { "id": "item", "component": "Text", "text": { "path": "title" } },
            { "id": "row", "component": "Row", "children": ["go"] },
            { "id": "go", "component": "Button", "child": "go_label",
              "action": { "event": { "name": "LAB_STATE" } } },
            { "id": "go_label", "component": "Text", "text": "Where am I?" },
            { "id": "x", "component": "Script", "src": "x.js" }
          ] } }
    ] })
}

fn a2ui_patch() -> Value {
    serde_json::json!({ "messages": [
        { "updateDataModel": { "surfaceId": "lab-form", "path": "/name", "value": "Grace" } }
    ] })
}

/// Every action the headless lab has says the same over MCP and over its
/// socket — the lab pumped here, so an action that hands its work to the
/// window's thread is answered.
#[test]
fn the_lab_says_the_same_over_mcp_and_the_cli() {
    isolated();
    let mut lab = HeadlessLab::open();
    lab.settle();
    let server = architect_mcp::Server::over(lab.surface());
    let report = std::thread::scope(|scope| {
        let check = scope.spawn(|| {
            Parity::new(&server)
                // An agent's UI, rendered, patched and read back the same
                // on both faces.
                .run("A2UI_RENDER", Some(a2ui_surface()))
                .run("A2UI_PATCH", Some(a2ui_patch()))
                .run(
                    "A2UI_SURFACE",
                    Some(serde_json::json!({ "surfaceId": "lab-form" })),
                )
                .check()
        });
        while !check.is_finished() {
            if !lab.pump() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        check.join().expect("the check ran")
    })
    .unwrap_or_else(|mismatch| panic!("{mismatch}"));
    assert!(report.actions >= 3, "{report:?}");
}

/// `architect-desktop actions … --headless`: the lab's own actions listed,
/// described and run, with no window.
#[test]
fn the_cli_runs_the_lab_headless() {
    let (code, out, err) = actions("list --headless --json");
    assert_eq!(code, 0, "{err}");
    let listed: Value = serde_json::from_str(&out).expect("json");
    let ids: Vec<&str> = listed["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(ids.contains(&"LAB_STATE"), "{ids:?}");

    let (code, out, err) = actions("describe LAB_STATE --headless");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("effect: read-only"), "{out}");

    let (code, out, err) = actions("run LAB_STATE --headless --output json");
    assert_eq!(code, 0, "{err}");
    let state: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(state["tab"], "inbox", "{state}");

    let (code, _, _) = actions("run LAB_NOTHING --headless");
    assert_eq!(code, 3);
}
