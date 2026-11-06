//! The Time Tracker against a real vault, as the app hosts it: the app's
//! list holds the vault's, Bases' and capture's actions over a folder,
//! and the extension reaches them only through the host service. Alone,
//! linked in and then as a process (its development copy); then with
//! Tasks and Projects, the rest of the Project Management bundle (#375):
//! a task found through `tasks`, a project by its name through
//! `projects`, and the project's page and the task list showing time
//! through `time`.

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
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, now_ms};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_settings::Registry;
use architect_time_tracker::{ID, TimeTracker};
use futures::executor::block_on;
use serde_json::{Value, json};

const PROJECT: &str = "Projects/Album/project.md";
const TASK: &str = "Projects/Album/Tasks/Mix the album.md";

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

fn try_run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
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
        .find(|s| s.owner == ID && s.item.id == "timer")
        .map(|s| s.item.text)
        .unwrap_or_default()
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

/// A project that bills at 60.00 USD an hour, and a task in it, written
/// as a person (or Obsidian) would.
fn seed(dir: &Path, app: &ActionList) {
    std::fs::create_dir_all(dir.join("Projects/Album/Tasks")).unwrap();
    std::fs::write(
        dir.join(PROJECT),
        "---\ntype: project\ntitle: Album\nstate: active\nbillableDefault: true\ndefaultRateCents: 6000\ncurrency: USD\n---\n# Album\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(TASK),
        "---\ntype: task\ntitle: Mix the album\nstatus: open\npriority: normal\nprojects:\n  - \"[[Projects/Album/project|Album]]\"\n---\n",
    )
    .unwrap();
    run(app, "VAULT_RELOAD", Value::Null);
}

