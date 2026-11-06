//! Events as vault notes, Task's shape (`task:features/scheduling/scheduling-proto/src/cal_event.rs`
//! `CalEvent`), and what a calendar draws: occurrences.
//!
//! An event is a note whose frontmatter says when:
//!
//! ```yaml
//! type: scheduling-cal-event
//! id: album-release-show
//! title: Album release — show
//! start: 2026-12-05T20:00:00-06:00
//! end: 2026-12-05T22:30:00-06:00
//! location: The Hall
//! for:
//!   - '[[Gigs/album-release]]'
//! ```
//!
//! `start` is inclusive and `end` exclusive, RFC 3339 with the offset
//! they were made in; an all-day event's are dates (`2026-11-06`, ending
//! `2026-11-07`). Any property that is a wikilink, or a list of them, is
//! a link to that note, under its key (`for`, `with`). Task's `type: event`
//! reads too.
//!
//! **Repeats** (`recurrence`, an RRULE, [`crate::recur`]) expand in the
//! person's time zone. Unlike Task's, a series has **exceptions**:
//! `exdates` lists the days of occurrences that are skipped, and an
//! occurrence moved or changed alone is a note of its own, an override,
//! that names its series (`recurrence_id`, the series' `id`) and the day
//! it replaces (`recurrence_date`) — RFC 5545's `EXDATE` and
//! `RECURRENCE-ID`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::recur::Rule;

/// Its frontmatter keys (Task's `CalEvent::RESERVED_KEYS` and ours).
pub mod keys {
    pub const TYPE: &str = "type";
    pub const ID: &str = "id";
    pub const TITLE: &str = "title";
    pub const START: &str = "start";
    pub const END: &str = "end";
    pub const ALL_DAY: &str = "all_day";
    pub const COLOR: &str = "color";
    pub const DESCRIPTION: &str = "description";
    pub const RECURRENCE: &str = "recurrence";
    pub const LOCATION: &str = "location";
    pub const EXTERNAL_REF: &str = "external_ref";
    pub const TOMBSTONE: &str = "tombstone";
    /// The days of a series' skipped occurrences.
    pub const EXDATES: &str = "exdates";
    /// An override's series (its `id`).
    pub const RECURRENCE_ID: &str = "recurrence_id";
    /// The day of the occurrence an override replaces.
    pub const RECURRENCE_DATE: &str = "recurrence_date";

    /// Keys that are never links.
    pub const RESERVED: &[&str] = &[
        TYPE,
        ID,
        TITLE,
        START,
        END,
        ALL_DAY,
        COLOR,
        DESCRIPTION,
        RECURRENCE,
        LOCATION,
        EXTERNAL_REF,
        TOMBSTONE,
        EXDATES,
        RECURRENCE_ID,
        RECURRENCE_DATE,
        "tags",
        "aliases",
        "cssclasses",
    ];
}

/// The `type` it writes (Task's).
pub const TYPE: &str = "scheduling-cal-event";

/// The types it reads as events.
pub const TYPES: &[&str] = &[TYPE, "event"];

/// Task's colours (`ColorTag`), with what they draw as.
pub const COLORS: &[(&str, &str)] = &[
    ("primary", "var(--primary)"),
    ("success", "#2f9e61"),
    ("warning", "#d08a16"),
    ("danger", "#d64545"),
    ("info", "#2f7fd0"),
    ("muted", "#7a8190"),
];

/// What a colour's name draws as (`primary` for a name it does not know;
/// a `#rrggbb` as it is).
#[must_use]
pub fn color_css(name: &str) -> String {
    if name.starts_with('#') && name.len() <= 9 {
        return name.to_owned();
    }
    COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or_else(|| "var(--primary)".to_owned(), |(_, css)| (*css).to_owned())
}

/// When: a day, or a time with its offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Day(NaiveDate),
    At(DateTime<FixedOffset>),
}

