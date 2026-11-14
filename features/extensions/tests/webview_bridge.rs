//! The web view kind's bridge (#222, `architect_extensions::webview`): a
//! web view's calls are its extension's, checked against what it may
//! reach. It runs its own action and is refused one outside its
//! capabilities; it keeps a secret it can never read back; it hears the
//! document events it listens for only while it may read documents; and
//! the kind loads only where an app registered it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use architect_extensions::events::DocEvent;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::webview::{Bridge, Outbound};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use architect_workspace::{KindSpec, ViewKinds};
use futures::executor::block_on;
use serde_json::{Value, json};

fn manifest(reads: &str) -> String {
    format!(
        r#"
id = "demo.board"
name = "Board"
version = "0.1.0"
kind = "data"

[capabilities]
reads = [{reads}]
hosts = ["api.example.com"]

[[actions]]
id = "DEMO_BOARD_PING"
name = "Ping"
preset = {{ action = "DEMO_BOARD_PING" }}

[[secrets]]
id = "token"
title = "API token"
hosts = ["api.example.com"]

[[views]]
id = "board"
title = "Board"
kind = "web"
src = "dist/index.html"
"#
    )
}

static THEME_SET: ActionMeta = ActionMeta {
    id: "THEME_SET",
    trait_name: "",
    method_name: "THEME_SET",
    display_name: "Set the theme",
    description: "Sets the theme.",
    category: "Theme",
    group: "Theme",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The app: a `THEME_SET` the web view's extension may not run.
fn host() -> Host {
    let list = ActionList::new();
    list.register_with(&THEME_SET, Effect::Mutating, |_: &ActionCaller, (): ()| {
        Ok::<_, String>(())
    })
    .unwrap();
    let mut kinds = ViewKinds::new();
    kinds.register(KindSpec::web()).unwrap();
    Host::new(list, SharedSections::new(Registry::default())).with_view_kinds(kinds)
}

fn load(host: &Host, reads: &str) {
    let text = manifest(reads);
    let package = Package::embedded(&text, &[("dist/index.html", "<p>board</p>")]).unwrap();
    host.load(&package, Origin::Installed).unwrap();
}

/// A bridge whose posts are kept.
fn bridge(host: &Host) -> (Bridge, Arc<Mutex<Vec<Outbound>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let b = Bridge::new(
        host,
        "demo.board",
        Arc::new(move |m| sink.lock().unwrap().push(m)),
        || {},
    );
    (b, seen)
}

fn call(b: &Bridge, seen: &Mutex<Vec<Outbound>>, id: u64, req: &Value) -> Value {
    let text = json!({"t": "call", "id": id, "req": req}).to_string();
    block_on(b.receive(&text).expect("a call is work"));
    let all = seen.lock().unwrap();
    all.iter()
        .rev()
        .find_map(|m| match m {
            Outbound::Answer { id: got, answer } if *got == id => Some(answer.clone()),
            _ => None,
        })
        .expect("answered")
}

#[test]
fn a_web_view_runs_as_its_extension_and_is_refused_past_its_capabilities() {
    let host = host();
    load(&host, "");
    let (b, seen) = bridge(&host);
    let refused = call(&b, &seen, 1, &json!({"op": "run", "action": "THEME_SET"}));
    assert_eq!(refused["error"]["kind"], "refused", "{refused}");
    let denied = call(&b, &seen, 2, &json!({"op": "list_documents"}));
    assert_eq!(denied["error"]["kind"], "refused", "{denied}");
    // Its own setting store needs nothing.
    let set = call(&b, &seen, 3, &json!({"op": "set_data", "key": "cards", "value": [1, 2]}));
    assert!(set.get("ok").is_some(), "{set}");
    let got = call(&b, &seen, 4, &json!({"op": "data", "key": "cards"}));
    assert_eq!(got["ok"], json!([1, 2]));
    // A malformed call is answered, not dropped.
    assert!(b.receive(r#"{"t":"call","id":5}"#).is_none());
    assert!(seen.lock().unwrap().iter().any(|m| matches!(m, Outbound::Answer { id: 5, .. })));
}

#[test]
fn a_secret_is_kept_and_never_read_back() {
    let host = host();
    load(&host, "");
    let (b, seen) = bridge(&host);
    let kept = call(&b, &seen, 1, &json!({"op": "store_secret", "name": "token", "value": "s3cret"}));
    assert_eq!(kept["ok"], true, "{kept}");
    let state = call(&b, &seen, 2, &json!({"op": "secret", "name": "token"}));
    assert_eq!(state["ok"]["set"], true);
    assert!(!state.to_string().contains("s3cret"), "{state}");
    let other = call(&b, &seen, 3, &json!({"op": "store_secret", "name": "other", "value": "x"}));
    assert_eq!(other["error"]["kind"], "refused", "{other}");
    // Its value is the host's: in a header to its own host only.
    assert!(host.secrets().is_set("demo.board", "token"));
}

#[test]
fn a_web_view_hears_document_events_only_while_it_may_read_documents() {
    let blind = host();
    load(&blind, "");
    let (b, seen) = bridge(&blind);
    assert!(b.receive(r#"{"t":"listen","events":["doc.*"]}"#).is_none());
    blind.raise_doc(&DocEvent::Saved { path: "a.md".into() });
    assert!(!seen.lock().unwrap().iter().any(|m| matches!(m, Outbound::Event { .. })));

    let reader = host();
    load(&reader, r#""documents""#);
    let (b, seen) = bridge(&reader);
    reader.raise_doc(&DocEvent::Saved { path: "before.md".into() });
    assert!(b.receive(r#"{"t":"listen","events":["doc.saved"]}"#).is_none());
    reader.raise_doc(&DocEvent::Saved { path: "a.md".into() });
    reader.raise_doc(&DocEvent::Created { path: "b.md".into() });
    let events: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|m| match m {
            Outbound::Event { name, detail } => Some((name.clone(), detail.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(events, vec![("doc.saved".to_owned(), json!({"path": "a.md"}))]);
    drop(b);
    // Dropped, it hears nothing more.
    reader.raise_doc(&DocEvent::Saved { path: "c.md".into() });
    assert_eq!(seen.lock().unwrap().iter().filter(|m| matches!(m, Outbound::Event { .. })).count(), 1);
}

#[test]
fn a_web_view_needs_the_kind_and_its_src() {
    // An app with no web kind refuses it, saying why.
    let bare = Host::new(ActionList::new(), SharedSections::new(Registry::default()));
    let text = manifest("");
    let package = Package::embedded(&text, &[("dist/index.html", "x")]).unwrap();
    let err = bare.load(&package, Origin::Installed).unwrap_err();
    assert!(err.to_string().contains("“web”"), "{err}");
    // A web view without its src is not a manifest.
    let no_src = text.replace("src = \"dist/index.html\"\n", "");
    let err = Package::embedded(&no_src, &[]).map(|_| ()).unwrap_err();
    assert!(err.to_string().contains("src"), "{err}");
}
