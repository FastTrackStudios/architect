//! Moment date patterns: how a periodic note's path is written
//! (`YYYY/YYYYMMDD`, `gggg/[W]ww`, `[Journal]/YYYY-[Q]Q`).
//!
//! Notebook Navigator formats a note's path with Moment and reads one back
//! with a strict parse checked by formatting again
//! (`src/utils/calendarCustomNotePatterns.ts`,
//! `src/components/calendar/calendarNoteResolution.ts`). This is the part
//! of Moment's format a path uses, in English: the date tokens (years,
//! week years, months, quarters, weeks, days, weekdays, ordinals),
//! `[literals]` and `\` escapes; time tokens format midnight. A letter that
//! is no token is itself, as in Moment.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::period::{WeekRules, add_days};

/// Moment's `en` month names.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Moment's `en` weekday names, Sunday first.
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Moment's `en` `weekdaysMin`, Sunday first: the calendar's header.
pub const WEEKDAYS_MIN: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// The month's name, long (`January`) or short (`Jan`).
#[must_use]
pub fn month_name(month0: u32, short: bool) -> &'static str {
    let name = MONTHS
        .get(usize::try_from(month0).unwrap_or(0))
        .copied()
        .unwrap_or("");
    if short {
        name.get(..3).unwrap_or(name)
    } else {
        name
    }
}

/// A token of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum T {
    /// `YYYY` (4), `YY` (2), `Y` (any), `YYYYY` (5), `YYYYYY` (6).
    Year(u8),
    /// `gggg`, `gg`, `ggggg`; ISO: `GGGG`, `GG`, `GGGGG`.
    WeekYear {
        iso: bool,
        len: u8,
    },
    /// `M`, `MM`.
    Month(u8),
    /// `MMM`, `MMMM`.
    MonthName {
        short: bool,
    },
    /// `Mo`.
    MonthOrd,
    /// `Q`, `Qo`.
    Quarter {
        ord: bool,
    },
    /// `D`, `DD`, `Do`.
    Day(u8),
    DayOrd,
    /// `DDD`, `DDDD`, `DDDo`.
    DayOfYear(u8),
    DayOfYearOrd,
    /// `d` (0 Sunday to 6), `do`, `e` (0 to 6 from the locale's first
    /// day), `E` (1 Monday to 7).
    Weekday,
    WeekdayOrd,
    LocaleWeekday,
    IsoWeekday,
    /// `dd`, `ddd`, `dddd`.
    WeekdayName(u8),
    /// `w`, `ww`, `wo`; ISO: `W`, `WW`, `Wo`.
    Week {
        iso: bool,
        len: u8,
    },
    WeekOrd {
        iso: bool,
    },
    /// The time tokens: `H` `HH` `k` `kk` `h` `hh` `m` `mm` `s` `ss` `S…`
    /// `a` `A`; a date alone writes them at midnight.
    Time(Tm),
    /// The time zone's offset: `Z` (`+01:00`), `ZZ` (`+0100`).
    Zone {
        colon: bool,
    },
}

/// A time token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tm {
    /// `H`, `HH`: 0 to 23.
    Hour24(u8),
    /// `k`, `kk`: 1 to 24.
    Hour1To24(u8),
    /// `h`, `hh`: 1 to 12.
    Hour12(u8),
    Minute(u8),
    Second(u8),
    /// `S`, `SS`, `SSS`: tenths, hundredths, thousandths.
    Fraction(u8),
    /// `a` `am`/`pm`, `A` `AM`/`PM`.
    Meridiem {
        upper: bool,
    },
}

