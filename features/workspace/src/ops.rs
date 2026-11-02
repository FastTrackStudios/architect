//! The operations: what every gesture and every key comes down to. Each
//! leaves the workspace repaired ([`Workspace::repair`]).

use crate::model::{
    Area, Axis, Child, Chrome, ClosedPane, Dock, Docks, Float, Group, GroupId, LinkId, Node, Pane,
    PaneId, Pin, Rect, Shown, Side, Snap, Split, SplitId, Window, WindowId, Workspace,
};

/// Where [`Workspace::open`] puts a pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// In place of the active tab, unless it is pinned (then a new tab).
    Here,
    /// A new tab beside the active one.
    Tab,
    /// A new group on this side of the active one.
    Split(Side),
    /// A window of its own.
    Window(Option<Rect>),
    /// The dock on this side, which opens.
    Dock(Side),
    /// Floating over the active window.
    Float(Rect),
}

/// Where [`Workspace::move_pane`] moves a pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dest {
    /// Into a group, at an index (the end when `None`).
    Group {
        group: GroupId,
        index: Option<usize>,
    },
    /// Beside a group, splitting it on that side.
    Edge { group: GroupId, side: Side },
    /// A window of its own.
    Window(Option<Rect>),
    /// The dock on this side.
    Dock(Side),
    /// Floating over the main window.
    Float(Rect),
}

/// What an operation could not do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    #[error("no pane {0:?}")]
    NoSuchPane(PaneId),
    #[error("no group {0:?}")]
    NoSuchGroup(GroupId),
    #[error("no split {0:?}")]
    NoSuchSplit(SplitId),
    #[error("no window {0:?}")]
    NoSuchWindow(WindowId),
    #[error("there is no dock on that side")]
    NoDock,
    #[error("the {0:?} dock has nothing in it to pop out")]
    EmptyDock(Side),
    #[error("the {0:?} dock is in the main window")]
    DockNotOut(Side),
    #[error("{0:?} is not floating")]
    NotFloating(PaneId),
    #[error("nothing to go back or forward to")]
    NoHistory,
    #[error("{0:?} is not in a window's center")]
    NotInCenter(GroupId),
}

type Result<T> = std::result::Result<T, WorkspaceError>;

/// Put `new` beside the group `target` on `side`, inside `node`. Splits the
/// group's share in a split on the same axis, or makes one. Hands `new`
/// back when the group is not in this tree.
fn insert_beside(
    node: &mut Node,
    target: GroupId,
    side: Side,
    new: Node,
    alloc: &mut dyn FnMut() -> u64,
) -> std::result::Result<(), Node> {
    let axis = side.axis();
    match node {
        Node::Group(group) if group.id == target => {
            let old = std::mem::replace(node, Node::Group(Group::new(GroupId(0))));
            let (first, second) = if side.leads() { (new, old) } else { (old, new) };
            *node = Node::Split(Split {
                id: SplitId(alloc()),
                axis,
                children: vec![
                    Child {
                        node: first,
                        share: 0.5,
                    },
                    Child {
                        node: second,
                        share: 0.5,
                    },
                ],
            });
            Ok(())
        }
        Node::Group(_) => Err(new),
        Node::Split(split) => {
            if split.axis == axis {
                let hit = split
                    .children
                    .iter()
                    .position(|c| matches!(&c.node, Node::Group(g) if g.id == target));
                if let Some(i) = hit {
                    let share = split.children.get(i).map_or(0.5, |c| c.share) / 2.0;
                    if let Some(c) = split.children.get_mut(i) {
                        c.share = share;
                    }
                    let at = if side.leads() { i } else { i.saturating_add(1) };
                    split.children.insert(at, Child { node: new, share });
                    return Ok(());
                }
            }
            let mut new = new;
            for child in &mut split.children {
                match insert_beside(&mut child.node, target, side, new, alloc) {
                    Ok(()) => return Ok(()),
                    Err(back) => new = back,
                }
            }
            Err(new)
        }
    }
}

impl Workspace {
    fn fresh_group(&mut self, tabs: Vec<PaneId>) -> Group {
        let mut group = Group::new(GroupId(self.alloc()));
        group.tabs = tabs;
        group
    }

