//! Time and events for extensions (#308; `docs/content/architecture/extensions.md`,
//! "Timers, events and background extensions").
//!
//! - **The clock** ([`Clock`]): what the host reads the time on — the
//!   system's ([`SystemClock`]), or one a test moves by hand
//!   ([`FakeClock`]) — so a timer, a scheduled hook and an idle timeout are
//!   tested without waiting.
//! - **Schedules** ([`Schedule`]): a hook's `every = "5m"` (an interval) or
//!   `at = "09:30"` / `at = "30 9 * * 1-5"` (a time of day, or five cron
//!   fields), and when each is next due.
//! - **Document events** ([`DocEvent`]): created, saved, renamed, deleted,
//!   raised by the layer that does them (the app, the document store) on
//!   the Action List ([`DocEvent::raise_on`]) or on the host
//!   (`Host::raise_doc`).
//! - **App events**: the built-in ones (`contrib::EVENTS`: `startup`,
//!   `workspace.changed`, `app.focused`, …) and those an app defines
//!   ([`AppEvent`], [`TypedEvent`]: Studio's transport and project events),
//!   heard as `app:<name>` by an extension whose manifest asks for them
//!   (`capabilities.events`) and a person granted.
//! - **Background extensions** ([`BackgroundDecl`], [`BackgroundStatus`]):
//!   started on their first event, stopped when idle.
//!
//! Pure data and arithmetic: wasm-clean, no clock read but [`SystemClock`]'s.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A document was created (`{path}`).
pub const DOC_CREATED: &str = "doc.created";
/// A document was saved (`{path}`).
pub const DOC_SAVED: &str = "doc.saved";
/// A document was renamed or moved (`{from, to}`).
pub const DOC_RENAMED: &str = "doc.renamed";
/// A document was deleted (`{path}`).
pub const DOC_DELETED: &str = "doc.deleted";
/// The four document events, as `watch_documents` hooks them.
pub const DOC_EVENTS: [&str; 4] = [DOC_CREATED, DOC_SAVED, DOC_RENAMED, DOC_DELETED];
/// Whether hook event `on` is one of the four document events: hearing
/// one is reading the documents (`reads = ["documents"]`).
#[must_use]
pub fn is_doc_event(on: &str) -> bool {
    DOC_EVENTS.contains(&on)
}

/// The app's window came to the front (`{window?}`).
pub const APP_FOCUSED: &str = "app.focused";
/// The app's window went to the back (`{window?}`).
pub const APP_BLURRED: &str = "app.blurred";
/// What a scheduled hook's run says it was (`every:5m`'s invocation
/// event): `{every}` or `{at}`, and when it was due.
pub const SCHEDULE: &str = "schedule";

/// An app-defined event's prefix in a hook's `on` (`app:transport.changed`).
pub const APP_PREFIX: &str = "app:";

/// The name of the app-defined event `on` names (`app:transport.changed`
/// → `transport.changed`).
#[must_use]
pub fn app_event(on: &str) -> Option<&str> {
    on.strip_prefix(APP_PREFIX)
}

/// Whether `name` is an app event's name: `a-z 0-9 . _ -`, not empty.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
}

/// Whether a `capabilities.events` entry covers app event `name`: the
/// name itself, `<prefix>.*` for every event under it, or `*`.
#[must_use]
pub fn covers(pattern: &str, name: &str) -> bool {
    pattern == "*"
        || pattern == name
        || pattern
            .strip_suffix(".*")
            .is_some_and(|p| name.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
}

// ── The clock ────────────────────────────────────────────────────────

/// What the host reads the time on.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;

    /// Minutes east of UTC: where an `at` schedule's time of day is.
    fn utc_offset_min(&self) -> i32 {
        0
    }
}

/// A clock shared by the host and what it runs.
pub type SharedClock = Arc<dyn Clock>;

/// The system's clock, its times of day `utc_offset_min` east of UTC
/// (the app says where it is; none: UTC).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock {
    pub utc_offset_min: i32,
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    fn utc_offset_min(&self) -> i32 {
        self.utc_offset_min
    }
}

/// A clock a test moves by hand. Cheap to clone: every clone is the same
/// clock.
#[derive(Clone, Debug, Default)]
pub struct FakeClock {
    ms: Arc<AtomicU64>,
    offset: i32,
}

impl FakeClock {
    /// A clock at `ms` since the epoch, in UTC.
    #[must_use]
    pub fn at(ms: u64) -> Self {
        Self {
            ms: Arc::new(AtomicU64::new(ms)),
            offset: 0,
        }
    }

