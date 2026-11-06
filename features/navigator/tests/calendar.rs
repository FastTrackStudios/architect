//! The calendar and the periodic notes without a window: Moment patterns,
//! weeks, a period's path and a path's period, the grid, opening or
//! making a note, moving between periods, and the date filters a search
//! takes, each as Notebook Navigator does it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::Clock;
use architect_navigator::source::FileOp;
use architect_navigator::{
    DateField, DateFilter, DayMonthOrder, MemorySource, NavigatorSettings, NavigatorSource,
    NavigatorState, NoteFile, OpenIn, Outcome, Pattern, Period, PeriodKind, PeriodicNotes,
    TaskCounts, VaultIndex, WeekRules, WeekStart,
};
use chrono::NaiveDate;

/// 2026-11-05 12:00 UTC, a Thursday.
const NOW: i64 = 1_793_880_000_000;

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn fmt(pattern: &str, date: &str, weeks: WeekRules) -> String {
    Pattern::new(pattern).format(day(date), weeks)
}

/// The demo vault's layout (`apps/lab/demo-vault`): `Journal/Daily/…`.
fn journal_settings() -> NavigatorSettings {
    NavigatorSettings {
        periodic_folder: "Journal".to_owned(),
        daily_pattern: "[Daily]/YYYY-MM-DD".to_owned(),
        weekly_pattern: "[Weekly]/GGGG-[W]WW".to_owned(),
        monthly_pattern: "[Monthly]/YYYY-MM".to_owned(),
        quarterly_pattern: "[Quarterly]/YYYY-[Q]Q".to_owned(),
        yearly_pattern: "[Yearly]/YYYY".to_owned(),
        ..NavigatorSettings::default()
    }
}

fn journal() -> MemorySource {
    let mut v = MemorySource::new("Demo");
    for d in 1..=5 {
        let mut f = NoteFile::new(format!("Journal/Daily/2026-11-0{d}.md"), NOW, NOW);
        if d == 3 {
            f.feature_image = Some("https://example.com/sky.jpg".to_owned());
        }
        if d == 4 {
            f.tasks = Some(TaskCounts {
                total: 3,
                unfinished: 2,
            });
        }
        v = v.with_file(f);
    }
    v.with_file(NoteFile::new("Journal/Weekly/2026-W45.md", NOW, NOW))
        .with_file(NoteFile::new("Journal/Monthly/2026-11.md", NOW, NOW))
        .with_file(NoteFile::new("Journal/Quarterly/2026-Q4.md", NOW, NOW))
        .with_file(NoteFile::new("Notes/Plan.md", NOW, NOW))
}

fn state(source: &MemorySource, settings: NavigatorSettings) -> NavigatorState {
    NavigatorState::new(
        VaultIndex::read(source),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    )
}

// ── Patterns ────────────────────────────────────────────────────────────

#[test]
fn moment_tokens_format_as_moment_does() {
    let us = WeekRules::SUNDAY;
    assert_eq!(fmt("YYYY/YYYYMMDD", "2026-11-05", us), "2026/20261105");
    assert_eq!(fmt("YYYY-MM-DD", "2026-03-07", us), "2026-03-07");
    assert_eq!(fmt("YY M D", "2026-03-07", us), "26 3 7");
    assert_eq!(
        fmt("dddd, MMMM Do YYYY", "2026-11-05", us),
        "Thursday, November 5th 2026"
    );
    assert_eq!(fmt("ddd MMM dd", "2026-11-01", us), "Sun Nov Su");
    assert_eq!(fmt("YYYY-[Q]Q Qo", "2026-11-05", us), "2026-Q4 4th");
    assert_eq!(fmt("DDDD DDD DDDo", "2026-02-01", us), "032 32 32nd");
    assert_eq!(fmt("d e E", "2026-11-01", WeekRules::ISO), "0 6 7");
    // Literals, escapes, and a letter that is no token.
    assert_eq!(fmt("[Work]/YYYY", "2026-11-05", us), "Work/2026");
    assert_eq!(fmt("[[x]]-YYYY", "2026-11-05", us), "[x]-2026");
    assert_eq!(fmt("\\Y-YYYY", "2026-11-05", us), "Y-2026");
    // `a` is am or pm: an unbracketed word is not itself.
    assert_eq!(fmt("Journal", "2026-11-05", us), "Journaml");
    // Ordinals: 11th to 13th.
    assert_eq!(fmt("Do", "2026-11-11", us), "11th");
    assert_eq!(fmt("Do", "2026-11-22", us), "22nd");
    assert_eq!(fmt("Do", "2026-11-23", us), "23rd");
}

