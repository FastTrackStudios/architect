//! Time entries, as the notes they live in.
//!
//! Task keeps two kinds of tracked time
//! (`task:features/timer/timer-proto/src/session.rs`): `task::TimeEntry`,
//! the inline list in a task note's frontmatter (`TaskNotes`'
//! `timeEntries`), and `WorkSession`, a database row with a billable
//! flag, a rate and an org. A vault extension reaches notes only, so this
//! is the first, carrying what it can of the second:
//!
//! ```yaml
//! timeEntries:
//!   - startTime: 2026-11-06T09:00:00Z
//!     endTime: 2026-11-06T10:30:00Z
//!     description: Rough mix
//!     billable: true
//! ```
//!
//! An entry with no `endTime` is a running timer; one at a time, as
//! `WorkSession`'s "one open session per user". Entries sit on the note
//! the time was spent on: a task, a project's `project.md`, or
//! `Time/Unfiled.md`. A task's time counts toward its project. Billing is
//! `SolidTime`'s, read off the project note with Task's keys
//! (`billableDefault`, `defaultRateCents`, `currency`).
//!
//! Everything here is pure: no host, no files, the clock and the time
//! zone passed in.

// Second counts and calendar arithmetic on times a person tracks: years
// of seconds stay far inside i64.
#![allow(clippy::arithmetic_side_effects)]

use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The frontmatter keys it reads and writes.
pub mod keys {
    /// `TaskNotes`' list of entries.
    pub const TIME_ENTRIES: &str = "timeEntries";
    pub const START: &str = "startTime";
    pub const END: &str = "endTime";
    pub const DESCRIPTION: &str = "description";
    pub const BILLABLE: &str = "billable";
    pub const TYPE: &str = "type";
    pub const TITLE: &str = "title";
    pub const PROJECTS: &str = "projects";
    pub const STATUS: &str = "status";
    /// A project's billing (Task's `features/project` keys).
    pub const BILLABLE_DEFAULT: &str = "billableDefault";
    pub const RATE_CENTS: &str = "defaultRateCents";
    pub const CURRENCY: &str = "currency";

    /// Every key a tracked note is read with.
    pub const ALL: &[&str] = &[
        TIME_ENTRIES,
        TYPE,
        TITLE,
        PROJECTS,
        STATUS,
        BILLABLE_DEFAULT,
        RATE_CENTS,
        CURRENCY,
    ];
}

/// One tracked period: `endTime` is `None` while it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub description: Option<String>,
    /// As written; `None` takes the project's `billableDefault`.
    pub billable: Option<bool>,
}

/// A time as it is written: RFC 3339, UTC, to the second (Task's
/// `DateTime<Utc>` as serde writes it).
#[must_use]
pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// A written time: RFC 3339 with any offset, or a bare local-less
/// `YYYY-MM-DDTHH:MM[:SS]` read as UTC.
#[must_use]
pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(s, f).ok())
        .map(|n| n.and_utc())
}

impl Entry {
    /// One row of `timeEntries`; `None` for a row with no start it can
    /// read (kept as it is when the list is written back).
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let start = m.get(keys::START)?.as_str().and_then(parse_time)?;
        let end = m
            .get(keys::END)
            .and_then(Value::as_str)
            .and_then(parse_time);
        let description = m
            .get(keys::DESCRIPTION)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let billable = m.get(keys::BILLABLE).and_then(Value::as_bool);
        Some(Self {
            start,
            end,
            description,
            billable,
        })
    }

    /// The row it is written as.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert(keys::START.to_owned(), json!(stamp(self.start)));
        if let Some(e) = self.end {
            m.insert(keys::END.to_owned(), json!(stamp(e)));
        }
        if let Some(d) = &self.description {
            m.insert(keys::DESCRIPTION.to_owned(), json!(d));
        }
        if let Some(b) = self.billable {
            m.insert(keys::BILLABLE.to_owned(), json!(b));
        }
        Value::Object(m)
    }

    #[must_use]
    pub const fn running(&self) -> bool {
        self.end.is_none()
    }

    /// Its length in seconds, a running one's up to `now` (never below
    /// zero).
    #[must_use]
    pub fn seconds(&self, now: DateTime<Utc>) -> i64 {
        (self.end.unwrap_or(now) - self.start).num_seconds().max(0)
    }

    /// The seconds of it inside `[from, to)`.
    #[must_use]
    pub fn seconds_in(&self, from: DateTime<Utc>, to: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
        let start = self.start.max(from);
        let end = self.end.unwrap_or(now).min(to);
        (end - start).num_seconds().max(0)
    }
}

