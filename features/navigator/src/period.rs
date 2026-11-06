//! Periods and date filters: the calendar's days, weeks, months, quarters
//! and years, and the `@2026-11` tokens a search narrows by.
//!
//! Notebook Navigator's `CalendarNoteKind` (`src/utils/calendarNotes.ts`),
//! its week math (Moment's `week()` and `isoWeek()` under a locale's
//! `dow` and `doy`), `buildDateFilterToken`
//! (`src/components/calendar/calendarUtils.ts`) and `parseDateFilterRange`
//! (`src/utils/filterSearchDate.ts`). A period is the same thing to the
//! calendar, to the periodic notes and to a search: the search (#338)
//! narrows a list with a [`DateFilter`], which a [`Period`] makes
//! ([`Period::filter_token`], [`Period::range`]).
//!
//! Dates are [`NaiveDate`]s, days in the person's time zone; a timestamp
//! becomes one through [`day_of`].

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::source::Millis;

/// What a periodic note covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeriodKind {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl PeriodKind {
    /// From the shortest to the longest: the order a parent is looked for
    /// in, and the order a path is tried against the patterns.
    pub const ALL: [Self; 5] = [
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::Year,
    ];

    /// Its id: `day`, `week`, `month`, `quarter`, `year`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
        }
    }

    /// The kind of an id; `daily`, `weekly`, … too.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|k| k.id() == id || k.adjective().eq_ignore_ascii_case(&id))
    }

    /// `daily`, `weekly`, `monthly`, `quarterly`, `yearly`.
    #[must_use]
    pub const fn adjective(self) -> &'static str {
        match self {
            Self::Day => "daily",
            Self::Week => "weekly",
            Self::Month => "monthly",
            Self::Quarter => "quarterly",
            Self::Year => "yearly",
        }
    }

    /// The kinds longer than it, nearest first.
    #[must_use]
    pub fn longer(self) -> &'static [Self] {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL.get(i.saturating_add(1)..).unwrap_or(&[])
    }
}

/// How weeks start and are numbered: Moment's locale `dow` (the first day,
/// 0 Sunday to 6 Saturday) and `doy` (week 1 is the week holding January
/// `7 + dow - doy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeekRules {
    pub dow: u8,
    pub doy: u8,
}

impl WeekRules {
    /// ISO 8601: weeks start on Monday, and week 1 holds January 4.
    pub const ISO: Self = Self { dow: 1, doy: 4 };
    /// Moment's `en` (the United States): weeks start on Sunday, and week
    /// 1 holds January 1.
    pub const SUNDAY: Self = Self { dow: 0, doy: 6 };
    /// Moment's `ar`: weeks start on Saturday, and week 1 holds January 1.
    pub const SATURDAY: Self = Self { dow: 6, doy: 12 };

    /// The day a week starts on.
    #[must_use]
    pub const fn first_day(self) -> Weekday {
        sunday_based(self.dow)
    }

    /// The first day of the week `date` is in.
    #[must_use]
    pub fn week_start(self, date: NaiveDate) -> NaiveDate {
        let from_sunday = date.weekday().num_days_from_sunday();
        let back = from_sunday.saturating_add(7).saturating_sub(u32::from(self.dow) % 7) % 7;
        date.checked_sub_days(Days::new(u64::from(back)))
            .unwrap_or(date)
    }

    /// The week year and the week number of `date` (Moment's `weekYear()`
    /// and `week()`; with [`Self::ISO`], `isoWeekYear()` and `isoWeek()`).
    // Moment's formula, over days of a year and weekdays: nothing near an
    // overflow.
    #[allow(clippy::arithmetic_side_effects)]
    #[must_use]
    pub fn week_of(self, date: NaiveDate) -> (i32, u32) {
        let year = date.year();
        let offset = self.first_week_offset(year);
        let doy = i64::from(date.ordinal());
        let week = (doy - offset - 1).div_euclid(7) + 1;
        if week < 1 {
            let prev = year - 1;
            let w = week + self.weeks_in_year(prev);
            (prev, u32::try_from(w).unwrap_or(1))
        } else if week > self.weeks_in_year(year) {
            let w = week - self.weeks_in_year(year);
            (year + 1, u32::try_from(w).unwrap_or(1))
        } else {
            (year, u32::try_from(week).unwrap_or(1))
        }
    }

    /// How many weeks `year` (a week year) has: 52 or 53.
    #[allow(clippy::arithmetic_side_effects)]
    #[must_use]
    pub fn weeks_in_year(self, year: i32) -> i64 {
        let days = if NaiveDate::from_ymd_opt(year, 2, 29).is_some() {
            366
        } else {
            365
        };
        (days - self.first_week_offset(year) + self.first_week_offset(year + 1)) / 7
    }

