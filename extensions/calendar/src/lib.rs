//! The Calendar (`extensions/calendar/extension.toml`): events as vault
//! notes, Task's calendar (`task:features/scheduling`,
//! `task:features/view/view-calendar`) ported as an extension (#350,
//! #375), with the navigator's calendar and periodic notes (#336) a
//! click away.
//!
//! One [`Calendar`] value runs it two ways: the app links it in
//! (`Extensions::with_linked`), and its program (`src/main.rs`) serves the
//! same value as a process (`Guest::extension`), so `EXT_CLONE {"id":
//! "calendar"}` gives a development copy that overrides the linked one.
//!
//! - **Data:** an event is a note with `start` and `end` in its
//!   frontmatter, Task's `scheduling-cal-event` ([`model`]). A series has
//!   an RRULE ([`recur`]) and exceptions: skipped days (`exdates`) and
//!   overrides, notes of their own that replace one occurrence. It reads
//!   them with one `BASES_ROWS` query and writes them with `VAULT_*`.
//! - **Views:** day, week, month and agenda, with drag to move and resize
//!   and a sweep to make one ([`ui`]), and two view types for Bases
//!   (`[[base_views]]`): `calendar` (a month of a base's notes by a date
//!   property) and `timeline` (a schedule of them as bars).
//! - **On it:** tasks on their scheduled or due day, through
//!   `architect.tasks` (`[uses]`), and tracked time as blocks, through
//!   `architect.time`. A day's heading opens its daily note through the
//!   navigator's `NAVIGATOR_OPEN_DAILY_NOTE`.
//! - **Service:** it provides `architect.calendar`, each method its
//!   `CALENDAR_<METHOD>` action: Task's `list_events`, `create_event`,
//!   `update_event`, `reschedule_event` and `cancel_event`, and more.
//! - **Booking** ([`booking`]): event types, the hours they are open,
//!   open slots and bookings, Task's `list_open_slots` and `book_slot`.
//! - **Subscriptions** ([`sync`], [`ics`]): iCalendar feeds, read only,
//!   their events notes synced by `external_ref`.
//! - **Also:** a capture target (`event`), a navigator provider ([`nav`])
//!   and a tour.

// Its views and service calls read JSON with `value["key"]`: serde_json's
// index answers `Null` for what is missing, never a panic. Spans of
// minutes and days stay far inside i64 and chrono's range.
#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

pub mod booking;
pub mod ics;
pub mod layout;
pub mod model;
pub mod nav;
pub mod quick;
pub mod recur;
pub mod sync;
pub mod ui;
pub mod vault;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::linked::{Builder, Extension};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc, Weekday};
use serde_json::{Map, Value, json};

use crate::layout::Mode;
use crate::model::{Block, Event, When, keys};
use crate::recur::Rule;

/// Its id: its actions are `CALENDAR_*`.
pub const ID: &str = "calendar";

/// The service it provides (`[provides]`): method `m` is `CALENDAR_<M>`.
pub const SERVICE: &str = "architect.calendar";

/// The services it uses when they are there.
const TASKS: &str = "architect.tasks";
const TIME: &str = "architect.time";

/// Its main view.
pub const VIEW: &str = "calendar";

/// Where new events go unless `calendar.folder` says otherwise; events
/// are read wherever they are.
pub const FOLDER: &str = "Calendar";

/// The bases `CALENDAR_BASE` writes, in the folder.
pub const CALENDAR_BASE: &str = "Calendar.base";
pub const TIMELINE_BASE: &str = "Timeline.base";

/// Its settings, as last read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub folder: String,
    pub week_start: Weekday,
    /// A new event's length, in minutes.
    pub event_minutes: i64,
    /// The hour the day and week views open at.
    pub day_starts: u32,
    pub show_tasks: bool,
    pub show_time: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: FOLDER.to_owned(),
            week_start: Weekday::Mon,
            event_minutes: 60,
            day_starts: 7,
            show_tasks: true,
            show_time: true,
        }
    }
}

/// What it knows of the vault's events, as last read.
#[derive(Debug, Default)]
struct Cache {
    events: Vec<Event>,
    tasks: Vec<Block>,
    time: Vec<Block>,
    /// Booking's notes ([`booking`]).
    event_types: Vec<booking::EventType>,
    schedules: Vec<booking::Schedule>,
    bookings: Vec<booking::Booking>,
    /// Properties the vault has besides an event's own: read too, as a
    /// link can be under any key.
    extra_keys: Vec<String>,
    /// The Tasks and the Time Tracker answered.
    tasks_found: bool,
    time_found: bool,
    settings: Option<Settings>,
    error: Option<String>,
    read: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    events: Vec::new(),
    tasks: Vec::new(),
    time: Vec::new(),
    event_types: Vec::new(),
    schedules: Vec::new(),
    bookings: Vec::new(),
    extra_keys: Vec::new(),
    tasks_found: false,
    time_found: false,
    settings: None,
    error: None,
    read: false,
});

/// What the calendar view shows: its mode and the day it is around. The
/// view and `CALENDAR_OPEN` share it, so an action can turn its pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub mode: Mode,
    pub cursor: NaiveDate,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

/// Moves on with every read and write, for the views.
static REVISION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

#[must_use]
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// What shows, today's week until something says otherwise.
#[must_use]
pub fn shown() -> Shown {
    SHOWN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or(Shown {
            mode: Mode::Week,
            cursor: today(),
        })
}

pub(crate) fn show(s: Shown) {
    *SHOWN.lock().unwrap_or_else(PoisonError::into_inner) = Some(s);
    bump();
}

/// Its settings (the defaults until read).
#[must_use]
pub fn settings() -> Settings {
    cache().settings.clone().unwrap_or_default()
}

