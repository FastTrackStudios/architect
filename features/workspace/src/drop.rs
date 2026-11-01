//! Where a dragged pane would land, and where a float shows: pure
//! geometry, no renderer.
//!
//! The host measures its own rectangle and every group's, then asks
//! [`drop_at`] for the point under the pointer. Near the window's left,
//! right or bottom edge a pane docks there; over a group its outer bands
//! split the group and its middle adds a tab (a band is a quarter of the
//! group, kept between 24 and 96 points).

use crate::model::{Axis, Corner, Float, GroupId, Rect, Side, Snap};
use crate::ops::Dest;

/// How close to the window's edge a drop docks to that edge, points.
pub const EDGE_BAND: f32 = 28.0;
/// A group's split bands: this share of its size …
pub const SPLIT_SHARE: f32 = 0.25;
/// … kept within these, points.
pub const SPLIT_MIN: f32 = 24.0;
pub const SPLIT_MAX: f32 = 96.0;
/// How far a pinned float sits from its corner, points.
pub const PIN_MARGIN: f32 = 12.0;

/// A drop: where the pane goes, and the rectangle to light up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drop {
    pub dest: Dest,
    pub highlight: Rect,
}

/// How wide the line showing where a tab goes in a tab bar is, points.
pub const INSERT_W: f32 = 2.0;

/// A group's tab bar, measured: where a dragged tab is put among its tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub group: GroupId,
    /// The bar itself.
    pub rect: Rect,
    /// Its tabs, in order.
    pub tabs: Vec<Rect>,
}

/// The drop at `(x, y)` in a window at `host` holding `groups` (the
/// topmost first: floats before the docks and the center), if any.
#[must_use]
pub fn drop_at(x: f32, y: f32, host: Rect, groups: &[(GroupId, Rect)]) -> Option<Drop> {
    drop_in(x, y, host, &[], groups)
}

/// [`drop_at`], with the groups' tab bars: over a bar the tab goes in
/// among its tabs, before the one whose middle is past the pointer (the
/// highlight a line in the gap).
#[must_use]
pub fn drop_in(
    x: f32,
    y: f32,
    host: Rect,
    bars: &[Bar],
    groups: &[(GroupId, Rect)],
) -> Option<Drop> {
    if !contains(host, x, y) {
        return None;
    }
    if let Some(bar) = bars.iter().find(|b| contains(b.rect, x, y)) {
        return Some(bar_drop(bar, x));
    }
    edge_drop(x, y, host).or_else(|| {
        groups
            .iter()
            .find(|(_, r)| contains(*r, x, y))
            .map(|(id, r)| group_drop(*id, *r, x, y))
    })
}

/// Where along `bar` a tab let go at `x` goes, and the line to show.
#[must_use]
pub fn bar_drop(bar: &Bar, x: f32) -> Drop {
    let index = bar
        .tabs
        .iter()
        .position(|t| x < t.x + t.w / 2.0)
        .unwrap_or(bar.tabs.len());
    let at = match (
        index.checked_sub(1).and_then(|i| bar.tabs.get(i)),
        bar.tabs.get(index),
    ) {
        (_, Some(next)) => next.x,
        (Some(last), None) => last.right(),
        (None, None) => bar.rect.x,
    };
    Drop {
        dest: Dest::Group {
            group: bar.group,
            index: Some(index),
        },
        highlight: Rect::new(at - INSERT_W / 2.0, bar.rect.y, INSERT_W, bar.rect.h),
    }
}

fn edge_drop(x: f32, y: f32, host: Rect) -> Option<Drop> {
    // There is no dock at the top: the top edge is the title bar's.
    let side = nearest_side(host, x, y, EDGE_BAND).filter(|s| *s != Side::Top)?;
    Some(Drop {
        dest: Dest::Dock(side),
        highlight: band(host, side, (host.w.min(host.h) * 0.25).max(EDGE_BAND)),
    })
}

fn group_drop(group: GroupId, r: Rect, x: f32, y: f32) -> Drop {
    let reach = (r.w.min(r.h) * SPLIT_SHARE).clamp(SPLIT_MIN, SPLIT_MAX);
    nearest_side(r, x, y, reach).map_or(
        Drop {
            dest: Dest::Group { group, index: None },
            highlight: r,
        },
        |side| Drop {
            dest: Dest::Edge { group, side },
            highlight: band(r, side, along(r, side) / 2.0),
        },
    )
}

/// Where a float shows in a window's `area`: snapped, pinned to a corner,
/// or where it was put.
#[must_use]
pub fn float_rect(float: &Float, area: Rect) -> Rect {
    match float.snap {
        Some(Snap::Maximised) => area,
        Some(Snap::Half(side)) => band(area, side, along(area, side) / 2.0),
        Some(Snap::EdgeDock { side, width }) => band(area, side, width),
        None => float.pin.map_or(float.rect, |pin| {
            let r = float.rect;
            let left = area.x + PIN_MARGIN;
            let top = area.y + PIN_MARGIN;
            let right = area.right() - PIN_MARGIN - r.w;
            let bottom = area.bottom() - PIN_MARGIN - r.h;
            let (x, y) = match pin.corner {
                Corner::TopLeft => (left, top),
                Corner::TopRight => (right, top),
                Corner::BottomLeft => (left, bottom),
                Corner::BottomRight => (right, bottom),
            };
            Rect::new(x, y, r.w, r.h)
        }),
    }
}

/// The snap a float dragged to `(x, y)` takes when let go there (odysseus's
/// zones): the top edge fills the window, the left and right edges half it.
#[must_use]
pub fn snap_at(x: f32, y: f32, host: Rect) -> Option<Snap> {
    match nearest_side(host, x, y, EDGE_BAND / 2.0)? {
        Side::Top => Some(Snap::Maximised),
        side @ (Side::Left | Side::Right) => Some(Snap::Half(side)),
        Side::Bottom => None,
    }
}

/// The side of `r` within `reach` of the point, the nearest first.
fn nearest_side(r: Rect, x: f32, y: f32, reach: f32) -> Option<Side> {
    [
        (Side::Left, x - r.x),
        (Side::Right, r.right() - x),
        (Side::Top, y - r.y),
        (Side::Bottom, r.bottom() - y),
    ]
    .into_iter()
    .filter(|(_, d)| *d <= reach)
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(side, _)| side)
}

/// `r`'s size across `side` (its width for the left or right).
const fn along(r: Rect, side: Side) -> f32 {
    match side.axis() {
        Axis::Row => r.w,
        Axis::Column => r.h,
    }
}

/// The strip `thick` points deep along `side` of `r`.
fn band(r: Rect, side: Side, thick: f32) -> Rect {
    let thick = thick.min(along(r, side));
    match side {
        Side::Left => Rect::new(r.x, r.y, thick, r.h),
        Side::Right => Rect::new(r.right() - thick, r.y, thick, r.h),
        Side::Top => Rect::new(r.x, r.y, r.w, thick),
        Side::Bottom => Rect::new(r.x, r.bottom() - thick, r.w, thick),
    }
}

/// Whether `(x, y)` is inside `r`.
#[must_use]
pub fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
}