    /// Moment's `firstWeekOffset`: where week 1 starts, as a day of the
    /// year less one (it can be before January 1, so negative).
    #[allow(clippy::arithmetic_side_effects)]
    fn first_week_offset(self, year: i32) -> i64 {
        let dow = i64::from(self.dow);
        let fwd = 7 + dow - i64::from(self.doy);
        let jan = NaiveDate::from_ymd_opt(year, 1, 1)
            .and_then(|d| d.checked_add_days(Days::new(u64::try_from(fwd - 1).unwrap_or(0))))
            .map_or(0, |d| i64::from(d.weekday().num_days_from_sunday()));
        let fwdlw = (7 + jan - dow).rem_euclid(7);
        -fwdlw + fwd - 1
    }
}

/// A weekday from Moment's number, 0 Sunday to 6 Saturday.
#[must_use]
pub const fn sunday_based(n: u8) -> Weekday {
    match n % 7 {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        _ => Weekday::Sat,
    }
}

/// A day, a week, a month, a quarter or a year: what one periodic note
/// covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Period {
    pub kind: PeriodKind,
    /// Its first day.
    pub start: NaiveDate,
}

impl Period {
    /// The period of `kind` holding `date`, weeks starting as `weeks` say.
    #[must_use]
    pub fn containing(kind: PeriodKind, date: NaiveDate, weeks: WeekRules) -> Self {
        let start = match kind {
            PeriodKind::Day => date,
            PeriodKind::Week => weeks.week_start(date),
            PeriodKind::Month => date.with_day(1).unwrap_or(date),
            PeriodKind::Quarter => quarter_start(quarter_of(date))
                .and_then(|month| NaiveDate::from_ymd_opt(date.year(), month, 1))
                .unwrap_or(date),
            PeriodKind::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
        };
        Self { kind, start }
    }

    /// The period `n` of its kind on (back, when negative): Moment's
    /// `add(n, kind)` on its start.
    #[must_use]
    pub fn shift(self, n: i32) -> Self {
        let start = match self.kind {
            PeriodKind::Day => add_days(self.start, i64::from(n)),
            PeriodKind::Week => add_days(self.start, i64::from(n).saturating_mul(7)),
            PeriodKind::Month => add_months(self.start, n),
            PeriodKind::Quarter => add_months(self.start, n.saturating_mul(3)),
            PeriodKind::Year => add_months(self.start, n.saturating_mul(12)),
        };
        Self { start, ..self }
    }

    #[must_use]
    pub fn next(self) -> Self {
        self.shift(1)
    }

    #[must_use]
    pub fn previous(self) -> Self {
        self.shift(-1)
    }

    /// Its last day.
    #[must_use]
    pub fn end(self) -> NaiveDate {
        add_days(self.next().start, -1)
    }

    /// Whether `date` is in it.
    #[must_use]
    pub fn contains(self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end()
    }

    /// The period of `kind` its first day is in: a week's parent month is
    /// the month of its first day (`Open parent periodic note`).
    #[must_use]
    pub fn within(self, kind: PeriodKind, weeks: WeekRules) -> Self {
        Self::containing(kind, self.start, weeks)
    }

    /// The days it covers, as a filter.
    #[must_use]
    pub fn range(self) -> DateFilter {
        DateFilter {
            field: DateField::Default,
            start: Some(self.start),
            end: Some(self.next().start),
        }
    }

    /// The search token for it (`buildDateFilterToken`): `@2026-11-05`,
    /// `@2026-11-02..2026-11-08`, `@2026-11`, `@2026-Q4`, `@2026`.
    #[must_use]
    pub fn filter_token(self) -> String {
        let d = self.start;
        match self.kind {
            PeriodKind::Day => format!("@{}", d.format("%Y-%m-%d")),
            PeriodKind::Week => format!(
                "@{}..{}",
                d.format("%Y-%m-%d"),
                add_days(d, 6).format("%Y-%m-%d")
            ),
            PeriodKind::Month => format!("@{}", d.format("%Y-%m")),
            PeriodKind::Quarter => format!("@{}-Q{}", d.year(), quarter_of(d)),
            PeriodKind::Year => format!("@{}", d.year()),
        }
    }
}

/// The quarter `date` is in, 1 to 4.
#[must_use]
pub fn quarter_of(date: NaiveDate) -> u32 {
    (date.month0() / 3).saturating_add(1)
}

/// The month quarter `q` (1 to 4) starts in.
#[must_use]
pub const fn quarter_start(q: u32) -> Option<u32> {
    match q {
        1 => Some(1),
        2 => Some(4),
        3 => Some(7),
        4 => Some(10),
        _ => None,
    }
}

/// `date` moved `n` days, held at the calendar's ends.
#[must_use]
pub fn add_days(date: NaiveDate, n: i64) -> NaiveDate {
    let by = Days::new(n.unsigned_abs());
    if n >= 0 {
        date.checked_add_days(by)
    } else {
        date.checked_sub_days(by)
    }
    .unwrap_or(date)
}