impl When {
    /// Reads `2026-11-06`, `2026-11-06T14:00:00-06:00`, `…Z`, or a time
    /// with no offset (`2026-11-06T14:00`, `2026-11-06 14:00`), which is
    /// `tz`'s.
    #[must_use]
    pub fn parse<Tz: TimeZone>(text: &str, tz: &Tz) -> Option<Self> {
        let t = text.trim().trim_matches('"').trim_matches('\'');
        if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
            return Some(Self::Day(d));
        }
        if let Ok(at) = DateTime::parse_from_rfc3339(t) {
            return Some(Self::At(at));
        }
        for f in [
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%dT%H:%M",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d %H:%M",
        ] {
            if let Ok(n) = NaiveDateTime::parse_from_str(t, f) {
                return Some(Self::At(fixed(tz, n)));
            }
        }
        None
    }

    /// Its wall-clock time in `tz` (a day's midnight).
    #[must_use]
    pub fn local<Tz: TimeZone>(&self, tz: &Tz) -> NaiveDateTime {
        match self {
            Self::Day(d) => d.and_time(NaiveTime::MIN),
            Self::At(at) => at.with_timezone(tz).naive_local(),
        }
    }

    /// As frontmatter writes it.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Day(d) => d.format("%Y-%m-%d").to_string(),
            Self::At(at) => at.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        }
    }
}

/// `n`, a wall-clock time in `tz`, with its offset (the earlier of two in
/// a clock change's repeated hour, the hour after one skipped).
#[must_use]
pub fn fixed<Tz: TimeZone>(tz: &Tz, n: NaiveDateTime) -> DateTime<FixedOffset> {
    let pick = tz
        .from_local_datetime(&n)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(n + Duration::hours(1))).earliest());
    pick.map_or_else(|| n.and_utc().fixed_offset(), |t| t.fixed_offset())
}

/// The text a time is written as: a day's date when `all_day`, else the
/// time in `tz` with its offset.
#[must_use]
pub fn when_text<Tz: TimeZone>(tz: &Tz, at: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        When::Day(at.date()).text()
    } else {
        When::At(fixed(tz, at)).text()
    }
}

/// A link from an event to a note: its key, and the note (`[[Gigs/x]]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Link {
    pub relation: String,
    pub target: String,
}

/// An event note, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub path: String,
    pub id: String,
    pub title: String,
    pub start: When,
    /// Exclusive; never before `start`.
    pub end: When,
    pub all_day: bool,
    pub color: String,
    pub description: Option<String>,
    pub recurrence: Option<String>,
    pub exdates: BTreeSet<NaiveDate>,
    /// An override's series.
    pub recurrence_id: Option<String>,
    pub recurrence_date: Option<NaiveDate>,
    pub location: Option<String>,
    pub links: Vec<Link>,
    pub external_ref: Option<String>,
    pub tombstone: Option<String>,
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn texts_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(text_of).collect(),
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether `s` is a wikilink (`[[Note]]`, `[[Note|alias]]`).
fn is_wikilink(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("[[") && s.ends_with("]]") && s.len() > 4
}

/// `Gigs/x`, `Gigs/x.md` or `[[Gigs/x]]` as a wikilink (Task's
/// `EventLink::new`).
#[must_use]
pub fn wikilink(target: &str) -> String {
    let t = target.trim();
    if is_wikilink(t) {
        return t.to_owned();
    }
    format!("[[{}]]", t.strip_suffix(".md").unwrap_or(t))
}

/// A link's note, `Gigs/x` from `[[Gigs/x|alias]]`.
#[must_use]
pub fn link_note(link: &str) -> &str {
    let inner = link.trim().trim_start_matches("[[").trim_end_matches("]]");
    inner.split(['|', '#']).next().unwrap_or(inner)
}

/// Whether `relation` can be a link's key: a slug that is not an event's
/// own key (Task's `EventLink::valid_relation`).
#[must_use]
pub fn valid_relation(relation: &str) -> bool {
    let mut chars = relation.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !keys::RESERVED.contains(&relation)
}

