//! Its navigator provider (`[[navigator_providers]] id = "projects"`):
//! the navigator's Projects row (#443).
//!
//! The projects on the left, nested (a sub-project under its parent: in
//! its folder, or linked by `parent`/`up`), each with its own icon, its
//! home org's badge when it is another org's, and a shared mark when it
//! is in a Space. On the right, the selected project's contents, grouped
//! in this order:
//! - **Songs:** its notes of `type: song` (an album's), in their hand
//!   order (`sort_index`, #425); a song loads onto a deck;
//! - **Notes:** its other notes (in its folder, or naming it in
//!   `project`/`projects`);
//! - **Sub-projects:** the projects under it;
//! - **Tasks:** its tasks (the Tasks extension's, `architect.tasks`
//!   `for_project`, when it is loaded).
//!
//! It implements `architect_navigator_api::NavigatorProvider`, the MIT
//! API: this crate never links the GPL navigator. `Builder::navigator`
//! serves it as its `PROJECTS_NAV_*` actions, linked in or as a process.
//!
//! - **Items:** a task in a project is item `<project.md>|<task path>`
//!   ([`item_id`]): a task in two projects is two items.
//! - **Open** a task: its note, in the workspace (asked of it, so a
//!   selection routes back here).
//! - **Move** a task to another project (a drag onto it): its file moves
//!   into that project's folder when it was in the old one's
//!   (`VAULT_RENAME`, links rewritten), and its `projects` become that one
//!   (`architect.tasks` `update`).
//! - **New** in a project: a task in it (`architect.tasks` `create`).

use architect_navigator_api::{
    Item, NavigatorProvider, Node, Open, PROJECT_ICON, ProviderFuture, Snapshot, date_millis,
};
use serde_json::{Value, json};

use crate::model::{Project, link_target, link_to};
use crate::{TASKS, refresh, vault};

/// Its actions' prefix (`[[navigator_providers]] actions`).
pub const ACTIONS: &str = "PROJECTS_NAV";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectsNav;

/// A task's item id in project `project`.
#[must_use]
pub fn item_id(project: &str, task: &str) -> String {
    format!("{project}|{task}")
}

/// An item id's project and task (an id without `|` is a task alone).
#[must_use]
pub fn split_item(id: &str) -> (&str, &str) {
    id.split_once('|').unwrap_or(("", id))
}

/// Where a task at `task` goes when it moves from project `from` to `to`
/// (each a `project.md`): into `to`'s folder when it is inside `from`'s,
/// else where it is.
#[must_use]
pub fn moved_path(task: &str, from: &str, to: &str) -> String {
    let folder = |p: &str| p.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
    let (from, to) = (folder(from), folder(to));
    match task.strip_prefix(&format!("{from}/")) {
        Some(rest) if !from.is_empty() && !to.is_empty() => format!("{to}/{rest}"),
        _ => task.to_owned(),
    }
}

/// A project's contents' groups, in order (`Snapshot::group_by`).
pub const SECTIONS: [&str; 4] = ["Songs", "Notes", "Sub-projects", "Tasks"];

/// The item property they are grouped by.
pub const SECTION: &str = "section";

/// Whether node `id` is a project (its `project.md`), not a state.
fn is_project(id: &str) -> bool {
    std::path::Path::new(id)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// The project `p` sits under among `projects`: the one whose folder
/// holds its folder most closely, else the one its `parent` (or `up`)
/// links to.
#[must_use]
pub fn parent_of<'a>(p: &Project, projects: &'a [Project]) -> Option<&'a Project> {
    let within = projects
        .iter()
        .filter(|q| q.path != p.path && !q.folder.is_empty())
        .filter(|q| p.folder.starts_with(&format!("{}/", q.folder)))
        .max_by_key(|q| q.folder.len());
    within.or_else(|| {
        let link = p.parent.as_deref().or(p.up.as_deref())?;
        let target = link_target(link);
        let target = target.strip_suffix(".md").unwrap_or(target);
        projects.iter().find(|q| {
            q.path != p.path
                && (q.path.strip_suffix(".md") == Some(target)
                    || q.folder == target
                    || q.title.eq_ignore_ascii_case(target))
        })
    })
}

