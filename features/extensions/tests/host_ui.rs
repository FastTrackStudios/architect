//! The app's interface lent to extensions (#306): notifications, the
//! clipboard, pickers, prompts, a context menu and badges, on a host whose
//! interface is a `FakeUi` (it answers at once). A Luau extension makes
//! each call through `host` and gets the answers back; a page's way, the
//! `EXT_HOST_CALL` action, does the same as its extension; a file picked
//! is one it may then read; a decoration is on its row until it unloads;
//! and every call is refused outside `capabilities.ui`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use architect::action::{ActionCaller, ActionError, ActionList, Principal};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest, Target};
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const MANIFEST: &str = r#"
id = "lab.ask"
name = "Ask"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
ui = ["notify", "clipboard", "pickers", "prompts", "menus", "badges"]

[[actions]]
id = "LAB_ASK_ALL"
name = "Ask everything"
script = "all.luau"

[[actions]]
id = "LAB_ASK_READ_PICKED"
name = "Read a picked file"
script = "read_picked.luau"

[[contribute."app.decorations"]]
target = { kind = "file", id = "README.md" }
icon = "star"
"#;

const ALL: &str = r#"
host.notify("hello from Luau", "success", "Ask")
host.clipboard_write("copied by Luau")
local clip = host.clipboard_read()
local name = host.prompt({ kind = "text", title = "Name?", default = "untitled" })
local sure = host.prompt({ kind = "confirm", title = "Sure?" })
local which = host.prompt({ kind = "pick_one", title = "Which?", choices = {
    { id = "a", label = "A" }, { id = "b", label = "B" } } })
local path = host.pick({ kind = "save", name = "out.md" })
local chosen = host.menu({ x = 10, y = 20, items = {
    { id = "cut", label = "Cut", disabled = true }, { id = "copy", label = "Copy" } } })
host.decorate({ kind = "file", id = "notes/a.md" }, { icon = "star", badge = "3", color = "primary" })
return { clip = clip, name = name, sure = sure, which = which, path = path, chosen = chosen }
"#;

const READ_PICKED: &str = r#"
local path = host.pick({ kind = "open" })
if path == nil then return "cancelled" end
return host.read_file(path)
"#;

fn package(tmp: &Path, manifest: &str) -> std::path::PathBuf {
    let dir = tmp.join("dev/ask");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    std::fs::write(dir.join("all.luau"), ALL).unwrap();
    std::fs::write(dir.join("read_picked.luau"), READ_PICKED).unwrap();
    dir
}

fn app(tmp: &Path, ui: &FakeUi, manifest: &str) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.join("data"))
        .with_ui(ui.clone());
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")));
    ext.install_contribution_actions().unwrap();
    let _ = ext.dev(&package(tmp, manifest));
    assert!(ext.host().get("lab.ask").is_some(), "{:?}", ext.problems());
    (list, ext)
}

fn run(list: &ActionList, caller: &ActionCaller, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(caller, id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

#[test]
fn a_luau_extension_notifies_copies_asks_and_gets_the_answers_back() {
    let tmp = tempfile::tempdir().unwrap();
    let saved = tmp.path().join("out.md").display().to_string();
    let ui = FakeUi::new()
        .answer(json!("Ada"))
        .answer(json!(false))
        .answer(json!("b"))
        .answer(json!(saved));
    let (list, ext) = app(tmp.path(), &ui, MANIFEST);

    let out = run(&list, &ActionCaller::app(), "LAB_ASK_ALL", &Value::Null).unwrap();
    assert_eq!(
        out,
        json!({
            "clip": "copied by Luau",
            "name": "Ada",
            "sure": false,
            "which": "b",
            "path": saved,
            // The fake chose the first item that may be chosen.
            "chosen": "copy",
        })
    );
    let notices = ui.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        (notices[0].text.as_str(), notices[0].kind.as_str(), notices[0].from.name.as_str()),
        ("hello from Luau", "success", "Ask")
    );
    assert_eq!(ui.clipboard().as_deref(), Some("copied by Luau"));
    let asked: Vec<&str> = ui.asked().iter().map(|q| q.request.op()).collect();
    assert_eq!(asked, ["prompt", "prompt", "prompt", "pick", "menu"]);
    assert!(ui.asked().iter().all(|q| q.from == "lab.ask"));
    let HostRequest::Menu(menu) = &ui.asked()[4].request else {
        panic!()
    };
    assert_eq!((menu.x, menu.y), (Some(10.0), Some(20.0)));

    // Its badge on the row, after the manifest's (none there), and the
    // manifest's star on README.md.
    let row = ext.host().decorations_of(&Target::file("notes/a.md"));
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].from, "lab.ask");
    assert_eq!(row[0].decoration.badge.as_deref(), Some("3"));
    let readme = ext.host().decorations_of(&Target::file("README.md"));
    assert_eq!(readme[0].decoration.icon.as_deref(), Some("star"));
    let listed = run(&list, &ActionCaller::app(), "EXT_DECORATIONS", &Value::Null).unwrap();
    let targets: Vec<&str> = listed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(targets, ["README.md", "notes/a.md"]);
    // Unloaded, its decorations go with it.
    assert!(ext.host().unload("lab.ask"));
    assert!(ext.host().decorations_of(&Target::file("notes/a.md")).is_empty());
    assert!(ext.host().decorations_of(&Target::file("README.md")).is_empty());
}

