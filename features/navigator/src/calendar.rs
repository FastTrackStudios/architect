//! The calendar and the periodic notes.
//!
//! Notebook Navigator's calendar (`src/components/calendar/`): a month of
//! days, or a few weeks of it, under the folders. A day with a daily note
//! has a dot, or the note's feature image; today is lit; the note open is
//! ringed. A click on a day opens its daily note, or makes it; on a week's
//! number, the weekly note; on the month, the quarter or the year in the
//! header, theirs. Its periodic notes (`src/utils/calendarNotes.ts`) are
//! paths made from Moment patterns ([`crate::pattern`]) under one folder:
//! [`PeriodicNotes`] writes a period's path and reads a path's period.
//! [`grid`] is what the calendar draws.

use chrono::{Datelike, NaiveDate};

use crate::index::VaultIndex;
use crate::list::{self, Thumb};
use crate::pattern::{self, Pattern, WEEKDAYS_MIN, normalize_path};
use crate::period::{Period, PeriodKind, WeekRules, add_days, add_months};
use crate::settings::{MonthFormat, NavigatorSettings, WeekStart, WeekendDays};

/// Notebook Navigator's default patterns
/// (`DEFAULT_CALENDAR_CUSTOM_*_PATTERN`).
pub const DEFAULT_DAILY: &str = "YYYY/YYYYMMDD";
pub const DEFAULT_WEEKLY: &str = "gggg/[W]ww";
pub const DEFAULT_MONTHLY: &str = "YYYY/YYYYMM";
pub const DEFAULT_QUARTERLY: &str = "YYYY/[Q]Q";
pub const DEFAULT_YEARLY: &str = "YYYY";

/// One kind's pattern, split where Notebook Navigator splits it: the
/// folders, and the file's name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KindPattern {
    folder: Pattern,
    file: Pattern,
    /// Folder and file as one, to read a path with.
    whole: Pattern,
}

/// Where each kind of periodic note lives, and how its name is written:
/// the settings' root folder and patterns, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicNotes {
    root: String,
    /// By kind ([`PeriodKind::ALL`]'s order): `Ok` when on, `Err` with why
    /// when its pattern cannot make a note's path. A kind left empty is
    /// off (`Err` of "").
    kinds: [Result<KindPattern, String>; 5],
    /// The locale's weeks (`week_start`), for `w`, `g` and `e`.
    weeks: WeekRules,
}

impl PeriodicNotes {
    #[must_use]
    pub fn from_settings(s: &NavigatorSettings) -> Self {
        let weeks = s.week_start.rules();
        let read = |kind: PeriodKind, text: &str| make(kind, text, weeks);
        Self {
            root: normalize_path(s.periodic_folder.trim()),
            kinds: [
                // A daily pattern left empty is the default; the others,
                // off.
                read(
                    PeriodKind::Day,
                    if s.daily_pattern.trim().is_empty() {
                        DEFAULT_DAILY
                    } else {
                        &s.daily_pattern
                    },
                ),
                read(PeriodKind::Week, &s.weekly_pattern),
                read(PeriodKind::Month, &s.monthly_pattern),
                read(PeriodKind::Quarter, &s.quarterly_pattern),
                read(PeriodKind::Year, &s.yearly_pattern),
            ],
            weeks,
        }
    }

    fn kind(&self, kind: PeriodKind) -> &Result<KindPattern, String> {
        let i = PeriodKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
        &self.kinds[i]
    }

    /// Whether notes of `kind` are on, with a pattern that makes paths.
    #[must_use]
    pub fn enabled(&self, kind: PeriodKind) -> bool {
        self.kind(kind).is_ok()
    }

    /// Why notes of `kind` cannot be made: off, or its pattern's fault.
    #[must_use]
    pub fn why_not(&self, kind: PeriodKind) -> Option<String> {
        match self.kind(kind) {
            Ok(_) => None,
            Err(e) if e.is_empty() => Some(format!(
                "{} notes are off: give them a pattern in Navigator's settings",
                capital(kind.adjective())
            )),
            Err(e) => Some(e.clone()),
        }
    }

