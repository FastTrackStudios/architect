//! The workspace as actions: every operation with an explicit target, for
//! a key, a menu, a script or a model to run the same way.
//!
//! Each action is a function over a [`State`] (the workspace, the
//! screensets and the class of screen) and an [`Env`] (what the app
//! offers: its views, whether it opens windows): no Dioxus, no I/O. The
//! host's commands call them with the active pane as the target
//! (`features/workspace/src/host/commands.rs`); feature `actions` puts
//! them on an Action List (`register`), behind the Blitz host or a
//! [`HeadlessWorkspace`] with no window at all.
//!
//! The rules every write keeps:
//!
//! - **Targets are named.** A pane, a group, a window or a dock side is
//!   always a parameter; nothing acts on "the active pane" unless its
//!   caller passed that pane.
//! - **Focus stays put.** The person's active pane moves only when a call
//!   says `focus: true` (or is `WORKSPACE_FOCUS`). A pane opened or moved
//!   without it shows in its group, but the keyboard stays where it was.
//! - **Every write answers [`Answer`]** — `changed`, a [`Reason`] when it
//!   did not, what it acted on and the layout after — so a no-op is
//!   explicit and a retry is safe. A call that names nothing there (an
//!   unknown pane) fails, and changes nothing.
//! - **Ids are opaque and never handed out twice.** A group or window
//!   that goes because its panes went elsewhere leaves an alias
//!   ([`crate::Aliases`]): an id read before still finds them.

mod args;
pub mod deck;
#[cfg(feature = "actions")]
mod list;
mod portable;
mod read;
mod write;

pub use args::*;

#[cfg(feature = "actions")]
pub use list::{Exec, HeadlessWorkspace, Job, ids, register};
pub use portable::{
    ApplyArgs, PortableChild, PortableDock, PortableFloat, PortableGroup, PortableLayout,
    PortableNode, PortableTab, PortableWindow, SplitAxis, apply, export,
};
pub use read::{
    ClassName, NeighborInfo, PaneInfo, PlaceName, ScreensetInfo, ScreensetsAnswer, ViewsAnswer,
    layout, neighbor, pane_get, screensets, views,
};
pub use write::*;

use std::collections::BTreeMap;

use facet::Facet;

use crate::guide::Arrangement;
use crate::layout::{Direction, area_rects, neighbour};
use crate::model::{
    Docks, GroupAlias, GroupId, Node, PaneId, Rect, Shown, Side, WindowAlias, WindowId, Workspace,
};
use crate::screenset::{FormClass, Screensets};

/// What the workspace actions change: the workspace, the screensets kept
/// beside it, and the class of screen layouts are recalled for.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub workspace: Workspace,
    pub screensets: Screensets,
    pub form: FormClass,
    /// The arrangement's revision: up by one with each change of it
    /// ([`crate::Arrangement::revision`]).
    pub revision: u64,
    /// Who the write under way is for: `app` (a key, a gesture, a
    /// command) or a caller's principal (`service:claude`). What a change
    /// event names as its actor.
    pub actor: String,
}

impl State {
    #[must_use]
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace,
            screensets: Screensets::default(),
            form: FormClass::Desk,
            revision: 0,
            actor: APP.to_owned(),
        }
    }

    #[must_use]
    pub fn with_screensets(mut self, screensets: Screensets) -> Self {
        self.screensets = screensets;
        self
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new(Workspace::new())
    }
}

/// A view the app offers, as an action sees it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ViewSpec {
    /// What `WORKSPACE_OPEN` takes as `view`.
    pub kind: String,
    pub title: String,
    /// At most one open: its pane's id is its kind.
    pub single: bool,
}

impl ViewSpec {
    pub fn new(kind: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            single: true,
        }
    }

    /// More than one may be open.
    #[must_use]
    pub const fn many(mut self) -> Self {
        self.single = false;
        self
    }
}

/// What the app offers the actions: its views, whether pop-outs are real
/// OS windows here, and where a dock popped out goes when no place is
/// given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Env {
    pub views: Vec<ViewSpec>,
    /// What the host measured: windows' areas, groups' rectangles.
    pub measured: crate::tree::Measured,
    /// Pop-outs open as OS windows (`WORKSPACE_DOCK_POP_OUT` needs them).
    pub windows: bool,
    pub dock_bounds: BTreeMap<u8, Rect>,
    /// The screensets the app ships, as it ships them: what
    /// `WORKSPACES_RESTORE_BUILTINS` (the escape hatch) puts back.
    pub shipped: Vec<crate::screenset::Screenset>,
}

