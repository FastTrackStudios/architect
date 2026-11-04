//! Toolbars and context-menu entries as an extension's contributions
//! (#307): a manifest's `[[toolbars]]` (in a dock or a pane's header, each
//! button an action), a toggle action (`toggle = true`) that says whether
//! it is on through its runtime state, and `[[context]]` rows on a tab's
//! and a file's context menus, run told what they were opened on. A bad
//! toolbar is a problem `APP_STATUS` lists, not a crash. A person's copy
//! of a toolbar (a file, `TOOLBAR_SAVE`) is used in its place until it is
//! reset; an armed action is the toolbars' to show.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use architect::action::{ActionCaller, ActionList, Invocation, InvocationSource};
use architect_extensions::contrib::ToolbarPlace;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use architect_workspace::{PaneId, context_invocation};
use futures::executor::block_on;
use serde_json::Value;

const ID: &str = "lab.tools";

const MANIFEST: &str = r#"
id = "lab.tools"
name = "Tools"
version = "0.1.0"
kind = "luau"

[[actions]]
id = "LAB_TOOLS_LOOP"
name = "Loop"
effect = "mutating"
script = "loop.luau"
toggle = true

[[actions]]
id = "LAB_TOOLS_WHERE"
name = "Where"
script = "where.luau"

[[toolbars]]
id = "transport"
title = "Transport"
where = "dock:right"

[[toolbars.items]]
action = "LAB_TOOLS_LOOP"
icon = "timer"

[[toolbars.items]]
action = "LAB_TOOLS_WHERE"
label = "Where am I"

[[toolbars]]
id = "notes"
title = "Note tools"
where = "view:note"

[[toolbars.items]]
action = "LAB_TOOLS_LOOP"

[[toolbars]]
id = "every"
title = "Every pane"
where = "pane"

[[toolbars.items]]
action = "LAB_TOOLS_WHERE"

[[context]]
target = "tab"
action = "LAB_TOOLS_WHERE"
title = "Where is this tab"

[[context]]
target = "file:md"
action = "LAB_TOOLS_WHERE"
title = "Where is this file"
"#;

/// The loop, on or off: its state says which.
const LOOP: &str = r#"
local on = host.data("loop") ~= true
host.set_data("loop", on)
ext.state{ kind = "checked", key = "LAB_TOOLS_LOOP", text = if on then "on" else "off" }
return { on = on }
"#;

/// What it was run on.
const WHERE: &str = "
return { source = ctx.source, target = ctx.target, id = ctx.id, pane = ctx.pane, view = ctx.view, path = ctx.path }
";

fn write(dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, text) in [
        ("extension.toml", manifest),
        ("loop.luau", LOOP),
        ("where.luau", WHERE),
    ] {
        std::fs::write(dir.join(name), text).unwrap();
    }
}

fn extensions(tmp: &Path) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")));
    (list, ext)
}

fn checked(list: &ActionList, id: &str) -> Option<bool> {
    list.info(id).unwrap().checked
}