#[test]
fn locale_and_iso_weeks_number_as_moment() {
    // Moment's `en`: weeks from Sunday, week 1 holds January 1.
    let us = WeekRules::SUNDAY;
    assert_eq!(fmt("gggg-[W]ww", "2026-11-05", us), "2026-W45");
    assert_eq!(fmt("gggg-[W]ww", "2026-12-31", us), "2027-W01");
    assert_eq!(fmt("gggg-[W]ww", "2025-12-28", us), "2026-W01");
    // ISO: weeks from Monday, week 1 holds January 4.
    assert_eq!(fmt("GGGG-[W]WW", "2026-11-05", us), "2026-W45");
    assert_eq!(fmt("GGGG-[W]WW", "2027-01-01", us), "2026-W53");
    assert_eq!(fmt("GGGG-[W]WW", "2025-12-28", us), "2025-W52");
    assert_eq!(WeekRules::ISO.weeks_in_year(2026), 53);
    assert_eq!(WeekRules::ISO.weeks_in_year(2025), 52);
    assert_eq!(WeekRules::SUNDAY.week_start(day("2026-11-05")), day("2026-11-01"));
    assert_eq!(WeekRules::ISO.week_start(day("2026-11-05")), day("2026-11-02"));
    assert_eq!(
        WeekRules::SATURDAY.week_start(day("2026-11-05")),
        day("2026-10-31")
    );
}

#[test]
fn a_pattern_reads_back_what_it_wrote() {
    let p = Pattern::new("YYYY/YYYYMMDD");
    let f = p.parse("2026/20261105").unwrap();
    assert_eq!((f.year, f.month, f.day), (Some(2026), Some(11), Some(5)));
    assert!(p.parse("2026/202611").is_none());
    assert!(p.parse("2026/20261105x").is_none());
    // One-or-two-digit tokens try each width.
    let f = Pattern::new("YYYY-M-D").parse("2026-1-15").unwrap();
    assert_eq!((f.month, f.day), (Some(1), Some(15)));
    // Names, any case.
    let f = Pattern::new("MMMM YYYY").parse("november 2026").unwrap();
    assert_eq!(f.month, Some(11));
    // A year written twice must agree.
    assert!(Pattern::new("YYYY/YYYY-MM").parse("2026/2027-01").is_none());
}

// ── Periodic notes: paths ──────────────────────────────────────────────

#[test]
fn notebook_navigators_defaults_write_these_paths() {
    let p = PeriodicNotes::from_settings(&NavigatorSettings::default());
    let at = |kind| p.path(p.period(kind, day("2026-11-05"))).unwrap();
    assert_eq!(at(PeriodKind::Day), "2026/20261105.md");
    assert_eq!(at(PeriodKind::Week), "2026/W45.md");
    assert_eq!(at(PeriodKind::Month), "2026/202611.md");
    assert_eq!(at(PeriodKind::Quarter), "2026/Q4.md");
    assert_eq!(at(PeriodKind::Year), "2026.md");
    // A week's note is named for its first day's week.
    assert_eq!(
        p.period(PeriodKind::Week, day("2026-11-05")).start,
        day("2026-11-01")
    );
}

#[test]
fn a_root_folder_and_bracketed_folders_hold_the_notes() {
    let p = PeriodicNotes::from_settings(&journal_settings());
    let at = |kind| p.path(p.period(kind, day("2026-11-05"))).unwrap();
    assert_eq!(at(PeriodKind::Day), "Journal/Daily/2026-11-05.md");
    assert_eq!(at(PeriodKind::Week), "Journal/Weekly/2026-W45.md");
    assert_eq!(at(PeriodKind::Month), "Journal/Monthly/2026-11.md");
    assert_eq!(at(PeriodKind::Quarter), "Journal/Quarterly/2026-Q4.md");
    assert_eq!(at(PeriodKind::Year), "Journal/Yearly/2026.md");
    // A weekly pattern with ISO tokens makes the weeks ISO's: Monday on.
    assert_eq!(p.week_rules(), WeekRules::ISO);
    assert_eq!(
        p.period(PeriodKind::Week, day("2026-11-05")).start,
        day("2026-11-02")
    );
}

