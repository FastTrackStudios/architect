//! The Time Tracker (`extensions/time-tracker/extension.toml`): timers
//! and time entries on vault notes, the last member of the Project
//! Management bundle (#375), and a model of how an extension is built
//! (#368).
//!
//! One [`TimeTracker`] value runs it two ways: the app links it in
//! (`Extensions::with_linked`), and its program (`src/main.rs`) serves the
//! same value as a process (`Guest::extension`), so `EXT_CLONE {"id":
//! "time"}` gives a development copy that overrides the linked one.
//!
//! - **Data:** an entry is a row of a note's `timeEntries` (`TaskNotes`'
//!   shape, Task's `task::TimeEntry`; [`model`]) on the task, the project
//!   or `Time/Unfiled.md`. A running timer is the entry with no `endTime`.
//!   It reads notes with one `BASES_ROWS` query and writes them with
//!   `VAULT_SET_PROPERTY` ([`vault`]).
//! - **Totals:** per note, per project (its own time and its tasks'), per
//!   day and per week ([`model::report`]); billing from the project note
//!   (`billableDefault`, `defaultRateCents`, `currency`).
//! - **Views:** a report, and a `timesheet` view type for Bases
//!   (`[[base_views]]`) that `TIME_TIMESHEET` opens over
//!   `Time/Timesheet.base`. Its status item says what is running.
//! - **Navigator:** running, today, this week and earlier ([`nav`]).
//! - **Service:** it provides `architect.time` (each method its
//!   `TIME_<METHOD>` action) and uses `architect.tasks` and
//!   `architect.projects` when they are there: a task named by its
//!   path or link, a project by its name, a task's time counted toward
//!   its project. The Projects page and the Tasks list show time through
//!   `architect.time`.

// Its views and service calls read JSON with `value["key"]`: serde_json's
// index answers `Null` for what is missing, never a panic. Sums of
// tracked seconds and spans of days stay far inside i64 and chrono's
// range.
#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

pub mod model;
pub mod nav;
pub mod vault;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::linked::{Builder, Extension};
use chrono::{DateTime, Local, NaiveDate, TimeZone as _, Utc};
use dioxus::prelude::*;
use serde_json::{Map, Value, json};

use crate::model::{Entry, Kind, Report, Span, Tracked, keys};

/// Its id: its actions are `TIME_*`.
pub const ID: &str = "time";

/// The service it provides (`[provides]`): method `m` is `TIME_<M>`.
pub const SERVICE: &str = "architect.time";

/// The services it uses when they are there.
const TASKS: &str = "architect.tasks";
const PROJECTS: &str = "architect.projects";

/// Its status item.
const TIMER: &str = "timer";

/// Its folder: the timesheet's base, and time on no note.
pub const FOLDER: &str = "Time";

/// Where time on no task or project goes.
pub const UNFILED: &str = "Time/Unfiled.md";

/// The base `TIME_TIMESHEET` writes and opens, in [`FOLDER`].
pub const TIMESHEET: &str = "Timesheet.base";

const FILTERS: &str = "filters:\n  or:\n    - file.hasProperty(\"timeEntries\")\n    - type == \"project\"\n    - file.name == \"project\"\n";

/// The base that finds every note with time on it, and every project
/// (whose note bills it).
#[must_use]
pub fn tracked_base() -> String {
    format!(
        "{FILTERS}views:\n  - type: table\n    name: Time\n    order:\n{}",
        keys::ALL.iter().fold(String::new(), |mut out, k| {
            let _ = writeln!(out, "      - {k}");
            out
        })
    )
}

/// The timesheet's base: the same notes, drawn by the `timesheet` view.
#[must_use]
pub fn timesheet_base() -> String {
    format!(
        "{FILTERS}views:\n  - type: timesheet\n    name: Timesheet\n    order:\n      - file.name\n      - title\n      - type\n      - projects\n      - timeEntries\n"
    )
}