fn run_with(list: &ActionList, id: &str, invocation: Invocation) -> Value {
    let caller = ActionCaller::app().with_invocation(invocation);
    let out = block_on(list.run_json_async_with(&caller, id, b"")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn a_toggle_says_whether_it_is_on_and_its_toolbars_are_where_they_say() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let (list, ext) = extensions(tmp.path());
    let _ = ext.dev(&dir);
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());

    // A toggle is off until it says otherwise; an action that is not one
    // has no tick at all.
    assert_eq!(checked(&list, "LAB_TOOLS_LOOP"), Some(false));
    assert_eq!(checked(&list, "LAB_TOOLS_WHERE"), None);
    // Run from its toolbar's button: on, then off, as its state says.
    let toolbar =
        Invocation::new(InvocationSource::Toolbar).on("dock:right", "lab.tools/transport");
    assert_eq!(
        run_with(&list, "LAB_TOOLS_LOOP", toolbar.clone())["on"],
        true
    );
    assert_eq!(checked(&list, "LAB_TOOLS_LOOP"), Some(true));
    let reg = ext.host().contributions().clone();
    assert!(reg.checked(ID, "LAB_TOOLS_LOOP", architect_extensions::host::now_ms()));
    assert_eq!(
        run_with(&list, "LAB_TOOLS_LOOP", toolbar.clone())["on"],
        false
    );
    assert_eq!(checked(&list, "LAB_TOOLS_LOOP"), Some(false));
    // A button's run is told it came from a toolbar, and which.
    let told = run_with(&list, "LAB_TOOLS_WHERE", toolbar);
    assert_eq!(told["source"], "toolbar");
    assert_eq!(told["target"], "dock:right");
    assert_eq!(told["id"], "lab.tools/transport");

    // Its toolbars: one in a dock, one on notes' headers, one on every
    // pane's.
    let bars = reg.toolbars();
    assert_eq!(bars.len(), 3);
    assert_eq!(
        bars[0].item.place(),
        Some(ToolbarPlace::Dock("right".to_owned()))
    );
    assert_eq!(bars[0].item.items.len(), 2);
    let ids = |kind: &str| -> Vec<String> {
        reg.header_toolbars(kind)
            .into_iter()
            .map(|b| b.item.id)
            .collect()
    };
    assert_eq!(ids("note"), ["notes", "every"]);
    assert_eq!(ids("files"), ["every"]);

    // Turned off, its toggle goes with it, and so do its toolbars.
    let _ = ext.disable(ID);
    assert!(list.info("LAB_TOOLS_LOOP").is_none());
    assert!(reg.toolbars().is_empty());
}

#[test]
fn its_entries_on_a_tab_and_a_file_run_told_what_they_were_opened_on() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let (list, ext) = extensions(tmp.path());
    let _ = ext.dev(&dir);
    let reg = ext.host().contributions().clone();

    // A tab's menu has its row; a Markdown file's has the other; a file of
    // another kind has none.
    let tab = reg.context("tab", "note-welcome");
    assert_eq!(tab.len(), 1);
    assert_eq!(tab[0].item.title, "Where is this tab");
    let file = reg.context("file", "Notes/Welcome.md");
    assert_eq!(file.len(), 1);
    assert_eq!(file[0].item.title, "Where is this file");
    assert!(reg.context("file", "Songs/Intro.kf").is_empty());

    // Each runs with the invocation the workspace's menu gives it.
    let pane = PaneId::new("note-welcome");
    let told = run_with(
        &list,
        &tab[0].item.action,
        context_invocation("tab", "note-welcome", &pane, "note"),
    );
    assert_eq!(told["source"], "context_menu");
    assert_eq!(told["target"], "tab");
    assert_eq!(told["id"], "note-welcome");
    assert_eq!(told["pane"], "note-welcome");
    assert_eq!(told["view"], "note");
    assert!(told.get("path").is_none_or(Value::is_null), "{told}");

    let files = PaneId::new("files");
    let told = run_with(
        &list,
        &file[0].item.action,
        context_invocation("file", "Notes/Welcome.md", &files, "files"),
    );
    assert_eq!(told["target"], "file");
    assert_eq!(told["id"], "Notes/Welcome.md");
    assert_eq!(told["path"], "Notes/Welcome.md");
    assert_eq!(told["pane"], "files");
    assert_eq!(told["view"], "files");
}

