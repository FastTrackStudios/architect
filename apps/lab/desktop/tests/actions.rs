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

#[derive(clap::Parser)]
struct Top {
    #[command(flatten)]
    args: ActionsArgs,
}

fn actions(line: &str) -> (u8, String, String) {
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

/// Every action the headless lab has says the same over MCP and over its
/// socket — the lab pumped here, so an action that hands its work to the
/// window's thread is answered.
#[test]
fn the_lab_says_the_same_over_mcp_and_the_cli() {
    let mut lab = HeadlessLab::open();
    lab.settle();
    let server = architect_mcp::Server::over(lab.surface());
    let report = std::thread::scope(|scope| {
        let check = scope.spawn(|| Parity::new(&server).check());
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