    /// Put `new` beside `target` on `side`, wherever `target` is.
    fn place_beside(&mut self, target: GroupId, side: Side, new: Node) -> Result<()> {
        let mut next = self.next_id;
        let mut alloc = || {
            let id = next;
            next = next.saturating_add(1);
            id
        };
        let mut new = new;
        for window in &mut self.windows {
            let Window {
                center,
                docks,
                floats,
                ..
            } = window;
            let roots = center.iter_mut().chain(
                [&mut docks.left, &mut docks.right, &mut docks.bottom]
                    .into_iter()
                    .filter_map(|d| d.root.as_mut()),
            );
            for root in roots {
                match insert_beside(root, target, side, new, &mut alloc) {
                    Ok(()) => {
                        self.next_id = next;
                        return Ok(());
                    }
                    Err(back) => new = back,
                }
            }
            // A float is one group: splitting it makes the float a group
            // of both (floats do not split; the new pane joins its tabs).
            if let Some(float) = floats.iter_mut().find(|f| f.group.id == target) {
                if let Node::Group(g) = new {
                    float.group.tabs.extend(g.tabs);
                }
                self.next_id = next;
                return Ok(());
            }
        }
        Err(WorkspaceError::NoSuchGroup(target))
    }

    /// Take `pane` out of its group, keeping it in `panes`. Where it was.
    fn detach(&mut self, pane: &PaneId) -> Result<(GroupId, usize)> {
        let loc = self
            .locate(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let group = self
            .group_mut(loc.group)
            .ok_or(WorkspaceError::NoSuchGroup(loc.group))?;
        if loc.index < group.tabs.len() {
            group.tabs.remove(loc.index);
        }
        if group.current > loc.index || group.current >= group.tabs.len() {
            group.current = group.current.saturating_sub(1);
        }
        Ok((loc.group, loc.index))
    }

    /// Put an already-registered pane at `dest`.
    pub(crate) fn place(&mut self, pane: PaneId, dest: Dest) -> Result<()> {
        match dest {
            Dest::Group { group, index } => {
                let g = self
                    .group_mut(group)
                    .ok_or(WorkspaceError::NoSuchGroup(group))?;
                let at = index.unwrap_or(g.tabs.len()).min(g.tabs.len());
                g.tabs.insert(at, pane);
                g.current = at;
            }
            Dest::Edge { group, side } => {
                let new = Node::Group(self.fresh_group(vec![pane]));
                self.place_beside(group, side, new)?;
            }
            Dest::Window(bounds) => {
                let group = self.fresh_group(vec![pane]);
                // Ids are never reused: a window closed and another opened
                // are two windows to the platform (and to a script).
                let n = self.alloc();
                let mut window = Window::new(WindowId(format!("window-{n}")), Node::Group(group));
                window.bounds = bounds;
                self.windows.push(window);
            }
            Dest::Dock(side) => {
                let fresh = self.fresh_group(Vec::new());
                // The dock wherever it is: popped out, a pane joins it there.
                let dock = self.dock_mut(side).ok_or(WorkspaceError::NoDock)?;
                dock.shown = Shown::Open;
                let mut fresh = fresh;
                match dock.root.as_mut().and_then(|r| match r {
                    Node::Group(g) => Some(g),
                    Node::Split(_) => None,
                }) {
                    Some(group) => {
                        group.tabs.push(pane);
                        group.current = group.tabs.len().saturating_sub(1);
                    }
                    None => {
                        if let Some(root) = dock.root.as_mut() {
                            // A split dock: the pane joins its first group.
                            let mut groups = Vec::new();
                            root.groups(&mut groups);
                            let first = groups.first().map(|g| g.id);
                            if let Some(g) = first.and_then(|id| root.group_mut(id)) {
                                g.tabs.push(pane);
                                g.current = g.tabs.len().saturating_sub(1);
                            }
                        } else {
                            fresh.tabs.push(pane);
                            fresh.chrome = Chrome::Icons;
                            dock.root = Some(Node::Group(fresh));
                        }
                    }
                }
            }
            Dest::Float(rect) => {
                let group = self.fresh_group(vec![pane]);
                let main = self
                    .windows
                    .first_mut()
                    .ok_or_else(|| WorkspaceError::NoSuchWindow(WindowId::main()))?;
                main.floats.push(Float {
                    group,
                    rect,
                    snap: None,
                    pin: None,
                    minimised: false,
                });
            }
        }
        Ok(())
    }

    /// Open `pane` at `target`. A pane whose id is already open is brought
    /// forward instead: ids are stable, so opening one twice finds it.
    pub fn open(&mut self, pane: Pane, target: Target) -> Result<PaneId> {
        let id = pane.id.clone();
        if self.pane(&id).is_some() {
            self.activate(&id)?;
            return Ok(id);
        }
        let active_group = self.active_group();
        let active = self.active.clone();
        self.panes.push(pane);
        let dest = match (target, active_group) {
            (Target::Here, Some(group)) => {
                let replace = active
                    .as_ref()
                    .filter(|a| self.pane(a).is_some_and(|p| !p.pinned))
                    .and_then(|a| {
                        self.locate(a)
                            .filter(|l| l.group == group)
                            .map(|l| (a.clone(), l.index))
                    });
                match replace {
                    Some((old, index)) => {
                        let g = self
                            .group_mut(group)
                            .ok_or(WorkspaceError::NoSuchGroup(group))?;
                        g.tabs.insert(index.saturating_add(1), id.clone());
                        self.close(&old)?;
                        return self.focus_new(id);
                    }
                    None => Dest::Group { group, index: None },
                }
            }
            (Target::Tab, Some(group)) => {
                let after = self.group(group).map(|g| g.current.saturating_add(1));
                Dest::Group {
                    group,
                    index: after,
                }
            }
            (Target::Split(side), Some(group)) => Dest::Edge { group, side },
            (Target::Window(bounds), _) => Dest::Window(bounds),
            (Target::Dock(side), _) => Dest::Dock(side),
            (Target::Float(rect), _) => Dest::Float(rect),
            (Target::Here | Target::Tab | Target::Split(_), None) => {
                let group = self.fresh_group(Vec::new());
                let gid = group.id;
                if let Some(main) = self.windows.first_mut() {
                    main.center = Some(Node::Group(group));
                }
                Dest::Group {
                    group: gid,
                    index: None,
                }
            }
        };
        self.place(id.clone(), dest)?;
        self.focus_new(id)
    }

    fn focus_new(&mut self, id: PaneId) -> Result<PaneId> {
        self.repair();
        self.activate(&id)?;
        Ok(id)
    }

    /// Move a pane: to another group (or another place in its own), beside
    /// a group, into a window, a dock, or a float.
    pub fn move_pane(&mut self, pane: &PaneId, dest: Dest) -> Result<()> {
        let (from, index) = self.detach(pane)?;
        // Reordering within one group counts the index after the removal.
        let dest = match dest {
            Dest::Group {
                group,
                index: Some(at),
            } if group == from && at > index => Dest::Group {
                group,
                index: Some(at.saturating_sub(1)),
            },
            other => other,
        };
        self.place(pane.clone(), dest)?;
        self.repair();
        self.activate(pane)
    }

    /// Close a pane, keeping it to reopen.
    pub fn close(&mut self, pane: &PaneId) -> Result<()> {
        let (group, index) = self.detach(pane)?;
        let at = self
            .panes
            .iter()
            .position(|p| &p.id == pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let closed = self.panes.remove(at);
        self.closed.push(ClosedPane {
            pane: closed,
            group,
            index,
        });
        if self.closed.len() > Self::CLOSED_KEPT {
            self.closed.remove(0);
        }
        self.recent.retain(|p| p != pane);
        if self.active.as_ref() == Some(pane) {
            self.active = self.recent.first().cloned();
        }
        self.repair();
        Ok(())
    }

    /// Close every other unpinned tab in this pane's group.
    pub fn close_others(&mut self, pane: &PaneId) -> Result<()> {
        let loc = self
            .locate(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let others: Vec<PaneId> = self
            .group(loc.group)
            .map(|g| g.tabs.iter().filter(|t| *t != pane).cloned().collect())
            .unwrap_or_default();
        for other in others {
            if self.pane(&other).is_some_and(|p| !p.pinned) {
                self.close(&other)?;
            }
        }
        Ok(())
    }

    /// Close every tab in a group.
    pub fn close_group(&mut self, group: GroupId) -> Result<()> {
        let tabs = self
            .group(group)
            .ok_or(WorkspaceError::NoSuchGroup(group))?
            .tabs
            .clone();
        for tab in tabs {
            self.close(&tab)?;
        }
        Ok(())
    }

    /// Reopen the last closed pane where it was (or as a new tab).
    pub fn reopen(&mut self) -> Result<Option<PaneId>> {
        let Some(closed) = self.closed.pop() else {
            return Ok(None);
        };
        let id = closed.pane.id.clone();
        if self.pane(&id).is_some() {
            self.activate(&id)?;
            return Ok(Some(id));
        }
        self.panes.push(closed.pane);
        let dest = if self.group(closed.group).is_some() {
            Dest::Group {
                group: closed.group,
                index: Some(closed.index),
            }
        } else {
            self.active_group()
                .map_or(Dest::Float(Rect::new(80.0, 80.0, 480.0, 360.0)), |group| {
                    Dest::Group { group, index: None }
                })
        };
        self.place(id.clone(), dest)?;
        self.focus_new(id).map(Some)
    }

    /// Make a pane the active one: its group shows it, a minimised float
    /// holding it comes back and to the front.
    pub fn activate(&mut self, pane: &PaneId) -> Result<()> {
        let loc = self
            .locate(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        if let Some(group) = self.group_mut(loc.group) {
            group.current = loc.index;
        }
        if let Area::Float(f) = loc.area {
            if let Some(window) = self.windows.get_mut(loc.window) {
                if f < window.floats.len() {
                    let mut float = window.floats.remove(f);
                    float.minimised = false;
                    window.floats.push(float);
                }
            }
        }
        self.recent.retain(|p| p != pane);
        self.recent.insert(0, pane.clone());
        self.active = Some(pane.clone());
        Ok(())
    }

    /// Show tab `n` (from zero) of the active group; the last when `n` is
    /// past the end (⌘9).
    pub fn goto_tab(&mut self, n: usize) -> Result<()> {
        let Some(group) = self.active_group() else {
            return Ok(());
        };
        let tab = self
            .group(group)
            .and_then(|g| g.tabs.get(n.min(g.tabs.len().saturating_sub(1))).cloned());
        tab.map_or(Ok(()), |tab| self.activate(&tab))
    }

    /// The next (or, `back`, the previous) tab of the active group, round.
    pub fn cycle_tab(&mut self, back: bool) -> Result<()> {
        let Some(group) = self.active_group() else {
            return Ok(());
        };
        let tab = self.group(group).and_then(|g| {
            let n = g.tabs.len();
            if n == 0 {
                return None;
            }
            let i = if back {
                g.current
                    .checked_sub(1)
                    .unwrap_or_else(|| n.saturating_sub(1))
            } else {
                g.current.saturating_add(1).checked_rem(n).unwrap_or(0)
            };
            g.tabs.get(i).cloned()
        });
        tab.map_or(Ok(()), |tab| self.activate(&tab))
    }

    pub fn set_pinned(&mut self, pane: &PaneId, pinned: bool) -> Result<()> {
        self.pane_mut(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?
            .pinned = pinned;
        Ok(())
    }

    pub fn set_link(&mut self, pane: &PaneId, link: Option<LinkId>) -> Result<()> {
        self.pane_mut(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?
            .link = link;
        Ok(())
    }

    /// The panes linked with this one (itself excluded).
    #[must_use]
    pub fn linked(&self, pane: &PaneId) -> Vec<&PaneId> {
        let Some(link) = self.pane(pane).and_then(|p| p.link.as_ref()) else {
            return Vec::new();
        };
        self.panes
            .iter()
            .filter(|p| &p.id != pane && p.link.as_ref() == Some(link))
            .map(|p| &p.id)
            .collect()
    }

    pub fn set_stacked(&mut self, group: GroupId, stacked: bool) -> Result<()> {
        self.group_mut(group)
            .ok_or(WorkspaceError::NoSuchGroup(group))?
            .stacked = stacked;
        Ok(())
    }

    pub fn set_chrome(&mut self, group: GroupId, chrome: Chrome) -> Result<()> {
        self.group_mut(group)
            .ok_or(WorkspaceError::NoSuchGroup(group))?
            .chrome = chrome;
        Ok(())
    }

    /// Set a split's shares (a divider dragged, or evened by a double
    /// click). They are normalised to sum to one.
    pub fn set_shares(&mut self, split: SplitId, shares: &[f32]) -> Result<()> {
        let s = self
            .windows
            .iter_mut()
            .find_map(|w| {
                let Window { center, docks, .. } = w;
                center
                    .as_mut()
                    .and_then(|n| n.split_mut(split))
                    .or_else(|| {
                        [&mut docks.left, &mut docks.right, &mut docks.bottom]
                            .into_iter()
                            .find_map(|d| d.root.as_mut()?.split_mut(split))
                    })
            })
            .ok_or(WorkspaceError::NoSuchSplit(split))?;
        for (child, share) in s.children.iter_mut().zip(shares) {
            child.share = *share;
        }
        self.repair();
        Ok(())
    }

    /// Even out a split's shares.
    pub fn even(&mut self, split: SplitId) -> Result<()> {
        let n = self
            .split_len(split)
            .ok_or(WorkspaceError::NoSuchSplit(split))?;
        let shares = vec![1.0; n];
        self.set_shares(split, &shares)
    }

    fn split_len(&mut self, split: SplitId) -> Option<usize> {
        self.windows.iter_mut().find_map(|w| {
            let Window { center, docks, .. } = w;
            center
                .as_mut()
                .and_then(|n| n.split_mut(split))
                .or_else(|| {
                    [&mut docks.left, &mut docks.right, &mut docks.bottom]
                        .into_iter()
                        .find_map(|d| d.root.as_mut()?.split_mut(split))
                })
                .map(|s| s.children.len())
        })
    }

    /// Show, collapse or hide the dock on `side` (wherever it is).
    pub fn set_dock_shown(&mut self, side: Side, shown: Shown) -> Result<()> {
        self.dock_mut(side).ok_or(WorkspaceError::NoDock)?.shown = shown;
        Ok(())
    }

    /// Resize the dock on `side` (wherever it is).
    pub fn set_dock_size(&mut self, side: Side, points: f32) -> Result<()> {
        self.dock_mut(side).ok_or(WorkspaceError::NoDock)?.size = points.max(120.0);
        Ok(())
    }

    /// Pop the main window's dock on `side` out into a window of its own
    /// ([`WindowId::dock`]), with its ribbon, its groups and its size; the
    /// main window's layout closes the gap. Tabs moved to that dock go to
    /// its window while it is out. A dock already out stays where it is.
    pub fn pop_out_dock(&mut self, side: Side, bounds: Option<Rect>) -> Result<WindowId> {
        if let Some(at) = self.dock_window(side) {
            return self
                .windows
                .get(at)
                .map(|w| w.id.clone())
                .ok_or(WorkspaceError::NoDock);
        }
        let id = WindowId::dock(side);
        let has = self
            .windows
            .first()
            .and_then(|w| w.docks.get(side))
            .ok_or(WorkspaceError::NoDock)?
            .root
            .is_some();
        if !has {
            return Err(WorkspaceError::EmptyDock(side));
        }
        // A window of that id left over (a layout edited by hand) gives
        // its tabs back first.
        if self.windows.iter().skip(1).any(|w| w.id == id) {
            self.close_window(&id)?;
        }
        let main = self
            .windows
            .first_mut()
            .ok_or_else(|| WorkspaceError::NoSuchWindow(WindowId::main()))?;
        let dock = main.docks.get_mut(side).ok_or(WorkspaceError::NoDock)?;
        let points = dock.size;
        let mut out = std::mem::replace(dock, Dock::new(points));
        // In a window of its own it is all there is: open.
        out.shown = Shown::Open;
        let mut docks = Docks::default();
        if let Some(d) = docks.get_mut(side) {
            *d = out;
        }
        self.windows.push(Window {
            id: id.clone(),
            bounds,
            maximized: false,
            center: None,
            docks,
            floats: Vec::new(),
            zoomed: None,
        });
        Ok(id)
    }

    /// Put the dock on `side` back in the main window, with its ribbon, its
    /// groups and its size; its window closes. If the main window has
    /// gained a dock there meanwhile, the returning groups go under its.
    pub fn dock_back(&mut self, side: Side) -> Result<()> {
        let at = self
            .dock_window(side)
            .ok_or(WorkspaceError::DockNotOut(side))?;
        self.return_dock(at, side);
        self.repair();
        Ok(())
    }

    /// Move the dock on `side` of the window at `at` into the main
    /// window's.
    fn return_dock(&mut self, at: usize, side: Side) {
        let split = SplitId(self.alloc());
        let Some(coming) = self
            .windows
            .get_mut(at)
            .and_then(|w| w.docks.get_mut(side))
            .map(|d| {
                let points = d.size;
                std::mem::replace(d, Dock::new(points))
            })
        else {
            return;
        };
        let Some(home) = self.windows.first_mut().and_then(|w| w.docks.get_mut(side)) else {
            return;
        };
        let root = match (home.root.take(), coming.root) {
            (Some(here), Some(back)) => Some(Node::Split(Split {
                id: split,
                axis: if side.axis() == Axis::Row {
                    Axis::Column
                } else {
                    Axis::Row
                },
                children: vec![
                    Child {
                        node: here,
                        share: 0.5,
                    },
                    Child {
                        node: back,
                        share: 0.5,
                    },
                ],
            })),
            (here, back) => back.or(here),
        };
        *home = Dock {
            root,
            size: coming.size,
            shown: Shown::Open,
            ribbon: coming.ribbon.or_else(|| home.ribbon.take()),
        };
    }

    /// The float holding `pane`.
    pub(crate) fn float_of(&mut self, pane: &PaneId) -> Result<&mut Float> {
        let loc = self
            .locate(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let Area::Float(f) = loc.area else {
            return Err(WorkspaceError::NotFloating(pane.clone()));
        };
        self.windows
            .get_mut(loc.window)
            .and_then(|w| w.floats.get_mut(f))
            .ok_or_else(|| WorkspaceError::NotFloating(pane.clone()))
    }

    /// Move or resize the float holding `pane`; it unsnaps.
    pub fn set_float_rect(&mut self, pane: &PaneId, rect: Rect) -> Result<()> {
        let float = self.float_of(pane)?;
        float.rect = rect;
        float.snap = None;
        Ok(())
    }

    /// Snap the float holding `pane` (or unsnap it, back to its rect).
    pub fn snap(&mut self, pane: &PaneId, snap: Option<Snap>) -> Result<()> {
        self.float_of(pane)?.snap = snap;
        Ok(())
    }

    /// Pin the float holding `pane` to a corner (or unpin it).
    pub fn pin(&mut self, pane: &PaneId, pin: Option<Pin>) -> Result<()> {
        self.float_of(pane)?.pin = pin;
        Ok(())
    }

    /// Minimise the float holding `pane` to the window's dock of chips.
    pub fn minimise(&mut self, pane: &PaneId) -> Result<()> {
        self.float_of(pane)?.minimised = true;
        if self.active.as_ref() == Some(pane) {
            self.active = self.recent.iter().find(|p| *p != pane).cloned();
        }
        Ok(())
    }

    /// Bring the float holding `pane` back from its chip, to the front.
    pub fn restore(&mut self, pane: &PaneId) -> Result<()> {
        self.float_of(pane)?;
        self.activate(pane)
    }

    /// The pane shows `state` now, keeping the one it showed to go back to.
    pub fn navigate(&mut self, pane: &PaneId, state: String) -> Result<()> {
        let p = self
            .pane_mut(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let old = std::mem::replace(&mut p.state, state);
        p.history.back.push(old);
        p.history.forward.clear();
        Ok(())
    }

    /// Back (or, `forward`, forward) through the pane's history.
    pub fn go(&mut self, pane: &PaneId, forward: bool) -> Result<()> {
        let p = self
            .pane_mut(pane)
            .ok_or_else(|| WorkspaceError::NoSuchPane(pane.clone()))?;
        let (from, to) = if forward {
            (&mut p.history.forward, &mut p.history.back)
        } else {
            (&mut p.history.back, &mut p.history.forward)
        };
        let state = from.pop().ok_or(WorkspaceError::NoHistory)?;
        to.push(std::mem::replace(&mut p.state, state));
        Ok(())
    }

    /// Close the pop-out `window`: its tabs come back to the main window,
    /// into the group with the focus there (closing a window loses no
    /// work; a tab is closed by its ×), and a dock popped out into it
    /// docks back on its side. The main window cannot close.
    pub fn close_window(&mut self, window: &WindowId) -> Result<Vec<PaneId>> {
        let at = self
            .windows
            .iter()
            .position(|w| &w.id == window)
            .filter(|&i| i > 0)
            .ok_or_else(|| WorkspaceError::NoSuchWindow(window.clone()))?;
        let docked: Vec<Side> = Docks::SIDES
            .into_iter()
            .filter(|&s| {
                self.windows
                    .get(at)
                    .and_then(|w| w.docks.get(s))
                    .is_some_and(|d| !d.is_empty())
            })
            .collect();
        for side in docked {
            self.return_dock(at, side);
        }
        let moved = self.window_panes(at);
        let into = self.main_group();
        if at < self.windows.len() {
            self.windows.remove(at);
        }
        if let Some(group) = self.group_mut(into) {
            group.tabs.extend(moved.iter().cloned());
        }
        self.repair();
        if let Some(first) = moved.first() {
            self.activate(first)?;
        }
        Ok(moved)
    }

    /// Every pane in the window at `index`, its center's then its floats'.
    fn window_panes(&self, index: usize) -> Vec<PaneId> {
        let Some(window) = self.windows.get(index) else {
            return Vec::new();
        };
        let mut groups = Vec::new();
        if let Some(center) = &window.center {
            center.groups(&mut groups);
        }
        groups.extend(window.floats.iter().map(|f| &f.group));
        groups.into_iter().flat_map(|g| g.tabs.clone()).collect()
    }

    /// The main window's group a tab coming back goes to: the one with the
    /// focus there, else its first (made when there is none).
    fn main_group(&mut self) -> GroupId {
        let focused = self
            .recent
            .iter()
            .filter_map(|p| self.locate(p))
            .find(|l| l.window == 0 && l.area == Area::Center)
            .map(|l| l.group);
        let first = self
            .windows
            .first()
            .and_then(|w| w.center.as_ref())
            .and_then(Node::first_group)
            .map(|g| g.id);
        if let Some(group) = focused.or(first) {
            return group;
        }
        let group = self.fresh_group(Vec::new());
        let id = group.id;
        if let Some(main) = self.windows.first_mut() {
            main.center = Some(Node::Group(group));
        }
        id
    }

    /// Move a pane back to the main window, into the group with the focus
    /// there (a pop-out's "Move to the main window").
    pub fn to_main_window(&mut self, pane: &PaneId) -> Result<()> {
        if self.locate(pane).is_some_and(|l| l.window == 0) {
            return self.activate(pane);
        }
        let group = self.main_group();
        self.move_pane(pane, Dest::Group { group, index: None })
    }

    /// Where the pop-out `window` is on screen, as the platform last told
    /// (so the layout puts it back there).
    pub fn set_window_bounds(&mut self, window: &WindowId, bounds: Rect) -> Result<()> {
        let w = self
            .windows
            .iter_mut()
            .skip(1)
            .find(|w| &w.id == window)
            .ok_or_else(|| WorkspaceError::NoSuchWindow(window.clone()))?;
        w.bounds = Some(bounds);
        Ok(())
    }

    /// Show the center group `group` alone in its window, filling the
    /// center (`true`), or the whole tree again (`false`). The tree is
    /// kept; a zoomed group that goes, or splits, unzooms.
    pub fn set_zoomed(&mut self, group: GroupId, on: bool) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.center.as_ref().is_some_and(|c| c.group(group).is_some()))
            .ok_or(WorkspaceError::NotInCenter(group))?;
        if on {
            window.zoomed = Some(group);
        } else if window.zoomed == Some(group) {
            window.zoomed = None;
        }
        Ok(())
    }

    /// Serialise (facet JSON).
    pub fn to_json(&self) -> std::result::Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// Read one back, repaired: a layout written by an older or newer app,
    /// or edited by hand, still opens.
    pub fn from_json(json: &str) -> std::result::Result<Self, String> {
        let mut ws: Self = facet_json::from_str(json).map_err(|e| e.to_string())?;
        ws.repair();
        Ok(ws)
    }
}
