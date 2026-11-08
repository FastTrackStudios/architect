//! Subscriptions: calendars from elsewhere, read only, kept as notes.
//!
//! A subscription is a note (`Calendar/Subscriptions/<name>.md`, `type:
//! calendar-subscription`) with its feed's `url`, a `color` and when it
//! last `synced`; its events are notes in `Calendar/<name>/`, written
//! from the feed ([`crate::ics`]) and never sent back. Each sync makes
//! what is new, changes what changed (key by key, so a note's body and
//! its own keys stay), and tombstones what the feed dropped or cancelled.
//! On the calendar they don't drag: the feed is where they change.

use std::collections::{BTreeMap, BTreeSet};

use architect_extensions::hostcall::{Fetch, HostRequest};
use chrono::{Duration, Local, Utc};
use serde_json::{Map, Value, json};

use crate::ics::{self, VEvent};
use crate::model::{Event, When, keys};

/// A subscription note's type.
pub const SUBSCRIPTION: &str = "calendar-subscription";

/// Where subscription notes go, in the calendar folder.
pub const SUBSCRIPTIONS_DIR: &str = "Subscriptions";

/// The most of a feed it reads.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// A calendar subscribed to.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Subscription {
    pub path: String,
    pub name: String,
    pub url: String,
    pub color: String,
    pub synced: Option<String>,
}

impl Subscription {
    fn from_values(path: &str, v: &Map<String, Value>) -> Option<Self> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
        };
        Some(Self {
            path: path.to_owned(),
            name: s("name").unwrap_or_else(|| crate::model::stem(path).to_owned()),
            url: s("url")?,
            color: s("color").unwrap_or_else(|| "info".to_owned()),
            synced: s("synced"),
        })
    }

    /// The folder its events are in.
    #[must_use]
    pub fn folder(&self) -> String {
        folder_of(&self.name)
    }
}

fn root() -> String {
    crate::settings().folder
}

fn folder_of(name: &str) -> String {
    let r = root();
    let name = name.replace('/', "-");
    if r.is_empty() {
        name
    } else {
        format!("{r}/{name}")
    }
}

/// Every subscription in the vault.
pub async fn subscriptions() -> Result<Vec<Subscription>, String> {
    let source = format!(
        "filters:\n  and:\n    - type == \"{SUBSCRIPTION}\"\nviews:\n  - type: table\n    name: Subscriptions\n    order:\n      - name\n      - url\n      - color\n      - synced\n"
    );
    let rows = crate::vault::rows(&source).await?;
    Ok(rows
        .iter()
        .filter_map(|r| Subscription::from_values(&r.path, &r.values))
        .collect())
}

/// An imported event's `id`: stable, from its UID (and day, for a moved
/// occurrence), so a series' overrides name it.
fn id_of(uid: &str, day: Option<chrono::NaiveDate>) -> String {
    let base = format!("ics-{}", crate::model::slug(uid));
    day.map_or_else(
        || base.clone(),
        |d| format!("{base}-{}", d.format("%Y%m%d")),
    )
}

/// The event a `VEVENT` makes, in `folder`, `color`.
fn event_of(v: &VEvent, folder: &str, color: &str) -> Option<Event> {
    let start = v.start?;
    let all_day = matches!(start, When::Day(_));
    let end = v.end.unwrap_or_else(|| match start {
        When::Day(d) => When::Day(d + Duration::days(1)),
        When::At(t) => When::At(t + Duration::hours(1)),
    });
    let title = if v.summary.is_empty() {
        "Busy".to_owned()
    } else {
        v.summary.clone()
    };
    let day = start.local(&Local).date();
    Some(Event {
        path: format!("{folder}/{}.md", crate::model::file_name(&title, day)),
        id: id_of(&v.uid, v.recurrence_id),
        title,
        start,
        end,
        all_day,
        color: color.to_owned(),
        description: v.description.clone(),
        recurrence: v.rrule.clone().filter(|_| v.recurrence_id.is_none()),
        exdates: v.exdates.iter().copied().collect(),
        recurrence_id: v.recurrence_id.map(|_| id_of(&v.uid, None)),
        recurrence_date: v.recurrence_id,
        location: v.location.clone(),
        links: Vec::new(),
        external_ref: Some(v.reference()),
        tombstone: v.cancelled.then(|| ics::stamp(Utc::now())),
        tags: Vec::new(),
    })
}

/// What a sync did.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub tombstoned: usize,
}

/// The keys of `e` an import owns (a note's own keys and body are left).
const OWNED: &[&str] = &[
    keys::TITLE,
    keys::START,
    keys::END,
    keys::ALL_DAY,
    keys::COLOR,
    keys::DESCRIPTION,
    keys::RECURRENCE,
    keys::EXDATES,
    keys::RECURRENCE_ID,
    keys::RECURRENCE_DATE,
    keys::LOCATION,
    keys::TOMBSTONE,
];

/// The calendars being imported now: one import of a calendar at a
/// time (the startup hook's sync and a subscribe's can meet).
static IMPORTING: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

