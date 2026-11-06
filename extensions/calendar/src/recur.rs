//! Repeats: RFC 5545's RRULE, the subset calendars write, expanded into
//! the times an event falls on.
//!
//! Task's calendar keeps a rule as text (`FREQ=WEEKLY;BYDAY=MO,TH`, no
//! `RRULE:`), and expands it with the `rrule` crate in the person's time
//! zone (`task:features/scheduling/scheduling-proto/src/recurrence.rs`
//! `expand_rrule`): an event at 09:00 stays at 09:00 across a clock
//! change. This does the same on wall-clock times ([`NaiveDateTime`]),
//! with no dependency: `FREQ` (daily, weekly, monthly, yearly),
//! `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with an ordinal under monthly
//! and yearly: `2TU`, `-1FR`), `BYMONTHDAY` (negative from the month's
//! end), `BYMONTH`, `BYSETPOS` and `WKST`. Hourly and finer rules are not
//! read; an event with one shows once, as Task's does with a bad rule.

use std::fmt::Write as _;

use chrono::{Datelike as _, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// At most this many times are made from one rule in one call (Task's
/// `MAX_INSTANCES`).
pub const MAX_INSTANCES: usize = 1024;

/// How often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    const fn word(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }
}

/// A rule, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    /// The last time it may fall on (inclusive). A date alone is its day's
    /// end.
    pub until: Option<NaiveDateTime>,
    /// Days, each with its ordinal in the month (or year) when it has one.
    pub by_day: Vec<(Option<i32>, Weekday)>,
    pub by_month_day: Vec<i32>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i32>,
    pub week_start: Weekday,
}

impl Rule {
    /// A plain one: every `interval` `freq`s.
    #[must_use]
    pub const fn every(freq: Freq, interval: u32) -> Self {
        Self {
            freq,
            interval,
            count: None,
            until: None,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month: Vec::new(),
            by_set_pos: Vec::new(),
            week_start: Weekday::Mon,
        }
    }