    /// The weeks a weekly note covers, and the calendar shows: ISO weeks
    /// when the weekly pattern uses `W` or `G`, else the locale's.
    #[must_use]
    pub fn week_rules(&self) -> WeekRules {
        match self.kind(PeriodKind::Week) {
            Ok(k) if k.whole.uses_iso_weeks() => WeekRules::ISO,
            _ => self.weeks,
        }
    }

    /// The period of `kind` holding `date`.
    #[must_use]
    pub fn period(&self, kind: PeriodKind, date: NaiveDate) -> Period {
        Period::containing(kind, date, self.week_rules())
    }

    /// The vault path of `period`'s note (it may not be there yet); `None`
    /// when its kind is off.
    #[must_use]
    pub fn path(&self, period: Period) -> Option<String> {
        let k = self.kind(period.kind).as_ref().ok()?;
        Some(self.path_with(k, period.start))
    }

    fn path_with(&self, k: &KindPattern, date: NaiveDate) -> String {
        let folder = normalize_path(&k.folder.format(date, self.weeks));
        let name = ensure_md(k.file.format(date, self.weeks).trim());
        [self.root.as_str(), folder.as_str(), name.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The period of the note at `path`: the first kind, day to year,
    /// whose pattern writes exactly that path
    /// (`parseCalendarNoteDateFromPath`).
    #[must_use]
    pub fn period_of(&self, path: &str) -> Option<Period> {
        PeriodKind::ALL
            .into_iter()
            .find_map(|kind| self.period_of_kind(kind, path))
    }

    /// The period of `kind` whose note is at `path`.
    #[must_use]
    pub fn period_of_kind(&self, kind: PeriodKind, path: &str) -> Option<Period> {
        let k = self.kind(kind).as_ref().ok()?;
        let path = normalize_path(path);
        let stem = path.strip_suffix(".md")?;
        let inner = if self.root.is_empty() {
            stem
        } else {
            stem.strip_prefix(self.root.as_str())?.strip_prefix('/')?
        };
        let fields = k.whole.parse(inner)?;
        let year = fields.any_year()?;
        let is = |p: Period| self.path_with(k, p.start) == path;
        let found = match kind {
            PeriodKind::Day => fields
                .month
                .zip(fields.day)
                .and_then(|(m, d)| NaiveDate::from_ymd_opt(year, m, d))
                .or_else(|| NaiveDate::from_yo_opt(year, fields.day_of_year?))
                .map(|d| Period::containing(kind, d, self.week_rules())),
            PeriodKind::Month => fields
                .month
                .and_then(|m| NaiveDate::from_ymd_opt(year, m, 1))
                .map(|d| Period::containing(kind, d, self.week_rules())),
            PeriodKind::Quarter => fields
                .quarter
                .map(|q| (q - 1) * 3 + 1)
                .or(fields.month)
                .and_then(|m| NaiveDate::from_ymd_opt(year, m, 1))
                .map(|d| Period::containing(kind, d, self.week_rules())),
            PeriodKind::Year => NaiveDate::from_ymd_opt(year, 1, 1)
                .map(|d| Period::containing(kind, d, self.week_rules())),
            // A week's number does not read back (`findWeeklyCalendarNoteDate`):
            // the weeks from December before to the year's end are tried,
            // the first that writes the path wins.
            PeriodKind::Week => {
                let from = NaiveDate::from_ymd_opt(year - 1, 12, 1)?;
                let mut p = self.period(PeriodKind::Week, from);
                let mut found = None;
                while p.start.year() <= year {
                    if is(p) {
                        found = Some(p);
                        break;
                    }
                    p = p.next();
                }
                found
            }
        }?;
        is(found).then_some(found)
    }

    /// The next longer period that has notes (`Open parent periodic note`):
    /// a day's week, or its month when weekly notes are off.
    #[must_use]
    pub fn parent(&self, period: Period) -> Option<Period> {
        let kind = period
            .kind
            .longer()
            .iter()
            .copied()
            .find(|k| self.enabled(*k))?;
        Some(period.within(kind, self.week_rules()))
    }

    /// What a new note of `period` holds until templates come (#339): a
    /// heading, its name.
    #[must_use]
    pub fn new_note_text(&self, path: &str) -> String {
        let name = path.rsplit('/').next().unwrap_or(path);
        format!("# {}\n", name.strip_suffix(".md").unwrap_or(name))
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

/// `name.md`, unless it ends so already.
fn ensure_md(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".md") {
        let stem = name.get(..name.len() - 3).unwrap_or(name);
        format!("{stem}.md")
    } else {
        format!("{name}.md")
    }
}

/// `text` as `kind`'s pattern (`splitCalendarCustomPattern`,
/// `isCalendarCustom*PatternValid`), or why it cannot be one.
fn make(kind: PeriodKind, text: &str, weeks: WeekRules) -> Result<KindPattern, String> {
    // `{title}` was once a token; a note's path is its date alone.
    let text = text.replace(" {title}", "").replace("{title}", "");
    let text = text.trim();
    let text = text.strip_suffix(".md").unwrap_or(text);
    let text = normalize_path(text);
    if text.is_empty() || text == "." {
        return Err(String::new());
    }
    let (folder, file) = text
        .rsplit_once('/')
        .map_or(("", text.as_str()), |(f, n)| (f, n));
    let file = file.trim();
    if file.is_empty() {
        return Err(String::new());
    }
    let whole_text = if folder.is_empty() {
        file.to_owned()
    } else {
        format!("{folder}/{file}")
    };
    let whole = Pattern::new(&whole_text);
    let (has, samples, error): (bool, &[(i32, u32, u32)], &str) = match kind {
        PeriodKind::Day => (
            whole.has_year() && whole.has_month() && whole.has_day(),
            &[(2026, 1, 16), (2027, 2, 17)],
            "The daily pattern must make a full date (year, month, day)",
        ),
        PeriodKind::Week => (
            whole.has_year() && whole.has_week(),
            &[(2020, 12, 31), (2021, 1, 1), (2026, 6, 19), (2027, 2, 17)],
            "The weekly pattern must make a full week (week year, week number)",
        ),
        PeriodKind::Month => (
            whole.has_year() && whole.has_month(),
            &[(2026, 1, 16), (2027, 2, 17)],
            "The monthly pattern must make a full month (year, month)",
        ),
        PeriodKind::Quarter => (
            whole.has_year() && whole.has_quarter(),
            &[(2026, 1, 16), (2026, 7, 15), (2027, 4, 3)],
            "The quarterly pattern must make a full quarter (year, quarter)",
        ),
        PeriodKind::Year => (
            whole.has_year(),
            &[(2026, 1, 16), (2027, 2, 17)],
            "The yearly pattern must make a full year (year)",
        ),
    };
    let rules = if whole.uses_iso_weeks() {
        WeekRules::ISO
    } else {
        weeks
    };
    let formats = samples.iter().all(|&(y, m, d)| {
        NaiveDate::from_ymd_opt(y, m, d).is_some_and(|date| {
            let date = if kind == PeriodKind::Week {
                rules.week_start(date)
            } else {
                date
            };
            pattern::is_valid_note_path(&whole.format(date, weeks))
        })
    });
    if !has || !formats {
        return Err(error.to_owned());
    }
    Ok(KindPattern {
        folder: Pattern::new(folder),
        file: Pattern::new(file),
        whole,
    })
}

/// A weekly pattern's quirks (`getCalendarCustomWeekPatternIssues`): why
/// its paths may not match the weeks the calendar shows. Empty when they
/// do.
#[must_use]
pub fn week_pattern_issues(text: &str, weeks: WeekRules) -> Vec<&'static str> {
    let p = Pattern::new(text);
    let mut out = Vec::new();
    if p.uses_iso_weeks() && weeks != WeekRules::ISO {
        out.push(
            "Weekly notes use ISO weeks (\"W\" or \"G\"), so the calendar starts weeks on Monday and shows ISO week numbers.",
        );
    }
    if p.has_week() && !p.has_week_year() {
        out.push(
            "Weekly notes take the year from \"YYYY\", so a week 1 that starts in December gets the previous year. Use \"GGGG\" with \"WW\", or \"gggg\" with \"ww\".",
        );
    }
    if p.uses_iso_weeks() && p.uses_locale_weeks() {
        out.push(
            "Weekly notes mix ISO week tokens (\"W\" or \"G\") with locale week tokens (\"w\" or \"g\").",
        );
    }
    out
}

/// One day on the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarDay {
    pub date: NaiveDate,
    /// In the month the calendar is on.
    pub in_month: bool,
    pub today: bool,
    pub weekend: bool,
    /// Its daily note, when there is one.
    pub note: Option<String>,
    /// Its note's feature image, when it has one and images are on.
    pub image: Option<Thumb>,
    /// Its note's tasks left, when tasks show.
    pub unfinished: u32,
    /// Its note is the one open.
    pub active: bool,
    /// A spacer: a day of another month, with those hidden.
    pub hidden: bool,
}

/// A row of the calendar: a week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarWeek {
    pub start: NaiveDate,
    /// Its number, ISO or the locale's as the weekly pattern is.
    pub number: u32,
    /// Its weekly note, when there is one.
    pub note: Option<String>,
    pub unfinished: u32,
    pub active: bool,
    pub days: Vec<CalendarDay>,
}

