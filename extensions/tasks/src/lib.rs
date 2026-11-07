//! Tasks (`extensions/tasks/extension.toml`): tasks as vault notes, a
//! first-party extension and a model of how one is built (#368, #375).
//!
//! One [`Tasks`] value runs it two ways: the app links it in
//! (`Extensions::with_linked`), and its program (`src/main.rs`) serves the
//! same value as a process (`Guest::extension`), so `EXT_CLONE {"id":
//! "tasks"}` gives a development copy that overrides the linked one.
//!
//! - **Data:** a task is a note (`type: task`, `TaskNotes`' frontmatter;
//!   [`model`]). It reads and writes notes only through the host service:
//!   `BASES_ROWS` to find every task in one query, `VAULT_CREATE` and
//!   `VAULT_SET_PROPERTY` to write ([`vault`]).
//! - **Views:** the task list, Today and My tasks (one component, three
//!   filters), and a kanban view type for Bases (`[[base_views]] id =
//!   "kanban"`) that `TASKS_BOARD` opens over `Tasks/Board.base`.
//! - **Navigator:** while its views show, the sidebar's navigator shows
//!   the status groups and the selected one's tasks ([`nav`],
//!   `[[navigator_providers]]`, #356).
//! - **Service:** it provides `architect.tasks` (each method its `TASKS_<METHOD>`
//!   action) and uses `architect.projects` when it is there: a task's project is a
//!   link to a `project.md`, resolved by the Projects extension. With the
//!   Time Tracker (`architect.time` `totals`), a task carries its time and
//!   its row a timer.

pub mod model;
pub mod nav;
pub mod queue;
pub mod quick;
pub mod vault;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::linked::{Builder, Extension};
use chrono::{Local, NaiveDate};
use dioxus::prelude::*;
use serde_json::{Map, Value, json};

use crate::model::{Group, Status, Task, keys, link_names, link_to};

/// Its id: its actions are `TASKS_*`.
pub const ID: &str = "tasks";

/// The service it provides (`[provides]`): method `m` is `TASKS_<M>`.
pub const SERVICE: &str = "architect.tasks";

/// The Projects extension's service, which it uses when it is there.
const PROJECTS: &str = "architect.projects";

/// The Time Tracker's service, which it uses when it is there.
const TIME: &str = "architect.time";

/// Notify, when it is installed: a task's new assignees are told.
const NOTIFY: &str = "architect.notify";

/// Its status item.
const DUE: &str = "due";

/// Where tasks go when no project says otherwise (`tasks.folder`).
pub const FOLDER: &str = "Tasks";

/// The base `TASKS_BOARD` writes and opens, in [`FOLDER`].
pub const BOARD: &str = "Board.base";

/// The base that finds every task: `type: task`, or `TaskNotes`' tag.
#[must_use]
pub fn tasks_base() -> String {
    format!(
        "filters:\n  or:\n    - type == \"task\"\n    - file.hasTag(\"task\")\nviews:\n  - type: table\n    name: Tasks\n    order:\n{}",
        keys::ALL.iter().fold(String::new(), |mut out, k| {
            let _ = writeln!(out, "      - {k}");
            out
        })
    )
}

/// The board's base: every task, a column per status.
#[must_use]
pub fn board_base() -> String {
    "filters:\n  or:\n    - type == \"task\"\n    - file.hasTag(\"task\")\nviews:\n  - type: kanban\n    name: Board\n    order:\n      - file.name\n      - status\n      - priority\n      - due\n      - projects\n      - assignees\n    columnBy: status\n".to_owned()
}

/// What it knows of the vault's tasks, as last read.
#[derive(Debug, Default)]
struct Cache {
    tasks: Vec<Task>,
    /// Each task's time by path (the Time Tracker's `totals`); `None`
    /// when it is not there.
    time: Option<Map<String, Value>>,
    error: Option<String>,
    read: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    tasks: Vec::new(),
    time: None,
    error: None,
    read: false,
});

