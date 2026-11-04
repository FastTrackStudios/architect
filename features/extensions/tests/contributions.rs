//! What an extension adds to the interface (#179, #239): a Luau
//! extension's openers, tabs, status items, context entries, hooks and
//! slots go into the host's registry as it loads, change as a diff when
//! its manifest is saved, and go — runtime state and all — when it is
//! turned off; its hooks run on startup and on the app's events; its
//! actions are told where they were invoked from; a script adds to the
//! interface at run time (`ext.contribute`, `ext.state`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, Invocation, InvocationSource};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, ViewOpen};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const ID: &str = "lab.board";

const MANIFEST: &str = r#"
id = "lab.board"
name = "Board"
version = "0.1.0"
kind = "luau"

[[actions]]
id = "LAB_BOARD_PIN"
name = "Pin"
script = "pin.luau"

[[actions]]
id = "LAB_BOARD_OPEN_ISSUE"
name = "Open an issue"
script = "issue.luau"

[[views]]
id = "board"
title = "Board"

[[views]]
id = "chart"
title = "Chart"
placement = "split"

[[views]]
id = "bar"
title = "Board status"

[[openers]]
id = "kf"
title = "Keyflow chart"
files = ["kf"]
view = "chart"

[[openers]]
id = "issue"
title = "GitHub issue"
urls = ['^https://github\.com/[^/]+/[^/]+/issues/\d+$']
action = "LAB_BOARD_OPEN_ISSUE"

[[tabs]]
id = "board"
title = "Board"
icon = "layout-dashboard"
view = "board"

[[status]]
id = "pins"
text = "No pins"

[[context]]
target = "view:note"
action = "LAB_BOARD_PIN"
title = "Pin to the board"

[[hooks]]
on = "startup"
script = "hooks.luau"
function = "started"

[[hooks]]
on = "doc.opened"
script = "hooks.luau"
function = "opened"

[[slots]]
slot = "status"
view = "bar"
"#;

const PIN: &str = r#"
ext.state{ key = "pins", text = "Pinned from " .. ctx.source .. " on " .. tostring(ctx.id) }
return { source = ctx.source, target = ctx.target, id = ctx.id }
"#;

const ISSUE: &str = "
return { url = ctx.url, source = ctx.source }
";

const HOOKS: &str = r#"
return {
    started = function(event)
        ext.state{ key = "started", text = "Board up (" .. ctx.event .. ")" }
        ext.contribute{ tabs = { { id = "live", title = "Live", view = "board" } } }
    end,
    opened = function(event)
        ext.state{ key = "opened", text = event.detail }
    end,
}
"#;

fn write(dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, text) in [
        ("extension.toml", manifest),
        ("pin.luau", PIN),
        ("issue.luau", ISSUE),
        ("hooks.luau", HOOKS),
    ] {
        std::fs::write(dir.join(name), text).unwrap();
    }
}

/// Waits for `ok` (hooks run on threads of their own).
fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run(list: &ActionList, id: &str, args: &Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap()
}

fn state_text(ext: &Extensions, key: &str) -> Option<String> {
    ext.host()
        .contributions()
        .state(architect_extensions::host::now_ms())
        .into_iter()
        .find(|s| s.source == format!("ext:{ID}") && s.key == key)
        .map(|s| s.text)
}

