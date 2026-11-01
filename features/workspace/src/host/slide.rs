//! Tabs making room for a dragged one (VS Code's and Zed's): the tab
//! follows the pointer along its own bar while the others slide aside; off
//! its bar its place closes up, and over another bar a gap opens where it
//! would go. Let go, every tab that moved settles from where it was drawn
//! into its new place (each is moved back by the distance, then sprung to
//! nothing), so nothing jumps.
//!
//! The geometry is measured once, when the drag begins ([`Row`]); the
//! offsets are `transform`s on the tabs, so the bars do not lay out again
//! while a tab is dragged.

use architect_shell::motion::Motion;
use dioxus::prelude::*;

use super::Host;
use crate::model::{GroupId, PaneId, Rect};
use crate::ops::Dest;

/// The space between two tabs in a bar, points.
pub(super) const TAB_GAP: f32 = 1.0;

/// A tab bar as a drag found it: its group, its rectangle, its tabs.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row {
    pub group: GroupId,
    pub bar: Rect,
    pub tabs: Vec<(PaneId, Rect)>,
}

impl Row {
    fn index(&self, pane: &PaneId) -> Option<usize> {
        self.tabs.iter().position(|(p, _)| p == pane)
    }

    /// Where the first tab starts.
    fn start(&self) -> f32 {
        self.tabs.first().map_or(self.bar.x, |(_, r)| r.x)
    }
}

/// Where the dragged tab would go among a bar's tabs: its group and the
/// index (before the move, as [`Dest::Group`] counts it).
const fn bar_index(dest: Option<Dest>) -> Option<(GroupId, usize)> {
    match dest {
        Some(Dest::Group {
            group,
            index: Some(i),
        }) => Some((group, i)),
        _ => None,
    }
}

/// How far each tab but `dragged` stands aside while it is over
/// `hover` (a bar and an index there), else away from its own bar.
pub(super) fn aside(
    rows: &[Row],
    dragged: &PaneId,
    hover: Option<(GroupId, usize)>,
) -> Vec<(PaneId, f32)> {
    let source = rows
        .iter()
        .find_map(|r| r.index(dragged).map(|k| (r.group, k)));
    let width = rows
        .iter()
        .flat_map(|r| r.tabs.iter())
        .find(|(p, _)| p == dragged)
        .map_or(0.0, |(_, r)| r.w + TAB_GAP);
    let mut out = Vec::new();
    for row in rows {
        let own = source.filter(|(g, _)| *g == row.group).map(|(_, k)| k);
        let over = hover.filter(|(g, _)| *g == row.group).map(|(_, j)| j);
        for (i, (pane, _)) in row.tabs.iter().enumerate() {
            if pane == dragged {
                continue;
            }
            let by = match (own, over) {
                // Along its own bar: the ones it passes give way.
                (Some(k), Some(j)) if k < i && i < j => -width,
                (Some(k), Some(j)) if j <= i && i < k => width,
                // Over another bar: a gap where it would go.
                (None, Some(j)) if i >= j => width,
                // Off its own bar: its place closes up.
                (Some(k), None) if i > k => -width,
                _ => 0.0,
            };
            out.push((pane.clone(), by));
        }
    }
    out
}

/// Where each tab of the bars a move touches stands after it: the source
/// bar without `dragged`, the bar it goes to (`to`, an index there) with
/// it. Laid out from the bars' first tabs at the widths measured.
pub(super) fn after(
    rows: &[Row],
    dragged: &PaneId,
    to: Option<(GroupId, usize)>,
) -> Vec<(PaneId, f32)> {
    let mut out = Vec::new();
    let dragged_rect = rows
        .iter()
        .flat_map(|r| r.tabs.iter())
        .find(|(p, _)| p == dragged)
        .map(|(_, r)| *r);
    for row in rows {
        let own = row.index(dragged);
        let into = to.filter(|(g, _)| *g == row.group).map(|(_, j)| j);
        if own.is_none() && into.is_none() {
            continue;
        }
        let mut order: Vec<(PaneId, f32)> = row
            .tabs
            .iter()
            .filter(|(p, _)| p != dragged)
            .map(|(p, r)| (p.clone(), r.w))
            .collect();
        if let (Some(j), Some(r)) = (into, dragged_rect) {
            // The index counts the bar before the tab left it.
            let j = match own {
                Some(k) if j > k => j.saturating_sub(1),
                _ => j,
            };
            order.insert(j.min(order.len()), (dragged.clone(), r.w));
        }
        let mut x = row.start();
        for (pane, w) in order {
            out.push((pane, x));
            x += w + TAB_GAP;
        }
    }
    out
}