impl Env {
    #[must_use]
    pub const fn new(views: Vec<ViewSpec>) -> Self {
        Self {
            views,
            measured: crate::tree::Measured {
                areas: BTreeMap::new(),
                groups: BTreeMap::new(),
            },
            windows: false,
            dock_bounds: BTreeMap::new(),
            shipped: Vec::new(),
        }
    }

    /// The screensets the app ships ([`Self::shipped`]).
    #[must_use]
    pub fn with_shipped(mut self, shipped: Vec<crate::screenset::Screenset>) -> Self {
        self.shipped = shipped;
        self
    }

    /// Pop-outs are OS windows here.
    #[must_use]
    pub const fn with_windows(mut self) -> Self {
        self.windows = true;
        self
    }

    #[must_use]
    pub fn view(&self, kind: &str) -> Option<&ViewSpec> {
        self.views.iter().find(|v| v.kind == kind)
    }

    /// Where the dock on `side` pops out to when no place is given.
    pub fn set_dock_bounds(&mut self, side: Side, bounds: Rect) {
        self.dock_bounds.insert(side_key(side), bounds);
    }

    #[must_use]
    pub fn dock_bounds(&self, side: Side) -> Option<Rect> {
        self.dock_bounds.get(&side_key(side)).copied()
    }
}

const fn side_key(side: Side) -> u8 {
    match side {
        Side::Left => 0,
        Side::Right => 1,
        Side::Top => 2,
        Side::Bottom => 3,
    }
}

/// The actor of a change the app made itself (a key, a gesture, a
/// command).
pub const APP: &str = "app";

/// Where a pane floated without a rectangle goes, window points.
pub const FLOAT_AT: Rect = Rect {
    x: 96.0,
    y: 72.0,
    w: 480.0,
    h: 320.0,
};

/// What becomes of the person's focus after a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Focus {
    /// The active pane (and what each group shows of it) as it was.
    Keep,
    /// This pane becomes the active one.
    Take(PaneId),
    /// As the operation left it (it was about focus).
    AsIs,
}

impl Focus {
    pub(crate) fn of(focus: bool, pane: &PaneId) -> Self {
        if focus {
            Self::Take(pane.clone())
        } else {
            Self::Keep
        }
    }
}

/// What a write acted on, for its [`Answer`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Acted {
    pub pane: Option<PaneId>,
    pub group: Option<GroupId>,
    pub window: Option<WindowId>,
    pub screenset: Option<String>,
}

impl Acted {
    pub(crate) fn pane(pane: &PaneId) -> Self {
        Self {
            pane: Some(pane.clone()),
            ..Self::default()
        }
    }
}

/// Settles a write: the focus as `focus` says, aliases for what went, and
/// the answer — `same` the reason when nothing changed.
pub(crate) fn finish(
    st: &mut State,
    env: &Env,
    before: &State,
    focus: Focus,
    acted: Acted,
    same: Reason,
) -> Answer {
    match focus {
        Focus::Keep => keep_focus(&mut st.workspace, &before.workspace),
        Focus::Take(pane) => {
            if st.workspace.pane(&pane).is_some() {
                let _ = st.workspace.activate(&pane);
            }
        }
        Focus::AsIs => {}
    }
    note_aliases(&before.workspace, &mut st.workspace);
    let changed = st != before;
    if rearranged(&before.workspace, &st.workspace) {
        st.revision = before.revision.saturating_add(1);
    }
    answer(st, env, changed, (!changed).then_some(same), acted)
}

/// Whether the arrangement (what [`crate::Arrangement`] shows, not a
/// view's own state) differs: what moves the revision on.
#[must_use]
pub fn rearranged(before: &Workspace, after: &Workspace) -> bool {
    Arrangement::of(before) != Arrangement::of(after)
}

