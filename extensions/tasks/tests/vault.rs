//! Tasks against a real vault, as the app hosts it: the app's list holds
//! the vault's and Bases' actions over a folder, and the extension
//! reaches them only through the host service. Linked in, then the same
//! value as a process (its development copy).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, now_ms};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_settings::Registry;
use architect_tasks::{ID, Tasks};
use futures::executor::block_on;
use serde_json::{Value, json};

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

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn status(ext: &Extensions) -> String {
    ext.host()
        .contributions()
        .status(now_ms())
        .into_iter()
        .find(|s| s.owner == ID && s.item.id == "due")
        .map(|s| s.item.text)
        .unwrap_or_default()
}

/// Adds, lists, finishes and relates tasks through `list` (the
/// extension's): every change is in the markdown.
fn exercise(dir: &Path, ext: &Extensions, list: &ActionList, app: &ActionList, origin: Origin) {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    until("its startup hook's status", || {
        status(ext).contains("today")
    });

    // Quick add: tokens become properties, the rest the title.
    let added = run(
        list,
        "TASKS_ADD",
        json!({ "text": "Call the venue today #gigs @phone !high" }),
    );
    assert_eq!(added["title"], "Call the venue");
    let path = added["path"].as_str().unwrap().to_owned();
    assert_eq!(path, "Tasks/Call the venue.md");
    let text = std::fs::read_to_string(dir.join(&path)).unwrap();
    for line in [
        "type: task",
        "status: open",
        "priority: high",
        "- gigs",
        "- '@phone'",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    // A second of the same name is a second note.
    let again = run(list, "TASKS_ADD", json!({ "text": "Call the venue" }));
    assert_eq!(again["path"], "Tasks/Call the venue 2.md");

    // A sub-task blocked by the first.
    let sub = run(
        list,
        "TASKS_CREATE",
        json!({
            "title": "Send the rider",
            "parent": "[[Tasks/Call the venue]]",
            "blocked_by": ["[[Tasks/Call the venue]]"],
            "assignees": ["Sam"],
        }),
    );
    let sub_path = sub["path"].as_str().unwrap().to_owned();
    let read = run(list, "TASKS_READ", json!({ "path": sub_path }));
    assert_eq!(read["relations"]["blocked"], true);
    assert_eq!(read["relations"]["parent"], path);

    // Lists: all, today, mine.
    let all = run(list, "TASKS_LIST", json!({}));
    assert_eq!(all["count"], 3);
    let today = run(list, "TASKS_LIST", json!({ "view": "today" }));
    assert_eq!(today["count"], 1, "only the one due today: {today}");
    until("the status item to count it", || {
        status(ext) == "1 task today"
    });

    // Done: the file says so, and its blocker no longer blocks.
    let done = run(
        list,
        "TASKS_SET_STATUS",
        json!({ "path": path, "status": "completed" }),
    );
    assert_eq!(
        done["status"], "done",
        "a synonym written as the status it means"
    );
    let text = std::fs::read_to_string(dir.join(&path)).unwrap();
    assert!(
        text.contains("status: done") && text.contains("completedDate:"),
        "{text}"
    );
    let read = run(list, "TASKS_READ", json!({ "path": sub_path }));
    assert_eq!(read["relations"]["blocked"], false);
    let parent = run(list, "TASKS_READ", json!({ "path": path }));
    assert_eq!(parent["relations"]["subtasks"], json!([sub_path]));

    // A repeating one, done, moves on and stays open.
    let habit = run(list, "TASKS_ADD", json!({ "text": "Stretch every day" }));
    let habit_path = habit["path"].as_str().unwrap().to_owned();
    let after = run(
        list,
        "TASKS_SET_STATUS",
        json!({ "path": habit_path, "status": "done" }),
    );
    assert_eq!(after["status"], "open");
    assert_eq!(after["complete_instances"].as_array().unwrap().len(), 1);

    // An update.
    let up = run(
        list,
        "TASKS_UPDATE",
        json!({ "path": sub_path, "priority": "low", "due": "2026-12-01" }),
    );
    assert_eq!(up["task"]["priority"], "low");
    assert!(
        std::fs::read_to_string(dir.join(&sub_path))
            .unwrap()
            .contains("due: 2026-12-01")
    );

    // The board's base: written once, a kanban view a column per status.
    let rows = run(
        app,
        "BASES_ROWS",
        json!({ "source": architect_tasks::board_base() }),
    );
    assert_eq!(rows["view"]["type"], "kanban", "{rows}");
}

#[test]
fn tasks_two_ways() {
    linked_in();
    as_a_process();
}

fn extensions(
    tmp: &Path,
    app: &Arc<ActionList>,
    runner: Option<ProcessRunner>,
) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.set_app_list((**app).clone());
    host.forward_actions_to(app);
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&Tasks)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (ext, list)
}

fn linked_in() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let app = app(&vault_dir);
    let (ext, list) = extensions(tmp.path(), &app, None);
    exercise(&vault_dir, &ext, &list, &app, Origin::Builtin);
    // Its kanban type for Bases, drawn by its code.
    let reg = ext.host().contributions();
    assert_eq!(reg.base_view("kanban").unwrap().owner, ID);
    assert!(ext.host().linked_view(ID, "base.kanban").is_some());
    // It provides `tasks`: a method is its action.
    assert_eq!(
        ext.host()
            .provider(architect_tasks::SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        ID
    );
    let _ = ext.disable(ID);
    assert!(ext.host().contributions().base_view("kanban").is_none());
    assert!(ext.host().contributions().of(ID).is_none());
}

fn as_a_process() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let dev = tmp.path().join("tasks");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-tasks"),
        dev.join("architect-tasks"),
    )
    .unwrap();
    let app = app(&vault_dir);
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let (ext, list) = extensions(tmp.path(), &app, Some(runner));
    let _ = ext.dev(&dev);
    exercise(&vault_dir, &ext, &list, &app, Origin::Dev);
    assert!(
        ext.host().linked_view(ID, "list").is_none(),
        "drawn remotely now"
    );
}
