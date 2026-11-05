//! The two panes: side by side, stacked, or one at a time.
//!
//! Notebook Navigator's `NAVIGATION_PANE_DIMENSIONS` (`src/types.ts`) and
//! its layout rule (`src/context/UIStateContext.tsx`,
//! `src/utils/paneLayout.ts`): a phone is always one pane at a time; two
//! side-by-side panes in a sidebar narrower than the folders' width plus
//! 150 become one at a time (or stacked, by `narrowSidebarLayout`).

use crate::settings::{NarrowLayout, NavigatorSettings, Orientation};

/// The folders' pane, side by side: 200 wide, at least 150.
pub const NAV_WIDTH: f64 = 200.0;
pub const NAV_MIN_WIDTH: f64 = 150.0;
/// The folders' pane, stacked: 260 tall, at least 160.
pub const NAV_HEIGHT: f64 = 260.0;
pub const NAV_MIN_HEIGHT: f64 = 160.0;
/// The notes' pane: at least 150 wide.
pub const LIST_MIN_WIDTH: f64 = 150.0;
/// How much wider than the folders' pane a sidebar must be for two panes
/// side by side (`fitPanes`).
pub const FIT_MARGIN: f64 = 150.0;

/// A pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    /// The folder tree.
    Navigation,
    /// The notes.
    #[default]
    Files,
}

/// How the panes are laid out now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneLayout {
    Dual(Orientation),
    Single,
}

/// The layout for `settings`, on a phone or not, in a sidebar `width`
/// wide (when known) with the folders' pane `nav_width` wide.
#[must_use]
pub fn resolve(
    settings: &NavigatorSettings,
    phone: bool,
    width: Option<f64>,
    nav_width: f64,
) -> PaneLayout {
    if phone || !settings.dual_pane {
        return PaneLayout::Single;
    }
    if settings.orientation == Orientation::Horizontal
        && let Some(w) = width
        && w > 0.0
        && w < nav_width + FIT_MARGIN
    {
        return match settings.narrow_layout {
            NarrowLayout::Keep => PaneLayout::Dual(Orientation::Horizontal),
            NarrowLayout::SinglePane => PaneLayout::Single,
            NarrowLayout::Vertical => PaneLayout::Dual(Orientation::Vertical),
        };
    }
    PaneLayout::Dual(settings.orientation)
}

/// The folders' pane's size after a drag, at least its minimum (no
/// maximum, as in Notebook Navigator) and leaving the notes `LIST_MIN_WIDTH`
/// when the sidebar's size is known.
#[must_use]
pub fn clamp_size(orientation: Orientation, size: f64, room: Option<f64>) -> f64 {
    let (min, keep) = match orientation {
        Orientation::Horizontal => (NAV_MIN_WIDTH, LIST_MIN_WIDTH),
        Orientation::Vertical => (NAV_MIN_HEIGHT, LIST_MIN_WIDTH),
    };
    let max = room
        .filter(|r| *r > 0.0)
        .map_or(f64::MAX, |r| (r - keep).max(min));
    size.clamp(min, max)
}