/// `p`'s node, with the projects under it.
fn project_node(p: &Project, projects: &[Project]) -> Node {
    // Its own icon, else the project glyph: never a folder's.
    let mut n = Node::new(&p.path, &p.title).icon(p.icon.as_deref().unwrap_or(PROJECT_ICON));
    n.colour.clone_from(&p.colour);
    if let Some(org) = &p.org {
        n = n.org(org);
    }
    if p.shared {
        n = n.shared();
    }
    for c in projects
        .iter()
        .filter(|c| parent_of(c, projects).is_some_and(|q| q.path == p.path))
    {
        n = n.child(project_node(c, projects));
    }
    n
}

/// The project a note at `path` with `values` belongs to: one it names
/// in `project`/`projects` (a song, in `album`), else the deepest whose
/// folder holds it.
fn owner_of<'a>(
    path: &str,
    values: &serde_json::Map<String, Value>,
    projects: &'a [Project],
) -> Option<&'a Project> {
    let named = ["project", "projects", "album"]
        .iter()
        .filter_map(|k| values.get(*k))
        .flat_map(|v| match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
            Value::String(s) => vec![s.as_str()],
            _ => Vec::new(),
        })
        .find_map(|link| {
            let target = link_target(link);
            let target = target.strip_suffix(".md").unwrap_or(target);
            projects.iter().find(|q| {
                q.path.strip_suffix(".md") == Some(target)
                    || q.folder == target
                    || q.title.eq_ignore_ascii_case(target)
            })
        });
    named.or_else(|| {
        projects
            .iter()
            .filter(|q| !q.folder.is_empty() && path.starts_with(&format!("{}/", q.folder)))
            .max_by_key(|q| q.folder.len())
    })
}

/// A note of a project's, in its contents.
fn content_item(row: &crate::vault::Row, project: &Project) -> Item {
    let v = &row.values;
    let text = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let song = text("type").is_some_and(|t| t.eq_ignore_ascii_case("song"))
        || v.get("tags")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|t| t.as_str() == Some("song")));
    let title = text("title").unwrap_or(&row.name);
    let created = text("created").and_then(date_millis).unwrap_or(0);
    let modified = text("modified").and_then(date_millis).unwrap_or(created);
    let mut item = Item::new(
        format!("{}|{}", project.path, row.path),
        &project.path,
        title,
    )
    .dates(created, modified)
    .property(SECTION, if song { "Songs" } else { "Notes" });
    // Its hand order (#425): a song's place on the album.
    let rank = v
        .get("sort_index")
        .or_else(|| v.get("track"))
        .and_then(|r| {
            r.as_f64()
                .or_else(|| r.as_str().and_then(|s| s.parse().ok()))
        });
    if let Some(r) = rank {
        item = item.property("sort_index", r.to_string());
    }
    if song {
        let shown = |x: &Value| x.as_str().map_or_else(|| x.to_string(), str::to_owned);
        let parts: Vec<String> = [
            v.get("key").map(shown),
            v.get("bpm")
                .or_else(|| v.get("tempo"))
                .map(|x| format!("{} bpm", shown(x))),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            item = item.preview(parts.join(" · "));
        }
        item.icon = Some("music".to_owned());
        // A song loads onto a deck (#441), in its project's folder.
        item = item.opens(Open::Context {
            context_type: "song".to_owned(),
            id: row.path.clone(),
            title: title.to_owned(),
            folder: Some(project.folder.clone()),
        });
    } else {
        item = item.opens(Open::File {
            path: row.path.clone(),
        });
    }
    for tag in v
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(t) = tag.as_str() {
            item = item.tag(t.trim_start_matches('#'));
        }
    }
    item
}