    /// Its times of day `minutes` east of UTC.
    #[must_use]
    pub const fn with_utc_offset(mut self, minutes: i32) -> Self {
        self.offset = minutes;
        self
    }

    /// Moves it on by `by`.
    pub fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.ms.fetch_add(by, Ordering::SeqCst);
    }

    /// Sets it to `ms` since the epoch.
    pub fn set(&self, ms: u64) {
        self.ms.store(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }

    fn utc_offset_min(&self) -> i32 {
        self.offset
    }
}

// ── Schedules ────────────────────────────────────────────────────────

/// The shortest interval a schedule may have.
pub const MIN_EVERY: Duration = Duration::from_secs(1);

/// When a scheduled hook runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// Every interval, the first one interval after it is scheduled.
    Every(Duration),
    /// At the minutes cron fields match, in the clock's time of day.
    At(Cron),
}

impl Schedule {
    /// A hook's schedule: `every` (an interval) or `at` (a time of day or
    /// five cron fields). `None` for neither.
    ///
    /// # Errors
    ///
    /// Both, or one that does not read.
    pub fn of(every: Option<&str>, at: Option<&str>) -> Option<Result<Self, String>> {
        match (every, at) {
            (None, None) => None,
            (Some(e), None) => Some(parse_duration(e).and_then(|d| {
                if d < MIN_EVERY {
                    Err(format!("every {e:?}: shorter than a second"))
                } else {
                    Ok(Self::Every(d))
                }
            })),
            (None, Some(a)) => Some(Cron::parse(a).map(Self::At)),
            (Some(_), Some(_)) => Some(Err("every or at, not both".to_owned())),
        }
    }

    /// When it is next due after `after_ms` (ms since the epoch), its time
    /// of day `offset_min` east of UTC; `None` when never (cron fields no
    /// day matches, `31 2 *`).
    #[must_use]
    pub fn next_after(&self, after_ms: u64, offset_min: i32) -> Option<u64> {
        match self {
            Self::Every(d) => {
                Some(after_ms.saturating_add(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)))
            }
            Self::At(cron) => cron.next_after(after_ms, offset_min),
        }
    }
}

/// `30s`, `5m`, `2h`, `1d`, `1h30m`, `500ms` as a duration.
///
/// # Errors
///
/// Not one.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("{text:?} is not a duration (30s, 5m, 1h30m, 1d)");
    let t = text.trim();
    if t.is_empty() {
        return Err(bad());
    }
    let mut total: u64 = 0;
    let mut rest = t;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
        if digits == 0 {
            return Err(bad());
        }
        let (n, after) = rest.split_at(digits);
        let n: u64 = n.parse().map_err(|_| bad())?;
        let unit_len = after
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(after.len());
        let (unit, next) = after.split_at(unit_len);
        let ms: u64 = match unit {
            "ms" => 1,
            "s" => 1_000,
            "m" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            _ => return Err(bad()),
        };
        total = total.saturating_add(n.saturating_mul(ms));
        rest = next;
    }
    Ok(Duration::from_millis(total))
}

/// Five cron fields, or a time of day (`09:30`, every day).
///
/// The fields are minute, hour, day of the month, month, day of the week:
/// `*`, `5`, `1-5`, `*/15`, `1,15`. A day matches as cron's do: when both
/// day fields are limited, either matching is enough.
#[derive(Clone, PartialEq, Eq)]
pub struct Cron {
    text: String,
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
    any_weekday: bool,
}

impl fmt::Debug for Cron {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cron({:?})", self.text)
    }
}

/// One field's values as bits, `lo..=hi`.
fn field(text: &str, lo: u32, hi: u32, what: &str) -> Result<u64, String> {
    let bad = || format!("{what} {text:?}: not a cron field ({lo}-{hi}, *, */n, a-b, a,b)");
    let mut bits = 0_u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| bad())?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(bad());
        }
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse::<u32>().map_err(|_| bad())?,
                b.parse::<u32>().map_err(|_| bad())?,
            )
        } else {
            let a = range.parse::<u32>().map_err(|_| bad())?;
            // `5/15`: from 5, every 15, to the end.
            (a, if part.contains('/') { hi } else { a })
        };
        if a < lo || b > hi || a > b {
            return Err(bad());
        }
        let mut v = a;
        while v <= b {
            bits |= 1 << v;
            v = v.saturating_add(step);
        }
    }
    Ok(bits)
}

