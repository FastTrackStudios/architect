//! Calendars from elsewhere: RFC 5545 iCalendar (`.ics`), read only.
//!
//! A feed (a `CalDAV` collection's export, Google's or iCloud's "secret
//! address", a `webcal://` link) is fetched by the host (`fetch`,
//! `capabilities.hosts`). Each `VEVENT` becomes an event note keyed by
//! `external_ref` `ics:<UID>`, so a sync updates notes in place, as
//! Task's `create_event` dedupes by its ref. An occurrence the feed moved
//! (`RECURRENCE-ID`) is an override of its series; `EXDATE` its skipped
//! days; `STATUS:CANCELLED` a tombstone. What the feed no longer has is
//! tombstoned too.
//!
//! Times: `…Z` is UTC; a `TZID` or floating time is read as the person's
//! own time zone (no `VTIMEZONE` is read), which is right for a calendar
//! in the zone you are in.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::model::When;

/// One `VEVENT`, read.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VEvent {
    pub uid: String,
    pub summary: String,
    pub start: Option<When>,
    pub end: Option<When>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub rrule: Option<String>,
    pub exdates: Vec<NaiveDate>,
    /// The day of the occurrence it replaces, for a moved one.
    pub recurrence_id: Option<NaiveDate>,
    pub cancelled: bool,
}

impl VEvent {
    /// Its `external_ref`: `ics:<UID>`, and the day for a moved
    /// occurrence.
    #[must_use]
    pub fn reference(&self) -> String {
        self.recurrence_id.map_or_else(
            || format!("ics:{}", self.uid),
            |d| format!("ics:{}@{d}", self.uid),
        )
    }
}

/// The calendar's lines, folded ones joined (RFC 5545 §3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
            }
        } else if !line.is_empty() {
            out.push(line.to_owned());
        }
    }
    out
}

/// A line's name, parameters and value: `DTSTART;TZID=Europe/Paris:2026…`.
fn split(line: &str) -> Option<(String, Vec<(String, String)>, String)> {
    // The value starts at the first ':' outside quotes.
    let mut quoted = false;
    let at = line.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    let (head, value) = (line.get(..at.0)?, line.get(at.0 + 1..)?);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_uppercase(),
                v.trim_matches('"').to_owned(),
            )
        })
        .collect();
    Some((name, params, value.to_owned()))
}

/// A `TEXT` value unescaped (`\n`, `\,`, `\;`, `\\`).
fn text(v: &str) -> String {
    let mut out = String::new();
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

/// A `DATE` or `DATE-TIME` value.
fn when(params: &[(String, String)], v: &str) -> Option<When> {
    let v = v.trim();
    let is_date = params
        .iter()
        .any(|(k, p)| k == "VALUE" && p.eq_ignore_ascii_case("DATE"))
        || v.len() == 8;
    if is_date {
        return NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d")
            .ok()
            .map(When::Day);
    }
    if let Some(z) = v.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(z, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::At(t.and_utc().with_timezone(&Local).fixed_offset()));
    }
    let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    Some(When::At(crate::model::fixed(&Local, t)))
}