/// An answer for `st` as it is.
pub(crate) fn answer(
    st: &State,
    env: &Env,
    changed: bool,
    reason: Option<Reason>,
    acted: Acted,
) -> Answer {
    let w = &st.workspace;
    let at = acted.pane.as_ref().and_then(|p| w.locate(p));
    Answer {
        changed,
        reason,
        pane: acted.pane.as_ref().map(|p| p.0.clone()),
        group: acted.group.or_else(|| at.map(|l| l.group)).map(|g| g.0),
        window: acted
            .window
            .or_else(|| at.and_then(|l| w.windows.get(l.window).map(|win| win.id.clone())))
            .map(|win| win.0),
        screenset: acted.screenset,
        revision: st.revision,
        layout: Arrangement::measured(w, st.revision, &env.measured),
    }
}

/// Puts the person's focus back as it was before a write: the active pane
/// (shown in its group again) and the order of recent panes, with any new
/// ones after them.
fn keep_focus(w: &mut Workspace, before: &Workspace) {
    match before.active.clone() {
        Some(active) if w.pane(&active).is_some() => {
            if let Some(l) = w.locate(&active) {
                if let Some(g) = w.group_mut(l.group) {
                    g.current = l.index;
                }
            }
            w.active = Some(active);
        }
        // Closed by the write: the model's choice stands.
        Some(_) => {}
        None => w.active = None,
    }
    let mut recent: Vec<PaneId> = before
        .recent
        .iter()
        .filter(|p| w.pane(p).is_some())
        .cloned()
        .collect();
    for p in &w.recent {
        if !recent.contains(p) {
            recent.push(p.clone());
        }
    }
    w.recent = recent;
}

/// Each group and window of `before` that is gone from `after` while all
/// its panes still open went to one other: an alias to it.
fn note_aliases(before: &Workspace, after: &mut Workspace) {
    for (id, tabs) in groups_of(before) {
        if after.group(id).is_some() {
            continue;
        }
        let to = one_place(tabs.iter().filter_map(|p| after.locate(p)).map(|l| l.group));
        if let Some(to) = to {
            after.aliases.groups.push(GroupAlias { from: id, to });
        }
    }
    for (index, window) in before.windows.iter().enumerate() {
        if after.window(&window.id).is_some() {
            continue;
        }
        let panes: Vec<PaneId> = groups_in_window(before, index)
            .into_iter()
            .flat_map(|(_, tabs)| tabs)
            .collect();
        let to = one_place(
            panes
                .iter()
                .filter_map(|p| after.locate(p))
                .filter_map(|l| after.windows.get(l.window).map(|w| w.id.clone())),
        );
        if let Some(to) = to {
            after.aliases.windows.push(WindowAlias {
                from: window.id.clone(),
                to,
            });
        }
    }
    let kept = crate::model::Aliases::KEPT;
    let groups = after.aliases.groups.len().saturating_sub(kept);
    after.aliases.groups.drain(..groups);
    let windows = after.aliases.windows.len().saturating_sub(kept);
    after.aliases.windows.drain(..windows);
}

/// The one value every item is, when there is at least one and they agree.
fn one_place<T: PartialEq>(mut items: impl Iterator<Item = T>) -> Option<T> {
    let first = items.next()?;
    items.all(|i| i == first).then_some(first)
}

/// Every group in the workspace with its tabs: centers, docks and floats.
pub(crate) fn groups_of(w: &Workspace) -> Vec<(GroupId, Vec<PaneId>)> {
    (0..w.windows.len())
        .flat_map(|i| groups_in_window(w, i))
        .collect()
}

/// The ids of every group in the window at `index`.
#[must_use]
pub fn groups_in(w: &Workspace, index: usize) -> Vec<u64> {
    groups_in_window(w, index)
        .into_iter()
        .map(|(g, _)| g.0)
        .collect()
}

/// Every group in the window at `index`, with its tabs.
fn groups_in_window(w: &Workspace, index: usize) -> Vec<(GroupId, Vec<PaneId>)> {
    let Some(window) = w.windows.get(index) else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    if let Some(center) = &window.center {
        center.groups(&mut groups);
    }
    for side in Docks::SIDES {
        if let Some(root) = window.docks.get(side).and_then(|d| d.root.as_ref()) {
            root.groups(&mut groups);
        }
    }
    groups.extend(window.floats.iter().map(|f| &f.group));
    groups.into_iter().map(|g| (g.id, g.tabs.clone())).collect()
}

/// The pane `id`, if it is open.
pub(crate) fn pane_id(w: &Workspace, id: &str) -> Result<PaneId, String> {
    let pane = PaneId::new(id);
    if w.pane(&pane).is_some() {
        Ok(pane)
    } else {
        Err(format!("no pane {id:?} is open"))
    }
}