/// Moves on with every read and write, for the views.
static REVISION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Reads every task again; its status item says what is due.
async fn refresh() -> Result<Vec<Task>, String> {
    let read = vault::rows(&tasks_base()).await.map(|rows| {
        rows.into_iter()
            .map(|r| Task::from_values(&r.path, &r.name, &r.values))
            .collect::<Vec<_>>()
    });
    let today = ymd(today());
    {
        let mut c = cache();
        c.read = true;
        match &read {
            Ok(tasks) => {
                c.tasks.clone_from(tasks);
                c.error = None;
            }
            Err(e) => c.error = Some(e.clone()),
        }
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    if let (Some(h), Ok(tasks)) = (vault::handle(), &read) {
        let due = tasks.iter().filter(|t| model::is_today(t, &today)).count();
        h.status(
            DUE,
            match due {
                0 => "Nothing due today".to_owned(),
                1 => "1 task today".to_owned(),
                n => format!("{n} tasks today"),
            },
        );
    }
    read
}

/// Reads each task's time again, from the Time Tracker when it is there.
async fn refresh_time() {
    let time = match vault::service(TIME, "totals", Value::Null).await {
        Some(Ok(v)) => Some(
            v.get("notes")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        ),
        _ => None,
    };
    let changed = {
        let mut c = cache();
        let changed = c.time != time;
        c.time = time;
        changed
    };
    if changed {
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Task `path`'s time, as last read: `null` without the Time Tracker,
/// zeroes for a task with none.
fn time_of(path: &str) -> Value {
    cache().time.as_ref().map_or(Value::Null, |t| {
        t.get(path)
            .cloned()
            .unwrap_or_else(|| json!({ "seconds": 0, "today": 0, "week": 0, "running": false }))
    })
}

/// Which tasks a list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Every task, open first.
    All,
    /// Open ones.
    Open,
    /// In progress, or due or planned by today.
    Today,
    /// Assigned to `tasks.me`.
    Mine,
}

impl Filter {
    fn parse(s: Option<&str>) -> Result<Self, String> {
        Ok(match s.unwrap_or("all") {
            "all" | "list" | "" => Self::All,
            "open" => Self::Open,
            "today" => Self::Today,
            "mine" | "my" => Self::Mine,
            other => return Err(format!("view {other:?}: all, open, today or mine")),
        })
    }
}

/// `tasks` as a list shows them: filtered, open work in its order, then
/// what is finished, newest first.
#[must_use]
pub fn select(tasks: &[Task], filter: Filter, me: &str, today: &str) -> Vec<Task> {
    let mut out: Vec<Task> = tasks
        .iter()
        .filter(|t| match filter {
            Filter::All => true,
            Filter::Open => t.is_open(),
            Filter::Today => model::is_today(t, today),
            Filter::Mine => t.is_open() && t.assigned_to(me),
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        b.is_open().cmp(&a.is_open()).then_with(|| {
            if a.is_open() {
                model::order(a, b)
            } else {
                b.completed_date.cmp(&a.completed_date)
            }
        })
    });
    out
}

fn task_json(t: &Task) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    if let Some(m) = v.as_object_mut() {
        m.insert("open".to_owned(), json!(t.is_open()));
        m.insert("group".to_owned(), json!(t.group()));
        let time = time_of(&t.path);
        if !time.is_null() {
            m.insert("time".to_owned(), time);
        }
    }
    v
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn arg_list(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).map(model::list_of)
}

/// The project `project` names (a `project.md`'s path, a link or a
/// name): its `project.md` path and title, through the Projects
/// extension's `architect.projects` `resolve` when it is loaded.
async fn resolve_project(project: &str) -> Option<(String, String)> {
    if let Some(Ok(found)) = vault::service(PROJECTS, "resolve", json!({ "link": project })).await {
        let path = found.get("path").and_then(Value::as_str)?.to_owned();
        let title = found
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Some((path, title));
    }
    // Alone: a path to a project.md is one as it is.
    let target = model::link_target(project);
    let path = if std::path::Path::new(target)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    {
        target.to_owned()
    } else {
        format!("{target}.md")
    };
    path.ends_with("/project.md").then(|| {
        let title = path
            .trim_end_matches("/project.md")
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        (path, title)
    })
}

/// Writes `task` as a new note: in its project's `Tasks` folder when it
/// has one that resolves, else in [`FOLDER`]. The task written.
async fn write_new(mut task: Task, body: &str) -> Result<Task, String> {
    let now = Local::now().to_rfc3339();
    task.date_created.get_or_insert_with(|| now.clone());
    task.date_modified = Some(now);
    let mut folder = FOLDER.to_owned();
    if let Some(first) = task.projects.first().cloned()
        && let Some((path, title)) = resolve_project(&first).await
    {
        // A link the vault and Obsidian resolve: by path, shown as its
        // title.
        if let Some(link) = task.projects.first_mut() {
            *link = link_to(&path, Some(&title));
        }
        if let Some((dir, _)) = path.rsplit_once('/') {
            folder = format!("{dir}/{FOLDER}");
        }
    }
    let path = format!("{folder}/{}.md", model::file_name(&task.title));
    task.path = vault::create(&path, &task.frontmatter(), body).await?;
    let _ = refresh().await;
    notify_assigned(&task, &[]).await;
    Ok(task)
}

/// An assignee as a person's name: a link's note name (`[[People/Alex]]`
/// is Alex), else the text.
fn person(assignee: &str) -> String {
    let target = model::link_target(assignee);
    target
        .rsplit('/')
        .next()
        .unwrap_or(target)
        .trim()
        .to_owned()
}

/// Tells each of `task`'s assignees not in `before` through Notify (when
/// it is installed): "<you> assigned you <task>", going to the task.
/// Whoever assigns is not told of their own assignment. The names told.
async fn notify_assigned(task: &Task, before: &[String]) -> Vec<String> {
    let old: Vec<String> = before.iter().map(|a| person(a).to_lowercase()).collect();
    let new: Vec<String> = task
        .assignees
        .iter()
        .map(|a| person(a))
        .filter(|p| !p.is_empty() && !old.contains(&p.to_lowercase()))
        .collect();
    if new.is_empty() {
        return Vec::new();
    }
    let actor = match vault::service(NOTIFY, "me", json!({})).await {
        Some(Ok(v)) => v
            .get("me")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        // No Notify: nobody to tell.
        None => return Vec::new(),
        Some(Err(_)) => String::new(),
    };
    let title = format!(
        "{} assigned you {}",
        if actor.is_empty() { "Someone" } else { &actor },
        task.title
    );
    let mut told = Vec::new();
    for who in new {
        let post = json!({
            "to": who, "kind": "task-assigned", "title": title, "actor": actor,
            "body": task.due.as_ref().map(|d| format!("Due {d}")).unwrap_or_default(),
            "source": { "service": SERVICE, "entity": task.path, "href": task.path },
        });
        if let Some(Ok(v)) = vault::service(NOTIFY, "post", post).await
            && v.get("sent")
                .and_then(Value::as_array)
                .is_some_and(|s| !s.is_empty())
        {
            told.push(who);
        }
    }
    told
}

async fn find(path: &str) -> Result<Task, String> {
    let tasks = refresh().await?;
    tasks
        .into_iter()
        .find(|t| t.path == path || link_names(path, &t.path))
        .ok_or_else(|| format!("no task at {path}"))
}

/// `TASKS_ADD {text, project?}`: quick add.
async fn add(args: Value) -> Result<Value, String> {
    let text = arg(&args, "text").ok_or("give `text`: the task, as typed")?;
    let mut task = quick::parse(text, today());
    if let Some(p) = arg(&args, "project") {
        task.projects.insert(0, p.to_owned());
    }
    write_new(task, "").await.map(|t| task_json(&t))
}

/// `TASKS_CAPTURE {title, body, properties, source, inbox?}`: the `task`
/// capture target (`[[capture_targets]]`): a capture from architect's
/// composer or inbox made into a task, its body the task's details.
async fn capture(args: Value) -> Result<Value, String> {
    let capture: architect_capture::Capture =
        serde_json::from_value(args).map_err(|e| format!("not a capture: {e}"))?;
    let task = quick::from_capture(&capture);
    write_new(task, &capture.body).await.map(|t| task_json(&t))
}

/// `TASKS_CREATE {title, …}`.
async fn create(args: Value) -> Result<Value, String> {
    let title = arg(&args, "title").ok_or("give `title`")?;
    let text = |k: &str| arg(&args, k).map(str::to_owned);
    let task = Task {
        title: title.to_owned(),
        status: text("status").unwrap_or_else(|| Status::Open.as_str().to_owned()),
        priority: text("priority").unwrap_or_else(|| "normal".to_owned()),
        due: text("due"),
        scheduled: text("scheduled"),
        tags: arg_list(&args, "tags").unwrap_or_default(),
        contexts: arg_list(&args, "contexts").unwrap_or_default(),
        projects: text("project").into_iter().collect(),
        milestone: text("milestone"),
        parent: text("parent"),
        blocked_by: arg_list(&args, "blocked_by").unwrap_or_default(),
        assignees: arg_list(&args, "assignees").unwrap_or_default(),
        recurrence: text("recurrence"),
        recurrence_anchor: text("recurrence_anchor"),
        time_estimate: args
            .get("time_estimate")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        ..Task::default()
    };
    let body = arg(&args, "details").unwrap_or_default().to_owned();
    write_new(task, &body).await.map(|t| task_json(&t))
}

/// `TASKS_SET_STATUS {path, status}`: a repeating task done moves on
/// ([`model::complete`]).
pub async fn set_status(path: &str, status: &str) -> Result<Value, String> {
    let task = find(path).await?;
    let status = Status::parse(status).map_or(status, |s| s.as_str());
    for (key, value) in model::complete(&task, status, today()) {
        vault::set(&task.path, key, &value).await?;
    }
    vault::set(
        &task.path,
        keys::DATE_MODIFIED,
        &json!(Local::now().to_rfc3339()),
    )
    .await?;
    let after = find(&task.path).await?;
    Ok(task_json(&after))
}

/// `TASKS_UPDATE {path, …}`: the properties given, each set (`null`
/// removes it).
async fn update(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let task = find(path).await?;
    let fields: &[(&str, &str, bool)] = &[
        ("title", keys::TITLE, false),
        ("priority", keys::PRIORITY, false),
        ("due", keys::DUE, false),
        ("scheduled", keys::SCHEDULED, false),
        ("milestone", keys::MILESTONE, false),
        ("parent", keys::PARENT, false),
        ("recurrence", keys::RECURRENCE, false),
        ("assignees", keys::ASSIGNEES, true),
        ("blocked_by", keys::BLOCKED_BY, true),
        ("relates", keys::RELATES, true),
        ("tags", keys::TAGS, true),
        ("contexts", keys::CONTEXTS, true),
        ("projects", keys::PROJECTS, true),
    ];
    let mut changed = Vec::new();
    for (arg_key, key, many) in fields {
        let Some(v) = args.get(*arg_key) else {
            continue;
        };
        let value = match v {
            Value::Null => Value::Null,
            v if *many => json!(model::list_of(v)),
            v => v.clone(),
        };
        vault::set(&task.path, key, &value).await?;
        changed.push(*key);
    }
    if let Some(project) = arg(&args, "project") {
        let link = match resolve_project(project).await {
            Some((p, title)) => link_to(&p, Some(&title)),
            None => project.to_owned(),
        };
        vault::set(&task.path, keys::PROJECTS, &json!([link])).await?;
        changed.push(keys::PROJECTS);
    }
    if let Some(s) = arg(&args, "status") {
        let _ = set_status(&task.path, s).await?;
        changed.push(keys::STATUS);
    }
    if changed.is_empty() {
        return Err("nothing to change: give a property to set".to_owned());
    }
    let after = find(&task.path).await?;
    let notified = if changed.contains(&keys::ASSIGNEES) {
        notify_assigned(&after, &task.assignees).await
    } else {
        Vec::new()
    };
    Ok(json!({ "task": task_json(&after), "changed": changed, "notified": notified }))
}

async fn me() -> String {
    model::scalar(&vault::setting("tasks.me").await).unwrap_or_default()
}

/// `TASKS_LIST {view?, project?}`.
async fn list(args: Value) -> Result<Value, String> {
    let filter = Filter::parse(arg(&args, "view"))?;
    let tasks = refresh().await?;
    refresh_time().await;
    let tasks: Vec<Task> = match arg(&args, "project") {
        Some(p) => {
            let path = resolve_project(p)
                .await
                .map_or_else(|| p.to_owned(), |(path, _)| path);
            tasks.into_iter().filter(|t| t.in_project(&path)).collect()
        }
        None => tasks,
    };
    let shown = select(&tasks, filter, &me().await, &ymd(today()));
    Ok(json!({
        "count": shown.len(),
        "tasks": shown.iter().map(task_json).collect::<Vec<_>>(),
    }))
}

/// `TASKS_FOR_PROJECT {project}`: its tasks, and how many are done.
async fn for_project(args: Value) -> Result<Value, String> {
    let project = arg(&args, "project").ok_or("give `project`: its project.md's path")?;
    let tasks = refresh().await?;
    refresh_time().await;
    let mine: Vec<Task> = select(
        &tasks
            .into_iter()
            .filter(|t| t.in_project(project))
            .collect::<Vec<_>>(),
        Filter::All,
        "",
        &ymd(today()),
    );
    let done = mine.iter().filter(|t| !t.is_open()).count();
    let mut counts: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    for t in &mine {
        if let Some(m) = &t.milestone {
            let (total, done) = counts.entry(model::link_target(m).to_owned()).or_default();
            *total = total.saturating_add(1);
            if !t.is_open() {
                *done = done.saturating_add(1);
            }
        }
    }
    let by_milestone: Map<String, Value> = counts
        .into_iter()
        .map(|(k, (total, done))| (k, json!({ "total": total, "done": done })))
        .collect();
    Ok(json!({
        "project": project,
        "total": mine.len(),
        "done": done,
        "by_milestone": by_milestone,
        "tasks": mine.iter().map(task_json).collect::<Vec<_>>(),
    }))
}

/// `TASKS_READ {path}`: one task and its relations.
async fn read(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let tasks = refresh().await?;
    refresh_time().await;
    let task = tasks
        .iter()
        .find(|t| t.path == path || link_names(path, &t.path))
        .ok_or_else(|| format!("no task at {path}"))?;
    Ok(json!({
        "task": task_json(task),
        "relations": model::relations_of(&task.path, &tasks),
    }))
}

/// `TASKS_OPEN {view?}`.
async fn open(args: Value) -> Result<Value, String> {
    let view = match arg(&args, "view").unwrap_or("list") {
        "list" | "all" => "list",
        "today" => "today",
        "mine" | "my" => "mine",
        "board" => return board().await,
        other => return Err(format!("view {other:?}: list, today, mine or board")),
    };
    vault::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": view })).await
}

/// `TASKS_BOARD`: `Tasks/Board.base` (written the first time), opened.
async fn board() -> Result<Value, String> {
    let path = format!("{FOLDER}/{BOARD}");
    let exists = vault::run("BASES_READ", json!({ "path": path }))
        .await
        .is_ok();
    if !exists {
        vault::run(
            "VAULT_CREATE",
            json!({ "path": path, "body": board_base() }),
        )
        .await?;
        // VAULT_CREATE indexes what it writes as a note: read the folder
        // again so Bases knows the new file is a base.
        vault::run("VAULT_RELOAD", Value::Null).await?;
    }
    let opened = vault::run("BASES_OPEN", json!({ "path": path })).await?;
    Ok(json!({ "path": path, "written": !exists, "opened": opened.get("opened") }))
}

/// The statuses, and the group each falls in.
fn statuses() -> Value {
    json!(
        Group::ALL
            .iter()
            .map(|g| json!({ "group": g, "label": g.label(), "status": g.status().as_str() }))
            .collect::<Vec<_>>()
    )
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tasks;

impl Extension for Tasks {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[(
            "tours/tasks-tour.styx",
            include_str!("../tours/tasks-tour.styx"),
        )]
    }

    fn build(&self, ext: &mut Builder) {
        vault::connect(ext.handle());
        // Its navigator provider: status groups, then their tasks (`nav`).
        ext.navigator(nav::ACTIONS, nav::TasksNav);
        ext.navigator(queue::ACTIONS, queue::QueueNav);
        ext.view("list", ListView)
            .view("today", TodayView)
            .view("mine", MineView)
            .base_view("kanban", Kanban)
            .action("TASKS_LIST", list)
            .action("TASKS_READ", read)
            .action("TASKS_ADD", add)
            .action("TASKS_CAPTURE", capture)
            .action("TASKS_CREATE", create)
            .action("TASKS_UPDATE", update)
            .action("TASKS_SET_STATUS", |args| async move {
                let path = arg(&args, "path").ok_or("give `path`")?.to_owned();
                let status = arg(&args, "status").ok_or("give `status`")?.to_owned();
                set_status(&path, &status).await
            })
            .action("TASKS_FOR_PROJECT", for_project)
            .action("TASKS_STATUSES", |_| async { Ok(statuses()) })
            .action("TASKS_OPEN", open)
            .action("TASKS_BOARD", |_| board())
            .action("TASKS_STARTED", |_| async {
                let _ = refresh().await;
                Ok(json!("ready"))
            });
    }
}

/// Re-renders the calling view when what it knows of the tasks changes,
/// and reads them again every few seconds while it shows (an edit made
/// in the note, or by another device, comes in).
fn use_tasks() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let _ = refresh().await;
        refresh_time().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(12) {
                let _ = refresh().await;
                refresh_time().await;
            }
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

#[component]
fn ListView() -> Element {
    rsx! { TaskList { filter: "all" } }
}

#[component]
fn TodayView() -> Element {
    rsx! { TaskList { filter: "today" } }
}

#[component]
fn MineView() -> Element {
    rsx! { TaskList { filter: "mine" } }
}

const MUTED: &str = "color: var(--muted-foreground);";

/// A list of tasks: a "New task" field (quick add), then a section per
/// status group, each task a row whose box finishes it.
#[component]
fn TaskList(filter: String) -> Element {
    let _ = use_tasks();
    let me = use_resource(me);
    let mut draft = use_signal(String::new);
    let mut problem = use_signal(|| None::<String>);
    let which = Filter::parse(Some(&filter)).unwrap_or(Filter::All);
    let today_s = ymd(today());
    let (tasks, error, read) = {
        let c = cache();
        (c.tasks.clone(), c.error.clone(), c.read)
    };
    let me_now = me.read().clone().unwrap_or_default();
    let shown = select(&tasks, which, &me_now, &today_s);
    let title = match which {
        Filter::All | Filter::Open => "Tasks",
        Filter::Today => "Today",
        Filter::Mine => "My tasks",
    };
    let mut groups: Vec<(Group, Vec<Task>)> = Vec::new();
    for t in shown {
        match groups.iter_mut().find(|(g, _)| *g == t.group()) {
            Some((_, v)) => v.push(t),
            None => groups.push((t.group(), vec![t])),
        }
    }
    groups.sort_by_key(|(g, _)| *g);
    let empty = match which {
        Filter::Today => "Nothing due today.",
        Filter::Mine if me_now.is_empty() => {
            "Set your name in Settings → Tasks to see the tasks assigned to you."
        }
        Filter::Mine => "Nothing assigned to you.",
        _ => "No tasks yet: type one above.",
    };
    let mut submit = move || {
        let text = draft.peek().trim().to_owned();
        if text.is_empty() {
            return;
        }
        draft.set(String::new());
        spawn(async move {
            let args = json!({ "text": text });
            if let Err(e) = add(args).await {
                problem.set(Some(e));
            } else {
                problem.set(None);
            }
        });
    };
    rsx! {
        div { class: "tasks-list", "data-filter": "{filter}",
            style: "display: flex; flex-direction: column; gap: 14px; padding: 20px 24px; height: 100%; \
                    box-sizing: border-box; overflow-y: auto; color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: baseline; gap: 10px;",
                span { style: "font-size: 20px; font-weight: 700;", "{title}" }
                if which == Filter::Mine && !me_now.is_empty() {
                    span { style: MUTED, "assigned to {me_now}" }
                }
            }
            input {
                class: "tasks-new",
                r#type: "text",
                "aria-label": "New task",
                placeholder: "New task — e.g. Call the venue tomorrow [[Album]] !high #gigs",
                value: "{draft}",
                style: "height: 36px; box-sizing: border-box; padding: 0 12px; border-radius: 8px; \
                        border: 1px solid var(--input); background: var(--background); color: var(--foreground); \
                        font-size: 13px;",
                oninput: move |e: FormEvent| draft.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() == Key::Named(NamedKey::Enter) {
                        submit();
                    }
                },
            }
            if let Some(p) = problem() {
                div { style: "color: var(--destructive);", "{p}" }
            }
            if let Some(e) = error {
                div { style: "color: var(--destructive);", "The vault could not be read: {e}" }
            } else if !read {
                div { style: MUTED, "Reading the vault…" }
            } else if groups.is_empty() {
                div { style: MUTED, "{empty}" }
            }
            for (group, list) in groups {
                div { key: "{group:?}", style: "display: flex; flex-direction: column; gap: 2px;",
                    div { style: "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; \
                                  padding: 6px 0; {MUTED}",
                        "{group.label()} · {list.len()}"
                    }
                    for t in list {
                        TaskRow { key: "{t.path}", time: time_of(&t.path), task: t, today: today_s.clone() }
                    }
                }
            }
        }
    }
}