/// Each frame of a tab's drag: the dragged tab follows the pointer along
/// its own bar; the others stand aside.
pub(super) fn during(host: Host, pane: &PaneId, rows: &[Row], dest: Option<Dest>, dx: f32) {
    let hover = bar_index(dest);
    let own = rows.iter().find(|r| r.index(pane).is_some());
    let on_own = own.is_some_and(|r| hover.is_some_and(|(g, _)| g == r.group));
    let mut lifted = host.lifted;
    let now = Some((pane.clone(), on_own));
    if *lifted.peek() != now {
        lifted.set(now);
    }
    let motions = host.tab_shift.peek();
    for (p, by) in aside(rows, pane, hover) {
        if let Some(m) = motions.get(&p) {
            if (m.target() - f64::from(by)).abs() > 0.1 {
                m.to(f64::from(by));
            }
        }
    }
    if let (Some(row), Some(m)) = (own, motions.get(pane)) {
        if on_own {
            // Kept within its bar.
            let r = row
                .tabs
                .iter()
                .find(|(p, _)| p == pane)
                .map_or(row.bar, |(_, r)| *r);
            let least = row.bar.x - r.x;
            let most = (row.bar.right() - r.right()).max(least);
            m.hold(f64::from(dx.clamp(least, most)));
        } else if m.target().abs() > 0.1 {
            m.to(0.0);
        }
    }
}

/// Where a dragged tab was let go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Landing {
    /// Nowhere: it goes back.
    Back,
    /// Somewhere: a bar (an index there), another group, another window.
    To(Option<Dest>),
}

/// The drag let go, before the move is made: every tab of the bars it
/// touches is moved back by how far its place moves, so each settles from
/// where it was drawn. `ghost` is where the dragged tab's left edge is
/// drawn when it is off its own bar.
pub(super) fn landed(host: Host, pane: &PaneId, rows: &[Row], landing: Landing, ghost: f32) {
    let mut lifted = host.lifted;
    let on_own = lifted.peek().as_ref().is_some_and(|(_, own)| *own);
    lifted.set(None);
    let motions = host.tab_shift.peek().clone();
    let Landing::To(dest) = landing else {
        for m in motions.values() {
            m.to(0.0);
        }
        return;
    };
    // Into a group's middle, it goes after its last tab.
    let to = match dest {
        Some(Dest::Group { group, index: None }) => rows
            .iter()
            .find(|r| r.group == group)
            .map(|r| (group, r.tabs.len())),
        other => bar_index(other),
    };
    let was = |p: &PaneId| {
        rows.iter()
            .flat_map(|r| r.tabs.iter())
            .find(|(q, _)| q == p)
            .map(|(_, r)| r.x)
    };
    // Where each was drawn: its place and its shift, the dragged one where
    // the pointer has it.
    let drawn = |p: &PaneId, x: f32| -> f32 {
        let shift = motions.get(p).map_or(0.0, |m| narrow(m.peek()));
        if p == pane && !on_own {
            ghost
        } else {
            x + shift
        }
    };
    let source = rows
        .iter()
        .find(|r| r.index(pane).is_some())
        .map(|r| r.group);
    let placed = after(rows, pane, to);
    let mut from = host.tab_from;
    for (p, x) in &placed {
        let Some(old) = was(p) else {
            continue;
        };
        let offset = f64::from(drawn(p, old) - x);
        // Into another bar it is drawn afresh there: it starts from it.
        let remounts = p == pane && to.is_some_and(|(g, _)| Some(g) != source);
        if remounts {
            from.write().insert(p.clone(), offset);
        } else if let Some(m) = motions.get(p) {
            m.jump(offset);
            m.to(0.0);
        }
    }
    // The tabs of bars it did not touch go back to their places.
    for (p, m) in &motions {
        if !placed.iter().any(|(q, _)| q == p) && (m.target().abs() > 0.01 || m.peek().abs() > 0.01)
        {
            m.to(0.0);
        }
    }
}

/// A drag that ended without landing (Escape, a menu): every tab back.
pub(super) fn cancel(host: Host) {
    let mut lifted = host.lifted;
    if lifted.peek().is_some() {
        lifted.set(None);
    }
    for m in host.tab_shift.peek().values() {
        if m.target().abs() > 0.01 || m.peek().abs() > 0.01 {
            m.to(0.0);
        }
    }
}

