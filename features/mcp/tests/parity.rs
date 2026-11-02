//! MCP and the command line say the same: one list, every kind of action
//! and of failure, asked over MCP (`Server::call_tool`), over the control
//! socket (`architect-control`), through the `actions` CLI, and through
//! MCP bridged onto the socket — the same set, the same JSON, and exit
//! codes from the same failures.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect, Principal};
use architect_control::Surface;
use architect_control::cli::{ActionsArgs, Cli, ListOnly};
use architect_control::server::{Control, Serving};
use architect_mcp::Server;
use architect_mcp::bridge::Bridge;
use architect_mcp::parity::Parity;
use architect_permissions::{Action, Decision, PermissionEngine, Resource};
use clap::Parser as _;
use facet::Facet;
use serde_json::{Value, json};

const fn meta(id: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name: id,
        description,
        category: "Test",
        group: "",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static HITS: ActionMeta = meta("DRUMS_HITS", "Every hit of a drum");
static ASK: ActionMeta = meta("PEER_ASK", "Ask a peer by name");
static ECHO: ActionMeta = meta("WORD_ECHO", "Echo a word");
static DELETE: ActionMeta = meta("PROJECT_DELETE", "Delete the open project");
static WHERE: ActionMeta = meta("ORG_WHERE", "The org this call acts in");
static RENDER: ActionMeta = meta("PROJECT_RENDER", "Render the project");
static SECRET: ActionMeta = meta("ADMIN_SECRET", "Only admins");

#[derive(Facet)]
struct Query {
    track: String,
}

#[derive(Facet)]
struct Ask {
    name: String,
}

/// A user may run anything but `ADMIN_SECRET`.
struct NoSecrets;
impl PermissionEngine for NoSecrets {
    fn check(&self, _who: &Principal, what: &Resource, _action: &Action) -> Decision {
        if what.as_str() == "action/ADMIN_SECRET" {
            Decision::deny("admins only")
        } else {
            Decision::Allow
        }
    }
}

/// A list with a sync, an async, a non-object, a destructive, a
/// caller-reading, a disabled and a forbidden action.
fn list() -> ActionList {
    let list = ActionList::new();
    list.register(&HITS, Effect::ReadOnly, |q: Query| {
        if q.track == "none" {
            return Err("no such track".to_owned());
        }
        Ok(vec![0.5_f64, 1.0])
    })
    .unwrap();
    list.register_async(&ASK, Effect::Mutating, |a: Ask| async move {
        Ok(format!("hello {}", a.name))
    })
    .unwrap();
    list.register(&ECHO, Effect::Mutating, |word: String| Ok(word))
        .unwrap();
    list.register(&DELETE, Effect::Destructive, |(): ()| Ok(()))
        .unwrap();
    list.register_with(&WHERE, Effect::ReadOnly, |cx: &ActionCaller, (): ()| {
        Ok(cx
            .principal()
            .map_or_else(|| "app".to_owned(), Principal::describe))
    })
    .unwrap();
    list.register(&RENDER, Effect::Mutating, |(): ()| Ok(()))
        .unwrap();
    list.set_enabled("PROJECT_RENDER", || Err("no project is open".to_owned()))
        .unwrap();
    list.register(&SECRET, Effect::ReadOnly, |(): ()| Ok(42))
        .unwrap();
    list.set_authorizer(NoSecrets);
    // A person approves a destructive call from outside first (#237).
    list.require_confirmation(std::time::Duration::from_secs(60));
    list
}

fn surface() -> Surface {
    Surface::new("Parity", "1.0", list()).with_principal(Principal::User {
        user_id: "cli".into(),
    })
}

fn socket(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("architect-mcp-{tag}-{}.sock", std::process::id()))
}

fn serve(tag: &str, surface: Surface) -> Serving {
    Control::at(socket(tag), surface).serve().expect("serves")
}

/// The whole list, every failure kind, and the runs that change things:
/// the same over MCP and the socket.
#[test]
fn mcp_and_the_socket_agree_on_everything() {
    let server = Server::over(surface());
    let report = Parity::new(&server)
        .run("DRUMS_HITS", Some(json!({"track": "kick"})))
        .run("DRUMS_HITS", Some(json!({"track": "none"})))
        .run("DRUMS_HITS", Some(json!({"trak": "kick"})))
        .run("PEER_ASK", Some(json!({"name": "ada"})))
        .run("WORD_ECHO", Some(json!("word")))
        .run("PROJECT_DELETE", None)
        .run("PROJECT_RENDER", None)
        .run("ADMIN_SECRET", None)
        .check()
        .unwrap_or_else(|mismatch| panic!("{mismatch}"));
    // The user sees six of the seven: not the admins' one.
    assert_eq!(report.actions, 6);
    assert!(report.requests > 20, "{report:?}");
}