/// One task's row: its box (done or open again), title, due date,
/// priority and project; with the Time Tracker (`time` not `null`), its
/// time and a button that starts or stops a timer on it.
#[component]
pub fn TaskRow(task: Task, today: String, time: Value) -> Element {
    let open = task.is_open();
    let path = task.path.clone();
    let overdue = open
        && task
            .due
            .as_deref()
            .is_some_and(|d| model::day(d) < today.as_str());
    let due = task.due.as_deref().map(model::day).map(str::to_owned);
    let project = task.projects.first().map(|p| {
        let t = model::link_target(p);
        p.split_once('|').map_or_else(
            || {
                t.trim_end_matches("/project")
                    .rsplit('/')
                    .next()
                    .unwrap_or(t)
                    .to_owned()
            },
            |(_, a)| a.trim_end_matches("]]").to_owned(),
        )
    });
    let priority = model::Priority::rank(&task.priority);
    let flag = match priority {
        model::Priority::Critical => Some(("!!", "var(--destructive)")),
        model::Priority::High => Some(("!", "var(--warning, var(--destructive))")),
        _ => None,
    };
    let toggle = move |_| {
        let path = path.clone();
        let to = if open { "done" } else { "open" };
        spawn(async move {
            let _ = set_status(&path, to).await;
        });
    };
    let open_note = {
        let path = task.path.clone();
        move |_| {
            let path = path.clone();
            spawn(async move {
                if let Some(h) = vault::handle() {
                    let _ = h
                        .host(architect_extensions::hostcall::HostRequest::Open { path })
                        .await;
                }
            });
        }
    };
    let doing = task.group() == Group::Doing;
    let timed = !time.is_null();
    let ticking = time
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let spent = time.get("seconds").and_then(Value::as_i64).unwrap_or(0);
    let spent_text = {
        let m = spent.max(0) / 60;
        format!("{}:{:02}", m / 60, m % 60)
    };
    let timer = {
        let path = task.path.clone();
        move |_| {
            let path = path.clone();
            spawn(async move {
                let (method, args) = if ticking {
                    ("stop", json!({}))
                } else {
                    ("start", json!({ "task": path }))
                };
                let _ = vault::service(TIME, method, args).await;
                refresh_time().await;
            });
        }
    };
    let (timer_bg, timer_fg) = if ticking {
        ("var(--destructive)", "var(--destructive-foreground, white)")
    } else {
        ("var(--secondary)", "var(--secondary-foreground)")
    };
    let fill = if open {
        "transparent"
    } else {
        "var(--primary)"
    };
    let strike = if open {
        ""
    } else {
        "text-decoration: line-through; color: var(--muted-foreground);"
    };
    let due_color = if overdue {
        "var(--destructive)"
    } else {
        "var(--muted-foreground)"
    };
    rsx! {
        div { class: "tasks-row", "data-path": "{task.path}", "data-status": "{task.status}",
            style: "display: flex; align-items: center; gap: 10px; padding: 6px 8px; border-radius: 6px;",
            button {
                class: "tasks-check",
                "aria-label": if open { "Mark done" } else { "Mark open" },
                style: "width: 18px; height: 18px; flex-shrink: 0; padding: 0; cursor: pointer; border-radius: 5px; \
                        border: 1.5px solid var(--border); font-size: 11px; line-height: 14px; \
                        background: {fill}; \
                        color: var(--primary-foreground);",
                onclick: toggle,
                if !open { "✓" }
            }
            if let Some((mark, color)) = flag {
                span { style: "color: {color}; font-weight: 700;", "{mark}" }
            }
            span {
                style: "flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; \
                        {strike}",
                onclick: open_note,
                "{task.title}"
            }
            if doing {
                span { style: "font-size: 11px; padding: 1px 6px; border-radius: 999px; background: var(--secondary); \
                               color: var(--secondary-foreground);", "doing" }
            }
            if let Some(p) = project {
                span { style: "font-size: 12px; {MUTED}", "{p}" }
            }
            if let Some(d) = due {
                span { style: "font-size: 12px; font-variant-numeric: tabular-nums; \
                               color: {due_color};",
                    "{d}"
                }
            }
            if timed {
                if spent > 0 {
                    span { class: "tasks-time", style: "font-size: 12px; font-variant-numeric: tabular-nums; {MUTED}", "{spent_text}" }
                }
                button {
                    class: "tasks-timer",
                    "aria-label": if ticking { "Stop the timer" } else { "Start a timer" },
                    style: "width: 22px; height: 22px; flex-shrink: 0; padding: 0; border: none; border-radius: 5px; cursor: pointer; \
                            font-size: 10px; background: {timer_bg}; color: {timer_fg};",
                    onclick: timer,
                    if ticking { "■" } else { "▶" }
                }
            }
        }
    }
}

