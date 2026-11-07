//! Its navigator provider (`[[navigator_providers]] id = "calendar"`):
//! the Calendar section of the navigator's left list (#444), as Notebook
//! Navigator draws a calendar.
//!
//! - **Left:** a month grid under the Calendar row (`Snapshot::calendar`:
//!   each day's dots and its day template's mark), and under it the
//!   periodic tree, Year → Quarter → Month → Week → Day, from the month
//!   before this one to two after.
//! - **Right:** the period selected, its days' contents by day and by
//!   time: events, tasks due or scheduled, tracked time, the day's plan
//!   (Day Plans', when it is there) and the period's note.
//! - **Open** one: its note; the plan opens the day view; the note opens
//!   or makes the daily, weekly, … note (`NAVIGATOR_OPEN_*_NOTE`).
//! - **Drag** an event or a task onto a day (in the tree or the grid): it
//!   moves there, its time kept (an occurrence of a series: that one
//!   alone; a task: `CALENDAR_MOVE_TASK`).
//! - **New** in a period: an all-day event on its first day.
//!
//! It implements `architect_navigator_api::NavigatorProvider`, the MIT
//! API: this crate never links the GPL navigator. `Builder::navigator`
//! serves it as its `CALENDAR_NAV_*` actions, linked in or as a process.

use std::collections::{BTreeMap, BTreeSet};

use architect_navigator_api::{
    CalendarGrid, GridDay, Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot,
};
use chrono::{Datelike as _, Duration, Local, Months, NaiveDate, NaiveDateTime, Weekday};
use serde_json::json;

use crate::layout::{midnight, week_start};
use crate::model::{self, Block, Source};
use crate::plan::PlanDay;

/// Its actions' prefix (`[[navigator_providers]] actions`).
pub const ACTIONS: &str = "CALENDAR_NAV";

/// The provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalendarNav;

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

/// The first of `d`'s month.
fn month_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// What the tree reaches: from the week holding the first of last month
/// to the end of the week holding the last of the month after next.
#[must_use]
pub fn window(today: NaiveDate, first: Weekday) -> (NaiveDate, NaiveDate) {
    let this = month_of(today);
    let from = this.checked_sub_months(Months::new(1)).unwrap_or(this);
    let end = this.checked_add_months(Months::new(3)).unwrap_or(this);
    let to = week_start(end - Duration::days(1), first) + Duration::days(7);
    (week_start(from, first), to)
}

/// A day's node id: `d:2026-11-09`.
#[must_use]
pub fn day_node(d: NaiveDate) -> String {
    format!("d:{d}")
}

/// The day a node starts on, and its kind's letter: `d:2026-11-09`,
/// `w:2026-11-09`, `m:2026-11`, `q:2026-4`, `y:2026`.
#[must_use]
pub fn node_start(id: &str) -> Option<(char, NaiveDate)> {
    let (kind, rest) = id.split_once(':')?;
    let kind = kind.chars().next()?;
    let date = match kind {
        'd' | 'w' => rest.parse().ok()?,
        'm' => format!("{rest}-01").parse().ok()?,
        'q' => {
            let (y, q) = rest.split_once('-')?;
            let q: u32 = q.parse().ok()?;
            NaiveDate::from_ymd_opt(y.parse().ok()?, q.checked_sub(1)? * 3 + 1, 1)?
        }
        'y' => NaiveDate::from_ymd_opt(rest.parse().ok()?, 1, 1)?,
        _ => return None,
    };
    Some((kind, date))
}