/// What it knows of the vault's time, as last read.
#[derive(Debug, Default)]
struct Cache {
    notes: Vec<Tracked>,
    /// Each note's project (its `project.md`), as the services resolve it.
    project_of: BTreeMap<String, String>,
    error: Option<String>,
    read: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    notes: Vec::new(),
    project_of: BTreeMap::new(),
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

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// A time of day here, `14:05`.
fn hhmm(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M").to_string()
}

/// What its status item says: the running timer, else today's total.
#[must_use]
pub fn status_text(notes: &[Tracked], now: DateTime<Utc>) -> String {
    if let Some((n, e)) = model::running(notes) {
        return format!("{} · since {}", n.title, hhmm(e.start));
    }
    let start = model::day_start(today(), &Local);
    let s: i64 = notes.iter().map(|n| n.seconds_in(start, now, now)).sum();
    if s == 0 {
        "No timer".to_owned()
    } else {
        format!("{} today", model::words(s))
    }
}

/// Each note's project: its own link with a folder, else the project
/// the Projects extension resolves a bare link to, else the project
/// whose folder it is in.
async fn projects_of(notes: &[Tracked]) -> BTreeMap<String, String> {
    let folders: Vec<(String, String)> = notes
        .iter()
        .filter(|n| n.kind == Kind::Project)
        .filter_map(|p| {
            p.path
                .rsplit_once('/')
                .map(|(dir, _)| (format!("{dir}/"), p.path.clone()))
        })
        .collect();
    let mut resolved: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for n in notes {
        let mut project = n.project_path();
        if project.is_none()
            && let Some(link) = n.projects.first()
        {
            if !resolved.contains_key(link) {
                let found = match vault::service(PROJECTS, "resolve", json!({ "link": link })).await
                {
                    Some(Ok(v)) => v.get("path").and_then(Value::as_str).map(str::to_owned),
                    _ => None,
                };
                resolved.insert(link.clone(), found);
            }
            project = resolved.get(link).cloned().flatten();
        }
        if project.is_none() {
            project = folders
                .iter()
                .filter(|(dir, _)| n.path.starts_with(dir.as_str()))
                .max_by_key(|(dir, _)| dir.len())
                .map(|(_, p)| p.clone());
        }
        if let Some(p) = project {
            out.insert(n.path.clone(), p);
        }
    }
    out
}

/// Reads every tracked note again; its status item says what is running.
async fn refresh() -> Result<Vec<Tracked>, String> {
    let read = vault::rows(&tracked_base()).await.map(|rows| {
        rows.into_iter()
            .map(|r| Tracked::from_values(&r.path, &r.name, &r.values))
            .collect::<Vec<_>>()
    });
    let project_of = match &read {
        Ok(notes) => Some(projects_of(notes).await),
        Err(_) => None,
    };
    {
        let mut c = cache();
        c.read = true;
        match &read {
            Ok(notes) => {
                c.notes.clone_from(notes);
                c.error = None;
            }
            Err(e) => c.error = Some(e.clone()),
        }
        if let Some(p) = project_of {
            c.project_of = p;
        }
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    if let (Some(h), Ok(notes)) = (vault::handle(), &read) {
        h.status(TIMER, status_text(notes, Utc::now()));
    }
    read
}

/// The report over `span` of what it last read.
fn report_now(span: Option<Span>) -> Report {
    let c = cache();
    let project_of = |n: &Tracked| c.project_of.get(&n.path).cloned();
    model::report(&c.notes, span, Utc::now(), &Local, &project_of)
}

/// The note time is tracked on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    path: String,
    title: String,
    kind: Kind,
}

/// A note `link` names, as it is: a path or a link with a folder (`.md`
/// added), else a name at the top.
async fn note_target(link: &str, kind: Kind) -> Result<Target, String> {
    let target = model::link_target(link);
    let path = model::link_path(link).unwrap_or_else(|| {
        if std::path::Path::new(target)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            target.to_owned()
        } else {
            format!("{target}.md")
        }
    });
    let props = vault::properties(&path)
        .await
        .ok_or_else(|| format!("no note at {path}"))?;
    let t = Tracked::from_values(&path, path.rsplit('/').next().unwrap_or(&path), &props);
    Ok(Target {
        path,
        title: t.title,
        kind: if kind == Kind::Note { t.kind } else { kind },
    })
}

/// The note `args` names (`task`, `project` or `note`), through Tasks'
/// and Projects' services when they are there; `Time/Unfiled.md` when it
/// names none.
async fn target(args: &Value) -> Result<Target, String> {
    if let Some(task) = arg(args, "task") {
        return match vault::service(TASKS, "read", json!({ "path": task })).await {
            Some(Ok(v)) => Ok(Target {
                path: v["task"]["path"].as_str().unwrap_or(task).to_owned(),
                title: v["task"]["title"].as_str().unwrap_or(task).to_owned(),
                kind: Kind::Task,
            }),
            Some(Err(e)) => Err(e),
            None => note_target(task, Kind::Task).await,
        };
    }
    if let Some(project) = arg(args, "project") {
        return match vault::service(PROJECTS, "resolve", json!({ "link": project })).await {
            Some(Ok(v)) => Ok(Target {
                path: v["path"].as_str().unwrap_or(project).to_owned(),
                title: v["title"].as_str().unwrap_or(project).to_owned(),
                kind: Kind::Project,
            }),
            Some(Err(e)) => Err(e),
            None => {
                let link = if model::link_target(project).contains('/') {
                    project.to_owned()
                } else {
                    format!("Projects/{}/project", model::link_target(project))
                };
                note_target(&link, Kind::Project).await
            }
        };
    }
    if let Some(note) = arg(args, "note").or_else(|| arg(args, "path")) {
        return note_target(note, Kind::Note).await;
    }
    if vault::properties(UNFILED).await.is_none() {
        let mut props = Map::new();
        props.insert(keys::TITLE.to_owned(), json!("Unfiled time"));
        props.insert(keys::TIME_ENTRIES.to_owned(), json!([]));
        vault::create(
            UNFILED,
            &props,
            "Time logged on no task or project. Move an entry to its note's `timeEntries` to file it.\n",
        )
        .await?;
    }
    Ok(Target {
        path: UNFILED.to_owned(),
        title: "Unfiled time".to_owned(),
        kind: Kind::Note,
    })
}