/// A month, quarter or year in the calendar's header, and its note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPeriod {
    pub period: Period,
    pub label: String,
    pub note: Option<String>,
    pub active: bool,
    /// Notes of its kind are on.
    pub enabled: bool,
}

/// What the calendar draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarGrid {
    /// The day it is around.
    pub cursor: NaiveDate,
    pub month: HeaderPeriod,
    pub quarter: HeaderPeriod,
    pub year: HeaderPeriod,
    /// The weekdays' heads, from the week's first day: `Su`, `Mo`, …
    pub weekdays: Vec<&'static str>,
    pub weeks: Vec<CalendarWeek>,
    /// Weekly notes are on: a week's number opens its note.
    pub week_notes: bool,
}

/// What the calendar needs to know besides the settings.
#[derive(Debug, Clone, Copy)]
pub struct CalendarContext<'a> {
    pub index: &'a VaultIndex,
    pub periodic: &'a PeriodicNotes,
    pub today: NaiveDate,
    pub active: Option<&'a str>,
}

impl WeekStart {
    /// Its week rules.
    #[must_use]
    pub const fn rules(self) -> WeekRules {
        match self {
            Self::Sunday => WeekRules::SUNDAY,
            Self::Monday => WeekRules::ISO,
            Self::Saturday => WeekRules::SATURDAY,
        }
    }
}

