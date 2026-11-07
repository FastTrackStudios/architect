//! The Queue (#443): everything waiting on you, one list, the navigator's
//! Inbox row (`[[navigator_providers]] id = "queue"`).
//!
//! Its groups, in order:
//! - **Inbox:** the captures still to process (`INBOX_LIST`), newest
//!   first, each with where it came from;
//! - **Today:** open tasks due or scheduled today, and the overdue;
//! - **Upcoming:** open tasks dated later, then the undated;
//! - **Waiting:** the parked ones (waiting, someday, blocked).
//!
//! A capture opens in the app's triage hook (`INBOX_PROCESS_CARD`, with
//! its `path`: `ProcessCard`'s shape from #445). Dragged onto a task
//! group, a capture becomes a task (`INBOX_PROCESS`, target `task`) and
//! moves into that group; a task dragged onto one is scheduled today
//! (Today), put on waiting (Waiting) or taken off today (Upcoming). The
//! row's count is the captures still to process.

use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::model::{Group, Task};
use crate::{refresh, set_status, vault};

/// Its actions' prefix.
pub const ACTIONS: &str = "TASKS_QUEUE_NAV";

/// What opening a capture asks the app: its triage card (#445's
/// `ProcessCard`), given `{"path": …}`.
pub const PROCESS_CARD: &str = "INBOX_PROCESS_CARD";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueNav;

/// One of its groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Inbox,
    Today,
    Upcoming,
    Waiting,
}

impl Section {
    pub const ALL: [Self; 4] = [Self::Inbox, Self::Today, Self::Upcoming, Self::Waiting];

    /// Its node's id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Today => "today",
            Self::Upcoming => "upcoming",
            Self::Waiting => "waiting",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Today => "Today",
            Self::Upcoming => "Upcoming",
            Self::Waiting => "Waiting",
        }
    }

    const fn icon(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Today => "star",
            Self::Upcoming => "calendar-days",
            Self::Waiting => "circle-pause",
        }
    }

    #[must_use]
    pub fn of(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

fn date(text: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text?.get(..10)?, "%Y-%m-%d").ok()
}

/// Where an open task goes on `today`; `None` for a done or cancelled one.
#[must_use]
pub fn section_of(t: &Task, today: NaiveDate) -> Option<Section> {
    if !t.is_open() {
        return None;
    }
    if t.group() == Group::Waiting {
        return Some(Section::Waiting);
    }
    let when = [date(t.due.as_deref()), date(t.scheduled.as_deref())]
        .into_iter()
        .flatten()
        .min();
    Some(match when {
        Some(d) if d <= today => Section::Today,
        _ => Section::Upcoming,
    })
}

/// A capture (`INBOX_LIST`'s) as an Inbox row.
fn capture_item(c: &Value) -> Option<Item> {
    let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
    let path = text("path");
    if path.is_empty() {
        return None;
    }
    let body = text("body");
    let title = Some(text("title"))
        .filter(|t| !t.is_empty())
        .or_else(|| body.lines().next())
        .unwrap_or(path);
    let at = architect_navigator_api::date_millis(text("created")).unwrap_or(0);
    let source = text("source");
    let mut preview = String::new();
    if !source.is_empty() {
        preview.push_str(source);
    }
    let first = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    if !first.is_empty() && first != title {
        if !preview.is_empty() {
            preview.push_str(" · ");
        }
        preview.push_str(first);
    }
    let mut item = Item::new(format!("capture:{path}"), Section::Inbox.id(), title)
        .dates(at, at)
        .property("section", Section::Inbox.title())
        .opens(Open::Action {
            id: PROCESS_CARD.to_owned(),
            args: json!({ "path": path }),
        });
    if !preview.is_empty() {
        item = item.preview(preview);
    }
    if !source.is_empty() {
        item = item.property("source", source);
    }
    item.icon = Some("inbox".to_owned());
    Some(item)
}

/// The Queue of these captures and tasks on `today`.
#[must_use]
pub fn snapshot_of(captures: &[Value], tasks: &[Task], today: NaiveDate) -> Snapshot {
    let mut snap = Snapshot::new("Inbox").grouped();
    for s in Section::ALL {
        snap = snap.node(Node::new(s.id(), s.title()).icon(s.icon()));
    }
    let inbox: Vec<Item> = captures.iter().filter_map(capture_item).collect();
    snap = snap.count(u32::try_from(inbox.len()).unwrap_or(u32::MAX));
    snap.items.extend(inbox);
    for t in tasks {
        let Some(s) = section_of(t, today) else {
            continue;
        };
        let mut item = crate::nav::task_item(t).property("section", s.title());
        s.id().clone_into(&mut item.container);
        let overdue = date(t.due.as_deref()).is_some_and(|d| d < today);
        if overdue {
            item = item.badge("overdue");
            item.preview = Some(
                item.preview
                    .take()
                    .map_or_else(|| "overdue".to_owned(), |p| format!("overdue · {p}")),
            );
        }
        snap.items.push(item);
    }
    snap
}