/// A note's file name without its folders and `.md`.
#[must_use]
pub fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

impl Event {
    /// The event a note's properties make, or `None` when they do not say
    /// when (Task's `parse/cal_event.rs`: the id falls back to the file's
    /// name, the title to it too, the colour to `primary`).
    #[must_use]
    pub fn from_values<Tz: TimeZone>(
        path: &str,
        values: &Map<String, Value>,
        tz: &Tz,
    ) -> Option<Self> {
        let get = |k: &str| values.get(k).and_then(text_of);
        let all_day_flag = values.get(keys::ALL_DAY).is_some_and(|v| match v {
            Value::Bool(b) => *b,
            Value::String(s) => s.eq_ignore_ascii_case("true"),
            _ => false,
        });
        let start = When::parse(&get(keys::START)?, tz)?;
        let all_day = all_day_flag || matches!(start, When::Day(_));
        let start = if all_day {
            When::Day(start.local(tz).date())
        } else {
            start
        };
        let end = get(keys::END).and_then(|e| When::parse(&e, tz));
        let end = match (all_day, end) {
            (true, Some(e)) => {
                let d = e.local(tz).date();
                // An all-day end is the day after its last.
                When::Day(if d <= start.local(tz).date() {
                    start.local(tz).date() + Duration::days(1)
                } else {
                    d
                })
            }
            (true, None) => When::Day(start.local(tz).date() + Duration::days(1)),
            (false, Some(e)) if e.local(tz) > start.local(tz) => e,
            (false, _) => When::At(fixed(tz, start.local(tz) + Duration::hours(1))),
        };
        let mut links = Vec::new();
        for (k, v) in values {
            if keys::RESERVED.contains(&k.as_str()) {
                continue;
            }
            let targets = texts_of(v);
            if !targets.is_empty()
                && targets.iter().all(|t| is_wikilink(t))
                && matches!(v, Value::String(_) | Value::Array(_))
            {
                for target in targets {
                    links.push(Link {
                        relation: k.clone(),
                        target,
                    });
                }
            }
        }
        Some(Self {
            path: path.to_owned(),
            id: get(keys::ID).unwrap_or_else(|| stem(path).to_owned()),
            title: get(keys::TITLE).unwrap_or_else(|| stem(path).to_owned()),
            start,
            end,
            all_day,
            color: get(keys::COLOR).unwrap_or_else(|| "primary".to_owned()),
            description: get(keys::DESCRIPTION),
            recurrence: get(keys::RECURRENCE),
            exdates: values
                .get(keys::EXDATES)
                .map(texts_of)
                .unwrap_or_default()
                .iter()
                .filter_map(|d| When::parse(d, tz))
                .map(|w| w.local(tz).date())
                .collect(),
            recurrence_id: get(keys::RECURRENCE_ID),
            recurrence_date: get(keys::RECURRENCE_DATE)
                .and_then(|d| When::parse(&d, tz))
                .map(|w| w.local(tz).date()),
            location: get(keys::LOCATION),
            links,
            external_ref: get(keys::EXTERNAL_REF),
            tombstone: get(keys::TOMBSTONE),
        })
    }