impl Cron {
    /// `09:30` (every day) or five cron fields.
    ///
    /// # Errors
    ///
    /// Neither.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        if let Some((h, m)) = t.split_once(':')
            && !t.contains(' ')
        {
            let bad = || format!("at {text:?}: not a time of day (09:30)");
            let h: u32 = h.parse().map_err(|_| bad())?;
            let m: u32 = m.parse().map_err(|_| bad())?;
            if h > 23 || m > 59 {
                return Err(bad());
            }
            return Self::parse(&format!("{m} {h} * * *")).map(|mut c| {
                t.clone_into(&mut c.text);
                c
            });
        }
        let fields: Vec<&str> = t.split_whitespace().collect();
        let [mi, ho, dom, mo, dow] = fields.as_slice() else {
            return Err(format!(
                "at {text:?}: a time of day (09:30) or five cron fields (30 9 * * 1-5)"
            ));
        };
        let mut weekdays = field(dow, 0, 7, "day of the week")?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays |= 1;
        }
        Ok(Self {
            text: t.to_owned(),
            minutes: field(mi, 0, 59, "minute")?,
            hours: u32::try_from(field(ho, 0, 23, "hour")?).unwrap_or(u32::MAX),
            days: u32::try_from(field(dom, 1, 31, "day")?).unwrap_or(u32::MAX),
            months: u16::try_from(field(mo, 1, 12, "month")?).unwrap_or(u16::MAX),
            weekdays: u8::try_from(weekdays & 0x7f).unwrap_or(u8::MAX),
            any_day: *dom == "*",
            any_weekday: *dow == "*",
        })
    }

    /// What it was written as.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    fn day_matches(&self, days_since_epoch: i64) -> bool {
        let (_, month, day) = civil(days_since_epoch);
        if self.months & (1 << month) == 0 {
            return false;
        }
        // 1970-01-01 was a Thursday (4).
        let weekday = days_since_epoch.saturating_add(4).rem_euclid(7);
        let dom = self.days & (1 << day) != 0;
        let dow = self.weekdays & (1 << weekday) != 0;
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (false, true) => dom,
            (true, false) => dow,
            (false, false) => dom || dow,
        }
    }

    /// The first minute it matches after `after_ms`, as ms since the epoch;
    /// `None` when none in the next eight years.
    #[must_use]
    pub fn next_after(&self, after_ms: u64, offset_min: i32) -> Option<u64> {
        let offset = i64::from(offset_min);
        let local_min = i64::try_from(after_ms.div_euclid(60_000))
            .ok()?
            .saturating_add(offset);
        let mut minute = local_min.saturating_add(1);
        let horizon = minute.saturating_add(8 * 366 * 1440);
        while minute < horizon {
            let day = minute.div_euclid(1440);
            if !self.day_matches(day) {
                minute = day.saturating_add(1).saturating_mul(1440);
                continue;
            }
            let of_day = minute.rem_euclid(1440);
            let (h, m) = (of_day.div_euclid(60), of_day.rem_euclid(60));
            if self.hours & (1 << h) == 0 {
                minute = day
                    .saturating_mul(1440)
                    .saturating_add(h.saturating_add(1).saturating_mul(60));
                continue;
            }
            if self.minutes & (1 << m) != 0 {
                return u64::try_from(minute.saturating_sub(offset).saturating_mul(60_000)).ok();
            }
            minute = minute.saturating_add(1);
        }
        None
    }
}

/// Days since 1970-01-01 as (year, month 1-12, day 1-31) — Howard
/// Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = doe
        .saturating_sub(doe.div_euclid(1460))
        .saturating_add(doe.div_euclid(36_524))
        .saturating_sub(doe.div_euclid(146_096))
        .div_euclid(365);
    let doy = doe.saturating_sub(
        yoe.saturating_mul(365)
            .saturating_add(yoe.div_euclid(4))
            .saturating_sub(yoe.div_euclid(100)),
    );
    let mp = doy.saturating_mul(5).saturating_add(2).div_euclid(153);
    let d = doy
        .saturating_sub(mp.saturating_mul(153).saturating_add(2).div_euclid(5))
        .saturating_add(1);
    let m = if mp < 10 {
        mp.saturating_add(3)
    } else {
        mp.saturating_sub(9)
    };
    let y = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(i64::from(m <= 2));
    // Both in range by construction (1..=31, 1..=12).
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

// ── Document events ──────────────────────────────────────────────────

/// Something that happened to a document, as the layer that did it says
/// (the app's documents, the document store): its hooks' `on` is
/// [`DocEvent::name`], their `detail` [`DocEvent::detail`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum DocEvent {
    #[serde(rename = "doc.created")]
    Created { path: String },
    #[serde(rename = "doc.saved")]
    Saved { path: String },
    #[serde(rename = "doc.renamed")]
    Renamed { from: String, to: String },
    #[serde(rename = "doc.deleted")]
    Deleted { path: String },
}

