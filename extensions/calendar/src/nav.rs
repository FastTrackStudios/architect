//! Its navigator provider (`[[navigator_providers]] id = "calendar"`).
//!
//! When events come, on the left (Today, Tomorrow, This week, Later,
//! Repeating, Past); each event in its own on the right, by when it next
//! falls, with its time and place.
//!
//! It implements `architect_navigator_api::NavigatorProvider`, the MIT
//! API: this crate never links the GPL navigator. `Builder::navigator`
//! serves it as its `CALENDAR_NAV_*` actions, linked in or as a process.
//!
//! - **Open** one: its note.
//! - **Drag** one onto Today or Tomorrow: it moves there, its time kept
//!   (an occurrence of a series: that one alone).
//! - **New** in a node: an all-day event on its first day.

use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use serde_json::json;

use crate::layout::midnight;
use crate::model::{self, Block, Event, Source};

/// Its actions' prefix (`[[navigator_providers]] actions`).
pub const ACTIONS: &str = "CALENDAR_NAV";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalendarNav;

/// Its nodes: id, title, icon.
const NODES: [(&str, &str, &str); 6] = [
    ("today", "Today", "calendar-check"),
    ("tomorrow", "Tomorrow", "sunrise"),
    ("week", "This week", "calendar-range"),
    ("later", "Later", "calendar-clock"),
    ("repeating", "Repeating", "repeat"),
    ("past", "Past", "history"),
];

/// How far ahead it looks for a series' next.
const AHEAD: i64 = 400;

/// When `e` next falls from `now` (the one under way, else the next),
/// else when it last did.
fn next_of(e: &Event, now: NaiveDateTime) -> Option<Block> {
    let ahead = model::occurrences(
        std::slice::from_ref(e),
        now,
        now + Duration::days(AHEAD),
        &Local,
    );
    ahead.into_iter().next().or_else(|| {
        let start = e.start.local(&Local);
        model::occurrences(std::slice::from_ref(e), start, now, &Local)
            .into_iter()
            .last()
    })
}

/// The node an event falls in.
fn node_of(e: &Event, next: Option<&Block>, today: NaiveDate) -> &'static str {
    if e.repeats() && e.recurrence_id.is_none() {
        return "repeating";
    }
    let Some(b) = next else { return "past" };
    let day = b.start.date();
    if b.end <= Local::now().naive_local() && day < today {
        "past"
    } else if day <= today {
        "today"
    } else if day == today + Duration::days(1) {
        "tomorrow"
    } else if day < today + Duration::days(7) {
        "week"
    } else {
        "later"
    }
}

/// When a block is, in words: `Fri 6 Nov · 14:00–15:00`.
#[must_use]
pub fn when_words(b: &Block) -> String {
    let day = b.start.format("%a %-d %b");
    if b.in_strip() {
        let last = (b.end - Duration::seconds(1)).date();
        if last > b.start.date() {
            return format!("{day} – {}", last.format("%a %-d %b"));
        }
        return format!("{day} · all day");
    }
    format!(
        "{day} · {}–{}",
        b.start.format("%H:%M"),
        b.end.format("%H:%M")
    )
}

/// The events, each in its node.
#[must_use]
pub fn snapshot_of(events: &[Event], now: NaiveDateTime) -> Snapshot {
    let today = now.date();
    // Its periods are the list's groups (#443): the Calendar row lists
    // every event under Today, Tomorrow, … (#444's month grid replaces it).
    let mut snap = Snapshot::new("Calendar").grouped();
    for (id, title, icon) in NODES {
        snap = snap.node(Node::new(id, title).icon(icon));
    }
    let millis = |t: NaiveDateTime| model::fixed(&Local, t).timestamp_millis();
    snap.items = events
        .iter()
        .filter(|e| e.tombstone.is_none())
        .map(|e| {
            let next = next_of(e, now);
            let node = node_of(e, next.as_ref(), today);
            let start = next
                .as_ref()
                .map_or_else(|| e.start.local(&Local), |b| b.start);
            let mut preview = next.as_ref().map(when_words).unwrap_or_default();
            if let Some(r) = e.recurrence.as_deref() {
                preview = format!("{} · {preview}", crate::recur::describe(r));
            }
            if let Some(l) = &e.location {
                preview = format!("{preview} · {l}");
            }
            let mut item = Item::new(&e.path, node, &e.title)
                .dates(millis(start), millis(start))
                .preview(preview)
                .property("color", &e.color)
                .opens(Open::File {
                    path: e.path.clone(),
                });
            if let Some(b) = &next
                && !b.in_strip()
            {
                item = item.badge(&b.start.format("%H:%M").to_string());
            }
            item
        })
        .collect();
    // The soonest first.
    snap.items.sort_by_key(|i| i.created);
    snap
}

