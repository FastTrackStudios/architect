//! Booking: event types, their hours, open slots and bookings.
//!
//! As vault notes (Task's `task:features/scheduling/scheduling-proto`
//! `EventType`, `AvailabilitySchedule`, `Booking`, and
//! `task:features/scheduling/scheduling/src/slots.rs` `list_open_slots`).
//!
//! ```yaml
//! # Calendar/Event types/intro-call.md
//! type: scheduling-event-type
//! id: intro-call
//! title: Intro call
//! slug: intro-call
//! duration_min: 30
//! buffer_min: 10
//! schedule_id: work-hours
//! notice_min: 120
//! days_ahead: 30
//! published: true
//! location: { kind: link, url: "https://meet.example/abc" }
//!
//! # Calendar/Schedules/work-hours.md
//! type: scheduling-schedule
//! id: work-hours
//! name: Work hours
//! rules:
//!   - { days: [mon, tue, wed, thu, fri], start: "09:00", end: "17:00" }
//!
//! # Calendar/Bookings/2026-11-10 Intro call — Sam.md
//! type: scheduling-booking
//! id: …
//! event_type_id: intro-call
//! start_utc: 2026-11-10T17:00:00Z
//! end_utc: 2026-11-10T17:30:00Z
//! attendee_name: Sam
//! attendee_email: sam@example.com
//! status: confirmed
//! created_utc: 2026-11-06T20:00:00Z
//! ```
//!
//! Slots step through each rule's hours by the length plus the buffer, as
//! Task's do. Where Task's are a first cut, these go further:
//!
//! - **Busy time:** bookings and the calendar's own events block a slot,
//!   each padded by the buffer on both sides.
//! - **Notice and horizon:** `notice_min` and `days_ahead` clamp the
//!   window.
//! - **Checked again:** a booking is refused when its slot is no longer
//!   open. Task's `create_booking` never checks.
//!
//! The rules are in the person's time zone; a schedule's `timezone` is
//! kept but not converted.

use std::fmt::Write as _;

use chrono::{
    DateTime, Datelike as _, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc, Weekday,
};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// Its notes' types.
pub const EVENT_TYPE: &str = "scheduling-event-type";
pub const SCHEDULE: &str = "scheduling-schedule";
pub const BOOKING: &str = "scheduling-booking";

/// Where it writes them, in the calendar folder.
pub const EVENT_TYPES_DIR: &str = "Event types";
pub const SCHEDULES_DIR: &str = "Schedules";
pub const BOOKINGS_DIR: &str = "Bookings";

/// What a booking is now (Task's `BookingStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Confirmed,
    Cancelled,
    NoShow,
    Completed,
}

impl Status {
    pub const ALL: [Self; 5] = [
        Self::Pending,
        Self::Confirmed,
        Self::Cancelled,
        Self::NoShow,
        Self::Completed,
    ];

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Cancelled => "cancelled",
            Self::NoShow => "no_show",
            Self::Completed => "completed",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.id().eq_ignore_ascii_case(id.trim()))
    }

    /// Whether it holds its time (pending or confirmed).
    #[must_use]
    pub const fn holds(self) -> bool {
        matches!(self, Self::Pending | Self::Confirmed)
    }
}

/// Hours on some weekdays (Task's `AvailabilityRule`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub days: Vec<Weekday>,
    pub start: NaiveTime,
    pub end: NaiveTime,
}

/// When an event type can be booked (Task's `AvailabilitySchedule`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Schedule {
    pub path: String,
    pub id: String,
    pub name: String,
    pub timezone: Option<String>,
    pub rules: Vec<Rule>,
}

/// Something that can be booked (Task's `EventType`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventType {
    pub path: String,
    pub id: String,
    pub title: String,
    pub slug: String,
    pub description: Option<String>,
    pub duration_min: u32,
    pub buffer_min: u32,
    pub schedule_id: Option<String>,
    /// How long before a slot it can still be booked.
    pub notice_min: u32,
    /// How far ahead it can be booked (none: no limit).
    pub days_ahead: Option<u32>,
    pub published: bool,
    /// `{kind: phone | in_person (address) | link (url) | tbd}`.
    pub location: Value,
}

/// A booking (Task's `Booking`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Booking {
    pub path: String,
    pub id: String,
    pub event_type_id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub attendee_name: String,
    pub attendee_email: String,
    pub status: Status,
    pub note: Option<String>,
}

