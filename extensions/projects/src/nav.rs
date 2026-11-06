//! Its navigator provider (`[[navigator_providers]] id = "projects"`).
//!
//! The projects on the left, under their state (Active, Planning, On hold,
//! Done), and the selected project's tasks on the right (the Tasks
//! extension's, `architect.tasks` `for_project`, when it is loaded).
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
    Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot, date_millis,
};
use serde_json::{Value, json};

use crate::model::{State, link_to};
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

/// Whether node `id` is a project (its `project.md`), not a state.
fn is_project(id: &str) -> bool {
    std::path::Path::new(id)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// A state's node id.
fn state_node(s: State) -> String {
    format!("state:{}", s.as_str())
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
            let mut snap = Snapshot::new("Projects");
            for state in State::ALL {
                let mut node = Node::new(state_node(state), state.label()).icon("layers");
                for p in book.projects.iter().filter(|p| p.state() == state) {
                    let mut n = Node::new(&p.path, &p.title).icon("folder-kanban");
                    if let Some(i) = &p.icon {
                        n.icon = Some(i.clone());
                    }
                    node = node.child(n);
                }
                if !node.children.is_empty() {
                    snap = snap.node(node);
                }
            }
            for p in &book.projects {
                // No Tasks extension loaded: projects with no items.
                let Some(Ok(out)) =
                    vault::service(TASKS, "for_project", json!({ "project": p.path })).await
                else {
                    continue;
                };
                let tasks = out.get("tasks").and_then(Value::as_array);
                snap.items.extend(
                    tasks
                        .into_iter()
                        .flatten()
                        .filter_map(|t| task_item(t, &p.path)),
                );
            }
            Ok(snap)
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