/// The project time on `target` counts toward: as last read, else
/// worked out from its note (a first entry on a task), as [`refresh`]
/// does.
async fn project_for(target: &Target) -> Option<String> {
    if target.kind == Kind::Project {
        return Some(target.path.clone());
    }
    let known = cache().project_of.get(&target.path).cloned();
    if known.is_some() {
        return known;
    }
    let props = vault::properties(&target.path).await?;
    let note = Tracked::from_values(&target.path, &target.path, &props);
    let mut notes: Vec<Tracked> = cache()
        .notes
        .iter()
        .filter(|n| n.kind == Kind::Project)
        .cloned()
        .collect();
    notes.push(note);
    projects_of(&notes).await.remove(&target.path)
}

/// The `billable` an entry on `target` is written with: as given, else
/// `true` when its project's `billableDefault` says so (Task's
/// `WorkSession` snapshot at start), left out when it does not.
async fn billable_for(args: &Value, target: &Target) -> Option<bool> {
    if let Some(b) = args.get("billable").and_then(Value::as_bool) {
        return Some(b);
    }
    let project = project_for(target).await?;
    cache()
        .notes
        .iter()
        .any(|n| n.path == project && n.billable_default)
        .then_some(true)
}

/// The note's `timeEntries` as written now (`VAULT_READ`, not the index).
async fn entries_at(path: &str) -> Result<Value, String> {
    let props = vault::properties(path)
        .await
        .ok_or_else(|| format!("no note at {path}"))?;
    Ok(props
        .get(keys::TIME_ENTRIES)
        .cloned()
        .unwrap_or(Value::Null))
}

fn entry_json(path: &str, title: &str, e: &Entry, now: DateTime<Utc>) -> Value {
    json!({
        "path": path,
        "title": title,
        "start": model::stamp(e.start),
        // Its start as a time of day here, for a page that shows it.
        "since": hhmm(e.start),
        "end": e.end.map(model::stamp),
        "seconds": e.seconds(now),
        "description": e.description,
        "billable": e.billable,
    })
}

/// Ends every running entry at `now`: the entries ended.
async fn stop_all(now: DateTime<Utc>) -> Result<Vec<Value>, String> {
    let notes = refresh().await?;
    let mut out = Vec::new();
    for n in notes.iter().filter(|n| n.running().is_some()) {
        let (after, ended) = model::stopped(&entries_at(&n.path).await?, now);
        if ended.is_empty() {
            continue;
        }
        vault::set(&n.path, keys::TIME_ENTRIES, &after).await?;
        out.extend(ended.iter().map(|e| entry_json(&n.path, &n.title, e, now)));
    }
    Ok(out)
}

/// `TIME_START {task? | project? | note?, description?, billable?}`: a
/// timer on the note, the running one stopped first (Task's
/// `switch_timer`: one runs at a time).
pub async fn start(args: Value) -> Result<Value, String> {
    let target = target(&args).await?;
    let now = Utc::now();
    let stopped = stop_all(now).await?;
    let entry = Entry {
        start: now,
        end: None,
        description: arg(&args, "description").map(str::to_owned),
        billable: billable_for(&args, &target).await,
    };
    let raw = entries_at(&target.path).await?;
    vault::set(
        &target.path,
        keys::TIME_ENTRIES,
        &model::appended(&raw, &entry),
    )
    .await?;
    let _ = refresh().await;
    Ok(json!({
        "started": entry_json(&target.path, &target.title, &entry, now),
        "kind": target.kind,
        "stopped": stopped,
    }))
}

/// `TIME_STOP`: the running timer, ended now.
pub async fn stop() -> Result<Value, String> {
    let stopped = stop_all(Utc::now()).await?;
    let _ = refresh().await;
    let first = stopped.first().cloned().ok_or("no timer is running")?;
    Ok(json!({ "stopped": first, "all": stopped }))
}

/// `TIME_ACTIVE`: the running timer, or `null`.
async fn active() -> Result<Value, String> {
    let notes = refresh().await?;
    let now = Utc::now();
    Ok(model::running(&notes).map_or(Value::Null, |(n, e)| entry_json(&n.path, &n.title, e, now)))
}

/// `TIME_TOGGLE`: its status item's press: stops the running timer, else
/// opens the report.
async fn toggle() -> Result<Value, String> {
    if active().await?.is_null() {
        open(json!({ "view": "report" })).await
    } else {
        stop().await
    }
}