    /// Its frontmatter, Task's keys (`write/cal_event.rs`): what a new
    /// note is written with. The vault writes a new note's keys in
    /// alphabetical order; a rewrite keeps a note's own order.
    #[must_use]
    pub fn properties(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(keys::TYPE.into(), json!(TYPE));
        m.insert(keys::ID.into(), json!(self.id));
        m.insert(keys::TITLE.into(), json!(self.title));
        m.insert(keys::START.into(), json!(self.start.text()));
        m.insert(keys::END.into(), json!(self.end.text()));
        m.insert(keys::ALL_DAY.into(), json!(self.all_day));
        m.insert(keys::COLOR.into(), json!(self.color));
        let opt = |m: &mut Map<String, Value>, k: &str, v: &Option<String>| {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        };
        opt(&mut m, keys::DESCRIPTION, &self.description);
        opt(&mut m, keys::RECURRENCE, &self.recurrence);
        if !self.exdates.is_empty() {
            m.insert(keys::EXDATES.into(), exdates_value(&self.exdates));
        }
        opt(&mut m, keys::RECURRENCE_ID, &self.recurrence_id);
        if let Some(d) = self.recurrence_date {
            m.insert(keys::RECURRENCE_DATE.into(), json!(d.to_string()));
        }
        opt(&mut m, keys::LOCATION, &self.location);
        opt(&mut m, keys::EXTERNAL_REF, &self.external_ref);
        opt(&mut m, keys::TOMBSTONE, &self.tombstone);
        let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for l in &self.links {
            by.entry(&l.relation).or_default().push(&l.target);
        }
        for (k, targets) in by {
            m.insert(k.into(), json!(targets));
        }
        m
    }

    /// Its length.
    #[must_use]
    pub fn length<Tz: TimeZone>(&self, tz: &Tz) -> Duration {
        self.end.local(tz) - self.start.local(tz)
    }

    /// Whether it is a series (a rule this reads).
    #[must_use]
    pub fn repeats(&self) -> bool {
        self.recurrence.as_deref().and_then(Rule::parse).is_some()
    }

    /// Whether it links to `note` (a path, a name, a link; Task's
    /// `CalEvent::links_to`).
    #[must_use]
    pub fn links_to(&self, note: &str) -> bool {
        let want = link_note(note);
        let want = want.strip_suffix(".md").unwrap_or(want);
        self.links.iter().any(|l| {
            let to = link_note(&l.target);
            to == want || stem(to) == stem(want) || to.rsplit('/').next() == Some(want)
        })
    }

    /// What `CALENDAR_LIST` and the service say of it.
    #[must_use]
    pub fn json<Tz: TimeZone>(&self, tz: &Tz) -> Value {
        json!({
            "path": self.path,
            "id": self.id,
            "title": self.title,
            "start": self.start.text(),
            "end": self.end.text(),
            "all_day": self.all_day,
            "color": self.color,
            "description": self.description,
            "recurrence": self.recurrence,
            "repeats": self.recurrence.as_deref().map(crate::recur::describe),
            "exdates": self.exdates.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "recurrence_id": self.recurrence_id,
            "recurrence_date": self.recurrence_date.map(|d| d.to_string()),
            "location": self.location,
            "links": self.links,
            "external_ref": self.external_ref,
            "tombstone": self.tombstone,
            "minutes": self.length(tz).num_minutes(),
        })
    }
}

/// `exdates` as frontmatter writes it: a list of days.
#[must_use]
pub fn exdates_value(days: &BTreeSet<NaiveDate>) -> Value {
    Value::Array(days.iter().map(|d| json!(d.to_string())).collect())
}

/// Where a block on the calendar comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// An event, or one occurrence of a series: its note, its `id`, and
    /// the day it was due on (an override's `recurrence_date`).
    Event {
        path: String,
        id: String,
        /// Of a series (the master's, or an override of one).
        series: bool,
        /// The series it belongs to, for an override.
        master: Option<String>,
        date: NaiveDate,
        /// From a subscription or an import: changed there, not here.
        readonly: bool,
    },
    /// A task's due or scheduled date (the Tasks extension's).
    Task {
        path: String,
        done: bool,
        field: String,
    },
    /// Time tracked (the Time Tracker's): the note it is on.
    Time { path: String, running: bool },
    /// A booking of an event type ([`crate::booking`]).
    Booking { path: String, status: String },
}

impl Source {
    /// The note it opens.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Event { path, .. }
            | Self::Task { path, .. }
            | Self::Time { path, .. }
            | Self::Booking { path, .. } => path,
        }
    }
}