fn text(v: &Map<String, Value>, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(v: &Map<String, Value>, k: &str) -> Option<u32> {
    let x = v.get(k)?;
    x.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| x.as_str()?.trim().parse().ok())
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s.trim().to_ascii_lowercase().get(..3)? {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// A weekday as Task writes it (`mon`).
#[must_use]
pub const fn day_word(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

fn time(s: &str) -> Option<NaiveTime> {
    let s = s.trim();
    if s == "24:00" {
        return NaiveTime::from_hms_opt(23, 59, 59);
    }
    NaiveTime::parse_from_str(s, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S"))
        .ok()
}

/// A rule from Task's map (`{days, start, end}`) or a line
/// (`mon-fri 09:00-17:00`, `sat 10:00-14:00`).
#[must_use]
pub fn rule_of(v: &Value) -> Option<Rule> {
    match v {
        Value::Object(m) => {
            let days = m
                .get("days")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .filter_map(weekday)
                .collect::<Vec<_>>();
            let start = time(m.get("start")?.as_str()?)?;
            let end = time(m.get("end")?.as_str()?)?;
            (!days.is_empty() && end > start).then_some(Rule { days, start, end })
        }
        Value::String(line) => {
            let (days, hours) = line.trim().split_once(' ')?;
            let days = days_of(days)?;
            let (a, b) = hours.trim().split_once('-')?;
            let (start, end) = (time(a)?, time(b)?);
            (end > start).then_some(Rule { days, start, end })
        }
        _ => None,
    }
}

/// `mon-fri`, `mon,wed,fri`, `weekdays`, `weekends`, `daily`.
fn days_of(s: &str) -> Option<Vec<Weekday>> {
    let all = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
    match s.to_ascii_lowercase().as_str() {
        "weekdays" => return Some(all.get(..5)?.to_vec()),
        "weekends" => return Some(vec![Weekday::Sat, Weekday::Sun]),
        "daily" | "every" => return Some(all.to_vec()),
        _ => {}
    }
    let mut out = Vec::new();
    for part in s.split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let (a, b) = (weekday(a)?, weekday(b)?);
            let mut d = a;
            loop {
                out.push(d);
                if d == b {
                    break;
                }
                d = d.succ();
            }
        } else {
            out.push(weekday(part)?);
        }
    }
    (!out.is_empty()).then_some(out)
}

impl Schedule {
    #[must_use]
    pub fn from_values(path: &str, v: &Map<String, Value>) -> Self {
        Self {
            path: path.to_owned(),
            id: text(v, "id").unwrap_or_else(|| crate::model::stem(path).to_owned()),
            name: text(v, "name")
                .or_else(|| text(v, "title"))
                .unwrap_or_else(|| crate::model::stem(path).to_owned()),
            timezone: text(v, "timezone"),
            rules: v
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(rule_of)
                .collect(),
        }
    }

    /// Its frontmatter (Task's `write/schedule.rs`).
    #[must_use]
    pub fn properties(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("type".into(), json!(SCHEDULE));
        m.insert("id".into(), json!(self.id));
        m.insert("name".into(), json!(self.name));
        if let Some(tz) = &self.timezone {
            m.insert("timezone".into(), json!(tz));
        }
        m.insert(
            "rules".into(),
            Value::Array(
                self.rules
                    .iter()
                    .map(|r| {
                        json!({
                            "days": r.days.iter().map(|d| day_word(*d)).collect::<Vec<_>>(),
                            "start": r.start.format("%H:%M").to_string(),
                            "end": r.end.format("%H:%M").to_string(),
                        })
                    })
                    .collect(),
            ),
        );
        m
    }
}

impl EventType {
    #[must_use]
    pub fn from_values(path: &str, v: &Map<String, Value>) -> Self {
        let id = text(v, "id").unwrap_or_else(|| crate::model::stem(path).to_owned());
        Self {
            path: path.to_owned(),
            title: text(v, "title").unwrap_or_else(|| crate::model::stem(path).to_owned()),
            slug: text(v, "slug").unwrap_or_else(|| id.clone()),
            id,
            description: text(v, "description"),
            duration_min: number(v, "duration_min").unwrap_or(30),
            buffer_min: number(v, "buffer_min").unwrap_or(0),
            schedule_id: text(v, "schedule_id"),
            notice_min: number(v, "notice_min").unwrap_or(0),
            days_ahead: number(v, "days_ahead"),
            published: v.get("published").and_then(Value::as_bool).unwrap_or(true),
            location: v
                .get("location")
                .cloned()
                .unwrap_or_else(|| json!({ "kind": "tbd" })),
        }
    }

    /// Its frontmatter (Task's `write/event_type.rs`, and ours).
    #[must_use]
    pub fn properties(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("type".into(), json!(EVENT_TYPE));
        m.insert("id".into(), json!(self.id));
        m.insert("title".into(), json!(self.title));
        m.insert("slug".into(), json!(self.slug));
        if let Some(d) = &self.description {
            m.insert("description".into(), json!(d));
        }
        m.insert("duration_min".into(), json!(self.duration_min));
        if self.buffer_min > 0 {
            m.insert("buffer_min".into(), json!(self.buffer_min));
        }
        if let Some(s) = &self.schedule_id {
            m.insert("schedule_id".into(), json!(s));
        }
        if self.notice_min > 0 {
            m.insert("notice_min".into(), json!(self.notice_min));
        }
        if let Some(d) = self.days_ahead {
            m.insert("days_ahead".into(), json!(d));
        }
        m.insert("published".into(), json!(self.published));
        m.insert("location".into(), self.location.clone());
        m
    }

    /// Where it is, in words.
    #[must_use]
    pub fn where_words(&self) -> String {
        match self.location["kind"].as_str() {
            Some("phone") => "Phone".to_owned(),
            Some("in_person") => self.location["address"]
                .as_str()
                .unwrap_or("In person")
                .to_owned(),
            Some("link") => self.location["url"].as_str().unwrap_or("Online").to_owned(),
            _ => "To be decided".to_owned(),
        }
    }
}

fn utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl Booking {
    #[must_use]
    pub fn from_values(path: &str, v: &Map<String, Value>) -> Option<Self> {
        Some(Self {
            path: path.to_owned(),
            id: text(v, "id").unwrap_or_else(|| crate::model::stem(path).to_owned()),
            event_type_id: text(v, "event_type_id")?,
            start: utc(&text(v, "start_utc")?)?,
            end: utc(&text(v, "end_utc")?)?,
            attendee_name: text(v, "attendee_name").unwrap_or_default(),
            attendee_email: text(v, "attendee_email").unwrap_or_default(),
            status: text(v, "status")
                .as_deref()
                .and_then(Status::from_id)
                .unwrap_or(Status::Confirmed),
            note: text(v, "note"),
        })
    }

    /// Its frontmatter (Task's `write/booking.rs`).
    #[must_use]
    pub fn properties(&self, created: DateTime<Utc>) -> Map<String, Value> {
        let stamp = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut m = Map::new();
        m.insert("type".into(), json!(BOOKING));
        m.insert("id".into(), json!(self.id));
        m.insert("event_type_id".into(), json!(self.event_type_id));
        m.insert("start_utc".into(), json!(stamp(self.start)));
        m.insert("end_utc".into(), json!(stamp(self.end)));
        m.insert("attendee_name".into(), json!(self.attendee_name));
        m.insert("attendee_email".into(), json!(self.attendee_email));
        m.insert("status".into(), json!(self.status.id()));
        m.insert("created_utc".into(), json!(stamp(created)));
        if let Some(n) = &self.note {
            m.insert("note".into(), json!(n));
        }
        m
    }
}

/// An open slot, wall-clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Slot {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

/// What `open_slots` needs besides the event type and its hours.
pub struct Busy<'a> {
    pub bookings: &'a [Booking],
    /// Other busy spans: the calendar's events, wall-clock.
    pub events: &'a [(NaiveDateTime, NaiveDateTime)],
    pub now: NaiveDateTime,
}

/// The open slots of `et` by `schedule` over `[from, to)` (Task's
/// `list_open_slots`, with busy time padded and notice and horizon kept).
///
/// # Errors
///
/// The event type is zero minutes long.
pub fn open_slots(
    et: &EventType,
    schedule: &Schedule,
    busy: &Busy<'_>,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Result<Vec<Slot>, String> {
    if et.duration_min == 0 {
        return Err("the event type is 0 minutes long".to_owned());
    }
    let length = Duration::minutes(i64::from(et.duration_min));
    let buffer = Duration::minutes(i64::from(et.buffer_min));
    let step = length + buffer;
    let from = from.max(busy.now + Duration::minutes(i64::from(et.notice_min)));
    let to = et
        .days_ahead
        .map_or(to, |d| to.min(busy.now + Duration::days(i64::from(d))));
    if to <= from {
        return Ok(Vec::new());
    }
    let mut taken: Vec<(NaiveDateTime, NaiveDateTime)> = busy
        .bookings
        .iter()
        .filter(|b| b.status.holds())
        .map(|b| {
            (
                b.start.with_timezone(&Local).naive_local(),
                b.end.with_timezone(&Local).naive_local(),
            )
        })
        .collect();
    taken.extend(busy.events.iter().copied());
    let free = |s: NaiveDateTime, e: NaiveDateTime| {
        !taken
            .iter()
            .any(|(a, b)| s < *b + buffer && *a - buffer < e)
    };
    let mut out = Vec::new();
    let mut day: NaiveDate = from.date();
    while day.and_time(NaiveTime::MIN) < to {
        for rule in schedule
            .rules
            .iter()
            .filter(|r| r.days.contains(&day.weekday()))
        {
            let mut at = day.and_time(rule.start);
            let end = day.and_time(rule.end);
            while at + length <= end {
                if at >= from && at + length <= to && free(at, at + length) {
                    out.push(Slot {
                        start: at,
                        end: at + length,
                    });
                }
                at += step;
            }
        }
        day = day.succ_opt().unwrap_or(day);
        if day == NaiveDate::MAX {
            break;
        }
    }
    out.sort_by_key(|s| s.start);
    out.dedup_by_key(|s| s.start);
    Ok(out)
}

// ── In the vault ────────────────────────────────────────────────────

/// The base that finds booking's notes.
#[must_use]
pub fn notes_base() -> String {
    let keys = [
        "type",
        "id",
        "title",
        "name",
        "slug",
        "description",
        "duration_min",
        "buffer_min",
        "schedule_id",
        "notice_min",
        "days_ahead",
        "published",
        "location",
        "timezone",
        "rules",
        "event_type_id",
        "start_utc",
        "end_utc",
        "attendee_name",
        "attendee_email",
        "status",
        "created_utc",
        "note",
    ];
    let mut out = String::from("filters:\n  or:\n");
    for t in [EVENT_TYPE, SCHEDULE, BOOKING] {
        let _ = writeln!(out, "    - type == \"{t}\"");
    }
    out.push_str("views:\n  - type: table\n    name: Booking\n    order:\n");
    for k in keys {
        let _ = writeln!(out, "      - {k}");
    }
    out
}

/// Every event type, schedule and booking in the vault.
pub(crate) async fn read() -> Result<(Vec<EventType>, Vec<Schedule>, Vec<Booking>), String> {
    let rows = crate::vault::rows(&notes_base()).await?;
    let (mut types, mut schedules, mut bookings) = (Vec::new(), Vec::new(), Vec::new());
    for r in rows {
        match r.values.get("type").and_then(Value::as_str) {
            Some(EVENT_TYPE) => types.push(EventType::from_values(&r.path, &r.values)),
            Some(SCHEDULE) => schedules.push(Schedule::from_values(&r.path, &r.values)),
            Some(BOOKING) => bookings.extend(Booking::from_values(&r.path, &r.values)),
            _ => {}
        }
    }
    bookings.sort_by_key(|b| b.start);
    Ok((types, schedules, bookings))
}

/// The bookings that hold their time, as calendar blocks.
#[must_use]
pub fn blocks(bookings: &[Booking], types: &[EventType]) -> Vec<crate::model::Block> {
    bookings
        .iter()
        .filter(|b| b.status.holds())
        .map(|b| {
            let what = types
                .iter()
                .find(|t| t.id == b.event_type_id)
                .map_or(b.event_type_id.as_str(), |t| t.title.as_str());
            crate::model::Block {
                key: format!("booking:{}", b.path),
                title: if b.attendee_name.is_empty() {
                    what.to_owned()
                } else {
                    format!("{what} — {}", b.attendee_name)
                },
                color: if b.status == Status::Pending {
                    "warning".to_owned()
                } else {
                    "info".to_owned()
                },
                start: b.start.with_timezone(&Local).naive_local(),
                end: b.end.with_timezone(&Local).naive_local(),
                all_day: false,
                location: types
                    .iter()
                    .find(|t| t.id == b.event_type_id)
                    .map(EventType::where_words),
                repeats: None,
                source: crate::model::Source::Booking {
                    path: b.path.clone(),
                    status: b.status.id().to_owned(),
                },
            }
        })
        .collect()
}

fn folder(sub: &str) -> String {
    let f = crate::settings().folder;
    if f.is_empty() {
        sub.to_owned()
    } else {
        format!("{f}/{sub}")
    }
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn num_arg(args: &Value, k: &str) -> Option<u32> {
    let v = args.get(k)?;
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| v.as_str()?.trim().parse().ok())
}

async fn fresh() -> (Vec<EventType>, Vec<Schedule>, Vec<Booking>) {
    let _ = crate::refresh().await;
    let c = crate::cache();
    (
        c.event_types.clone(),
        c.schedules.clone(),
        c.bookings.clone(),
    )
}

fn find_type<'a>(types: &'a [EventType], key: &str) -> Result<&'a EventType, String> {
    let k = key.trim();
    types
        .iter()
        .find(|t| t.id == k || t.slug == k || t.path == k || t.title.eq_ignore_ascii_case(k))
        .ok_or_else(|| format!("no event type {k:?}"))
}

