//! Projects (`extensions/projects/extension.toml`): a page and actions on
//! architect's built-in `project` kind, project types, goals and
//! milestones; a first-party extension and a model of how one is built
//! (#368, #375).
//!
//! One [`Projects`] value runs it two ways: the app links it in, and its
//! program (`src/main.rs`) serves the same value as a process, so
//! `EXT_CLONE {"id":"projects"}` gives a development copy.
//!
//! - **Data:** a project is a folder whose `project.md` is the project
//!   (#344); goals and milestones are notes of their own kinds
//!   ([`model`]). It reads and writes them only through the host service
//!   ([`vault`]).
//! - **Project types:** it contributes `software` and `client`
//!   (`[[project_types]]`), and creates a project of any type a loaded
//!   extension adds (`EXT_PROJECT_TYPES`).
//! - **Views:** the projects list, and a project's page: its state,
//!   members, roadmap (milestones with their tasks' progress), goals,
//!   tasks and notes.
//! - **Service:** it provides `architect.projects` and uses `architect.tasks` when it is
//!   there: the page's tasks are the Tasks extension's
//!   (`architect.tasks` `for_project`), and a task's project is resolved here
//!   (`architect.projects` `resolve`).

pub mod model;
pub mod vault;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::linked::{Builder, Extension};
use dioxus::prelude::*;
use serde_json::{Map, Value, json};

use crate::model::{Goal, Milestone, Project, State, TYPE_KEY, link_label, link_to};

/// Its id: its actions are `PROJECTS_*`.
pub const ID: &str = "projects";

/// The service it provides (`[provides]`): method `m` is `PROJECTS_<M>`.
pub const SERVICE: &str = "architect.projects";

/// The Tasks extension's service, which it uses when it is there.
const TASKS: &str = "architect.tasks";

/// Where new projects go (`projects.folder`).
pub const FOLDER: &str = "Projects";

/// Its status item.
const COUNT: &str = "count";

/// The base that finds projects, goals and milestones in one query.
#[must_use]
pub fn notes_base() -> String {
    let order = [
        "type",
        "title",
        "state",
        "status",
        TYPE_KEY,
        "start",
        "due",
        "up",
        "area",
        "members",
        "icon",
        "tags",
        "kind",
        "parent",
        "parentId",
        "targetDate",
        "project",
        "goal",
        "dueDate",
    ];
    format!(
        "filters:\n  or:\n    - file.name == \"project\"\n    - type == \"project\"\n    - type == \"goal\"\n    - type == \"milestone\"\nviews:\n  - type: table\n    name: Projects\n    order:\n{}",
        order
            .iter()
            .map(|k| format!("      - {k}\n"))
            .collect::<String>()
    )
}

/// The vault's projects, goals and milestones, as last read.
#[derive(Debug, Default, Clone)]
pub struct Book {
    pub projects: Vec<Project>,
    pub goals: Vec<Goal>,
    pub milestones: Vec<Milestone>,
}