/// One thing on the calendar: an occurrence of an event, a task on its
/// day, a span of tracked time. Times are wall-clock, the person's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Block {
    /// Unique among those drawn: its note, and the day for a series'.
    pub key: String,
    pub title: String,
    pub color: String,
    pub start: NaiveDateTime,
    /// Exclusive.
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: Option<String>,
    /// A series' rule, in words.
    pub repeats: Option<String>,
    pub source: Source,
}

impl Block {
    /// Whether it touches `[from, to)`.
    #[must_use]
    pub fn overlaps(&self, from: NaiveDateTime, to: NaiveDateTime) -> bool {
        self.start < to && (self.end > from || (self.end == self.start && self.start >= from))
    }

    /// Whether it sits in the all-day strip: all day, or a day or more
    /// from a midnight (Task's `is_effectively_all_day`).
    #[must_use]
    pub fn in_strip(&self) -> bool {
        self.all_day
            || (self.start.time() == NaiveTime::MIN && self.end - self.start >= Duration::hours(24))
    }

    /// Whether it can be moved here: an event's, or a task's.
    #[must_use]
    pub const fn movable(&self) -> bool {
        !matches!(
            self.source,
            Source::Time { .. } | Source::Booking { .. } | Source::Event { readonly: true, .. }
        )
    }
}

/// The events' blocks that touch `[from, to)`: each one-off, each
/// occurrence of a series (less its `exdates` and the days an override
/// replaces), each override; not the tombstoned. In time order.
#[must_use]
pub fn occurrences<Tz: TimeZone>(
    events: &[Event],
    from: NaiveDateTime,
    to: NaiveDateTime,
    tz: &Tz,
) -> Vec<Block> {
    let live: Vec<&Event> = events.iter().filter(|e| e.tombstone.is_none()).collect();
    let replaced: BTreeSet<(&str, NaiveDate)> = live
        .iter()
        .filter_map(|e| Some((e.recurrence_id.as_deref()?, e.recurrence_date?)))
        .collect();
    // Overrides whose series is gone show anyway.
    let mut out = Vec::new();
    for e in &live {
        let start = e.start.local(tz);
        let length = e.length(tz);
        let block = |at: NaiveDateTime, series: bool| Block {
            key: if series && e.recurrence_id.is_none() {
                format!("{}@{}", e.path, at.date())
            } else {
                e.path.clone()
            },
            title: e.title.clone(),
            color: e.color.clone(),
            start: at,
            end: at + length,
            all_day: e.all_day,
            location: e.location.clone(),
            repeats: e.recurrence.as_deref().map(crate::recur::describe),
            source: Source::Event {
                path: e.path.clone(),
                id: e.id.clone(),
                series,
                master: e.recurrence_id.clone(),
                date: e.recurrence_date.unwrap_or_else(|| at.date()),
                readonly: e
                    .external_ref
                    .as_deref()
                    .is_some_and(|r| r.starts_with("ics:")),
            },
        };
        match e.recurrence.as_deref().and_then(Rule::parse) {
            Some(rule) if e.recurrence_id.is_none() => {
                // From early enough that one running into the window shows.
                for at in rule.between(start, from - length, to) {
                    let day = at.date();
                    if e.exdates.contains(&day) || replaced.contains(&(e.id.as_str(), day)) {
                        continue;
                    }
                    let b = block(at, true);
                    if b.overlaps(from, to) {
                        out.push(b);
                    }
                }
            }
            // A bad rule shows the first only, as Task's does.
            _ => {
                let b = block(start, e.recurrence_id.is_some() || e.repeats());
                if b.overlaps(from, to) {
                    out.push(b);
                }
            }
        }
    }
    sort(&mut out);
    out
}