/// The periodic tree over `from..to` (whole weeks): years, their
/// quarters, months, weeks (each in the month its fourth day is in) and
/// days.
#[must_use]
pub fn tree(from: NaiveDate, to: NaiveDate, first: Weekday, today: NaiveDate) -> Vec<Node> {
    // (year, quarter, month) → its weeks' first days.
    let mut months: BTreeMap<(i32, u32, u32), Vec<NaiveDate>> = BTreeMap::new();
    let mut start = week_start(from, first);
    while start < to {
        let anchor = start + Duration::days(3);
        let q = (anchor.month() - 1) / 3 + 1;
        months
            .entry((anchor.year(), q, anchor.month()))
            .or_default()
            .push(start);
        start += Duration::days(7);
    }
    let mut years: Vec<Node> = Vec::new();
    for ((y, q, m), weeks) in months {
        let month_start = NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(from);
        let mut month = Node::new(
            format!("m:{y}-{m:02}"),
            month_start.format("%B").to_string(),
        )
        .icon("calendar");
        for w in weeks {
            let anchor = w + Duration::days(3);
            let last = w + Duration::days(6);
            let first_day = if w.month() == last.month() {
                w.format("%-d").to_string()
            } else {
                w.format("%-d %b").to_string()
            };
            let mut week = Node::new(
                format!("w:{w}"),
                format!(
                    "Week {} · {first_day}–{}",
                    anchor.iso_week().week(),
                    last.format("%-d %b")
                ),
            )
            .icon("calendar-range");
            for d in w.iter_days().take(7) {
                let title = if d == today {
                    format!("{} · Today", d.format("%a %-d %b"))
                } else {
                    d.format("%a %-d %b").to_string()
                };
                week = week.child(Node::new(day_node(d), title).icon(if d == today {
                    "sun"
                } else {
                    "calendar-days"
                }));
            }
            month = month.child(week);
        }
        let year_id = format!("y:{y}");
        if years.last().is_none_or(|n| n.id != year_id) {
            years.push(Node::new(year_id, y.to_string()).icon("calendar-clock"));
        }
        let Some(year) = years.last_mut() else {
            continue;
        };
        let q_id = format!("q:{y}-{q}");
        if year.children.last().is_none_or(|n| n.id != q_id) {
            year.children
                .push(Node::new(q_id, format!("Q{q}")).icon("calendar-range"));
        }
        if let Some(quarter) = year.children.last_mut() {
            quarter.children.push(month);
        }
    }
    years
}

fn millis(t: NaiveDateTime) -> i64 {
    model::fixed(&Local, t).timestamp_millis()
}

