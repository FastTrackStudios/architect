//! The workspace as a guide scenario sees it (`architect-guide`).
//!
//! Its targets as CSS selectors, and its arrangement as plain data a
//! check compares (`WORKSPACE_LAYOUT`, which feature `ui`'s
//! [`use_workspace_actions`](crate::use_workspace_actions) puts on the
//! app's Action List).

use facet::Facet;

use crate::model::{Axis, Node, Workspace};

/// The read-only Action List action that answers the workspace's
/// [`Arrangement`].
pub const LAYOUT_ACTION: &str = "WORKSPACE_LAYOUT";

/// A scenario's target in the workspace, as a CSS selector over the
/// host's `data-*` attributes, or `None` for a kind that is not the
/// workspace's:
///
/// - `tab` — a pane's tab (or its spine, stacked), by the pane's id;
/// - `pane` — the pane's body;
/// - `group` — the tab group holding the pane;
/// - `tab-bar` — that group's tab bar;
/// - `split` — the divider right after the pane (the innermost split's,
///   where splits nest);
/// - `card` — the pane's card in a phone's tab switcher;
/// - `card-close` — that card's ✕.
#[must_use]
pub fn target_selector(kind: &str, name: &str) -> Option<String> {
    Some(match kind {
        "tab" => format!("[data-tab=\"{name}\"]"),
        "pane" => format!("[data-pane=\"{name}\"]"),
        "group" => format!("[data-group-tabs~=\"{name}\"]"),
        "tab-bar" => format!("[data-tab-bar~=\"{name}\"]"),
        "split" => format!("[data-split-after~=\"{name}\"]"),
        "card" => format!("[data-card=\"{name}\"]"),
        "card-close" => format!("[data-card-close=\"{name}\"]"),
        _ => return None,
    })
}

/// The arrangement, as a check reads it: every window's groups (each its
/// tabs in order) and splits.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct Arrangement {
    /// The active pane's id; empty when none is.
    pub active: String,
    pub windows: Vec<ArrangedWindow>,
}

#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct ArrangedWindow {
    pub id: String,
    /// Its center's groups, in order (left to right, top to bottom).
    pub groups: Vec<ArrangedGroup>,
    /// Its center's splits, outermost first.
    pub splits: Vec<ArrangedSplit>,
    /// Its docks' groups (the main window's), left, right, bottom.
    pub docked: Vec<ArrangedGroup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct ArrangedGroup {
    pub id: u64,
    pub tabs: Vec<String>,
    /// The tab showing.
    pub current: String,
    pub stacked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct ArrangedSplit {
    /// `row` (side by side) or `column` (one above another).
    pub axis: String,
    /// Each child's share of its length.
    pub shares: Vec<f32>,
}

impl Arrangement {
    /// The layout of `w`.
    #[must_use]
    pub fn of(w: &Workspace) -> Self {
        let windows = w
            .windows
            .iter()
            .map(|win| {
                let mut groups = Vec::new();
                let mut splits = Vec::new();
                if let Some(center) = &win.center {
                    walk(center, &mut groups, &mut splits);
                }
                let mut docked = Vec::new();
                for dock in [&win.docks.left, &win.docks.right, &win.docks.bottom] {
                    if let Some(root) = &dock.root {
                        walk(root, &mut docked, &mut Vec::new());
                    }
                }
                ArrangedWindow {
                    id: win.id.0.clone(),
                    groups,
                    splits,
                    docked,
                }
            })
            .collect();
        Self {
            active: w.active.as_ref().map(|p| p.0.clone()).unwrap_or_default(),
            windows,
        }
    }
}

fn walk(node: &Node, groups: &mut Vec<ArrangedGroup>, splits: &mut Vec<ArrangedSplit>) {
    match node {
        Node::Group(g) => groups.push(ArrangedGroup {
            id: g.id.0,
            tabs: g.tabs.iter().map(|p| p.0.clone()).collect(),
            current: g.current_tab().map(|p| p.0.clone()).unwrap_or_default(),
            stacked: g.stacked,
        }),
        Node::Split(s) => {
            splits.push(ArrangedSplit {
                axis: match s.axis {
                    Axis::Row => "row",
                    Axis::Column => "column",
                }
                .to_owned(),
                shares: s.children.iter().map(|c| c.share).collect(),
            });
            for child in &s.children {
                walk(&child.node, groups, splits);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pane, Side};
    use crate::ops::Target;

    #[test]
    fn a_layout_lists_each_group_in_order_and_each_split() {
        let mut w = Workspace::new();
        let _ = w.open(Pane::new("a", "note", "A"), Target::Tab);
        let _ = w.open(Pane::new("b", "note", "B"), Target::Tab);
        let _ = w.open(Pane::new("c", "note", "C"), Target::Split(Side::Right));
        let layout = Arrangement::of(&w);
        assert_eq!(layout.active, "c");
        let main = &layout.windows[0];
        assert_eq!(main.groups.len(), 2);
        assert_eq!(main.groups[0].tabs, ["a", "b"]);
        assert_eq!(main.groups[1].tabs, ["c"]);
        assert_eq!(main.splits.len(), 1);
        assert_eq!(main.splits[0].axis, "row");
    }

    #[test]
    fn targets_are_selectors_over_the_hosts_attributes() {
        assert_eq!(
            target_selector("tab", "files").as_deref(),
            Some("[data-tab=\"files\"]")
        );
        assert_eq!(
            target_selector("group", "files").as_deref(),
            Some("[data-group-tabs~=\"files\"]")
        );
        assert_eq!(target_selector("palette-row", "X"), None);
    }
}
