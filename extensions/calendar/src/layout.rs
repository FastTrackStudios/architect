//! Where blocks go: columns for those that overlap in a day, tracks for
//! those that run across days, and the days each view shows (Task's
//! `task:features/view/view-calendar/src/layout.rs` and `time.rs`).

use chrono::{Datelike as _, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use crate::model::Block;

/// What the calendar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    Day,
    #[default]
    Week,
    Month,
    /// The days ahead as a list.
    Agenda,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Agenda];

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Agenda => "agenda",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Agenda => "Agenda",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(id))
    }
}

/// How many days the agenda lists.
pub const AGENDA_DAYS: i64 = 30;

/// The first day of `day`'s week.
#[must_use]
pub fn week_start(day: NaiveDate, first: Weekday) -> NaiveDate {
    let back = (7 + day.weekday().num_days_from_monday() - first.num_days_from_monday()) % 7;
    day - Duration::days(i64::from(back))
}

/// The days `mode` shows around `cursor`: a day, a week, the month's six
/// weeks (Task's month grid is always 6 × 7), the agenda's days.
#[must_use]
pub fn days(mode: Mode, cursor: NaiveDate, first: Weekday) -> Vec<NaiveDate> {
    let (start, n) = match mode {
        Mode::Day => (cursor, 1),
        Mode::Week => (week_start(cursor, first), 7),
        Mode::Month => {
            let first_of = cursor.with_day(1).unwrap_or(cursor);
            (week_start(first_of, first), 42)
        }
        Mode::Agenda => (cursor, AGENDA_DAYS),
    };
    (0..n).map(|i| start + Duration::days(i)).collect()
}

/// Where `cursor` goes on ‹ or › (`by` −1 or 1).
#[must_use]
pub fn step(mode: Mode, cursor: NaiveDate, by: i32) -> NaiveDate {
    match mode {
        Mode::Day => cursor + Duration::days(i64::from(by)),
        Mode::Week => cursor + Duration::weeks(i64::from(by)),
        Mode::Agenda => cursor + Duration::days(AGENDA_DAYS * i64::from(by)),
        Mode::Month => {
            let total = cursor.year() * 12 + i32::try_from(cursor.month0()).unwrap_or(0) + by;
            let (y, m) = (
                total.div_euclid(12),
                u32::try_from(total.rem_euclid(12)).unwrap_or(0) + 1,
            );
            NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(cursor)
        }
    }
}

/// The toolbar's words for what shows: `November 2026`, `Nov 2 – 8, 2026`.
#[must_use]
pub fn range_label(mode: Mode, cursor: NaiveDate, first: Weekday) -> String {
    let d = days(mode, cursor, first);
    let (Some(a), Some(b)) = (d.first().copied(), d.last().copied()) else {
        return String::new();
    };
    match mode {
        Mode::Day => cursor.format("%A, %B %-d, %Y").to_string(),
        Mode::Month => cursor.format("%B %Y").to_string(),
        Mode::Week | Mode::Agenda if a.month() == b.month() => {
            format!("{} – {}", a.format("%b %-d"), b.format("%-d, %Y"))
        }
        Mode::Week | Mode::Agenda if a.year() == b.year() => {
            format!("{} – {}", a.format("%b %-d"), b.format("%b %-d, %Y"))
        }
        Mode::Week | Mode::Agenda => {
            format!("{} – {}", a.format("%b %-d, %Y"), b.format("%b %-d, %Y"))
        }
    }
}

/// Midnight starting `day`.
#[must_use]
pub const fn midnight(day: NaiveDate) -> NaiveDateTime {
    day.and_time(NaiveTime::MIN)
}

/// One block placed in a day's column: its minutes from midnight (clipped
/// to the day), and its column of how many.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: usize,
    pub top: i64,
    pub height: i64,
    pub column: usize,
    pub columns: usize,
}

/// The shortest a block is drawn, in minutes (Task's 15).
pub const MIN_HEIGHT: i64 = 15;

