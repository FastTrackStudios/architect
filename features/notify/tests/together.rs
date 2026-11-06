//! Notify with Threads and Tasks, as the app links them (#348): an
//! @mention in a comment notifies the person mentioned, a reply the
//! thread's other participants, a task's new assignee is notified, and
//! "go to" on a mention asks Threads to reveal the thread.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::MemoryValues;
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn app(dir: &Path) -> Arc<ActionList> {
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &open).unwrap();
    let types: &'static architect_bases::model::ViewTypes =
        Box::leak(Box::new(architect_bases::model::ViewTypes::new()));
    architect_bases::actions::register(&list, &open, types).unwrap();
    list
}

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

#[test]
fn a_mention_and_an_assignment_notify() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(dir.join("Notes")).unwrap();
    std::fs::write(dir.join("Notes/Album.md"), "# Album\n").unwrap();
    let app = app(&dir);
    run(&app, "VAULT_RELOAD", Value::Null);
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_ui(FakeUi::new())
        .with_setting_values(MemoryValues::default());
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&architect_notify::Notify)
        .unwrap()
        .with_linked(&architect_threads::Threads)
        .unwrap()
        .with_linked(&architect_tasks::Tasks)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    run(&list, "NOTIFY_ME", json!({ "name": "cody" }));

    // Cody comments, mentioning Alex: Alex is told; Cody, the author, not.
    let started = run(
        &list,
        "THREADS_START",
        json!({ "path": "Notes/Album.md", "body": "@alex is the mix done?" }),
    );
    assert_eq!(
        started["comment"]["author"], "cody",
        "the author is who Notify says you are"
    );
    assert_eq!(started["notified"], json!(["alex"]));
    let alex = run(&list, "NOTIFY_LIST", json!({ "who": "alex" }));
    let n = &alex["notifications"][0];
    assert_eq!(n["kind"], "mention");
    assert_eq!(n["title"], "cody mentioned you on Album");
    assert_eq!(n["body"], "@alex is the mix done?");
    assert_eq!(n["source"]["service"], architect_threads::SERVICE);
    assert_eq!(n["source"]["href"], "Notes/Album.md");

    // Alex replies, mentioning Sam: Sam by mention, Cody as a participant.
    let thread = started["thread"]["id"].as_str().unwrap();
    let replied = run(
        &list,
        "THREADS_POST",
        json!({ "thread": thread, "body": "Nearly, @Sam has the stems", "author": "alex" }),
    );
    assert_eq!(replied["notified"], json!(["Sam", "cody"]));
    let mine = run(&list, "NOTIFY_LIST", json!({}));
    assert_eq!(mine["unread"], 1);
    assert_eq!(mine["notifications"][0]["kind"], "thread-reply");
    assert_eq!(mine["notifications"][0]["title"], "alex replied on Album");

    // Go to: Threads reveals the thread (no window here to show it in).
    let id = mine["notifications"][0]["id"].as_str().unwrap();
    let went = run(&list, "NOTIFY_OPEN", json!({ "id": id }));
    assert_eq!(
        went["went"]["revealed_by"],
        architect_threads::SERVICE,
        "{went}"
    );
    assert_eq!(went["went"]["answer"]["path"], "Notes/Album.md");
    assert_eq!(run(&list, "NOTIFY_UNREAD", json!({}))["unread"], 0);

    // A task assigned to Alex (by link) and Cody: Alex is told; Cody, who
    // assigned it, not.
    let task = run(
        &list,
        "TASKS_CREATE",
        json!({ "title": "Bounce the stems", "assignees": ["[[People/Alex]]", "cody"], "due": "2026-11-09" }),
    );
    let alex = run(&list, "NOTIFY_LIST", json!({ "who": "alex" }));
    let n = &alex["notifications"][0];
    assert_eq!(n["kind"], "task-assigned");
    assert_eq!(n["title"], "cody assigned you Bounce the stems");
    assert_eq!(n["body"], "Due 2026-11-09");
    assert_eq!(n["source"]["href"], task["path"]);
    assert_eq!(run(&list, "NOTIFY_UNREAD", json!({}))["unread"], 0);

    // Reassigned: only the one who is new is told.
    let updated = run(
        &list,
        "TASKS_UPDATE",
        json!({ "path": task["path"], "assignees": ["[[People/Alex]]", "Robin"] }),
    );
    assert_eq!(updated["notified"], json!(["Robin"]));
    assert_eq!(
        run(&list, "NOTIFY_UNREAD", json!({ "who": "robin" }))["unread"],
        1
    );
}