    /// Reads a rule (`RRULE:` optional); `None` when it is not one this
    /// reads.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text
            .strip_prefix("RRULE:")
            .or_else(|| text.strip_prefix("rrule:"))
            .unwrap_or(text);
        let mut rule = Self::every(Freq::Daily, 1);
        let mut freq = None;
        for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=')?;
            let v = v.trim();
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    });
                }
                "INTERVAL" => rule.interval = v.parse().ok().filter(|n| *n > 0)?,
                "COUNT" => rule.count = Some(v.parse().ok()?),
                "UNTIL" => rule.until = Some(until(v)?),
                "BYDAY" => {
                    for d in v.split(',') {
                        rule.by_day.push(by_day(d.trim())?);
                    }
                }
                "BYMONTHDAY" => {
                    for d in v.split(',') {
                        let n: i32 = d.trim().parse().ok()?;
                        (n != 0 && n.abs() <= 31).then_some(())?;
                        rule.by_month_day.push(n);
                    }
                }
                "BYMONTH" => {
                    for m in v.split(',') {
                        let n: u32 = m.trim().parse().ok()?;
                        (1..=12).contains(&n).then_some(())?;
                        rule.by_month.push(n);
                    }
                }
                "BYSETPOS" => {
                    for p in v.split(',') {
                        let n: i32 = p.trim().parse().ok()?;
                        (n != 0).then_some(())?;
                        rule.by_set_pos.push(n);
                    }
                }
                "WKST" => rule.week_start = weekday(v)?,
                // Anything else (BYHOUR, …) is not one this reads.
                _ => return None,
            }
        }
        rule.freq = freq?;
        Some(rule)
    }

    /// The rule as calendars write it, `FREQ` first, no `RRULE:`.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = format!("FREQ={}", self.freq.word());
        if self.interval > 1 {
            let _ = write!(out, ";INTERVAL={}", self.interval);
        }
        if !self.by_day.is_empty() {
            let days: Vec<String> = self
                .by_day
                .iter()
                .map(|(n, d)| {
                    format!(
                        "{}{}",
                        n.map(|n| n.to_string()).unwrap_or_default(),
                        day_code(*d)
                    )
                })
                .collect();
            let _ = write!(out, ";BYDAY={}", days.join(","));
        }
        if !self.by_month_day.is_empty() {
            let days: Vec<String> = self.by_month_day.iter().map(ToString::to_string).collect();
            let _ = write!(out, ";BYMONTHDAY={}", days.join(","));
        }
        if !self.by_month.is_empty() {
            let months: Vec<String> = self.by_month.iter().map(ToString::to_string).collect();
            let _ = write!(out, ";BYMONTH={}", months.join(","));
        }
        if !self.by_set_pos.is_empty() {
            let pos: Vec<String> = self.by_set_pos.iter().map(ToString::to_string).collect();
            let _ = write!(out, ";BYSETPOS={}", pos.join(","));
        }
        if let Some(n) = self.count {
            let _ = write!(out, ";COUNT={n}");
        }
        if let Some(u) = self.until {
            let _ = write!(out, ";UNTIL={}", u.format("%Y%m%dT%H%M%S"));
        }
        if self.week_start != Weekday::Mon {
            let _ = write!(out, ";WKST={}", day_code(self.week_start));
        }
        out
    }

    /// The times it falls on from `start` (the event's first, which is
    /// always one) that are at or after `from` and before `to`, in order,
    /// at most [`MAX_INSTANCES`].
    #[must_use]
    pub fn between(
        &self,
        start: NaiveDateTime,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> Vec<NaiveDateTime> {
        let mut out = Vec::new();
        let mut made: u32 = 0;
        let time = start.time();
        let step = i64::from(self.interval);
        // Periods are walked from the first; a long one is cut at the end
        // of what is asked or at its count, and never past 50,000.
        for k in 0..50_000_i64 {
            let Some((begins, days)) = self.period(start.date(), k * step) else {
                break;
            };
            if begins.and_time(NaiveTime::MIN) >= to {
                break;
            }
            for day in days {
                let at = day.and_time(time);
                if at < start {
                    continue;
                }
                if self.until.is_some_and(|u| at > u) {
                    return out;
                }
                if self.count.is_some_and(|c| made >= c) {
                    return out;
                }
                made = made.saturating_add(1);
                if at >= to {
                    return out;
                }
                if at >= from {
                    out.push(at);
                    if out.len() >= MAX_INSTANCES {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// The period `n` periods after `first`'s: the day it begins, and its
    /// days in order, the candidates its BY parts allow. `None` past
    /// chrono's range.
    fn period(&self, first: NaiveDate, n: i64) -> Option<(NaiveDate, Vec<NaiveDate>)> {
        let begins = match self.freq {
            Freq::Daily => first.checked_add_signed(Duration::days(n))?,
            Freq::Weekly => first
                .checked_add_signed(Duration::weeks(n))?
                .checked_sub_signed(Duration::days(i64::from(
                    (7 + first.weekday().num_days_from_monday()
                        - self.week_start.num_days_from_monday())
                        % 7,
                )))?,
            Freq::Monthly => {
                let (y, m) = add_months(first.year(), first.month(), n)?;
                NaiveDate::from_ymd_opt(y, m, 1)?
            }
            Freq::Yearly => {
                NaiveDate::from_ymd_opt(first.year().checked_add(i32::try_from(n).ok()?)?, 1, 1)?
            }
        };
        let mut days = match self.freq {
            Freq::Daily => {
                let d = begins;
                let ok = (self.by_month.is_empty() || self.by_month.contains(&d.month()))
                    && (self.by_month_day.is_empty() || month_day_matches(d, &self.by_month_day))
                    && (self.by_day.is_empty()
                        || self.by_day.iter().any(|(_, w)| *w == d.weekday()));
                if ok { vec![d] } else { Vec::new() }
            }
            Freq::Weekly => {
                let week = begins;
                let wanted: Vec<Weekday> = if self.by_day.is_empty() {
                    vec![first.weekday()]
                } else {
                    self.by_day.iter().map(|(_, w)| *w).collect()
                };
                (0..7)
                    .filter_map(|i| week.checked_add_signed(Duration::days(i)))
                    .filter(|d| wanted.contains(&d.weekday()))
                    .filter(|d| self.by_month.is_empty() || self.by_month.contains(&d.month()))
                    .collect()
            }
            Freq::Monthly => {
                let (y, m) = add_months(first.year(), first.month(), n)?;
                if !self.by_month.is_empty() && !self.by_month.contains(&m) {
                    Vec::new()
                } else {
                    self.month_days(y, m, first.day())
                }
            }
            Freq::Yearly => {
                let y = first.year().checked_add(i32::try_from(n).ok()?)?;
                let months: Vec<u32> = if self.by_month.is_empty() {
                    if self.by_day.iter().any(|(o, _)| o.is_some()) && self.by_month_day.is_empty()
                    {
                        // `BYDAY=20MO` alone counts through the year.
                        return Some((begins, self.set_pos(year_weekdays(y, &self.by_day))));
                    }
                    vec![first.month()]
                } else {
                    let mut m = self.by_month.clone();
                    m.sort_unstable();
                    m
                };
                months
                    .into_iter()
                    .flat_map(|m| self.month_days(y, m, first.day()))
                    .collect()
            }
        };
        days.sort_unstable();
        days.dedup();
        Some((begins, self.set_pos(days)))
    }

    /// A month's days by `BYMONTHDAY` and `BYDAY` (both: those in each),
    /// or the first event's day of the month.
    fn month_days(&self, y: i32, m: u32, day: u32) -> Vec<NaiveDate> {
        let all: Vec<NaiveDate> = (1..=31)
            .filter_map(|d| NaiveDate::from_ymd_opt(y, m, d))
            .collect();
        match (self.by_month_day.is_empty(), self.by_day.is_empty()) {
            // A month without that day (the 31st) is skipped, as RFC 5545
            // says.
            (true, true) => NaiveDate::from_ymd_opt(y, m, day).into_iter().collect(),
            (false, true) => all
                .into_iter()
                .filter(|d| month_day_matches(*d, &self.by_month_day))
                .collect(),
            (true, false) => all
                .iter()
                .copied()
                .filter(|d| weekday_in(*d, &all, &self.by_day))
                .collect(),
            (false, false) => all
                .iter()
                .copied()
                .filter(|d| {
                    month_day_matches(*d, &self.by_month_day) && weekday_in(*d, &all, &self.by_day)
                })
                .collect(),
        }
    }

    /// `BYSETPOS`: the n-th of a period's days (negative from its end).
    fn set_pos(&self, days: Vec<NaiveDate>) -> Vec<NaiveDate> {
        if self.by_set_pos.is_empty() {
            return days;
        }
        let len = i32::try_from(days.len()).unwrap_or(i32::MAX);
        let mut out: Vec<NaiveDate> = self
            .by_set_pos
            .iter()
            .filter_map(|p| {
                let i = if *p > 0 { p - 1 } else { len + p };
                usize::try_from(i).ok().and_then(|i| days.get(i).copied())
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// In words: "Every week on Mon, Thu", "Every 2 months", "Daily".
    #[must_use]
    pub fn describe(&self) -> String {
        let unit = match self.freq {
            Freq::Daily => "day",
            Freq::Weekly => "week",
            Freq::Monthly => "month",
            Freq::Yearly => "year",
        };
        let mut out = match (self.interval, self.freq) {
            (1, Freq::Daily) => "Daily".to_owned(),
            (1, Freq::Weekly) => "Weekly".to_owned(),
            (1, Freq::Monthly) => "Monthly".to_owned(),
            (1, Freq::Yearly) => "Yearly".to_owned(),
            (2, Freq::Weekly) => "Every other week".to_owned(),
            (n, _) => format!("Every {n} {unit}s"),
        };
        let weekdays = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ];
        if !self.by_day.is_empty() {
            let plain: Vec<Weekday> = self
                .by_day
                .iter()
                .filter(|(o, _)| o.is_none())
                .map(|(_, d)| *d)
                .collect();
            if plain.len() == 5
                && weekdays.iter().all(|d| plain.contains(d))
                && self.by_day.len() == 5
            {
                out.push_str(" on weekdays");
            } else {
                let names: Vec<String> = self
                    .by_day
                    .iter()
                    .map(|(o, d)| {
                        o.map_or_else(
                            || day_name(*d).to_owned(),
                            |n| format!("{} {}", ordinal(n), day_name(*d)),
                        )
                    })
                    .collect();
                let _ = write!(out, " on {}", names.join(", "));
            }
        } else if !self.by_month_day.is_empty() {
            let days: Vec<String> = self.by_month_day.iter().map(|d| ordinal(*d)).collect();
            let _ = write!(out, " on the {}", days.join(", "));
        }
        if let Some(n) = self.count {
            let _ = write!(out, ", {n} times");
        }
        if let Some(u) = self.until {
            let _ = write!(out, ", until {}", u.date().format("%b %-d, %Y"));
        }
        out
    }

    /// A copy that stops before `day`: what ending a series there writes
    /// (`UNTIL` the day before, `COUNT` dropped).
    #[must_use]
    pub fn ending_before(&self, day: NaiveDate) -> Self {
        let mut r = self.clone();
        r.count = None;
        r.until = day
            .pred_opt()
            .map(|d| d.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or(NaiveTime::MIN)));
        r
    }
}

/// The rules the event editor offers (Task's `REPEAT_PRESETS`), with
/// their labels; the first is none.
pub const PRESETS: &[(&str, &str)] = &[
    ("", "Does not repeat"),
    ("FREQ=DAILY", "Daily"),
    ("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "Every weekday"),
    ("FREQ=WEEKLY", "Weekly"),
    ("FREQ=WEEKLY;INTERVAL=2", "Every other week"),
    ("FREQ=MONTHLY", "Monthly"),
    ("FREQ=YEARLY", "Yearly"),
];

/// A rule's words, or the rule itself when it is not one this reads.
#[must_use]
pub fn describe(text: &str) -> String {
    Rule::parse(text).map_or_else(|| text.to_owned(), |r| r.describe())
}

fn until(v: &str) -> Option<NaiveDateTime> {
    let v = v.trim_end_matches('Z');
    if let Ok(t) = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S") {
        return Some(t);
    }
    let d = NaiveDate::parse_from_str(v, "%Y%m%d")
        .or_else(|_| NaiveDate::parse_from_str(v, "%Y-%m-%d"))
        .ok()?;
    d.and_hms_opt(23, 59, 59)
}

fn by_day(s: &str) -> Option<(Option<i32>, Weekday)> {
    let split = s.len().checked_sub(2)?;
    let (n, d) = s.split_at(split);
    let n = if n.is_empty() {
        None
    } else {
        let n: i32 = n.trim_start_matches('+').parse().ok()?;
        (n != 0 && n.abs() <= 53).then_some(n)
    };
    Some((n, weekday(d)?))
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// A weekday as RRULE writes it.
#[must_use]
pub const fn day_code(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

const fn day_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn ordinal(n: i32) -> String {
    match n {
        -1 => "last".to_owned(),
        n if n < 0 => format!("{} from last", ordinal(-n)),
        n => {
            let suffix = match (n % 10, n % 100) {
                (1, x) if x != 11 => "st",
                (2, x) if x != 12 => "nd",
                (3, x) if x != 13 => "rd",
                _ => "th",
            };
            format!("{n}{suffix}")
        }
    }
}

fn add_months(y: i32, m: u32, n: i64) -> Option<(i32, u32)> {
    let total = i64::from(y) * 12 + i64::from(m) - 1 + n;
    let year = i32::try_from(total.div_euclid(12)).ok()?;
    let month = u32::try_from(total.rem_euclid(12)).ok()? + 1;
    Some((year, month))
}

fn days_in_month(d: NaiveDate) -> i32 {
    let (y, m) = add_months(d.year(), d.month(), 1).unwrap_or_else(|| (d.year(), d.month()));
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|n| n.pred_opt())
        .map_or(31, |last| i32::try_from(last.day()).unwrap_or(31))
}

fn month_day_matches(d: NaiveDate, days: &[i32]) -> bool {
    let day = i32::try_from(d.day()).unwrap_or(0);
    let len = days_in_month(d);
    days.iter().any(|n| {
        if *n > 0 {
            *n == day
        } else {
            len + n + 1 == day
        }
    })
}

/// Whether `d` is one of `by`'s days in its month (`all`): any such
/// weekday, or the n-th (from the end when negative).
fn weekday_in(d: NaiveDate, all: &[NaiveDate], by: &[(Option<i32>, Weekday)]) -> bool {
    by.iter().any(|(n, w)| {
        if d.weekday() != *w {
            return false;
        }
        let Some(n) = n else { return true };
        let same: Vec<&NaiveDate> = all.iter().filter(|x| x.weekday() == *w).collect();
        let len = i32::try_from(same.len()).unwrap_or(0);
        let i = if *n > 0 { n - 1 } else { len + n };
        usize::try_from(i)
            .ok()
            .and_then(|i| same.get(i))
            .is_some_and(|x| **x == d)
    })
}

/// A year's n-th weekdays (`BYDAY=20MO` in a yearly rule).
fn year_weekdays(y: i32, by: &[(Option<i32>, Weekday)]) -> Vec<NaiveDate> {
    let all: Vec<NaiveDate> = (1..=366)
        .filter_map(|o| NaiveDate::from_yo_opt(y, o))
        .collect();
    let mut out: Vec<NaiveDate> = all
        .iter()
        .copied()
        .filter(|d| weekday_in(*d, &all, by))
        .collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn days(rule: &str, start: &str, from: &str, to: &str) -> Vec<String> {
        Rule::parse(rule)
            .unwrap()
            .between(at(start), at(from), at(to))
            .iter()
            .map(|t| t.format("%Y-%m-%d").to_string())
            .collect()
    }

    // Task's `weekly_expands_on_bydays`.
    #[test]
    fn weekly_expands_on_its_days() {
        assert_eq!(
            days(
                "FREQ=WEEKLY;BYDAY=MO,TH",
                "2026-11-02 09:00",
                "2026-11-01 00:00",
                "2026-11-15 00:00"
            ),
            ["2026-11-02", "2026-11-05", "2026-11-09", "2026-11-12"]
        );
    }

    // `biweekly_skips_the_off_week`.
    #[test]
    fn every_other_week_skips_one() {
        assert_eq!(
            days(
                "FREQ=WEEKLY;INTERVAL=2",
                "2026-11-03 18:00",
                "2026-11-01 00:00",
                "2026-12-01 00:00"
            ),
            ["2026-11-03", "2026-11-17"]
        );
    }

    // `monthly_repeats_on_the_anchor_day`; a month without the 31st is skipped.
    #[test]
    fn monthly_keeps_its_day() {
        assert_eq!(
            days(
                "FREQ=MONTHLY",
                "2026-01-31 10:00",
                "2026-01-01 00:00",
                "2026-06-01 00:00"
            ),
            ["2026-01-31", "2026-03-31", "2026-05-31"]
        );
        assert_eq!(
            days(
                "FREQ=MONTHLY;BYDAY=-1FR",
                "2026-01-30 10:00",
                "2026-01-01 00:00",
                "2026-04-01 00:00"
            ),
            ["2026-01-30", "2026-02-27", "2026-03-27"]
        );
        assert_eq!(
            days(
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
                "2026-01-30 10:00",
                "2026-01-01 00:00",
                "2026-04-01 00:00"
            ),
            ["2026-01-30", "2026-02-27", "2026-03-31"]
        );
        assert_eq!(
            days(
                "FREQ=MONTHLY;BYMONTHDAY=-1",
                "2026-01-31 10:00",
                "2026-01-01 00:00",
                "2026-04-01 00:00"
            ),
            ["2026-01-31", "2026-02-28", "2026-03-31"]
        );
    }

    #[test]
    fn count_and_until_end_it() {
        // COUNT counts from the first, even before the window.
        assert_eq!(
            days(
                "FREQ=DAILY;COUNT=5",
                "2026-11-01 08:00",
                "2026-11-03 00:00",
                "2026-12-01 00:00"
            ),
            ["2026-11-03", "2026-11-04", "2026-11-05"]
        );
        assert_eq!(
            days(
                "FREQ=WEEKLY;UNTIL=20261115",
                "2026-11-01 08:00",
                "2026-11-01 00:00",
                "2026-12-01 00:00"
            ),
            ["2026-11-01", "2026-11-08", "2026-11-15"]
        );
    }

    #[test]
    fn yearly_and_its_months() {
        assert_eq!(
            days(
                "FREQ=YEARLY",
                "2024-02-29 09:00",
                "2024-01-01 00:00",
                "2029-01-01 00:00"
            ),
            ["2024-02-29", "2028-02-29"]
        );
        // Thanksgiving: the fourth Thursday of November.
        assert_eq!(
            days(
                "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
                "2026-11-26 12:00",
                "2026-01-01 00:00",
                "2028-01-01 00:00"
            ),
            ["2026-11-26", "2027-11-25"]
        );
    }

    // `bad_rules_return_none`.
    #[test]
    fn rules_it_cannot_read_are_none() {
        assert!(Rule::parse("FREQ=HOURLY").is_none());
        assert!(Rule::parse("FREQ=WEEKLY;BYDAY=XX").is_none());
        assert!(Rule::parse("BYDAY=MO").is_none());
        assert!(Rule::parse("FREQ=DAILY;BYHOUR=9").is_none());
    }

    #[test]
    fn it_writes_back_what_it_read() {
        for r in [
            "FREQ=WEEKLY;BYDAY=MO,TH",
            "FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR",
            "FREQ=DAILY;COUNT=3",
            "FREQ=YEARLY;BYDAY=4TH;BYMONTH=11",
            "FREQ=WEEKLY;UNTIL=20261115T235959",
        ] {
            assert_eq!(Rule::parse(r).unwrap().text(), r);
        }
        assert_eq!(
            Rule::parse("RRULE:FREQ=DAILY").unwrap().text(),
            "FREQ=DAILY"
        );
    }

    // `describes_common_cadences`.
    #[test]
    fn it_says_what_it_is() {
        assert_eq!(describe("FREQ=DAILY"), "Daily");
        assert_eq!(
            describe("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
            "Weekly on weekdays"
        );
        assert_eq!(describe("FREQ=WEEKLY;INTERVAL=2"), "Every other week");
        assert_eq!(describe("FREQ=MONTHLY;BYDAY=-1FR"), "Monthly on last Fri");
        assert_eq!(
            describe("FREQ=MONTHLY;INTERVAL=3;COUNT=4"),
            "Every 3 months, 4 times"
        );
    }

    #[test]
    fn ending_a_series_before_a_day() {
        let r = Rule::parse("FREQ=DAILY;COUNT=10")
            .unwrap()
            .ending_before(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap());
        assert_eq!(r.text(), "FREQ=DAILY;UNTIL=20261103T235959");
        assert_eq!(
            r.between(
                at("2026-11-01 09:00"),
                at("2026-11-01 00:00"),
                at("2026-12-01 00:00")
            )
            .len(),
            3
        );
    }

    #[test]
    fn a_long_rule_stops_at_the_cap() {
        let r = Rule::parse("FREQ=DAILY").unwrap();
        assert_eq!(
            r.between(
                at("2000-01-01 09:00"),
                at("2000-01-01 00:00"),
                at("2100-01-01 00:00")
            )
            .len(),
            MAX_INSTANCES
        );
    }
}