/// The group `id` names now (following an alias).
pub(crate) fn group_id(w: &Workspace, id: u64) -> Result<GroupId, String> {
    w.resolve_group(GroupId(id))
        .ok_or_else(|| format!("no group {id}"))
}

/// The window `id` names now (following an alias).
pub(crate) fn window_id(w: &Workspace, id: &str) -> Result<WindowId, String> {
    w.resolve_window(&WindowId(id.to_owned()))
        .ok_or_else(|| format!("no window {id:?}"))
}

/// The tree a pane's group is laid out in: its window's center, or its
/// dock's; `None` for a float.
pub(crate) fn area_root<'a>(
    w: &'a Workspace,
    pane: &PaneId,
) -> Option<(usize, Option<Side>, &'a Node)> {
    let l = w.locate(pane)?;
    let window = w.windows.get(l.window)?;
    match l.area {
        crate::model::Area::Center => window.center.as_ref().map(|n| (l.window, None, n)),
        crate::model::Area::Dock(side) => window
            .docks
            .get(side)
            .and_then(|d| d.root.as_ref())
            .map(|n| (l.window, Some(side), n)),
        crate::model::Area::Float(_) => None,
    }
}

/// The group next to `pane`'s toward `direction`, in the tree its group is
/// laid out in (a float has none).
pub(crate) fn neighbour_group(
    w: &Workspace,
    pane: &PaneId,
    direction: Direction,
) -> Result<Option<GroupId>, String> {
    let l = w
        .locate(pane)
        .ok_or_else(|| format!("{:?} is in no group", pane.0))?;
    let Some((_, _, root)) = area_root(w, pane) else {
        return Ok(None);
    };
    let placed = area_rects(root, Rect::new(0.0, 0.0, 1000.0, 1000.0));
    Ok(neighbour(&placed, l.group, direction))
}

/// The topic of a change to the saved layouts (load, save, rename, slot,
/// options, delete): its detail is the active set's id (empty for none).
pub const SETS_TOPIC: &str = "workspaces";

/// The detail of a [`SETS_TOPIC`] event, when the saved layouts (not the
/// working copy) differ.
#[must_use]
pub fn sets_changed(before: &Screensets, after: &Screensets) -> Option<String> {
    (before.saved() != after.saved()).then(|| after.active.clone().unwrap_or_default())
}

/// What a `Changed { topic: "workspace" }` event's detail says.
///
/// As JSON: which revision the layout is at now, who changed it, what kinds of
/// change, and which panes. A watcher treats it as invalidation: a
/// revision that skips one (events dropped while it was away) means read
/// `WORKSPACE_LAYOUT` again.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Change {
    pub revision: u64,
    /// `app`, or the caller's principal (`service:claude`).
    pub actor: String,
    /// Some of `opened`, `closed`, `moved`, `focus`, `shown`, `resized`,
    /// `groups`, `tabs`, `docks`, `floats`, `windows`, `zoom`.
    pub kinds: Vec<String>,
    /// The panes opened, closed or moved.
    pub panes: Vec<String>,
}

