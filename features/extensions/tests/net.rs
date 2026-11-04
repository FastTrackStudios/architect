//! The network, for extensions (#310), against a local HTTP server (no
//! real network): a fetch reaches a host in `capabilities.hosts` and is
//! refused another, directly and through a redirect; a Luau script does
//! the same through `host.fetch`; a streamed body is read a part at a
//! time, by its own extension only; time and size limits hold; a secret
//! the person enters once is put into requests to its hosts by the host,
//! taken off on a redirect elsewhere, and never readable by the
//! extension. And a hook on a document's events needs `reads =
//! ["documents"]`: declared (refused at load) or made at run time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::{ActionCaller, ActionList};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{Fetch, HostError, HostRequest};
use architect_extensions::hostui::FakeUi;
use architect_extensions::net::Secrets;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

#[path = "support/server.rs"]
mod server;
use server::{serve, streamed};

// ── The app ─────────────────────────────────────────────────────────

const NET: &str = "lab.net";

fn manifest(hosts: &str, extra: &str) -> String {
    format!(
        r#"
id = "lab.net"
name = "Net"
version = "0.1.0"
kind = "luau"

[capabilities]
hosts = [{hosts}]

[[actions]]
id = "LAB_NET_FETCH"
name = "Fetch"
params = '{{"type":"object"}}'
script = "fetch.luau"
{extra}
"#
    )
}

const FETCH_LUAU: &str = r#"
local ok, out = pcall(host.fetch, args.url, args.options)
if ok then return out end
return { error = tostring(out) }
"#;

fn app(tmp: &Path, manifest: &str) -> (ActionList, Host) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.join("data"));
    load(&host, &tmp.join("dev/net"), manifest).unwrap();
    (list, host)
}