#[test]
fn a_path_reads_back_as_its_period() {
    for settings in [NavigatorSettings::default(), journal_settings()] {
        let p = PeriodicNotes::from_settings(&settings);
        for kind in PeriodKind::ALL {
            for date in ["2026-11-05", "2027-01-01", "2025-12-28", "2024-02-29"] {
                let period = p.period(kind, day(date));
                let path = p.path(period).unwrap();
                assert_eq!(p.period_of(&path), Some(period), "{path}");
            }
        }
        assert_eq!(p.period_of("Notes/Plan.md"), None);
        assert_eq!(p.period_of("2026/20261105.txt"), None);
    }
    // A strict parse alone accepts paths a pattern never writes.
    let p = PeriodicNotes::from_settings(&NavigatorSettings {
        daily_pattern: "YYYY-M-D".to_owned(),
        ..NavigatorSettings::default()
    });
    assert!(p.period_of("2026/2026-11-5.md").is_none());
    assert!(p.period_of("2026-11-05.md").is_none());
    assert_eq!(
        p.period_of("2026-11-5.md").map(|x| x.kind),
        Some(PeriodKind::Day)
    );
}

#[test]
fn empty_patterns_turn_a_kind_off_and_bad_ones_say_why() {
    let p = PeriodicNotes::from_settings(&NavigatorSettings {
        weekly_pattern: String::new(),
        quarterly_pattern: "YYYY".to_owned(),
        daily_pattern: String::new(),
        ..NavigatorSettings::default()
    });
    // An empty daily pattern is the default.
    assert!(p.enabled(PeriodKind::Day));
    assert!(!p.enabled(PeriodKind::Week));
    assert!(p.why_not(PeriodKind::Week).unwrap().contains("off"));
    assert!(!p.enabled(PeriodKind::Quarter));
    assert!(p.why_not(PeriodKind::Quarter).unwrap().contains("quarter"));
    // A parent skips a kind that is off: a day's is its month.
    let d = p.period(PeriodKind::Day, day("2026-11-05"));
    assert_eq!(p.parent(d).unwrap().kind, PeriodKind::Month);
    // A pattern that writes no file name is no pattern.
    let p = PeriodicNotes::from_settings(&NavigatorSettings {
        monthly_pattern: "YYYY/MM/.".to_owned(),
        yearly_pattern: "Daily".to_owned(),
        ..NavigatorSettings::default()
    });
    assert!(!p.enabled(PeriodKind::Month));
    // Unbracketed, `Daily` is Moment's `D`, `a` and `y` (a year): it
    // writes a path, if not the one meant.
    assert!(p.enabled(PeriodKind::Year));
}

// ── Periods ─────────────────────────────────────────────────────────────

#[test]
fn periods_step_and_climb() {
    let w = WeekRules::ISO;
    let d = Period::containing(PeriodKind::Day, day("2026-12-31"), w);
    assert_eq!(d.next().start, day("2027-01-01"));
    let m = Period::containing(PeriodKind::Month, day("2026-01-31"), w);
    assert_eq!(m.next().start, day("2026-02-01"));
    assert_eq!(m.end(), day("2026-01-31"));
    let q = Period::containing(PeriodKind::Quarter, day("2026-11-05"), w);
    assert_eq!((q.start, q.end()), (day("2026-10-01"), day("2026-12-31")));
    assert_eq!(q.previous().start, day("2026-07-01"));
    let week = Period::containing(PeriodKind::Week, day("2026-12-01"), w);
    assert_eq!(week.start, day("2026-11-30"));
    // A week's month is its first day's.
    assert_eq!(week.within(PeriodKind::Month, w).start, day("2026-11-01"));
    assert!(week.contains(day("2026-12-06")));
    assert!(!week.contains(day("2026-12-07")));
    assert_eq!(
        Period::containing(PeriodKind::Year, day("2026-11-05"), w).shift(-2).start,
        day("2024-01-01")
    );
}

// ── The grid ────────────────────────────────────────────────────────────