impl DocEvent {
    /// Its event's name: `doc.created`, `doc.saved`, `doc.renamed`,
    /// `doc.deleted`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Created { .. } => DOC_CREATED,
            Self::Saved { .. } => DOC_SAVED,
            Self::Renamed { .. } => DOC_RENAMED,
            Self::Deleted { .. } => DOC_DELETED,
        }
    }

    /// What a hook is given: `{path}`, or `{from, to}` for a rename.
    #[must_use]
    pub fn detail(&self) -> Value {
        match self {
            Self::Created { path } | Self::Saved { path } | Self::Deleted { path } => {
                json!({ "path": path })
            }
            Self::Renamed { from, to } => json!({ "from": from, "to": to }),
        }
    }

    /// Raises it on `list` (its `Changed` event, topic [`Self::name`]):
    /// a host watching the list (`Host::watch`) runs the hooks on it. What
    /// a layer below the extensions (the document store, the app) calls.
    #[cfg(feature = "host")]
    pub fn raise_on(&self, list: &architect::action::ActionList) {
        list.changed(self.name(), &self.detail().to_string());
    }
}

// ── App-defined events ───────────────────────────────────────────────

/// An event the app defines (`Host::define_event`).
///
/// Studio's `transport.changed`, `project.opened`: an extension hears it
/// with a hook on `app:<name>`, when its manifest asks
/// (`capabilities.events`) and a person granted it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppEvent {
    /// `transport.changed`: `a-z 0-9 . _ -`.
    pub name: String,
    /// What a person reads asked for it: "Transport changes".
    pub title: String,
    pub description: String,
    /// Its detail's JSON Schema, for an agent and an extension's author.
    pub detail: Option<Value>,
}

/// An app event as a Rust type: its name and words, and its detail as the
/// value itself (`Host::define`, `Host::raise`).
///
/// ```ignore
/// #[derive(serde::Serialize)]
/// struct TransportChanged { playing: bool, position: f64 }
/// impl TypedEvent for TransportChanged {
///     const NAME: &'static str = "transport.changed";
///     const TITLE: &'static str = "Transport changes";
/// }
/// host.define::<TransportChanged>();
/// host.raise(&TransportChanged { playing: true, position: 0.0 });
/// ```
pub trait TypedEvent: Serialize {
    const NAME: &'static str;
    const TITLE: &'static str;
    const DESCRIPTION: &'static str = "";

    /// Its detail's JSON Schema, when it says one.
    #[must_use]
    fn schema() -> Option<Value> {
        None
    }

    /// Its definition.
    #[must_use]
    fn definition() -> AppEvent {
        AppEvent {
            name: Self::NAME.to_owned(),
            title: Self::TITLE.to_owned(),
            description: Self::DESCRIPTION.to_owned(),
            detail: Self::schema(),
        }
    }
}

// ── Background extensions ────────────────────────────────────────────

/// `[background]`: the extension is not started as it loads, but on its
/// first event (an action run, a hook), and stopped once it has been idle
/// `idle` (`5m` when none), started again by the next.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct BackgroundDecl {
    pub idle: String,
}

impl Default for BackgroundDecl {
    fn default() -> Self {
        Self {
            idle: "5m".to_owned(),
        }
    }
}

impl BackgroundDecl {
    /// Its idle timeout.
    ///
    /// # Errors
    ///
    /// It does not read as a duration, or is under a second.
    pub fn idle(&self) -> Result<Duration, String> {
        let d = parse_duration(&self.idle)?;
        if d < MIN_EVERY {
            return Err(format!("idle {:?}: shorter than a second", self.idle));
        }
        Ok(d)
    }
}

/// Where a background extension is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundState {
    /// Not running: waiting for an event.
    Stopped,
    Running,
    /// Its last start failed; the next event tries again.
    Failed,
}