/// Its Bases view type (`[[base_views]] id = "kanban"`): a column per
/// value of the view's `columnBy` (`status` by default), a card per note;
/// a card's arrows move it to the next column, writing that property (a
/// task's status through `TASKS_SET_STATUS`, so a repeating one moves
/// on).
#[component]
fn Kanban() -> Element {
    let Some(input) = architect_extensions::linked::use_base_view() else {
        return rsx! {};
    };
    let by = input
        .view
        .get("options")
        .and_then(|o| o.get("columnBy"))
        .or_else(|| input.view.get("columnBy"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(keys::STATUS)
        .to_owned();
    let rows = vault::rows_of(&json!({ "rows": input.rows }));
    let by_status = by == keys::STATUS;
    let value_of = |r: &vault::Row| {
        r.values
            .get(&by)
            .and_then(model::scalar)
            .unwrap_or_default()
    };
    // A status board's columns are the groups, in order, all shown; any
    // other property's are its values as they come.
    let mut columns: Vec<(String, String, Vec<vault::Row>)> = if by_status {
        Group::ALL
            .iter()
            .filter(|g| **g != Group::Cancelled)
            .map(|g| {
                (
                    g.status().as_str().to_owned(),
                    g.label().to_owned(),
                    Vec::new(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    for r in rows {
        let v = value_of(&r);
        let key = if by_status {
            Status::group_of(&v).status().as_str().to_owned()
        } else {
            v.clone()
        };
        if let Some((_, _, list)) = columns.iter_mut().find(|(k, _, _)| *k == key) {
            list.push(r);
        } else {
            let label = if key.is_empty() {
                "(none)".to_owned()
            } else {
                key.clone()
            };
            columns.push((key, label, vec![r]));
        }
    }
    let keys_in_order: Vec<String> = columns.iter().map(|(k, _, _)| k.clone()).collect();
    rsx! {
        div { class: "tasks-kanban", "data-by": "{by}",
            style: "display: flex; gap: 12px; align-items: flex-start; padding: 4px; overflow-x: auto; \
                    color: var(--foreground); font-size: 13px;",
            for (i, (key, label, cards)) in columns.into_iter().enumerate() {
                div { key: "{key}", class: "tasks-kanban-column", "data-column": "{key}",
                    style: "display: flex; flex-direction: column; gap: 6px; width: 220px; flex-shrink: 0; padding: 8px; \
                            border-radius: 10px; background: var(--muted);",
                    div { style: "display: flex; justify-content: space-between; font-weight: 700; padding: 2px 4px;",
                        span { "{label}" }
                        span { style: MUTED, "{cards.len()}" }
                    }
                    for card in cards {
                        KanbanCard {
                            key: "{card.path}",
                            card: card,
                            by: by.clone(),
                            prev: i.checked_sub(1).and_then(|p| keys_in_order.get(p)).cloned(),
                            next: keys_in_order.get(i + 1).cloned(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KanbanCard(card: vault::Row, by: String, prev: Option<String>, next: Option<String>) -> Element {
    let title = card
        .values
        .get(keys::TITLE)
        .and_then(model::scalar)
        .unwrap_or_else(|| card.name.clone());
    let due = card.values.get(keys::DUE).and_then(model::scalar);
    let priority = card.values.get(keys::PRIORITY).and_then(model::scalar);
    let mover = |to: Option<String>| {
        let (path, by) = (card.path.clone(), by.clone());
        move |_| {
            let (path, by) = (path.clone(), by.clone());
            if let Some(to) = to.clone() {
                spawn(async move {
                    let _ = if by == keys::STATUS {
                        set_status(&path, &to).await.map(|_| ())
                    } else {
                        vault::set(&path, &by, &json!(to)).await
                    };
                    // Bases redraws on a VAULT_* write it sees; the
                    // status through TASKS_SET_STATUS is one.
                });
            }
        }
    };
    let arrow = "width: 22px; height: 22px; padding: 0; border: none; border-radius: 5px; cursor: pointer; \
                 background: var(--secondary); color: var(--secondary-foreground); font-size: 12px;";
    rsx! {
        div { class: "tasks-kanban-card", "data-path": "{card.path}",
            style: "display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border-radius: 8px; \
                    border: 1px solid var(--border); background: var(--card); color: var(--card-foreground);",
            div { style: "font-weight: 600;", "{title}" }
            div { style: "display: flex; align-items: center; gap: 6px; font-size: 12px; {MUTED}",
                if let Some(d) = due {
                    span { "{model::day(&d)}" }
                }
                if let Some(p) = priority.filter(|p| p != "normal") {
                    span { "{p}" }
                }
                span { style: "flex: 1;" }
                if prev.is_some() {
                    button { style: "{arrow}", "aria-label": "Move back", onclick: mover(prev), "‹" }
                }
                if next.is_some() {
                    button { style: "{arrow}", "aria-label": "Move on", onclick: mover(next), "›" }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn its_bases_parse() {
        for source in [tasks_base(), board_base()] {
            let v: serde_yaml::Value = serde_yaml::from_str(&source).unwrap();
            assert!(v.get("views").is_some(), "{source}");
        }
    }

    #[test]
    fn lists_filter_and_order() {
        let t = |path: &str, status: &str, due: Option<&str>, who: &[&str]| Task {
            path: path.into(),
            title: path.into(),
            status: status.into(),
            due: due.map(Into::into),
            assignees: who.iter().map(|s| (*s).to_owned()).collect(),
            ..Task::default()
        };
        let all = [
            t("a", "done", Some("2026-11-01"), &[]),
            t("b", "open", Some("2026-11-05"), &["[[People/Cody Wright]]"]),
            t("c", "open", Some("2026-11-20"), &["Sam"]),
            t("d", "in-progress", None, &[]),
        ];
        let today = "2026-11-05";
        let paths = |v: Vec<Task>| v.into_iter().map(|t| t.path).collect::<Vec<_>>();
        assert_eq!(
            paths(select(&all, Filter::All, "", today)),
            ["d", "b", "c", "a"]
        );
        assert_eq!(paths(select(&all, Filter::Today, "", today)), ["d", "b"]);
        assert_eq!(
            paths(select(&all, Filter::Mine, "cody wright", today)),
            ["b"]
        );
        assert!(
            select(&all, Filter::Mine, "", today).is_empty(),
            "no name, nothing mine"
        );
    }
}
