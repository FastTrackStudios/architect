//! Its navigator provider (`[[navigator_providers]] id = "tasks"`).
//!
//! The status groups on the left (To do, In progress, Waiting, Done,
//! Cancelled), the tasks in the selected one on the right.
//!
//! It implements `architect_navigator_api::NavigatorProvider`, the MIT
//! API: this crate never links the GPL navigator. `Builder::navigator`
//! serves it as its `TASKS_NAV_*` actions, linked in or as a process.
//!
//! - **Open** a task: its note (each item says so itself, [`Item::open`]).
//! - **Drag** tasks onto a group: they take its status (a repeating task
//!   dragged to Done moves on, as `TASKS_SET_STATUS` does).
//! - **New** in a group: a task with its status.

use architect_navigator_api::{
    Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot, date_millis,
};
use serde_json::{Value, json};

use crate::model::{Group, Task};
use crate::{refresh, set_status};

/// Its actions' prefix (`[[navigator_providers]] actions`).
pub const ACTIONS: &str = "TASKS_NAV";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct TasksNav;

/// A group's node id (`group:doing`).
fn group_node(g: Group) -> String {
    let name = serde_json::to_value(g)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!("group:{name}")
}

fn group_of_node(id: &str) -> Option<Group> {
    Group::ALL.into_iter().find(|g| group_node(*g) == id)
}

const fn group_icon(g: Group) -> &'static str {
    match g {
        Group::Todo => "circle",
        Group::Doing => "circle-dot",
        Group::Waiting => "circle-pause",
        Group::Done => "circle-check",
        Group::Cancelled => "circle-x",
    }
}

/// A link's alias, else its target's last folder or name.
fn link_label(link: &str) -> String {
    if let Some((_, alias)) = link
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split_once('|')
    {
        return alias.to_owned();
    }
    let target = crate::model::link_target(link);
    let target = target.strip_suffix(".md").unwrap_or(target);
    let mut parts = target.rsplit('/');
    match (parts.next(), parts.next()) {
        (Some("project"), Some(folder)) => folder.to_owned(),
        (Some(name), _) => name.to_owned(),
        _ => target.to_owned(),
    }
}

/// A task as an item of its group.
#[must_use]
pub fn task_item(t: &Task) -> Item {
    let created = t.date_created.as_deref().and_then(date_millis).unwrap_or(0);
    let modified = t
        .date_modified
        .as_deref()
        .and_then(date_millis)
        .unwrap_or(created);
    let mut preview = Vec::new();
    if let Some(due) = &t.due {
        preview.push(format!("due {due}"));
    }
    // Its projects, by name: `[[Projects/Alpha/project|Alpha]]` is Alpha.
    preview.extend(t.projects.iter().map(|p| link_label(p)));
    let mut item = Item::new(&t.path, group_node(t.group()), &t.title)
        .dates(created, modified)
        .progress(u32::from(!t.is_open()), 1)
        .property("status", &t.status)
        .opens(Open::File {
            path: t.path.clone(),
        });
    if !preview.is_empty() {
        item = item.preview(preview.join(" · "));
    }
    if !t.priority.is_empty() && t.priority != "normal" && t.priority != "none" {
        item = item.badge(&t.priority).property("priority", &t.priority);
    }
    if let Some(due) = &t.due {
        item = item.property("due", due);
    }
    for tag in &t.tags {
        item = item.tag(tag.trim_start_matches('#'));
    }
    item
}

/// The groups, and every task in its own.
#[must_use]
pub fn snapshot_of(tasks: &[Task]) -> Snapshot {
    let mut snap = Snapshot::new("Tasks").ordered();
    for g in Group::ALL {
        snap = snap.node(Node::new(group_node(g), g.label()).icon(group_icon(g)));
    }
    snap.items = tasks.iter().map(task_item).collect();
    snap
}

impl NavigatorProvider for TasksNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async { Ok(snapshot_of(&refresh().await?)) })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            let g = group_of_node(&to).ok_or("drop tasks on a status")?;
            for path in items {
                set_status(&path, g.status().as_str()).await?;
            }
            Ok(())
        })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let status = group_of_node(&container).unwrap_or(Group::Todo).status();
            let made = crate::create(json!({ "title": title, "status": status.as_str() })).await?;
            Ok(made
                .get("path")
                .and_then(Value::as_str)
                .map_or(Open::Nothing, |p| Open::File { path: p.to_owned() }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_fall_in_their_status_group() {
        let mut a = Task::default();
        a.path = "Tasks/A.md".to_owned();
        a.title = "A".to_owned();
        a.status = "in-progress".to_owned();
        a.due = Some("2026-11-09".to_owned());
        let mut b = Task::default();
        b.path = "Tasks/B.md".to_owned();
        b.title = "B".to_owned();
        b.status = "shipped".to_owned();
        let snap = snapshot_of(&[a, b]);
        assert_eq!(snap.tree.len(), 5);
        assert_eq!(snap.items[0].container, "group:doing");
        assert_eq!(snap.items[0].preview.as_deref(), Some("due 2026-11-09"));
        assert_eq!(snap.items[1].container, "group:done");
        assert_eq!(snap.items[1].progress.map(|p| p.done), Some(1));
        assert_eq!(group_of_node("group:waiting"), Some(Group::Waiting));
        assert_eq!(group_of_node("state:active"), None);
    }
}
