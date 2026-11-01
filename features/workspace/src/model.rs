//! The workspace's data: windows, their trees of splits and tab groups,
//! docks, floats, and the panes in them.

use facet::Facet;

/// A pane's id: stable and addressable.
///
/// The one its maker gave it (a script's `"timer"`, a plugin window's
/// `"fx:<track>/<slot>"`), or its view and subject (`"studio.song:<id>"`).
/// Opening an id that is open brings that pane forward instead of making
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct PaneId(pub String);

impl PaneId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// What shows a pane: a view the app (or an extension) registered
/// (`"session.mixer"`). A kind the running app does not know keeps its
/// pane — as a placeholder — rather than losing the arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct ViewKind(pub String);

impl ViewKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }
}

/// A window's id: `"main"`, or the one its maker gave it. Carried to the
/// platform, so a window manager can place it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct WindowId(pub String);

impl WindowId {
    /// The app's main window.
    #[must_use]
    pub fn main() -> Self {
        Self("main".to_owned())
    }
}

/// Panes sharing a link follow each other (open a song in one, its mixer
/// in a linked pane follows).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct LinkId(pub String);

/// A tab group's id, unique within its workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct GroupId(pub u64);

/// A split's id, unique within its workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct SplitId(pub u64);

/// Which way a split lays its children out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Axis {
    /// Side by side.
    Row,
    /// One above another.
    Column,
}

/// A side of something: of a group (where a split goes), of a window
/// (where a dock is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
#[repr(u8)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    /// The axis a split on this side runs along.
    #[must_use]
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Row,
            Self::Top | Self::Bottom => Axis::Column,
        }
    }

    /// Whether something on this side comes first along its axis.
    #[must_use]
    pub const fn leads(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

/// How much of a dock shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum Shown {
    #[default]
    Open,
    /// Down to its ribbon (or its tabs' icons).
    Collapsed,
    Hidden,
}

/// How a group shows its tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum Chrome {
    /// A tab bar of names.
    #[default]
    Tabs,
    /// A row of icons (a dock is narrow).
    Icons,
    /// No tab bar (a full-bleed view); arrange mode still finds it.
    Bare,
}

/// A corner of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// What a pinned float stays above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum Above {
    /// The other floats in its window.
    #[default]
    Window,
    /// Every window (a popped-out float the platform keeps on top).
    All,
}

/// A place and size, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Default, Facet)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// A tab group: its tabs, the one showing, and how it shows them.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Group {
    pub id: GroupId,
    pub tabs: Vec<PaneId>,
    /// Index into `tabs` of the tab showing.
    pub current: usize,
    /// Its tabs side by side in a sliding strip (Obsidian's stacked tabs).
    pub stacked: bool,
    pub chrome: Chrome,
}

impl Group {
    #[must_use]
    pub const fn new(id: GroupId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            current: 0,
            stacked: false,
            chrome: Chrome::Tabs,
        }
    }

    /// The tab showing.
    #[must_use]
    pub fn current_tab(&self) -> Option<&PaneId> {
        self.tabs.get(self.current)
    }
}

/// One child of a split, and its share of the split's length.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Child {
    pub node: Node,
    /// Its share of the split's length; a split's shares sum to one.
    pub share: f32,
}

/// Children laid out along an axis.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Split {
    pub id: SplitId,
    pub axis: Axis,
    pub children: Vec<Child>,
}

/// A tree of splits ending in groups.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(C)]
pub enum Node {
    Split(Split),
    Group(Group),
}

impl Node {
    /// Every group in the tree, in order.
    pub fn groups<'a>(&'a self, out: &mut Vec<&'a Group>) {
        match self {
            Self::Group(group) => out.push(group),
            Self::Split(split) => {
                for child in &split.children {
                    child.node.groups(out);
                }
            }
        }
    }

    /// The group `id`, if it is in this tree.
    #[must_use]
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        match self {
            Self::Group(group) => (group.id == id).then_some(group),
            Self::Split(split) => split.children.iter().find_map(|c| c.node.group(id)),
        }
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        match self {
            Self::Group(group) => (group.id == id).then_some(group),
            Self::Split(split) => split.children.iter_mut().find_map(|c| c.node.group_mut(id)),
        }
    }

    pub fn split_mut(&mut self, id: SplitId) -> Option<&mut Split> {
        match self {
            Self::Group(_) => None,
            Self::Split(split) => {
                if split.id == id {
                    return Some(split);
                }
                split.children.iter_mut().find_map(|c| c.node.split_mut(id))
            }
        }
    }

    /// Where `pane` is in this tree: its group and its index there.
    #[must_use]
    pub fn find_pane(&self, pane: &PaneId) -> Option<(GroupId, usize)> {
        match self {
            Self::Group(group) => group
                .tabs
                .iter()
                .position(|t| t == pane)
                .map(|i| (group.id, i)),
            Self::Split(split) => split.children.iter().find_map(|c| c.node.find_pane(pane)),
        }
    }

    /// The first group in the tree (top left).
    #[must_use]
    pub fn first_group(&self) -> Option<&Group> {
        match self {
            Self::Group(group) => Some(group),
            Self::Split(split) => split.children.iter().find_map(|c| c.node.first_group()),
        }
    }
}

