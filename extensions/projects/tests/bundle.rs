//! Projects against a real vault, alone and with Tasks (#375): a project
//! made from a type its extension adds, its tasks on its page through the
//! `tasks` service, a task's project resolved through `projects`, the
//! types withdrawn when it is off, and the same value as a process.

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
use architect_projects::{ID, Projects};
use architect_settings::Registry;
use architect_tasks::Tasks;
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

/// A host whose app list is `app`, the extensions' own `EXT_*` on it as
/// the app forwards them.
fn host(
    tmp: &Path,
    app: &Arc<ActionList>,
    runner: Option<ProcessRunner>,
    tasks: bool,
) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.set_app_list((**app).clone());
    host.forward_actions_to(app);
    let mut ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&Projects)
        .unwrap();
    if tasks {
        ext = ext.with_linked(&Tasks).unwrap();
    }
    ext.install_contribution_actions().unwrap();
    // The app puts these on its own list, where the extensions' host
    // calls run (the lab: `FORWARDED`).
    for id in ["EXT_PROJECT_TYPES", "EXT_VIEW_OPEN"] {
        let info = list.info(id).unwrap();
        let from = list.clone();
        let id = id.to_owned();
        app.register_forwarded_with(
            &info,
            Arc::new(
                move |caller: &architect::action::ActionCaller, args: &[u8]| {
                    let (from, caller, id, args) =
                        (from.clone(), caller.clone(), id.clone(), args.to_vec());
                    let run: architect::action::ActionFuture =
                        Box::pin(
                            async move { from.run_json_async_with(&caller, &id, &args).await },
                        );
                    Ok(run)
                },
            ),
        )
        .unwrap();
    }
    let _ = ext.sync();
    (ext, list)
}

fn status(ext: &Extensions) -> String {
    ext.host()
        .contributions()
        .status(now_ms())
        .into_iter()
        .find(|s| s.owner == ID && s.item.id == "count")
        .map(|s| s.item.text)
        .unwrap_or_default()
}

/// Makes a software project, gives it a milestone, a member and tasks,
/// moves them on, and reads its page: every change in the markdown.
fn exercise(dir: &Path, ext: &Extensions, list: &ActionList, origin: Origin) {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    until("its startup hook's status", || {
        status(ext).contains("active project")
    });

    // Its two types are architect's registry's (EXT_PROJECT_TYPES).
    let types = run(list, "PROJECTS_TYPES", json!({}));
    let ids: Vec<&str> = types
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["software", "client"]);
    assert!(
        types[0]["template_text"]
            .as_str()
            .unwrap()
            .contains("{{title}}")
    );

    let made = run(
        list,
        "PROJECTS_CREATE",
        json!({ "title": "Site", "type": "software" }),
    );
    let page = made["project"]["path"].as_str().unwrap().to_owned();
    assert_eq!(page, "Projects/Site/project.md");
    let text = std::fs::read_to_string(dir.join(&page)).unwrap();
    for line in [
        "type: project",
        "projectType: software",
        "state: planning",
        "# Site",
        "## Decisions",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(
        dir.join("Projects/Site/Docs/Docs.md").is_file(),
        "its type's folders"
    );
    assert!(
        try_run(
            list,
            "PROJECTS_CREATE",
            json!({ "title": "X", "type": "nope" })
        )
        .unwrap_err()
        .contains("software, client")
    );

    let active = run(
        list,
        "PROJECTS_SET_STATE",
        json!({ "path": "Site", "state": "active" }),
    );
    assert_eq!(active["state"], "active");
    assert!(
        std::fs::read_to_string(dir.join(&page))
            .unwrap()
            .contains("state: active")
    );
    until("the status item to count it", || {
        status(ext) == "1 active project"
    });
    let _ = run(
        list,
        "PROJECTS_ADD_MEMBER",
        json!({ "path": page, "member": "[[People/Sam]]" }),
    );

    let beta = run(
        list,
        "PROJECTS_MILESTONE_CREATE",
        json!({ "project": page, "title": "Beta", "due": "2026-12-01" }),
    );
    assert_eq!(beta["path"], "Projects/Site/Milestones/Beta.md");
    let goal = run(
        list,
        "PROJECTS_GOAL_CREATE",
        json!({ "title": "Launch", "project": "Site", "kind": "quarter" }),
    );
    assert_eq!(goal["path"], "Projects/Site/Goals/Launch.md");
}

