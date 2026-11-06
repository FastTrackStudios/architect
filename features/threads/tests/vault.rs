//! Threads against a real vault, as the app hosts it: the app's list
//! holds the vault's and Bases' actions over a folder, and the extension
//! reaches them only through the host service. A thread on a note, a
//! reply, a mention, resolve and reopen, counts and delete, each read
//! back from the markdown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use architect_threads::{ID, SERVICE, Threads};
use futures::executor::block_on;
use serde_json::{Value, json};

const NOTE: &str = "Projects/Album.md";

/// The app's side: its list with `VAULT_*` and `BASES_*` over `dir`.
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

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

#[test]
fn a_conversation_on_a_note_lives_in_the_vault() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(dir.join("Projects")).unwrap();
    std::fs::write(dir.join(NOTE), "---\ntitle: Album\n---\n# Album\n").unwrap();
    let app = app(&dir);
    run(&app, "VAULT_RELOAD", Value::Null);

    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Threads)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    assert_eq!(
        ext.host()
            .provider(SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        ID
    );

    // A thread on the note: its own note in Threads/, linking back.
    let started = run(
        &list,
        "THREADS_START",
        json!({ "path": NOTE, "body": "Is the mix done? @alex", "author": "cody" }),
    );
    let thread = started["thread"]["id"].as_str().unwrap().to_owned();
    let path = started["thread"]["path"].as_str().unwrap().to_owned();
    assert!(path.starts_with("Threads/album-"), "{path}");
    assert_eq!(started["comment"]["mentions"], json!(["alex"]));
    // No Notify loaded: nobody is told, the comment still lands.
    assert_eq!(started["notified"], json!([]));
    let text = read(&dir, &path);
    for line in [
        "type: thread",
        "entityType: note",
        "entityId: Projects/Album.md",
        "author: cody",
        "[[Projects/Album]]",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }

    // A reply under the first comment.
    let first = started["comment"]["id"].as_str().unwrap().to_owned();
    let replied = run(
        &list,
        "THREADS_REPLY",
        json!({ "thread": thread, "comment": first, "body": "Nearly", "author": "alex" }),
    );
    assert_eq!(replied["comment"]["replyTo"], json!(first));
    let listed = run(&list, "THREADS_LIST", json!({ "path": NOTE }));
    assert_eq!(listed["open"], 1);
    assert_eq!(
        listed["threads"][0]["comments"].as_array().unwrap().len(),
        2
    );

    // Resolve, then a new comment reopens it.
    let resolved = run(
        &list,
        "THREADS_RESOLVE",
        json!({ "thread": thread, "by": "cody" }),
    );
    assert_eq!(resolved["resolved"], true);
    assert_eq!(resolved["resolvedBy"], "cody");
    assert_eq!(
        run(
            &list,
            "THREADS_LIST",
            json!({ "path": NOTE, "resolved": false })
        )["open"],
        0
    );
    let counts = run(&list, "THREADS_COUNTS", json!({}));
    assert_eq!(counts["note:Projects/Album.md"]["open"], 0);
    assert_eq!(counts["note:Projects/Album.md"]["threads"], 1);
    run(
        &list,
        "THREADS_POST",
        json!({ "thread": thread, "body": "One more pass", "author": "cody" }),
    );
    assert_eq!(
        run(&list, "THREADS_GET", json!({ "thread": thread }))["resolved"],
        false
    );

    // Mentions, then any entity: a project by its id.
    let m = run(&list, "THREADS_MENTIONS", json!({ "who": "Alex" }));
    assert_eq!(m["mentions"].as_array().unwrap().len(), 1);
    let other = run(
        &list,
        "THREADS_START",
        json!({ "entity_type": "project", "entity_id": "album", "anchor_path": "/goals/q1/", "body": "Goal?", "author": "cody" }),
    );
    assert_eq!(other["thread"]["anchorPath"], "goals/q1");
    let at = run(
        &list,
        "THREADS_LIST",
        json!({ "entity_type": "project", "entity_id": "album", "anchor_path": "goals" }),
    );
    assert_eq!(at["threads"].as_array().unwrap().len(), 1);

    // Delete: the note goes.
    run(&list, "THREADS_DELETE", json!({ "thread": thread }));
    assert!(!dir.join(&path).exists());
    assert_eq!(
        run(&list, "THREADS_LIST", json!({ "path": NOTE }))["threads"],
        json!([])
    );
}
