//! Notify against a real vault, as the app hosts it: notifications for a
//! person land in their note, the bell counts the unread ones, a row
//! another device wrote shows as a toast, read / read all / clear / go to
//! change the note, and the push relay is a seam nothing fills yet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, now_ms};
use architect_extensions::hostapi::MemoryValues;
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_notify::{BELL, ID, Notify, SERVICE};
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

fn bell(ext: &Extensions) -> String {
    ext.host()
        .contributions()
        .status(now_ms())
        .into_iter()
        .find(|s| s.owner == ID && s.item.id == BELL)
        .map(|s| s.item.text)
        .unwrap_or_default()
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_notification_lands_in_the_persons_note_and_on_the_bell() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    let app = app(&dir);
    run(&app, "VAULT_RELOAD", Value::Null);
    let list = ActionList::new();
    let ui = FakeUi::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_ui(ui.clone())
        .with_setting_values(MemoryValues::default());
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Notify)
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

    assert_eq!(
        run(&list, "NOTIFY_ME", json!({ "name": "cody" }))["me"],
        "cody"
    );
    // Nothing yet: no note, no count.
    assert_eq!(run(&list, "NOTIFY_UNREAD", json!({}))["unread"], 0);

    // To two people, by the one who caused it: the actor is left out.
    let sent = run(
        &list,
        "NOTIFY_POST",
        json!({
            "to": ["cody", "alex", "Sam"], "actor": "sam", "kind": "task-assigned",
            "title": "Sam assigned you Mix the album", "source": { "service": "architect.tasks", "href": "Tasks/Mix.md" }
        }),
    );
    let sent = sent["sent"].as_array().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["path"], "Notifications/cody.md");
    assert_eq!(sent[0]["pushed"], false, "no push relay yet");
    let text = std::fs::read_to_string(dir.join("Notifications/alex.md")).unwrap();
    for line in [
        "type: notifications",
        "for: alex",
        "kind: task-assigned",
        "actor: sam",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    until("the bell to count it", || bell(&ext) == "1");

    // A row another device wrote (as sync does): it toasts.
    let note = dir.join("Notifications/cody.md");
    let synced = std::fs::read_to_string(&note).unwrap().replace(
        "items:\n",
        "items:\n- id: from-elsewhere\n  kind: mention\n  title: alex mentioned you on Album\n  body: can you check the mix?\n  actor: alex\n  created: 2026-11-06T20:00:00Z\n",
    );
    std::fs::write(&note, synced).unwrap();
    run(&app, "VAULT_RELOAD", Value::Null);
    let polled = run(&list, "NOTIFY_POLL", json!({}));
    assert_eq!(polled["unread"], 2, "{polled}");
    assert_eq!(bell(&ext), "2");
    let notices = ui.notices();
    assert!(
        notices
            .iter()
            .any(|n| n.text.contains("alex mentioned you on Album")
                && n.title.as_deref() == Some("Mention")),
        "{notices:?}"
    );

    // Read one, then go to the other (no window: its href is said, not opened).
    let listed = run(&list, "NOTIFY_LIST", json!({ "unread_only": true }));
    assert_eq!(listed["notifications"].as_array().unwrap().len(), 2);
    let read = run(&list, "NOTIFY_READ", json!({ "id": "from-elsewhere" }));
    assert!(read["read"].is_string(), "{read}");
    let other = sent[0]["id"].as_str().unwrap();
    let went = run(&list, "NOTIFY_OPEN", json!({ "id": other }));
    assert!(went["notification"]["read"].is_string());
    assert_eq!(went["went"]["href"], "Tasks/Mix.md");
    assert_eq!(run(&list, "NOTIFY_UNREAD", json!({}))["unread"], 0);
    until("the bell to empty", || bell(&ext).is_empty());

    // Unread again, read all, clear.
    run(&list, "NOTIFY_READ", json!({ "id": other, "read": false }));
    assert_eq!(run(&list, "NOTIFY_READ_ALL", json!({}))["marked"], 1);
    assert_eq!(run(&list, "NOTIFY_CLEAR", json!({}))["left"], 0);
    // Someone else's list, by name.
    assert_eq!(
        run(&list, "NOTIFY_LIST", json!({ "who": "Alex" }))["unread"],
        1
    );

    // A toast another extension showed is kept for the session.
    run(
        &list,
        "NOTIFY_TOASTED",
        json!({ "from": "time", "text": "Timer stopped" }),
    );
    assert_eq!(
        run(&list, "NOTIFY_LIST", json!({}))["toasts"][0]["text"],
        "Timer stopped"
    );
}