/// The Projects row's snapshot: `book`'s projects nested, each one's
/// notes and songs (`rows`, a contents base's), sub-projects and tasks
/// (`tasks`, by project path; `architect.tasks`' JSON) as its contents.
#[must_use]
pub fn snapshot_of(
    projects: &[Project],
    rows: &[crate::vault::Row],
    tasks: &[(String, Vec<Value>)],
) -> Snapshot {
    let mut snap = Snapshot::new("Projects").group_by(SECTION, SECTIONS);
    for p in projects.iter().filter(|p| parent_of(p, projects).is_none()) {
        snap = snap.node(project_node(p, projects));
    }
    let tasked: std::collections::HashSet<&str> = tasks
        .iter()
        .flat_map(|(_, ts)| {
            ts.iter()
                .filter_map(|t| t.get("path").and_then(Value::as_str))
        })
        .collect();
    for row in rows {
        let is_project = projects.iter().any(|q| q.path == row.path)
            || row.name == "project"
            || row
                .values
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| matches!(t, "project" | "goal" | "milestone" | "task"));
        if is_project || tasked.contains(row.path.as_str()) {
            continue;
        }
        if let Some(p) = owner_of(&row.path, &row.values, projects) {
            snap.items.push(content_item(row, p));
        }
    }
    for c in projects {
        if let Some(p) = parent_of(c, projects) {
            let mut item = Item::new(format!("{}|{}", p.path, c.path), &p.path, &c.title)
                .property(SECTION, "Sub-projects")
                .preview(c.state.replace('_', " "))
                .opens(Open::File {
                    path: c.path.clone(),
                });
            item.icon = Some(c.icon.clone().unwrap_or_else(|| PROJECT_ICON.to_owned()));
            snap.items.push(item);
        }
    }
    for (project, ts) in tasks {
        snap.items.extend(
            ts.iter()
                .filter_map(|t| task_item(t, project))
                .map(|i| i.property(SECTION, "Tasks")),
        );
    }
    snap
}

/// A base of every note in a project's folder, or naming one.
fn contents_base(projects: &[Project]) -> String {
    use std::fmt::Write as _;
    let mut filters = String::from(
        "    - file.hasProperty(\"project\")\n    - file.hasProperty(\"projects\")\n    - file.hasProperty(\"album\")\n",
    );
    for p in projects.iter().filter(|p| !p.folder.is_empty()) {
        let _ = writeln!(filters, "    - file.inFolder({:?})", p.folder);
    }
    format!(
        "filters:\n  or:\n{filters}views:\n  - type: table\n    name: Contents\n    order:\n      - type\n      - title\n      - project\n      - projects\n      - sort_index\n      - track\n      - key\n      - bpm\n      - tempo\n      - album\n      - tags\n      - created\n      - modified\n"
    )
}

/// A task (`architect.tasks`' JSON) as an item in project `project`.
fn task_item(t: &Value, project: &str) -> Option<Item> {
    let text = |k: &str| t.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let path = text("path")?;
    let title = text("title").unwrap_or(path);
    let open = t.get("open").and_then(Value::as_bool).unwrap_or(true);
    let created = text("date_created").and_then(date_millis).unwrap_or(0);
    let modified = text("date_modified")
        .and_then(date_millis)
        .unwrap_or(created);
    let mut item = Item::new(item_id(project, path), project, title)
        .dates(created, modified)
        .progress(u32::from(!open), 1);
    let status = text("status").unwrap_or("open");
    let mut preview = status.replace('_', " ");
    if let Some(due) = text("due") {
        preview.push_str(" · due ");
        preview.push_str(due);
        item = item.property("due", due);
    }
    item = item.preview(preview).property("status", status);
    if let Some(p) = text("priority").filter(|p| *p != "normal" && *p != "none") {
        item = item.badge(p).property("priority", p);
    }
    for tag in t
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(tag) = tag.as_str() {
            item = item.tag(tag.trim_start_matches('#'));
        }
    }
    Some(item)
}

