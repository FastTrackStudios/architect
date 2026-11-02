//! A layout as a portable tree: `WORKSPACE_EXPORT` writes one,
//! `WORKSPACE_APPLY` makes the workspace match one.
//!
//! The tree is every window's center, docks and floats: splits with their
//! shares, groups with their tabs, each tab its pane's id, view, state and
//! title. No rectangles, no history. Applying builds the whole new
//! workspace first and only then puts it in place, so a tree that does not
//! fit (an unknown view, a pane twice) changes nothing. Panes keep their
//! ids, so their views are not built again; groups and splits keep the
//! ids the tree carries.

// The facet derive's generated layout code for a data enum trips the
// arithmetic lint; the allowance stays in this module. Every action takes
// its arguments by value: one signature for all (`super::Write`).
#![allow(clippy::arithmetic_side_effects, clippy::needless_pass_by_value)]

use std::collections::HashSet;

use facet::Facet;

use super::{Acted, Answer, DockShown, DockSide, Env, Focus, Reason, State, TabChrome, finish};
use crate::model::{
    Axis, Child, Chrome, ClosedPane, Dock, Docks, Float, Group, GroupId, Node, Pane, PaneId, Rect,
    Shown, Side, Split, SplitId, Window, WindowId, Workspace,
};

/// The way a split lays out its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum SplitAxis {
    /// Side by side.
    Row,
    /// One above another.
    Column,
}

/// A whole layout.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PortableLayout {
    /// The first is the main window (`"main"`).
    pub windows: Vec<PortableWindow>,
    /// The pane to make active; unset, the active pane stays (when it is
    /// still placed).
    #[facet(default)]
    pub active: Option<String>,
}

/// One window.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PortableWindow {
    pub id: String,
    /// Screen points; the main window's is the platform's.
    #[facet(default)]
    pub bounds: Option<Rect>,
    #[facet(default)]
    pub maximized: bool,
    #[facet(default)]
    pub center: Option<PortableNode>,
    #[facet(default)]
    pub docks: Vec<PortableDock>,
    #[facet(default)]
    pub floats: Vec<PortableFloat>,
}

/// A split, or a group.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum PortableNode {
    Split {
        /// Kept when given and not used twice; else a fresh one.
        #[facet(default)]
        id: Option<u64>,
        axis: SplitAxis,
        children: Vec<PortableChild>,
    },
    Group(PortableGroup),
}

/// A split's child and its share of the split's length (shares are
/// normalised).
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PortableChild {
    pub share: f32,
    pub node: PortableNode,
}

/// A tab group.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PortableGroup {
    /// Kept when given and not used twice; else a fresh one.
    #[facet(default)]
    pub id: Option<u64>,
    pub tabs: Vec<PortableTab>,
    /// The tab on show; the first when unset.
    #[facet(default)]
    pub current: Option<String>,
    #[facet(default)]
    pub stacked: bool,
    #[facet(default)]
    pub chrome: Option<TabChrome>,
}

/// One tab: an open pane by id, or a new one (with its `view`).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PortableTab {
    pub pane: String,
    /// Needed for a pane that is not open; an open pane keeps its view.
    #[facet(default)]
    pub view: Option<String>,
    #[facet(default)]
    pub state: Option<String>,
    #[facet(default)]
    pub title: Option<String>,
    #[facet(default)]
    pub pinned: bool,
}

/// A dock: its groups, size and how much shows. Its ribbon is the
/// person's and stays as it is.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PortableDock {
    pub side: DockSide,
    #[facet(default)]
    pub size: Option<f32>,
    #[facet(default)]
    pub shown: Option<DockShown>,
    #[facet(default)]
    pub root: Option<PortableNode>,
}

/// A float.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PortableFloat {
    pub group: PortableGroup,
    pub rect: Rect,
    #[facet(default)]
    pub minimised: bool,
}

/// `WORKSPACE_APPLY`.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ApplyArgs {
    pub layout: PortableLayout,
    /// Open panes the layout does not place: kept as tabs of the main
    /// window's first group (`true`, the default), or closed, kept to
    /// reopen (`false`).
    #[facet(default = true)]
    pub keep_unplaced: bool,
}

/// `WORKSPACE_EXPORT`.
///
/// # Errors
///
/// Never.
pub fn export(st: &State, _env: &Env, (): ()) -> Result<PortableLayout, String> {
    let w = &st.workspace;
    let windows = w
        .windows
        .iter()
        .map(|win| PortableWindow {
            id: win.id.0.clone(),
            bounds: win.bounds,
            maximized: win.maximized,
            center: win.center.as_ref().map(|n| node_out(w, n)),
            docks: [
                (DockSide::Left, &win.docks.left),
                (DockSide::Right, &win.docks.right),
                (DockSide::Bottom, &win.docks.bottom),
            ]
            .into_iter()
            .filter(|(_, d)| d.root.is_some())
            .map(|(side, d)| PortableDock {
                side,
                size: Some(d.size),
                shown: Some(match d.shown {
                    Shown::Open => DockShown::Open,
                    Shown::Collapsed => DockShown::Collapsed,
                    Shown::Hidden => DockShown::Hidden,
                }),
                root: d.root.as_ref().map(|n| node_out(w, n)),
            })
            .collect(),
            floats: win
                .floats
                .iter()
                .map(|f| PortableFloat {
                    group: group_out(w, &f.group),
                    rect: f.rect,
                    minimised: f.minimised,
                })
                .collect(),
        })
        .collect();
    Ok(PortableLayout {
        windows,
        active: w.active.as_ref().map(|p| p.0.clone()),
    })
}

