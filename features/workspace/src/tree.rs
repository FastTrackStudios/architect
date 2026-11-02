//! The layout as a tree a caller can reason about: what `WORKSPACE_LAYOUT`
//! answers beside its flat lists ([`crate::Arrangement`]).
//!
//! Every window's center, docks and floats as nested splits and groups,
//! each with its id, its path from its area's root and its rectangle;
//! every tab with its view, title and history. A path names a place in
//! one tree: `center`, `center/0/1` (the second child of the first child
//! of the center's split), `left/0`, `float/2`. Ids are what an action
//! takes; a path finds an anonymous place (`WORKSPACE_RESIZE {path}`) and
//! holds for one revision only, as a split's id may after a repair.
//!
//! Rectangles are window points. A group the host has measured carries
//! the rectangle Blitz laid it out at ([`Measured`]); any other is laid out
//! from its window's measured area by the splits' shares and the docks'
//! sizes, and says so (`measured: false`). A window whose area nobody
//! measured (a headless workspace) has none.

// The facet derive's generated layout code for a data enum trips the
// arithmetic lint; the allowance stays in this module.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use facet::Facet;

use crate::model::{Axis, Chrome, Group, Node, Rect, Shown, Side, Snap, Window, Workspace};

/// What the host measured: each window's workspace area, and each group's
/// own rectangle, in window points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measured {
    /// By window id.
    pub areas: BTreeMap<String, Rect>,
    /// By group id.
    pub groups: BTreeMap<u64, Rect>,
}

/// A place in a window: a split or a group.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum LayoutNode {
    Split(LayoutSplit),
    Group(LayoutGroup),
}

/// A split: its children along its axis, each its share.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct LayoutSplit {
    pub id: u64,
    pub path: String,
    /// `row` (side by side) or `column` (one above another).
    pub axis: String,
    pub shares: Vec<f32>,
    #[facet(default)]
    pub rect: Option<Rect>,
    pub children: Vec<LayoutNode>,
}

/// A tab group.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct LayoutGroup {
    pub id: u64,
    pub path: String,
    #[facet(default)]
    pub rect: Option<Rect>,
    /// The rectangle is the host's measure, not one laid out from shares.
    pub measured: bool,
    pub tabs: Vec<LayoutTab>,
    /// The tab on show.
    pub current: String,
    pub stacked: bool,
    /// `tabs`, `icons` or `bare`.
    pub chrome: String,
    /// Shown alone in its window ([`crate::Workspace::set_zoomed`]).
    pub zoomed: bool,
}

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct LayoutTab {
    pub pane: String,
    pub view: String,
    pub title: String,
    pub pinned: bool,
    #[facet(default)]
    pub link: Option<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// A dock.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct LayoutDock {
    /// `left`, `right` or `bottom`.
    pub side: String,
    /// `open`, `collapsed` or `hidden`.
    pub shown: String,
    /// Width (height, at the bottom), points.
    pub size: f32,
    /// In a window of its own.
    pub popped_out: bool,
    #[facet(default)]
    pub rect: Option<Rect>,
    #[facet(default)]
    pub root: Option<LayoutNode>,
}

/// A float.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct LayoutFloat {
    pub group: LayoutGroup,
    /// Where it is when not snapped.
    pub rect: Rect,
    /// `maximised`, `left_half`, `right_half`, `left_panel`,
    /// `right_panel`; unset when not snapped.
    #[facet(default)]
    pub snap: Option<String>,
    /// The corner it is pinned to (`top_left`, …); unset when not.
    #[facet(default)]
    pub pin: Option<String>,
    pub minimised: bool,
}

const fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

/// Lays out one window's tree.
pub(crate) struct Builder<'a> {
    pub w: &'a Workspace,
    pub window: &'a Window,
    pub measured: &'a Measured,
}