/// The CLI's output and exit code for `line` against `socket`.
fn cli(socket: &Path, line: &str) -> (u8, String, String) {
    let mut words: Vec<String> = vec![
        "actions".into(),
        "--app".into(),
        socket.display().to_string(),
    ];
    words.extend(line.split(' ').map(str::to_owned));
    let args = Top::try_parse_from(words).expect("parses").args;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = Cli::new("parity").execute(&args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("utf-8"),
        String::from_utf8(err).expect("utf-8"),
    )
}

#[derive(clap::Parser)]
struct Top {
    #[command(flatten)]
    args: ActionsArgs,
}

/// The issue's command line, end to end over the socket: list, describe,
/// run (text and JSON), the schema, events — and each failure its exit
/// code, with what helps on stderr.
#[test]
fn the_cli_lists_describes_and_runs_with_exit_codes() {
    let serving = serve("cli", surface());
    let at = serving.path();

    let (code, out, _) = cli(at, "list");
    assert_eq!(code, 0);
    assert!(
        out.contains("DRUMS_HITS") && out.contains("read-only"),
        "{out}"
    );
    assert!(out.contains("(disabled: no project is open)"), "{out}");
    assert!(
        !out.contains("ADMIN_SECRET"),
        "the user cannot see it: {out}"
    );
    let (_, out, _) = cli(at, "list --effect destructive --json");
    let listed: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["actions"][0]["id"], "PROJECT_DELETE");

    let (code, out, _) = cli(at, "describe DRUMS_HITS");
    assert_eq!(code, 0);
    assert!(
        out.contains("effect: read-only") && out.contains("\"track\""),
        "{out}"
    );

    let (code, out, _) = cli(at, "run DRUMS_HITS --arg track=kick --output json");
    assert_eq!((code, out.trim()), (0, "[0.5,1]"));
    let (code, out, _) = cli(at, "run PEER_ASK --json {\"name\":\"ada\"}");
    assert_eq!((code, out.trim()), (0, "hello ada"));
    let (code, out, _) = cli(at, "run ORG_WHERE");
    assert_eq!(
        (code, out.trim()),
        (0, "user:cli"),
        "the CLI runs as the surface's principal"
    );

    let (code, _, err) = cli(at, "run DRUMS_HITS --arg track=none");
    assert_eq!(code, 1);
    assert!(err.contains("no such track"), "{err}");
    let (code, _, err) = cli(at, "run NOPE_NOPE");
    assert_eq!(code, 3, "{err}");
    let (code, _, err) = cli(at, "run DRUMS_HITS --arg trak=kick");
    assert_eq!(code, 4);
    assert!(err.contains("it takes:") && err.contains("track"), "{err}");
    let (code, _, err) = cli(at, "run ADMIN_SECRET");
    assert_eq!(code, 5, "{err}");
    assert!(err.contains("admins only"), "{err}");
    let (code, _, err) = cli(at, "describe ADMIN_SECRET");
    assert_eq!(code, 3, "a hidden action is not described: {err}");
    let (code, _, err) = cli(at, "run PROJECT_RENDER");
    assert_eq!(code, 6);
    assert!(err.contains("no project is open"), "{err}");
    let (code, _, _) = cli(at, "run DRUMS_HITS --arg nokey");
    assert_eq!(code, 2);

    let (code, out, _) = cli(
        at,
        r#"batch [{"id":"DRUMS_HITS","args":{"track":"a"}},{"id":"DRUMS_HITS","args":{"track":"none"}},{"id":"PEER_ASK"}]"#,
    );
    assert_eq!(code, 1);
    assert!(out.contains("ran 2 of 3"), "{out}");

    let (code, out, _) = cli(at, "schema");
    assert_eq!(code, 0);
    let schema: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(6));
    let (_, out, _) = cli(at, "schema --api");
    let api: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(api["oneOf"].as_array().map(Vec::len), Some(7));

    let (code, out, _) = cli(at, "events --json");
    assert_eq!(code, 0);
    let ran: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).expect("a json line"))
        .filter(|e: &Value| e.get("ran").is_some())
        .collect();
    assert!(ran.len() >= 5, "{out}");
    let (_, out, _) = cli(at, "events --since 999999");
    assert!(
        out.starts_with("lost:"),
        "a watcher from another run is told: {out}"
    );
}