/// When a logged entry starts and ends: `start` and `end`; or a length
/// (`duration` as typed, `minutes`, `seconds`) ending now, or on `date`
/// (another day) starting at 09:00 there.
fn logged_span(args: &Value, now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    if let (Some(s), Some(e)) = (arg(args, "start"), arg(args, "end")) {
        let s = model::parse_time(s).ok_or_else(|| format!("start {s:?}: not a time"))?;
        let e = model::parse_time(e).ok_or_else(|| format!("end {e:?}: not a time"))?;
        if e <= s {
            return Err("end comes before start".to_owned());
        }
        return Ok((s, e));
    }
    let length = args
        .get("seconds")
        .and_then(Value::as_i64)
        .or_else(|| {
            args.get("minutes")
                .and_then(Value::as_i64)
                .map(|m| m.saturating_mul(60))
        })
        .or_else(|| arg(args, "duration").and_then(model::parse_duration))
        .filter(|s| *s > 0)
        .ok_or("say how long: `duration` (1h30m, 45m, 1:30), `minutes`, or `start` and `end`")?;
    let length = chrono::Duration::seconds(length);
    let day = match arg(args, "date") {
        Some(d) => Some(
            NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d")
                .map_err(|_| format!("date {d:?}: YYYY-MM-DD"))?,
        ),
        None => None,
    };
    Ok(day.filter(|d| *d != today()).map_or_else(
        || (now - length, now),
        |d| {
            let nine = d.and_hms_opt(9, 0, 0).unwrap_or_default();
            let s = Local
                .from_local_datetime(&nine)
                .earliest()
                .map_or_else(|| nine.and_utc(), |t| t.with_timezone(&Utc));
            (s, s + length)
        },
    ))
}

/// `TIME_LOG {task? | project? | note?, duration? | minutes? | start+end,
/// date?, description?, billable?}`: a finished entry (Task's
/// `log_session`, "I forgot to start the timer"); a running timer stays.
pub async fn log(args: Value) -> Result<Value, String> {
    let now = Utc::now();
    let (s, e) = logged_span(&args, now)?;
    let target = target(&args).await?;
    // The project's billing, read before the first entry on it.
    if !cache().read {
        let _ = refresh().await;
    }
    let entry = Entry {
        start: s,
        end: Some(e),
        description: arg(&args, "description").map(str::to_owned),
        billable: billable_for(&args, &target).await,
    };
    let raw = entries_at(&target.path).await?;
    vault::set(
        &target.path,
        keys::TIME_ENTRIES,
        &model::appended(&raw, &entry),
    )
    .await?;
    let _ = refresh().await;
    Ok(json!({
        "logged": entry_json(&target.path, &target.title, &entry, now),
        "path": target.path,
        "kind": target.kind,
    }))
}

/// `TIME_CAPTURE {title, body, properties, source, inbox?}`: the `time`
/// capture target ("Log time"): `for:1h30` (or a length in the title),
/// its project (`[[Album]]`) and its day (`today`, a date) make an entry;
/// the rest of the title is its description.
async fn capture(args: Value) -> Result<Value, String> {
    let capture: architect_capture::Capture =
        serde_json::from_value(args).map_err(|e| format!("not a capture: {e}"))?;
    let (in_title, link, description) = model::log_line(&capture.title);
    let props = &capture.properties;
    let length = props
        .get("duration")
        .and_then(model::scalar)
        .and_then(|d| model::parse_duration(&d))
        .or(in_title)
        .ok_or("say how long: `for:1h30`, or a length such as 45m in the line")?;
    let project = props
        .get(keys::PROJECTS)
        .map(model::list_of)
        .and_then(|l| l.into_iter().next())
        .or(link);
    let date = props
        .get("due")
        .or_else(|| props.get("scheduled"))
        .and_then(model::scalar);
    let mut log_args = json!({ "seconds": length, "description": description });
    if let Some(a) = log_args.as_object_mut() {
        if let Some(p) = project {
            a.insert("project".to_owned(), json!(p));
        }
        if let Some(d) = date {
            a.insert("date".to_owned(), json!(d));
        }
        if !capture.body.trim().is_empty() && description.is_empty() {
            a.insert("description".to_owned(), json!(capture.body.trim()));
        }
    }
    log(log_args).await
}

/// One note's totals: every second, today's, this week's, and whether it
/// runs.
fn totals_of(n: &Tracked, now: DateTime<Utc>) -> Value {
    let day = model::day_start(today(), &Local);
    let week = model::day_start(model::week_start(today()), &Local);
    json!({
        "title": n.title,
        "kind": n.kind,
        "seconds": n.seconds(now),
        "today": n.seconds_in(day, now, now),
        "week": n.seconds_in(week, now, now),
        "running": n.running().is_some(),
    })
}

/// `TIME_TOTALS {paths?}`: each tracked note's totals, by path.
async fn totals(args: Value) -> Result<Value, String> {
    let notes = refresh().await?;
    let only = args.get("paths").map(model::list_of);
    let now = Utc::now();
    let out: Map<String, Value> = notes
        .iter()
        .filter(|n| !n.entries.is_empty())
        .filter(|n| only.as_ref().is_none_or(|o| o.contains(&n.path)))
        .map(|n| (n.path.clone(), totals_of(n, now)))
        .collect();
    Ok(json!({ "notes": out }))
}