/// A dock's ribbon: the two-stage sidebar's first stage. Its views
/// (navigation), then the commands pinned under them, each in the
/// person's order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct Ribbon {
    /// Show this view in the dock.
    pub views: Vec<ViewKind>,
    /// Run this action (an Action List id).
    pub commands: Vec<String>,
    /// Views the person has hidden from the ribbon.
    pub hidden: Vec<ViewKind>,
}

/// A dock along one side of the main window.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Dock {
    /// Its groups (tabs shown as icons), split top and bottom.
    pub root: Option<Node>,
    /// Its width (or height, at the bottom), in points.
    pub size: f32,
    pub shown: Shown,
    pub ribbon: Option<Ribbon>,
}

impl Dock {
    #[must_use]
    pub const fn new(size: f32) -> Self {
        Self {
            root: None,
            size,
            shown: Shown::Open,
            ribbon: None,
        }
    }
}

/// The main window's docks.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Docks {
    pub left: Dock,
    pub right: Dock,
    pub bottom: Dock,
}

impl Default for Docks {
    fn default() -> Self {
        Self {
            left: Dock::new(264.0),
            right: Dock::new(320.0),
            bottom: Dock::new(240.0),
        }
    }
}

impl Docks {
    /// The dock on `side` (there is none at the top).
    #[must_use]
    pub const fn get(&self, side: Side) -> Option<&Dock> {
        match side {
            Side::Left => Some(&self.left),
            Side::Right => Some(&self.right),
            Side::Bottom => Some(&self.bottom),
            Side::Top => None,
        }
    }

    pub const fn get_mut(&mut self, side: Side) -> Option<&mut Dock> {
        match side {
            Side::Left => Some(&mut self.left),
            Side::Right => Some(&mut self.right),
            Side::Bottom => Some(&mut self.bottom),
            Side::Top => None,
        }
    }

    /// The sides that have docks.
    pub const SIDES: [Side; 3] = [Side::Left, Side::Right, Side::Bottom];
}

pub use snap::Snap;

// The facet derive's generated layout code for a data enum trips the
// arithmetic lint; the allowance stays in this module.
#[allow(clippy::arithmetic_side_effects)]
mod snap {
    use super::Side;
    use facet::Facet;

    /// Where a float has snapped to (odysseus's snap zones).
    #[derive(Debug, Clone, Copy, PartialEq, Facet)]
    #[repr(C)]
    pub enum Snap {
        /// Filling the window (dragged to the top edge).
        Maximised,
        /// Half the window (dragged to the left or right edge).
        Half(Side),
        /// A full-height panel at an edge the page makes room for.
        EdgeDock { side: Side, width: f32 },
    }
}

/// A float kept in a corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Pin {
    pub corner: Corner,
    pub above: Above,
}

/// A group floating over a window.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Float {
    pub group: Group,
    /// Where it is when not snapped (and where it goes back to).
    pub rect: Rect,
    pub snap: Option<Snap>,
    pub pin: Option<Pin>,
    /// Kept alive as a chip in the window's dock of minimised floats.
    pub minimised: bool,
}

/// One window: the main one, or a pop-out.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Window {
    pub id: WindowId,
    /// Where it is on screen; `None` for the main window (the platform
    /// places it).
    pub bounds: Option<Rect>,
    pub maximized: bool,
    /// The main area.
    pub center: Option<Node>,
    /// The main window's docks; a pop-out's stay empty.
    pub docks: Docks,
    /// Floating over it; the last is the front.
    pub floats: Vec<Float>,
}

impl Window {
    #[must_use]
    pub fn new(id: WindowId, center: Node) -> Self {
        Self {
            id,
            bounds: None,
            maximized: false,
            center: Some(center),
            docks: Docks::default(),
            floats: Vec::new(),
        }
    }
}

/// A pane's back and forward, for views that navigate: each entry a state
/// the pane showed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct History {
    pub back: Vec<String>,
    pub forward: Vec<String>,
}

/// One tab: a view and what it remembers.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Pane {
    pub id: PaneId,
    pub view: ViewKind,
    /// The view's own state, opaque here (facet JSON).
    pub state: String,
    /// Cached, to draw a tab without building its view.
    pub title: String,
    pub icon: Option<String>,
    /// Never reused: what would open in it opens in a new tab.
    pub pinned: bool,
    pub link: Option<LinkId>,
    pub history: History,
}

impl Pane {
    pub fn new(id: impl Into<String>, view: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: PaneId::new(id),
            view: ViewKind::new(view),
            state: String::new(),
            title: title.into(),
            icon: None,
            pinned: false,
            link: None,
            history: History::default(),
        }
    }
}