impl Tm {
    /// The token at `t`.
    fn write(self, t: NaiveTime) -> String {
        let n = |v: u32, w: u8| format!("{v:0w$}", w = usize::from(w));
        match self {
            Self::Hour24(w) => n(t.hour(), w),
            Self::Hour1To24(w) => n(if t.hour() == 0 { 24 } else { t.hour() }, w),
            Self::Hour12(w) => n(
                match t.hour() % 12 {
                    0 => 12,
                    h => h,
                },
                w,
            ),
            Self::Minute(w) => n(t.minute(), w),
            Self::Second(w) => n(t.second(), w),
            Self::Fraction(w) => {
                let ms = t.nanosecond() / 1_000_000 % 1000;
                let v = match w {
                    1 => ms / 100,
                    2 => ms / 10,
                    _ => ms,
                };
                n(v, w)
            }
            Self::Meridiem { upper } => {
                let pm = t.hour() >= 12;
                match (pm, upper) {
                    (false, false) => "am",
                    (true, false) => "pm",
                    (false, true) => "AM",
                    (true, true) => "PM",
                }
                .to_owned()
            }
        }
    }
}

/// A pattern's parts: tokens, and literal text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Lit(String),
    Tok(T),
}

/// Moment's tokens, the longest first where one starts another.
const TOKENS: &[(&str, T)] = &[
    ("YYYYYY", T::Year(6)),
    ("YYYYY", T::Year(5)),
    ("YYYY", T::Year(4)),
    ("YY", T::Year(2)),
    ("Y", T::Year(0)),
    ("yyyy", T::Year(4)),
    ("yy", T::Year(2)),
    ("y", T::Year(0)),
    ("ggggg", T::WeekYear { iso: false, len: 5 }),
    ("gggg", T::WeekYear { iso: false, len: 4 }),
    ("gg", T::WeekYear { iso: false, len: 2 }),
    ("GGGGG", T::WeekYear { iso: true, len: 5 }),
    ("GGGG", T::WeekYear { iso: true, len: 4 }),
    ("GG", T::WeekYear { iso: true, len: 2 }),
    ("MMMM", T::MonthName { short: false }),
    ("MMM", T::MonthName { short: true }),
    ("MM", T::Month(2)),
    ("Mo", T::MonthOrd),
    ("M", T::Month(1)),
    ("Qo", T::Quarter { ord: true }),
    ("Q", T::Quarter { ord: false }),
    ("DDDD", T::DayOfYear(3)),
    ("DDDo", T::DayOfYearOrd),
    ("DDD", T::DayOfYear(1)),
    ("DD", T::Day(2)),
    ("Do", T::DayOrd),
    ("D", T::Day(1)),
    ("dddd", T::WeekdayName(4)),
    ("ddd", T::WeekdayName(3)),
    ("dd", T::WeekdayName(2)),
    ("do", T::WeekdayOrd),
    ("d", T::Weekday),
    ("e", T::LocaleWeekday),
    ("E", T::IsoWeekday),
    ("ww", T::Week { iso: false, len: 2 }),
    ("wo", T::WeekOrd { iso: false }),
    ("w", T::Week { iso: false, len: 1 }),
    ("WW", T::Week { iso: true, len: 2 }),
    ("Wo", T::WeekOrd { iso: true }),
    ("W", T::Week { iso: true, len: 1 }),
    ("HH", T::Time(Tm::Hour24(2))),
    ("H", T::Time(Tm::Hour24(1))),
    ("kk", T::Time(Tm::Hour1To24(2))),
    ("k", T::Time(Tm::Hour1To24(1))),
    ("hh", T::Time(Tm::Hour12(2))),
    ("h", T::Time(Tm::Hour12(1))),
    ("mm", T::Time(Tm::Minute(2))),
    ("m", T::Time(Tm::Minute(1))),
    ("ss", T::Time(Tm::Second(2))),
    ("s", T::Time(Tm::Second(1))),
    ("SSS", T::Time(Tm::Fraction(3))),
    ("SS", T::Time(Tm::Fraction(2))),
    ("S", T::Time(Tm::Fraction(1))),
    ("a", T::Time(Tm::Meridiem { upper: false })),
    ("A", T::Time(Tm::Meridiem { upper: true })),
    ("ZZ", T::Zone { colon: false }),
    ("Z", T::Zone { colon: true }),
];