/// The entries of a `timeEntries` value it can read.
#[must_use]
pub fn entries_of(v: &Value) -> Vec<Entry> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Entry::from_value)
        .collect()
}

/// `raw` (a note's `timeEntries` as written) with `entry` added.
#[must_use]
pub fn appended(raw: &Value, entry: &Entry) -> Value {
    let mut rows = raw.as_array().cloned().unwrap_or_default();
    rows.push(entry.to_value());
    Value::Array(rows)
}

/// `raw` with every running entry ended at `now` (an end before its
/// start ends it at its start): the list, and the entries it ended.
#[must_use]
pub fn stopped(raw: &Value, now: DateTime<Utc>) -> (Value, Vec<Entry>) {
    let mut ended = Vec::new();
    let rows = raw
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| match Entry::from_value(row) {
            Some(mut e) if e.running() => {
                e.end = Some(now.max(e.start));
                let mut out = row.clone();
                if let Some(m) = out.as_object_mut() {
                    m.insert(keys::END.to_owned(), json!(stamp(now.max(e.start))));
                }
                ended.push(e);
                out
            }
            _ => row.clone(),
        })
        .collect();
    (Value::Array(rows), ended)
}

/// What a tracked note is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Task,
    Project,
    #[default]
    Note,
}

/// A note with time on it, as a base row reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Tracked {
    pub path: String,
    pub title: String,
    pub kind: Kind,
    /// Its `projects` links (a task's).
    pub projects: Vec<String>,
    pub entries: Vec<Entry>,
    /// Its `timeEntries` as written, rows it cannot read included.
    pub raw: Value,
    /// A project's billing.
    pub billable_default: bool,
    pub rate_cents: Option<i64>,
    pub currency: Option<String>,
}

impl Tracked {
    /// The note at `path` (file name `name`) with properties `values`.
    #[must_use]
    pub fn from_values(path: &str, name: &str, values: &Map<String, Value>) -> Self {
        let text = |k: &str| values.get(k).and_then(scalar).filter(|s| !s.is_empty());
        let kind = match text(keys::TYPE).as_deref() {
            Some("task") => Kind::Task,
            Some("project") => Kind::Project,
            _ if path.ends_with("/project.md") || path == "project.md" => Kind::Project,
            _ => Kind::Note,
        };
        let title = text(keys::TITLE).unwrap_or_else(|| {
            if kind == Kind::Project {
                project_name(path)
            } else {
                name.strip_suffix(".md").unwrap_or(name).to_owned()
            }
        });
        let raw = values
            .get(keys::TIME_ENTRIES)
            .cloned()
            .unwrap_or(Value::Null);
        Self {
            path: path.to_owned(),
            title,
            kind,
            projects: values.get(keys::PROJECTS).map(list_of).unwrap_or_default(),
            entries: entries_of(&raw),
            raw,
            billable_default: values
                .get(keys::BILLABLE_DEFAULT)
                .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true")),
            rate_cents: values.get(keys::RATE_CENTS).and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            }),
            currency: text(keys::CURRENCY),
        }
    }

    /// Its running entry.
    #[must_use]
    pub fn running(&self) -> Option<&Entry> {
        self.entries.iter().find(|e| e.running())
    }

    /// Its seconds inside `[from, to)`.
    #[must_use]
    pub fn seconds_in(&self, from: DateTime<Utc>, to: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
        self.entries
            .iter()
            .map(|e| e.seconds_in(from, to, now))
            .sum()
    }

    /// Every second on it.
    #[must_use]
    pub fn seconds(&self, now: DateTime<Utc>) -> i64 {
        self.entries.iter().map(|e| e.seconds(now)).sum()
    }

    /// The path of the project it counts toward: itself when it is one,
    /// a task's first project link as a path (`[[Projects/A/project|A]]`
    /// → `Projects/A/project.md`) when that link has a folder.
    #[must_use]
    pub fn project_path(&self) -> Option<String> {
        if self.kind == Kind::Project {
            return Some(self.path.clone());
        }
        self.projects.first().and_then(|l| link_path(l))
    }
}