impl Book {
    /// Sorts rows by their `type` (a `project.md` is a project whatever
    /// it says).
    #[must_use]
    pub fn of(rows: &[vault::Row]) -> Self {
        let mut book = Self::default();
        for r in rows {
            let ty = r
                .values
                .get("type")
                .and_then(model::scalar)
                .unwrap_or_default();
            if model::is_project_page(&r.path) || ty == "project" {
                if model::is_project_page(&r.path) {
                    book.projects.push(Project::from_values(&r.path, &r.values));
                }
            } else if ty == "goal" {
                book.goals
                    .push(Goal::from_values(&r.path, &r.name, &r.values));
            } else if ty == "milestone" {
                book.milestones
                    .push(Milestone::from_values(&r.path, &r.name, &r.values));
            }
        }
        book.projects.sort_by(|a, b| {
            a.state()
                .cmp(&b.state())
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        book.milestones.sort_by(|a, b| {
            a.due_date
                .as_deref()
                .unwrap_or("9999")
                .cmp(b.due_date.as_deref().unwrap_or("9999"))
        });
        book
    }
}

#[derive(Debug, Default)]
struct Cache {
    book: Book,
    error: Option<String>,
    read: bool,
    /// The project the page shows.
    current: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    book: Book {
        projects: Vec::new(),
        goals: Vec::new(),
        milestones: Vec::new(),
    },
    error: None,
    read: false,
    current: None,
});

static REVISION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn changed() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

/// Reads the projects, goals and milestones again.
async fn refresh() -> Result<Book, String> {
    let read = vault::rows(&notes_base()).await.map(|rows| Book::of(&rows));
    {
        let mut c = cache();
        c.read = true;
        match &read {
            Ok(book) => {
                c.book = book.clone();
                c.error = None;
            }
            Err(e) => c.error = Some(e.clone()),
        }
    }
    changed();
    if let (Some(h), Ok(book)) = (vault::handle(), &read) {
        let active = book
            .projects
            .iter()
            .filter(|p| p.state() == State::Active)
            .count();
        h.status(
            COUNT,
            format!(
                "{active} active project{}",
                if active == 1 { "" } else { "s" }
            ),
        );
    }
    read
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

async fn project_at(link: &str) -> Result<Project, String> {
    let book = refresh().await?;
    model::resolve(link, &book.projects)
        .cloned()
        .ok_or_else(|| format!("no project {link:?}"))
}

fn project_json(p: &Project) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    if let Some(m) = v.as_object_mut() {
        m.insert("state_group".to_owned(), json!(p.state()));
        m.insert("link".to_owned(), json!(p.link()));
    }
    v
}

/// `PROJECTS_LIST {state?, type?}`.
async fn list(args: Value) -> Result<Value, String> {
    let book = refresh().await?;
    let state = arg(&args, "state").map(State::parse);
    let ty = arg(&args, "type");
    let shown: Vec<Value> = book
        .projects
        .iter()
        .filter(|p| state.is_none_or(|s| p.state() == s))
        .filter(|p| ty.is_none_or(|t| p.project_type.as_deref() == Some(t)))
        .map(project_json)
        .collect();
    Ok(json!({ "count": shown.len(), "projects": shown }))
}

/// `PROJECTS_RESOLVE {link}`: the project a link, path or name names.
async fn resolve(args: Value) -> Result<Value, String> {
    let link = arg(&args, "link").ok_or("give `link`")?;
    project_at(link).await.map(|p| project_json(&p))
}

/// The project types loaded extensions add.
async fn types() -> Result<Value, String> {
    vault::run("EXT_PROJECT_TYPES", Value::Null).await
}

/// `PROJECTS_CREATE {title, type?, folder?, state?, members?, due?}`:
/// `<folder>/<title>/project.md`, its type's properties (their defaults),
/// its template as its body, and a folder note in each of its folders.
async fn create(args: Value) -> Result<Value, String> {
    let title = arg(&args, "title").ok_or("give `title`")?.to_owned();
    let ty = match arg(&args, "type") {
        Some(id) => {
            let t = vault::run("EXT_PROJECT_TYPES", json!({ "id": id })).await?;
            if t.is_null() {
                let known = types().await.unwrap_or_default();
                let ids: Vec<&str> = known
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.get("id").and_then(Value::as_str))
                    .collect();
                return Err(format!(
                    "no project type {id:?} (there are: {})",
                    ids.join(", ")
                ));
            }
            Some(t)
        }
        None => None,
    };
    let parent = arg(&args, "folder").unwrap_or(FOLDER).trim_end_matches('/');
    let folder = if parent.is_empty() {
        model::file_name(&title)
    } else {
        format!("{parent}/{}", model::file_name(&title))
    };
    let mut props = Map::new();
    props.insert("type".to_owned(), json!("project"));
    props.insert("title".to_owned(), json!(title));
    props.insert(
        "state".to_owned(),
        json!(State::parse(arg(&args, "state").unwrap_or("planning")).as_str()),
    );
    if let Some(due) = arg(&args, "due") {
        props.insert("due".to_owned(), json!(due));
    }
    if let Some(m) = args.get("members") {
        props.insert("members".to_owned(), json!(model::list_of(m)));
    }
    let mut body = format!("# {title}\n");
    let mut folders = Vec::new();
    if let Some(t) = &ty {
        let id = t.get("id").and_then(Value::as_str).unwrap_or_default();
        props.insert(TYPE_KEY.to_owned(), json!(id));
        for p in t
            .get("properties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = p.get("key").and_then(Value::as_str) else {
                continue;
            };
            let value = p
                .get("default")
                .and_then(Value::as_str)
                .and_then(|d| serde_json::from_str(d).ok())
                .unwrap_or(Value::Null);
            props.entry(key.to_owned()).or_insert(value);
        }
        if let Some(text) = t.get("template_text").and_then(Value::as_str) {
            body = model::fill(text, &title);
        }
        folders = t.get("folders").map(model::list_of).unwrap_or_default();
    }
    let path = format!("{folder}/{}", model::PROJECT_PAGE);
    vault::run(
        "VAULT_CREATE",
        json!({ "path": path, "body": body, "properties": Value::Object(props).to_string() }),
    )
    .await?;
    for sub in &folders {
        let name = model::file_name(sub);
        let _ = vault::create(
            &format!("{folder}/{name}/{name}.md"),
            &Map::new(),
            &format!("# {name}\n"),
        )
        .await;
    }
    let p = project_at(&path).await?;
    Ok(json!({ "project": project_json(&p), "folders": folders }))
}

