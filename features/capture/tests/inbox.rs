//! `CAPTURE_*` and `INBOX_*` over a real vault folder and its `VAULT_*`
//! actions, with a made-up extension's target: a capture lands in
//! `Inbox/` as markdown, and leaves it filed, made into the target's
//! thing, put off or discarded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::Path;
use std::sync::Arc;

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect, RawJson};
use architect_capture::actions::{self, Config, ids};
use serde_json::{Value, json};

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Test",
        method_name: id,
        display_name: id,
        description: "",
        category: "Test",
        group: "Test",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static REGISTRY: ActionMeta = meta("EXT_CAPTURE_TARGETS");
static MAKE: ActionMeta = meta("FAKE_CAPTURE");

/// A list over a vault in `dir`, with a target `task` whose action writes
/// `Tasks/<title>.md` with the capture's properties.
fn app(dir: &Path) -> Arc<ActionList> {
    let list = Arc::new(ActionList::new());
    let vault = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &vault).unwrap();
    list.register_with(&REGISTRY, Effect::ReadOnly, |_: &ActionCaller, (): ()| {
        Ok::<_, String>(RawJson::from(
            json!([{
                "id": "task", "name": "Task", "icon": "check", "action": "FAKE_CAPTURE",
                "chips": ["due"], "tokens": [{ "prefix": "key:", "key": "key" }], "from": "fake",
            }])
            .to_string(),
        ))
    })
    .unwrap();
    let weak = Arc::downgrade(&list);
    list.register_async_with(
        &MAKE,
        Effect::Mutating,
        move |cx: &ActionCaller, a: facet_value_args::Args| {
            let (list, cx) = (weak.upgrade().unwrap(), cx.clone());
            async move {
                let capture: Value = serde_json::from_str(&a.json()).unwrap();
                let title = capture["title"].as_str().unwrap().to_owned();
                let mut props = capture["properties"].as_object().cloned().unwrap();
                props.insert("type".into(), json!("task"));
                let path = format!("Tasks/{title}.md");
                let args = json!({
                    "path": path, "body": capture["body"],
                    "properties": Value::Object(props).to_string(),
                });
                list.run_json_async_with(&cx, "VAULT_CREATE", args.to_string().as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(RawJson::from(json!({ "path": path }).to_string()))
            }
        },
    )
    .unwrap();
    actions::register(&list, Config::default(), None).unwrap();
    list
}

/// The test's made-up target takes any JSON object.
mod facet_value_args {
    use facet::Facet;

    #[derive(Facet)]
    pub struct Args {
        #[facet(default)]
        pub title: Option<String>,
        #[facet(default)]
        pub body: Option<String>,
        #[facet(default)]
        pub properties: Option<facet_value::Value>,
        #[facet(default)]
        pub source: Option<String>,
        #[facet(default)]
        pub inbox: Option<String>,
    }

    impl Args {
        pub fn json(&self) -> String {
            let props = self
                .properties
                .as_ref()
                .and_then(|p| facet_json::to_string(p).ok())
                .unwrap_or_else(|| "{}".to_owned());
            format!(
                r#"{{"title":{},"body":{},"properties":{props}}}"#,
                serde_json::json!(self.title),
                serde_json::json!(self.body.clone().unwrap_or_default()),
            )
        }
    }
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    let out = futures::executor::block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map_err(|e| e.to_string())?;
    Ok(if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).unwrap()
    })
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

#[test]
fn a_capture_lands_in_the_inbox_as_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let list = app(dir.path());
    let out = run(
        &list,
        ids::ADD,
        &json!({
            "text": "Sketch the toolbar [[Lab]] tomorrow !high #design\nThree ideas",
            "properties": { "priority": "critical" },
            "source": "window",
        }),
    )
    .unwrap();
    assert_eq!(out["path"], "Inbox/Sketch the toolbar.md");
    assert_eq!(out["target"], "inbox");
    let md = read(dir.path(), "Inbox/Sketch the toolbar.md");
    for line in [
        "type: inbox-item",
        "status: open",
        "source: window",
        "priority: critical",
        "Three ideas",
    ] {
        assert!(md.contains(line), "{line:?} in:\n{md}");
    }
    assert!(md.contains("Lab"), "the project link:\n{md}");
    assert!(md.contains("design"), "the tag:\n{md}");
    // The same title again: a new note beside it.
    let again = run(&list, ids::ADD, &json!({ "text": "Sketch the toolbar" })).unwrap();
    assert_eq!(again["path"], "Inbox/Sketch the toolbar 2.md");
    let inbox = run(&list, ids::INBOX_LIST, &json!({})).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 2);
    assert_eq!(inbox[0]["title"], "Sketch the toolbar");
    assert_eq!(inbox[0]["body"], "Three ideas");
    assert_eq!(inbox[0]["properties"]["priority"], "critical");
    assert!(inbox[0]["properties"].get("status").is_none(), "the inbox's own keys stay home");
    assert!(run(&list, ids::ADD, &json!({ "text": "  " })).is_err());
}