/// A project's name from its `project.md`'s folder.
#[must_use]
pub fn project_name(path: &str) -> String {
    let stem = path.strip_suffix(".md").unwrap_or(path);
    let stem = stem.strip_suffix("/project").unwrap_or(stem);
    stem.rsplit('/').next().unwrap_or(stem).to_owned()
}

/// A frontmatter value as text.
#[must_use]
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A frontmatter value as a list of texts.
#[must_use]
pub fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().filter_map(scalar).collect(),
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// What a link points at: `[[Projects/A/project|A]]` → `Projects/A/project`.
#[must_use]
pub fn link_target(link: &str) -> &str {
    let inner = link
        .trim()
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .unwrap_or_else(|| link.trim());
    let inner = inner.split_once('|').map_or(inner, |(t, _)| t);
    inner.split_once('#').map_or(inner, |(t, _)| t).trim()
}

/// A link with a folder as the note's path (`.md` added); `None` for a
/// bare name, which only the vault (or the Projects extension) resolves.
#[must_use]
pub fn link_path(link: &str) -> Option<String> {
    let t = link_target(link);
    if !t.contains('/') {
        return None;
    }
    Some(if t.to_ascii_lowercase().ends_with(".md") {
        t.to_owned()
    } else {
        format!("{t}.md")
    })
}

/// A link to the note at `path`, shown as `alias`.
#[must_use]
pub fn link_to(path: &str, alias: &str) -> String {
    let target = path.strip_suffix(".md").unwrap_or(path);
    if alias.is_empty() {
        format!("[[{target}]]")
    } else {
        format!("[[{target}|{alias}]]")
    }
}

/// A length as typed: `1h30m`, `1h 30m`, `1h30`, `90m`, `1.5h`, `1:30`,
/// or a bare number of minutes. Its seconds; `None` for anything else or
/// nothing.
#[must_use]
pub fn parse_duration(s: &str) -> Option<i64> {
    let s: String = s
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if let Some((h, m)) = s.split_once(':') {
        let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
        return (m < 60).then_some(h * 3600 + m * 60).filter(|n| *n > 0);
    }
    let mut total = 0_i64;
    let mut num = String::new();
    let mut seen_hours = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let unit = match c {
            'h' => {
                seen_hours = true;
                3600
            }
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        total += scaled(&num, unit)?;
        num.clear();
    }
    if !num.is_empty() {
        // A bare number is minutes; `1h30`'s are minutes after hours.
        if total > 0 && !seen_hours {
            return None;
        }
        total += scaled(&num, 60)?;
    }
    // Past a year is no entry.
    (1..31_622_400).contains(&total).then_some(total)
}

/// `num` (`1`, `1.5`) of `unit` seconds, to the second.
fn scaled(num: &str, unit: i64) -> Option<i64> {
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac = frac.get(..frac.len().min(6)).unwrap_or_default();
    let part = if frac.is_empty() {
        0
    } else {
        let scale = 10_i64.pow(u32::try_from(frac.len()).ok()?);
        (frac.parse::<i64>().ok()? * unit + scale / 2) / scale
    };
    whole.checked_mul(unit)?.checked_add(part)
}