/// A `DURATION` (`PT1H30M`, `P1D`, `P2W`).
fn duration(v: &str) -> Option<Duration> {
    let v = v.trim();
    let (neg, v) = v
        .strip_prefix('-')
        .map_or_else(|| (false, v.trim_start_matches('+')), |r| (true, r));
    let v = v.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', _) => Duration::weeks(n),
                    ('D', _) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

/// The day a `DATE` or `DATE-TIME` falls on, here.
fn day_of(w: When) -> NaiveDate {
    w.local(&Local).date()
}

/// Every `VEVENT` in `text` (an `.ics` file's), the ones it cannot place
/// left out.
#[must_use]
pub fn parse(text_in: &str) -> Vec<VEvent> {
    let mut out = Vec::new();
    let mut cur: Option<VEvent> = None;
    let mut length: Option<Duration> = None;
    // Inside a `VALARM` (or another part of the event): its lines are not
    // the event's.
    let mut nested = 0_u32;
    for line in unfold(text_in) {
        let Some((name, params, value)) = split(&line) else {
            continue;
        };
        match (name.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => {
                cur = Some(VEvent::default());
                length = None;
                nested = 0;
                continue;
            }
            ("BEGIN", _) if cur.is_some() => {
                nested += 1;
                continue;
            }
            ("END", "VEVENT") => {
                if let Some(mut e) = cur.take() {
                    if e.end.is_none()
                        && let (Some(s), Some(l)) = (e.start, length)
                    {
                        e.end = Some(match s {
                            When::Day(d) => When::Day(d + Duration::days(l.num_days().max(1))),
                            When::At(t) => When::At(t + l),
                        });
                    }
                    if !e.uid.is_empty() && e.start.is_some() {
                        out.push(e);
                    }
                }
                continue;
            }
            ("END", _) if nested > 0 => {
                nested -= 1;
                continue;
            }
            _ => {}
        }
        let Some(e) = cur.as_mut() else { continue };
        if nested > 0 {
            continue;
        }
        match name.as_str() {
            "UID" => value.trim().clone_into(&mut e.uid),
            "SUMMARY" => e.summary = text(&value),
            "DESCRIPTION" => e.description = Some(text(&value)).filter(|d| !d.is_empty()),
            "LOCATION" => e.location = Some(text(&value)).filter(|d| !d.is_empty()),
            "DTSTART" => e.start = when(&params, &value),
            "DTEND" => e.end = when(&params, &value),
            "DURATION" => length = duration(&value),
            "RRULE" => e.rrule = Some(value.trim().to_owned()),
            "EXDATE" => e.exdates.extend(
                value
                    .split(',')
                    .filter_map(|v| when(&params, v))
                    .map(day_of),
            ),
            "RECURRENCE-ID" => e.recurrence_id = when(&params, &value).map(day_of),
            "STATUS" => e.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    out
}

/// A `webcal://` link as the `https://` one it is.
#[must_use]
pub fn feed_url(url: &str) -> String {
    let u = url.trim();
    u.strip_prefix("webcal://")
        .map_or_else(|| u.to_owned(), |rest| format!("https://{rest}"))
}

/// When a sync ran, as a subscription note keeps it.
#[must_use]
pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Midnight of a date, for a feed's all-day ends.
#[must_use]
pub const fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_time(NaiveTime::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\nUID:standup@example.com\r\nSUMMARY:Standup\r\nDTSTART;TZID=America/Chicago:20261102T093000\r\n\
DURATION:PT15M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nEXDATE;TZID=America/Chicago:20261104T093000,20261106T093000\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:standup@example.com\r\nRECURRENCE-ID;TZID=America/Chicago:20261109T093000\r\nSUMMARY:Standup (late)\r\n\
DTSTART;TZID=America/Chicago:20261109T110000\r\nDTEND;TZID=America/Chicago:20261109T111500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:holiday-1\r\nSUMMARY:Holiday\\, the long one\r\nDTSTART;VALUE=DATE:20261126\r\nDTEND;VALUE=DATE:20261128\r\n\
DESCRIPTION:Line one\\nline two that is long and is\r\n  folded onto the next\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:gone\r\nSUMMARY:Cancelled call\r\nDTSTART:20261110T170000Z\r\nDTEND:20261110T173000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:No uid\r\nDTSTART:20261110T170000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn a_feed_reads() {
        let e = parse(FEED);
        assert_eq!(e.len(), 4, "one without a UID is left out");
        let s = &e[0];
        assert_eq!(
            (s.summary.as_str(), s.rrule.as_deref()),
            ("Standup", Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"))
        );
        assert_eq!(
            s.end.unwrap().local(&Local) - s.start.unwrap().local(&Local),
            Duration::minutes(15)
        );
        assert_eq!(
            s.exdates
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["2026-11-04", "2026-11-06"]
        );
        assert_eq!(s.description, None, "the alarm's lines are not the event's");
        assert_eq!(
            e[1].recurrence_id.map(|d| d.to_string()).as_deref(),
            Some("2026-11-09")
        );
        assert_eq!(e[1].reference(), "ics:standup@example.com@2026-11-09");
        let h = &e[2];
        assert_eq!(h.summary, "Holiday, the long one");
        assert_eq!(
            h.start,
            Some(When::Day(NaiveDate::from_ymd_opt(2026, 11, 26).unwrap()))
        );
        assert_eq!(
            h.description.as_deref(),
            Some("Line one\nline two that is long and is folded onto the next")
        );
        assert!(e[3].cancelled);
        assert_eq!(
            e[3].start.unwrap().local(&chrono::Utc).to_string(),
            "2026-11-10 17:00:00"
        );
    }

    #[test]
    fn durations_and_links() {
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(duration("P2W"), Some(Duration::weeks(2)));
        assert_eq!(duration("1H"), None);
        assert_eq!(
            feed_url("webcal://example.com/a.ics"),
            "https://example.com/a.ics"
        );
        assert_eq!(feed_url("https://x/y.ics"), "https://x/y.ics");
    }
}