impl NavigatorProvider for ProjectsNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            let book = refresh().await?;
            // Their notes and songs; none when the base cannot be read.
            let rows = vault::rows(&contents_base(&book.projects))
                .await
                .unwrap_or_default();
            let mut tasks = Vec::new();
            for p in &book.projects {
                // No Tasks extension loaded: projects with no tasks.
                let Some(Ok(out)) =
                    vault::service(TASKS, "for_project", json!({ "project": p.path })).await
                else {
                    continue;
                };
                let ts = out
                    .get("tasks")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                tasks.push((p.path.clone(), ts));
            }
            Ok(snapshot_of(&book.projects, &rows, &tasks))
        })
    }

    /// A task opens as its note.
    fn open(&self, item: String) -> ProviderFuture<Open> {
        let path = split_item(&item).1.to_owned();
        Box::pin(async move { Ok(Open::File { path }) })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            if !is_project(&to) {
                return Err("drop tasks on a project".to_owned());
            }
            for item in items {
                let (from, task) = split_item(&item);
                let path = moved_path(task, from, &to);
                if path != task {
                    vault::run("VAULT_RENAME", json!({ "from": task, "to": path })).await?;
                }
                vault::service(
                    TASKS,
                    "update",
                    json!({ "path": path, "projects": [link_to(&to, None)] }),
                )
                .await
                .ok_or("the Tasks extension is not loaded")??;
            }
            Ok(())
        })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            if !is_project(&container) {
                return Err("make a task in a project".to_owned());
            }
            let made = vault::service(
                TASKS,
                "create",
                json!({ "title": title, "project": container }),
            )
            .await
            .ok_or("the Tasks extension is not loaded")??;
            Ok(made
                .pointer("/task/path")
                .or_else(|| made.get("path"))
                .and_then(Value::as_str)
                .map_or(Open::Nothing, |p| Open::File { path: p.to_owned() }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_is_an_item_of_its_project() {
        let t = json!({
            "path": "Tasks/Ship.md", "title": "Ship it", "status": "in_progress",
            "priority": "high", "due": "2026-11-09", "open": true,
            "tags": ["#release"], "date_created": "2026-11-01",
        });
        let item = task_item(&t, "Projects/Alpha/project.md").unwrap_or_default();
        assert_eq!(item.id, "Projects/Alpha/project.md|Tasks/Ship.md");
        assert_eq!(
            split_item(&item.id),
            ("Projects/Alpha/project.md", "Tasks/Ship.md")
        );
        assert_eq!(item.container, "Projects/Alpha/project.md");
        assert_eq!(
            item.preview.as_deref(),
            Some("in progress · due 2026-11-09")
        );
        assert_eq!(item.badges, ["high"]);
        assert_eq!(item.tags, ["release"]);
        assert_eq!(item.progress.map(|p| (p.done, p.total)), Some((0, 1)));
        assert!(item.created > 0);
        assert!(task_item(&json!({ "title": "no path" }), "p").is_none());
    }

    fn row(path: &str, values: &Value) -> crate::vault::Row {
        let name = path
            .rsplit('/')
            .next()
            .and_then(|n| n.strip_suffix(".md"))
            .unwrap_or(path)
            .to_owned();
        crate::vault::Row {
            path: path.to_owned(),
            name,
            values: values.as_object().cloned().unwrap_or_default(),
        }
    }

    fn project(path: &str, values: &Value) -> Project {
        let empty = serde_json::Map::new();
        Project::from_values(path, values.as_object().unwrap_or(&empty))
    }

    #[test]
    fn an_album_lists_its_songs_in_order_then_notes_sub_projects_and_tasks() {
        let projects = vec![
            project(
                "Projects/Crescendum/project.md",
                &json!({"title": "Crescendum", "type": "project", "icon": "disc-3"}),
            ),
            project(
                "Projects/Crescendum/Mixing/project.md",
                &json!({"title": "Mixing", "type": "project"}),
            ),
            project(
                "Projects/Duet/project.md",
                &json!({"title": "Duet", "org": "tombrooksmusic", "space": "crescendum"}),
            ),
        ];
        let rows = vec![
            row(
                "Projects/Crescendum/Songs/Overture.md",
                &json!({"type": "song", "sort_index": 1, "key": "D", "bpm": 92}),
            ),
            row(
                "Projects/Crescendum/Songs/Finale.md",
                &json!({"type": "song", "sort_index": 3}),
            ),
            row(
                "Projects/Crescendum/Songs/Interlude.md",
                &json!({"type": "song", "track": "2"}),
            ),
            row("Projects/Crescendum/Liner notes.md", &json!({})),
            row(
                "Notes/Press kit.md",
                &json!({"projects": ["[[Projects/Crescendum/project|Crescendum]]"]}),
            ),
            row("Projects/Crescendum/Mixing/Mix notes.md", &json!({})),
            row(
                "Projects/Crescendum/project.md",
                &json!({"type": "project"}),
            ),
            row(
                "Projects/Crescendum/Tasks/Book studio.md",
                &json!({"type": "task"}),
            ),
        ];
        let tasks = vec![(
            "Projects/Crescendum/project.md".to_owned(),
            vec![
                json!({"path": "Projects/Crescendum/Tasks/Book studio.md", "title": "Book studio", "status": "open"}),
            ],
        )];
        let snap = snapshot_of(&projects, &rows, &tasks);
        assert_eq!(snap.group_by.as_deref(), Some("section"));
        assert_eq!(snap.group_order, SECTIONS);
        // Nested: Mixing under Crescendum; Duet at the top, another org's
        // and shared.
        let titles: Vec<&str> = snap.tree.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["Crescendum", "Duet"]);
        assert_eq!(snap.tree[0].children[0].title, "Mixing");
        assert_eq!(snap.tree[0].icon.as_deref(), Some("disc-3"));
        assert_eq!(snap.tree[1].org.as_deref(), Some("tombrooksmusic"));
        assert!(snap.tree[1].shared && !snap.tree[0].shared);
        let of = |project: &str, section: &str| -> Vec<String> {
            snap.items
                .iter()
                .filter(|i| i.container == project)
                .filter(|i| {
                    i.properties
                        .iter()
                        .any(|(k, v)| k == SECTION && v == section)
                })
                .map(|i| i.title.clone())
                .collect()
        };
        let album = "Projects/Crescendum/project.md";
        let mut songs: Vec<(String, f64)> = snap
            .items
            .iter()
            .filter(|i| i.container == album && i.icon.as_deref() == Some("music"))
            .map(|i| {
                let r = i
                    .properties
                    .iter()
                    .find(|(k, _)| k == "sort_index")
                    .map_or(0.0, |(_, v)| v.parse().unwrap_or(0.0));
                (i.title.clone(), r)
            })
            .collect();
        songs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let songs: Vec<&str> = songs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(songs, ["Overture", "Interlude", "Finale"]);
        assert_eq!(of(album, "Notes"), ["Liner notes", "Press kit"]);
        assert_eq!(of(album, "Sub-projects"), ["Mixing"]);
        assert_eq!(of(album, "Tasks"), ["Book studio"]);
        // The sub-project's own note is its, not the album's.
        assert_eq!(
            of("Projects/Crescendum/Mixing/project.md", "Notes"),
            ["Mix notes"]
        );
        let overture = snap.items.iter().find(|i| i.title == "Overture");
        assert!(matches!(
            overture.and_then(|i| i.open.clone()),
            Some(Open::Context { context_type, .. }) if context_type == "song"
        ));
        assert_eq!(
            overture.and_then(|i| i.preview.clone()).as_deref(),
            Some("D · 92 bpm")
        );
    }

    #[test]
    fn a_task_in_its_project_folder_moves_with_it() {
        let (a, b) = (
            "Projects/Album/project.md",
            "Projects/Half Marathon/project.md",
        );
        assert_eq!(
            moved_path("Projects/Album/Tasks/Mix.md", a, b),
            "Projects/Half Marathon/Tasks/Mix.md"
        );
        // Kept elsewhere: only its link changes.
        assert_eq!(moved_path("Tasks/Mix.md", a, b), "Tasks/Mix.md");
    }
}