/// A Moment date pattern, read into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    source: String,
    parts: Vec<Part>,
}

/// What a parse read from the text: each field the pattern has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fields {
    pub year: Option<i32>,
    pub week_year: Option<i32>,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub day_of_year: Option<u32>,
    pub quarter: Option<u32>,
    pub week: Option<u32>,
}

impl Fields {
    /// The calendar year it names: `YYYY`'s, else a week year's (Notebook
    /// Navigator reads `gggg` as `YYYY` to find a week).
    #[must_use]
    pub fn any_year(&self) -> Option<i32> {
        self.year.or(self.week_year)
    }
}

impl Pattern {
    /// `source` read as Moment does.
    #[must_use]
    pub fn new(source: &str) -> Self {
        let mut parts: Vec<Part> = Vec::new();
        let push_lit = |parts: &mut Vec<Part>, s: &str| {
            if let Some(Part::Lit(last)) = parts.last_mut() {
                last.push_str(s);
            } else if !s.is_empty() {
                parts.push(Part::Lit(s.to_owned()));
            }
        };
        let mut rest = source;
        while let Some(c) = rest.chars().next() {
            if c == '[' {
                // A literal: up to the next `]`, with no `[` inside.
                if let Some(end) = rest.get(1..).and_then(|r| r.find(']'))
                    && !rest.get(1..=end).unwrap_or("").contains('[')
                {
                    push_lit(&mut parts, rest.get(1..=end).unwrap_or(""));
                    rest = rest.get(end.saturating_add(2)..).unwrap_or("");
                    continue;
                }
            }
            if c == '\\' {
                let next = rest.get(1..).and_then(|r| r.chars().next());
                if let Some(n) = next {
                    push_lit(&mut parts, &n.to_string());
                    rest = rest.get(n.len_utf8().saturating_add(1)..).unwrap_or("");
                    continue;
                }
            }
            if let Some((text, t)) = TOKENS.iter().find(|(text, _)| rest.starts_with(text)) {
                parts.push(Part::Tok(*t));
                rest = rest.get(text.len()..).unwrap_or("");
                continue;
            }
            push_lit(&mut parts, &c.to_string());
            rest = rest.get(c.len_utf8()..).unwrap_or("");
        }
        Self {
            source: source.to_owned(),
            parts,
        }
    }