#[test]
fn the_grid_is_the_month_in_weeks() {
    let source = journal();
    let mut settings = journal_settings();
    settings.calendar_weeks = 6;
    settings.week_start = WeekStart::Sunday;
    settings.calendar_week_numbers = true;
    let mut s = state(&source, settings);
    s.set_active(Some("Journal/Daily/2026-11-05.md"));
    let g = s.calendar();
    // ISO weekly notes: the weeks start on Monday, whatever the setting.
    assert_eq!(g.weekdays, ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]);
    // November 2026: Monday October 26 to Sunday December 6.
    assert_eq!(g.weeks.len(), 6);
    assert_eq!(g.weeks[0].start, day("2026-10-26"));
    assert_eq!(g.weeks[1].number, 45);
    assert_eq!(
        g.weeks[1].note.as_deref(),
        Some("Journal/Weekly/2026-W45.md")
    );
    assert!(g.weeks[0].note.is_none());
    let days: Vec<_> = g.weeks.iter().flat_map(|w| &w.days).collect();
    let on = |d: &str| days.iter().find(|x| x.date == day(d)).unwrap();
    assert!(!on("2026-10-31").in_month);
    assert!(on("2026-10-31").weekend);
    assert!(on("2026-11-05").today);
    assert!(on("2026-11-05").active);
    assert!(!on("2026-11-04").active);
    assert_eq!(
        on("2026-11-01").note.as_deref(),
        Some("Journal/Daily/2026-11-01.md")
    );
    assert!(on("2026-11-06").note.is_none());
    assert!(on("2026-11-03").image.is_some());
    assert_eq!(on("2026-11-04").unfinished, 2);
    assert_eq!(g.month.label, "November");
    assert_eq!(g.month.note.as_deref(), Some("Journal/Monthly/2026-11.md"));
    assert_eq!(g.quarter.label, "Q4");
    assert!(g.quarter.note.is_some());
    assert_eq!(g.year.label, "2026");
    assert!(g.year.note.is_none());
}

#[test]
fn a_few_weeks_centre_on_the_cursor_and_page_by_their_count() {
    let source = journal();
    let mut settings = NavigatorSettings::default();
    settings.calendar_weeks = 3;
    let mut s = state(&source, settings);
    let g = s.calendar();
    // Sunday weeks: the cursor's week, one before, one after.
    assert_eq!(g.weeks.len(), 3);
    assert_eq!(g.weeks[0].start, day("2026-10-25"));
    assert_eq!(g.weekdays[0], "Su");
    s.calendar_move(1);
    assert_eq!(s.calendar_cursor(), day("2026-11-26"));
    s.calendar_move(-2);
    assert_eq!(s.calendar_cursor(), day("2026-10-15"));
    // A whole month pages by months, and hides other months' days when
    // asked.
    let mut settings = NavigatorSettings::default();
    settings.calendar_weeks = 6;
    settings.calendar_outside_days = false;
    let mut s = state(&source, settings);
    s.calendar_move(1);
    assert_eq!(s.calendar_cursor(), day("2026-12-05"));
    let g = s.calendar();
    assert!(g.weeks[0].days[0].hidden);
    assert_eq!(g.month.label, "December");
}

#[test]
fn the_calendar_follows_the_note_open() {
    let source = journal();
    let mut s = state(&source, journal_settings());
    s.set_active(Some("Journal/Daily/2026-11-05.md"));
    assert_eq!(s.calendar_cursor(), day("2026-11-05"));
    // A day out of the week on show: the calendar goes there.
    s.set_active(Some("Journal/Daily/2026-11-20.md"));
    assert_eq!(s.calendar_cursor(), day("2026-11-20"));
    // The month on show already: it stays.
    s.set_active(Some("Journal/Monthly/2026-11.md"));
    assert_eq!(s.calendar_cursor(), day("2026-11-20"));
    // A quarter's note does not move it.
    s.set_active(Some("Journal/Quarterly/2026-Q4.md"));
    assert_eq!(s.calendar_cursor(), day("2026-11-20"));
    // A month's goes to its month, though the whole of November shows
    // December 1.
    let mut settings = journal_settings();
    settings.calendar_weeks = 6;
    let mut s = state(&source, settings);
    s.set_active(Some("Journal/Monthly/2026-12.md"));
    assert_eq!(s.calendar_cursor(), day("2026-12-01"));
    assert_eq!(s.calendar().month.label, "December");
}

// ── Opening and making notes ────────────────────────────────────────────