#[test]
fn a_file_picked_is_one_it_may_read_and_a_cancel_is_nil() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("elsewhere/data.csv");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, "a,b\n1,2\n").unwrap();
    let ui = FakeUi::new()
        .answer(Value::Null)
        .answer(json!(outside.display().to_string()));
    let (list, ext) = app(tmp.path(), &ui, MANIFEST);
    // Not picked: outside its folders.
    let api = ext.host().api("lab.ask");
    let read = block_on(api.call(HostRequest::ReadFile {
        path: outside.display().to_string(),
    }));
    assert!(
        matches!(&read, Err(HostError::Refused { capability, .. }) if capability == "folders"),
        "{read:?}"
    );
    assert_eq!(
        run(&list, &ActionCaller::app(), "LAB_ASK_READ_PICKED", &Value::Null).unwrap(),
        json!("cancelled")
    );
    assert_eq!(
        run(&list, &ActionCaller::app(), "LAB_ASK_READ_PICKED", &Value::Null).unwrap(),
        json!("a,b\n1,2\n")
    );
    // Picked to open, not to write.
    let write = block_on(api.call(HostRequest::WriteFile {
        path: outside.display().to_string(),
        text: "x".to_owned(),
    }));
    assert!(matches!(write, Err(HostError::Refused { .. })), "{write:?}");
}

#[test]
fn a_page_asks_through_ext_host_call_as_its_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let ui = FakeUi::new().answer(json!("from the page"));
    let (list, _ext) = app(tmp.path(), &ui, MANIFEST);
    let as_ask = ActionCaller::new(Principal::extension("lab.ask", None));
    run(
        &list,
        &as_ask,
        "EXT_HOST_CALL",
        &json!({ "op": "notify", "text": "a page's" }),
    )
    .unwrap();
    assert_eq!(ui.notices()[0].from.id, "lab.ask");
    let answer = run(
        &list,
        &as_ask,
        "EXT_HOST_CALL",
        &json!({ "op": "prompt", "kind": "text", "title": "?" }),
    )
    .unwrap();
    assert_eq!(answer, json!("from the page"));
    // The app is no extension.
    let why = run(
        &list,
        &ActionCaller::app(),
        "EXT_HOST_CALL",
        &json!({ "op": "clipboard_read" }),
    )
    .unwrap_err();
    assert!(why.to_string().contains("only an extension"), "{why}");
}

#[test]
fn every_interface_call_is_refused_outside_its_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    let ui = FakeUi::new();
    // It asks for notifications only.
    let manifest = MANIFEST.replace(
        r#"ui = ["notify", "clipboard", "pickers", "prompts", "menus", "badges"]"#,
        r#"ui = ["notify"]"#,
    );
    let (list, ext) = app(tmp.path(), &ui, &manifest);
    let api = ext.host().api("lab.ask");
    let calls = [
        (r#"{"op":"clipboard_read"}"#, "ui.clipboard"),
        (r#"{"op":"clipboard_write","text":"x"}"#, "ui.clipboard"),
        (r#"{"op":"pick","kind":"open"}"#, "ui.pickers"),
        (r#"{"op":"prompt","kind":"confirm","title":"?"}"#, "ui.prompts"),
        (r#"{"op":"menu","items":[{"id":"a","label":"A"}]}"#, "ui.menus"),
        (
            r#"{"op":"decorate","target":{"kind":"tab","id":"note"},"decoration":{"badge":"1"}}"#,
            "ui.badges",
        ),
    ];
    for (call, needs) in calls {
        let request: HostRequest = serde_json::from_str(call).unwrap();
        match block_on(api.call(request)) {
            Err(HostError::Refused { capability, .. }) => assert_eq!(capability, needs, "{call}"),
            other => panic!("{call}: {other:?}"),
        }
    }
    assert!(ui.asked().is_empty(), "nothing reached the person");
    assert_eq!(ui.clipboard(), None);
    // In its log, each refusal.
    let refused = ext
        .host()
        .logs()
        .since("lab.ask", None, 100)
        .iter()
        .filter(|l| l.text.contains("refused (ui."))
        .count();
    assert_eq!(refused, calls.len());
    // The one it asked for works; through Luau, a refusal is its error.
    block_on(api.call(HostRequest::Notify {
        text: "ok".to_owned(),
        kind: None,
        title: None,
    }))
    .unwrap();
    let why = run(&list, &ActionCaller::app(), "LAB_ASK_ALL", &Value::Null).unwrap_err();
    assert!(why.to_string().contains("refused (ui.clipboard)"), "{why}");
}

#[test]
fn a_headless_host_says_it_cannot_ask() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    let _ = ext.dev(&package(tmp.path(), MANIFEST));
    let api = ext.host().api("lab.ask");
    let answer = block_on(api.call(serde_json::from_str(r#"{"op":"pick","kind":"folder"}"#).unwrap()));
    assert!(matches!(answer, Err(HostError::Unsupported { .. })), "{answer:?}");
    // A notification is still its log's.
    block_on(api.call(HostRequest::Notify {
        text: "into the log".to_owned(),
        kind: Some("warning".to_owned()),
        title: None,
    }))
    .unwrap();
    assert!(
        ext.host()
            .logs()
            .since("lab.ask", None, 100)
            .iter()
            .any(|l| l.text == "into the log")
    );
}