/// With Tasks: tasks land in the project, and its page shows them.
fn with_tasks(dir: &Path, list: &ActionList) {
    let page = "Projects/Site/project.md";
    // By name: Tasks asks Projects which project that is.
    let t = run(
        list,
        "TASKS_ADD",
        json!({ "text": "Draft copy [[Site]] !high" }),
    );
    assert_eq!(
        t["path"], "Projects/Site/Tasks/Draft copy.md",
        "in its project's folder"
    );
    assert_eq!(
        t["projects"],
        json!(["[[Projects/Site/project|Site]]"]),
        "a link that resolves"
    );
    let t2 = run(
        list,
        "TASKS_CREATE",
        json!({ "title": "Wire up deploys", "project": page, "milestone": "[[Projects/Site/Milestones/Beta|Beta]]" }),
    );
    let _ = run(
        list,
        "TASKS_SET_STATUS",
        json!({ "path": t2["path"], "status": "done" }),
    );
    let read = run(list, "PROJECTS_READ", json!({ "path": page }));
    assert_eq!(read["tasks"]["total"], 2, "{read}");
    assert_eq!(read["tasks"]["done"], 1);
    assert_eq!(read["roadmap"][0]["title"], "Beta");
    assert_eq!(
        read["roadmap"][0]["progress"],
        json!({ "total": 1, "done": 1 })
    );
    assert_eq!(read["type"]["name"], "Software");
    assert_eq!(read["project"]["members"], json!(["[[People/Sam]]"]));
    let notes: Vec<&str> = read["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["path"].as_str())
        .collect();
    assert!(notes.contains(&"Projects/Site/Docs/Docs.md"), "{notes:?}");
    assert!(
        !notes
            .iter()
            .any(|n| n.contains("/Tasks/") || n.ends_with("project.md")),
        "{notes:?}"
    );
    assert!(
        std::fs::read_to_string(dir.join(t2["path"].as_str().unwrap()))
            .unwrap()
            .contains("status: done")
    );
}

/// One after the other: an extension's handle is its process's, so two
/// hosts at once in one test process would share it.
#[test]
fn projects_two_ways() {
    with_tasks_and_alone();
    as_a_process();
}

fn with_tasks_and_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    let app = app(&dir);
    let (ext, list) = host(tmp.path(), &app, None, true);
    exercise(&dir, &ext, &list, Origin::Builtin);
    with_tasks(&dir, &list);

    // Tasks off: the page says so, and Projects keeps working.
    let _ = ext.disable(architect_tasks::ID);
    let read = run(&list, "PROJECTS_READ", json!({ "path": "Site" }));
    assert!(
        read["tasks"].is_null(),
        "no tasks service: {}",
        read["tasks"]
    );
    assert_eq!(read["roadmap"][0]["title"], "Beta");

    // Projects off: its types leave the registry; Tasks alone still files
    // a task under a project.md it is given by path.
    let _ = ext.enable(architect_tasks::ID);
    let _ = ext.disable(ID);
    assert_eq!(run(&list, "EXT_PROJECT_TYPES", json!({})), json!([]));
    assert!(
        ext.host()
            .contributions()
            .project_type("software")
            .is_none()
    );
    let t = run(
        &list,
        "TASKS_ADD",
        json!({ "text": "Alone", "project": "Projects/Site/project.md" }),
    );
    assert_eq!(t["path"], "Projects/Site/Tasks/Alone.md");
}

fn as_a_process() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    let dev = tmp.path().join("projects");
    std::fs::create_dir_all(dev.join("templates")).unwrap();
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    for f in [
        "extension.toml",
        "templates/software.md",
        "templates/client.md",
    ] {
        std::fs::copy(here.join(f), dev.join(f)).unwrap();
    }
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-projects"),
        dev.join("architect-projects"),
    )
    .unwrap();
    let app = app(&dir);
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let (ext, list) = host(tmp.path(), &app, Some(runner), false);
    let _ = ext.dev(&dev);
    exercise(&dir, &ext, &list, Origin::Dev);
    assert!(
        ext.host().linked_view(ID, "page").is_none(),
        "drawn remotely now"
    );
}