/// A background extension now (`Host::background`, `APP_STATUS`'s
/// `background`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundStatus {
    pub id: String,
    pub state: BackgroundState,
    /// How many times it has started.
    pub starts: u64,
    /// What started it last (`action FTS_X`, `hook doc.saved`).
    pub started_by: Option<String>,
    /// Calls running in it now.
    pub busy: usize,
    /// When it was last used, ms since the epoch on the host's clock.
    pub last_used_ms: u64,
    pub idle_ms: u64,
    /// When it stops unless used again (running and not busy).
    pub stops_at_ms: Option<u64>,
    /// Why its last start failed.
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(parse_duration("5m"), Ok(Duration::from_mins(5)));
        assert_eq!(parse_duration("1h30m"), Ok(Duration::from_mins(90)));
        assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_duration("1d"), Ok(Duration::from_hours(24)));
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("5 minutes").is_err());
        assert!(Schedule::of(Some("500ms"), None).unwrap().is_err());
        assert!(Schedule::of(Some("5m"), Some("09:00")).unwrap().is_err());
        assert!(Schedule::of(None, None).is_none());
    }

    #[test]
    fn every_is_one_interval_on() {
        let s = Schedule::of(Some("5m"), None).unwrap().unwrap();
        assert_eq!(s.next_after(1_000, 0), Some(1_000 + 5 * MIN));
    }

    #[test]
    fn a_time_of_day_is_the_next_one_in_the_clocks_zone() {
        // 2026-11-04 (a Wednesday) 08:00 UTC.
        let wed_8 = 1_793_750_400_000_u64 + 8 * 60 * MIN;
        let at = Schedule::of(None, Some("09:30")).unwrap().unwrap();
        assert_eq!(at.next_after(wed_8, 0), Some(wed_8 + 90 * MIN));
        // At 10:00 it is tomorrow's.
        assert_eq!(
            at.next_after(wed_8 + 120 * MIN, 0),
            Some(wed_8 + (24 * 60 + 90) * MIN)
        );
        // Two hours east of UTC, 09:30 there is 07:30 here: tomorrow's.
        assert_eq!(
            at.next_after(wed_8, 120),
            Some(wed_8 + (24 * 60 - 30) * MIN)
        );
    }

    #[test]
    fn cron_fields_match_as_crons_do() {
        let wed_8 = 1_793_750_400_000_u64 + 8 * 60 * MIN;
        // Weekdays at 09:00: Wednesday's.
        let c = Cron::parse("0 9 * * 1-5").unwrap();
        assert_eq!(c.next_after(wed_8, 0), Some(wed_8 + 60 * MIN));
        // Saturdays (6) at 09:00: three days on.
        let sat = Cron::parse("0 9 * * 6").unwrap();
        assert_eq!(
            sat.next_after(wed_8, 0),
            Some(wed_8 + (3 * 24 * 60 + 60) * MIN)
        );
        // Every 15 minutes.
        let q = Cron::parse("*/15 * * * *").unwrap();
        assert_eq!(q.next_after(wed_8 + MIN, 0), Some(wed_8 + 15 * MIN));
        // Sunday as 7.
        assert_eq!(
            Cron::parse("0 0 * * 7").unwrap().next_after(wed_8, 0),
            Cron::parse("0 0 * * 0").unwrap().next_after(wed_8, 0)
        );
        // The 31st of February never comes.
        assert_eq!(
            Cron::parse("0 0 31 2 *").unwrap().next_after(wed_8, 0),
            None
        );
        for bad in [
            "",
            "60 * * * *",
            "* * *",
            "25:00",
            "a b c d e",
            "*/0 * * * *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn civil_dates_are_right() {
        assert_eq!(civil(0), (1970, 1, 1));
        // 2026-11-04.
        assert_eq!(civil(20_761), (2026, 11, 4));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn capability_patterns_cover_names_under_them() {
        assert!(covers("transport.changed", "transport.changed"));
        assert!(covers("transport.*", "transport.changed"));
        assert!(!covers("transport.*", "transporter.changed"));
        assert!(!covers("transport.*", "transport"));
        assert!(covers("*", "project.opened"));
        assert!(!covers("project.opened", "project.closed"));
        assert_eq!(
            app_event("app:transport.changed"),
            Some("transport.changed")
        );
        assert!(valid_name("transport.changed") && !valid_name("Transport") && !valid_name(""));
    }

    #[test]
    fn a_doc_event_names_itself_and_says_what_it_was_on() {
        let saved = DocEvent::Saved {
            path: "notes/a.md".into(),
        };
        assert_eq!(saved.name(), "doc.saved");
        assert_eq!(saved.detail(), json!({ "path": "notes/a.md" }));
        let renamed = DocEvent::Renamed {
            from: "a.md".into(),
            to: "b.md".into(),
        };
        assert_eq!(renamed.detail()["to"], "b.md");
    }

    #[test]
    fn a_fake_clock_moves_when_told() {
        let c = FakeClock::at(1_000);
        let same = c.clone();
        c.advance(Duration::from_secs(2));
        assert_eq!(same.now_ms(), 3_000);
    }
}