impl Builder<'_> {
    fn tab(&self, id: &crate::model::PaneId) -> Option<LayoutTab> {
        let p = self.w.pane(id)?;
        Some(LayoutTab {
            pane: p.id.0.clone(),
            view: p.view.0.clone(),
            title: p.title.clone(),
            pinned: p.pinned,
            link: p.link.as_ref().map(|l| l.0.clone()),
            can_go_back: !p.history.back.is_empty(),
            can_go_forward: !p.history.forward.is_empty(),
        })
    }

    fn group(&self, g: &Group, path: String, estimate: Option<Rect>) -> LayoutGroup {
        let measured = self.measured.groups.get(&g.id.0).copied();
        LayoutGroup {
            id: g.id.0,
            path,
            rect: measured.or(estimate),
            measured: measured.is_some(),
            tabs: g.tabs.iter().filter_map(|t| self.tab(t)).collect(),
            current: g.current_tab().map(|p| p.0.clone()).unwrap_or_default(),
            stacked: g.stacked,
            chrome: match g.chrome {
                Chrome::Tabs => "tabs",
                Chrome::Icons => "icons",
                Chrome::Bare => "bare",
            }
            .to_owned(),
            zoomed: self.window.zoomed == Some(g.id),
        }
    }

    /// `node` laid out in `rect` (when known), at `path`.
    fn node(&self, node: &Node, path: String, rect: Option<Rect>) -> LayoutNode {
        match node {
            Node::Group(g) => LayoutNode::Group(self.group(g, path, rect)),
            Node::Split(s) => {
                // Each child's rect: the shares laid out in this one.
                let parts: Vec<Option<Rect>> = rect.map_or_else(
                    || vec![None; s.children.len()],
                    |r| {
                        let mut at = 0.0_f32;
                        s.children
                            .iter()
                            .map(|c| {
                                let part = match s.axis {
                                    Axis::Row => Rect::new(r.x + at, r.y, r.w * c.share, r.h),
                                    Axis::Column => Rect::new(r.x, r.y + at, r.w, r.h * c.share),
                                };
                                at += match s.axis {
                                    Axis::Row => part.w,
                                    Axis::Column => part.h,
                                };
                                Some(part)
                            })
                            .collect()
                    },
                );
                LayoutNode::Split(LayoutSplit {
                    id: s.id.0,
                    axis: match s.axis {
                        Axis::Row => "row",
                        Axis::Column => "column",
                    }
                    .to_owned(),
                    shares: s.children.iter().map(|c| c.share).collect(),
                    rect,
                    children: s
                        .children
                        .iter()
                        .zip(parts)
                        .enumerate()
                        .map(|(i, (c, r))| self.node(&c.node, format!("{path}/{i}"), r))
                        .collect(),
                    path,
                })
            }
        }
    }

    /// The window's center, docks and floats; `area` its measured area.
    pub fn build(&self) -> (Option<LayoutNode>, Vec<LayoutDock>, Vec<LayoutFloat>) {
        let area = self.measured.areas.get(&self.window.id.0).copied();
        let popped = !self.window.id.0.eq("main");
        let docks = &self.window.docks;
        let open = |s: Side| {
            docks
                .get(s)
                .filter(|d| d.root.is_some() && (d.shown == Shown::Open || popped))
                .map_or(0.0, |d| d.size)
        };
        let (left, right, bottom) = (open(Side::Left), open(Side::Right), open(Side::Bottom));
        // Left and right docks run the area's height; the bottom dock sits
        // under the center, between them.
        let center_rect = area.map(|a| {
            Rect::new(
                a.x + left,
                a.y,
                (a.w - left - right).max(0.0),
                (a.h - bottom).max(0.0),
            )
        });
        let dock_rect = |side: Side| {
            area.map(|a| match side {
                Side::Left => Rect::new(a.x, a.y, left, a.h),
                Side::Right => Rect::new(a.x + a.w - right, a.y, right, a.h),
                _ => Rect::new(
                    a.x + left,
                    a.y + a.h - bottom,
                    (a.w - left - right).max(0.0),
                    bottom,
                ),
            })
        };
        // A zoomed group fills the center.
        let center = self.window.center.as_ref().map(|c| {
            let zoomed = self.window.zoomed.and_then(|g| c.group(g)).map(|g| g.id);
            let mut tree = self.node(c, "center".to_owned(), center_rect);
            if let (Some(z), Some(r)) = (zoomed, center_rect) {
                set_rect(&mut tree, z.0, r);
            }
            tree
        });
        let docks = crate::model::Docks::SIDES
            .iter()
            .filter_map(|&side| {
                let d = docks.get(side)?;
                let name = side_name(side);
                let rect = d.root.as_ref().and_then(|_| dock_rect(side));
                Some(LayoutDock {
                    side: name.to_owned(),
                    shown: match d.shown {
                        Shown::Open => "open",
                        Shown::Collapsed => "collapsed",
                        Shown::Hidden => "hidden",
                    }
                    .to_owned(),
                    size: d.size,
                    popped_out: popped && d.root.is_some(),
                    rect,
                    root: d.root.as_ref().map(|n| self.node(n, name.to_owned(), rect)),
                })
            })
            .filter(|d| d.root.is_some() || !popped)
            .collect();
        let floats = self
            .window
            .floats
            .iter()
            .enumerate()
            .map(|(i, f)| LayoutFloat {
                group: self.group(&f.group, format!("float/{i}"), Some(f.rect)),
                rect: f.rect,
                snap: f.snap.map(|s| {
                    match s {
                        Snap::Maximised => "maximised",
                        Snap::Half(Side::Left) => "left_half",
                        Snap::Half(_) => "right_half",
                        Snap::EdgeDock {
                            side: Side::Left, ..
                        } => "left_panel",
                        Snap::EdgeDock { .. } => "right_panel",
                    }
                    .to_owned()
                }),
                pin: f.pin.map(|p| {
                    match p.corner {
                        crate::model::Corner::TopLeft => "top_left",
                        crate::model::Corner::TopRight => "top_right",
                        crate::model::Corner::BottomLeft => "bottom_left",
                        crate::model::Corner::BottomRight => "bottom_right",
                    }
                    .to_owned()
                }),
                minimised: f.minimised,
            })
            .collect();
        (center, docks, floats)
    }
}

/// Gives the group `id` the rectangle `rect` (unless measured).
fn set_rect(node: &mut LayoutNode, id: u64, rect: Rect) {
    match node {
        LayoutNode::Group(g) if g.id == id && !g.measured => g.rect = Some(rect),
        LayoutNode::Group(_) => {}
        LayoutNode::Split(s) => {
            for c in &mut s.children {
                set_rect(c, id, rect);
            }
        }
    }
}

/// The split at `path` in `w`'s window `window` (`"main"` by default):
/// what `WORKSPACE_RESIZE {path}` resizes.
#[must_use]
pub fn split_at_path(w: &Workspace, window: &str, path: &str) -> Option<crate::model::SplitId> {
    let win = w.windows.iter().find(|x| x.id.0 == window)?;
    let mut parts = path.split('/');
    let root = match parts.next()? {
        "center" => win.center.as_ref()?,
        "left" => win.docks.left.root.as_ref()?,
        "right" => win.docks.right.root.as_ref()?,
        "bottom" => win.docks.bottom.root.as_ref()?,
        _ => return None,
    };
    let mut at = root;
    for part in parts {
        let i: usize = part.parse().ok()?;
        let Node::Split(s) = at else {
            return None;
        };
        at = &s.children.get(i)?.node;
    }
    match at {
        Node::Split(s) => Some(s.id),
        Node::Group(_) => None,
    }
}