/// `TIME_FOR_PROJECT {project}`: a project's time, its own and its
/// tasks' (those Tasks' `for_project` names, and those that link it or
/// sit in its folder): totals, billing, the running timer, each task's.
async fn for_project(args: Value) -> Result<Value, String> {
    let link = arg(&args, "project").ok_or("give `project`: its project.md's path")?;
    let notes = refresh().await?;
    let path = match vault::service(PROJECTS, "resolve", json!({ "link": link })).await {
        Some(Ok(v)) => v["path"].as_str().unwrap_or(link).to_owned(),
        _ => model::link_path(link).unwrap_or_else(|| link.to_owned()),
    };
    let mut members: Vec<String> =
        match vault::service(TASKS, "for_project", json!({ "project": path })).await {
            Some(Ok(v)) => v["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["path"].as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
    {
        let c = cache();
        members.extend(
            c.project_of
                .iter()
                .filter(|(n, p)| **p == path && **n != path)
                .map(|(n, _)| n.clone()),
        );
    }
    members.sort();
    members.dedup();
    let now = Utc::now();
    let day = model::day_start(today(), &Local);
    let week = model::day_start(model::week_start(today()), &Local);
    let project = notes.iter().find(|n| n.path == path);
    let default = project.is_some_and(|p| p.billable_default);
    let rate = project.and_then(|p| p.rate_cents).unwrap_or(0);
    let currency = project.and_then(|p| p.currency.clone()).unwrap_or_default();
    let sum = |n: &Tracked, (total, today, wk, billable): &mut (i64, i64, i64, i64)| {
        *total += n.seconds(now);
        *today += n.seconds_in(day, now, now);
        *wk += n.seconds_in(week, now, now);
        *billable += n
            .entries
            .iter()
            .filter(|e| e.billable.unwrap_or(default))
            .map(|e| e.seconds(now))
            .sum::<i64>();
    };
    let mut all = (0, 0, 0, 0);
    let mut own = (0, 0, 0, 0);
    if let Some(p) = project {
        sum(p, &mut own);
        sum(p, &mut all);
    }
    let mut by_task = Map::new();
    let mut running = project
        .and_then(|p| p.running().map(|e| entry_json(&p.path, &p.title, e, now)))
        .unwrap_or(Value::Null);
    for n in notes.iter().filter(|n| members.contains(&n.path)) {
        sum(n, &mut all);
        if !n.entries.is_empty() {
            by_task.insert(n.path.clone(), totals_of(n, now));
        }
        if let Some(e) = n.running() {
            running = entry_json(&n.path, &n.title, e, now);
        }
    }
    let cents = model::amount(all.3, rate);
    Ok(json!({
        "project": path,
        "seconds": all.0,
        "today": all.1,
        "week": all.2,
        "billable_seconds": all.3,
        "own_seconds": own.0,
        "rate_cents": rate,
        "currency": currency,
        "cents": cents,
        "amount": (rate > 0).then(|| model::money(cents, &currency)),
        "running": running,
        "by_task": by_task,
    }))
}

/// `TIME_REPORT {range?, from?, to?}`: totals per day, note and project
/// over `range` (today, week — the default —, last-week, month, all) or
/// `from`..`to` (`to` exclusive).
async fn report(args: Value) -> Result<Value, String> {
    let notes = refresh().await?;
    let span = if let (Some(f), Some(t)) = (arg(&args, "from"), arg(&args, "to")) {
        let d = |s: &str| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("{s:?}: YYYY-MM-DD"))
        };
        Some(Span {
            from: d(f)?,
            to: d(t)?,
        })
    } else {
        let name = arg(&args, "range").unwrap_or("week");
        Span::named(name, today())
            .ok_or_else(|| format!("range {name:?}: today, week, last-week, month or all"))?
    };
    let r = report_now(span);
    let now = Utc::now();
    let mut v = serde_json::to_value(&r).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert(
            "running".to_owned(),
            model::running(&notes)
                .map_or(Value::Null, |(n, e)| entry_json(&n.path, &n.title, e, now)),
        );
        o.insert("total".to_owned(), json!(model::words(r.seconds)));
    }
    Ok(v)
}

/// `TIME_OPEN {view?}`: the report (the default) or the timesheet.
async fn open(args: Value) -> Result<Value, String> {
    match arg(&args, "view").unwrap_or("report") {
        "report" => {
            vault::run(
                "EXT_VIEW_OPEN",
                json!({ "extension": ID, "view": "report" }),
            )
            .await
        }
        "timesheet" => timesheet().await,
        other => Err(format!("view {other:?}: report or timesheet")),
    }
}

/// `TIME_TIMESHEET`: `Time/Timesheet.base` (written the first time),
/// opened.
async fn timesheet() -> Result<Value, String> {
    let path = format!("{FOLDER}/{TIMESHEET}");
    let exists = vault::run("BASES_READ", json!({ "path": path }))
        .await
        .is_ok();
    if !exists {
        vault::run(
            "VAULT_CREATE",
            json!({ "path": path, "body": timesheet_base() }),
        )
        .await?;
        // VAULT_CREATE indexes what it writes as a note: read the folder
        // again so Bases knows the new file is a base.
        vault::run("VAULT_RELOAD", Value::Null).await?;
    }
    let opened = vault::run("BASES_OPEN", json!({ "path": path })).await?;
    Ok(json!({ "path": path, "written": !exists, "opened": opened.get("opened") }))
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeTracker;

impl Extension for TimeTracker {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[(
            "tours/time-tour.styx",
            include_str!("../tours/time-tour.styx"),
        )]
    }

    fn build(&self, ext: &mut Builder) {
        vault::connect(ext.handle());
        // Its navigator provider: running, today, this week, earlier.
        ext.navigator(nav::ACTIONS, nav::TimeNav);
        ext.view("report", ReportView)
            .base_view("timesheet", Timesheet)
            .action("TIME_START", start)
            .action("TIME_STOP", |_| stop())
            .action("TIME_ACTIVE", |_| active())
            .action("TIME_TOGGLE", |_| toggle())
            .action("TIME_LOG", log)
            .action("TIME_CAPTURE", capture)
            .action("TIME_TOTALS", totals)
            .action("TIME_FOR_PROJECT", for_project)
            .action("TIME_REPORT", report)
            .action("TIME_OPEN", open)
            .action("TIME_TIMESHEET", |_| timesheet())
            .action("TIME_STARTED", |_| async {
                let _ = refresh().await;
                Ok(json!("ready"))
            });
    }
}