#[test]
fn a_day_opens_its_note_or_makes_it() {
    let source = journal();
    let s = state(&source, journal_settings());
    let out = s
        .open_periodic(PeriodKind::Day, day("2026-11-05"), OpenIn::Here)
        .unwrap();
    assert_eq!(
        out,
        Outcome::Open {
            path: "Journal/Daily/2026-11-05.md".to_owned(),
            how: OpenIn::Here,
            settle: false
        }
    );
    let out = s
        .open_periodic(PeriodKind::Year, day("2026-11-05"), OpenIn::Tab)
        .unwrap();
    let Outcome::Write { ops, open } = out else {
        panic!("{out:?}")
    };
    assert_eq!(
        ops,
        [FileOp::CreateFile {
            path: "Journal/Yearly/2026.md".to_owned(),
            text: "# 2026\n".to_owned()
        }]
    );
    assert_eq!(open, Some(("Journal/Yearly/2026.md".to_owned(), OpenIn::Tab)));
    // The source makes it, and its folder.
    source.write(&ops).unwrap();
    assert!(source.folders().contains(&"Journal/Yearly".to_owned()));
    assert!(source.write(&ops).is_err());
}

#[test]
fn today_opens_todays_note_only_when_there_is_one() {
    let source = journal();
    let mut s = state(&source, journal_settings());
    s.calendar_move(3);
    assert!(matches!(s.calendar_today(), Some(Outcome::Open { .. })));
    assert_eq!(s.calendar_cursor(), day("2026-11-05"));
    let mut s = state(&source, NavigatorSettings::default());
    assert_eq!(s.calendar_today(), None);
}

#[test]
fn next_previous_and_parent_periods_move_from_the_note_open() {
    let source = journal();
    let mut s = state(&source, journal_settings());
    assert!(s.open_adjacent_period(1, OpenIn::Here).is_err());
    s.set_active(Some("Notes/Plan.md"));
    assert!(s.open_parent_period(OpenIn::Here).is_err());
    let path = |o: Outcome| match o {
        Outcome::Open { path, .. } => path,
        Outcome::Write {
            open: Some((path, _)),
            ..
        } => format!("new {path}"),
        other => panic!("{other:?}"),
    };
    s.set_active(Some("Journal/Daily/2026-11-05.md"));
    assert_eq!(
        path(s.open_adjacent_period(-1, OpenIn::Here).unwrap()),
        "Journal/Daily/2026-11-04.md"
    );
    assert_eq!(
        path(s.open_adjacent_period(1, OpenIn::Here).unwrap()),
        "new Journal/Daily/2026-11-06.md"
    );
    assert_eq!(
        path(s.open_parent_period(OpenIn::Here).unwrap()),
        "Journal/Weekly/2026-W45.md"
    );
    s.set_active(Some("Journal/Weekly/2026-W45.md"));
    assert_eq!(
        path(s.open_adjacent_period(1, OpenIn::Here).unwrap()),
        "new Journal/Weekly/2026-W46.md"
    );
    assert_eq!(
        path(s.open_parent_period(OpenIn::Here).unwrap()),
        "Journal/Monthly/2026-11.md"
    );
    s.set_active(Some("Journal/Monthly/2026-11.md"));
    assert_eq!(
        path(s.open_adjacent_period(-1, OpenIn::Here).unwrap()),
        "new Journal/Monthly/2026-10.md"
    );
    assert_eq!(
        path(s.open_parent_period(OpenIn::Here).unwrap()),
        "Journal/Quarterly/2026-Q4.md"
    );
    s.set_active(Some("Journal/Quarterly/2026-Q4.md"));
    assert_eq!(
        path(s.open_adjacent_period(1, OpenIn::Here).unwrap()),
        "new Journal/Quarterly/2027-Q1.md"
    );
    assert_eq!(
        path(s.open_parent_period(OpenIn::Here).unwrap()),
        "new Journal/Yearly/2026.md"
    );
    s.set_active(Some("Journal/Yearly/2026.md"));
    assert!(s.open_parent_period(OpenIn::Here).is_err());
}

#[test]
fn the_calendar_is_kept_between_runs() {
    let source = journal();
    let mut s = state(&source, NavigatorSettings::default());
    assert!(!s.calendar_visible());
    s.toggle_calendar();
    assert!(s.calendar_visible());
    let kept = s.view_state();
    let mut t = state(&source, NavigatorSettings::default());
    t.restore(&kept);
    assert!(t.calendar_open);
    t.set_settings(NavigatorSettings {
        calendar: false,
        ..NavigatorSettings::default()
    });
    assert!(!t.calendar_visible());
}

// ── Date filters ────────────────────────────────────────────────────────

fn filter(token: &str) -> Option<(Option<NaiveDate>, Option<NaiveDate>)> {
    DateFilter::parse(token, day("2026-11-05"), DayMonthOrder::Dmy).map(|f| (f.start, f.end))
}