fn load(host: &Host, dir: &Path, manifest: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    std::fs::write(dir.join("fetch.luau"), FETCH_LUAU).unwrap();
    let package = Package::read(dir).map_err(|e| e.to_string())?;
    host.load(&package, Origin::Dev)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn fetch(url: impl Into<String>) -> Fetch {
    Fetch {
        url: url.into(),
        ..Fetch::default()
    }
}

fn call(host: &Host, request: HostRequest) -> Result<Value, HostError> {
    block_on(host.api(NET).call(request))
}

fn refused(r: Result<Value, HostError>) -> (String, String) {
    match r {
        Err(HostError::Refused { capability, why }) => (capability, why),
        other => panic!("not refused: {other:?}"),
    }
}

fn run(list: &ActionList, args: &Value) -> Value {
    let out = block_on(list.run_json_async_with(
        &ActionCaller::app(),
        "LAB_NET_FETCH",
        args.to_string().as_bytes(),
    ))
    .unwrap();
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn a_fetch_reaches_a_declared_host_and_is_refused_another_and_a_redirect_to_one() {
    let server = serve();
    let tmp = tempfile::tempdir().unwrap();
    let (_list, host) = app(tmp.path(), &manifest(r#""127.0.0.1""#, ""));
    let base = format!("http://127.0.0.1:{}", server.port);

    let out = call(&host, HostRequest::Fetch(fetch(format!("{base}/hello")))).unwrap();
    assert_eq!(out["status"], 200, "{out}");
    assert_eq!(out["body"], "hello from the server", "{out}");
    assert_eq!(out["headers"]["content-type"], "text/plain", "{out}");

    // A redirect to a host it may reach is followed.
    let out = call(&host, HostRequest::Fetch(fetch(format!("{base}/redirect-in")))).unwrap();
    assert_eq!(out["body"], "hello from the server", "{out}");
    assert!(out["url"].as_str().unwrap().ends_with("/hello"), "{out}");

    // Another host: refused before anything is sent.
    let (capability, why) = refused(call(
        &host,
        HostRequest::Fetch(fetch(format!("http://localhost:{}/hello", server.port))),
    ));
    assert_eq!(capability, "hosts");
    assert!(why.contains("localhost"), "{why}");

    // A redirect to it: refused, not followed.
    let before = server.heard.to("/hello").len();
    let (capability, why) = refused(call(
        &host,
        HostRequest::Fetch(fetch(format!("{base}/redirect-out"))),
    ));
    assert_eq!(capability, "hosts");
    assert!(why.contains("redirected") && why.contains("localhost"), "{why}");
    assert_eq!(server.heard.to("/hello").len(), before, "the redirect was not followed");

    // Not http: refused.
    let (capability, _) = refused(call(&host, HostRequest::Fetch(fetch("file:///etc/passwd"))));
    assert_eq!(capability, "hosts");

    // The refusals are in its log.
    let lines: Vec<String> = host.logs().since(NET, None, 1000).iter().map(|l| l.text.clone()).collect();
    assert!(lines.iter().any(|l| l.contains("fetch: refused (hosts)")), "{lines:?}");
}

#[test]
fn a_luau_script_fetches_from_its_host_and_is_refused_another() {
    let server = serve();
    let tmp = tempfile::tempdir().unwrap();
    let (list, _host) = app(tmp.path(), &manifest(r#""127.0.0.1""#, ""));
    let base = format!("http://127.0.0.1:{}", server.port);

    let out = run(&list, &json!({ "url": format!("{base}/hello") }));
    assert_eq!(out["status"], 200, "{out}");
    assert_eq!(out["body"], "hello from the server", "{out}");

    let out = run(
        &list,
        &json!({ "url": format!("http://localhost:{}/hello", server.port) }),
    );
    let error = out["error"].as_str().unwrap_or_default();
    assert!(error.contains("refused (hosts)"), "{out}");

    let out = run(&list, &json!({ "url": format!("{base}/redirect-out") }));
    let error = out["error"].as_str().unwrap_or_default();
    assert!(error.contains("refused (hosts)") && error.contains("redirected"), "{out}");
}

#[test]
fn a_streamed_body_is_read_in_parts_by_its_own_extension_only() {
    let server = serve();
    let tmp = tempfile::tempdir().unwrap();
    let (_list, host) = app(tmp.path(), &manifest(r#""127.0.0.1""#, ""));
    let other = tmp.path().join("dev/other");
    load(
        &host,
        &other,
        &manifest(r#""127.0.0.1""#, "").replace("lab.net", "lab.other").replace("LAB_NET_", "LAB_OTHER_"),
    )
    .unwrap();
    let out = call(
        &host,
        HostRequest::Fetch(Fetch {
            stream: true,
            ..fetch(format!("http://127.0.0.1:{}/stream", server.port))
        }),
    )
    .unwrap();
    assert_eq!(out["status"], 200, "{out}");
    assert!(out.get("body").is_none(), "{out}");
    let stream = out["stream"].as_str().unwrap().to_owned();

    // Another extension cannot read it.
    let theirs = block_on(host.api("lab.other").call(HostRequest::ReadBody {
        stream: stream.clone(),
        max: None,
    }));
    assert!(matches!(theirs, Err(HostError::NotFound { .. })), "{theirs:?}");

    let mut text = String::new();
    let mut parts = 0;
    loop {
        let part = call(
            &host,
            HostRequest::ReadBody {
                stream: stream.clone(),
                max: Some(1000),
            },
        )
        .unwrap();
        let chunk = part["chunk"].as_str().unwrap();
        assert!(chunk.len() <= 1000, "a part is at most its max");
        text.push_str(chunk);
        parts += 1;
        if part["done"] == true {
            break;
        }
        assert!(parts < 10_000, "it never ended");
    }
    assert_eq!(text, streamed(), "every character whole, in order");
    assert!(parts > 10, "{parts} parts");
    // Done: closed.
    assert_eq!(
        call(&host, HostRequest::CloseBody { stream }).unwrap(),
        json!(false)
    );
}

#[test]
fn time_and_size_limits_hold() {
    let server = serve();
    let tmp = tempfile::tempdir().unwrap();
    let (_list, host) = app(tmp.path(), &manifest(r#""127.0.0.1""#, ""));
    let base = format!("http://127.0.0.1:{}", server.port);
    let big = call(
        &host,
        HostRequest::Fetch(Fetch {
            max_bytes: Some(1 << 20),
            ..fetch(format!("{base}/big"))
        }),
    );
    assert!(
        matches!(&big, Err(HostError::Failed { message }) if message.contains("past the most")),
        "{big:?}"
    );
    let slow = call(
        &host,
        HostRequest::Fetch(Fetch {
            timeout_ms: Some(200),
            ..fetch(format!("{base}/slow"))
        }),
    );
    assert!(
        matches!(&slow, Err(HostError::Failed { message }) if message.contains("timed out")),
        "{slow:?}"
    );
    // A header that is the host's to set.
    let mut bad = fetch(format!("{base}/hello"));
    bad.headers.insert("Host".to_owned(), "evil.example".to_owned());
    assert!(matches!(call(&host, HostRequest::Fetch(bad)), Err(HostError::Failed { .. })));
}

const SECRETS: &str = r#"
[[secrets]]
id = "api_key"
title = "Test API key"
description = "From the test's dashboard."
hosts = ["127.0.0.1"]
"#;

#[test]
fn a_secret_is_asked_for_once_put_into_requests_to_its_host_and_never_readable() {
    let server = serve();
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let file = tmp.path().join("app/extension-secrets.json");
    let ui = FakeUi::new().answer(json!("sk-test-123"));
    let host = Host::new(list, SharedSections::new(Registry::new()))
        .with_data_root(tmp.path().join("data"))
        .with_secrets(Secrets::open(&file));
    host.set_ui(Arc::new(ui.clone()));
    load(
        &host,
        &tmp.path().join("dev/net"),
        &manifest(r#""127.0.0.1", "localhost""#, SECRETS),
    )
    .unwrap();
    let base = format!("http://127.0.0.1:{}", server.port);
    let mut with_key = fetch(format!("{base}/hello"));
    with_key
        .headers
        .insert("Authorization".to_owned(), "Bearer {{secret:api_key}}".to_owned());

    // Not set yet: the host asks the person (a secret prompt), keeps it.
    assert_eq!(
        call(&host, HostRequest::Secret { name: "api_key".to_owned() }).unwrap()["set"],
        false
    );
    let out = call(&host, HostRequest::Fetch(with_key.clone())).unwrap();
    assert_eq!(out["status"], 200);
    let asked = ui.asked();
    assert_eq!(asked.len(), 1, "asked once");
    let HostRequest::Prompt(prompt) = &asked[0].request else {
        panic!("{:?}", asked[0])
    };
    assert_eq!(prompt.kind, architect_extensions::hostcall::PromptKind::Secret);
    assert!(prompt.title.contains("Test API key"), "{prompt:?}");

    // The server got it; the extension's answer does not have it.
    let seen = server.heard.to("/hello");
    assert_eq!(
        seen.last().unwrap().1.get("authorization").map(String::as_str),
        Some("Bearer sk-test-123")
    );
    assert!(!out.to_string().contains("sk-test-123"), "{out}");

    // Asked once: the next request uses the kept one.
    call(&host, HostRequest::Fetch(with_key.clone())).unwrap();
    assert_eq!(ui.asked().len(), 1);

    // Readable by no call of its own: `secret` says whether, not what.
    let status = call(&host, HostRequest::Secret { name: "api_key".to_owned() }).unwrap();
    assert_eq!(status, json!({ "set": true, "title": "Test API key" }));
    for request in [
        HostRequest::Setting { key: "api_key".to_owned() },
        HostRequest::Data { key: "api_key".to_owned() },
    ] {
        let got = call(&host, request).unwrap_or(Value::Null);
        assert!(!got.to_string().contains("sk-test"), "{got}");
    }
    let read = call(
        &host,
        HostRequest::ReadFile {
            path: file.display().to_string(),
        },
    );
    assert!(matches!(read, Err(HostError::Refused { .. })), "{read:?}");
    // Kept by the app, not in the extension's folder.
    assert!(std::fs::read_to_string(&file).unwrap().contains("sk-test-123"));
    for entry in walk(&tmp.path().join("data")) {
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        assert!(!text.contains("sk-test-123"), "{}", entry.display());
    }
    // In no log line.
    for line in host.logs().since(NET, None, 1000) {
        assert!(!line.text.contains("sk-test-123"), "{}", line.text);
    }

    // To a host it may reach that is not the secret's: refused.
    let mut elsewhere = with_key.clone();
    elsewhere.url = format!("http://localhost:{}/hello", server.port);
    let (capability, _) = refused(call(&host, HostRequest::Fetch(elsewhere)));
    assert_eq!(capability, "secrets");
    // In the URL or the body: refused.
    let mut in_url = fetch(format!("{base}/hello?key={{{{secret:api_key}}}}"));
    let (capability, _) = refused(call(&host, HostRequest::Fetch(in_url.clone())));
    assert_eq!(capability, "secrets");
    in_url.url = format!("{base}/hello");
    in_url.body = Some("{{secret:api_key}}".to_owned());
    let (capability, _) = refused(call(&host, HostRequest::Fetch(in_url)));
    assert_eq!(capability, "secrets");
    // One it does not declare: refused.
    let mut undeclared = fetch(format!("{base}/hello"));
    undeclared
        .headers
        .insert("X-Key".to_owned(), "{{secret:other}}".to_owned());
    let (capability, _) = refused(call(&host, HostRequest::Fetch(undeclared)));
    assert_eq!(capability, "secrets");

    // A redirect to another host it may reach: followed, without the secret.
    let mut out = with_key;
    out.url = format!("{base}/redirect-out");
    call(&host, HostRequest::Fetch(out)).unwrap();
    let hop = server
        .heard
        .to("/hello")
        .into_iter()
        .find(|(h, _)| h.starts_with("localhost"))
        .expect("the hop to localhost");
    assert!(!hop.1.contains_key("authorization"), "{:?}", hop.1);
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

#[test]
fn a_secret_must_be_one_of_its_hosts() {
    let bad = manifest(r#""127.0.0.1""#, &SECRETS.replace("127.0.0.1", "api.example.com"));
    let m = architect_extensions::manifest::Manifest::parse(&bad);
    assert!(
        m.as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("api.example.com")),
        "{m:?}"
    );
}

// ── Document events need reads = ["documents"] (#308/#309) ─────────

const DOC_HOOK: &str = r#"
[[hooks]]
on = "doc.saved"
action = "LAB_NET_FETCH"
"#;

#[test]
fn a_declared_hook_on_a_documents_events_needs_reads_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let why = load(&host, &tmp.path().join("dev/net"), &manifest("", DOC_HOOK)).unwrap_err();
    assert!(why.contains("doc.saved") && why.contains("documents"), "{why}");
    assert!(host.get(NET).is_none());

    let with = manifest("", DOC_HOOK).replace("hosts = []", "hosts = []\nreads = [\"documents\"]");
    load(&host, &tmp.path().join("dev/net"), &with).unwrap();
    assert!(host.get(NET).is_some());
}

#[test]
fn a_hook_made_at_run_time_on_a_documents_events_needs_reads_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let (_list, host) = app(tmp.path(), &manifest("", ""));
    let hook = || {
        HostRequest::Subscribe(architect_extensions::contrib::HookDecl {
            on: "doc.saved".to_owned(),
            action: Some("LAB_NET_FETCH".to_owned()),
            ..Default::default()
        })
    };
    let (capability, _) = refused(call(&host, hook()));
    assert_eq!(capability, "reads.documents");
    assert!(host.subscriptions(NET).is_empty());
    // And none fires for it: a document saved starts nothing of its.
    let started = host.raise_doc(&architect_extensions::events::DocEvent::Saved {
        path: "notes/a.md".to_owned(),
    });
    assert_eq!(started, 0);

    // With the capability, it is made and hears it.
    let tmp = tempfile::tempdir().unwrap();
    let with = manifest("", "").replace("hosts = []", "hosts = []\nreads = [\"documents\"]");
    let (_list, host) = app(tmp.path(), &with);
    call(&host, hook()).unwrap();
    let started = host.raise_doc(&architect_extensions::events::DocEvent::Saved {
        path: "notes/a.md".to_owned(),
    });
    assert_eq!(started, 1);
}

#[test]
fn a_declared_document_hook_is_not_heard_by_an_installed_extension_not_granted_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()))
        .with_grants(architect_extensions::consent::Grants::in_memory());
    let with = manifest("", DOC_HOOK).replace("hosts = []", "hosts = []\nreads = [\"documents\"]");
    let dir = tmp.path().join("installed/net");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), with).unwrap();
    std::fs::write(dir.join("fetch.luau"), FETCH_LUAU).unwrap();
    host.load(&Package::read(&dir).unwrap(), Origin::Installed)
        .unwrap();
    let started = host.raise_doc(&architect_extensions::events::DocEvent::Saved {
        path: "notes/a.md".to_owned(),
    });
    assert_eq!(started, 0, "it asked for documents; no one granted them");
    let lines: Vec<String> = host.logs().since(NET, None, 1000).iter().map(|l| l.text.clone()).collect();
    assert!(
        lines.iter().any(|l| l.contains("doc.saved: refused")),
        "{lines:?}"
    );
}