/// In time order: all-day first on a day, then by start, the longer
/// first, then by title.
pub fn sort(blocks: &mut [Block]) {
    blocks.sort_by(|a, b| {
        a.start
            .date()
            .cmp(&b.start.date())
            .then_with(|| b.in_strip().cmp(&a.in_strip()))
            .then_with(|| a.start.cmp(&b.start))
            .then_with(|| (b.end - b.start).cmp(&(a.end - a.start)))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

/// A task's block, from the Tasks extension's JSON (`TASKS_LIST`'s rows).
///
/// On its `scheduled` day, else its `due` one; at its time when
/// the date has one, for its `timeEstimate` (30 minutes without), else
/// all day. `None` when it has neither date.
#[must_use]
pub fn task_block<Tz: TimeZone>(task: &Value, tz: &Tz) -> Option<Block> {
    let path = task.get("path")?.as_str()?.to_owned();
    let (field, text) = ["scheduled", "due"].into_iter().find_map(|f| {
        Some((
            f,
            task.get(f)?.as_str().filter(|s| !s.is_empty())?.to_owned(),
        ))
    })?;
    let when = When::parse(&text, tz)?;
    let done = task.get("open").and_then(Value::as_bool).map_or_else(
        || {
            task.get("status")
                .and_then(Value::as_str)
                .is_some_and(|s| matches!(s, "done" | "completed" | "cancelled"))
        },
        |open| !open,
    );
    let (start, end, all_day) = match when {
        When::Day(d) => (
            d.and_time(NaiveTime::MIN),
            d.and_time(NaiveTime::MIN) + Duration::days(1),
            true,
        ),
        When::At(_) => {
            let s = when.local(tz);
            let minutes = task
                .get("time_estimate")
                .and_then(Value::as_i64)
                .filter(|m| *m > 0)
                .unwrap_or(30);
            (s, s + Duration::minutes(minutes), false)
        }
    };
    Some(Block {
        key: format!("task:{path}"),
        title: task
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_else(|| stem(&path))
            .to_owned(),
        color: "muted".to_owned(),
        start,
        end,
        all_day,
        location: None,
        repeats: task
            .get("recurrence")
            .and_then(Value::as_str)
            .map(crate::recur::describe),
        source: Source::Task {
            path,
            done,
            field: field.to_owned(),
        },
    })
}

/// A tracked span's block, from the Time Tracker's `TIME_ENTRIES` rows
/// (`{path, title, start, end, running}`, RFC 3339); a running one runs
/// to `now`.
#[must_use]
pub fn time_block<Tz: TimeZone>(entry: &Value, now: NaiveDateTime, tz: &Tz) -> Option<Block> {
    let path = entry.get("path")?.as_str()?.to_owned();
    let start = When::parse(entry.get("start")?.as_str()?, tz)?.local(tz);
    let running = entry.get("end").and_then(Value::as_str).is_none();
    let end = entry
        .get("end")
        .and_then(Value::as_str)
        .and_then(|e| When::parse(e, tz))
        .map_or(now, |w| w.local(tz))
        .max(start + Duration::minutes(1));
    let title = entry
        .get("description")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .or_else(|| entry.get("title").and_then(Value::as_str))
        .unwrap_or_else(|| stem(&path))
        .to_owned();
    Some(Block {
        key: format!("time:{path}@{start}"),
        title,
        color: "muted".to_owned(),
        start,
        end,
        all_day: false,
        location: None,
        repeats: None,
        source: Source::Time { path, running },
    })
}

/// A file name for an event titled `title`, its day first.
///
/// Task's `sanitize`: what is not `[A-Za-z0-9._-]` becomes `-`), its day first so a folder of them
/// sorts by date: `2026-11-06 Lunch with Sam`. Spaces are kept, as a
/// note's name has them.
#[must_use]
pub fn file_name(title: &str, day: NaiveDate) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " ._-()&,'".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.');
    let clean: String = clean.chars().take(80).collect();
    if clean.is_empty() {
        format!("{day} Event")
    } else {
        format!("{day} {clean}")
    }
}

/// A new event's `id`: its name as a slug, made unique by the caller.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "event".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn tz() -> FixedOffset {
        FixedOffset::west_opt(6 * 3600).unwrap()
    }

    fn values(yaml: &[(&str, Value)]) -> Map<String, Value> {
        yaml.iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn task_s_example_reads() {
        let e = Event::from_values(
            "Records/events/example-album-release-show.md",
            &values(&[
                ("type", json!("scheduling-cal-event")),
                ("id", json!("example-album-release-show")),
                ("title", json!("Example Album release — show")),
                ("start", json!("2026-12-05T20:00:00-06:00")),
                ("end", json!("2026-12-05T22:30:00-06:00")),
                ("location", json!("The Hall")),
                ("for", json!(["[[Gigs/example-album-release]]"])),
                ("mood", json!("loud")),
            ]),
            &tz(),
        )
        .unwrap();
        assert_eq!(e.length(&tz()), Duration::minutes(150));
        assert_eq!(e.start.local(&tz()), at("2026-12-05 20:00"));
        assert_eq!(
            e.links,
            [Link {
                relation: "for".into(),
                target: "[[Gigs/example-album-release]]".into()
            }]
        );
        assert!(e.links_to("Gigs/example-album-release.md"));
        assert!(e.links_to("[[example-album-release]]"));
        assert!(!e.all_day);
        // What it writes back holds what it read (less the foreign key,
        // which a rewrite leaves where it is).
        let again = Event::from_values(&e.path, &e.properties(), &tz()).unwrap();
        assert_eq!(again, e);
    }

    #[test]
    fn lenient_defaults() {
        let e = Event::from_values(
            "Calendar/Dentist.md",
            &values(&[("start", json!("2026-11-10"))]),
            &tz(),
        )
        .unwrap();
        assert_eq!(e.id, "Dentist");
        assert_eq!(e.title, "Dentist");
        assert!(e.all_day);
        assert_eq!(
            e.end,
            When::Day(NaiveDate::from_ymd_opt(2026, 11, 11).unwrap())
        );
        assert_eq!(e.color, "primary");
        // No start, no event.
        assert!(Event::from_values("x.md", &values(&[("title", json!("x"))]), &tz()).is_none());
        // A timed one with no end lasts an hour; an all_day one stored with
        // times is its days.
        let e = Event::from_values(
            "y.md",
            &values(&[("start", json!("2026-11-10T09:00"))]),
            &tz(),
        )
        .unwrap();
        assert_eq!(e.length(&tz()), Duration::hours(1));
        let e = Event::from_values(
            "z.md",
            &values(&[
                ("start", json!("2026-11-10T00:00:00-06:00")),
                ("end", json!("2026-11-12T00:00:00-06:00")),
                ("all_day", json!(true)),
            ]),
            &tz(),
        )
        .unwrap();
        assert_eq!(
            (e.start.text(), e.end.text()),
            ("2026-11-10".into(), "2026-11-12".into())
        );
    }

    fn weekly() -> Event {
        Event::from_values(
            "Calendar/Standup.md",
            &values(&[
                ("id", json!("standup")),
                ("start", json!("2026-11-02T09:00:00-06:00")),
                ("end", json!("2026-11-02T09:15:00-06:00")),
                ("recurrence", json!("FREQ=WEEKLY;BYDAY=MO,WE,FR")),
                ("exdates", json!(["2026-11-04"])),
            ]),
            &tz(),
        )
        .unwrap()
    }

    #[test]
    fn a_series_with_exceptions() {
        let moved = Event::from_values(
            "Calendar/Standup 2026-11-06.md",
            &values(&[
                ("title", json!("Standup (late)")),
                ("start", json!("2026-11-06T11:00:00-06:00")),
                ("end", json!("2026-11-06T11:30:00-06:00")),
                ("recurrence_id", json!("standup")),
                ("recurrence_date", json!("2026-11-06")),
            ]),
            &tz(),
        )
        .unwrap();
        let blocks = occurrences(
            &[weekly(), moved],
            at("2026-11-02 00:00"),
            at("2026-11-09 00:00"),
            &tz(),
        );
        let shown: Vec<(String, String)> = blocks
            .iter()
            .map(|b| (b.start.format("%a %H:%M").to_string(), b.title.clone()))
            .collect();
        // Wednesday skipped, Friday moved to 11:00.
        assert_eq!(
            shown,
            [
                ("Mon 09:00".into(), "Standup".into()),
                ("Fri 11:00".into(), "Standup (late)".into())
            ]
        );
        assert!(
            matches!(&blocks[1].source, Source::Event { series: true, master: Some(m), .. } if m == "standup")
        );
        assert_eq!(blocks[0].key, "Calendar/Standup.md@2026-11-02");
    }

    #[test]
    fn tombstones_and_bad_rules() {
        let mut gone = weekly();
        gone.tombstone = Some(Utc::now().to_rfc3339());
        assert!(
            occurrences(
                &[gone],
                at("2026-11-01 00:00"),
                at("2026-12-01 00:00"),
                &tz()
            )
            .is_empty()
        );
        let mut bad = weekly();
        bad.recurrence = Some("FREQ=SECONDLY".into());
        assert_eq!(
            occurrences(
                &[bad],
                at("2026-11-01 00:00"),
                at("2026-12-01 00:00"),
                &tz()
            )
            .len(),
            1
        );
    }

    #[test]
    fn one_running_into_the_window_shows() {
        let e = Event::from_values(
            "late.md",
            &values(&[
                ("start", json!("2026-11-05T23:00")),
                ("end", json!("2026-11-06T01:00")),
                ("recurrence", json!("FREQ=DAILY")),
            ]),
            &tz(),
        )
        .unwrap();
        let b = occurrences(&[e], at("2026-11-06 00:00"), at("2026-11-07 00:00"), &tz());
        assert_eq!(
            b.iter()
                .map(|b| b.start.date().to_string())
                .collect::<Vec<_>>(),
            ["2026-11-05", "2026-11-06"]
        );
    }

    #[test]
    fn tasks_and_time_as_blocks() {
        let t = task_block(
            &json!({"path": "Tasks/Call.md", "title": "Call", "due": "2026-11-06", "open": true}),
            &tz(),
        )
        .unwrap();
        assert!(t.all_day && t.in_strip());
        assert_eq!(
            t.source,
            Source::Task {
                path: "Tasks/Call.md".into(),
                done: false,
                field: "due".into()
            }
        );
        let t = task_block(
            &json!({"path": "Tasks/Mix.md", "title": "Mix", "due": "2026-11-09", "scheduled": "2026-11-06T14:00", "time_estimate": 90, "status": "done"}),
            &tz(),
        )
        .unwrap();
        assert_eq!(
            (t.start, t.end),
            (at("2026-11-06 14:00"), at("2026-11-06 15:30"))
        );
        assert!(
            matches!(t.source, Source::Task { done: true, ref field, .. } if field == "scheduled")
        );
        assert!(task_block(&json!({"path": "Tasks/None.md"}), &tz()).is_none());
        let s = time_block(
            &json!({"path": "Tasks/Mix.md", "title": "Mix", "start": "2026-11-06T20:00:00Z", "end": null}),
            at("2026-11-06 15:00"),
            &tz(),
        )
        .unwrap();
        assert_eq!(
            (s.start, s.end),
            (at("2026-11-06 14:00"), at("2026-11-06 15:00"))
        );
        assert!(!s.movable());
    }

    #[test]
    fn names_and_relations() {
        assert_eq!(
            file_name(
                "Lunch w/ Sam: soon?",
                NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
            ),
            "2026-11-06 Lunch w- Sam- soon-"
        );
        assert_eq!(slug("Lunch w/ Sam!"), "lunch-w-sam");
        assert_eq!(wikilink("Gigs/x.md"), "[[Gigs/x]]");
        assert!(valid_relation("for") && valid_relation("with_2"));
        assert!(!valid_relation("start") && !valid_relation("For") && !valid_relation(""));
        assert_eq!(
            when_text(&tz(), at("2026-11-06 09:30"), false),
            "2026-11-06T09:30:00-06:00"
        );
    }
}