/// Writes `props` to the note at `path`: a new one, or each key of one
/// that is there.
async fn save(path: &str, existing: bool, props: &Map<String, Value>) -> Result<(), String> {
    if existing {
        for (k, v) in props {
            crate::vault::set(path, k, v).await?;
        }
        Ok(())
    } else {
        crate::vault::create(path, props, "").await
    }
}

/// `CALENDAR_EVENT_TYPES`: the event types (each with its schedule's
/// name), the schedules, and how many bookings hold time.
pub async fn list_types(_: Value) -> Result<Value, String> {
    let (types, schedules, bookings) = fresh().await;
    let out: Vec<Value> = types
        .iter()
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
            v["schedule"] = json!(
                schedules
                    .iter()
                    .find(|s| Some(&s.id) == t.schedule_id.as_ref())
                    .map(|s| s.name.clone())
            );
            v["where"] = json!(t.where_words());
            v
        })
        .collect();
    Ok(json!({
        "event_types": out,
        "schedules": schedules,
        "bookings": bookings.iter().filter(|b| b.status.holds()).count(),
    }))
}

/// `CALENDAR_SCHEDULE`: makes or changes a schedule (`id`, `name`,
/// `rules`: `mon-fri 09:00-17:00` lines or Task's `{days, start, end}`,
/// `timezone`).
pub async fn save_schedule(args: Value) -> Result<Value, String> {
    let (_, schedules, _) = fresh().await;
    let rules: Vec<Rule> = args
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("give its `rules`: [\"mon-fri 09:00-17:00\"]")?
        .iter()
        .map(|r| {
            rule_of(r)
                .ok_or_else(|| format!("rule {r}: `mon-fri 09:00-17:00`, or {{days, start, end}}"))
        })
        .collect::<Result<_, _>>()?;
    let name = str_arg(&args, "name").unwrap_or("Hours").to_owned();
    let id = str_arg(&args, "id").map_or_else(|| crate::model::slug(&name), str::to_owned);
    let old = schedules.iter().find(|s| s.id == id);
    let s = Schedule {
        path: old.map_or_else(
            || format!("{}/{}.md", folder(SCHEDULES_DIR), crate::model::slug(&id)),
            |o| o.path.clone(),
        ),
        id,
        name,
        timezone: str_arg(&args, "timezone")
            .map(str::to_owned)
            .or_else(|| old.and_then(|o| o.timezone.clone())),
        rules,
    };
    save(&s.path, old.is_some(), &s.properties()).await?;
    let _ = crate::refresh().await;
    Ok(serde_json::to_value(&s).unwrap_or(Value::Null))
}

