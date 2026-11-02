//! Confirmation tickets over MCP (#237): a destructive action a model runs
//! waits for a person's yes in the app, then runs once with its ticket.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use architect::action::{ActionList, ActionMeta, Effect, Principal};
use architect_mcp::{ErrorStyle, Server, Tools};
use facet::Facet;
use serde_json::{Value, json};

static DELETE: ActionMeta = ActionMeta {
    id: "FILES_DELETE",
    trait_name: "Files",
    method_name: "delete",
    display_name: "Delete a file",
    description: "Deletes a file for good",
    category: "Files",
    group: "Files",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static LIST: ActionMeta = ActionMeta {
    id: "FILES_LIST",
    method_name: "list",
    display_name: "List files",
    description: "Every file",
    ..DELETE
};

#[derive(Facet)]
struct Path {
    path: String,
}

/// A server whose model is `service:agent`, on a list that asks before a
/// destructive action, with a counter of the deletes that ran.
fn server(ttl: Duration) -> (Server, Arc<AtomicUsize>) {
    let list = ActionList::new();
    let deleted = Arc::new(AtomicUsize::new(0));
    let ran = deleted.clone();
    list.register(&DELETE, Effect::Destructive, move |p: Path| {
        ran.fetch_add(1, Ordering::SeqCst);
        Ok(format!("deleted {}", p.path))
    })
    .expect("registered");
    list.register(&LIST, Effect::ReadOnly, |(): ()| {
        Ok(vec!["a.txt".to_owned()])
    })
    .expect("registered");
    list.require_confirmation(ttl);
    let server = Server::new("Test", "0", list).with_principal(Principal::Service {
        name: "agent".to_owned(),
    });
    (server, deleted)
}

fn call(server: &Server, name: &str, arguments: &Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}});
    serde_json::from_str(&server.handle(&request.to_string()).expect("a reply")).expect("json")
}

/// The tool result's text as JSON, and whether it is an error.
fn answer(reply: &Value) -> (Value, bool) {
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    (
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())),
        reply["result"]["isError"] == json!(true),
    )
}

fn delete(server: &Server, path: &str, ticket: Option<&str>) -> (Value, bool) {
    let mut args = json!({"id": "FILES_DELETE", "args": {"path": path}});
    if let Some(ticket) = ticket {
        args["ticket"] = json!(ticket);
    }
    answer(&call(server, "run", &args))
}

fn ticket(answer: &(Value, bool)) -> String {
    assert!(answer.1, "an error result: {answer:?}");
    assert_eq!(answer.0["confirmation_required"], true, "{answer:?}");
    answer.0["ticket"].as_str().expect("a ticket").to_owned()
}

fn refused(answer: &(Value, bool), why: &str) {
    assert!(answer.1, "an error result: {answer:?}");
    let error = answer.0["error"].as_str().unwrap_or_default();
    assert!(error.contains(why), "{error:?} should say {why:?}");
}

#[test]
fn a_destructive_run_waits_for_a_person_then_runs_once() {
    let (s, deleted) = server(Duration::from_secs(60));
    let first = delete(&s, "a.txt", None);
    let t = ticket(&first);
    assert!(first.0["expires_in_ms"].as_u64().is_some_and(|ms| ms > 0));

    // What the app's sheet shows: the action, its arguments, the caller.
    let pending = s.list().pending_confirmations();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].display_name, "Delete a file");
    assert_eq!(pending[0].args, r#"{"path":"a.txt"}"#);
    assert_eq!(pending[0].caller, "service:agent");

    // Not approved yet: the same ticket, still waiting, and nothing ran.
    assert_eq!(ticket(&delete(&s, "a.txt", Some(&t))), t);
    assert_eq!(deleted.load(Ordering::SeqCst), 0);

    assert!(s.list().approve(&t));
    let ran = delete(&s, "a.txt", Some(&t));
    assert_eq!(ran, (json!("deleted a.txt"), false));
    assert_eq!(deleted.load(Ordering::SeqCst), 1);

    // Single use.
    refused(&delete(&s, "a.txt", Some(&t)), "already used");
    assert_eq!(deleted.load(Ordering::SeqCst), 1);
}