impl NavigatorProvider for QueueNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            // No capture feature in the app: no Inbox rows, the tasks still.
            let captures = vault::run("INBOX_LIST", json!({}))
                .await
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let tasks = refresh().await?;
            Ok(snapshot_of(&captures, &tasks, crate::today()))
        })
    }

    fn open(&self, item: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let open = item.strip_prefix("capture:").map_or_else(
                || Open::File { path: item.clone() },
                |path| Open::Action {
                    id: PROCESS_CARD.to_owned(),
                    args: json!({ "path": path }),
                },
            );
            Ok(open)
        })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            let to = Section::of(&to).ok_or("drop on Inbox, Today, Upcoming or Waiting")?;
            for item in items {
                let path = match item.strip_prefix("capture:") {
                    Some(capture) if to != Section::Inbox => {
                        // A capture processed into a task, then placed.
                        let made = vault::run(
                            "INBOX_PROCESS",
                            json!({ "path": capture, "target": "task" }),
                        )
                        .await?;
                        let Some(p) = made
                            .pointer("/task/path")
                            .or_else(|| made.pointer("/made/path"))
                            .or_else(|| made.get("path"))
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        p.to_owned()
                    }
                    Some(_) => continue,
                    None if to == Section::Inbox => {
                        return Err("a task does not go back to the inbox".to_owned());
                    }
                    None => item,
                };
                place(&path, to).await?;
            }
            Ok(())
        })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let to = Section::of(&container).unwrap_or(Section::Inbox);
            if to == Section::Inbox {
                vault::run("CAPTURE_ADD", json!({ "text": title })).await?;
                return Ok(Open::Nothing);
            }
            let mut args = serde_json::Map::new();
            args.insert("title".to_owned(), json!(title));
            match to {
                Section::Today => {
                    args.insert("scheduled".to_owned(), json!(crate::ymd(crate::today())));
                }
                Section::Waiting => {
                    args.insert("status".to_owned(), json!("waiting"));
                }
                Section::Upcoming | Section::Inbox => {}
            }
            let made = crate::create(Value::Object(args)).await?;
            Ok(made
                .pointer("/task/path")
                .or_else(|| made.get("path"))
                .and_then(Value::as_str)
                .map_or(Open::Nothing, |p| Open::File { path: p.to_owned() }))
        })
    }
}

/// Puts task `path` in section `to`.
async fn place(path: &str, to: Section) -> Result<(), String> {
    match to {
        Section::Today => {
            crate::update(json!({ "path": path, "scheduled": crate::ymd(crate::today()) })).await?;
        }
        Section::Waiting => {
            set_status(path, "waiting").await?;
        }
        Section::Upcoming => {
            crate::update(json!({ "path": path, "scheduled": "" })).await?;
            set_status(path, "open").await?;
        }
        Section::Inbox => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn task(path: &str, values: Value) -> Task {
        let Value::Object(m) = values else {
            return Task::from_values(path, path, &Map::new());
        };
        Task::from_values(path, path, &m)
    }

    #[test]
    fn captures_and_tasks_are_one_queue_in_four_groups() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap_or_default();
        let captures = vec![
            json!({"path": "Inbox/Gift idea.md", "title": "Gift idea", "body": "Capo + strap", "source": "quick-add", "created": "2026-11-01T17:30:00"}),
            json!({"path": "Inbox/Voice memo.md", "title": "", "body": "Chorus melody\nhum", "source": "voice", "created": "2026-11-06T09:00:00"}),
        ];
        let tasks = vec![
            task(
                "Tasks/Mix.md",
                json!({"title": "Mix the album", "status": "open", "due": "2026-11-07"}),
            ),
            task(
                "Tasks/Late.md",
                json!({"title": "Invoice", "status": "open", "due": "2026-11-01"}),
            ),
            task(
                "Tasks/Master.md",
                json!({"title": "Master", "status": "open", "scheduled": "2026-11-20"}),
            ),
            task(
                "Tasks/Someday.md",
                json!({"title": "Strings", "status": "open"}),
            ),
            task(
                "Tasks/Wait.md",
                json!({"title": "Artwork", "status": "waiting"}),
            ),
            task("Tasks/Done.md", json!({"title": "Demo", "status": "done"})),
        ];
        let snap = snapshot_of(&captures, &tasks, today);
        assert!(snap.groups && snap.ordered);
        assert_eq!(snap.count, Some(2));
        let ids: Vec<&str> = snap.tree.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["inbox", "today", "upcoming", "waiting"]);
        let at = |title: &str| {
            snap.items
                .iter()
                .find(|i| i.title == title)
                .map(|i| i.container.as_str())
        };
        assert_eq!(at("Gift idea"), Some("inbox"));
        // A capture with no title: its body's first line.
        assert_eq!(at("Chorus melody"), Some("inbox"));
        assert_eq!(at("Mix the album"), Some("today"));
        assert_eq!(at("Invoice"), Some("today"));
        assert_eq!(at("Master"), Some("upcoming"));
        assert_eq!(at("Strings"), Some("upcoming"));
        assert_eq!(at("Artwork"), Some("waiting"));
        assert_eq!(at("Demo"), None, "done is not waiting on you");
        let late = snap.items.iter().find(|i| i.title == "Invoice");
        assert!(late.is_some_and(|i| i.badges.contains(&"overdue".to_owned())));
        // A capture opens the triage card, with its path.
        let gift = snap.items.iter().find(|i| i.title == "Gift idea");
        assert_eq!(
            gift.and_then(|i| i.open.clone()),
            Some(Open::Action {
                id: PROCESS_CARD.to_owned(),
                args: json!({ "path": "Inbox/Gift idea.md" }),
            })
        );
        assert_eq!(
            gift.and_then(|i| i.preview.clone()).as_deref(),
            Some("quick-add · Capo + strap")
        );
    }
}