/// A line typed to log time (`45m Call with the label [[Album]]`).
///
/// Its length (the first word that reads as one with a unit, `for:1h30`
/// too; a bare number stays words), its project (the first `[[link]]`)
/// and the rest, its description.
#[must_use]
pub fn log_line(text: &str) -> (Option<i64>, Option<String>, String) {
    let mut rest = text.to_owned();
    let mut project = None;
    if let Some(open) = rest.find("[[")
        && let Some(len) = rest.get(open..).and_then(|s| s.find("]]"))
    {
        let close = open + len + 2;
        project = rest.get(open..close).map(str::to_owned);
        rest.replace_range(open..close, " ");
    }
    let mut length = None;
    let words: Vec<&str> = rest
        .split_whitespace()
        .filter(|w| {
            if length.is_some() {
                return true;
            }
            let bare = w.strip_prefix("for:").unwrap_or(w);
            let has_unit = bare.chars().any(|c| matches!(c, 'h' | 'm' | ':'))
                && bare.chars().next().is_some_and(|c| c.is_ascii_digit());
            let found = parse_duration(bare).filter(|_| has_unit || w.starts_with("for:"));
            if found.is_some() {
                length = found;
            }
            found.is_none()
        })
        .collect();
    (length, project, words.join(" "))
}

/// Seconds as `H:MM` (`1:05`, `0:00`).
#[must_use]
pub fn clock(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// Seconds in words, short: `1h 5m`, `45m`, `0m`.
#[must_use]
pub fn words(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Cents as money: `1234` USD → `USD 12.34`.
#[must_use]
pub fn money(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let c = cents.unsigned_abs();
    let amount = format!("{sign}{}.{:02}", c / 100, c % 100);
    if currency.is_empty() {
        amount
    } else {
        format!("{currency} {amount}")
    }
}

/// What `seconds` bill at `rate_cents` an hour, to the cent.
#[must_use]
pub fn amount(seconds: i64, rate_cents: i64) -> i64 {
    let n = i128::from(seconds) * i128::from(rate_cents);
    let rounded = (n + 1800) / 3600;
    i64::try_from(rounded).unwrap_or(i64::MAX)
}

/// The instant day `d` starts in `tz`.
#[must_use]
pub fn day_start<Tz: TimeZone>(d: NaiveDate, tz: &Tz) -> DateTime<Utc> {
    let midnight = d.and_hms_opt(0, 0, 0).unwrap_or_default();
    tz.from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc(), |t| t.with_timezone(&Utc))
}

/// The Monday of `d`'s week.
#[must_use]
pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(i64::from(d.weekday().num_days_from_monday()))
}

/// A span of days, `[from, to)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl Span {
    /// A named range around `today`: `today`, `week` (Monday on),
    /// `last-week`, `month`, `all` (`None`: every entry).
    #[must_use]
    pub fn named(name: &str, today: NaiveDate) -> Option<Option<Self>> {
        let tomorrow = today + Duration::days(1);
        Some(Some(match name {
            "today" | "day" => Self {
                from: today,
                to: tomorrow,
            },
            "week" | "" => {
                let from = week_start(today);
                Self {
                    from,
                    to: from + Duration::days(7),
                }
            }
            "last-week" => {
                let to = week_start(today);
                Self {
                    from: to - Duration::days(7),
                    to,
                }
            }
            "month" => {
                let from = today.with_day(1).unwrap_or(today);
                let to = if from.month() == 12 {
                    NaiveDate::from_ymd_opt(from.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(from.year(), from.month() + 1, 1)
                }
                .unwrap_or(tomorrow);
                Self { from, to }
            }
            "all" => return Some(None),
            _ => return None,
        }))
    }

    /// Its days, in order.
    #[must_use]
    pub fn days(&self) -> Vec<NaiveDate> {
        self.from.iter_days().take_while(|d| *d < self.to).collect()
    }
}