    /// The text it was read from.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    fn tokens(&self) -> impl Iterator<Item = T> + '_ {
        self.parts.iter().filter_map(|p| match p {
            Part::Tok(t) => Some(*t),
            Part::Lit(_) => None,
        })
    }

    /// It has a year: `Y`, `g` or `G`.
    #[must_use]
    pub fn has_year(&self) -> bool {
        self.tokens()
            .any(|t| matches!(t, T::Year(_) | T::WeekYear { .. }))
    }

    /// It has a month (`M…`).
    #[must_use]
    pub fn has_month(&self) -> bool {
        self.tokens()
            .any(|t| matches!(t, T::Month(_) | T::MonthName { .. } | T::MonthOrd))
    }

    /// It has a day of the month or of the year (`D…`).
    #[must_use]
    pub fn has_day(&self) -> bool {
        self.tokens()
            .any(|t| matches!(t, T::Day(_) | T::DayOrd | T::DayOfYear(_) | T::DayOfYearOrd))
    }

    #[must_use]
    pub fn has_quarter(&self) -> bool {
        self.tokens().any(|t| matches!(t, T::Quarter { .. }))
    }

    /// It has a week number (`w` or `W`).
    #[must_use]
    pub fn has_week(&self) -> bool {
        self.tokens()
            .any(|t| matches!(t, T::Week { .. } | T::WeekOrd { .. }))
    }

    /// It has a week year (`g` or `G`).
    #[must_use]
    pub fn has_week_year(&self) -> bool {
        self.tokens().any(|t| matches!(t, T::WeekYear { .. }))
    }

    /// It uses ISO weeks (`W`, `G` or `E`): its weeks start on Monday
    /// (`getCalendarCustomWeekAnchorUnit`).
    #[must_use]
    pub fn uses_iso_weeks(&self) -> bool {
        self.tokens().any(|t| {
            matches!(
                t,
                T::Week { iso: true, .. }
                    | T::WeekOrd { iso: true }
                    | T::WeekYear { iso: true, .. }
            )
        })
    }

    /// It uses the locale's weeks (`w` or `g`).
    #[must_use]
    pub fn uses_locale_weeks(&self) -> bool {
        self.tokens().any(|t| {
            matches!(
                t,
                T::Week { iso: false, .. }
                    | T::WeekOrd { iso: false }
                    | T::WeekYear { iso: false, .. }
            )
        })
    }

    /// `date` written in it; locale weeks (`w`, `g`, `e`) by `weeks`. Its
    /// time tokens write midnight, its zone UTC.
    #[must_use]
    pub fn format(&self, date: NaiveDate, weeks: WeekRules) -> String {
        self.format_at(date.and_time(NaiveTime::MIN), 0, weeks)
    }

    /// `at` written in it, in a time zone `offset` seconds east of UTC
    /// (for `Z`); locale weeks by `weeks`.
    #[must_use]
    pub fn format_at(&self, at: NaiveDateTime, offset: i32, weeks: WeekRules) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Lit(s) => out.push_str(s),
                Part::Tok(T::Time(t)) => out.push_str(&t.write(at.time())),
                Part::Tok(T::Zone { colon }) => out.push_str(&zone(offset, *colon)),
                Part::Tok(t) => out.push_str(&token(*t, at.date(), weeks)),
            }
        }
        out
    }

    /// What `text` says in this pattern, when the whole of it reads
    /// (Moment's strict parse, case aside for names); `None` when it does
    /// not.
    #[must_use]
    pub fn parse(&self, text: &str) -> Option<Fields> {
        read(&self.parts, text, Fields::default())
    }
}

fn pad(n: i64, width: usize) -> String {
    if n < 0 {
        format!("-{:0width$}", n.unsigned_abs())
    } else {
        format!("{n:0width$}")
    }
}

/// Moment's `en` ordinal: 1st, 2nd, 3rd, 4th, 11th.
#[must_use]
pub fn ordinal(n: u32) -> String {
    let suffix = if (n % 100) / 10 == 1 {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

fn token(t: T, d: NaiveDate, weeks: WeekRules) -> String {
    let year = i64::from(d.year());
    let rules = |iso: bool| if iso { WeekRules::ISO } else { weeks };
    match t {
        T::Year(2) => pad(year.rem_euclid(100), 2),
        T::Year(0 | 4) => pad(year, 4),
        T::Year(n) => pad(year, usize::from(n)),
        T::WeekYear { iso, len } => {
            let y = i64::from(rules(iso).week_of(d).0);
            if len == 2 {
                pad(y.rem_euclid(100), 2)
            } else {
                pad(y, usize::from(len))
            }
        }
        T::Month(n) => pad(i64::from(d.month()), usize::from(n)),
        T::MonthName { short } => month_name(d.month0(), short).to_owned(),
        T::MonthOrd => ordinal(d.month()),
        T::Quarter { ord } => {
            let q = crate::period::quarter_of(d);
            if ord { ordinal(q) } else { q.to_string() }
        }
        T::Day(n) => pad(i64::from(d.day()), usize::from(n)),
        T::DayOrd => ordinal(d.day()),
        T::DayOfYear(n) => pad(i64::from(d.ordinal()), usize::from(n)),
        T::DayOfYearOrd => ordinal(d.ordinal()),
        T::Weekday => d.weekday().num_days_from_sunday().to_string(),
        T::WeekdayOrd => ordinal(d.weekday().num_days_from_sunday()),
        T::LocaleWeekday => {
            let from_sunday = d.weekday().num_days_from_sunday();
            (from_sunday
                .saturating_add(7)
                .saturating_sub(u32::from(weeks.dow) % 7)
                % 7)
            .to_string()
        }
        T::IsoWeekday => d.weekday().number_from_monday().to_string(),
        T::WeekdayName(n) => {
            let i = usize::try_from(d.weekday().num_days_from_sunday()).unwrap_or(0);
            match n {
                2 => WEEKDAYS_MIN.get(i).copied().unwrap_or("").to_owned(),
                3 => WEEKDAYS
                    .get(i)
                    .and_then(|w| w.get(..3))
                    .unwrap_or("")
                    .to_owned(),
                _ => WEEKDAYS.get(i).copied().unwrap_or("").to_owned(),
            }
        }
        T::Week { iso, len } => pad(i64::from(rules(iso).week_of(d).1), usize::from(len)),
        T::WeekOrd { iso } => ordinal(rules(iso).week_of(d).1),
        T::Time(t) => t.write(NaiveTime::MIN),
        T::Zone { colon } => zone(0, colon),
    }
}

/// An offset east of UTC as Moment's `Z` (`+01:00`) or `ZZ` (`+0100`).
fn zone(offset: i32, colon: bool) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let m = offset.unsigned_abs() / 60;
    let (h, m) = (m / 60, m % 60);
    if colon {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}")
    }
}