/// `CALENDAR_EVENT_TYPE`: makes or changes an event type (`id`, `title`,
/// `duration_min`, `buffer_min`, `schedule_id`, `notice_min`,
/// `days_ahead`, `location`, `published`, `description`).
pub async fn save_type(args: Value) -> Result<Value, String> {
    let (types, schedules, _) = fresh().await;
    let old = str_arg(&args, "id")
        .and_then(|k| find_type(&types, k).ok())
        .cloned();
    let title = str_arg(&args, "title")
        .map(str::to_owned)
        .or_else(|| old.as_ref().map(|o| o.title.clone()))
        .ok_or("give its `title`")?;
    let id = old.as_ref().map_or_else(
        || str_arg(&args, "id").map_or_else(|| crate::model::slug(&title), str::to_owned),
        |o| o.id.clone(),
    );
    let schedule_id = str_arg(&args, "schedule_id")
        .map(str::to_owned)
        .or_else(|| old.as_ref().and_then(|o| o.schedule_id.clone()));
    if let Some(sid) = &schedule_id
        && !schedules.iter().any(|s| &s.id == sid)
    {
        return Err(format!(
            "no schedule {sid:?}: make it with CALENDAR_SCHEDULE"
        ));
    }
    let base = old
        .clone()
        .unwrap_or_else(|| EventType::from_values("", &Map::new()));
    let t = EventType {
        path: old.as_ref().map_or_else(
            || format!("{}/{}.md", folder(EVENT_TYPES_DIR), crate::model::slug(&id)),
            |o| o.path.clone(),
        ),
        slug: str_arg(&args, "slug").map_or_else(
            || {
                if old.is_some() {
                    base.slug.clone()
                } else {
                    crate::model::slug(&title)
                }
            },
            str::to_owned,
        ),
        id,
        title,
        description: str_arg(&args, "description")
            .map(str::to_owned)
            .or(base.description),
        duration_min: num_arg(&args, "duration_min").unwrap_or(base.duration_min),
        buffer_min: num_arg(&args, "buffer_min").unwrap_or(base.buffer_min),
        schedule_id,
        notice_min: num_arg(&args, "notice_min").unwrap_or(base.notice_min),
        days_ahead: num_arg(&args, "days_ahead").or(base.days_ahead),
        published: args
            .get("published")
            .and_then(Value::as_bool)
            .unwrap_or(base.published),
        location: args
            .get("location")
            .filter(|l| l.is_object())
            .cloned()
            .unwrap_or(base.location),
    };
    if t.duration_min == 0 {
        return Err("`duration_min` must be more than 0".to_owned());
    }
    save(&t.path, old.is_some(), &t.properties()).await?;
    let _ = crate::refresh().await;
    Ok(serde_json::to_value(&t).unwrap_or(Value::Null))
}