/// The events, as last read.
#[must_use]
pub fn events() -> Vec<Event> {
    cache().events.clone()
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A whole number, given as one or as its text (the CLI's `--arg`).
fn int_arg(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key)?;
    v.as_i64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// A flag, given as one or as its text.
fn bool_arg(args: &Value, key: &str) -> Option<bool> {
    let v = args.get(key)?;
    v.as_bool().or_else(|| v.as_str()?.trim().parse().ok())
}

fn day_arg(args: &Value, key: &str) -> Result<Option<NaiveDate>, String> {
    arg(args, key)
        .map(|d| match d {
            "today" => Ok(today()),
            "tomorrow" => Ok(today() + Duration::days(1)),
            d => NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map_err(|_| format!("{key} {d:?}: a date, YYYY-MM-DD")),
        })
        .transpose()
}

/// The base that finds every event: Task's types, each property it reads.
#[must_use]
pub fn events_base(extra: &[String]) -> String {
    let mut out = String::from("filters:\n  or:\n");
    for t in model::TYPES {
        let _ = writeln!(out, "    - type == \"{t}\"");
    }
    out.push_str("views:\n  - type: table\n    name: Events\n    order:\n");
    for k in keys::RESERVED
        .iter()
        .map(|k| (*k).to_owned())
        .chain(extra.iter().cloned())
    {
        let _ = writeln!(out, "      - {k}");
    }
    out
}

/// A base of events (and anything else with a `start`) for a view type
/// of its own: `calendar` or `timeline`.
#[must_use]
pub fn view_base(kind: &str) -> String {
    let name = if kind == "timeline" {
        "Timeline"
    } else {
        "Calendar"
    };
    let mut out = String::from("filters:\n  or:\n");
    for t in model::TYPES {
        let _ = writeln!(out, "    - type == \"{t}\"");
    }
    let _ = write!(
        out,
        "views:\n  - type: {kind}\n    name: {name}\n    order:\n      - file.name\n      - title\n      - start\n      - end\n      - all_day\n      - color\n    startBy: start\n    endBy: end\n"
    );
    out
}

/// Each row's column names that are not an event's own nor a file's.
fn extras_of(out: &Value) -> Vec<String> {
    let mut extra: Vec<String> = out
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|k| {
            !k.starts_with("file.") && !k.starts_with("formula.") && !keys::RESERVED.contains(k)
        })
        .filter(|k| {
            k.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
        .map(str::to_owned)
        .collect();
    extra.sort();
    extra.dedup();
    extra
}

async fn read_settings() -> Settings {
    let d = Settings::default();
    let text = |v: Value| {
        v.as_str()
            .map(str::to_owned)
            .filter(|s| !s.trim().is_empty())
    };
    let int = |v: &Value| v.as_i64().or_else(|| v.as_str()?.parse().ok());
    let flag = |v: &Value, d: bool| v.as_bool().unwrap_or(d);
    let week = vault::setting("calendar.week_start").await;
    Settings {
        folder: text(vault::setting("calendar.folder").await)
            .map(|f| f.trim_matches('/').to_owned())
            .unwrap_or(d.folder),
        week_start: match week.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("sunday") => Weekday::Sun,
            Some("saturday") => Weekday::Sat,
            _ => Weekday::Mon,
        },
        event_minutes: int(&vault::setting("calendar.event_minutes").await)
            .filter(|m| (5..=24 * 60).contains(m))
            .unwrap_or(d.event_minutes),
        day_starts: int(&vault::setting("calendar.day_starts").await)
            .and_then(|h| u32::try_from(h).ok())
            .filter(|h| *h < 24)
            .unwrap_or(d.day_starts),
        show_tasks: flag(&vault::setting("calendar.show_tasks").await, true),
        show_time: flag(&vault::setting("calendar.show_time").await, true),
    }
}

/// Reads the vault's events again, and the tasks and time on the
/// calendar from their extensions, when they are there.
pub async fn refresh() -> Result<Vec<Event>, String> {
    let settings = read_settings().await;
    let extra = cache().extra_keys.clone();
    let mut read = vault::run("BASES_ROWS", json!({ "source": events_base(&extra) })).await;
    if let Ok(out) = &read {
        let now = extras_of(out);
        if now != extra {
            cache().extra_keys.clone_from(&now);
            read = vault::run("BASES_ROWS", json!({ "source": events_base(&now) })).await;
        }
    }
    let read = read.map(|out| {
        vault::rows_of(&out)
            .into_iter()
            .filter_map(|r| Event::from_values(&r.path, &r.values, &Local))
            .collect::<Vec<_>>()
    });
    let (tasks, tasks_found) = if settings.show_tasks {
        match vault::service(TASKS, "list", json!({ "view": "all" })).await {
            Some(Ok(v)) => (
                v["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| model::task_block(t, &Local))
                    .collect(),
                true,
            ),
            _ => (Vec::new(), false),
        }
    } else {
        (Vec::new(), false)
    };
    let (time, time_found) = if settings.show_time {
        let around = shown().cursor;
        let args = json!({
            "from": (around - Duration::days(62)).to_string(),
            "to": (around + Duration::days(62)).to_string(),
        });
        let now = Local::now().naive_local();
        match vault::service(TIME, "entries", args).await {
            Some(Ok(v)) => (
                v["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| model::time_block(e, now, &Local))
                    .collect(),
                true,
            ),
            _ => (Vec::new(), false),
        }
    } else {
        (Vec::new(), false)
    };
    let booked = booking::read().await.ok();
    {
        let mut c = cache();
        c.settings = Some(settings);
        c.tasks = tasks;
        c.time = time;
        c.tasks_found = tasks_found;
        c.time_found = time_found;
        if let Some((types, schedules, bookings)) = booked {
            c.event_types = types;
            c.schedules = schedules;
            c.bookings = bookings;
        }
        c.read = true;
        match &read {
            Ok(events) => {
                c.events.clone_from(events);
                c.error = None;
            }
            Err(e) => c.error = Some(e.clone()),
        }
    }
    bump();
    read
}

/// What the calendar draws over `[from, to)`: the events' occurrences,
/// and the tasks' and tracked time's blocks when they show.
#[must_use]
pub fn blocks(from: NaiveDateTime, to: NaiveDateTime) -> Vec<Block> {
    let (events, tasks, time, booked) = {
        let c = cache();
        (
            c.events.clone(),
            c.tasks.clone(),
            c.time.clone(),
            booking::blocks(&c.bookings, &c.event_types),
        )
    };
    let mut out = model::occurrences(&events, from, to, &Local);
    out.extend(booked.into_iter().filter(|b| b.overlaps(from, to)));
    out.extend(tasks.into_iter().filter(|b| b.overlaps(from, to)));
    out.extend(time.into_iter().filter(|b| b.overlaps(from, to)));
    model::sort(&mut out);
    out
}

/// The event `args` names by `path`, `id` or `external_ref`.
fn find(args: &Value) -> Result<Event, String> {
    let c = cache();
    let hit = if let Some(p) = arg(args, "path") {
        let p = p
            .strip_suffix(".md")
            .map_or_else(|| p.to_owned(), |s| format!("{s}.md"));
        c.events.iter().find(|e| e.path == p)
    } else if let Some(id) = arg(args, "id") {
        c.events
            .iter()
            .find(|e| e.id == id)
            .or_else(|| c.events.iter().find(|e| model::stem(&e.path) == id))
    } else if let Some(r) = arg(args, "external_ref") {
        c.events
            .iter()
            .find(|e| e.external_ref.as_deref() == Some(r))
    } else {
        return Err("name the event: its `path` or `id`".to_owned());
    };
    hit.cloned().ok_or_else(|| "no such event".to_owned())
}

/// A free path in `folder` for an event named `title` on `day`.
fn free_path(folder: &str, title: &str, day: NaiveDate) -> String {
    let c = cache();
    let base = model::file_name(title, day);
    let dir = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let taken = |p: &str| c.events.iter().any(|e| e.path.eq_ignore_ascii_case(p));
    let mut path = format!("{dir}{base}.md");
    let mut n = 2;
    while taken(&path) {
        path = format!("{dir}{base} {n}.md");
        n += 1;
    }
    path
}

/// A free `id` from `title`.
fn free_id(title: &str, day: NaiveDate) -> String {
    let c = cache();
    let base = format!("{}-{}", model::slug(title), day.format("%Y%m%d"));
    let mut id = base.clone();
    let mut n = 2;
    while c.events.iter().any(|e| e.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Writes `event` as a new note (refused when its path is taken: one
/// more number is tried).
async fn write_new(mut event: Event) -> Result<Event, String> {
    for _ in 0..5 {
        let props = event.properties();
        match vault::create(&event.path, &props, "").await {
            Ok(()) => {
                let _ = refresh().await;
                return Ok(event);
            }
            Err(e) if e.contains("taken") || e.contains("exists") => {
                let stem = model::stem(&event.path).to_owned();
                let dir = event
                    .path
                    .rsplit_once('/')
                    .map(|(d, _)| format!("{d}/"))
                    .unwrap_or_default();
                event.path = format!("{dir}{stem} {}.md", Utc::now().timestamp_subsec_millis());
            }
            Err(e) => return Err(e),
        }
    }
    Err("no free name for the event".to_owned())
}

/// The start and end `args` give (`start`, then `end`, or `minutes`),
/// all day when `start` is a date or `all_day` says so.
fn span_of(
    args: &Value,
    default_minutes: i64,
) -> Result<(NaiveDateTime, NaiveDateTime, bool), String> {
    let start = arg(args, "start").ok_or("give the event's `start`")?;
    let start_w = When::parse(start, &Local)
        .ok_or_else(|| format!("start {start:?}: a date or an RFC 3339 time"))?;
    let all_day = bool_arg(args, "all_day").unwrap_or(matches!(start_w, When::Day(_)));
    let s = if all_day {
        start_w.local(&Local).date().and_time(NaiveTime::MIN)
    } else {
        start_w.local(&Local)
    };
    let e = match (arg(args, "end"), int_arg(args, "minutes")) {
        (Some(end), _) => {
            let w = When::parse(end, &Local)
                .ok_or_else(|| format!("end {end:?}: a date or an RFC 3339 time"))?;
            let e = w.local(&Local);
            if all_day {
                // A date given as the last day is the day after it.
                let d = e.date();
                let d = if matches!(w, When::Day(_)) && d > s.date() {
                    d
                } else {
                    d.max(s.date()) + Duration::days(i64::from(d <= s.date()))
                };
                d.and_time(NaiveTime::MIN)
            } else {
                e
            }
        }
        (None, Some(m)) if m > 0 => s + Duration::minutes(m),
        _ if all_day => s + Duration::days(1),
        _ => s + Duration::minutes(default_minutes),
    };
    if e <= s {
        return Err("the end must come after the start".to_owned());
    }
    Ok((s, e, all_day))
}

fn links_of(args: &Value) -> Result<Option<Vec<model::Link>>, String> {
    let Some(list) = args.get("links").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for l in list {
        let relation = l["relation"].as_str().unwrap_or("for").to_owned();
        if !model::valid_relation(&relation) {
            return Err(format!(
                "relation {relation:?}: a lowercase slug that is not an event's own key"
            ));
        }
        let target = l["target"].as_str().ok_or("a link needs its `target`")?;
        out.push(model::Link {
            relation,
            target: model::wikilink(target),
        });
    }
    Ok(Some(out))
}

/// `CALENDAR_CREATE`: Task's `create_event`. An `external_ref` already
/// used answers that event (`existing: true`) and writes nothing.
pub async fn create(args: Value) -> Result<Value, String> {
    if cache().settings.is_none() || !cache().read {
        let _ = refresh().await;
    }
    let s = settings();
    if let Some(r) = arg(&args, "external_ref")
        .map(str::to_owned)
        .or_else(|| external_ref(&args))
        && let Some(e) = cache()
            .events
            .iter()
            .find(|e| e.external_ref.as_deref() == Some(r.as_str()))
    {
        let mut v = e.json(&Local);
        v["existing"] = json!(true);
        return Ok(v);
    }
    let title = arg(&args, "title")
        .ok_or("give the event's `title`")?
        .to_owned();
    let (start, end, all_day) = span_of(&args, s.event_minutes)?;
    if let Some(rule) = arg(&args, "recurrence")
        && Rule::parse(rule).is_none()
    {
        return Err(format!(
            "recurrence {rule:?}: an RRULE this reads (FREQ=DAILY|WEEKLY|MONTHLY|YEARLY …)"
        ));
    }
    let folder =
        arg(&args, "folder").map_or_else(|| s.folder.clone(), |f| f.trim_matches('/').to_owned());
    let event = Event {
        path: free_path(&folder, &title, start.date()),
        id: free_id(&title, start.date()),
        title,
        start: if all_day {
            When::Day(start.date())
        } else {
            When::At(model::fixed(&Local, start))
        },
        end: if all_day {
            When::Day(end.date())
        } else {
            When::At(model::fixed(&Local, end))
        },
        all_day,
        color: arg(&args, "color").unwrap_or("primary").to_owned(),
        description: arg(&args, "description").map(str::to_owned),
        recurrence: arg(&args, "recurrence")
            .map(|r| Rule::parse(r).map_or_else(|| r.to_owned(), |r| r.text())),
        exdates: BTreeSet::new(),
        recurrence_id: None,
        recurrence_date: None,
        location: arg(&args, "location").map(str::to_owned),
        links: links_of(&args)?.unwrap_or_default(),
        external_ref: arg(&args, "external_ref")
            .map(str::to_owned)
            .or_else(|| external_ref(&args)),
        tombstone: None,
    };
    let written = write_new(event).await?;
    Ok(written.json(&Local))
}

/// `external_ref` given as `{system, id}`.
fn external_ref(args: &Value) -> Option<String> {
    let r = args.get("external_ref")?;
    Some(format!(
        "{}:{}",
        r.get("system")?.as_str()?,
        r.get("id")?.as_str()?
    ))
}

/// `CALENDAR_UPDATE`: Task's `update_event`, which never moves one.
///
/// `title`, `description`, `location` and `color` (`""` clears the
/// middle two), `recurrence` (`""` ends the repeats), `links` (the whole
/// set), `tombstone` (true stamps now, false clears).
pub async fn update(args: Value) -> Result<Value, String> {
    let _ = refresh().await;
    let e = find(&args)?;
    let mut changed = Vec::new();
    let mut writes: Vec<(String, Value)> = Vec::new();
    if let Some(t) = arg(&args, "title") {
        writes.push((keys::TITLE.into(), json!(t)));
        changed.push("title");
    }
    for (k, name) in [
        (keys::DESCRIPTION, "description"),
        (keys::LOCATION, "location"),
    ] {
        if let Some(v) = args.get(name).and_then(Value::as_str) {
            writes.push((
                k.into(),
                if v.trim().is_empty() {
                    Value::Null
                } else {
                    json!(v.trim())
                },
            ));
            changed.push(name);
        }
    }
    if let Some(c) = arg(&args, "color") {
        writes.push((keys::COLOR.into(), json!(c)));
        changed.push("color");
    }
    if let Some(r) = args.get("recurrence").and_then(Value::as_str) {
        let r = r.trim();
        if r.is_empty() {
            writes.push((keys::RECURRENCE.into(), Value::Null));
        } else {
            let rule =
                Rule::parse(r).ok_or_else(|| format!("recurrence {r:?}: an RRULE this reads"))?;
            writes.push((keys::RECURRENCE.into(), json!(rule.text())));
        }
        changed.push("recurrence");
    }
    if let Some(links) = links_of(&args)? {
        for old in e.links.iter().map(|l| l.relation.clone()) {
            writes.push((old, Value::Null));
        }
        let mut by: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for l in links {
            by.entry(l.relation).or_default().push(l.target);
        }
        for (k, v) in by {
            writes.retain(|(w, _)| *w != k);
            writes.push((k, json!(v)));
        }
        changed.push("links");
    }
    if let Some(t) = bool_arg(&args, "tombstone") {
        writes.push((
            keys::TOMBSTONE.into(),
            if t {
                json!(Utc::now().to_rfc3339())
            } else {
                Value::Null
            },
        ));
        changed.push("tombstone");
    }
    if writes.is_empty() {
        return Err("nothing to change: give a title, description, location, color, recurrence, links or tombstone".to_owned());
    }
    for (k, v) in writes {
        vault::set(&e.path, &k, &v).await?;
    }
    let _ = refresh().await;
    let now = find(&json!({ "path": e.path }))?;
    let mut v = now.json(&Local);
    v["changed"] = json!(changed);
    Ok(v)
}

/// The occurrence of `e` on `day`: its start then.
fn occurrence_start(e: &Event, day: NaiveDate) -> NaiveDateTime {
    day.and_time(e.start.local(&Local).time())
}

/// Writes `e`'s times (and `all_day`).
async fn set_times(
    e: &Event,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
) -> Result<(), String> {
    vault::set(
        &e.path,
        keys::START,
        &json!(model::when_text(&Local, start, all_day)),
    )
    .await?;
    vault::set(
        &e.path,
        keys::END,
        &json!(model::when_text(&Local, end, all_day)),
    )
    .await?;
    if all_day != e.all_day {
        vault::set(&e.path, keys::ALL_DAY, &json!(all_day)).await?;
    }
    Ok(())
}

/// `CALENDAR_RESCHEDULE`: Task's `reschedule_event`, with a series'
/// scope.
///
/// To `start` (and `end`, or its length kept). For an occurrence
/// of a series (`occurrence`, its day): `this` (the default) makes an
/// override, `following` ends the series there and starts a new one,
/// `series` moves them all by as much.
pub async fn reschedule(args: Value) -> Result<Value, String> {
    let _ = refresh().await;
    let e = find(&args)?;
    let tz = Local;
    let occurrence = day_arg(&args, "occurrence")?;
    let length = e.length(&tz);
    let start_text = arg(&args, "start").ok_or("give the new `start`")?;
    let start_w = When::parse(start_text, &tz)
        .ok_or_else(|| format!("start {start_text:?}: a date or an RFC 3339 time"))?;
    let all_day = bool_arg(&args, "all_day").unwrap_or_else(|| {
        matches!(start_w, When::Day(_)) && (e.all_day || arg(&args, "end").is_none())
    });
    let start = if all_day {
        start_w.local(&tz).date().and_time(NaiveTime::MIN)
    } else {
        start_w.local(&tz)
    };
    let end = match arg(&args, "end").and_then(|t| When::parse(t, &tz)) {
        Some(w) => w.local(&tz),
        None if all_day && !e.all_day => start + Duration::days(1),
        None if !all_day && e.all_day => start + Duration::minutes(settings().event_minutes),
        None => start + length,
    };
    if end <= start {
        return Err("the end must come after the start".to_owned());
    }
    let moved_from = json!({ "start": e.start.text(), "end": e.end.text() });
    let rule = e
        .recurrence
        .as_deref()
        .and_then(Rule::parse)
        .filter(|_| e.recurrence_id.is_none());
    let scope = arg(&args, "scope").unwrap_or_else(|| {
        if occurrence.is_some() {
            "this"
        } else {
            "series"
        }
    });
    let out = match (rule, occurrence) {
        (Some(rule), Some(day)) if scope != "series" => {
            let was = occurrence_start(&e, day);
            let moved = match scope {
                "this" => write_new(override_of(&e, day, start, end, all_day)).await?,
                "following" if day > e.start.local(&tz).date() => {
                    split(&e, &rule, day, start, end, all_day).await?
                }
                "following" => {
                    // From the first on is the whole series.
                    set_times(&e, start, end, all_day).await?;
                    refreshed(&e.path).await?
                }
                other => return Err(format!("scope {other:?}: this, following or series")),
            };
            let mut v = moved.json(&tz);
            v["moved_occurrence"] = json!(was.format("%Y-%m-%dT%H:%M").to_string());
            v
        }
        (Some(_), occurrence) => {
            // The whole series, by as much as the occurrence moved.
            let was = occurrence.map_or_else(|| e.start.local(&tz), |d| occurrence_start(&e, d));
            let delta = start - was;
            let new_start = e.start.local(&tz) + delta;
            set_times(&e, new_start, new_start + (end - start), all_day).await?;
            let days = delta.num_days();
            if days != 0 && !e.exdates.is_empty() {
                let shifted: BTreeSet<NaiveDate> = e
                    .exdates
                    .iter()
                    .map(|d| *d + Duration::days(days))
                    .collect();
                vault::set(&e.path, keys::EXDATES, &model::exdates_value(&shifted)).await?;
            }
            refreshed(&e.path).await?.json(&tz)
        }
        (None, _) => {
            set_times(&e, start, end, all_day).await?;
            refreshed(&e.path).await?.json(&tz)
        }
    };
    let mut out = out;
    out["moved_from"] = moved_from;
    Ok(out)
}

async fn refreshed(path: &str) -> Result<Event, String> {
    let _ = refresh().await;
    find(&json!({ "path": path }))
}

/// An override of `e`'s occurrence on `day`, at its new times: the
/// series' title, colour, place, notes and links, its `id` and the day.
fn override_of(
    e: &Event,
    day: NaiveDate,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
) -> Event {
    let folder = e
        .path
        .rsplit_once('/')
        .map(|(d, _)| d.to_owned())
        .unwrap_or_default();
    Event {
        path: free_path(&folder, &e.title, day),
        id: free_id(&e.title, day),
        title: e.title.clone(),
        start: if all_day {
            When::Day(start.date())
        } else {
            When::At(model::fixed(&Local, start))
        },
        end: if all_day {
            When::Day(end.date())
        } else {
            When::At(model::fixed(&Local, end))
        },
        all_day,
        color: e.color.clone(),
        description: e.description.clone(),
        recurrence: None,
        exdates: BTreeSet::new(),
        recurrence_id: Some(e.id.clone()),
        recurrence_date: Some(day),
        location: e.location.clone(),
        links: e.links.clone(),
        external_ref: None,
        tombstone: None,
    }
}

/// Ends `e`'s series before `day`, and starts a new one there at the new
/// times, with the rule (less its count) and the skipped days after it.
async fn split(
    e: &Event,
    rule: &Rule,
    day: NaiveDate,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
) -> Result<Event, String> {
    let ended = rule.ending_before(day);
    vault::set(&e.path, keys::RECURRENCE, &json!(ended.text())).await?;
    let mut next_rule = rule.clone();
    next_rule.count = None;
    let delta = (start.date() - day).num_days();
    let mut new = override_of(e, day, start, end, all_day);
    new.recurrence_id = None;
    new.recurrence_date = None;
    new.recurrence = Some(next_rule.text());
    new.exdates = e
        .exdates
        .iter()
        .filter(|d| **d >= day)
        .map(|d| *d + Duration::days(delta))
        .collect();
    let before: BTreeSet<NaiveDate> = e.exdates.iter().filter(|d| **d < day).copied().collect();
    if before != e.exdates {
        vault::set(
            &e.path,
            keys::EXDATES,
            &if before.is_empty() {
                Value::Null
            } else {
                model::exdates_value(&before)
            },
        )
        .await?;
    }
    write_new(new).await
}

/// `CALENDAR_SKIP`: one occurrence of a series is no more (its day joins
/// `exdates`; an override of it goes).
pub async fn skip(args: Value) -> Result<Value, String> {
    let _ = refresh().await;
    let e = find(&args)?;
    let (master, day) = if let (Some(m), Some(d)) = (&e.recurrence_id, e.recurrence_date) {
        let master = find(&json!({ "id": m }))?;
        vault::run("VAULT_DELETE", json!({ "path": e.path })).await?;
        (master, d)
    } else {
        let day = day_arg(&args, "occurrence")?.ok_or("give the `occurrence` to skip, its day")?;
        if !e.repeats() {
            return Err("it does not repeat: cancel it instead".to_owned());
        }
        (e, day)
    };
    let mut days = master.exdates.clone();
    days.insert(day);
    vault::set(&master.path, keys::EXDATES, &model::exdates_value(&days)).await?;
    let now = refreshed(&master.path).await?;
    let mut v = now.json(&Local);
    v["skipped"] = json!(day.to_string());
    Ok(v)
}

/// `CALENDAR_CANCEL`: Task's `cancel_event`, which deletes its note (a
/// series' overrides too). With `occurrence`, only that one, as
/// `CALENDAR_SKIP`.
pub async fn cancel(args: Value) -> Result<Value, String> {
    if arg(&args, "occurrence").is_some() {
        return skip(args).await;
    }
    let _ = refresh().await;
    let e = find(&args)?;
    let overrides: Vec<String> = cache()
        .events
        .iter()
        .filter(|o| o.recurrence_id.as_deref() == Some(e.id.as_str()))
        .map(|o| o.path.clone())
        .collect();
    for path in overrides.iter().chain(std::iter::once(&e.path)) {
        vault::run("VAULT_DELETE", json!({ "path": path })).await?;
    }
    let _ = refresh().await;
    Ok(json!({ "cancelled": e.id, "path": e.path, "overrides": overrides }))
}

/// `CALENDAR_LIST`: Task's `list_events`.
///
/// The event notes (not their occurrences), those with one in `from`..`to` (YYYY-MM-DD, `to`
/// exclusive) when given, `linked_to` a note, by `external_ref`; the
/// tombstoned only when asked (or looked up by their ref).
pub async fn list(args: Value) -> Result<Value, String> {
    let _ = refresh().await;
    let from = day_arg(&args, "from")?;
    let to = day_arg(&args, "to")?;
    let linked = arg(&args, "linked_to");
    let by_ref = arg(&args, "external_ref");
    let tombstoned = bool_arg(&args, "include_tombstoned").unwrap_or(false) || by_ref.is_some();
    let limit =
        int_arg(&args, "limit").map_or(50, |n| usize::try_from(n).unwrap_or(1).clamp(1, 200));
    let events = events();
    let window = match (from, to) {
        (None, None) => None,
        (f, t) => {
            let f = f.unwrap_or(NaiveDate::MIN);
            let t = t.unwrap_or_else(|| f + Duration::days(366 * 10));
            Some((layout::midnight(f), layout::midnight(t)))
        }
    };
    let mut hits: Vec<&Event> = events
        .iter()
        .filter(|e| tombstoned || e.tombstone.is_none())
        .filter(|e| linked.is_none_or(|n| e.links_to(n)))
        .filter(|e| by_ref.is_none_or(|r| e.external_ref.as_deref() == Some(r)))
        .filter(|e| {
            window.is_none_or(|(f, t)| {
                let mut one = (*e).clone();
                one.tombstone = None;
                !model::occurrences(std::slice::from_ref(&one), f, t, &Local).is_empty()
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        a.start
            .local(&Local)
            .cmp(&b.start.local(&Local))
            .then_with(|| a.title.cmp(&b.title))
    });
    let count = hits.len();
    let events: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|e| e.json(&Local))
        .collect();
    Ok(json!({ "count": count, "events": events }))
}

/// A block as the actions say it.
#[must_use]
pub fn block_json(b: &Block) -> Value {
    let mut v = serde_json::to_value(b).unwrap_or(Value::Null);
    v["start"] = json!(b.start.format("%Y-%m-%dT%H:%M").to_string());
    v["end"] = json!(b.end.format("%Y-%m-%dT%H:%M").to_string());
    v
}

/// `CALENDAR_AGENDA`: what the calendar draws over `from`..`to` (today
/// and the six days after by default): each occurrence, task and span of
/// time, in order.
pub async fn agenda(args: Value) -> Result<Value, String> {
    let _ = refresh().await;
    let from = day_arg(&args, "from")?.unwrap_or_else(today);
    let to = day_arg(&args, "to")?.unwrap_or(from + Duration::days(7));
    if to <= from {
        return Err("`to` must come after `from`".to_owned());
    }
    let want_tasks = bool_arg(&args, "tasks").unwrap_or(true);
    let want_time = bool_arg(&args, "time").unwrap_or(true);
    let blocks: Vec<Value> = blocks(layout::midnight(from), layout::midnight(to))
        .iter()
        .filter(|b| match b.source {
            model::Source::Task { .. } => want_tasks,
            model::Source::Time { .. } => want_time,
            model::Source::Event { .. } | model::Source::Booking { .. } => true,
        })
        .map(block_json)
        .collect();
    let c = cache();
    Ok(json!({
        "from": from.to_string(),
        "to": to.to_string(),
        "count": blocks.len(),
        "blocks": blocks,
        "tasks": c.tasks_found,
        "time": c.time_found,
    }))
}

/// What `CALENDAR_SHOWN` and `CALENDAR_OPEN` say: the view's mode, day
/// and range, and what is on it.
#[must_use]
pub fn shown_json() -> Value {
    let s = shown();
    let first = settings().week_start;
    let days = layout::days(s.mode, s.cursor, first);
    let (Some(a), Some(b)) = (days.first().copied(), days.last().copied()) else {
        return Value::Null;
    };
    let blocks: Vec<Value> = blocks(layout::midnight(a), layout::midnight(b + Duration::days(1)))
        .iter()
        .map(block_json)
        .collect();
    json!({
        "view": s.mode.id(),
        "date": s.cursor.to_string(),
        "from": a.to_string(),
        "to": (b + Duration::days(1)).to_string(),
        "label": layout::range_label(s.mode, s.cursor, first),
        "count": blocks.len(),
        "blocks": blocks,
    })
}

/// `CALENDAR_OPEN {view?, date?, move?}`: opens the calendar: `view` day,
/// week, month or agenda; around `date` (`today` by default when the
/// view is not yet open); `move` pages by as many.
pub async fn open(args: Value) -> Result<Value, String> {
    let mut s = shown();
    if let Some(v) = arg(&args, "view") {
        s.mode =
            Mode::from_id(v).ok_or_else(|| format!("view {v:?}: day, week, month or agenda"))?;
    }
    if let Some(d) = day_arg(&args, "date")? {
        s.cursor = d;
    }
    if let Some(by) = int_arg(&args, "move") {
        s.cursor = layout::step(s.mode, s.cursor, i32::try_from(by).unwrap_or(0));
    }
    show(s);
    let _ = refresh().await;
    vault::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": VIEW })).await?;
    Ok(shown_json())
}

/// `CALENDAR_DAILY {date?}`: opens `date`'s daily note (today's by
/// default), made when missing, through the navigator's periodic notes
/// (`NAVIGATOR_OPEN_DAILY_NOTE`).
pub async fn daily(args: Value) -> Result<Value, String> {
    let day = day_arg(&args, "date")?.unwrap_or_else(today);
    vault::run(
        "NAVIGATOR_OPEN_DAILY_NOTE",
        json!({ "date": day.to_string(), "confirm": false }),
    )
    .await
    .map_err(|e| {
        if e.contains("no action") || e.contains("not found") || e.contains("unknown") {
            "daily notes are the navigator's: turn Navigator on".to_owned()
        } else {
            e
        }
    })
}

/// The event a line makes on `day` (`quick` read the times): its args for
/// [`create`].
fn quick_args(
    text: &str,
    day: Option<NaiveDate>,
    extra: &Map<String, Value>,
) -> Result<Value, String> {
    let draft = architect_capture::parse(text, today());
    let q = quick::parse(&draft.title);
    let mut q = q;
    if q.start.is_none()
        && let Some(t) = extra
            .get("time")
            .and_then(Value::as_str)
            .and_then(quick::time_of_day)
    {
        q.start = Some(t);
    }
    if q.length.is_none()
        && let Some(l) = extra
            .get("duration")
            .and_then(Value::as_str)
            .and_then(quick::length)
    {
        q.length = Some(l);
    }
    if q.title.trim().is_empty() {
        return Err("say what the event is".to_owned());
    }
    let day = draft.due.or(day).unwrap_or_else(today);
    let (s, e, all_day) = quick::span(&q, day, Duration::minutes(settings().event_minutes));
    let mut args = json!({
        "title": q.title,
        "start": model::when_text(&Local, s, all_day),
        "end": model::when_text(&Local, e, all_day),
        "all_day": all_day,
    });
    if let Some(r) = draft.repeat {
        args["recurrence"] = json!(r.rule);
    }
    let links: Vec<Value> = draft
        .projects
        .iter()
        .map(|p| json!({ "relation": "projects", "target": p }))
        .collect();
    if !links.is_empty() {
        args["links"] = json!(links);
    }
    Ok(args)
}

/// `CALENDAR_QUICK {text, date?}`: an event from a line, as the
/// calendar's own field reads it (`Lunch with Sam tomorrow 1pm for 90m`).
pub async fn quick_add(args: Value) -> Result<Value, String> {
    let text = arg(&args, "text").ok_or("say what the event is: `text`")?;
    let created = create(quick_args(text, day_arg(&args, "date")?, &Map::new())?).await?;
    let _ = refresh().await;
    Ok(created)
}

/// `CALENDAR_CAPTURE`: the `event` capture target's action.
///
/// A capture from architect's composer or inbox, its day from `properties.due`,
/// its time and length from the title (`1pm`, `2-3pm`) or `at:` and
/// `for:`, its links from `properties.projects`.
pub async fn capture(args: Value) -> Result<Value, String> {
    let capture: architect_capture::Capture =
        serde_json::from_value(args).map_err(|e| e.to_string())?;
    let day = capture
        .properties
        .get("due")
        .and_then(Value::as_str)
        .and_then(|d| NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d").ok());
    let mut a = quick_args(&capture.title, day, &capture.properties)?;
    if let Some(r) = capture.properties.get("recurrence").and_then(Value::as_str) {
        a["recurrence"] = json!(r);
    }
    if let Some(p) = capture.properties.get("projects").and_then(Value::as_array) {
        let links: Vec<Value> = p
            .iter()
            .filter_map(Value::as_str)
            .map(|t| json!({ "relation": "projects", "target": t }))
            .collect();
        if !links.is_empty() {
            a["links"] = json!(links);
        }
    }
    if !capture.body.trim().is_empty() {
        a["description"] = json!(capture.body.trim());
    }
    let created = create(a).await?;
    Ok(json!({ "path": created["path"], "event": created }))
}

/// `CALENDAR_BASE {view?}`: writes (the first time) and opens a base of
/// the events drawn by its `calendar` (the default) or `timeline` view.
pub async fn base(args: Value) -> Result<Value, String> {
    if cache().settings.is_none() {
        let _ = refresh().await;
    }
    let kind = arg(&args, "view").unwrap_or("calendar");
    let name = match kind {
        "calendar" => CALENDAR_BASE,
        "timeline" => TIMELINE_BASE,
        other => return Err(format!("view {other:?}: calendar or timeline")),
    };
    let folder = settings().folder;
    let path = if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    };
    let exists = vault::run("BASES_READ", json!({ "path": path }))
        .await
        .is_ok();
    if !exists {
        vault::run(
            "VAULT_CREATE",
            json!({ "path": path, "body": view_base(kind) }),
        )
        .await?;
        // VAULT_CREATE indexes what it writes as a note: read the folder
        // again so Bases knows the new file is a base.
        vault::run("VAULT_RELOAD", Value::Null).await?;
    }
    let opened = vault::run("BASES_OPEN", json!({ "path": path })).await?;
    Ok(json!({ "path": path, "written": !exists, "opened": opened.get("opened") }))
}

/// `CALENDAR_MOVE_TASK {path, date, time?}`: a task dragged on the
/// calendar: its scheduled date (or its due one, when that is the one it
/// has) moves, through the Tasks extension.
pub async fn move_task(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give the task's `path`")?;
    let day = day_arg(&args, "date")?.ok_or("give the `date` it moves to")?;
    let field = cache()
        .tasks
        .iter()
        .find(|b| b.source.path() == path)
        .and_then(|b| match &b.source {
            model::Source::Task { field, .. } => Some(field.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "scheduled".to_owned());
    let value = arg(&args, "time").and_then(quick::time_of_day).map_or_else(
        || day.to_string(),
        |t| day.and_time(t).format("%Y-%m-%dT%H:%M").to_string(),
    );
    let out = match vault::service(
        TASKS,
        "update",
        json!({ "path": path, field.clone(): value }),
    )
    .await
    {
        Some(r) => r?,
        None => return Err("tasks are the Tasks extension's: turn it on".to_owned()),
    };
    let _ = refresh().await;
    Ok(
        json!({ "path": path, "field": field, "value": value, "task": out.get("task").cloned().unwrap_or(out) }),
    )
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Calendar;

impl Extension for Calendar {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[(
            "tours/calendar-tour.styx",
            include_str!("../tours/calendar-tour.styx"),
        )]
    }

    fn build(&self, ext: &mut Builder) {
        vault::connect(ext.handle());
        ext.navigator(nav::ACTIONS, nav::CalendarNav);
        ext.view(VIEW, ui::CalendarView)
            .view("bookings", booking::BookingsView)
            .base_view("calendar", ui::MonthBase)
            .base_view("timeline", ui::TimelineBase)
            .action("CALENDAR_LIST", list)
            .action("CALENDAR_AGENDA", agenda)
            .action("CALENDAR_CREATE", create)
            .action("CALENDAR_UPDATE", update)
            .action("CALENDAR_RESCHEDULE", reschedule)
            .action("CALENDAR_SKIP", skip)
            .action("CALENDAR_CANCEL", cancel)
            .action("CALENDAR_QUICK", quick_add)
            .action("CALENDAR_CAPTURE", capture)
            .action("CALENDAR_MOVE_TASK", move_task)
            .action("CALENDAR_OPEN", open)
            .action("CALENDAR_SHOWN", |_| async {
                if !cache().read {
                    let _ = refresh().await;
                }
                Ok(shown_json())
            })
            .action("CALENDAR_DAILY", daily)
            .action("CALENDAR_EVENT_TYPES", booking::list_types)
            .action("CALENDAR_EVENT_TYPE", booking::save_type)
            .action("CALENDAR_SCHEDULE", booking::save_schedule)
            .action("CALENDAR_OPEN_SLOTS", booking::list_slots)
            .action("CALENDAR_BOOK", booking::book)
            .action("CALENDAR_BOOKINGS", booking::list_bookings)
            .action("CALENDAR_BOOKING_STATUS", booking::set_status)
            .action("CALENDAR_BOOKINGS_OPEN", booking::open_view)
            .action("CALENDAR_SUBSCRIBE", sync::subscribe)
            .action("CALENDAR_SYNC", sync::sync)
            .action("CALENDAR_IMPORT", sync::import_text)
            .action("CALENDAR_SUBSCRIPTIONS", sync::list)
            .action("CALENDAR_UNSUBSCRIBE", sync::unsubscribe)
            .action("CALENDAR_BASE", base)
            .action("CALENDAR_STARTED", |_| async {
                let _ = refresh().await;
                // Subscribed calendars are fetched again at each start.
                let synced = sync::sync(Value::Null).await.ok();
                Ok(json!({ "ready": true, "synced": synced }))
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_bases_parse() {
        for source in [
            events_base(&["for".into()]),
            view_base("calendar"),
            view_base("timeline"),
        ] {
            let v: serde_yaml::Value = serde_yaml::from_str(&source).unwrap();
            assert!(v["views"][0]["order"].as_sequence().is_some(), "{source}");
        }
        assert!(view_base("timeline").contains("type: timeline"));
    }

    #[test]
    fn extra_columns_are_read() {
        let out =
            json!({ "columns": ["file.name", "title", "for", "formula.x", "with people", "with"] });
        assert_eq!(extras_of(&out), ["for", "with"]);
    }
}
