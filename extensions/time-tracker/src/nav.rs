//! Its navigator provider (`[[navigator_providers]] id = "time"`).
//!
//! When it was worked on, on the left (Running, Today, This week,
//! Earlier); the tasks, projects and notes last tracked then on the right,
//! each with its week's and its whole time.
//!
//! It implements `architect_navigator_api::NavigatorProvider`, the MIT
//! API: this crate never links the GPL navigator. `Builder::navigator`
//! serves it as its `TIME_NAV_*` actions, linked in or as a process.
//!
//! - **Open** one: its note.
//! - **Drag** one onto Running: a timer starts on it.

use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde_json::json;

use crate::model::{self, Kind, Tracked};
use crate::{refresh, start};

/// Its actions' prefix (`[[navigator_providers]] actions`).
pub const ACTIONS: &str = "TIME_NAV";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeNav;

/// Its nodes: id, title, icon.
const NODES: [(&str, &str, &str); 4] = [
    ("running", "Running", "timer"),
    ("today", "Today", "calendar"),
    ("week", "This week", "calendar-range"),
    ("earlier", "Earlier", "history"),
];

/// The node a note falls in: when it was last worked on.
fn when(n: &Tracked, today: NaiveDate) -> &'static str {
    if n.running().is_some() {
        return "running";
    }
    let last = n
        .entries
        .iter()
        .filter_map(|e| e.end)
        .max()
        .map(|t| t.with_timezone(&Local).date_naive());
    match last {
        Some(d) if d == today => "today",
        Some(d) if d >= model::week_start(today) => "week",
        _ => "earlier",
    }
}

/// A tracked note as an item of its node.
#[must_use]
pub fn note_item(n: &Tracked, now: DateTime<Utc>, today: NaiveDate) -> Item {
    let week = model::day_start(model::week_start(today), &Local);
    let first = n.entries.iter().map(|e| e.start).min();
    let last = n.entries.iter().map(|e| e.end.unwrap_or(now)).max();
    let millis = |t: Option<DateTime<Utc>>| t.map_or(0, |t| t.timestamp_millis());
    let kind = match n.kind {
        Kind::Task => "task",
        Kind::Project => "project",
        Kind::Note => "note",
    };
    let mut item = Item::new(&n.path, when(n, today), &n.title)
        .dates(millis(first), millis(last))
        .preview(format!(
            "{} this week · {} in all",
            model::words(n.seconds_in(week, now, now)),
            model::words(n.seconds(now))
        ))
        .property("kind", kind)
        .opens(Open::File {
            path: n.path.clone(),
        });
    if let Some(e) = n.running() {
        item = item.badge(&model::clock(e.seconds(now)));
    }
    item
}

/// The nodes, and every note with time on it in its own.
#[must_use]
pub fn snapshot_of(notes: &[Tracked], now: DateTime<Utc>, today: NaiveDate) -> Snapshot {
    let mut snap = Snapshot::new("Time").ordered();
    for (id, title, icon) in NODES {
        snap = snap.node(Node::new(id, title).icon(icon));
    }
    snap.items = notes
        .iter()
        .filter(|n| !n.entries.is_empty())
        .map(|n| note_item(n, now, today))
        .collect();
    snap
}

impl NavigatorProvider for TimeNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            let notes = refresh().await?;
            Ok(snapshot_of(&notes, Utc::now(), Local::now().date_naive()))
        })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            if to != "running" {
                return Err("drop a task, project or note on Running to time it".to_owned());
            }
            let path = items.into_iter().next().ok_or("nothing to time")?;
            let kind = crate::cache()
                .notes
                .iter()
                .find(|n| n.path == path)
                .map_or(Kind::Note, |n| n.kind);
            let args = match kind {
                Kind::Task => json!({ "task": path }),
                Kind::Project => json!({ "project": path }),
                Kind::Note => json!({ "note": path }),
            };
            start(args).await.map(|_| ())
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use serde_json::{Map, Value};

    use super::*;

    fn note(path: &str, entries: Value) -> Tracked {
        let mut values = Map::new();
        values.insert("type".into(), json!("task"));
        values.insert("timeEntries".into(), entries);
        Tracked::from_values(path, path, &values)
    }

    #[test]
    fn notes_fall_in_when_they_were_worked_on() {
        let now = model::parse_time("2026-11-06T12:00:00Z").unwrap();
        let today = now.with_timezone(&Local).date_naive();
        let stamp = |h: i64| model::stamp(now - chrono::Duration::hours(h));
        let running = note("A.md", json!([{ "startTime": stamp(1) }]));
        let earlier = note(
            "B.md",
            json!([{ "startTime": stamp(24 * 30), "endTime": stamp(24 * 30 - 1) }]),
        );
        let none = note("C.md", json!([]));
        let snap = snapshot_of(&[running, earlier, none], now, today);
        assert_eq!(snap.tree.len(), 4);
        assert_eq!(snap.items.len(), 2, "a note with no time is not listed");
        assert_eq!(snap.items[0].container, "running");
        assert_eq!(snap.items[0].badges, ["1:00"]);
        assert_eq!(snap.items[1].container, "earlier");
        assert_eq!(
            snap.items[1].preview.as_deref(),
            Some("0m this week · 1h in all")
        );
    }
}