/// The calendar's events as busy spans over `[from, to)`.
fn busy_events(from: NaiveDateTime, to: NaiveDateTime) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let events = crate::events();
    crate::model::occurrences(&events, from, to, &Local)
        .into_iter()
        .filter(|b| !b.all_day)
        .map(|b| (b.start, b.end))
        .collect()
}

fn slots_for(
    t: &EventType,
    schedules: &[Schedule],
    bookings: &[Booking],
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Result<Vec<Slot>, String> {
    let sid = t
        .schedule_id
        .as_deref()
        .ok_or_else(|| format!("{} has no schedule: give it a `schedule_id`", t.title))?;
    let schedule = schedules
        .iter()
        .find(|s| s.id == sid)
        .ok_or_else(|| format!("no schedule {sid:?}"))?;
    let events = busy_events(from, to);
    let busy = Busy {
        bookings,
        events: &events,
        now: Local::now().naive_local(),
    };
    open_slots(t, schedule, &busy, from, to)
}

fn day_or(args: &Value, k: &str, d: NaiveDate) -> Result<NaiveDate, String> {
    str_arg(args, k).map_or(Ok(d), |s| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("{k} {s:?}: YYYY-MM-DD"))
    })
}

fn slot_json(s: &Slot) -> Value {
    json!({
        "start": crate::model::when_text(&Local, s.start, false),
        "end": crate::model::when_text(&Local, s.end, false),
    })
}

/// `CALENDAR_OPEN_SLOTS`: Task's `list_open_slots`: event type
/// `event_type`'s open slots over `from`..`to` (today and the next six
/// days by default).
pub async fn list_slots(args: Value) -> Result<Value, String> {
    let (types, schedules, bookings) = fresh().await;
    let t = find_type(
        &types,
        str_arg(&args, "event_type").ok_or("give the `event_type`")?,
    )?;
    let from = day_or(&args, "from", crate::today())?;
    let to = day_or(&args, "to", from + Duration::days(7))?;
    let slots = slots_for(
        t,
        &schedules,
        &bookings,
        crate::layout::midnight(from),
        crate::layout::midnight(to),
    )?;
    Ok(
        json!({ "event_type": t.id, "count": slots.len(), "slots": slots.iter().map(slot_json).collect::<Vec<_>>() }),
    )
}