#[test]
fn a_bad_toolbar_is_a_problem_app_status_lists_not_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, ext) = extensions(tmp.path());
    architect::action::app_status::register(&list).unwrap();
    ext.report_problems(list.problems());

    for (bad, why) in [
        (
            MANIFEST.replace("where = \"dock:right\"", "where = \"dock:top\""),
            "\"dock:top\" is not a place",
        ),
        (
            MANIFEST.replace(
                "[[toolbars.items]]\naction = \"LAB_TOOLS_LOOP\"\n\n[[toolbars]]\nid = \"every\"",
                "[[toolbars]]\nid = \"every\"",
            ),
            "it has no items",
        ),
        (
            MANIFEST.replace(
                "[[toolbars.items]]\naction = \"LAB_TOOLS_LOOP\"\nicon",
                "[[toolbars.items]]\naction = \"WORKSPACE_CLOSE\"\nicon",
            ),
            "item 0: WORKSPACE_CLOSE is neither its own",
        ),
    ] {
        let dir = tmp.path().join("tools");
        write(&dir, &bad);
        let _ = ext.dev(&dir);
        let status = String::from_utf8(list.run_json("APP_STATUS", b"").unwrap()).unwrap();
        let status: Value = serde_json::from_str(&status).unwrap();
        let problems: Vec<&str> = status["problems"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["message"].as_str())
            .collect();
        let problems = problems.join("\n");
        assert!(
            problems.contains("toolbar") && problems.contains(why),
            "{why}: {problems}"
        );
        // Nothing of it landed; the app runs on.
        assert!(ext.host().get(ID).is_none());
        assert!(ext.host().contributions().toolbars().is_empty());
        let _ = ext.end_dev(&dir);
    }
    // Put right, it loads, and the problem goes.
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let _ = ext.dev(&dir);
    assert!(ext.host().get(ID).is_some());
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
}