#[test]
fn parse_and_targets_read_extensions_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let list = app(dir.path());
    let targets = run(&list, ids::TARGETS, &json!({})).unwrap();
    let ids_: Vec<&str> = targets
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids_, ["note", "task"]);
    let parsed = run(&list, ids::PARSE, &json!({ "text": "Bridge key:D [[Song]]" })).unwrap();
    assert_eq!(parsed["title"], "Bridge");
    assert_eq!(parsed["properties"]["key"], "D");
    assert_eq!(parsed["suggests"], "task");
    let plain = run(&list, ids::PARSE, &json!({ "text": "A thought" })).unwrap();
    assert_eq!(plain["suggests"], "note");
}

#[test]
fn processing_makes_the_target_s_thing_and_the_capture_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let list = app(dir.path());
    run(
        &list,
        ids::ADD,
        &json!({ "text": "Call the venue tomorrow\nAsk about load-in" }),
    )
    .unwrap();
    let made = run(
        &list,
        ids::INBOX_PROCESS,
        &json!({
            "path": "Inbox/Call the venue.md", "target": "task",
            "properties": { "priority": "high" },
        }),
    )
    .unwrap();
    assert_eq!(made["path"], "Tasks/Call the venue.md");
    assert_eq!(made["target"], "task");
    let md = read(dir.path(), "Tasks/Call the venue.md");
    assert!(md.contains("type: task"), "{md}");
    assert!(md.contains("priority: high"), "{md}");
    assert!(md.contains("due:"), "{md}");
    assert!(md.contains("Ask about load-in"), "{md}");
    assert!(!dir.path().join("Inbox/Call the venue.md").exists());
    assert!(
        run(&list, ids::INBOX_LIST, &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let err = run(&list, ids::INBOX_PROCESS, &json!({ "path": "Inbox/Gone.md", "target": "nope" }));
    assert!(err.unwrap_err().contains("no capture target"));
}

#[test]
fn file_defer_and_discard() {
    let dir = tempfile::tempdir().unwrap();
    let list = app(dir.path());
    for text in ["An idea #music", "Later thing", "Junk"] {
        run(&list, ids::ADD, &json!({ "text": text })).unwrap();
    }
    let filed = run(
        &list,
        ids::INBOX_FILE,
        &json!({ "path": "Inbox/An idea.md", "folder": "Notes" }),
    )
    .unwrap();
    assert_eq!(filed["path"], "Notes/An idea.md");
    let md = read(dir.path(), "Notes/An idea.md");
    assert!(md.contains("music"), "its tag kept:\n{md}");
    assert!(!md.contains("inbox-item") && !md.contains("source:"), "{md}");

    let put_off = run(
        &list,
        ids::INBOX_DEFER,
        &json!({ "path": "Inbox/Later thing.md", "until": "2099-01-01" }),
    )
    .unwrap();
    assert_eq!(put_off["resurface_on"], "2099-01-01");
    let now: Vec<String> = run(&list, ids::INBOX_LIST, &json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(now, ["Junk"], "put off until 2099");
    let all = run(&list, ids::INBOX_LIST, &json!({ "all": true })).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);

    run(&list, ids::INBOX_DISCARD, &json!({ "path": "Inbox/Junk.md" })).unwrap();
    assert!(!dir.path().join("Inbox/Junk.md").exists());
    assert!(run(&list, ids::INBOX_DISCARD, &json!({ "path": "Notes/An idea.md" })).is_err());
    assert!(
        run(&list, ids::INBOX_DEFER, &json!({ "path": "Inbox/Later thing.md", "until": "soonish" }))
            .unwrap_err()
            .contains("not a day")
    );
}

#[test]
fn straight_into_a_target_skips_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let list = app(dir.path());
    let out = run(
        &list,
        ids::ADD,
        &json!({ "text": "Pay rent !high", "target": "task" }),
    )
    .unwrap();
    assert_eq!(out["path"], "Tasks/Pay rent.md");
    let note = run(&list, ids::ADD, &json!({ "text": "Quote #x", "target": "note" })).unwrap();
    assert_eq!(note["path"], "Quote.md");
    assert!(!read(dir.path(), "Quote.md").contains("inbox-item"));
    assert!(!dir.path().join("Inbox").exists() || std::fs::read_dir(dir.path().join("Inbox")).unwrap().next().is_none());
    assert!(run(&list, ids::OPEN, &json!({})).unwrap_err().contains("no window"));
}
