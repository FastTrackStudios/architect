//! Keeping a workspace to its invariants, after every operation and after
//! reading one back.
//!
//! - A pane is in exactly one group, and every tab names a pane.
//! - An empty group goes; a split with one child becomes that child; a
//!   split inside a split on the same axis merges into it.
//! - Shares are positive and sum to one.
//! - A group's `current` is one of its tabs.
//! - A window with nothing in it closes; the main window keeps one empty
//!   group ("New tab"). A dock's own window whose last tab went closes
//!   too, its ribbon and size going back to the main window's side.
//! - `active` and `recent` name panes that are open.
//! - A window's zoomed group is one of its center's, and the center is
//!   split (a lone group has nothing to zoom out of).

use std::collections::HashSet;

use crate::model::{Child, Docks, Group, GroupId, Node, PaneId, Split, Workspace};

/// The smallest share a child keeps, so a divider can always be found.
const MIN_SHARE: f32 = 0.02;

/// Drop tabs that are unknown or already seen; clamp `current`.
fn repair_group(group: &mut Group, known: &HashSet<PaneId>, seen: &mut HashSet<PaneId>) {
    group
        .tabs
        .retain(|t| known.contains(t) && seen.insert(t.clone()));
    if group.current >= group.tabs.len() {
        group.current = group.tabs.len().saturating_sub(1);
    }
}

/// Repair a tree; `None` when nothing is left in it.
fn repair_node(node: Node, known: &HashSet<PaneId>, seen: &mut HashSet<PaneId>) -> Option<Node> {
    match node {
        Node::Group(mut group) => {
            repair_group(&mut group, known, seen);
            (!group.tabs.is_empty()).then_some(Node::Group(group))
        }
        Node::Split(split) => {
            let axis = split.axis;
            let mut children: Vec<Child> = Vec::new();
            for child in split.children {
                let share = child.share;
                match repair_node(child.node, known, seen) {
                    None => {}
                    // A split on the same axis merges into its parent, its
                    // children taking their share of its share.
                    Some(Node::Split(inner)) if inner.axis == axis => {
                        let total: f32 = inner.children.iter().map(|c| c.share.max(0.0)).sum();
                        for c in inner.children {
                            let part = if total > 0.0 {
                                c.share.max(0.0) / total
                            } else {
                                0.0
                            };
                            children.push(Child {
                                node: c.node,
                                share: share * part,
                            });
                        }
                    }
                    Some(node) => children.push(Child { node, share }),
                }
            }
            match children.len() {
                0 => None,
                1 => children.pop().map(|c| c.node),
                _ => {
                    normalise(&mut children);
                    Some(Node::Split(Split {
                        id: split.id,
                        axis,
                        children,
                    }))
                }
            }
        }
    }
}

/// Shares positive, at least [`MIN_SHARE`], summing to one; even when
/// they are all unusable.
fn normalise(children: &mut [Child]) {
    let usable = |s: f32| s.is_finite() && s > 0.0;
    if !children.iter().any(|c| usable(c.share)) {
        for c in children.iter_mut() {
            c.share = 1.0;
        }
    }
    for c in children.iter_mut() {
        if !usable(c.share) {
            c.share = MIN_SHARE;
        }
    }
    let total: f32 = children.iter().map(|c| c.share).sum();
    for c in children.iter_mut() {
        c.share = (c.share / total).max(MIN_SHARE);
    }
    let total: f32 = children.iter().map(|c| c.share).sum();
    for c in children.iter_mut() {
        c.share /= total;
    }
}