#[test]
fn a_persons_copy_is_used_in_its_place_and_an_action_arms() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let (list, ext) = extensions(tmp.path());
    ext.install_contribution_actions().unwrap();
    ext.host().toolbars().set_dir(tmp.path().join("toolbars"));
    let _ = ext.dev(&dir);
    let run = |id: &str, args: Value| -> Value {
        let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        serde_json::from_slice(&out).unwrap()
    };

    let all = run("TOOLBAR_LIST", serde_json::json!({}));
    assert_eq!(all["toolbars"][0]["key"], "lab.tools/transport");
    assert_eq!(all["toolbars"][0]["source"], "extension");
    let got = run(
        "TOOLBAR_GET",
        serde_json::json!({ "id": "lab.tools/transport" }),
    );
    assert_eq!(got["toolbar"]["items"][1]["label"], "Where am I");

    // The person's copy: a dropdown in place of the second button.
    let mine = serde_json::json!({
        "title": "My transport",
        "where": "float",
        "items": [
            { "action": "LAB_TOOLS_LOOP", "right_click": "arm" },
            { "label": "More", "menu": [{ "kind": "heading", "label": "Here" }, { "action": "LAB_TOOLS_WHERE" }] },
        ],
    });
    let saved = run(
        "TOOLBAR_SAVE",
        serde_json::json!({ "id": "lab.tools/transport", "toolbar": mine.to_string() }),
    );
    assert!(
        saved["file"]
            .as_str()
            .unwrap()
            .ends_with("lab.tools-transport.toml")
    );
    let reg = ext.host().contributions().clone();
    let t = ext
        .host()
        .toolbars()
        .get(&reg, "lab.tools/transport")
        .unwrap();
    assert_eq!(t.decl.title, "My transport");
    assert_eq!(t.decl.items[1].menu[1].action, "LAB_TOOLS_WHERE");
    let all = run("TOOLBAR_LIST", serde_json::json!({}));
    assert_eq!(all["toolbars"][0]["source"], "override");
    assert_eq!(all["toolbars"][0]["where"], "float");
    // A bad one is refused, and nothing changes.
    let bad = block_on(list.run_json_async(
        "TOOLBAR_SAVE",
        br#"{"id": "lab.tools/transport", "toolbar": "{\"title\": \"x\", \"where\": \"dock:top\", \"items\": []}"}"#,
    ));
    assert!(bad.is_err());

    // Armed: one at a time, an action there is.
    run(
        "TOOLBAR_ARM",
        serde_json::json!({ "action": "LAB_TOOLS_LOOP" }),
    );
    assert_eq!(
        ext.host().toolbars().armed().as_deref(),
        Some("LAB_TOOLS_LOOP")
    );
    assert!(block_on(list.run_json_async("TOOLBAR_ARM", br#"{"action": "NOPE"}"#)).is_err());
    let off = run("TOOLBAR_DISARM", serde_json::json!({}));
    assert_eq!(off["disarmed"], "LAB_TOOLS_LOOP");

    // Reset: the extension's is back.
    assert_eq!(
        run(
            "TOOLBAR_RESET",
            serde_json::json!({ "id": "lab.tools/transport" })
        )["reset"],
        true
    );
    let t = ext
        .host()
        .toolbars()
        .get(&reg, "lab.tools/transport")
        .unwrap();
    assert_eq!(t.decl.title, "Transport");
}

/// The visual editor's edits, through `TOOLBAR_*` alone (#313): a toolbar
/// from nothing (an action, a dropdown, a widget), regrouped, restyled,
/// read back after a restart; an edit from an item's context menu needs no
/// arguments; every write keeps the file it replaced.
#[test]
fn a_toolbar_is_built_from_empty_through_its_actions_and_survives_a_restart() {
    use serde_json::json;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let (list, ext) = extensions(tmp.path());
    ext.install_contribution_actions().unwrap();
    let folder = tmp.path().join("toolbars");
    ext.host().toolbars().set_dir(&folder);
    let _ = ext.dev(&dir);
    let run = |id: &str, args: Value| -> Value {
        let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        serde_json::from_slice(&out).unwrap()
    };
    let refused = |id: &str, args: Value| {
        block_on(list.run_json_async(id, args.to_string().as_bytes())).unwrap_err()
    };

    assert_eq!(run("TOOLBAR_EDIT", json!({}))["editing"], true);
    assert_eq!(checked(&list, "TOOLBAR_EDIT"), Some(true));
    let key = run(
        "TOOLBAR_NEW",
        json!({ "title": "My tools", "where": "float" }),
    )["key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(key, "user/my-tools");
    let id = |more: Value| {
        let mut v = json!({ "id": key });
        v.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        v
    };
    // An action, a dropdown, a widget, another action.
    run(
        "TOOLBAR_INSERT",
        id(json!({ "item": { "action": "LAB_TOOLS_LOOP" } })),
    );
    run(
        "TOOLBAR_INSERT",
        id(
            json!({ "item": { "label": "More", "menu": [{ "kind": "heading", "label": "Here" }] } }),
        ),
    );
    run(
        "TOOLBAR_INSERT",
        id(json!({ "item": { "kind": "widget", "view": "lab.tools/meter", "width": 90 } })),
    );
    let at = run(
        "TOOLBAR_INSERT",
        id(json!({ "at": "0", "item": { "action": "LAB_TOOLS_WHERE" } })),
    );
    assert_eq!(at["at"], "0");
    // A row in the dropdown; the two buttons as one group, the dropdown
    // dragged into it.
    run(
        "TOOLBAR_SET",
        id(
            json!({ "at": "2", "set": { "menu": [{ "kind": "heading", "label": "Here" }, { "action": "LAB_TOOLS_WHERE" }] } }),
        ),
    );
    run(
        "TOOLBAR_GROUP",
        id(json!({ "at": "0", "count": 2, "label": "Both" })),
    );
    let moved = run("TOOLBAR_MOVE", id(json!({ "from": "1", "to": "0.2" })));
    assert_eq!(moved["at"], "0.2");
    // Restyled: a button's label and colour, the toolbar's corners and preset.
    run(
        "TOOLBAR_SET",
        id(json!({ "at": "0.1", "set": { "label": "Loop it", "colors": { "fill": "primary" } } })),
    );
    run(
        "TOOLBAR_SET",
        id(
            json!({ "set": { "style": { "rounding": 10, "preset": "soft" }, "hide_labels": true } }),
        ),
    );
    run("TOOLBAR_RENAME", id(json!({ "title": "Mine" })));
    // What a refused edit says, and that it changed nothing.
    let bad = refused(
        "TOOLBAR_SET",
        id(json!({ "set": { "style": { "size": 500 } } })),
    );
    assert!(bad.to_string().contains("style.size"), "{bad}");
    assert!(
        refused("TOOLBAR_MOVE", id(json!({ "from": "0", "to": "0.1" })))
            .to_string()
            .contains("group")
    );

    let want = |t: &Value| {
        assert_eq!(t["title"], "Mine");
        assert_eq!(t["style"]["preset"], "soft");
        assert_eq!(t["items"][0]["kind"], "group");
        assert_eq!(t["items"][0]["items"][1]["label"], "Loop it");
        assert_eq!(
            t["items"][0]["items"][2]["menu"][1]["action"],
            "LAB_TOOLS_WHERE"
        );
        assert_eq!(t["items"][1]["view"], "lab.tools/meter");
    };
    want(&run("TOOLBAR_GET", id(json!({})))["toolbar"]);

    // From the item's context menu: no arguments, the item it was opened on.
    let caller = ActionCaller::app().with_invocation(
        Invocation::new(InvocationSource::ContextMenu).on("toolbar-item", format!("{key}#1")),
    );
    block_on(list.run_json_async_with(&caller, "TOOLBAR_SELECT", b"")).unwrap();
    assert_eq!(
        ext.host().toolbars().selected(),
        Some((key.clone(), Some("1".to_owned())))
    );
    // Every write kept the one it replaced.
    let kept = run("TOOLBAR_BACKUPS", id(json!({})))["backups"]
        .as_array()
        .unwrap()
        .len();
    assert!(kept >= 8, "{kept} backups");

    // A restart: the same folder, read again.
    let (list2, ext2) = extensions(tmp.path());
    ext2.install_contribution_actions().unwrap();
    ext2.host().toolbars().set_dir(&folder);
    let _ = ext2.dev(&dir);
    let out = block_on(list2.run_json_async("TOOLBAR_GET", id(json!({})).to_string().as_bytes()))
        .unwrap();
    let again: Value = serde_json::from_slice(&out).unwrap();
    want(&again["toolbar"]);

    // Duplicated and shown in its place; the original deleted.
    let copy = run("TOOLBAR_DUPLICATE", id(json!({ "title": "Second" })))["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let shown = run("TOOLBAR_SHOW", json!({ "id": copy, "where": "float" }));
    assert_eq!(shown["hidden"], json!([key]));
    run("TOOLBAR_DELETE", id(json!({})));
    let all = run("TOOLBAR_LIST", json!({}));
    let keys: Vec<&str> = all["toolbars"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["key"].as_str())
        .collect();
    assert!(
        keys.contains(&copy.as_str()) && !keys.contains(&key.as_str()),
        "{keys:?}"
    );
    assert!(
        refused("TOOLBAR_DELETE", json!({ "id": "lab.tools/transport" }))
            .to_string()
            .contains("extension")
    );
    run("TOOLBAR_EDIT", json!({ "on": false }));
    assert_eq!(checked(&list, "TOOLBAR_EDIT"), Some(false));
}

/// Context-menu targets are points the features that own them declare
/// (#313): the workspace's and the toolbars' are there with the host; an
/// extension's rows are items of their target's point; rows for a target
/// no feature declared wait until one does.
#[test]
fn context_targets_are_points_their_features_declare() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools");
    write(&dir, MANIFEST);
    let (_list, ext) = extensions(tmp.path());
    let host = ext.host();
    let _ = ext.dev(&dir);
    let points = host.points();
    for p in [
        "context.tab",
        "context.pane",
        "context.view",
        "context.toolbar",
        "context.toolbar-item",
    ] {
        assert!(points.declared(p).is_some(), "{p}");
    }
    let tab = points.items("context.tab");
    assert_eq!(tab.len(), 1);
    assert_eq!(tab[0].from, ID);
    assert_eq!(tab[0].value["action"], "LAB_TOOLS_WHERE");
    // `file:md` is `context.file`'s: nobody here owns files, so it waits.
    assert!(points.declared("context.file").is_none());
    assert!(points.waiting().contains_key("context.file"));
    host.declare_context_targets("lab.files", &[("file", "a file-tree row")]);
    assert_eq!(points.items("context.file").len(), 1);
    assert!(!points.waiting().contains_key("context.file"));
    assert!(points.rejected().is_empty(), "{:?}", points.rejected());
}