impl WeekendDays {
    /// Whether `day` (0 Sunday to 6) is a weekend's (`isWeekendDay`).
    #[must_use]
    pub const fn has(self, day: u32) -> bool {
        match self {
            Self::None => false,
            Self::SatSun => day == 0 || day == 6,
            Self::FriSat => day == 5 || day == 6,
            Self::ThuFri => day == 4 || day == 5,
        }
    }
}

/// The weeks the calendar shows around `cursor`
/// (`resolveCalendarWeekWindow`): the first day, and how many. Six is the
/// whole month, in as many rows as it takes; fewer are centred on the
/// cursor's week.
#[must_use]
pub fn week_window(cursor: NaiveDate, weeks_to_show: u32, rules: WeekRules) -> (NaiveDate, u32) {
    let n = weeks_to_show.clamp(1, 6);
    if n == 6 {
        let first = cursor.with_day(1).unwrap_or(cursor);
        let start = rules.week_start(first);
        let last = add_days(add_months(first, 1), -1);
        let rows = (rules.week_start(last) - start).num_weeks() + 1;
        return (start, u32::try_from(rows.clamp(1, 6)).unwrap_or(6));
    }
    let back = i64::from((n - 1) / 2) * 7;
    (add_days(rules.week_start(cursor), -back), n)
}