/// `CALENDAR_BOOK`: Task's `book_slot`: books `event_type` at `start` for
/// `attendee_name` (`attendee_email`, `note`), refused unless the slot is
/// open now.
pub async fn book(args: Value) -> Result<Value, String> {
    let (types, schedules, bookings) = fresh().await;
    let t = find_type(
        &types,
        str_arg(&args, "event_type").ok_or("give the `event_type`")?,
    )?
    .clone();
    let start = str_arg(&args, "start")
        .and_then(|s| crate::model::When::parse(s, &Local))
        .map(|w| w.local(&Local))
        .ok_or("give the slot's `start` (RFC 3339 or YYYY-MM-DDTHH:MM)")?;
    let name = str_arg(&args, "attendee_name")
        .ok_or("give the `attendee_name`")?
        .to_owned();
    let email = str_arg(&args, "attendee_email")
        .unwrap_or_default()
        .to_owned();
    if !email.is_empty() && !email.contains('@') {
        return Err(format!("attendee_email {email:?} is not an address"));
    }
    let day = crate::layout::midnight(start.date());
    let open = slots_for(&t, &schedules, &bookings, day, day + Duration::days(1))?;
    let Some(slot) = open.iter().find(|s| s.start == start) else {
        return Err(format!(
            "{} is not open for {}",
            start.format("%a %-d %b %H:%M"),
            t.title
        ));
    };
    let now = Utc::now();
    let id = format!("{}-{}", t.id, now.timestamp_millis());
    let b = Booking {
        path: format!(
            "{}/{} {} — {}.md",
            folder(BOOKINGS_DIR),
            slot.start.format("%Y-%m-%d %H%M"),
            t.title,
            crate::model::file_name(&name, slot.start.date())
                .trim_start_matches(&format!("{} ", slot.start.date()))
                .to_owned()
        ),
        id,
        event_type_id: t.id.clone(),
        start: crate::model::fixed(&Local, slot.start).with_timezone(&Utc),
        end: crate::model::fixed(&Local, slot.end).with_timezone(&Utc),
        attendee_name: name,
        attendee_email: email,
        status: Status::Confirmed,
        note: str_arg(&args, "note").map(str::to_owned),
    };
    crate::vault::create(&b.path, &b.properties(now), "").await?;
    let _ = crate::refresh().await;
    let mut v = serde_json::to_value(&b).unwrap_or(Value::Null);
    v["slot"] = slot_json(slot);
    Ok(v)
}

/// `CALENDAR_BOOKINGS {status?, from?, to?}`: the bookings, by start.
pub async fn list_bookings(args: Value) -> Result<Value, String> {
    let (types, _, bookings) = fresh().await;
    let status = str_arg(&args, "status")
        .map(|s| Status::from_id(s).ok_or_else(|| format!("status {s:?}")))
        .transpose()?;
    let from = str_arg(&args, "from")
        .map(|_| day_or(&args, "from", crate::today()))
        .transpose()?;
    let to = str_arg(&args, "to")
        .map(|_| day_or(&args, "to", crate::today()))
        .transpose()?;
    let out: Vec<Value> = bookings
        .iter()
        .filter(|b| status.is_none_or(|s| b.status == s))
        .filter(|b| from.is_none_or(|f| b.start.with_timezone(&Local).date_naive() >= f))
        .filter(|b| to.is_none_or(|t| b.start.with_timezone(&Local).date_naive() < t))
        .map(|b| {
            let mut v = serde_json::to_value(b).unwrap_or(Value::Null);
            v["event_type"] = json!(
                types
                    .iter()
                    .find(|t| t.id == b.event_type_id)
                    .map(|t| t.title.clone())
            );
            v
        })
        .collect();
    Ok(json!({ "count": out.len(), "bookings": out }))
}

/// `CALENDAR_BOOKING_STATUS {path | id, status}`: Task's
/// `update_booking_status` (`cancelled` frees its slot).
pub async fn set_status(args: Value) -> Result<Value, String> {
    let (_, _, bookings) = fresh().await;
    let status = str_arg(&args, "status")
        .and_then(Status::from_id)
        .ok_or("`status`: pending, confirmed, cancelled, no_show or completed")?;
    let key = str_arg(&args, "path")
        .or_else(|| str_arg(&args, "id"))
        .ok_or("name the booking: its `path` or `id`")?;
    let b = bookings
        .iter()
        .find(|b| b.path == key || b.id == key)
        .ok_or("no such booking")?;
    crate::vault::set(&b.path, "status", &json!(status.id())).await?;
    let _ = crate::refresh().await;
    Ok(json!({ "path": b.path, "id": b.id, "status": status.id(), "was": b.status.id() }))
}

/// `CALENDAR_BOOKINGS_OPEN`: opens the bookings view.
pub async fn open_view(_: Value) -> Result<Value, String> {
    crate::vault::run(
        "EXT_VIEW_OPEN",
        json!({ "extension": crate::ID, "view": "bookings" }),
    )
    .await
}