#[test]
fn date_filters_read_as_notebook_navigators() {
    let range = |a: &str, b: &str| Some((Some(day(a)), Some(day(b))));
    assert_eq!(filter("@2026-11-05"), range("2026-11-05", "2026-11-06"));
    assert_eq!(filter("@20261105"), range("2026-11-05", "2026-11-06"));
    assert_eq!(filter("@2026-11"), range("2026-11-01", "2026-12-01"));
    assert_eq!(filter("@2026/11"), range("2026-11-01", "2026-12-01"));
    assert_eq!(filter("@202611"), range("2026-11-01", "2026-12-01"));
    assert_eq!(filter("@2026"), range("2026-01-01", "2027-01-01"));
    assert_eq!(filter("@2026-Q4"), range("2026-10-01", "2027-01-01"));
    assert_eq!(filter("@2026q1"), range("2026-01-01", "2026-04-01"));
    assert_eq!(filter("@2026-W45"), range("2026-11-02", "2026-11-09"));
    assert_eq!(filter("@2026-w54"), None);
    assert_eq!(
        filter("@2026-11-01..2026-11-07"),
        range("2026-11-01", "2026-11-08")
    );
    assert_eq!(filter("@..2026"), Some((None, Some(day("2027-01-01")))));
    assert_eq!(filter("@2026-11.."), Some((Some(day("2026-11-01")), None)));
    assert_eq!(filter("@2026-11-07..2026-11-01"), None);
    assert_eq!(filter("@today"), range("2026-11-05", "2026-11-06"));
    assert_eq!(filter("@yesterday"), range("2026-11-04", "2026-11-05"));
    assert_eq!(filter("@last7d"), range("2026-10-30", "2026-11-06"));
    assert_eq!(filter("@thisweek"), range("2026-11-02", "2026-11-09"));
    assert_eq!(filter("@thismonth"), range("2026-11-01", "2026-12-01"));
    assert_eq!(filter("@2026-02-30"), None);
    assert_eq!(filter("@soon"), None);
    assert_eq!(filter("2026"), None);
    // Day and month either way: the one that is a date, else the locale's.
    assert_eq!(filter("@31/12/2026"), range("2026-12-31", "2027-01-01"));
    assert_eq!(filter("@05.11.2026"), range("2026-11-05", "2026-11-06"));
    let mdy = DateFilter::parse("@05/11/2026", day("2026-11-05"), DayMonthOrder::Mdy).unwrap();
    assert_eq!(mdy.start, Some(day("2026-05-11")));
    // Which date.
    let c = DateFilter::parse("@c:2026-11", day("2026-11-05"), DayMonthOrder::Dmy).unwrap();
    assert_eq!(c.field, DateField::Created);
    let m = DateFilter::parse("@modified:today", day("2026-11-05"), DayMonthOrder::Dmy).unwrap();
    assert_eq!(m.field, DateField::Modified);
    assert!(DateFilter::is_candidate("@"));
    assert!(DateFilter::is_candidate("@2"));
    assert!(DateFilter::is_candidate("@c:yes"));
    assert!(!DateFilter::is_candidate("@soon"));
}

#[test]
fn a_periods_filter_token_reads_back_as_its_days() {
    let today = day("2026-11-05");
    for kind in PeriodKind::ALL {
        let p = Period::containing(kind, today, WeekRules::SUNDAY);
        let token = p.filter_token();
        let parsed = DateFilter::parse(&token, today, DayMonthOrder::Dmy).unwrap();
        assert_eq!(parsed, p.range(), "{token}");
        assert!(parsed.contains(p.start) && parsed.contains(p.end()));
        assert!(!parsed.contains(p.next().start));
    }
    let week = Period::containing(PeriodKind::Week, today, WeekRules::SUNDAY);
    assert_eq!(week.filter_token(), "@2026-11-01..2026-11-07");
    assert_eq!(
        Period::containing(PeriodKind::Quarter, today, WeekRules::SUNDAY).filter_token(),
        "@2026-Q4"
    );
    // A timestamp's day, in the person's time zone.
    let nov5 = DateFilter::parse("@2026-11-05", today, DayMonthOrder::Dmy).unwrap();
    assert!(nov5.matches(NOW, 0));
    // 12:00 UTC is 01:00 the next day at UTC+13.
    assert!(!nov5.matches(NOW, 13 * 3_600_000));
}