/// The day a node is.
fn node_day(node: &str, today: NaiveDate) -> Option<NaiveDate> {
    match node {
        "today" | "repeating" => Some(today),
        "tomorrow" => Some(today + Duration::days(1)),
        "week" => Some(today + Duration::days(2)),
        "later" => Some(today + Duration::days(7)),
        _ => None,
    }
}

impl NavigatorProvider for CalendarNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            let events = crate::refresh().await?;
            Ok(snapshot_of(&events, Local::now().naive_local()))
        })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            let today = Local::now().date_naive();
            let day = match to.as_str() {
                "today" | "tomorrow" => node_day(&to, today),
                _ => None,
            }
            .ok_or("drop an event on Today or Tomorrow to move it there")?;
            let now = Local::now().naive_local();
            for path in items {
                let Some(e) = crate::events().into_iter().find(|e| e.path == path) else {
                    continue;
                };
                let Some(b) = next_of(&e, now) else { continue };
                let start = day.and_time(b.start.time());
                let mut args = json!({
                    "path": e.path,
                    "start": model::when_text(&Local, start, b.all_day),
                    "end": model::when_text(&Local, start + (b.end - b.start), b.all_day),
                });
                if let Source::Event {
                    series: true,
                    date,
                    master: None,
                    ..
                } = b.source
                {
                    args["occurrence"] = json!(date.to_string());
                    args["scope"] = json!("this");
                }
                crate::reschedule(args).await?;
            }
            Ok(())
        })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let today = Local::now().date_naive();
            let day = node_day(&container, today).unwrap_or(today);
            let made =
                crate::create(json!({ "title": title, "start": midnight(day).date().to_string() }))
                    .await?;
            let path = made["path"].as_str().unwrap_or_default().to_owned();
            Ok(Open::File { path })
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::*;

    fn event(path: &str, start: NaiveDateTime, rule: Option<&str>) -> Event {
        let mut v = Map::new();
        v.insert(
            "start".into(),
            json!(model::when_text(&Local, start, false)),
        );
        v.insert(
            "end".into(),
            json!(model::when_text(&Local, start + Duration::hours(1), false)),
        );
        if let Some(r) = rule {
            v.insert("recurrence".into(), Value::String(r.into()));
        }
        Event::from_values(path, &v, &Local).unwrap()
    }

    #[test]
    fn events_fall_in_when_they_come() {
        let now = NaiveDate::from_ymd_opt(2026, 11, 6)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let at = |days: i64, h: u32| {
            (now.date() + Duration::days(days))
                .and_hms_opt(h, 0, 0)
                .unwrap()
        };
        let snap = snapshot_of(
            &[
                event("Later.md", at(30, 9), None),
                event("Now.md", at(0, 15), None),
                event("Tomorrow.md", at(1, 9), None),
                event("Past.md", at(-3, 9), None),
                event("Weekly.md", at(-14, 10), Some("FREQ=WEEKLY")),
            ],
            now,
        );
        assert_eq!(snap.tree.len(), 6);
        let by: Vec<(&str, &str)> = snap
            .items
            .iter()
            .map(|i| (i.id.as_str(), i.container.as_str()))
            .collect();
        assert_eq!(
            by,
            [
                ("Past.md", "past"),
                ("Now.md", "today"),
                ("Tomorrow.md", "tomorrow"),
                ("Weekly.md", "repeating"),
                ("Later.md", "later")
            ]
        );
        let weekly = snap.items.iter().find(|i| i.id == "Weekly.md").unwrap();
        assert!(
            weekly
                .preview
                .as_deref()
                .unwrap()
                .starts_with("Weekly · Fri 13 Nov · 10:00–11:00"),
            "{:?}",
            weekly.preview
        );
        assert_eq!(weekly.badges, ["10:00"]);
    }
}