/// The day `cursor` moves to by `delta` (`handleNavigate`): a month at a
/// time on a whole month, else as many weeks as show.
#[must_use]
pub fn step(cursor: NaiveDate, delta: i32, weeks_to_show: u32) -> NaiveDate {
    let n = weeks_to_show.clamp(1, 6);
    if n == 6 {
        add_months(cursor, delta)
    } else {
        add_days(cursor, i64::from(delta) * 7 * i64::from(n))
    }
}

/// The note at `path`, if the vault has it.
fn existing(index: &VaultIndex, path: Option<String>) -> Option<String> {
    path.filter(|p| index.file(p).is_some())
}

fn unfinished(index: &VaultIndex, path: Option<&str>, show: bool) -> u32 {
    if !show {
        return 0;
    }
    path.and_then(|p| index.file(p))
        .and_then(|f| f.tasks)
        .map_or(0, |t| t.unfinished)
}

/// The calendar around `cursor`.
#[must_use]
pub fn grid(cx: CalendarContext<'_>, s: &NavigatorSettings, cursor: NaiveDate) -> CalendarGrid {
    let rules = cx.periodic.week_rules();
    let n = s.calendar_weeks.clamp(1, 6);
    let (start, rows) = week_window(cursor, n, rules);
    let hide_outside = !s.calendar_outside_days && n == 6;
    // The feature images, as the list finds them, in any row mode.
    let mut image_settings = s.clone();
    image_settings.show_feature_image = s.calendar_feature_images;
    image_settings.mode = crate::settings::ListMode::Standard;
    let note_of = |kind: PeriodKind, date: NaiveDate| {
        existing(cx.index, cx.periodic.path(cx.periodic.period(kind, date)))
    };
    let is_active = |note: &Option<String>| note.is_some() && note.as_deref() == cx.active;
    let weeks = (0..rows)
        .map(|w| {
            let week_start = add_days(start, i64::from(w) * 7);
            let days = (0..7)
                .map(|i| {
                    let date = add_days(week_start, i);
                    let in_month = date.month() == cursor.month() && date.year() == cursor.year();
                    let hidden = hide_outside && !in_month;
                    let note = if hidden {
                        None
                    } else {
                        note_of(PeriodKind::Day, date)
                    };
                    let image = note
                        .as_deref()
                        .and_then(|p| cx.index.file(p))
                        .and_then(|f| list::thumb(cx.index, f, &image_settings))
                        .filter(|t| !matches!(t, Thumb::Badge(_)));
                    CalendarDay {
                        date,
                        in_month,
                        today: date == cx.today,
                        weekend: s
                            .weekend_days
                            .has(date.weekday().num_days_from_sunday()),
                        unfinished: unfinished(cx.index, note.as_deref(), s.calendar_tasks),
                        active: is_active(&note),
                        note,
                        image,
                        hidden,
                    }
                })
                .collect();
            let note = note_of(PeriodKind::Week, week_start);
            CalendarWeek {
                start: week_start,
                number: rules.week_of(week_start).1,
                unfinished: unfinished(cx.index, note.as_deref(), s.calendar_tasks),
                active: is_active(&note),
                note,
                days,
            }
        })
        .collect();
    let header = |kind: PeriodKind, label: String| {
        let period = cx.periodic.period(kind, cursor);
        let note = existing(cx.index, cx.periodic.path(period));
        HeaderPeriod {
            period,
            label,
            active: is_active(&note),
            note,
            enabled: cx.periodic.enabled(kind),
        }
    };
    let first = usize::from(rules.dow % 7);
    CalendarGrid {
        cursor,
        month: header(
            PeriodKind::Month,
            pattern::month_name(cursor.month0(), s.month_format == MonthFormat::Short).to_owned(),
        ),
        quarter: header(
            PeriodKind::Quarter,
            format!("Q{}", cursor.month0() / 3 + 1),
        ),
        year: header(PeriodKind::Year, cursor.year().to_string()),
        weekdays: (0..7)
            .filter_map(|i| WEEKDAYS_MIN.get((first + i) % 7).copied())
            .collect(),
        weeks,
        week_notes: cx.periodic.enabled(PeriodKind::Week),
    }
}