/// A closed pane, kept to reopen where it was.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ClosedPane {
    pub pane: Pane,
    pub group: GroupId,
    pub index: usize,
}

/// Which part of a window a group is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Center,
    Dock(Side),
    /// The float at this index in its window's `floats`.
    Float(usize),
}

/// Where a pane is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// Index into the workspace's `windows`.
    pub window: usize,
    pub area: Area,
    pub group: GroupId,
    /// Its index among the group's tabs.
    pub index: usize,
}

/// Everything a person has arranged: the main window and its pop-outs, and
/// every pane in them.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Workspace {
    /// The first is the main window.
    pub windows: Vec<Window>,
    pub panes: Vec<Pane>,
    /// The active pane, across all windows.
    pub active: Option<PaneId>,
    /// Panes most recent first: "the current document" is the most recent
    /// one in a main area.
    pub recent: Vec<PaneId>,
    /// Closed panes, newest last, to reopen.
    pub closed: Vec<ClosedPane>,
    /// The next group or split id.
    pub next_id: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// How many closed panes are kept to reopen.
    pub const CLOSED_KEPT: usize = 50;

    /// A main window with one empty group.
    #[must_use]
    pub fn new() -> Self {
        Self {
            windows: vec![Window::new(
                WindowId::main(),
                Node::Group(Group::new(GroupId(1))),
            )],
            panes: Vec::new(),
            active: None,
            recent: Vec::new(),
            closed: Vec::new(),
            next_id: 2,
        }
    }

    /// A fresh id for a group or a split.
    pub(crate) const fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    #[must_use]
    pub fn pane(&self, id: &PaneId) -> Option<&Pane> {
        self.panes.iter().find(|p| &p.id == id)
    }

    pub fn pane_mut(&mut self, id: &PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|p| &p.id == id)
    }

    /// Where `pane` is.
    #[must_use]
    pub fn locate(&self, pane: &PaneId) -> Option<Location> {
        self.windows.iter().enumerate().find_map(|(w, window)| {
            let center = window
                .center
                .as_ref()
                .and_then(|n| n.find_pane(pane))
                .map(|(g, i)| (Area::Center, g, i));
            let dock = || {
                Docks::SIDES.iter().find_map(|&side| {
                    window
                        .docks
                        .get(side)?
                        .root
                        .as_ref()?
                        .find_pane(pane)
                        .map(|(g, i)| (Area::Dock(side), g, i))
                })
            };
            let float = || {
                window.floats.iter().enumerate().find_map(|(f, float)| {
                    float
                        .group
                        .tabs
                        .iter()
                        .position(|t| t == pane)
                        .map(|i| (Area::Float(f), float.group.id, i))
                })
            };
            center
                .or_else(dock)
                .or_else(float)
                .map(|(area, group, index)| Location {
                    window: w,
                    area,
                    group,
                    index,
                })
        })
    }

    /// The group `id`, wherever it is.
    #[must_use]
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.windows.iter().find_map(|window| {
            window
                .center
                .as_ref()
                .and_then(|n| n.group(id))
                .or_else(|| {
                    Docks::SIDES
                        .iter()
                        .find_map(|&s| window.docks.get(s)?.root.as_ref()?.group(id))
                })
                .or_else(|| window.floats.iter().map(|f| &f.group).find(|g| g.id == id))
        })
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.windows.iter_mut().find_map(|window| {
            let Window {
                center,
                docks,
                floats,
                ..
            } = window;
            if let Some(group) = center.as_mut().and_then(|n| n.group_mut(id)) {
                return Some(group);
            }
            if let Some(group) = [&mut docks.left, &mut docks.right, &mut docks.bottom]
                .into_iter()
                .find_map(|dock| dock.root.as_mut()?.group_mut(id))
            {
                return Some(group);
            }
            floats.iter_mut().map(|f| &mut f.group).find(|g| g.id == id)
        })
    }

    /// The group the active pane is in, else the main window's first.
    #[must_use]
    pub fn active_group(&self) -> Option<GroupId> {
        self.active
            .as_ref()
            .and_then(|a| self.locate(a))
            .map(|l| l.group)
            .or_else(|| {
                self.windows
                    .first()?
                    .center
                    .as_ref()?
                    .first_group()
                    .map(|g| g.id)
            })
    }

    /// "The current document": the most recent pane in a main area (not a
    /// dock), so a command meant for it keeps its target while a dock has
    /// the focus.
    #[must_use]
    pub fn current_document(&self) -> Option<&PaneId> {
        self.recent.iter().find(|p| {
            self.locate(p)
                .is_some_and(|l| !matches!(l.area, Area::Dock(_)))
        })
    }

    /// The window `id`.
    #[must_use]
    pub fn window(&self, id: &WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| &w.id == id)
    }
}