/// Day `day`'s timed blocks in columns (Task's `day_overlap_layout`).
///
/// By start, the longer first; one joins the cluster while it starts before
/// the cluster's last end, and takes the first column free by then; when
/// a cluster closes its members split its width.
#[must_use]
pub fn day_columns(day: NaiveDate, blocks: &[Block]) -> Vec<Placed> {
    let from = midnight(day);
    let to = from + Duration::days(1);
    let mut items: Vec<(usize, NaiveDateTime, NaiveDateTime)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.in_strip() && b.overlaps(from, to))
        .map(|(i, b)| (i, b.start.max(from), b.end.min(to)))
        .collect();
    items.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| (b.2 - b.1).cmp(&(a.2 - a.1))));
    let mut out = Vec::new();
    let mut cluster: Vec<Placed> = Vec::new();
    let mut cluster_end = from;
    // Each column's last end.
    let mut ends: Vec<NaiveDateTime> = Vec::new();
    let close =
        |cluster: &mut Vec<Placed>, out: &mut Vec<Placed>, ends: &mut Vec<NaiveDateTime>| {
            let n = cluster.iter().map(|p| p.column + 1).max().unwrap_or(1);
            for mut p in cluster.drain(..) {
                p.columns = n;
                out.push(p);
            }
            ends.clear();
        };
    for (index, s, e) in items {
        // Drawn at least MIN_HEIGHT long, so it is counted so too.
        let e = e.max(s + Duration::minutes(MIN_HEIGHT));
        if !cluster.is_empty() && s >= cluster_end {
            close(&mut cluster, &mut out, &mut ends);
        }
        let column = ends.iter().position(|end| *end <= s).unwrap_or(ends.len());
        if column == ends.len() {
            ends.push(e);
        } else if let Some(slot) = ends.get_mut(column) {
            *slot = e;
        }
        cluster_end = if cluster.is_empty() {
            e
        } else {
            cluster_end.max(e)
        };
        cluster.push(Placed {
            index,
            top: (s - from).num_minutes(),
            height: (e - s).num_minutes(),
            column,
            columns: 1,
        });
    }
    close(&mut cluster, &mut out, &mut ends);
    out
}

/// A block across a row of days (a month's week, the all-day strip):
/// its first and last columns, its track, and whether it runs on past
/// either end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub index: usize,
    pub first: usize,
    pub last: usize,
    pub track: usize,
    pub continues_left: bool,
    pub continues_right: bool,
}

/// The blocks over the `len` days from `start` in tracks (Task's
/// `track_layout`).
///
/// By first day, the longer first, each in the first
/// track free across its days. `all` puts timed blocks in too (a month's
/// rows); else only the strip's.
#[must_use]
pub fn tracks(start: NaiveDate, len: usize, blocks: &[Block], all: bool) -> Vec<Track> {
    let from = midnight(start);
    let to = from + Duration::days(i64::try_from(len).unwrap_or(0));
    let col = |t: NaiveDateTime| {
        usize::try_from((t.date() - start).num_days())
            .unwrap_or(0)
            .min(len.saturating_sub(1))
    };
    let mut items: Vec<(usize, usize, usize, bool, bool)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| (all || b.in_strip()) && b.overlaps(from, to))
        .map(|(i, b)| {
            // The day an exclusive end falls on is its last only past midnight.
            let last_moment = if b.end > b.start {
                b.end - Duration::seconds(1)
            } else {
                b.end
            };
            (
                i,
                col(b.start.max(from)),
                col(last_moment.min(to - Duration::seconds(1))),
                b.start < from,
                b.end > to,
            )
        })
        .collect();
    items.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| (b.2 - b.1).cmp(&(a.2 - a.1))));
    // Each track's taken columns.
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::new();
    for (index, first, last, left, right) in items {
        let track = taken
            .iter()
            .position(|t| (first..=last).all(|c| !t.get(c).copied().unwrap_or(false)))
            .unwrap_or(taken.len());
        if track == taken.len() {
            taken.push(vec![false; len]);
        }
        if let Some(t) = taken.get_mut(track) {
            for c in first..=last {
                if let Some(slot) = t.get_mut(c) {
                    *slot = true;
                }
            }
        }
        out.push(Track {
            index,
            first,
            last,
            track,
            continues_left: left,
            continues_right: right,
        });
    }
    out
}