/// The span every entry of `notes` falls in, as local days.
#[must_use]
pub fn span_of<Tz: TimeZone>(notes: &[Tracked], now: DateTime<Utc>, tz: &Tz) -> Span {
    let day = |t: DateTime<Utc>| t.with_timezone(tz).date_naive();
    let today = day(now);
    let entries = notes.iter().flat_map(|n| &n.entries);
    let from = entries.clone().map(|e| day(e.start)).min().unwrap_or(today);
    let to = entries
        .map(|e| day(e.end.unwrap_or(now)))
        .max()
        .unwrap_or(today)
        .max(today);
    Span {
        from,
        to: to + Duration::days(1),
    }
}

/// The running entry among `notes`, and the note it is on (the latest
/// started, should a synced vault hold two).
#[must_use]
pub fn running(notes: &[Tracked]) -> Option<(&Tracked, &Entry)> {
    notes
        .iter()
        .flat_map(|n| {
            n.entries
                .iter()
                .filter(|e| e.running())
                .map(move |e| (n, e))
        })
        .max_by_key(|(_, e)| e.start)
}

/// One note's line in a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteLine {
    pub path: String,
    pub title: String,
    pub kind: Kind,
    pub project: Option<String>,
    pub seconds: i64,
    pub billable_seconds: i64,
}

/// One project's line in a report: its own time and its tasks'.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectLine {
    /// Its `project.md`; empty for time on no project.
    pub path: String,
    pub title: String,
    pub seconds: i64,
    pub billable_seconds: i64,
    pub rate_cents: i64,
    pub currency: String,
    /// What its billable time comes to.
    pub cents: i64,
}

/// A report over a span: totals per day, per note, per project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Report {
    /// `YYYY-MM-DD`, inclusive; `to` exclusive.
    pub from: String,
    pub to: String,
    pub seconds: i64,
    pub billable_seconds: i64,
    /// `(day, seconds)` for each day of the span.
    pub days: Vec<(String, i64)>,
    pub notes: Vec<NoteLine>,
    pub projects: Vec<ProjectLine>,
}