/// Starts, switches, stops and logs time through `list` (the
/// extension's), and reads it back as totals, a project's time, a report
/// and a timesheet: every entry in the markdown.
fn exercise(dir: &Path, ext: &Extensions, list: &ActionList, app: &ActionList, origin: Origin) {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    until("its startup hook's status", || status(ext) == "No timer");

    // A timer on the task: an open entry in its frontmatter, billable as
    // its project bills.
    let started = run(
        list,
        "TIME_START",
        json!({ "task": TASK, "description": "Rough mix" }),
    );
    assert_eq!(started["started"]["path"], TASK, "{started}");
    assert_eq!(started["stopped"], json!([]));
    let text = read(dir, TASK);
    for line in [
        "timeEntries:",
        "startTime:",
        "description: Rough mix",
        "billable: true",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(!text.contains("endTime"), "it runs\n{text}");
    assert_eq!(run(list, "TIME_ACTIVE", json!({}))["path"], TASK);
    until("the status item to say it runs", || {
        status(ext).starts_with("Mix the album · since ")
    });

    // One at a time: a timer on the project stops the task's.
    let switched = run(list, "TIME_START", json!({ "project": PROJECT }));
    assert_eq!(switched["stopped"][0]["path"], TASK, "{switched}");
    assert!(read(dir, TASK).contains("endTime:"));
    let stopped = run(list, "TIME_STOP", json!({}));
    assert_eq!(stopped["stopped"]["path"], PROJECT);
    assert!(read(dir, PROJECT).contains("endTime:"));
    assert!(
        try_run(list, "TIME_STOP", json!({}))
            .unwrap_err()
            .contains("no timer")
    );
    assert!(run(list, "TIME_ACTIVE", json!({})).is_null());

    // By hand: 1h30m on the task, and half an hour on no note at all.
    let logged = run(
        list,
        "TIME_LOG",
        json!({ "task": TASK, "duration": "1h30m", "description": "Vocals" }),
    );
    assert_eq!(logged["logged"]["seconds"], 5400, "{logged}");
    let text = read(dir, TASK);
    assert_eq!(text.matches("startTime:").count(), 2, "{text}");
    assert!(text.contains("description: Vocals"), "{text}");
    let unfiled = run(
        list,
        "TIME_LOG",
        json!({ "date": "2026-01-05", "minutes": 30, "billable": false }),
    );
    assert_eq!(unfiled["path"], architect_time_tracker::UNFILED);
    let text = read(dir, architect_time_tracker::UNFILED);
    assert!(
        text.contains("startTime: 2026-01-05T") && text.contains("billable: false"),
        "{text}"
    );
    assert!(
        try_run(list, "TIME_LOG", json!({ "task": TASK }))
            .unwrap_err()
            .contains("say how long")
    );

    // Totals: per note, and the project's (its own and its task's).
    let totals = run(list, "TIME_TOTALS", json!({}));
    assert!(
        totals["notes"][TASK]["seconds"].as_i64().unwrap() >= 5400,
        "{totals}"
    );
    let album = run(list, "TIME_FOR_PROJECT", json!({ "project": PROJECT }));
    assert!(album["seconds"].as_i64().unwrap() >= 5400, "{album}");
    assert!(album["by_task"][TASK].is_object(), "{album}");
    assert_eq!(album["currency"], "USD");
    assert!(
        album["cents"].as_i64().unwrap() >= 9000,
        "1h30m at 60.00: {album}"
    );

    // The report over everything: the project first, no project last.
    let all = run(list, "TIME_REPORT", json!({ "range": "all" }));
    assert_eq!(all["from"], "2026-01-05", "{all}");
    assert_eq!(all["projects"][0]["path"], PROJECT, "{all}");
    let none = all["projects"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(none["title"], "No project");
    assert_eq!(none["seconds"], 1800);
    assert!(
        try_run(list, "TIME_REPORT", json!({ "range": "fortnight" }))
            .unwrap_err()
            .contains("week")
    );

    // The timesheet's base: written once, a timesheet view over the notes.
    let rows = run(
        app,
        "BASES_ROWS",
        json!({ "source": architect_time_tracker::timesheet_base() }),
    );
    assert_eq!(rows["view"]["type"], "timesheet", "{rows}");
    let sheet = run(list, "TIME_TIMESHEET", json!({}));
    assert_eq!(sheet["written"], true, "{sheet}");
    assert!(dir.join("Time/Timesheet.base").is_file());
    assert_eq!(run(list, "TIME_TIMESHEET", json!({}))["written"], false);

    // A target of architect's capture (#352): "Log time" from a line.
    let targets = run(app, "CAPTURE_TARGETS", json!({}));
    let time = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "time")
        .unwrap_or_else(|| panic!("no time target in {targets}"));
    assert_eq!(time["from"], ID);
    let captured = run(
        app,
        "CAPTURE_ADD",
        json!({
            "text": "Call with the label for:45m [[Projects/Album/project|Album]]",
            "target": "time",
            "source": "window",
        }),
    );
    assert_eq!(captured["path"], PROJECT, "{captured}");
    let text = read(dir, PROJECT);
    assert!(text.contains("description: Call with the label"), "{text}");
}

#[test]
fn time_two_ways() {
    linked_in();
    as_a_process();
}

#[test]
fn time_with_the_bundle() {
    with_tasks_and_projects();
}

fn extensions(
    tmp: &Path,
    app: &Arc<ActionList>,
    runner: Option<ProcessRunner>,
    bundle: bool,
) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.set_app_list((**app).clone());
    host.forward_actions_to(app);
    let mut ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&TimeTracker)
        .unwrap();
    if bundle {
        ext = ext
            .with_linked(&architect_projects::Projects)
            .unwrap()
            .with_linked(&architect_tasks::Tasks)
            .unwrap();
    }
    ext.install_contribution_actions().unwrap();
    // As the app has it: the registries on its list, and capture over it.
    for id in ["EXT_CAPTURE_TARGETS", "EXT_PROJECT_TYPES", "EXT_VIEW_OPEN"] {
        forward(app, &list, id);
    }
    architect_capture::actions::register(app, architect_capture::actions::Config::default(), None)
        .unwrap();
    let _ = ext.sync();
    (ext, list)
}