/// `date` moved `n` months (to the month's last day when it is shorter).
#[must_use]
pub fn add_months(date: NaiveDate, n: i32) -> NaiveDate {
    let by = Months::new(n.unsigned_abs());
    if n >= 0 {
        date.checked_add_months(by)
    } else {
        date.checked_sub_months(by)
    }
    .unwrap_or(date)
}

/// The day a timestamp falls on, `offset` from UTC (both [`Millis`]).
#[must_use]
pub fn day_of(at: Millis, offset: Millis) -> NaiveDate {
    let days = at.saturating_add(offset).div_euclid(86_400_000);
    NaiveDate::from_ymd_opt(1970, 1, 1).map_or(NaiveDate::MIN, |epoch| add_days(epoch, days))
}

/// The timestamp a day starts at, `offset` from UTC.
#[must_use]
pub fn start_of(date: NaiveDate, offset: Millis) -> Millis {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or(NaiveDate::MIN);
    date.signed_duration_since(epoch)
        .num_days()
        .saturating_mul(86_400_000)
        .saturating_sub(offset)
}

/// Which date a filter is about (`@c:`, `@m:`, or the list's own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateField {
    /// The date the list sorts by.
    #[default]
    Default,
    Created,
    Modified,
}

/// How `@05/11/2026` reads when both ways are dates: the person's locale
/// order (`Intl.DateTimeFormat`'s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DayMonthOrder {
    /// Day, month, year.
    #[default]
    Dmy,
    /// Month, day, year (the United States).
    Mdy,
}

/// A search's date filter (`DateFilterRange`): days from `start` up to,
/// not including, `end`; either end may be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateFilter {
    pub field: DateField,
    pub start: Option<NaiveDate>,
    /// The first day after it.
    pub end: Option<NaiveDate>,
}

/// The words a date filter takes for days relative to today.
pub const RELATIVE: [&str; 6] = [
    "today",
    "yesterday",
    "last7d",
    "last30d",
    "thisweek",
    "thismonth",
];

impl DateFilter {
    /// The filter `token` writes (`@2026-11-05`, `@c:2026-11`,
    /// `@2026-Q4`, `@2026-w45`, `@2026-11-01..2026-11-07`, `@..2026`,
    /// `@today`, `@last7d`, `@05.11.2026`, …), or `None` when it is not
    /// one. `today` is the person's; `order` reads `@05/11/2026`.
    #[must_use]
    pub fn parse(token: &str, today: NaiveDate, order: DayMonthOrder) -> Option<Self> {
        let raw = token.strip_prefix('@')?;
        let (field, rest) = field_prefix(raw);
        let rest = rest.trim().to_ascii_lowercase();
        if rest.is_empty() {
            return None;
        }
        if RELATIVE.contains(&rest.as_str()) {
            let (start, end) = relative(&rest, today)?;
            return Some(Self {
                field,
                start: Some(start),
                end: Some(end),
            });
        }
        if let Some((left, right)) = rest.split_once("..") {
            let (left, right) = (left.trim(), right.trim());
            if left.is_empty() && right.is_empty() {
                return None;
            }
            let start = if left.is_empty() {
                None
            } else {
                Some(date_token(left, order)?.0)
            };
            let end = if right.is_empty() {
                None
            } else {
                Some(date_token(right, order)?.1)
            };
            if let (Some(s), Some(e)) = (start, end)
                && s >= e
            {
                return None;
            }
            return Some(Self { field, start, end });
        }
        let (start, end) = date_token(&rest, order)?;
        Some(Self {
            field,
            start: Some(start),
            end: Some(end),
        })
    }

    /// Whether `token` may become a filter as it is typed
    /// (`isDateFilterCandidate`): `@`, `@2`, `@.`, `@to`, …
    #[must_use]
    pub fn is_candidate(token: &str) -> bool {
        let Some(raw) = token.strip_prefix('@') else {
            return false;
        };
        let rest = field_prefix(raw).1.trim().to_ascii_lowercase();
        match rest.chars().next() {
            None => true,
            Some(c) if c == '.' || c.is_ascii_digit() => true,
            Some(_) => RELATIVE.iter().any(|k| k.starts_with(&rest)),
        }
    }

    /// Whether `date` is in it.
    #[must_use]
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start.is_none_or(|s| date >= s) && self.end.is_none_or(|e| date < e)
    }

    /// Whether the timestamp `at` is in it, `offset` from UTC.
    #[must_use]
    pub fn matches(&self, at: Millis, offset: Millis) -> bool {
        self.contains(day_of(at, offset))
    }
}