/// Held while `name` is imported.
struct Importing(String);

impl Importing {
    async fn wait(name: &str) -> Self {
        let key = name.to_lowercase();
        loop {
            if IMPORTING
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key.clone())
            {
                return Self(key);
            }
            futures_timer::Delay::new(std::time::Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Importing {
    fn drop(&mut self) {
        IMPORTING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Writes the feed's events into `name`'s folder, in `color`.
///
/// # Errors
///
/// A write failed.
pub async fn import(name: &str, color: &str, text: &str) -> Result<Report, String> {
    let _one = Importing::wait(name).await;
    let _ = crate::refresh().await;
    let folder = folder_of(name);
    let feed: Vec<Event> = ics::parse(text)
        .iter()
        .filter_map(|v| event_of(v, &folder, color))
        .collect();
    let have: BTreeMap<String, Event> = crate::events()
        .into_iter()
        .filter(|e| e.path.starts_with(&format!("{folder}/")))
        .filter_map(|e| Some((e.external_ref.clone()?, e)))
        .collect();
    let mut report = Report::default();
    let mut seen = BTreeSet::new();
    let mut taken: BTreeSet<String> = crate::events()
        .into_iter()
        .map(|e| e.path.to_lowercase())
        .collect();
    for mut e in feed {
        let r = e.external_ref.clone().unwrap_or_default();
        if !seen.insert(r.clone()) {
            continue;
        }
        if let Some(old) = have.get(&r) {
            let want = e.properties();
            let was = old.properties();
            let mut changed = false;
            for k in OWNED {
                let (a, b) = (want.get(*k), was.get(*k));
                // A tombstone stamped again is the same tombstone.
                let same = a == b || (*k == keys::TOMBSTONE && a.is_some() == b.is_some());
                if !same {
                    crate::vault::set(&old.path, k, a.unwrap_or(&Value::Null)).await?;
                    changed = true;
                }
            }
            if changed {
                report.updated += 1;
            } else {
                report.unchanged += 1;
            }
        } else {
            let stem = e.path.trim_end_matches(".md").to_owned();
            let mut n = 2;
            while taken.contains(&e.path.to_lowercase()) {
                e.path = format!("{stem} {n}.md");
                n += 1;
            }
            taken.insert(e.path.to_lowercase());
            crate::vault::create(&e.path, &e.properties(), "").await?;
            report.created += 1;
        }
    }
    // What the feed no longer has.
    for (r, old) in &have {
        if !seen.contains(r) && old.tombstone.is_none() {
            crate::vault::set(&old.path, keys::TOMBSTONE, &json!(ics::stamp(Utc::now()))).await?;
            report.tombstoned += 1;
        }
    }
    let _ = crate::refresh().await;
    Ok(report)
}

async fn fetch(url: &str) -> Result<String, String> {
    let h = crate::vault::handle().ok_or("the calendar is not running")?;
    let out = h
        .host(HostRequest::Fetch(Fetch {
            url: ics::feed_url(url),
            timeout_ms: Some(30_000),
            max_bytes: Some(MAX_BYTES),
            ..Fetch::default()
        }))
        .await
        .map_err(|e| e.to_string())?;
    let status = out["status"].as_u64().unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("{url} answered {status}"));
    }
    out["body"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{url} sent no calendar"))
}

/// Fetches `s`'s feed and imports it; notes when it ran.
async fn sync_one(s: &Subscription) -> Result<Report, String> {
    let text = fetch(&s.url).await?;
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(format!("{} is not an iCalendar feed", s.url));
    }
    let report = import(&s.name, &s.color, &text).await?;
    crate::vault::set(&s.path, "synced", &json!(ics::stamp(Utc::now()))).await?;
    Ok(report)
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `CALENDAR_SUBSCRIBE {url, name?, color?}`: subscribes to a feed (an
/// `https://` or `webcal://` `.ics`), and syncs it.
pub async fn subscribe(args: Value) -> Result<Value, String> {
    let url = str_arg(&args, "url").ok_or("give the calendar's `url` (https:// or webcal://)")?;
    let feed = ics::feed_url(url);
    let Some(rest) = feed
        .strip_prefix("https://")
        .or_else(|| feed.strip_prefix("http://"))
    else {
        return Err(format!("{url}: an http, https or webcal address"));
    };
    let host = rest
        .split(['/', ':', '?'])
        .next()
        .filter(|h| !h.is_empty())
        .unwrap_or("Calendar");
    let name = str_arg(&args, "name").map_or_else(|| host.to_owned(), str::to_owned);
    if subscriptions()
        .await?
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("there is a subscription named {name:?} already"));
    }
    let dir = {
        let r = root();
        if r.is_empty() {
            SUBSCRIPTIONS_DIR.to_owned()
        } else {
            format!("{r}/{SUBSCRIPTIONS_DIR}")
        }
    };
    let s = Subscription {
        path: format!("{dir}/{}.md", name.replace('/', "-")),
        name,
        url: url.to_owned(),
        color: str_arg(&args, "color").unwrap_or("info").to_owned(),
        synced: None,
    };
    let mut m = Map::new();
    m.insert("type".into(), json!(SUBSCRIPTION));
    m.insert("name".into(), json!(s.name));
    m.insert("url".into(), json!(s.url));
    m.insert("color".into(), json!(s.color));
    crate::vault::create(&s.path, &m, "").await?;
    let synced = sync_one(&s).await;
    Ok(json!({
        "subscription": s,
        "report": synced.as_ref().ok(),
        "error": synced.err(),
    }))
}

/// `CALENDAR_SYNC {name?}`: fetches each subscription (or `name`'s) and
/// updates its events.
pub async fn sync(args: Value) -> Result<Value, String> {
    let all = subscriptions().await?;
    let want = str_arg(&args, "name");
    if let Some(n) = want
        && !all.iter().any(|s| s.name.eq_ignore_ascii_case(n))
    {
        return Err(format!("no subscription {n:?}"));
    }
    let mut out = Vec::new();
    for s in all
        .iter()
        .filter(|s| want.is_none_or(|n| s.name.eq_ignore_ascii_case(n)))
    {
        match sync_one(s).await {
            Ok(r) => out.push(json!({ "name": s.name, "report": r })),
            Err(e) => out.push(json!({ "name": s.name, "error": e })),
        }
    }
    Ok(json!({ "synced": out }))
}

/// `CALENDAR_IMPORT {text, name?, color?}`: imports an `.ics` file's
/// text, as a subscription's sync does, into `Calendar/<name>/`.
pub async fn import_text(args: Value) -> Result<Value, String> {
    let text = str_arg(&args, "text").ok_or("give the calendar's `text` (an .ics file's)")?;
    if !text.contains("BEGIN:VCALENDAR") && !text.contains("BEGIN:VEVENT") {
        return Err("that is not iCalendar text".to_owned());
    }
    let name = str_arg(&args, "name").unwrap_or("Imported");
    let r = import(name, str_arg(&args, "color").unwrap_or("info"), text).await?;
    Ok(json!({ "name": name, "folder": folder_of(name), "report": r }))
}

/// `CALENDAR_SUBSCRIPTIONS`: each subscription, its feed, colour and last
/// sync, and how many events it has.
pub async fn list(_: Value) -> Result<Value, String> {
    let all = subscriptions().await?;
    let events = crate::events();
    let out: Vec<Value> = all
        .iter()
        .map(|s| {
            let prefix = format!("{}/", s.folder());
            let n = events
                .iter()
                .filter(|e| e.path.starts_with(&prefix) && e.tombstone.is_none())
                .count();
            let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
            v["events"] = json!(n);
            v
        })
        .collect();
    Ok(json!({ "count": out.len(), "subscriptions": out }))
}

/// `CALENDAR_UNSUBSCRIBE {name}`: removes a subscription and its events'
/// notes.
pub async fn unsubscribe(args: Value) -> Result<Value, String> {
    let name = str_arg(&args, "name").ok_or("give the subscription's `name`")?;
    let all = subscriptions().await?;
    let s = all
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no subscription {name:?}"))?;
    let _ = crate::refresh().await;
    let prefix = format!("{}/", s.folder());
    let notes: Vec<String> = crate::events()
        .into_iter()
        .filter(|e| e.path.starts_with(&prefix))
        .map(|e| e.path)
        .collect();
    for p in notes.iter().chain(std::iter::once(&s.path)) {
        crate::vault::run("VAULT_DELETE", json!({ "path": p })).await?;
    }
    let _ = crate::refresh().await;
    Ok(json!({ "removed": s.name, "events": notes.len() }))
}

/// Whether the event at `path` came from a subscription or an import
/// (read only here).
#[must_use]
pub fn imported(e: &Event) -> bool {
    e.external_ref
        .as_deref()
        .is_some_and(|r| r.starts_with("ics:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vevent_is_an_event() {
        let feed = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:A@x\nSUMMARY:Gig\nDTSTART:20261120T020000Z\nDTEND:20261120T040000Z\nRRULE:FREQ=WEEKLY\nEND:VEVENT\nBEGIN:VEVENT\nUID:A@x\nRECURRENCE-ID:20261127T020000Z\nSUMMARY:Gig (moved)\nDTSTART:20261127T030000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let v = ics::parse(feed);
        let a = event_of(&v[0], "Calendar/Band", "info").unwrap();
        assert_eq!(
            (a.id.as_str(), a.external_ref.as_deref()),
            ("ics-a-x", Some("ics:A@x"))
        );
        assert_eq!(a.recurrence.as_deref(), Some("FREQ=WEEKLY"));
        assert!(a.path.starts_with("Calendar/Band/") && imported(&a));
        let b = event_of(&v[1], "Calendar/Band", "info").unwrap();
        assert_eq!(b.recurrence_id.as_deref(), Some("ics-a-x"));
        assert_eq!(b.recurrence, None);
        assert_eq!(b.length(&Local), Duration::hours(1), "no end: an hour");
    }
}