/// A length in words: `1h 30m`, `45m`.
fn length(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// A block (an occurrence, a task, tracked time) as an item on its day.
fn block_item(b: &Block, day: NaiveDate) -> Item {
    let container = day_node(day);
    let at = b.start.format("%Y-%m-%dT%H:%M");
    let (id, kind) = match &b.source {
        Source::Event { path, .. } => (format!("e:{path}#{at}"), "event"),
        Source::Booking { path, .. } => (format!("b:{path}"), "booking"),
        Source::Task { path, .. } => (format!("t:{path}"), "task"),
        Source::Time { path, .. } => (format!("time:{path}#{at}"), "time"),
    };
    let mut item = Item::new(&id, &container, &b.title)
        .dates(millis(b.start), millis(b.start))
        .property("kind", kind)
        .property("color", &b.color)
        .opens(Open::File {
            path: b.source.path().to_owned(),
        });
    item = match &b.source {
        Source::Task { done, field, .. } => {
            let mut i = item
                .icon(if *done { "square-check" } else { "square" })
                .badge(field);
            i.progress = Some(architect_navigator_api::Progress {
                done: u32::from(*done),
                total: 1,
            });
            i.preview(format!("Task, {field}"))
        }
        Source::Time { running, .. } => {
            let mins = (b.end - b.start).num_minutes();
            item.icon("timer")
                .badge(if *running {
                    "running".to_owned()
                } else {
                    length(mins)
                })
                .preview(format!(
                    "Tracked {}–{}",
                    b.start.format("%H:%M"),
                    b.end.format("%H:%M")
                ))
        }
        Source::Event { .. } | Source::Booking { .. } => {
            let mut preview = when_words(b);
            if let Some(r) = &b.repeats {
                preview = format!("{r} · {preview}");
            }
            if let Some(l) = &b.location {
                preview = format!("{preview} · {l}");
            }
            let item = item.icon("calendar").preview(preview);
            if b.in_strip() {
                item.badge("all day")
            } else {
                item.badge(&b.start.format("%H:%M").to_string())
            }
        }
    };
    item
}

/// A day's plan as an item: its template and what its blocks are given
/// to; opens the day view.
fn plan_item(p: &PlanDay) -> Item {
    let title = p
        .template
        .as_deref()
        .map_or_else(|| "Day plan".to_owned(), |t| format!("Day plan · {t}"));
    let mut item = Item::new(format!("plan:{}", p.date), day_node(p.date), title)
        .dates(millis(midnight(p.date)), millis(midnight(p.date)))
        .icon("layout-list")
        .property("kind", "plan")
        .preview(format!("{} · {}", p.summary(), p.reason))
        .opens(Open::Action {
            id: "CALENDAR_OPEN".into(),
            args: json!({ "view": "day", "date": p.date.to_string() }),
        });
    if p.edited {
        item = item.badge("edited");
    }
    if p.chosen {
        item = item.badge("override");
    }
    item
}

/// A period's note as an item: the daily, weekly, … note, opened or made
/// by the navigator.
fn note_item(node: &str, kind: char, start: NaiveDate) -> Option<Item> {
    let (name, action) = match kind {
        'd' => ("Daily note", "NAVIGATOR_OPEN_DAILY_NOTE"),
        'w' => ("Weekly note", "NAVIGATOR_OPEN_WEEKLY_NOTE"),
        'm' => ("Monthly note", "NAVIGATOR_OPEN_MONTHLY_NOTE"),
        'q' => ("Quarterly note", "NAVIGATOR_OPEN_QUARTERLY_NOTE"),
        'y' => ("Yearly note", "NAVIGATOR_OPEN_YEARLY_NOTE"),
        _ => return None,
    };
    Some(
        Item::new(format!("note:{node}"), node, name)
            .dates(millis(midnight(start)), millis(midnight(start)))
            .icon("notebook-pen")
            .property("kind", "note")
            .preview("Open it, or make it from its template")
            .opens(Open::Action {
                id: action.into(),
                args: json!({ "date": start.to_string() }),
            }),
    )
}

/// The Calendar section: the periodic tree over the window around
/// `now`, its days' contents (`blocks`: occurrences, tasks, tracked time;
/// `plans`), the periods' notes, and the month grid.
#[must_use]
pub fn snapshot_of(
    blocks: &[Block],
    plans: &[PlanDay],
    now: NaiveDateTime,
    first: Weekday,
) -> Snapshot {
    let today = now.date();
    let (from, to) = window(today, first);
    let mut snap = Snapshot::new("Calendar").ordered();
    snap.tree = tree(from, to, first, today);
    let mut busy: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    let mut with_items: BTreeSet<NaiveDate> = BTreeSet::new();
    for b in blocks {
        let day = b.start.date().max(from);
        if day >= to {
            continue;
        }
        snap.items.push(block_item(b, day));
        with_items.insert(day);
        if !matches!(b.source, Source::Time { .. }) {
            *busy.entry(day).or_default() += 1;
        }
    }
    let mut grid = CalendarGrid::default();
    let plan_of: BTreeMap<NaiveDate, &PlanDay> = plans.iter().map(|p| (p.date, p)).collect();
    for p in plans.iter().filter(|p| p.date >= from && p.date < to) {
        snap.items.push(plan_item(p));
    }
    let this_week = week_start(today, first);
    let ids: Vec<String> = snap
        .nodes()
        .into_iter()
        .map(|(_, n)| n.id.clone())
        .collect();
    for id in ids {
        let Some((kind, start)) = node_start(&id) else {
            continue;
        };
        if kind == 'd' {
            grid.days.insert(
                start.to_string(),
                GridDay {
                    node: id.clone(),
                    count: busy.get(&start).copied().unwrap_or(0),
                    label: plan_of.get(&start).and_then(|p| p.template.clone()),
                    colour: plan_of.get(&start).and_then(|p| p.color.clone()),
                },
            );
            // A day's note where something is on, and this week's.
            let near = (this_week..this_week + Duration::days(7)).contains(&start);
            if !(near || with_items.contains(&start)) {
                continue;
            }
        }
        snap.items.extend(note_item(&id, kind, start));
    }
    snap.calendar = Some(grid);
    // By time; at a day's start its note, then its plan, then the rest
    // (a stable sort over them put first).
    let rank = |i: &Item| match i.id.split(':').next() {
        Some("note") => 0,
        Some("plan") => 1,
        _ => 2,
    };
    snap.items.sort_by_key(|i| (i.created, rank(i)));
    snap
}

impl NavigatorProvider for CalendarNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            crate::refresh().await?;
            let now = Local::now().naive_local();
            let first = crate::settings().week_start;
            let (from, to) = window(now.date(), first);
            // The plans of the weeks around today (Day Plans answers 62
            // days at most).
            let today = now.date();
            crate::refresh_plans(
                week_start(today, first) - Duration::days(21),
                week_start(today, first) + Duration::days(35),
            )
            .await;
            let blocks = crate::blocks(midnight(from), midnight(to));
            let plans = crate::plans(from, to);
            Ok(snapshot_of(&blocks, &plans, now, first))
        })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        Box::pin(async move {
            let Some(('d', day)) = node_start(&to) else {
                return Err("drop an event or a task on a day to move it there".into());
            };
            let mut moved = 0;
            for id in items {
                if let Some(path) = id.strip_prefix("t:") {
                    crate::move_task(json!({ "path": path, "date": day.to_string() })).await?;
                    moved += 1;
                    continue;
                }
                let Some((path, at)) = id.strip_prefix("e:").and_then(|r| r.rsplit_once('#'))
                else {
                    continue;
                };
                let Ok(at) = NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M") else {
                    continue;
                };
                let Some(e) = crate::events().into_iter().find(|e| e.path == path) else {
                    continue;
                };
                let occurrences = model::occurrences(
                    std::slice::from_ref(&e),
                    at,
                    at + Duration::minutes(1),
                    &Local,
                );
                let Some(b) = occurrences.into_iter().find(|b| b.start == at) else {
                    continue;
                };
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
                moved += 1;
            }
            if moved == 0 {
                return Err("only events and tasks move to a day".into());
            }
            Ok(())
        })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let day = node_start(&container).map_or_else(crate::today, |(_, d)| d);
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
    use crate::model::Event;

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

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn the_tree_is_years_quarters_months_weeks_and_days() {
        let (from, to) = window(d("2026-11-07"), Weekday::Mon);
        assert_eq!((from, to), (d("2026-09-28"), d("2027-02-01")));
        let t = tree(from, to, Weekday::Mon, d("2026-11-07"));
        let years: Vec<&str> = t.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(years, ["2026", "2027"]);
        let q4 = &t[0].children.last().unwrap();
        assert_eq!(q4.title, "Q4");
        let months: Vec<&str> = q4.children.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(months, ["October", "November", "December"]);
        // Week 45 (2–8 Nov) is November's: its Thursday is.
        let nov = &q4.children[1];
        assert_eq!(nov.children[0].title, "Week 45 · 2–8 Nov");
        let days: Vec<&str> = nov.children[0]
            .children
            .iter()
            .map(|n| n.title.as_str())
            .collect();
        assert_eq!(days[5], "Sat 7 Nov · Today");
        assert_eq!(nov.children[0].children[0].id, "d:2026-11-02");
        assert_eq!(node_start("q:2026-4"), Some(('q', d("2026-10-01"))));
        assert_eq!(node_start("m:2026-11"), Some(('m', d("2026-11-01"))));
    }

    #[test]
    fn a_days_list_has_its_events_plan_and_note_and_the_grid_its_dots() {
        let now = d("2026-11-06").and_hms_opt(12, 0, 0).unwrap();
        let at = |days: i64, h: u32| {
            (now.date() + Duration::days(days))
                .and_hms_opt(h, 0, 0)
                .unwrap()
        };
        let events = [
            event("Now.md", at(0, 15), None),
            event("Weekly.md", at(-14, 10), Some("FREQ=WEEKLY")),
        ];
        let blocks = model::occurrences(
            &events,
            midnight(d("2026-09-28")),
            midnight(d("2027-02-01")),
            &Local,
        );
        let plan = PlanDay {
            date: now.date(),
            template: Some("Workday".into()),
            reason: "Rule 3: Mon–Fri".into(),
            color: Some("#3b82f6".into()),
            ..PlanDay::default()
        };
        let snap = snapshot_of(&blocks, &[plan], now, Weekday::Mon);
        let today: Vec<(&str, &str)> = snap
            .items
            .iter()
            .filter(|i| i.container == "d:2026-11-06")
            .map(|i| (i.id.as_str(), i.title.as_str()))
            .collect();
        assert_eq!(
            today,
            [
                ("note:d:2026-11-06", "Daily note"),
                ("plan:2026-11-06", "Day plan · Workday"),
                ("e:Weekly.md#2026-11-06T10:00", "Weekly"),
                ("e:Now.md#2026-11-06T15:00", "Now"),
            ]
        );
        let grid = snap.calendar.as_ref().unwrap();
        let day = &grid.days["2026-11-06"];
        assert_eq!((day.count, day.label.as_deref()), (2, Some("Workday")));
        assert_eq!(grid.days["2026-11-13"].count, 1, "the series' next");
        assert!(snap.items.iter().any(|i| i.id == "note:w:2026-11-02"));
        assert!(snap.ordered);
    }
}