impl Change {
    /// What changed from `before` to `st`'s workspace.
    #[must_use]
    pub fn between(before: &Workspace, st: &State) -> Self {
        let after = &st.workspace;
        let mut kinds: Vec<&str> = Vec::new();
        let mut panes: Vec<String> = Vec::new();
        let note = |p: &PaneId, panes: &mut Vec<String>| {
            if !panes.contains(&p.0) {
                panes.push(p.0.clone());
            }
        };
        let opened: Vec<&PaneId> = after
            .panes
            .iter()
            .map(|p| &p.id)
            .filter(|id| before.pane(id).is_none())
            .collect();
        let closed: Vec<&PaneId> = before
            .panes
            .iter()
            .map(|p| &p.id)
            .filter(|id| after.pane(id).is_none())
            .collect();
        let place = |w: &Workspace, p: &PaneId| {
            w.locate(p)
                .map(|l| (l.window, format!("{:?}", l.area), l.group))
        };
        // Its order among the tabs its group had before and has still: a
        // tab opened or closed beside it does not move it.
        let order = |w: &Workspace, other: &Workspace, p: &PaneId| {
            let group = w.locate(p).and_then(|l| w.group(l.group))?;
            group
                .tabs
                .iter()
                .filter(|t| other.locate(t).map(|l| l.group) == Some(group.id) || *t == p)
                .position(|t| t == p)
        };
        let moved: Vec<&PaneId> = after
            .panes
            .iter()
            .map(|p| &p.id)
            .filter(|id| {
                let was = order(before, after, id);
                let is = order(after, before, id);
                before.pane(id).is_some() && (place(before, id) != place(after, id) || was != is)
            })
            .collect();
        for (kind, list) in [("opened", &opened), ("closed", &closed), ("moved", &moved)] {
            if !list.is_empty() {
                kinds.push(kind);
                for p in list.as_slice() {
                    note(p, &mut panes);
                }
            }
        }
        if before.active != after.active {
            kinds.push("focus");
        }
        let a = Arrangement::of(before);
        let b = Arrangement::of(after);
        let current = |x: &Arrangement| -> Vec<(u64, String)> {
            x.windows
                .iter()
                .flat_map(|w| w.groups.iter().chain(&w.docked))
                .map(|g| (g.id, g.current.clone()))
                .collect()
        };
        if current(&a) != current(&b) {
            kinds.push("shown");
        }
        let shares = |x: &Arrangement| -> Vec<Vec<f32>> {
            x.windows
                .iter()
                .flat_map(|w| w.splits.iter().map(|s| s.shares.clone()))
                .collect()
        };
        let sizes = |w: &Workspace| -> Vec<f32> {
            w.windows
                .iter()
                .flat_map(|win| {
                    Docks::SIDES
                        .iter()
                        .filter_map(|&s| win.docks.get(s))
                        .map(|d| d.size)
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        if shares(&a) != shares(&b) || sizes(before) != sizes(after) {
            kinds.push("resized");
        }
        let groups = |x: &Arrangement| -> Vec<(u64, bool)> {
            x.windows
                .iter()
                .flat_map(|w| w.groups.iter().chain(&w.docked))
                .map(|g| (g.id, g.stacked))
                .collect()
        };
        let chrome = |w: &Workspace| -> Vec<String> {
            groups_of(w)
                .iter()
                .filter_map(|(g, _)| w.group(*g).map(|g| format!("{:?}", g.chrome)))
                .collect()
        };
        if groups(&a) != groups(&b) || chrome(before) != chrome(after) {
            kinds.push("groups");
        }
        let tabs = |w: &Workspace| -> Vec<(String, String, bool, Option<String>)> {
            w.panes
                .iter()
                .map(|p| {
                    (
                        p.id.0.clone(),
                        p.title.clone(),
                        p.pinned,
                        p.link.as_ref().map(|l| l.0.clone()),
                    )
                })
                .collect()
        };
        if tabs(before) != tabs(after) {
            kinds.push("tabs");
        }
        let docks = |w: &Workspace| -> Vec<(Shown, Option<usize>)> {
            Docks::SIDES
                .iter()
                .map(|&s| {
                    (
                        w.dock(s).map_or(Shown::Hidden, |d| d.shown),
                        w.dock_window(s),
                    )
                })
                .collect()
        };
        if docks(before) != docks(after) {
            kinds.push("docks");
        }
        let floats = |w: &Workspace| -> Vec<String> {
            w.windows
                .iter()
                .flat_map(|win| &win.floats)
                .map(|f| {
                    format!(
                        "{:?}{:?}{:?}{:?}{}",
                        f.group.id, f.rect, f.snap, f.pin, f.minimised
                    )
                })
                .collect()
        };
        if floats(before) != floats(after) {
            kinds.push("floats");
        }
        let windows = |w: &Workspace| -> Vec<(WindowId, Option<Rect>, bool)> {
            w.windows
                .iter()
                .map(|win| (win.id.clone(), win.bounds, win.maximized))
                .collect()
        };
        if windows(before) != windows(after) {
            kinds.push("windows");
        }
        let zoom = |w: &Workspace| -> Vec<Option<GroupId>> {
            w.windows.iter().map(|win| win.zoomed).collect()
        };
        if zoom(before) != zoom(after) {
            kinds.push("zoom");
        }
        Self {
            revision: st.revision,
            actor: st.actor.clone(),
            kinds: kinds.into_iter().map(str::to_owned).collect(),
            panes,
        }
    }

    /// As JSON, an event's detail.
    #[must_use]
    pub fn to_json(&self) -> String {
        facet_json::to_string(self).unwrap_or_default()
    }
}