// ── Its view ────────────────────────────────────────────────────────

use dioxus::prelude::*;

const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted-foreground); padding: 4px 0;";
const BUTTON: &str = "padding: 3px 10px; border-radius: 7px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";

/// The bookings view: each event type with its open slots this week (a
/// click books one, for the name typed), and the bookings.
#[component]
pub fn BookingsView() -> Element {
    let _ = crate::ui::use_calendar_revision();
    let mut name = use_signal(String::new);
    let mut problem = use_signal(|| None::<String>);
    let (types, schedules, bookings) = {
        let c = crate::cache();
        (
            c.event_types.clone(),
            c.schedules.clone(),
            c.bookings.clone(),
        )
    };
    let from = crate::layout::midnight(crate::today());
    let to = from + Duration::days(7);
    rsx! {
        div { class: "cal-bookings", style: "display: flex; flex-direction: column; gap: 16px; padding: 20px 24px; height: 100%; box-sizing: border-box; overflow-y: auto; color: var(--foreground); font-size: 13px;",
            span { style: "font-size: 20px; font-weight: 700;", "Bookings" }
            if let Some(p) = problem() { div { style: "color: var(--destructive);", "{p}" } }
            input { class: "cal-book-name", r#type: "text", "aria-label": "Who", placeholder: "Who is booking — a name",
                value: "{name}",
                style: "height: 32px; box-sizing: border-box; padding: 0 10px; border-radius: 8px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); width: 320px;",
                oninput: move |e: FormEvent| name.set(e.value()) }
            if types.is_empty() {
                div { style: MUTED, "No event types yet. Make one with CALENDAR_SCHEDULE and CALENDAR_EVENT_TYPE (⌘K)." }
            }
            for t in types {
                {
                    let slots = slots_for(&t, &schedules, &bookings, from, to).unwrap_or_default();
                    let tid = t.id.clone();
                    let summary = format!("{} min · {}", t.duration_min, t.where_words());
                    rsx! {
                        div { key: "{t.id}", class: "cal-event-type", "data-id": "{t.id}",
                            style: "display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--border); background: var(--card);",
                            div { style: "display: flex; gap: 10px; align-items: baseline;",
                                span { style: "font-weight: 700;", "{t.title}" }
                                span { style: MUTED, "{summary}" }
                            }
                            if slots.is_empty() { span { style: MUTED, "No open slots this week." } }
                            div { style: "display: flex; flex-wrap: wrap; gap: 4px;",
                                for s in slots.into_iter().take(40) {
                                    {
                                        let tid = tid.clone();
                                        let label = s.start.format("%a %-d · %H:%M").to_string();
                                        let at = crate::model::when_text(&Local, s.start, false);
                                        rsx! {
                                            button { key: "{at}", class: "cal-slot", "data-start": "{at}", style: BUTTON,
                                                onclick: move |_| {
                                                    let who = name.peek().trim().to_owned();
                                                    if who.is_empty() { problem.set(Some("Type who is booking first.".to_owned())); return; }
                                                    let args = json!({ "event_type": tid, "start": at, "attendee_name": who });
                                                    spawn(async move {
                                                        match book(args).await { Ok(_) => problem.set(None), Err(e) => problem.set(Some(e)) }
                                                    });
                                                },
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { style: HEADING, "Booked" }
            if bookings.is_empty() { div { style: MUTED, "No bookings." } }
            for b in bookings {
                {
                    let when = b.start.with_timezone(&Local).format("%a %-d %b · %H:%M").to_string();
                    let path = b.path.clone();
                    let status = b.status;
                    rsx! {
                        div { key: "{b.path}", class: "cal-booking", "data-status": "{b.status.id()}", style: "display: flex; gap: 10px; align-items: center;",
                            span { style: "width: 150px; {MUTED}", "{when}" }
                            span { style: "flex: 1; font-weight: 600;", "{b.attendee_name}" }
                            span { style: MUTED, "{b.status.id()}" }
                            if status.holds() {
                                button { style: BUTTON, onclick: move |_| {
                                    let path = path.clone();
                                    spawn(async move { if let Err(e) = set_status(json!({ "path": path, "status": "cancelled" })).await { problem.set(Some(e)); } });
                                }, "Cancel" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn et(duration: u32, buffer: u32) -> EventType {
        let mut v = Map::new();
        v.insert("duration_min".into(), json!(duration));
        v.insert("buffer_min".into(), json!(buffer));
        EventType::from_values("Calendar/Event types/call.md", &v)
    }

    fn hours(rule: &Value) -> Schedule {
        let mut v = Map::new();
        v.insert("rules".into(), json!([rule]));
        Schedule::from_values("Calendar/Schedules/hours.md", &v)
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn starts(slots: &[Slot]) -> Vec<String> {
        slots
            .iter()
            .map(|s| s.start.format("%a %H:%M").to_string())
            .collect()
    }

    fn none() -> Busy<'static> {
        Busy {
            bookings: &[],
            events: &[],
            now: at("2026-01-01 00:00"),
        }
    }

    // Task's `one_weekday_yields_four_30_min_slots`.
    #[test]
    fn a_weekday_s_hours_make_slots() {
        let s = hours(&json!({ "days": ["mon"], "start": "09:00", "end": "11:00" }));
        let slots = open_slots(
            &et(30, 0),
            &s,
            &none(),
            at("2026-11-09 00:00"),
            at("2026-11-10 00:00"),
        )
        .unwrap();
        assert_eq!(
            starts(&slots),
            ["Mon 09:00", "Mon 09:30", "Mon 10:00", "Mon 10:30"]
        );
    }

    // `weekend_yields_no_slots`.
    #[test]
    fn days_off_have_none() {
        let s = hours(&json!("mon-fri 09:00-17:00"));
        assert!(
            open_slots(
                &et(30, 0),
                &s,
                &none(),
                at("2026-11-07 00:00"),
                at("2026-11-09 00:00")
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(s.rules[0].days.len(), 5);
    }

    // `booking_blocks_overlapping_slots`, `cancelled_booking_doesnt_block`,
    // and busy time padded by the buffer.
    #[test]
    fn busy_time_blocks_slots() {
        let s = hours(&json!({ "days": ["mon"], "start": "09:00", "end": "11:00" }));
        let b = |status: Status| Booking {
            path: String::new(),
            id: "b".into(),
            event_type_id: "call".into(),
            start: model_utc(at("2026-11-09 09:30")),
            end: model_utc(at("2026-11-09 10:00")),
            attendee_name: String::new(),
            attendee_email: String::new(),
            status,
            note: None,
        };
        let held = [b(Status::Confirmed)];
        let busy = Busy {
            bookings: &held,
            events: &[],
            now: at("2026-01-01 00:00"),
        };
        let slots = open_slots(
            &et(30, 0),
            &s,
            &busy,
            at("2026-11-09 00:00"),
            at("2026-11-10 00:00"),
        )
        .unwrap();
        assert_eq!(starts(&slots), ["Mon 09:00", "Mon 10:00", "Mon 10:30"]);
        let gone = [b(Status::Cancelled)];
        let busy = Busy {
            bookings: &gone,
            events: &[],
            now: at("2026-01-01 00:00"),
        };
        assert_eq!(
            open_slots(
                &et(30, 0),
                &s,
                &busy,
                at("2026-11-09 00:00"),
                at("2026-11-10 00:00")
            )
            .unwrap()
            .len(),
            4
        );
        // An event, padded by a 15-minute buffer, takes the slots beside it.
        let events = [(at("2026-11-09 10:00"), at("2026-11-09 10:30"))];
        let busy = Busy {
            bookings: &[],
            events: &events,
            now: at("2026-01-01 00:00"),
        };
        let slots = open_slots(
            &et(30, 15),
            &s,
            &busy,
            at("2026-11-09 00:00"),
            at("2026-11-10 00:00"),
        )
        .unwrap();
        // Steps of 45: 09:00 (ends 09:30, 30 before the event: free), 09:45 (ends 10:15: busy), 10:30 (starts at its end: within the buffer).
        assert_eq!(starts(&slots), ["Mon 09:00"]);
    }

    fn model_utc(t: NaiveDateTime) -> DateTime<Utc> {
        crate::model::fixed(&Local, t).with_timezone(&Utc)
    }

    // `buffer_min_widens_step`.
    #[test]
    fn a_buffer_widens_the_step() {
        let s = hours(&json!({ "days": ["mon"], "start": "09:00", "end": "11:00" }));
        let slots = open_slots(
            &et(30, 15),
            &s,
            &none(),
            at("2026-11-09 00:00"),
            at("2026-11-10 00:00"),
        )
        .unwrap();
        assert_eq!(starts(&slots), ["Mon 09:00", "Mon 09:45", "Mon 10:30"]);
    }

    #[test]
    fn notice_and_horizon() {
        let s = hours(&json!("daily 09:00-10:00"));
        let mut t = et(30, 0);
        t.notice_min = 120;
        t.days_ahead = Some(2);
        let busy = Busy {
            bookings: &[],
            events: &[],
            now: at("2026-11-09 08:00"),
        };
        let slots = open_slots(
            &t,
            &s,
            &busy,
            at("2026-11-09 00:00"),
            at("2026-11-20 00:00"),
        )
        .unwrap();
        assert_eq!(starts(&slots), ["Tue 09:00", "Tue 09:30"]);
        t.duration_min = 0;
        assert!(
            open_slots(
                &t,
                &s,
                &busy,
                at("2026-11-09 00:00"),
                at("2026-11-20 00:00")
            )
            .is_err()
        );
    }

    #[test]
    fn notes_round_trip() {
        let s = hours(&json!({ "days": ["mon", "wed"], "start": "09:00", "end": "17:00" }));
        assert_eq!(Schedule::from_values(&s.path, &s.properties()), s);
        let t = et(45, 5);
        assert_eq!(EventType::from_values(&t.path, &t.properties()), t);
        assert_eq!(t.where_words(), "To be decided");
        assert_eq!(
            days_of("fri-mon").unwrap(),
            [Weekday::Fri, Weekday::Sat, Weekday::Sun, Weekday::Mon]
        );
    }
}