impl Workspace {
    /// Restore the invariants (the module's list). Every operation calls
    /// it; so does reading a workspace back.
    pub fn repair(&mut self) {
        // Panes: unique ids.
        let mut ids = HashSet::new();
        self.panes.retain(|p| ids.insert(p.id.clone()));
        let known: HashSet<PaneId> = ids;
        let mut seen: HashSet<PaneId> = HashSet::new();

        for (w, window) in self.windows.iter_mut().enumerate() {
            window.center = match window.center.take() {
                // The main window's lone group stays, empty or not: it is
                // where the next pane opens, and keeps its id.
                Some(Node::Group(mut group)) if w == 0 => {
                    repair_group(&mut group, &known, &mut seen);
                    Some(Node::Group(group))
                }
                other => other.and_then(|n| repair_node(n, &known, &mut seen)),
            };
            for dock in [
                &mut window.docks.left,
                &mut window.docks.right,
                &mut window.docks.bottom,
            ] {
                dock.root = dock
                    .root
                    .take()
                    .and_then(|n| repair_node(n, &known, &mut seen));
                if !dock.size.is_finite() || dock.size < 120.0 {
                    dock.size = 120.0;
                }
            }
            for float in &mut window.floats {
                repair_group(&mut float.group, &known, &mut seen);
            }
            window.floats.retain(|f| !f.group.tabs.is_empty());
        }

        // A pane in no group (read from an older layout, or orphaned) joins
        // the main window's first group.
        let orphans: Vec<PaneId> = self
            .panes
            .iter()
            .map(|p| p.id.clone())
            .filter(|p| !seen.contains(p))
            .collect();

        // A dock's window with no tabs left gives its ribbon (and size)
        // back to the main window's side, so the sidebar is not lost.
        let mut returning = Vec::new();
        for window in self.windows.iter_mut().skip(1) {
            if window.center.is_some() || !window.floats.is_empty() {
                continue;
            }
            for side in Docks::SIDES {
                if let Some(dock) = window.docks.get_mut(side) {
                    if dock.root.is_none() && dock.ribbon.is_some() {
                        returning.push((side, dock.clone()));
                        dock.ribbon = None;
                    }
                }
            }
        }
        if let Some(main) = self.windows.first_mut() {
            for (side, dock) in returning {
                if let Some(home) = main.docks.get_mut(side).filter(|d| d.is_empty()) {
                    *home = dock;
                }
            }
        }

        // Pop-outs with nothing left close; the main window stays.
        let mut first = true;
        self.windows.retain(|w| {
            let docked = Docks::SIDES
                .iter()
                .any(|&s| w.docks.get(s).is_some_and(|d| d.root.is_some()));
            let keep = first || w.center.is_some() || !w.floats.is_empty() || docked;
            first = false;
            keep
        });
        if self.windows.is_empty() {
            *self = Self {
                panes: std::mem::take(&mut self.panes),
                ..Self::new()
            };
        }
        let needs_group = self.windows.first().is_some_and(|w| w.center.is_none());
        let fresh = if needs_group {
            Some(GroupId(self.alloc()))
        } else {
            None
        };
        if let Some(main) = self.windows.first_mut() {
            if let (None, Some(id)) = (&main.center, fresh) {
                main.center = Some(Node::Group(Group::new(id)));
            }
            let Some(center) = main.center.as_mut() else {
                return;
            };
            if !orphans.is_empty() {
                let mut groups = Vec::new();
                center.groups(&mut groups);
                let id = groups.first().map(|g| g.id);
                if let Some(group) = id.and_then(|id| center.group_mut(id)) {
                    group.tabs.extend(orphans);
                }
            }
        }

        for window in &mut self.windows {
            let split = matches!(window.center, Some(Node::Split(_)));
            let held = window
                .zoomed
                .is_some_and(|g| window.center.as_ref().is_some_and(|c| c.group(g).is_some()));
            if !(split && held) {
                window.zoomed = None;
            }
        }

        let open: HashSet<&PaneId> = self.panes.iter().map(|p| &p.id).collect();
        self.recent.retain(|p| open.contains(p));
        let mut dup = HashSet::new();
        self.recent.retain(|p| dup.insert(p.clone()));
        if self.active.as_ref().is_some_and(|a| !open.contains(a)) {
            self.active = self.recent.first().cloned();
        }

        // Ids handed out next stay past every id in use.
        let mut highest = 0u64;
        let mut record = |id: u64| highest = highest.max(id);
        for window in &self.windows {
            if let Some(center) = &window.center {
                max_ids(center, &mut record);
            }
            for dock in [
                &window.docks.left,
                &window.docks.right,
                &window.docks.bottom,
            ] {
                if let Some(root) = &dock.root {
                    max_ids(root, &mut record);
                }
            }
            for float in &window.floats {
                record(float.group.id.0);
            }
        }
        self.next_id = self.next_id.max(highest.saturating_add(1));
    }
}

fn max_ids(node: &Node, record: &mut dyn FnMut(u64)) {
    match node {
        Node::Group(g) => record(g.id.0),
        Node::Split(s) => {
            record(s.id.0);
            for c in &s.children {
                max_ids(&c.node, record);
            }
        }
    }
}