#[test]
fn an_extensions_contributions_come_and_go_with_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("board");
    write(&dir, MANIFEST);
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ViewOpen>::new()));
    let seen = std::sync::Arc::clone(&opened);
    host.set_view_opener(move |open| {
        seen.lock().unwrap().push(open);
        Ok(json!("opened"))
    });
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.dev(&dir);
    let reg = ext.host().contributions().clone();

    // Declared: in the registry as it loads.
    assert_eq!(reg.tabs()[0].item.id, "board");
    assert_eq!(reg.slots()["status"], (ID.to_owned(), "bar".to_owned()));
    // Its startup hook ran once: its state, and a tab added at run time.
    until("the startup hook", || {
        state_text(&ext, "started").is_some() && reg.tabs().len() == 2
    });
    assert_eq!(
        state_text(&ext, "started").unwrap(),
        "Board up (startup)",
        "the hook's ctx.event"
    );

    // Its action is told where it was invoked from: a context menu, on a
    // pane.
    let caller = ActionCaller::app()
        .with_invocation(Invocation::new(InvocationSource::ContextMenu).on("view:note", "note-1"));
    let out = block_on(list.run_json_async_with(&caller, "LAB_BOARD_PIN", b"")).expect("pinned");
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["source"], "context_menu");
    assert_eq!(out["id"], "note-1");
    assert_eq!(
        reg.status(architect_extensions::host::now_ms())[0]
            .item
            .text,
        "Pinned from context_menu on note-1",
        "a status item's text, from runtime state"
    );

    // A .kf file opens in its view; a link with its action; doc.opened
    // runs its hook.
    let answer = run(&list, "EXT_OPEN", &json!({ "path": "songs/intro.kf" }));
    assert_eq!(answer["opener"], "kf");
    let open = opened.lock().unwrap().last().cloned().unwrap();
    assert_eq!(open.kind, "lab.board/chart");
    assert_eq!(open.state.as_deref(), Some("songs/intro.kf"));
    assert_eq!(open.invocation.source, InvocationSource::Open);
    until("the doc.opened hook", || {
        state_text(&ext, "opened").as_deref() == Some("songs/intro.kf")
    });
    let answer = run(
        &list,
        "EXT_OPEN",
        &json!({ "url": "https://github.com/fts/architect/issues/179" }),
    );
    assert_eq!(answer["result"]["source"], "link");
    assert_eq!(
        answer["result"]["url"],
        "https://github.com/fts/architect/issues/179"
    );
    let nothing = block_on(list.run_json_async("EXT_OPEN", br#"{"path":"a.rpp"}"#));
    assert!(
        nothing
            .unwrap_err()
            .to_string()
            .contains("nothing opens a.rpp")
    );

    // An agent's state is its own; an extension's is not the list's to
    // write.
    run(
        &list,
        "EXT_STATE_SET",
        &json!({ "source": "agent:claude", "key": "turn", "text": "thinking", "ttl_ms": 60000 }),
    );
    assert!(
        reg.status(architect_extensions::host::now_ms())
            .iter()
            .any(|s| s.owner == "agent:claude" && s.item.text == "thinking")
    );
    let refused = block_on(list.run_json_async(
        "EXT_STATE_SET",
        br#"{"source":"ext:lab.board","key":"pins","text":"x"}"#,
    ));
    assert!(refused.is_err());

    // A saved manifest is a diff: the status item goes, the tab added at
    // run time stays.
    write(
        &dir,
        &MANIFEST.replace("[[status]]\nid = \"pins\"\ntext = \"No pins\"\n", ""),
    );
    let _ = ext.reload_changed();
    let keys = reg.of(ID).unwrap().keys();
    assert!(!keys.contains("status:pins"));
    assert!(keys.contains("tab:live"));
    // Not started again by a reload.
    assert_eq!(reg.tabs().len(), 2);

    // Turned off: every contribution and its runtime state go; the
    // agent's stays.
    let off = run(&list, "EXT_DISABLE", &json!({ "id": ID }));
    assert_eq!(off["events"][0]["unloaded"], ID);
    assert!(reg.tabs().is_empty() && reg.slots().is_empty());
    assert!(reg.of(ID).is_none());
    assert!(state_text(&ext, "pins").is_none());
    assert!(reg.state(0).iter().any(|s| s.source == "agent:claude"));
    let contributions = run(&list, "EXT_CONTRIBUTIONS", &json!({}));
    assert_eq!(contributions["extensions"], json!([]));
    // And on again: back as it was declared.
    run(&list, "EXT_ENABLE", &json!({ "id": ID }));
    assert_eq!(reg.tabs()[0].item.id, "board");
    let contributions = run(&list, "EXT_CONTRIBUTIONS", &json!({}));
    assert_eq!(contributions["extensions"][0]["extension"], ID);
    assert_eq!(contributions["slots"]["status"]["view"], "bar");
}

#[test]
fn an_app_event_runs_the_hooks_on_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("board");
    write(
        &dir,
        &MANIFEST.replace("on = \"doc.opened\"", "on = \"workspace.changed\""),
    );
    let list = ActionList::new();
    let app = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.watch(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_contribution_actions().unwrap();
    let _ = ext.dev(&dir);
    until("the startup hook", || state_text(&ext, "started").is_some());
    app.changed("workspace", "note-2");
    until("the workspace.changed hook", || {
        state_text(&ext, "opened").as_deref() == Some("note-2")
    });
    let started = run(
        &list,
        "EXT_EMIT",
        &json!({ "event": "workspace.changed", "detail": "x" }),
    );
    assert_eq!(started["started"], 1);
    assert!(
        block_on(list.run_json_async("EXT_EMIT", br#"{"event":"teatime"}"#)).is_err(),
        "an unknown event"
    );
}