#[test]
fn a_mismatched_denied_or_expired_ticket_is_refused() {
    let (s, deleted) = server(Duration::from_millis(150));
    let t = ticket(&delete(&s, "a.txt", None));
    assert!(s.list().approve(&t));
    refused(&delete(&s, "b.txt", Some(&t)), "another call");
    refused(
        &answer(&call(
            &s,
            "run",
            &json!({"id": "FILES_DELETE", "args": {"path": "a.txt"}, "ticket": "made-up"}),
        )),
        "no ticket",
    );

    let no = ticket(&delete(&s, "a.txt", None));
    assert!(s.list().deny(&no));
    refused(&delete(&s, "a.txt", Some(&no)), "declined");

    std::thread::sleep(Duration::from_millis(200));
    refused(&delete(&s, "a.txt", Some(&t)), "expired");
    assert_eq!(deleted.load(Ordering::SeqCst), 0);
}

#[test]
fn a_person_at_the_app_is_not_asked_and_reads_are_never_asked() {
    let (s, deleted) = server(Duration::from_secs(60));
    // A key, a menu, the palette: the app's own door.
    s.list()
        .run_json("FILES_DELETE", br#"{"path":"a.txt"}"#)
        .expect("ran");
    assert_eq!(deleted.load(Ordering::SeqCst), 1);
    let (files, failed) = answer(&call(&s, "run", &json!({"id": "FILES_LIST"})));
    assert_eq!((files, failed), (json!(["a.txt"]), false));
    assert!(s.list().pending_confirmations().is_empty());
}

#[test]
fn named_tools_and_batch_steps_carry_tickets() {
    let (s, deleted) = server(Duration::from_secs(60));
    let s = s.with_tools(Tools::Both);
    let t = ticket(&answer(&call(
        &s,
        "files_delete",
        &json!({"path": "a.txt"}),
    )));
    assert!(s.list().approve(&t));
    let ran = answer(&call(
        &s,
        "files_delete",
        &json!({"path": "a.txt", "_ticket": t}),
    ));
    assert_eq!(ran, (json!("deleted a.txt"), false));

    let steps = |ticket: Option<&str>| {
        let mut step = json!({"id": "FILES_DELETE", "args": {"path": "b.txt"}});
        if let Some(ticket) = ticket {
            step["ticket"] = json!(ticket);
        }
        json!({"steps": [{"id": "FILES_LIST"}, step]})
    };
    let (asked, failed) = answer(&call(&s, "batch", &steps(None)));
    assert!(failed);
    let t = asked["results"][1]["error"]["ticket"]
        .as_str()
        .expect("a ticket")
        .to_owned();
    assert!(s.list().approve(&t));
    let (done, failed) = answer(&call(&s, "batch", &steps(Some(&t))));
    assert!(!failed, "{done}");
    assert_eq!(done["results"][1]["result"], "deleted b.txt");
    assert_eq!(deleted.load(Ordering::SeqCst), 2);
}

#[test]
fn text_errors_name_the_ticket_and_watch_hears_the_answer() {
    let (s, _) = server(Duration::from_secs(60));
    let s = s.with_errors(ErrorStyle::Text);
    let since = s.list().latest();
    let (said, failed) = delete(&s, "a.txt", None);
    assert!(failed);
    let said = said.as_str().expect("text").to_owned();
    assert!(said.starts_with("confirmation_required"), "{said}");
    let t = s.list().pending_confirmations()[0].ticket.clone();
    assert!(said.contains(&t), "{said}");
    assert!(s.list().approve(&t));
    let (watched, _) = answer(&call(&s, "watch", &json!({"since": since})));
    let details: Vec<&str> = watched["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["changed"] == "confirmation")
        .filter_map(|e| e["which"].as_str())
        .collect();
    assert_eq!(
        details,
        [format!("{t} pending").as_str(), &format!("{t} approved")]
    );
}