/// `PROJECTS_SET_STATE {path, state}`.
async fn set_state(args: Value) -> Result<Value, String> {
    let link = arg(&args, "path").ok_or("give `path`")?;
    let state = State::parse(arg(&args, "state").ok_or("give `state`")?);
    let p = project_at(link).await?;
    vault::set(&p.path, "state", &json!(state.as_str())).await?;
    // The older key would say otherwise: it goes.
    vault::set(&p.path, "status", &Value::Null).await?;
    project_at(&p.path).await.map(|p| project_json(&p))
}

/// `PROJECTS_SET_MEMBERS {path, members}`; `PROJECTS_ADD_MEMBER {path,
/// member}`.
async fn set_members(path: &str, members: Vec<String>) -> Result<Value, String> {
    let p = project_at(path).await?;
    vault::set(&p.path, "members", &json!(members)).await?;
    project_at(&p.path).await.map(|p| project_json(&p))
}

async fn add_member(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let who = arg(&args, "member").ok_or("give `member`: a name or a link")?;
    let p = project_at(path).await?;
    let mut members = p.members.clone();
    if !members.iter().any(|m| m.eq_ignore_ascii_case(who)) {
        members.push(who.to_owned());
    }
    set_members(&p.path, members).await
}

/// Where a goal or milestone of `project` goes: `<its folder>/<sub>`, or
/// the vault's `<sub>`.
async fn place(project: Option<&str>, sub: &str) -> Result<(String, Option<Project>), String> {
    match project {
        Some(link) => {
            let p = project_at(link).await?;
            Ok((format!("{}/{sub}", p.folder), Some(p)))
        }
        None => Ok((sub.to_owned(), None)),
    }
}

/// `PROJECTS_GOAL_CREATE {title, project?, kind?, status?, target?,
/// parent?}`.
async fn goal_create(args: Value) -> Result<Value, String> {
    let title = arg(&args, "title").ok_or("give `title`")?;
    let (folder, project) = place(arg(&args, "project"), "Goals").await?;
    let mut props = Map::new();
    props.insert("type".to_owned(), json!("goal"));
    props.insert("title".to_owned(), json!(title));
    props.insert(
        "kind".to_owned(),
        json!(model::goal_kind(arg(&args, "kind").unwrap_or("other"))),
    );
    props.insert(
        "status".to_owned(),
        json!(model::goal_status(arg(&args, "status").unwrap_or("active"))),
    );
    if let Some(p) = &project {
        props.insert("project".to_owned(), json!(p.link()));
    }
    if let Some(t) = arg(&args, "target") {
        props.insert("targetDate".to_owned(), json!(t));
    }
    if let Some(p) = arg(&args, "parent") {
        props.insert("parent".to_owned(), json!(p));
    }
    let path = vault::create(
        &format!("{folder}/{}.md", model::file_name(title)),
        &props,
        "",
    )
    .await?;
    let _ = refresh().await;
    Ok(json!({ "path": path, "title": title }))
}

/// `PROJECTS_MILESTONE_CREATE {project, title, due?, goal?}`.
async fn milestone_create(args: Value) -> Result<Value, String> {
    let title = arg(&args, "title").ok_or("give `title`")?;
    let project = arg(&args, "project").ok_or("give `project`: a milestone is a project's")?;
    let (folder, p) = place(Some(project), "Milestones").await?;
    let mut props = Map::new();
    props.insert("type".to_owned(), json!("milestone"));
    props.insert("title".to_owned(), json!(title));
    props.insert("status".to_owned(), json!("open"));
    if let Some(p) = &p {
        props.insert("project".to_owned(), json!(p.link()));
    }
    if let Some(d) = arg(&args, "due") {
        props.insert("dueDate".to_owned(), json!(d));
    }
    if let Some(g) = arg(&args, "goal") {
        props.insert("goal".to_owned(), json!(g));
    }
    let path = vault::create(
        &format!("{folder}/{}.md", model::file_name(title)),
        &props,
        "",
    )
    .await?;
    let _ = refresh().await;
    Ok(json!({ "path": path, "title": title, "link": link_to(&path, Some(title)) }))
}

/// `PROJECTS_MILESTONE_SET_STATUS {path, status}`.
async fn milestone_status(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let status = model::milestone_status(arg(&args, "status").ok_or("give `status`")?);
    vault::set(path, "status", &json!(status)).await?;
    let _ = refresh().await;
    Ok(json!({ "path": path, "status": status }))
}

/// A project's tasks, from the Tasks extension when it is loaded:
/// `null` when it is not.
async fn tasks_of(project: &Project) -> Value {
    match vault::service(TASKS, "for_project", json!({ "project": project.path })).await {
        Some(Ok(v)) => v,
        Some(Err(e)) => json!({ "error": e }),
        None => Value::Null,
    }
}