/// The report of `notes` over `span` (every entry when `None`) in `tz`.
///
/// A task counts toward the project `project_of` names (its path) and is
/// billed at that project's rate. `project_of` is where the Tasks and
/// Projects services come in; [`Tracked::project_path`] alone does for a
/// vault whose links have folders.
#[must_use]
pub fn report<Tz: TimeZone>(
    notes: &[Tracked],
    span: Option<Span>,
    now: DateTime<Utc>,
    tz: &Tz,
    project_of: &dyn Fn(&Tracked) -> Option<String>,
) -> Report {
    let span = span.unwrap_or_else(|| span_of(notes, now, tz));
    let (from, to) = (day_start(span.from, tz), day_start(span.to, tz));
    let by_path = |p: &str| notes.iter().find(|n| n.path == p);
    let mut out = Report {
        from: span.from.to_string(),
        to: span.to.to_string(),
        ..Report::default()
    };
    for d in span.days() {
        let (a, b) = (day_start(d, tz), day_start(d + Duration::days(1), tz));
        out.days.push((
            d.to_string(),
            notes.iter().map(|n| n.seconds_in(a, b, now)).sum(),
        ));
    }
    for n in notes {
        let project = project_of(n);
        let billing = project.as_deref().and_then(by_path);
        let default = billing.is_some_and(|p| p.billable_default);
        let mut seconds = 0;
        let mut billable = 0;
        for e in &n.entries {
            let s = e.seconds_in(from, to, now);
            seconds += s;
            if e.billable.unwrap_or(default) {
                billable += s;
            }
        }
        if seconds == 0 {
            continue;
        }
        out.seconds += seconds;
        out.billable_seconds += billable;
        out.notes.push(NoteLine {
            path: n.path.clone(),
            title: n.title.clone(),
            kind: n.kind,
            project: project.clone(),
            seconds,
            billable_seconds: billable,
        });
        let key = project.clone().unwrap_or_default();
        let line = if let Some(i) = out.projects.iter().position(|p| p.path == key) {
            out.projects.get_mut(i)
        } else {
            out.projects.push(ProjectLine {
                title: if key.is_empty() {
                    "No project".to_owned()
                } else {
                    by_path(&key).map_or_else(|| project_name(&key), |p| p.title.clone())
                },
                rate_cents: billing.and_then(|p| p.rate_cents).unwrap_or(0),
                currency: billing.and_then(|p| p.currency.clone()).unwrap_or_default(),
                path: key,
                ..ProjectLine::default()
            });
            out.projects.last_mut()
        };
        if let Some(line) = line {
            line.seconds += seconds;
            line.billable_seconds += billable;
            line.cents = amount(line.billable_seconds, line.rate_cents);
        }
    }
    out.notes
        .sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.title.cmp(&b.title)));
    out.projects.sort_by(|a, b| {
        a.path
            .is_empty()
            .cmp(&b.path.is_empty())
            .then(b.seconds.cmp(&a.seconds))
            .then(a.title.cmp(&b.title))
    });
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        parse_time(s).unwrap()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn note(path: &str, ty: &str, entries: Value, extra: &Value) -> Tracked {
        let mut values = extra.as_object().cloned().unwrap_or_default();
        values.insert("type".into(), json!(ty));
        values.insert("timeEntries".into(), entries);
        let name = path.rsplit('/').next().unwrap();
        Tracked::from_values(path, name, &values)
    }

    #[test]
    fn durations_read_as_typed() {
        for (s, want) in [
            ("1h30m", 5400),
            ("1h 30m", 5400),
            ("1h30", 5400),
            ("90m", 5400),
            ("1.5h", 5400),
            ("1:30", 5400),
            ("45", 2700),
            ("2h", 7200),
            ("20s", 20),
        ] {
            assert_eq!(parse_duration(s), Some(want), "{s}");
        }
        for s in ["", "soon", "0", "1:75", "30x", "h"] {
            assert_eq!(parse_duration(s), None, "{s}");
        }
        assert_eq!(
            log_line("45m Call with the label [[Album Two]]"),
            (
                Some(2700),
                Some("[[Album Two]]".into()),
                "Call with the label".into()
            )
        );
        assert_eq!(
            log_line("Call 3 people for:1h30"),
            (Some(5400), None, "Call 3 people".into()),
            "a bare number is words"
        );
        assert_eq!(log_line("Mixing"), (None, None, "Mixing".into()));
        assert_eq!(clock(3900), "1:05");
        assert_eq!(words(3900), "1h 5m");
        assert_eq!(words(7200), "2h");
        assert_eq!(words(59), "0m");
        assert_eq!(money(12_345, "USD"), "USD 123.45");
        assert_eq!(amount(5400, 10_000), 15_000, "1.5h at 100.00");
    }

    #[test]
    fn entries_round_trip_and_stop() {
        let raw = json!([
            { "startTime": "2026-11-06T09:00:00Z", "endTime": "2026-11-06T10:00:00Z", "mood": "good" },
            { "startTime": "nonsense" },
            { "startTime": "2026-11-06T11:00:00+01:00", "description": "Mix" },
        ]);
        let entries = entries_of(&raw);
        assert_eq!(entries.len(), 2, "the unreadable row is skipped");
        assert_eq!(entries[1].start, t("2026-11-06T10:00:00Z"), "offset read");
        assert!(entries[1].running());
        let now = t("2026-11-06T10:45:00Z");
        assert_eq!(entries[1].seconds(now), 2700);
        let (after, ended) = stopped(&raw, now);
        assert_eq!(ended.len(), 1);
        assert_eq!(after[2]["endTime"], "2026-11-06T10:45:00Z");
        assert_eq!(after[0]["mood"], "good", "keys it does not know survive");
        assert_eq!(after[1]["startTime"], "nonsense", "rows it cannot read too");
        let added = appended(
            &after,
            &Entry {
                start: now,
                end: None,
                description: None,
                billable: Some(true),
            },
        );
        assert_eq!(added.as_array().unwrap().len(), 4);
        assert_eq!(
            added[3],
            json!({ "startTime": "2026-11-06T10:45:00Z", "billable": true })
        );
    }

    #[test]
    fn spans_and_weeks() {
        let fri = date("2026-11-06");
        assert_eq!(week_start(fri), date("2026-11-02"));
        let w = Span::named("week", fri).unwrap().unwrap();
        assert_eq!(w.days().len(), 7);
        let m = Span::named("month", date("2026-12-15")).unwrap().unwrap();
        assert_eq!((m.from, m.to), (date("2026-12-01"), date("2027-01-01")));
        assert_eq!(Span::named("all", fri), Some(None));
        assert_eq!(Span::named("fortnight", fri), None);
    }

    #[test]
    fn a_report_totals_by_day_note_and_project() {
        let tz = FixedOffset::east_opt(3600).unwrap();
        let project = note(
            "Projects/Album/project.md",
            "project",
            json!([{ "startTime": "2026-11-02T08:00:00Z", "endTime": "2026-11-02T09:00:00Z" }]),
            &json!({ "billableDefault": true, "defaultRateCents": 6000, "currency": "USD" }),
        );
        assert_eq!(project.title, "Album");
        let task = note(
            "Projects/Album/Tasks/Mix.md",
            "task",
            json!([
                // Across local midnight (UTC+1): 30m on Mon, 30m on Tue.
                { "startTime": "2026-11-02T22:30:00Z", "endTime": "2026-11-02T23:30:00Z" },
                { "startTime": "2026-11-06T09:00:00Z", "billable": false },
            ]),
            &json!({ "projects": ["[[Projects/Album/project|Album]]"] }),
        );
        let loose = note(
            "Time/Unfiled.md",
            "",
            json!([{ "startTime": "2026-10-30T09:00:00Z", "endTime": "2026-10-30T10:00:00Z" }]),
            &json!({}),
        );
        let notes = [project, task, loose];
        let now = t("2026-11-06T09:30:00Z");
        let (n, e) = running(&notes).unwrap();
        assert_eq!(n.path, "Projects/Album/Tasks/Mix.md");
        assert_eq!(e.start, t("2026-11-06T09:00:00Z"));

        let week = Span::named("week", date("2026-11-06")).unwrap();
        let r = report(&notes, week, now, &tz, &Tracked::project_path);
        assert_eq!(r.from, "2026-11-02");
        assert_eq!(r.seconds, 3600 + 3600 + 1800, "last week's hour is out");
        assert_eq!(r.days[0], ("2026-11-02".into(), 3600 + 1800));
        assert_eq!(r.days[1], ("2026-11-03".into(), 1800));
        assert_eq!(r.days[4], ("2026-11-06".into(), 1800), "running, to now");
        assert_eq!(r.projects.len(), 1);
        let album = &r.projects[0];
        assert_eq!(album.path, "Projects/Album/project.md");
        assert_eq!(album.seconds, 9000, "its own hour and its task's");
        assert_eq!(
            album.billable_seconds, 7200,
            "the entry marked not billable is not"
        );
        assert_eq!(album.cents, 12_000);
        assert_eq!(r.notes[0].path, "Projects/Album/Tasks/Mix.md");

        let all = report(&notes, None, now, &tz, &Tracked::project_path);
        assert_eq!(all.from, "2026-10-30");
        assert_eq!(all.projects.last().unwrap().title, "No project");
    }
}
