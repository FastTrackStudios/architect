//! The Calendar section (#444): a provider whose snapshot has a month
//! grid (`Snapshot::calendar`) gets it under its top row, over its tree;
//! a day picked selects its day's folder and lists what is on it; the
//! calendar at the foot of a pane gives way to it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::Arc;

use architect_navigator::list::Clock;
use architect_navigator::{
    ListRowKind, MemorySource, Mount, Mounts, NavRowKind, NavTarget, NavigatorSettings,
    NavigatorState, NoteFile, OrgBadge, Pane, Place, ProviderSource, VaultIndex,
};
use architect_navigator_api::{CalendarGrid, GridDay, Item, Node, Snapshot};
use chrono::NaiveDate;

/// 2026-11-05, noon UTC.
const NOW: i64 = 1_793_880_000_000;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn calendar() -> Snapshot {
    let mut grid = CalendarGrid::default();
    for (day, count, label) in [("2026-11-05", 2, Some("Workday")), ("2026-11-06", 0, None)] {
        grid.days.insert(
            day.to_owned(),
            GridDay {
                node: format!("d:{day}"),
                count,
                label: label.map(str::to_owned),
                colour: label.map(|_| "#3b82f6".to_owned()),
            },
        );
    }
    Snapshot::new("Calendar")
        .ordered()
        .node(
            Node::new("y:2026", "2026").child(
                Node::new("m:2026-11", "November").child(
                    Node::new("w:2026-11-02", "Week 45")
                        .child(Node::new("d:2026-11-05", "Thu 5 Nov"))
                        .child(Node::new("d:2026-11-06", "Fri 6 Nov")),
                ),
            ),
        )
        .item(Item::new("e:Mix review.md", "d:2026-11-05", "Mix review"))
        .item(Item::new(
            "plan:2026-11-05",
            "d:2026-11-05",
            "Day plan · Workday",
        ))
        .item(Item::new("e:Show.md", "d:2026-11-06", "Show"))
        .calendar(grid)
}

const ID: &str = "~calendar:calendar";

fn state() -> (NavigatorState, Mounts) {
    let source = ProviderSource::with(calendar());
    let mut mount = Mount::provider("calendar/calendar", "Calendar", Place::Top).icon("calendar");
    mount.calendar = source.section_calendar(ID);
    let m = Mounts::new("Navigator").with(mount, Arc::new(source)).with(
        Mount::vault(OrgBadge::new("codywright", "Cody Wright", "#047857")),
        Arc::new(
            MemorySource::new("codywright")
                .with_folder("Notes")
                .with_file(NoteFile::new("Notes/A.md", NOW, NOW)),
        ),
    );
    let settings = NavigatorSettings {
        calendar: true,
        ..NavigatorSettings::default()
    };
    let mut s = NavigatorState::new(
        VaultIndex::read(&m),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 2000.0);
    (s, m)
}

#[test]
fn a_providers_grid_is_mapped_to_its_days_folders() {
    let source = ProviderSource::with(calendar());
    let cal = source.section_calendar(ID).unwrap();
    let thu = &cal.days[&d("2026-11-05")];
    assert_eq!(
        thu.folder,
        "~calendar:calendar/2026/November/Week 45/Thu 5 Nov"
    );
    assert_eq!((thu.count, thu.label.as_deref()), (2, Some("Workday")));
    assert!(
        ProviderSource::with(Snapshot::new("Plain"))
            .section_calendar(ID)
            .is_none()
    );
}

#[test]
fn the_grid_sits_under_its_row_and_a_day_picked_lists_whats_on_it() {
    let (mut s, _m) = state();
    let rows: Vec<String> = s
        .nav_rows()
        .iter()
        .take(4)
        .map(|r| match &r.kind {
            NavRowKind::Folder { name, .. } => name.clone(),
            NavRowKind::MonthGrid { mount } => format!("grid {mount}"),
            other => format!("{other:?}").chars().take(9).collect(),
        })
        .collect();
    assert_eq!(rows[1..3], ["Calendar".to_owned(), format!("grid {ID}")]);

    // The month shown is today's, and pages.
    assert_eq!(s.section_cursor(), d("2026-11-05"));
    let g = s.section_grid();
    assert_eq!(
        g.weeks.len(),
        5,
        "the whole month: November 2026 spans five weeks"
    );
    assert!(g.weeks.iter().flat_map(|w| &w.days).any(|x| x.today));
    s.section_move(1);
    assert_eq!(s.section_cursor().format("%Y-%m").to_string(), "2026-12");

    // Thursday picked: its folder selected, its items listed, the grid
    // back on its month.
    let _ = s.pick_section_day(d("2026-11-05"));
    assert_eq!(s.section_cursor(), d("2026-11-05"));
    assert_eq!(s.selected_section_day(), Some(d("2026-11-05")));
    assert_eq!(
        s.selection().target(),
        Some(&NavTarget::Folder(format!(
            "{ID}/2026/November/Week 45/Thu 5 Nov"
        )))
    );
    let listed: Vec<String> = s
        .list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            ListRowKind::File { path, .. } => Some(path.rsplit('/').next().unwrap().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().any(|p| p.starts_with("Mix review")));

    // A day the provider has no node for selects nothing.
    assert!(s.pick_section_day(d("2026-11-20")).is_none());
    assert_eq!(s.section_cursor(), d("2026-11-20"));
}

#[test]
fn the_calendar_at_the_foot_gives_way_to_the_section() {
    let (mut s, _m) = state();
    s.toggle_calendar();
    assert!(!s.calendar_visible(), "the section's grid is the calendar");
    let plain = MemorySource::new("v").with_folder("Notes");
    let mut p = NavigatorState::new(
        VaultIndex::read(&plain),
        NavigatorSettings {
            calendar: true,
            ..NavigatorSettings::default()
        },
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    p.toggle_calendar();
    assert!(p.calendar_visible());
}