/// Puts `id` of `from` on `to`, run on `from` as whoever ran it (the
/// app's `forward`, `apps/ui/src/extensions.rs`).
fn forward(to: &ActionList, from: &ActionList, id: &str) {
    let info = from.info(id).unwrap();
    let (list, id) = (from.clone(), id.to_owned());
    to.register_forwarded_with(
        &info,
        Arc::new(
            move |caller: &architect::action::ActionCaller, args: &[u8]| {
                let (list, caller, id, args) =
                    (list.clone(), caller.clone(), id.clone(), args.to_vec());
                let run: architect::action::ActionFuture =
                    Box::pin(async move { list.run_json_async_with(&caller, &id, &args).await });
                Ok(run)
            },
        ),
    )
    .unwrap();
}

fn linked_in() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let app = app(&vault_dir);
    seed(&vault_dir, &app);
    let (ext, list) = extensions(tmp.path(), &app, None, false);
    exercise(&vault_dir, &ext, &list, &app, Origin::Builtin);
    // Its timesheet type for Bases, drawn by its code.
    let reg = ext.host().contributions();
    assert_eq!(reg.base_view("timesheet").unwrap().owner, ID);
    assert!(ext.host().linked_view(ID, "base.timesheet").is_some());
    // It provides `time`: a method is its action.
    assert_eq!(
        ext.host()
            .provider(architect_time_tracker::SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        ID
    );
    let _ = ext.disable(ID);
    assert!(ext.host().contributions().base_view("timesheet").is_none());
    assert!(ext.host().contributions().of(ID).is_none());
}

fn as_a_process() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let dev = tmp.path().join("time");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    // Its tour: the manifest names it, so its package has it.
    std::fs::create_dir_all(dev.join("tours")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tours/time-tour.styx"),
        dev.join("tours/time-tour.styx"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-time-tracker"),
        dev.join("architect-time-tracker"),
    )
    .unwrap();
    let app = app(&vault_dir);
    seed(&vault_dir, &app);
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let (ext, list) = extensions(tmp.path(), &app, Some(runner), false);
    let _ = ext.dev(&dev);
    exercise(&vault_dir, &ext, &list, &app, Origin::Dev);
    assert!(
        ext.host().linked_view(ID, "report").is_none(),
        "drawn remotely now"
    );
}

/// With Tasks and Projects: names resolve through their services, and
/// their views' data carries time through `time`.
fn with_tasks_and_projects() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let app = app(&vault_dir);
    seed(&vault_dir, &app);
    let (ext, list) = extensions(tmp.path(), &app, None, true);
    until("its startup hook's status", || status(&ext) == "No timer");

    // A project by its name (Projects' `resolve`), a task by a bare link
    // (Tasks' `read`).
    let on_project = run(
        &list,
        "TIME_LOG",
        json!({ "project": "Album", "minutes": 20 }),
    );
    assert_eq!(on_project["path"], PROJECT, "{on_project}");
    let on_task = run(&list, "TIME_START", json!({ "task": "[[Mix the album]]" }));
    assert_eq!(on_task["started"]["path"], TASK, "{on_task}");
    assert_eq!(on_task["kind"], "task");

    // The project's page has its time, its task's included, and the
    // task's own line; the running timer is the task's.
    let page = run(&list, "PROJECTS_READ", json!({ "path": PROJECT }));
    let time = &page["time"];
    assert!(time["seconds"].as_i64().unwrap() >= 1200, "{page}");
    assert_eq!(time["running"]["path"], TASK, "{time}");
    assert!(
        time["by_task"][TASK]["running"].as_bool().unwrap(),
        "{time}"
    );

    // The task list knows its time.
    let tasks = run(&list, "TASKS_LIST", json!({}));
    let mix = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == TASK)
        .unwrap()
        .clone();
    assert_eq!(mix["time"]["running"], true, "{mix}");

    let _ = run(&list, "TIME_STOP", json!({}));
    // Without the Time Tracker, the page has no time and says nothing of it.
    let _ = ext.disable(ID);
    let page = run(&list, "PROJECTS_READ", json!({ "path": PROJECT }));
    assert!(page["time"].is_null(), "{page}");
}