fn node_out(w: &Workspace, node: &Node) -> PortableNode {
    match node {
        Node::Group(g) => PortableNode::Group(group_out(w, g)),
        Node::Split(s) => PortableNode::Split {
            id: Some(s.id.0),
            axis: match s.axis {
                Axis::Row => SplitAxis::Row,
                Axis::Column => SplitAxis::Column,
            },
            children: s
                .children
                .iter()
                .map(|c| PortableChild {
                    share: c.share,
                    node: node_out(w, &c.node),
                })
                .collect(),
        },
    }
}

fn group_out(w: &Workspace, g: &Group) -> PortableGroup {
    PortableGroup {
        id: Some(g.id.0),
        tabs: g
            .tabs
            .iter()
            .filter_map(|t| w.pane(t))
            .map(|p| PortableTab {
                pane: p.id.0.clone(),
                view: Some(p.view.0.clone()),
                state: Some(p.state.clone()),
                title: Some(p.title.clone()),
                pinned: p.pinned,
            })
            .collect(),
        current: g.current_tab().map(|p| p.0.clone()),
        stacked: g.stacked,
        chrome: Some(match g.chrome {
            Chrome::Tabs => TabChrome::Tabs,
            Chrome::Icons => TabChrome::Icons,
            Chrome::Bare => TabChrome::Bare,
        }),
    }
}

/// Builds a workspace from a portable layout, over the one there is.
struct Build<'a> {
    old: &'a Workspace,
    env: &'a Env,
    next: u64,
    panes: Vec<Pane>,
    placed: HashSet<PaneId>,
    groups: HashSet<u64>,
    splits: HashSet<u64>,
}

