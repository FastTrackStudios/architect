//! 100,000 notes: the virtual window draws a screenful whatever the
//! scroll, scrolling and jumping to the end stay quick, and a note at the
//! very end is revealed into view.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::while_float
)]

use std::time::{Duration, Instant};

use architect_navigator::list::Clock;
use architect_navigator::virtualize::{Align, OVERSCAN, Offsets};
use architect_navigator::{
    Key, MemorySource, Mods, NavigatorSettings, NavigatorState, Pane, VaultIndex,
};

const NOTES: usize = 100_000;
const NOW: i64 = 1_793_880_000_000;

#[test]
fn the_window_is_a_screenful_at_any_offset() {
    let offsets = Offsets::new((0..NOTES).map(|i| if i % 50 == 0 { 27.0 } else { 73.0 }));
    assert_eq!(offsets.len(), NOTES);
    let view = 800.0;
    let started = Instant::now();
    let mut y = 0.0;
    while y < offsets.total() {
        let w = offsets.window(y, view, OVERSCAN);
        assert!(w.len() <= 800 / 27 + 2 * OVERSCAN + 2, "{w:?} at {y}");
        assert!(offsets.start(w.start) <= y + 0.5 || w.start == 0);
        y += 997.0;
    }
    // ~7,300 windows over 100k rows: binary searches, not scans.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // The last row, centred and at the end.
    let last = NOTES - 1;
    let to = offsets.scroll_to(last, Align::Center, 0.0, view);
    assert!((to - offsets.max_scroll(view)).abs() < 0.5);
    assert!(offsets.window(to, view, OVERSCAN).contains(&last));
}

#[test]
fn a_hundred_thousand_notes_scroll_and_reveal() {
    let source = MemorySource::synthetic("Big", 20, NOTES, NOW);
    let started = Instant::now();
    let mut settings = NavigatorSettings::default();
    settings.show_root = true;
    settings.include_descendants = true;
    let mut s = NavigatorState::new(
        VaultIndex::read(&source),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 700.0);
    s.set_view(Pane::Files, 700.0);
    // The root lists every note (from the folders below).
    s.click_folder("");
    assert_eq!(s.list_order().len(), NOTES);
    let built = started.elapsed();
    assert!(
        built < Duration::from_secs(10),
        "indexed and listed in {built:?}"
    );
    // Only a screenful (and the overscan) is drawn, at the top, the middle
    // and the end.
    for y in [0.0, s.list_offsets().total() / 2.0, f64::MAX] {
        s.scroll_to(Pane::Files, y);
        let w = s.window(Pane::Files);
        assert!(!w.is_empty() && w.len() < 60, "{w:?} at {y}");
    }
    // End: the last note selected and in view.
    s.focus_pane(Pane::Files);
    let t = Instant::now();
    s.key(Key::End, Mods::NONE);
    let last = s.list_order().last().map(|p| (*p).to_owned()).unwrap();
    assert_eq!(s.selection().cursor(), Some(last.as_str()));
    let row = s
        .list_rows()
        .iter()
        .position(|r| r.file() == Some(last.as_str()))
        .unwrap();
    assert!(s.window(Pane::Files).contains(&row));
    // Home: back to the top.
    s.key(Key::Home, Mods::NONE);
    assert!(s.scroll(Pane::Files).offset < 100.0);
    // A page at a time.
    s.key(Key::PageDown, Mods::NONE);
    let paged = s
        .list_order()
        .iter()
        .position(|p| Some(*p) == s.selection().cursor())
        .unwrap();
    assert!(paged > 3, "a page moved {paged}");
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "keys took {:?}",
        t.elapsed()
    );
    // A reveal deep in the vault opens its folders and scrolls to it.
    let deep = "Area 1/Topic 9/Note 99999.md";
    assert!(s.file(deep).is_some());
    s.set_active(Some(deep));
    assert_eq!(s.selection().cursor(), Some(deep));
    let row = s
        .list_rows()
        .iter()
        .position(|r| r.file() == Some(deep))
        .unwrap();
    assert!(s.window(Pane::Files).contains(&row));
}