/// A destructive run waits for a person: the CLI prints the ticket and
/// exits 7; approved in the app, `--ticket` runs it once.
#[test]
fn the_cli_prints_the_ticket_and_runs_with_it_once_approved() {
    let surface = surface();
    let list = surface.list().clone();
    let serving = serve("confirm", surface);
    let at = serving.path();
    let (code, _, err) = cli(at, "run PROJECT_DELETE");
    assert_eq!(code, 7, "{err}");
    let pending = list.pending_confirmations();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].caller, "user:cli",
        "the CLI's principal is who is named"
    );
    let ticket = pending[0].ticket.clone();
    assert!(err.contains(&format!("--ticket {ticket}")), "{err}");
    let (code, _, err) = cli(at, &format!("run PROJECT_DELETE --ticket {ticket}"));
    assert_eq!(code, 7, "not approved yet: {err}");
    assert!(list.approve(&ticket));
    let (code, _, err) = cli(at, &format!("run PROJECT_DELETE --ticket {ticket}"));
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = cli(at, &format!("run PROJECT_DELETE --ticket {ticket}"));
    assert_ne!(code, 0, "a ticket runs once: {err}");
}

/// A command for an app that is not running says so, with its own code.
#[test]
fn no_app_is_unavailable() {
    let (code, _, err) = cli(&socket("nobody"), "list");
    assert_eq!(code, 69);
    assert!(err.contains("no app answers"), "{err}");
}

/// `--headless` asks the list started in the process: the same answers.
#[test]
fn headless_answers_as_the_socket_does() {
    let headless = Cli::new("parity").headless(|| Ok(Box::new(ListOnly::new(surface()))));
    let run = |cli: &Cli, line: &str| {
        let mut words = vec!["actions".to_owned()];
        words.extend(line.split(' ').map(str::to_owned));
        let args = Top::try_parse_from(words).expect("parses").args;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = cli.execute(&args, &mut out, &mut err);
        (code, String::from_utf8(out).expect("utf-8"))
    };
    let serving = serve("headless", surface());
    for line in [
        "list --json",
        "describe PEER_ASK --json",
        "run PEER_ASK --arg name=bo --output json",
        "run DRUMS_HITS --output json",
        "schema",
    ] {
        let here = run(&headless, &format!("{line} --headless"));
        let there = run(
            &Cli::new("parity"),
            &format!("{line} --app {}", serving.path().display()),
        );
        assert_eq!(here, there, "{line}");
    }
    let (code, _) = run(&Cli::new("nothing"), "list --headless");
    assert_eq!(code, 2, "an app with no headless mode says so");
}

/// MCP bridged onto the socket answers every tool as the server does.
#[test]
fn the_bridge_answers_as_the_server_does() {
    let surface = surface();
    let server = Server::over(surface.clone());
    let serving = serve("bridge", surface);
    let bridge = Bridge::connect(serving.path()).expect("connects");
    let ask = |message: Value| {
        let text = message.to_string();
        let direct: Value = serde_json::from_str(&server.handle(&text).expect("a reply")).unwrap();
        let bridged: Value = serde_json::from_str(&bridge.handle(&text).expect("a reply")).unwrap();
        (direct, bridged)
    };
    let call = |name: &str, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}})
    };
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": {"uri": "actions://list"}}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": {"uri": "actions://schema"}}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        call("actions", json!({"filter": "project"})),
        call("describe", json!({"id": "WORD_ECHO"})),
        call("run", json!({"id": "WORD_ECHO", "args": "hi"})),
        call("run", json!({"id": "DRUMS_HITS", "args": {"trak": 1}})),
        call("run", json!({"id": "ADMIN_SECRET"})),
        call("run", json!({"id": "X_Y", "stray": true})),
        call(
            "batch",
            json!({"steps": [{"id": "PEER_ASK", "args": {"name": "a"}}, {"id": "NOPE_X"}]}),
        ),
        call("nope", json!({})),
    ] {
        let (direct, bridged) = ask(message.clone());
        assert_eq!(direct, bridged, "{message}");
    }
}