/// Digits at the start of `text`: at least `min`, at most `max`, longest
/// first; each with what is left.
fn digits(text: &str, min: usize, max: usize) -> Vec<(u32, &str)> {
    let run = text.bytes().take_while(u8::is_ascii_digit).count().min(max);
    (min..=run)
        .rev()
        .filter_map(|n| Some((text.get(..n)?.parse().ok()?, text.get(n..)?)))
        .collect()
}

/// An ordinal at the start of `text` (`5th`), and what is left.
fn ordinals(text: &str) -> Vec<(u32, &str)> {
    digits(text, 1, 3)
        .into_iter()
        .filter_map(|(n, rest)| {
            let suffix = ordinal(n);
            let suffix = suffix.get(n.to_string().len()..)?;
            rest.strip_prefix(suffix).map(|r| (n, r))
        })
        .collect()
}

/// A name at the start of `text`, any case: its index and what is left.
fn names<'a>(text: &'a str, list: &[String]) -> Vec<(u32, &'a str)> {
    list.iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let head = text.get(..name.len())?;
            head.eq_ignore_ascii_case(name)
                .then(|| Some((u32::try_from(i).ok()?, text.get(name.len()..)?)))
                .flatten()
        })
        .collect()
}

const fn set(slot: &mut Option<u32>, v: u32) -> bool {
    match *slot {
        Some(old) if old != v => false,
        _ => {
            *slot = Some(v);
            true
        }
    }
}