/// A project's milestones, each with its tasks' progress.
fn roadmap(book: &Book, p: &Project, tasks: &Value) -> Vec<Value> {
    book.milestones
        .iter()
        .filter(|m| model::belongs(&m.path, m.project.as_deref(), p))
        .map(|m| {
            let key = m.path.strip_suffix(".md").unwrap_or(&m.path);
            let progress = tasks
                .get("by_milestone")
                .and_then(|b| {
                    b.as_object()?
                        .iter()
                        .find(|(k, _)| {
                            k.as_str() == key || key.rsplit('/').next() == Some(k.as_str())
                        })
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or_else(|| json!({ "total": 0, "done": 0 }));
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.insert("progress".to_owned(), progress);
                o.insert("link".to_owned(), json!(link_to(&m.path, Some(&m.title))));
            }
            v
        })
        .collect()
}

/// `PROJECTS_READ {path}`: the project, its type, roadmap, goals, tasks
/// (when Tasks is there) and notes.
async fn read(args: Value) -> Result<Value, String> {
    let link = arg(&args, "path").ok_or("give `path`")?;
    let p = project_at(link).await?;
    let book = cache().book.clone();
    let tasks = tasks_of(&p).await;
    let goals: Vec<&Goal> = book
        .goals
        .iter()
        .filter(|g| model::belongs(&g.path, g.project.as_deref(), &p))
        .collect();
    let ty = match &p.project_type {
        Some(id) => vault::run("EXT_PROJECT_TYPES", json!({ "id": id }))
            .await
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    let mut known: Vec<String> = vec![p.path.clone()];
    known.extend(book.milestones.iter().map(|m| m.path.clone()));
    known.extend(book.goals.iter().map(|g| g.path.clone()));
    known.extend(
        tasks
            .get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.get("path").and_then(Value::as_str).map(str::to_owned)),
    );
    let notes: Vec<Value> = vault::run("VAULT_LIST", json!({ "folder": p.folder }))
        .await
        .ok()
        .and_then(|v| {
            let list = v.get("notes").cloned().unwrap_or(v);
            list.as_array().cloned()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|n| {
            n.get("path").and_then(Value::as_str).is_some_and(|path| {
                path.starts_with(&format!("{}/", p.folder)) && !known.iter().any(|k| k == path)
            })
        })
        .collect();
    Ok(json!({
        "project": project_json(&p),
        "type": ty,
        "roadmap": roadmap(&book, &p, &tasks),
        "goals": goals,
        "tasks": tasks,
        "notes": notes,
    }))
}

/// `PROJECTS_GOALS {project?}` / `PROJECTS_MILESTONES {project}`.
async fn goals(args: Value) -> Result<Value, String> {
    let book = refresh().await?;
    let p = match arg(&args, "project") {
        Some(l) => Some(project_at(l).await?),
        None => None,
    };
    let goals: Vec<&Goal> = book
        .goals
        .iter()
        .filter(|g| {
            p.as_ref()
                .is_none_or(|p| model::belongs(&g.path, g.project.as_deref(), p))
        })
        .collect();
    Ok(json!({ "goals": goals }))
}

async fn milestones(args: Value) -> Result<Value, String> {
    let link = arg(&args, "project").ok_or("give `project`")?;
    let p = project_at(link).await?;
    let book = cache().book.clone();
    let tasks = tasks_of(&p).await;
    Ok(json!({ "milestones": roadmap(&book, &p, &tasks) }))
}

/// `PROJECTS_OPEN {path?}`: a project's page, or the projects list.
async fn open(args: Value) -> Result<Value, String> {
    match arg(&args, "path") {
        Some(link) => {
            let p = project_at(link).await?;
            cache().current = Some(p.path.clone());
            changed();
            vault::run(
                "EXT_VIEW_OPEN",
                json!({ "extension": ID, "view": "page", "state": p.path }),
            )
            .await
        }
        None => vault::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": "list" })).await,
    }
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Projects;

impl Extension for Projects {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[
            (
                "templates/software.md",
                include_str!("../templates/software.md"),
            ),
            (
                "templates/client.md",
                include_str!("../templates/client.md"),
            ),
        ]
    }

    fn build(&self, ext: &mut Builder) {
        vault::connect(ext.handle());
        ext.view("list", ListView)
            .view("page", PageView)
            .action("PROJECTS_LIST", list)
            .action("PROJECTS_READ", read)
            .action("PROJECTS_CREATE", create)
            .action("PROJECTS_SET_STATE", set_state)
            .action("PROJECTS_ADD_MEMBER", add_member)
            .action("PROJECTS_SET_MEMBERS", |args| async move {
                let path = arg(&args, "path").ok_or("give `path`")?.to_owned();
                let members = args.get("members").map(model::list_of).unwrap_or_default();
                set_members(&path, members).await
            })
            .action("PROJECTS_RESOLVE", resolve)
            .action("PROJECTS_TYPES", |_| types())
            .action("PROJECTS_GOAL_CREATE", goal_create)
            .action("PROJECTS_GOALS", goals)
            .action("PROJECTS_MILESTONE_CREATE", milestone_create)
            .action("PROJECTS_MILESTONES", milestones)
            .action("PROJECTS_MILESTONE_SET_STATUS", milestone_status)
            .action("PROJECTS_OPEN", open)
            .action("PROJECTS_STARTED", |_| async {
                let _ = refresh().await;
                Ok(json!("ready"))
            });
    }
}

/// Re-renders when what it knows changes; reads again every few seconds
/// while it shows.
fn use_projects() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let _ = refresh().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks % 12 == 0 {
                let _ = refresh().await;
            }
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; \
                       color: var(--muted-foreground); padding: 4px 0;";
const FIELD: &str = "height: 32px; box-sizing: border-box; padding: 0 10px; border-radius: 8px; \
                     border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 13px;";

fn chip(active: bool) -> String {
    format!(
        "padding: 3px 10px; border-radius: 999px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {};",
        if active {
            "var(--primary)"
        } else {
            "transparent"
        },
        if active {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

/// The projects, a section per state, and a field to start one.
#[component]
fn ListView() -> Element {
    let _ = use_projects();
    let mut draft = use_signal(String::new);
    let mut ty = use_signal(String::new);
    let mut problem = use_signal(|| None::<String>);
    let kinds = use_resource(types);
    let (book, error, read) = {
        let c = cache();
        (c.book.clone(), c.error.clone(), c.read)
    };
    let kinds: Vec<(String, String)> = kinds
        .read()
        .clone()
        .and_then(Result::ok)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            Some((
                t.get("id")?.as_str()?.to_owned(),
                t.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ))
        })
        .collect();
    let mut submit = move || {
        let title = draft.peek().trim().to_owned();
        if title.is_empty() {
            return;
        }
        let kind = ty.peek().clone();
        draft.set(String::new());
        spawn(async move {
            let mut args = json!({ "title": title });
            if !kind.is_empty() {
                args["type"] = json!(kind);
            }
            match create(args).await {
                Ok(made) => {
                    problem.set(None);
                    if let Some(path) = made["project"]["path"].as_str() {
                        let _ = open(json!({ "path": path })).await;
                    }
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    rsx! {
        div { class: "projects-list",
            style: "display: flex; flex-direction: column; gap: 14px; padding: 20px 24px; height: 100%; box-sizing: border-box; \
                    overflow-y: auto; color: var(--foreground); font-size: 13px;",
            div { style: "font-size: 20px; font-weight: 700;", "Projects" }
            div { style: "display: flex; gap: 8px; align-items: center; flex-wrap: wrap;",
                input {
                    class: "projects-new", r#type: "text", "aria-label": "New project",
                    placeholder: "New project", value: "{draft}",
                    style: "{FIELD} flex: 1; min-width: 180px;",
                    oninput: move |e: FormEvent| draft.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { submit() },
                }
                button { style: chip(ty().is_empty()), onclick: move |_| ty.set(String::new()), "Plain" }
                for (id, name) in kinds {
                    button { key: "{id}", style: chip(ty() == id), onclick: move |_| ty.set(id.clone()), "{name}" }
                }
            }
            if let Some(p) = problem() {
                div { style: "color: var(--destructive);", "{p}" }
            }
            if let Some(e) = error {
                div { style: "color: var(--destructive);", "The vault could not be read: {e}" }
            } else if !read {
                div { style: MUTED, "Reading the vault…" }
            } else if book.projects.is_empty() {
                div { style: MUTED, "No projects yet: name one above." }
            }
            for state in State::ALL {
                if book.projects.iter().any(|p| p.state() == state) {
                    div { key: "{state:?}", style: "display: flex; flex-direction: column; gap: 2px;",
                        div { style: HEADING, "{state.label()}" }
                        for p in book.projects.iter().filter(|p| p.state() == state).cloned() {
                            ProjectRow { key: "{p.path}", project: p }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ProjectRow(project: Project) -> Element {
    let path = project.path.clone();
    rsx! {
        div { class: "projects-row", "data-path": "{project.path}",
            style: "display: flex; align-items: center; gap: 10px; padding: 7px 8px; border-radius: 6px; cursor: pointer;",
            onclick: move |_| {
                let path = path.clone();
                spawn(async move { let _ = open(json!({ "path": path })).await; });
            },
            span { style: "flex: 1; font-weight: 600;", "{project.title}" }
            if let Some(t) = &project.project_type {
                span { style: "font-size: 12px; {MUTED}", "{t}" }
            }
            if !project.members.is_empty() {
                span { style: "font-size: 12px; {MUTED}", "{project.members.len()} 👤" }
            }
            if let Some(d) = &project.due {
                span { style: "font-size: 12px; font-variant-numeric: tabular-nums; {MUTED}", "{d}" }
            }
        }
    }
}

/// A project's page.
#[component]
fn PageView() -> Element {
    let revision = use_projects();
    let current = cache().current.clone();
    let mut page = use_signal(|| None::<Result<Value, String>>);
    // Read again whenever what it knows moves on (a write, a tick).
    use_effect(use_reactive!(|revision, current| {
        let _ = revision;
        if let Some(path) = current.clone() {
            spawn(async move {
                let got = read(json!({ "path": path })).await;
                if page.peek().as_ref() != Some(&got) {
                    page.set(Some(got));
                }
            });
        }
    }));
    let Some(current) = current else {
        return rsx! {
            div { style: "padding: 24px; {MUTED}",
                "Open a project from the Projects list (or PROJECTS_OPEN {{path}}) to see its page."
            }
        };
    };
    let Some(got) = page() else {
        return rsx! { div { style: "padding: 24px; {MUTED}", "Reading {current}…" } };
    };
    let data = match got {
        Ok(d) => d,
        Err(e) => return rsx! { div { style: "padding: 24px; color: var(--destructive);", "{e}" } },
    };
    let p: Project = serde_json::from_value(data["project"].clone()).unwrap_or_default();
    rsx! { ProjectPage { key: "{p.path}", project: p, data } }
}

#[component]
fn ProjectPage(project: Project, data: Value) -> Element {
    let p = project;
    let mut member = use_signal(String::new);
    let mut milestone = use_signal(String::new);
    let mut task = use_signal(String::new);
    let mut problem = use_signal(|| None::<String>);
    let state = p.state();
    let type_name = data["type"]["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| p.project_type.clone());
    let type_props: Vec<(String, String)> = data["type"]["properties"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|prop| {
            Some((
                prop.get("key")?.as_str()?.to_owned(),
                prop.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ))
        })
        .collect();
    let roadmap = data["roadmap"].as_array().cloned().unwrap_or_default();
    let goals = data["goals"].as_array().cloned().unwrap_or_default();
    let notes = data["notes"].as_array().cloned().unwrap_or_default();
    let tasks = data["tasks"].clone();
    let has_tasks = !tasks.is_null() && tasks.get("error").is_none();
    let task_rows = tasks["tasks"].as_array().cloned().unwrap_or_default();
    let (done, total) = (
        tasks["done"].as_u64().unwrap_or(0),
        tasks["total"].as_u64().unwrap_or(0),
    );
    let path = p.path.clone();
    let set = move |s: State| {
        let path = path.clone();
        move |_| {
            let path = path.clone();
            spawn(async move {
                let _ = set_state(json!({ "path": path, "state": s.as_str() })).await;
            });
        }
    };
    let mut add_member_now = {
        let path = p.path.clone();
        move || {
            let who = member.peek().trim().to_owned();
            if who.is_empty() {
                return;
            }
            member.set(String::new());
            let path = path.clone();
            spawn(async move {
                if let Err(e) = add_member(json!({ "path": path, "member": who })).await {
                    problem.set(Some(e));
                }
            });
        }
    };
    let mut add_milestone = {
        let path = p.path.clone();
        move || {
            let text = milestone.peek().trim().to_owned();
            if text.is_empty() {
                return;
            }
            milestone.set(String::new());
            // "Beta 2026-12-01": a trailing date is its due date.
            let (title, due) = match text.rsplit_once(' ') {
                Some((t, d)) if d.len() == 10 && d.as_bytes().get(4) == Some(&b'-') => {
                    (t.to_owned(), Some(d.to_owned()))
                }
                _ => (text, None),
            };
            let path = path.clone();
            spawn(async move {
                let mut args = json!({ "project": path, "title": title });
                if let Some(d) = due {
                    args["due"] = json!(d);
                }
                if let Err(e) = milestone_create(args).await {
                    problem.set(Some(e));
                }
            });
        }
    };
    let mut add_task = {
        let path = p.path.clone();
        move || {
            let text = task.peek().trim().to_owned();
            if text.is_empty() {
                return;
            }
            task.set(String::new());
            let path = path.clone();
            spawn(async move {
                match vault::service(TASKS, "add", json!({ "text": text, "project": path })).await {
                    Some(Err(e)) => problem.set(Some(e)),
                    _ => changed(),
                }
            });
        }
    };
    let section = "display: flex; flex-direction: column; gap: 6px;";
    rsx! {
        div { class: "projects-page", "data-path": "{p.path}",
            style: "display: flex; flex-direction: column; gap: 18px; padding: 20px 24px; height: 100%; box-sizing: border-box; \
                    overflow-y: auto; color: var(--foreground); font-size: 13px;",
            // Head: title, type, due, state.
            div { style: "display: flex; flex-direction: column; gap: 8px;",
                div { style: "display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;",
                    span { class: "projects-title", style: "font-size: 22px; font-weight: 700;", "{p.title}" }
                    if let Some(t) = type_name {
                        span { style: "padding: 2px 8px; border-radius: 999px; font-size: 12px; background: var(--secondary); \
                                       color: var(--secondary-foreground);", "{t}" }
                    }
                    if let Some(d) = &p.due {
                        span { style: "font-size: 12px; {MUTED}", "due {d}" }
                    }
                    span { style: "flex: 1;" }
                    if has_tasks && total > 0 {
                        span { style: "font-size: 12px; {MUTED}", "{done}/{total} tasks done" }
                    }
                }
                div { class: "projects-state", style: "display: flex; gap: 6px; flex-wrap: wrap;",
                    for s in [State::Planning, State::Active, State::OnHold, State::Done] {
                        button { key: "{s:?}", "data-state": "{s.as_str()}", style: chip(s == state), onclick: set(s), "{s.label()}" }
                    }
                }
            }
            if let Some(e) = problem() {
                div { style: "color: var(--destructive);", "{e}" }
            }
            // Members.
            div { style: section,
                div { style: HEADING, "Members" }
                div { style: "display: flex; gap: 6px; flex-wrap: wrap; align-items: center;",
                    for m in p.members.iter() {
                        span { key: "{m}", style: "padding: 3px 10px; border-radius: 999px; font-size: 12px; background: var(--muted);",
                            "{link_label(m)}"
                        }
                    }
                    input {
                        r#type: "text", "aria-label": "Add a member", placeholder: "Add a member",
                        value: "{member}", style: "{FIELD} width: 160px;",
                        oninput: move |e: FormEvent| member.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { add_member_now() },
                    }
                }
            }
            // Its type's properties.
            if !type_props.is_empty() {
                div { style: section,
                    div { style: HEADING, "Details" }
                    for (key, name) in type_props {
                        div { key: "{key}", style: "display: flex; gap: 10px;",
                            span { style: "width: 120px; {MUTED}", if name.is_empty() { "{key}" } else { "{name}" } }
                            span { {data["project"].get(&key).and_then(model::scalar).unwrap_or_else(|| "—".to_owned())} }
                        }
                    }
                }
            }
            // Roadmap.
            div { class: "projects-roadmap", style: section,
                div { style: HEADING, "Roadmap" }
                if roadmap.is_empty() {
                    div { style: MUTED, "No milestones yet." }
                }
                for m in roadmap {
                    MilestoneLine { key: "{m[\"path\"]}", milestone: m }
                }
                input {
                    r#type: "text", "aria-label": "New milestone", placeholder: "New milestone (a date at the end is its due date)",
                    value: "{milestone}", style: FIELD,
                    oninput: move |e: FormEvent| milestone.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { add_milestone() },
                }
            }
            if !goals.is_empty() {
                div { style: section,
                    div { style: HEADING, "Goals" }
                    for g in goals {
                        div { key: "{g[\"path\"]}", style: "display: flex; gap: 10px;",
                            span { style: "flex: 1;", {g["title"].as_str().unwrap_or_default().to_owned()} }
                            span { style: "font-size: 12px; {MUTED}", {format!("{} · {}", g["kind"].as_str().unwrap_or(""), g["status"].as_str().unwrap_or(""))} }
                        }
                    }
                }
            }
            // Tasks, from the Tasks extension.
            div { class: "projects-tasks", style: section,
                div { style: HEADING, "Tasks" }
                if !has_tasks {
                    div { style: MUTED,
                        {tasks.get("error").and_then(Value::as_str).map_or_else(
                            || "Turn on the Tasks extension to see and add this project's tasks here.".to_owned(),
                            |e| format!("Its tasks could not be read: {e}"),
                        )}
                    }
                } else {
                    if task_rows.is_empty() {
                        div { style: MUTED, "No tasks yet." }
                    }
                    for t in task_rows {
                        TaskLine { key: "{t[\"path\"]}", task: t }
                    }
                    input {
                        r#type: "text", "aria-label": "New task in this project", placeholder: "New task — e.g. Draft the brief tomorrow !high",
                        value: "{task}", style: FIELD,
                        oninput: move |e: FormEvent| task.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { add_task() },
                    }
                }
            }
            // Its other notes.
            div { style: section,
                div { style: HEADING, "Notes" }
                if notes.is_empty() {
                    div { style: MUTED, "Nothing else in {p.folder} yet." }
                }
                for n in notes {
                    NoteLine { key: "{n[\"path\"]}", note: n }
                }
            }
        }
    }
}

#[component]
fn MilestoneLine(milestone: Value) -> Element {
    let m = milestone;
    let total = m["progress"]["total"].as_u64().unwrap_or(0);
    let done = m["progress"]["done"].as_u64().unwrap_or(0);
    let pct = if total == 0 { 0 } else { done * 100 / total };
    let open = m["status"].as_str() == Some("open");
    let path = m["path"].as_str().unwrap_or_default().to_owned();
    let fill = if open {
        "transparent"
    } else {
        "var(--primary)"
    };
    rsx! {
        div { class: "projects-milestone", style: "display: flex; align-items: center; gap: 10px; padding: 4px 0;",
            button {
                "aria-label": if open { "Close the milestone" } else { "Open the milestone again" },
                style: "width: 16px; height: 16px; padding: 0; border-radius: 50%; cursor: pointer; border: 1.5px solid var(--border); \
                        background: {fill};",
                onclick: move |_| {
                    let path = path.clone();
                    spawn(async move {
                        let _ = milestone_status(json!({ "path": path, "status": if open { "closed" } else { "open" } })).await;
                    });
                },
            }
            span { style: "width: 180px; font-weight: 600;", {m["title"].as_str().unwrap_or_default().to_owned()} }
            div { style: "flex: 1; height: 6px; border-radius: 3px; background: var(--muted); overflow: hidden;",
                div { style: "height: 100%; width: {pct}%; background: var(--primary);" }
            }
            span { style: "width: 70px; text-align: right; font-size: 12px; {MUTED}", "{done}/{total}" }
            span { style: "width: 84px; text-align: right; font-size: 12px; font-variant-numeric: tabular-nums; {MUTED}",
                {m["dueDate"].as_str().or(m["due_date"].as_str()).unwrap_or("").to_owned()}
            }
        }
    }
}

/// A task's line on the page: its box runs Tasks' `set_status`.
#[component]
fn TaskLine(task: Value) -> Element {
    let open = task["open"].as_bool().unwrap_or(true);
    let path = task["path"].as_str().unwrap_or_default().to_owned();
    let doing = task["group"].as_str() == Some("doing");
    let status = task["status"].as_str().unwrap_or_default().to_owned();
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
    rsx! {
        div { class: "projects-task", "data-path": "{path}", "data-status": "{status}",
            style: "display: flex; align-items: center; gap: 10px; padding: 4px 0;",
            button {
                "aria-label": if open { "Mark done" } else { "Mark open" },
                style: "width: 18px; height: 18px; padding: 0; cursor: pointer; border-radius: 5px; border: 1.5px solid var(--border); \
                        font-size: 11px; line-height: 14px; color: var(--primary-foreground); \
                        background: {fill};",
                onclick: move |_| {
                    let path = path.clone();
                    spawn(async move {
                        let _ = vault::service(TASKS, "set_status", json!({ "path": path, "status": if open { "done" } else { "open" } })).await;
                        changed();
                    });
                },
                if !open { "✓" }
            }
            span { style: "flex: 1; {strike}",
                {task["title"].as_str().unwrap_or_default().to_owned()}
            }
            if doing {
                span { style: "font-size: 11px; padding: 1px 6px; border-radius: 999px; background: var(--secondary); color: var(--secondary-foreground);", "doing" }
            }
            span { style: "font-size: 12px; font-variant-numeric: tabular-nums; {MUTED}",
                {task["due"].as_str().map(|d| d.get(..10).unwrap_or(d).to_owned()).unwrap_or_default()}
            }
        }
    }
}

#[component]
fn NoteLine(note: Value) -> Element {
    let path = note["path"].as_str().unwrap_or_default().to_owned();
    let title = note["title"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| path.clone());
    rsx! {
        div { style: "padding: 3px 0; cursor: pointer;",
            onclick: move |_| {
                let path = path.clone();
                spawn(async move {
                    if let Some(h) = vault::handle() {
                        let _ = h.host(architect_extensions::hostcall::HostRequest::Open { path }).await;
                    }
                });
            },
            "📄 {title}"
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn its_base_parses() {
        let v: serde_yaml::Value = serde_yaml::from_str(&notes_base()).unwrap();
        assert!(v.get("views").is_some());
    }

    #[test]
    fn a_book_sorts_rows_by_kind() {
        let row = |path: &str, v: Value| vault::Row {
            path: path.into(),
            name: path
                .rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".md")
                .into(),
            values: serde_json::from_value(v).unwrap(),
        };
        let book = Book::of(&[
            row("Projects/B/project.md", json!({ "state": "done" })),
            row("Projects/A/project.md", json!({ "status": "active" })),
            row(
                "Projects/A/Milestones/Beta.md",
                json!({ "type": "milestone", "dueDate": "2026-12-01" }),
            ),
            row(
                "Projects/A/Milestones/Alpha.md",
                json!({ "type": "milestone", "dueDate": "2026-11-01" }),
            ),
            row("Goals/Run.md", json!({ "type": "goal" })),
            row("Elsewhere/type-only.md", json!({ "type": "project" })),
        ]);
        let titles: Vec<&str> = book.projects.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(
            titles,
            ["A", "B"],
            "active before done; only project.md pages"
        );
        let ms: Vec<&str> = book.milestones.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(ms, ["Alpha", "Beta"], "by due date");
        assert_eq!(book.goals.len(), 1);
    }
}