impl Build<'_> {
    const fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }

    /// A tree id: the one given when it is free, else a fresh one.
    fn id(&mut self, given: Option<u64>, split: bool) -> u64 {
        let used = if split { &self.splits } else { &self.groups };
        let other = if split { &self.groups } else { &self.splits };
        let id = given
            .filter(|id| *id > 0 && !used.contains(id) && !other.contains(id))
            .unwrap_or_else(|| self.fresh());
        if split {
            self.splits.insert(id);
        } else {
            self.groups.insert(id);
        }
        id
    }

    fn tab(&mut self, t: &PortableTab) -> Result<PaneId, String> {
        let id = PaneId::new(t.pane.clone());
        if !self.placed.insert(id.clone()) {
            return Err(format!("{:?} is placed twice", t.pane));
        }
        let mut pane = if let Some(open) = self.old.pane(&id) {
            if let Some(view) = t.view.as_ref().filter(|v| **v != open.view.0) {
                return Err(format!(
                    "{:?} shows {:?}, not {view:?}: give a new pane id for another view",
                    t.pane, open.view.0
                ));
            }
            open.clone()
        } else {
            let view = t
                .view
                .as_ref()
                .ok_or_else(|| format!("{:?} is not open: give its `view`", t.pane))?;
            let spec = self
                .env
                .view(view)
                .ok_or_else(|| format!("no view {view:?} (WORKSPACE_VIEWS lists them)"))?;
            Pane::new(t.pane.clone(), view.clone(), spec.title.clone())
        };
        if let Some(state) = &t.state {
            pane.state.clone_from(state);
        }
        if let Some(title) = &t.title {
            pane.title.clone_from(title);
        }
        pane.pinned = t.pinned;
        self.panes.push(pane);
        Ok(id)
    }

    fn group(&mut self, g: &PortableGroup) -> Result<Group, String> {
        let mut group = Group::new(GroupId(self.id(g.id, false)));
        for t in &g.tabs {
            let id = self.tab(t)?;
            group.tabs.push(id);
        }
        group.current = g
            .current
            .as_ref()
            .and_then(|c| group.tabs.iter().position(|t| t.0 == *c))
            .unwrap_or(0);
        group.stacked = g.stacked;
        group.chrome = g.chrome.map_or(Chrome::Tabs, Into::into);
        Ok(group)
    }

    fn node(&mut self, n: &PortableNode) -> Result<Node, String> {
        Ok(match n {
            PortableNode::Group(g) => Node::Group(self.group(g)?),
            PortableNode::Split { id, axis, children } => {
                let id = SplitId(self.id(*id, true));
                let children = children
                    .iter()
                    .map(|c| {
                        Ok(Child {
                            node: self.node(&c.node)?,
                            share: c.share,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Node::Split(Split {
                    id,
                    axis: match axis {
                        SplitAxis::Row => Axis::Row,
                        SplitAxis::Column => Axis::Column,
                    },
                    children,
                })
            }
        })
    }

    fn window(&mut self, p: &PortableWindow, main: bool) -> Result<Window, String> {
        let id = if main {
            WindowId::main()
        } else {
            WindowId(p.id.clone())
        };
        // The ribbons are the person's: they stay with the window that has
        // them now.
        let kept = self.old.window(&id).map(|w| w.docks.clone());
        let mut docks = Docks::default();
        for side in Docks::SIDES {
            if let (Some(d), Some(k)) =
                (docks.get_mut(side), kept.as_ref().and_then(|k| k.get(side)))
            {
                *d = Dock {
                    root: None,
                    ..k.clone()
                };
            }
        }
        for pd in &p.docks {
            let side: Side = pd.side.into();
            let root = pd.root.as_ref().map(|n| self.node(n)).transpose()?;
            if let Some(d) = docks.get_mut(side) {
                d.root = root;
                if let Some(size) = pd.size {
                    d.size = size;
                }
                if let Some(shown) = pd.shown {
                    d.shown = shown.into();
                }
            }
        }
        let center = p.center.as_ref().map(|n| self.node(n)).transpose()?;
        let floats = p
            .floats
            .iter()
            .map(|f| {
                let group = self.group(&f.group)?;
                // A float kept by its group keeps its snap and pin.
                let was = self
                    .old
                    .windows
                    .iter()
                    .flat_map(|w| &w.floats)
                    .find(|of| of.group.id == group.id);
                Ok(Float {
                    rect: f.rect,
                    snap: was.and_then(|w| w.snap),
                    pin: was.and_then(|w| w.pin),
                    minimised: f.minimised,
                    group,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Window {
            bounds: if main { None } else { p.bounds },
            maximized: p.maximized,
            center: if main {
                Some(center.unwrap_or_else(|| Node::Group(Group::new(GroupId(self.fresh())))))
            } else {
                center
            },
            docks,
            floats,
            zoomed: self.old.window(&id).and_then(|w| w.zoomed),
            id,
        })
    }
}

/// `WORKSPACE_APPLY`.
///
/// # Errors
///
/// A layout that does not fit: no windows, a pane placed twice, a new
/// pane without a known view, an open pane given another view, an active
/// pane it does not place. Then nothing changes.
pub fn apply(st: &mut State, env: &Env, a: ApplyArgs) -> Result<Answer, String> {
    let before = st.clone();
    let old = &st.workspace;
    if a.layout.windows.is_empty() {
        return Err("a layout has at least its main window".to_owned());
    }
    let mut ids = HashSet::new();
    if a.layout.windows.iter().any(|w| !ids.insert(w.id.clone())) {
        return Err("two windows have one id".to_owned());
    }
    let mut build = Build {
        old,
        env,
        next: old.next_id,
        panes: Vec::new(),
        placed: HashSet::new(),
        groups: HashSet::new(),
        splits: HashSet::new(),
    };
    let mut windows = Vec::new();
    for (i, p) in a.layout.windows.iter().enumerate() {
        windows.push(build.window(p, i == 0)?);
    }
    let active = match &a.layout.active {
        Some(a) if !build.placed.contains(&PaneId::new(a.clone())) => {
            return Err(format!("the active pane {a:?} is not in the layout"));
        }
        Some(a) => Some(PaneId::new(a.clone())),
        None => None,
    };
    let unplaced: Vec<Pane> = old
        .panes
        .iter()
        .filter(|p| !build.placed.contains(&p.id))
        .cloned()
        .collect();
    let mut closed = old.closed.clone();
    let Build {
        mut panes, next, ..
    } = build;
    if a.keep_unplaced {
        let first = windows
            .first_mut()
            .and_then(|w| w.center.as_mut())
            .and_then(|c| {
                let mut groups = Vec::new();
                c.groups(&mut groups);
                let id = groups.first().map(|g| g.id)?;
                c.group_mut(id)
            });
        if let Some(group) = first {
            group.tabs.extend(unplaced.iter().map(|p| p.id.clone()));
        }
        panes.extend(unplaced);
    } else {
        for pane in unplaced {
            let at = old.locate(&pane.id);
            closed.push(ClosedPane {
                pane,
                group: at.map_or(GroupId(0), |l| l.group),
                index: at.map_or(0, |l| l.index),
            });
        }
        let extra = closed.len().saturating_sub(Workspace::CLOSED_KEPT);
        closed.drain(..extra);
    }
    // Open panes keep their order; new ones follow, in the layout's.
    panes.sort_by_key(|p| {
        old.panes
            .iter()
            .position(|o| o.id == p.id)
            .unwrap_or(usize::MAX)
    });
    let mut made = Workspace {
        windows,
        panes,
        active: active.clone().or_else(|| old.active.clone()),
        recent: old.recent.clone(),
        closed,
        next_id: next.max(old.next_id),
        aliases: old.aliases.clone(),
    };
    made.repair();
    st.workspace = made;
    let focus = active.map_or(Focus::Keep, Focus::Take);
    Ok(finish(
        st,
        &before,
        focus,
        Acted::default(),
        Reason::Unchanged,
    ))
}