/// The offset a tab drawn afresh starts from (it landed from another
/// bar), and its motion, registered so a drag can move it.
pub(super) fn tab_motion(host: Host, pane: &PaneId) -> Motion {
    let mut from = host.tab_from;
    let start = from.write().remove(pane).unwrap_or(0.0);
    let m = Motion::new(start, architect_shell::motion::Spring::SNAPPY, 0.25);
    let mut shift = host.tab_shift;
    shift.write().insert(pane.clone(), m);
    m
}

/// A tab no longer drawn: its motion goes, unless another took its place.
pub(super) fn forget(host: Host, pane: &PaneId, m: Motion) {
    let mut shift = host.tab_shift;
    if let Ok(mut all) = shift.try_write() {
        if all.get(pane) == Some(&m) {
            all.remove(pane);
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

#[cfg(test)]
mod tests {
    use super::{Row, TAB_GAP, after, aside};
    use crate::model::{GroupId, PaneId, Rect};

    fn row(group: u64, at: f32, names: &[&str]) -> Row {
        let mut x = at;
        let tabs = names
            .iter()
            .map(|n| {
                let r = Rect::new(x, 0.0, 100.0, 30.0);
                x += 100.0 + TAB_GAP;
                (PaneId::new(*n), r)
            })
            .collect();
        Row {
            group: GroupId(group),
            bar: Rect::new(at, 0.0, 600.0, 36.0),
            tabs,
        }
    }

    fn by(out: &[(PaneId, f32)], name: &str) -> f32 {
        out.iter()
            .find(|(p, _)| p.0 == name)
            .map_or(f32::NAN, |(_, v)| *v)
    }

    #[test]
    fn along_its_own_bar_the_tabs_it_passes_give_way() {
        let rows = [row(1, 0.0, &["a", "b", "c", "d"])];
        let a = PaneId::new("a");
        // Over the gap before d: b and c slide left into its place.
        let out = aside(&rows, &a, Some((GroupId(1), 3)));
        assert_eq!(by(&out, "b"), -101.0);
        assert_eq!(by(&out, "c"), -101.0);
        assert_eq!(by(&out, "d"), 0.0);
        // d dragged back to the front: the rest slide right.
        let d = PaneId::new("d");
        let out = aside(&rows, &d, Some((GroupId(1), 0)));
        assert_eq!(by(&out, "a"), 101.0);
        assert_eq!(by(&out, "c"), 101.0);
    }

    #[test]
    fn off_its_bar_its_place_closes_and_another_bar_opens_a_gap() {
        let rows = [row(1, 0.0, &["a", "b", "c"]), row(2, 700.0, &["x", "y"])];
        let b = PaneId::new("b");
        let out = aside(&rows, &b, Some((GroupId(2), 1)));
        assert_eq!(by(&out, "a"), 0.0);
        assert_eq!(by(&out, "c"), -101.0);
        assert_eq!(by(&out, "x"), 0.0);
        assert_eq!(by(&out, "y"), 101.0);
        // Over a group's middle, not a bar: only its place closes.
        let out = aside(&rows, &b, None);
        assert_eq!(by(&out, "c"), -101.0);
        assert_eq!(by(&out, "y"), 0.0);
    }

    #[test]
    fn after_a_move_each_tab_is_where_it_stood_aside_to() {
        let rows = [row(1, 0.0, &["a", "b", "c", "d"])];
        let a = PaneId::new("a");
        let out = after(&rows, &a, Some((GroupId(1), 3)));
        // b c a d: b and c are where they had slid to, a lands third.
        assert_eq!(by(&out, "b"), 0.0);
        assert_eq!(by(&out, "c"), 101.0);
        assert_eq!(by(&out, "a"), 202.0);
        assert_eq!(by(&out, "d"), 303.0);
        let rows = [row(1, 0.0, &["a", "b", "c"]), row(2, 700.0, &["x", "y"])];
        let b = PaneId::new("b");
        let out = after(&rows, &b, Some((GroupId(2), 1)));
        assert_eq!(by(&out, "c"), 101.0);
        assert_eq!(by(&out, "b"), 801.0);
        assert_eq!(by(&out, "y"), 902.0);
    }

    #[test]
    fn into_its_own_groups_middle_it_goes_last() {
        let rows = [row(1, 0.0, &["a", "b", "c"])];
        let a = PaneId::new("a");
        let out = after(&rows, &a, Some((GroupId(1), 3)));
        assert_eq!(by(&out, "b"), 0.0);
        assert_eq!(by(&out, "c"), 101.0);
        assert_eq!(by(&out, "a"), 202.0);
    }
}