/// Re-renders the calling view when what it knows changes, and reads
/// again every few seconds while it shows (a running timer's clock, an
/// edit made in the note or on another device).
fn use_time() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let _ = refresh().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(12) {
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
const NUM: &str = "font-variant-numeric: tabular-nums; text-align: right;";

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

fn open_note(path: String) {
    spawn(async move {
        if let Some(h) = vault::handle() {
            let _ = h
                .host(architect_extensions::hostcall::HostRequest::Open { path })
                .await;
        }
    });
}

/// The report: the running timer, a field to log time, totals for the
/// range, then per day, per project and per note.
#[component]
fn ReportView() -> Element {
    let _ = use_time();
    let mut range = use_signal(|| "week".to_owned());
    let mut draft = use_signal(String::new);
    let mut problem = use_signal(|| None::<String>);
    let (notes, error, read) = {
        let c = cache();
        (c.notes.clone(), c.error.clone(), c.read)
    };
    let now = Utc::now();
    let span = Span::named(&range(), today()).unwrap_or(None);
    let r = report_now(span);
    let running = model::running(&notes).map(|(n, e)| (n.path.clone(), n.title.clone(), e.clone()));
    let max_day = r.days.iter().map(|(_, s)| *s).max().unwrap_or(0).max(1);
    let amounts: Vec<String> = {
        let mut by: BTreeMap<String, i64> = BTreeMap::new();
        for p in r.projects.iter().filter(|p| p.cents > 0) {
            *by.entry(p.currency.clone()).or_default() += p.cents;
        }
        by.into_iter()
            .map(|(cur, cents)| model::money(cents, &cur))
            .collect()
    };
    let mut submit = move || {
        let text = draft.peek().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let (length, project, description) = model::log_line(&text);
        let Some(length) = length else {
            problem.set(Some("Say how long: 45m, 1h30m or 1:30.".to_owned()));
            return;
        };
        draft.set(String::new());
        spawn(async move {
            let mut args = json!({ "seconds": length, "description": description });
            if let (Some(p), Some(a)) = (project, args.as_object_mut()) {
                a.insert("project".to_owned(), json!(p));
            }
            match log(args).await {
                Ok(_) => problem.set(None),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let days_shown = r.days.len() <= 31;
    rsx! {
        div { class: "time-report", "data-range": "{range}",
            style: "display: flex; flex-direction: column; gap: 16px; padding: 20px 24px; height: 100%; \
                    box-sizing: border-box; overflow-y: auto; color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;",
                span { style: "font-size: 20px; font-weight: 700;", "Time" }
                span { style: "flex: 1;" }
                for (id, label) in [("today", "Today"), ("week", "This week"), ("last-week", "Last week"), ("month", "This month"), ("all", "All")] {
                    button { key: "{id}", style: chip(range() == id), onclick: move |_| range.set(id.to_owned()), "{label}" }
                }
            }
            // The running timer.
            div { class: "time-running",
                style: "display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 10px; \
                        border: 1px solid var(--border); background: var(--card); color: var(--card-foreground);",
                if let Some((path, title, e)) = running {
                    span { style: "flex: 1; font-weight: 600; cursor: pointer;", onclick: move |_| open_note(path.clone()), "{title}" }
                    span { style: MUTED, "since {hhmm(e.start)}" }
                    span { class: "time-clock", style: "font-size: 18px; font-weight: 700; {NUM}", "{model::clock(e.seconds(now))}" }
                    button {
                        class: "time-stop",
                        style: "padding: 5px 14px; border-radius: 8px; border: none; cursor: pointer; \
                                background: var(--destructive); color: var(--destructive-foreground, white);",
                        onclick: move |_| { spawn(async move { let _ = stop().await; }); },
                        "Stop"
                    }
                } else {
                    span { style: "flex: 1; {MUTED}", "No timer running. Start one from a task, a project, or a row below." }
                }
            }
            input {
                class: "time-log", r#type: "text", "aria-label": "Log time",
                placeholder: "Log time — e.g. 45m Call with the label [[Album]]",
                value: "{draft}",
                style: "height: 36px; box-sizing: border-box; padding: 0 12px; border-radius: 8px; \
                        border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 13px;",
                oninput: move |e: FormEvent| draft.set(e.value()),
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { submit() },
            }
            if let Some(p) = problem() {
                div { style: "color: var(--destructive);", "{p}" }
            }
            if let Some(e) = error {
                div { style: "color: var(--destructive);", "The vault could not be read: {e}" }
            } else if !read {
                div { style: MUTED, "Reading the vault…" }
            }
            // Totals.
            div { class: "time-totals", style: "display: flex; gap: 28px; flex-wrap: wrap;",
                div { style: "display: flex; flex-direction: column;",
                    span { style: HEADING, "Total" }
                    span { class: "time-total", style: "font-size: 22px; font-weight: 700;", "{model::words(r.seconds)}" }
                }
                div { style: "display: flex; flex-direction: column;",
                    span { style: HEADING, "Billable" }
                    span { style: "font-size: 22px; font-weight: 700;", "{model::words(r.billable_seconds)}" }
                }
                if !amounts.is_empty() {
                    div { style: "display: flex; flex-direction: column;",
                        span { style: HEADING, "Amount" }
                        span { style: "font-size: 22px; font-weight: 700;", {amounts.join(" + ")} }
                    }
                }
            }
            if days_shown && !r.days.is_empty() {
                div { class: "time-days", style: "display: flex; flex-direction: column; gap: 3px;",
                    div { style: HEADING, "By day" }
                    for (day, s) in r.days.clone() {
                        div { key: "{day}", style: "display: flex; align-items: center; gap: 10px;",
                            span { style: "width: 110px; {MUTED}", {day_label(&day)} }
                            div { style: "flex: 1; height: 8px; border-radius: 4px; background: var(--muted); overflow: hidden;",
                                div { style: "height: 100%; width: {s * 100 / max_day}%; background: var(--primary);" }
                            }
                            span { style: "width: 60px; {NUM}", "{model::clock(s)}" }
                        }
                    }
                }
            }
            div { class: "time-projects", style: "display: flex; flex-direction: column; gap: 3px;",
                div { style: HEADING, "By project" }
                if r.projects.is_empty() {
                    div { style: MUTED, "No time in this range." }
                }
                for p in r.projects.clone() {
                    div { key: "{p.path}", class: "time-project", "data-path": "{p.path}",
                        style: "display: flex; align-items: center; gap: 10px; padding: 3px 0;",
                        span { style: "flex: 1; font-weight: 600;", "{p.title}" }
                        span { style: "white-space: nowrap; {NUM} {MUTED}",
                            if p.billable_seconds > 0 { "{model::words(p.billable_seconds)} billable" }
                        }
                        span { style: "min-width: 90px; white-space: nowrap; {NUM} {MUTED}",
                            if p.cents > 0 { {model::money(p.cents, &p.currency)} }
                        }
                        span { style: "width: 60px; font-weight: 600; {NUM}", "{model::clock(p.seconds)}" }
                    }
                }
            }
            div { class: "time-notes", style: "display: flex; flex-direction: column; gap: 3px;",
                div { style: HEADING, "By task and note" }
                for n in r.notes {
                    NoteRow { key: "{n.path}", line: n }
                }
            }
        }
    }
}

/// `2026-11-06` as "Fri 6 Nov".
fn day_label(ymd: &str) -> String {
    NaiveDate::parse_from_str(ymd, "%Y-%m-%d")
        .map_or_else(|_| ymd.to_owned(), |d| d.format("%a %-d %b").to_string())
}

/// One note's line in the report: ▶ starts a timer on it.
#[component]
fn NoteRow(line: model::NoteLine) -> Element {
    let model::NoteLine {
        path,
        title,
        kind,
        project,
        seconds,
        ..
    } = line;
    let project = project.as_deref().map(model::project_name);
    let start_args = match kind {
        Kind::Task => json!({ "task": path }),
        Kind::Project => json!({ "project": path }),
        Kind::Note => json!({ "note": path }),
    };
    let label = match kind {
        Kind::Task => "task",
        Kind::Project => "project",
        Kind::Note => "note",
    };
    let p = path.clone();
    rsx! {
        div { class: "time-note", "data-path": "{path}",
            style: "display: flex; align-items: center; gap: 10px; padding: 3px 0;",
            button {
                "aria-label": "Start a timer",
                style: "width: 22px; height: 22px; padding: 0; border: none; border-radius: 5px; cursor: pointer; \
                        background: var(--secondary); color: var(--secondary-foreground); font-size: 10px;",
                onclick: move |_| {
                    let a = start_args.clone();
                    spawn(async move { let _ = start(a).await; });
                },
                "▶"
            }
            span { style: "flex: 1; cursor: pointer;", onclick: move |_| open_note(p.clone()), "{title}" }
            span { style: "font-size: 12px; {MUTED}", "{label}" }
            if let Some(pr) = project {
                span { style: "font-size: 12px; {MUTED}", "▸ {pr}" }
            }
            span { style: "width: 60px; {NUM}", "{model::clock(seconds)}" }
        }
    }
}

/// Its Bases view type (`[[base_views]] id = "timesheet"`): a week, a
/// row per note with time in it and a column per day, totals at the end
/// of each; ‹ and › page through weeks.
#[component]
fn Timesheet() -> Element {
    let Some(input) = architect_extensions::linked::use_base_view() else {
        return rsx! {};
    };
    let mut offset = use_signal(|| 0_i64);
    let notes: Vec<Tracked> = vault::rows_of(&json!({ "rows": input.rows }))
        .into_iter()
        .map(|r| Tracked::from_values(&r.path, &r.name, &r.values))
        .collect();
    let monday = model::week_start(today()) + chrono::Duration::weeks(offset());
    let days: Vec<NaiveDate> = monday.iter_days().take(7).collect();
    let bounds: Vec<(DateTime<Utc>, DateTime<Utc>)> = days
        .iter()
        .map(|d| {
            (
                model::day_start(*d, &Local),
                model::day_start(*d + chrono::Duration::days(1), &Local),
            )
        })
        .collect();
    let now = Utc::now();
    let rows: Vec<(String, String, Vec<i64>)> = notes
        .iter()
        .map(|n| {
            let cells: Vec<i64> = bounds
                .iter()
                .map(|(a, b)| n.seconds_in(*a, *b, now))
                .collect();
            (n.path.clone(), n.title.clone(), cells)
        })
        .filter(|(_, _, cells)| cells.iter().any(|s| *s > 0))
        .collect();
    let day_totals: Vec<i64> = (0..7)
        .map(|i| {
            rows.iter()
                .map(|(_, _, c)| c.get(i).copied().unwrap_or(0))
                .sum()
        })
        .collect();
    let week_total: i64 = day_totals.iter().sum();
    // A narrow pane scrolls the week sideways; nothing wraps.
    let cell = "padding: 6px 8px; border-bottom: 1px solid var(--border); white-space: nowrap;";
    let name = "min-width: 140px; max-width: 220px; overflow: hidden; text-overflow: ellipsis;";
    let arrow = "width: 26px; height: 26px; padding: 0; border: 1px solid var(--border); border-radius: 6px; cursor: pointer; \
                 background: transparent; color: var(--foreground);";
    rsx! {
        div { class: "time-timesheet", "data-week": "{monday}",
            style: "display: flex; flex-direction: column; gap: 10px; padding: 4px; color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: center; gap: 8px;",
                button { style: arrow, "aria-label": "Week before", onclick: move |_| offset -= 1, "‹" }
                span { style: "font-weight: 700;", "Week of {monday.format(\"%-d %b %Y\")}" }
                button { style: arrow, "aria-label": "Week after", onclick: move |_| offset += 1, "›" }
                if offset() != 0 {
                    button { style: chip(false), onclick: move |_| offset.set(0), "This week" }
                }
                span { style: "flex: 1;" }
                span { style: MUTED, "{model::words(week_total)} this week" }
            }
            div { style: "overflow-x: auto;",
            table { style: "border-collapse: collapse; width: 100%;",
                thead {
                    tr {
                        th { style: "{cell} text-align: left; {MUTED}", "Note" }
                        for d in days.iter() {
                            th { key: "{d}", style: "{cell} {NUM} {MUTED} min-width: 44px;", {d.format("%a %-d").to_string()} }
                        }
                        th { style: "{cell} {NUM} min-width: 48px;", "Total" }
                    }
                }
                tbody {
                    for (path, title, cells) in rows {
                        tr { key: "{path}", class: "time-timesheet-row", "data-path": "{path}",
                            td { style: "{cell} {name} cursor: pointer;", title: "{title}", onclick: move |_| open_note(path.clone()), "{title}" }
                            for (i, s) in cells.iter().enumerate() {
                                td { key: "{i}", style: "{cell} {NUM}",
                                    if *s > 0 { "{model::clock(*s)}" } else { span { style: MUTED, "·" } }
                                }
                            }
                            td { style: "{cell} {NUM} font-weight: 600;", {model::clock(cells.iter().sum())} }
                        }
                    }
                    tr {
                        td { style: "{cell} font-weight: 700;", "Total" }
                        for (i, s) in day_totals.iter().enumerate() {
                            td { key: "{i}", style: "{cell} {NUM} font-weight: 700;", "{model::clock(*s)}" }
                        }
                        td { class: "time-timesheet-total", style: "{cell} {NUM} font-weight: 700;", "{model::clock(week_total)}" }
                    }
                }
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
        for source in [tracked_base(), timesheet_base()] {
            let v: serde_yaml::Value = serde_yaml::from_str(&source).unwrap();
            assert!(v.get("views").is_some(), "{source}");
        }
    }

    #[test]
    fn the_status_says_what_runs() {
        let mut values = Map::new();
        values.insert("type".into(), json!("task"));
        values.insert("title".into(), json!("Mix"));
        values.insert(
            "timeEntries".into(),
            json!([{ "startTime": model::stamp(Utc::now()) }]),
        );
        let n = Tracked::from_values("Tasks/Mix.md", "Mix.md", &values);
        assert!(status_text(&[n], Utc::now()).starts_with("Mix · since "));
        assert_eq!(status_text(&[], Utc::now()), "No timer");
    }
}