/// `c:`, `m:`, `created:` or `modified:` taken off the front.
fn field_prefix(raw: &str) -> (DateField, &str) {
    let lower = raw.to_ascii_lowercase();
    for (prefix, field) in [
        ("created:", DateField::Created),
        ("modified:", DateField::Modified),
        ("c:", DateField::Created),
        ("m:", DateField::Modified),
    ] {
        if lower.starts_with(prefix) {
            return (field, raw.get(prefix.len()..).unwrap_or(""));
        }
    }
    (DateField::Default, raw)
}

fn relative(word: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let tomorrow = add_days(today, 1);
    Some(match word {
        "today" => (today, tomorrow),
        "yesterday" => (add_days(today, -1), today),
        "last7d" => (add_days(today, -6), tomorrow),
        "last30d" => (add_days(today, -29), tomorrow),
        // Monday to Sunday, whatever the calendar's weeks.
        "thisweek" => {
            let start = WeekRules::ISO.week_start(today);
            (start, add_days(start, 7))
        }
        "thismonth" => {
            let start = today.with_day(1)?;
            (start, add_months(start, 1))
        }
        _ => return None,
    })
}

/// Digits only, `min` to `max` of them.
fn number(text: &str, min: usize, max: usize) -> Option<u32> {
    if !(min..=max).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

const fn is_sep(c: char) -> bool {
    matches!(c, '-' | '/' | '.')
}

/// A year, as a filter takes one (1 to 9999).
fn year(text: &str, min: usize) -> Option<i32> {
    number(text, min, 4)
        .and_then(|y| i32::try_from(y).ok())
        .filter(|y| (1..=9999).contains(y))
}

fn day(y: i32, m: u32, d: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(y, m, d)?;
    Some((start, add_days(start, 1)))
}

fn month(y: i32, m: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(y, m, 1)?;
    Some((start, add_months(start, 1)))
}

/// One side of a filter (`parseDateToken`): its first day and the day
/// after its last.
fn date_token(t: &str, order: DayMonthOrder) -> Option<(NaiveDate, NaiveDate)> {
    // A year: 2026.
    if let Some(y) = year(t, 3) {
        let start = NaiveDate::from_ymd_opt(y, 1, 1)?;
        return Some((start, add_months(start, 12)));
    }
    // A quarter or a week: 2026-q4, 2026q4, 2026-w45.
    for (mark, is_week) in [('q', false), ('w', true)] {
        if let Some(at) = t.find(mark) {
            let (left, right) = (t.get(..at)?, t.get(at.saturating_add(1)..)?);
            let left = left.strip_suffix(is_sep).unwrap_or(left);
            let y = year(left, 3)?;
            if is_week {
                let w = number(right, 1, 2)?;
                return iso_week(y, w);
            }
            let q = number(right, 1, 1)?;
            let start = NaiveDate::from_ymd_opt(y, quarter_start(q)?, 1)?;
            return Some((start, add_months(start, 3)));
        }
    }
    let parts: Vec<&str> = t.split(is_sep).collect();
    match parts.as_slice() {
        // 2026-11
        [y, m] => month(year(y, 3)?, number(m, 1, 2)?),
        // 2026-11-05, or 05-11-2026 / 11-05-2026
        [a, b, c] if a.len() >= 3 => day(year(a, 3)?, number(b, 1, 2)?, number(c, 1, 2)?),
        [a, b, c] => day_month_year(number(a, 1, 2)?, number(b, 1, 2)?, year(c, 3)?, order),
        // 202611, 20261105, 05112026
        [compact] if compact.len() == 6 => month(
            year(compact.get(..4)?, 4)?,
            number(compact.get(4..)?, 2, 2)?,
        ),
        [compact] if compact.len() == 8 => day(
            year(compact.get(..4)?, 4)?,
            number(compact.get(4..6)?, 2, 2)?,
            number(compact.get(6..)?, 2, 2)?,
        )
        .or_else(|| {
            day_month_year(
                number(compact.get(..2)?, 2, 2)?,
                number(compact.get(2..4)?, 2, 2)?,
                year(compact.get(4..)?, 4)?,
                order,
            )
        }),
        _ => None,
    }
}

/// `first`/`second`/`year`: the one way that is a date, else `order`'s.
fn day_month_year(
    first: u32,
    second: u32,
    y: i32,
    order: DayMonthOrder,
) -> Option<(NaiveDate, NaiveDate)> {
    let dmy = day(y, second, first);
    let mdy = day(y, first, second);
    match (dmy, mdy) {
        (Some(d), None) => Some(d),
        (None, Some(m)) => Some(m),
        (Some(d), Some(m)) => Some(if order == DayMonthOrder::Mdy { m } else { d }),
        (None, None) => None,
    }
}

/// ISO week `w` of ISO year `y`, Monday to Monday.
fn iso_week(y: i32, w: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_isoywd_opt(y, w, Weekday::Mon)?;
    Some((start, add_days(start, 7)))
}