/// `minutes` from midnight to the 15 below (Task's `SNAP_MINUTES`).
#[must_use]
pub const fn snap(minutes: i64) -> i64 {
    minutes.div_euclid(SNAP) * SNAP
}

/// `minutes` to the nearest 15.
#[must_use]
pub const fn snap_round(minutes: i64) -> i64 {
    (minutes + SNAP / 2).div_euclid(SNAP) * SNAP
}

pub const SNAP: i64 = 15;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn block(start: &str, end: &str) -> Block {
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        Block {
            key: start.into(),
            title: start.into(),
            color: "primary".into(),
            start: at(start),
            end: at(end),
            all_day: false,
            location: None,
            repeats: None,
            source: Source::Time {
                path: String::new(),
                running: false,
            },
        }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
    }

    // `overlap_two_back_to_back_share_a_column`.
    #[test]
    fn back_to_back_share_a_column() {
        let p = day_columns(
            day(),
            &[
                block("2026-11-06 09:00", "2026-11-06 10:00"),
                block("2026-11-06 10:00", "2026-11-06 11:00"),
            ],
        );
        assert!(p.iter().all(|p| p.column == 0 && p.columns == 1));
        assert_eq!((p[0].top, p[0].height), (540, 60));
    }

    // `overlap_two_concurrent_split_to_two_columns`.
    #[test]
    fn concurrent_ones_split() {
        let p = day_columns(
            day(),
            &[
                block("2026-11-06 09:00", "2026-11-06 11:00"),
                block("2026-11-06 09:30", "2026-11-06 10:00"),
                block("2026-11-06 10:00", "2026-11-06 10:30"),
                block("2026-11-06 13:00", "2026-11-06 14:00"),
            ],
        );
        let cols: Vec<(usize, usize)> = p.iter().map(|p| (p.column, p.columns)).collect();
        // The third reuses the second's column; the last is alone.
        assert_eq!(cols, [(0, 2), (1, 2), (1, 2), (0, 1)]);
    }

    #[test]
    fn a_day_is_clipped() {
        let p = day_columns(day(), &[block("2026-11-05 23:00", "2026-11-06 01:00")]);
        assert_eq!((p[0].top, p[0].height), (0, 60));
    }

    // `month_layout_multi_day_spans_three_columns`, `…_wraps_at_week_edge`.
    #[test]
    fn tracks_across_days() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let mut long = block("2026-11-03 00:00", "2026-11-06 00:00");
        long.all_day = true;
        let mut over = block("2026-11-07 00:00", "2026-11-10 00:00");
        over.all_day = true;
        let t = tracks(
            start,
            7,
            &[long, over, block("2026-11-04 09:00", "2026-11-04 10:00")],
            true,
        );
        assert_eq!((t[0].first, t[0].last, t[0].track), (1, 3, 0));
        assert_eq!((t[1].first, t[1].last, t[1].track), (2, 2, 1));
        assert_eq!((t[2].first, t[2].last, t[2].continues_right), (5, 6, true));
        assert_eq!(
            tracks(
                start,
                7,
                &[block("2026-11-04 09:00", "2026-11-04 10:00")],
                false
            ),
            []
        );
    }

    #[test]
    fn views_and_steps() {
        let d = day();
        assert_eq!(
            days(Mode::Week, d, Weekday::Mon)[0].to_string(),
            "2026-11-02"
        );
        assert_eq!(
            days(Mode::Week, d, Weekday::Sun)[0].to_string(),
            "2026-11-01"
        );
        let m = days(Mode::Month, d, Weekday::Mon);
        assert_eq!((m.len(), m[0].to_string()), (42, "2026-10-26".to_owned()));
        assert_eq!(
            step(
                Mode::Month,
                NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                -1
            )
            .to_string(),
            "2025-12-01"
        );
        assert_eq!(range_label(Mode::Week, d, Weekday::Mon), "Nov 2 – 8, 2026");
        assert_eq!(range_label(Mode::Month, d, Weekday::Mon), "November 2026");
        assert_eq!((snap(52), snap(-1), snap_round(52)), (45, -15, 45));
    }
}