/// The parts read from `text`, trying every width a token may take.
fn read(parts: &[Part], text: &str, f: Fields) -> Option<Fields> {
    let Some((first, rest_parts)) = parts.split_first() else {
        return text.is_empty().then_some(f);
    };
    let t = match first {
        Part::Lit(s) => return read(rest_parts, text.strip_prefix(s.as_str())?, f),
        Part::Tok(t) => *t,
    };
    // Each way this token can read, with the fields it sets.
    let options: Vec<(u32, &str)> = match t {
        T::Year(0) => digits(text, 1, 4),
        T::Year(n) | T::WeekYear { len: n, .. } => digits(text, usize::from(n), usize::from(n)),
        T::Month(2) | T::Day(2) | T::Week { len: 2, .. } => digits(text, 2, 2),
        T::Month(_) | T::Day(_) | T::Week { .. } => digits(text, 1, 2),
        T::DayOfYear(3) => digits(text, 3, 3),
        T::DayOfYear(_) => digits(text, 1, 3),
        T::Quarter { ord: false } | T::Weekday | T::LocaleWeekday | T::IsoWeekday => {
            digits(text, 1, 1)
        }
        T::MonthOrd
        | T::DayOrd
        | T::DayOfYearOrd
        | T::WeekdayOrd
        | T::WeekOrd { .. }
        | T::Quarter { ord: true } => ordinals(text),
        T::MonthName { short } => {
            let list: Vec<String> = (0..12).map(|m| month_name(m, short).to_owned()).collect();
            names(text, &list)
        }
        T::WeekdayName(n) => {
            let list: Vec<String> = (0..7)
                .map(|i| {
                    let d = add_days(
                        NaiveDate::from_ymd_opt(2023, 1, 1).unwrap_or(NaiveDate::MIN),
                        i,
                    );
                    token(T::WeekdayName(n), d, WeekRules::SUNDAY)
                })
                .collect();
            names(text, &list)
        }
        T::Time(t) => text
            .strip_prefix(t.write(NaiveTime::MIN).as_str())
            .map(|r| vec![(0, r)])
            .unwrap_or_default(),
        T::Zone { .. } => Vec::new(),
    };
    options.into_iter().find_map(|(n, rest)| {
        let mut f = f;
        let ok = match t {
            T::Year(2) => {
                // Moment: 69–99 are the 1900s.
                let century = if n > 68 { 1900 } else { 2000 };
                let y = i32::try_from(n).ok()?.saturating_add(century);
                f.year.is_none_or(|old| old == y) && {
                    f.year = Some(y);
                    true
                }
            }
            T::Year(_) => {
                let y = i32::try_from(n).ok()?;
                f.year.is_none_or(|old| old == y) && {
                    f.year = Some(y);
                    true
                }
            }
            T::WeekYear { len, .. } => {
                let mut y = i32::try_from(n).ok()?;
                if len == 2 {
                    y = y.saturating_add(if n > 68 { 1900 } else { 2000 });
                }
                f.week_year.is_none_or(|old| old == y) && {
                    f.week_year = Some(y);
                    true
                }
            }
            T::Month(_) | T::MonthOrd => set(&mut f.month, n),
            T::MonthName { .. } => set(&mut f.month, n.saturating_add(1)),
            T::Quarter { .. } => (1..=4).contains(&n) && set(&mut f.quarter, n),
            T::Day(_) | T::DayOrd => set(&mut f.day, n),
            T::DayOfYear(_) | T::DayOfYearOrd => set(&mut f.day_of_year, n),
            T::Week { .. } | T::WeekOrd { .. } => set(&mut f.week, n),
            T::Weekday
            | T::WeekdayOrd
            | T::LocaleWeekday
            | T::IsoWeekday
            | T::WeekdayName(_)
            | T::Time(_)
            | T::Zone { .. } => true,
        };
        if ok { read(rest_parts, rest, f) } else { None }
    })
}

/// Obsidian's `normalizePath`, as a path in the vault: `\` to `/`, runs
/// of `/` as one, none at either end.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Windows' device names: no file may be called so (`CON.txt` neither).
const RESERVED: [&str; 23] = [
    "con", "prn", "aux", "nul", "clock$", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Whether a formatted note path is one a vault can hold
/// (`isRenderedCalendarNotePathValid`): no empty, hidden, dotted or
/// reserved part, nothing a file name may not have.
#[must_use]
pub fn is_valid_note_path(rendered: &str) -> bool {
    if rendered != rendered.trim() {
        return false;
    }
    let path = normalize_path(rendered);
    if path.is_empty() || path == "." {
        return false;
    }
    let ok = path.split('/').all(|seg| {
        let base = seg.trim_end_matches('.');
        let base = base.split('.').next().unwrap_or(base).to_ascii_lowercase();
        !seg.is_empty()
            && seg == seg.trim()
            && seg != "."
            && seg != ".."
            && !seg.starts_with('.')
            && !seg.ends_with('.')
            && !RESERVED.contains(&base.as_str())
            && !seg.contains([':', '/', '<', '>', '"', '\\', '|', '?', '*'])
    });
    ok && path
        .rsplit('/')
        .next()
        .is_some_and(|last| !last.trim_end_matches(".md").is_empty())
}
