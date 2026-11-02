//! What each workspace action takes and answers, as facet types: their
//! JSON Schemas are what a model reads before it calls one.
//!
//! Ids are the model's: a pane's `"note-welcome"`, a group's number, a
//! window's `"main"` / `"window-7"` / `"dock-left"`. They are opaque and
//! never handed out twice; read them from an answer or a layout, never
//! make them up.

// The facet derive's generated layout code for a data enum trips the
// arithmetic lint; the allowance stays in this module.
#![allow(clippy::arithmetic_side_effects)]

use facet::Facet;

use crate::guide::Arrangement;
use crate::layout::Direction;
use crate::model::{Chrome, Corner, Rect, Shown, Side};

/// A side of a group, where a split goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum PaneSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl From<PaneSide> for Side {
    fn from(side: PaneSide) -> Self {
        match side {
            PaneSide::Left => Self::Left,
            PaneSide::Right => Self::Right,
            PaneSide::Top => Self::Top,
            PaneSide::Bottom => Self::Bottom,
        }
    }
}

/// A side of the window that has a dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum DockSide {
    Left,
    Right,
    Bottom,
}

impl From<DockSide> for Side {
    fn from(side: DockSide) -> Self {
        match side {
            DockSide::Left => Self::Left,
            DockSide::Right => Self::Right,
            DockSide::Bottom => Self::Bottom,
        }
    }
}

/// A direction from a pane, to its neighbour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum Toward {
    Left,
    Right,
    Up,
    Down,
}

impl From<Toward> for Direction {
    fn from(t: Toward) -> Self {
        match t {
            Toward::Left => Self::Left,
            Toward::Right => Self::Right,
            Toward::Up => Self::Up,
            Toward::Down => Self::Down,
        }
    }
}

/// On, off, or the other of the two: `on` and `off` are safe to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    On,
    Off,
    Toggle,
}

impl Mode {
    /// The state to be in, from the state it is in.
    #[must_use]
    pub const fn apply(self, now: bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Toggle => !now,
        }
    }
}

/// How much of a dock shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum DockShown {
    Open,
    /// Down to its ribbon (or its tabs' icons).
    Collapsed,
    Hidden,
}

impl From<DockShown> for Shown {
    fn from(s: DockShown) -> Self {
        match s {
            DockShown::Open => Self::Open,
            DockShown::Collapsed => Self::Collapsed,
            DockShown::Hidden => Self::Hidden,
        }
    }
}

/// How a group shows its tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum TabChrome {
    /// A tab bar of names.
    Tabs,
    /// A row of icons.
    Icons,
    /// No tab bar.
    Bare,
}

impl From<TabChrome> for Chrome {
    fn from(c: TabChrome) -> Self {
        match c {
            TabChrome::Tabs => Self::Tabs,
            TabChrome::Icons => Self::Icons,
            TabChrome::Bare => Self::Bare,
        }
    }
}

/// Back or forward through a pane's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum Go {
    Back,
    Forward,
}

/// Where a float snaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum SnapTo {
    /// Back to its own rectangle.
    None,
    /// Filling the window.
    Maximised,
    /// The window's left half.
    LeftHalf,
    /// The window's right half.
    RightHalf,
    /// A full-height panel at the left edge the page makes room for
    /// (`panel_width` wide).
    LeftPanel,
    /// The same, at the right edge.
    RightPanel,
}

/// The corner a float is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum PinTo {
    /// Not pinned.
    None,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PinTo {
    pub(crate) const fn corner(self) -> Option<Corner> {
        match self {
            Self::None => None,
            Self::TopLeft => Some(Corner::TopLeft),
            Self::TopRight => Some(Corner::TopRight),
            Self::BottomLeft => Some(Corner::BottomLeft),
            Self::BottomRight => Some(Corner::BottomRight),
        }
    }
}

/// Where `WORKSPACE_OPEN` puts a new pane: always relative to something
/// named, never "wherever the person is".
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum At {
    /// In place of this pane's tab, unless it is pinned (then a new tab
    /// right after it).
    Replace { pane: String },
    /// A new tab right after this pane, in its group.
    Beside { pane: String },
    /// A new tab in this group, at `index` (the end when unset).
    Group {
        group: u64,
        #[facet(default)]
        index: Option<usize>,
    },
    /// A new group on `side` of the group holding this pane.
    Split { pane: String, side: PaneSide },
    /// A window of its own, at `bounds` (screen points) when given.
    Window {
        #[facet(default)]
        bounds: Option<Rect>,
    },
    /// The dock on `side`, which opens.
    Dock { side: DockSide },
    /// Floating over the main window, at `rect` (window points) when
    /// given.
    Float {
        #[facet(default)]
        rect: Option<Rect>,
    },
}

/// Where `WORKSPACE_MOVE` moves a pane.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum To {
    /// Into this group, at `index` (the end when unset).
    Group {
        group: u64,
        #[facet(default)]
        index: Option<usize>,
    },
    /// Right after this pane, in its group.
    Beside { pane: String },
    /// A new group on `side` of this group.
    Edge { group: u64, side: PaneSide },
    /// A new group on `side` of the group holding this pane.
    Split { pane: String, side: PaneSide },
    /// A window: `id` `"main"` is the main window (into its focused
    /// group), another existing window's id its first group; unset, a new
    /// window at `bounds` (screen points) when given.
    Window {
        #[facet(default)]
        id: Option<String>,
        #[facet(default)]
        bounds: Option<Rect>,
    },
    /// The dock on `side`.
    Dock { side: DockSide },
    /// Floating over the main window.
    Float {
        #[facet(default)]
        rect: Option<Rect>,
    },
}

/// `WORKSPACE_OPEN`: a view, opened at a place named explicitly.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct OpenArgs {
    /// The view's kind (`WORKSPACE_VIEWS` lists them).
    pub view: String,
    /// The pane's id. A view that is single always has its kind as id;
    /// otherwise a fresh one is made when unset. An id that is open
    /// already is not opened again: `changed: false, reason:
    /// already_open`.
    #[facet(default)]
    pub id: Option<String>,
    /// The view's state (its own JSON, opaque here).
    #[facet(default)]
    pub state: Option<String>,
    /// Its tab's title; the view's when unset.
    #[facet(default)]
    pub title: Option<String>,
    pub at: At,
    /// Make it the active pane. Unset, the person's focus stays where it
    /// is.
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_MOVE`.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct MoveArgs {
    pub pane: String,
    pub to: To,
    /// Make it the active pane. Unset, the focus stays.
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_SPLIT`: the pane out of its group, into a new group on
/// `side` of it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SplitArgs {
    pub pane: String,
    pub side: PaneSide,
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_FOCUS`: `pane`, or the pane showing next to `from` toward
/// `direction`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct FocusArgs {
    #[facet(default)]
    pub pane: Option<String>,
    #[facet(default)]
    pub from: Option<String>,
    #[facet(default)]
    pub direction: Option<Toward>,
}

/// `WORKSPACE_ZOOM`: the group holding `pane` alone in its window's
/// center, or the whole tree again.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ZoomArgs {
    pub pane: String,
    #[facet(default)]
    pub mode: Mode,
}

/// `WORKSPACE_RESIZE`: one of — a split's `shares` (or `even`), a dock's
/// `size`, or `pane` grown toward `direction` by `amount` (a share of its
/// split, 0.1 when unset).
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ResizeArgs {
    #[facet(default)]
    pub split: Option<u64>,
    /// Each child's share, normalised to sum to one.
    #[facet(default)]
    pub shares: Option<Vec<f32>>,
    /// Even the split's shares out.
    #[facet(default)]
    pub even: bool,
    #[facet(default)]
    pub dock: Option<DockSide>,
    /// The dock's width (height, at the bottom), points.
    #[facet(default)]
    pub size: Option<f32>,
    #[facet(default)]
    pub pane: Option<String>,
    #[facet(default)]
    pub direction: Option<Toward>,
    #[facet(default)]
    pub amount: Option<f32>,
}

/// `WORKSPACE_GROUP_SET`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct GroupSetArgs {
    pub group: u64,
    #[facet(default)]
    pub stacked: Option<bool>,
    #[facet(default)]
    pub chrome: Option<TabChrome>,
    /// The tab to show.
    #[facet(default)]
    pub current: Option<String>,
}

/// `WORKSPACE_TAB_SET`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct TabSetArgs {
    pub pane: String,
    #[facet(default)]
    pub pinned: Option<bool>,
    /// Link it with every pane of this link; `""` unlinks it.
    #[facet(default)]
    pub link: Option<String>,
    #[facet(default)]
    pub title: Option<String>,
}

/// `WORKSPACE_NAVIGATE`: `go` back or forward, or show `state` (the old
/// one kept to go back to).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NavigateArgs {
    pub pane: String,
    #[facet(default)]
    pub go: Option<Go>,
    #[facet(default)]
    pub state: Option<String>,
}

/// `WORKSPACE_DOCK_SET`.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct DockSetArgs {
    pub side: DockSide,
    #[facet(default)]
    pub shown: Option<DockShown>,
    /// Width (height, at the bottom), points; 120 at least.
    #[facet(default)]
    pub size: Option<f32>,
}

/// `WORKSPACE_DOCK_POP_OUT`.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct DockPopOutArgs {
    pub side: DockSide,
    /// Where its window goes (screen points); unset, over where it was.
    #[facet(default)]
    pub bounds: Option<Rect>,
}

/// `WORKSPACE_DOCK_BACK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct DockBackArgs {
    pub side: DockSide,
}

/// `WORKSPACE_FLOAT`: the pane floating (moved out of its group when it
/// is not), with its rectangle, snap and pin as given.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct FloatArgs {
    pub pane: String,
    /// Window points.
    #[facet(default)]
    pub rect: Option<Rect>,
    #[facet(default)]
    pub snap: Option<SnapTo>,
    /// A panel snap's width, points (360 when unset).
    #[facet(default)]
    pub panel_width: Option<f32>,
    #[facet(default)]
    pub pin: Option<PinTo>,
    /// A pinned float stays above every window, not only its own.
    #[facet(default)]
    pub above_all: bool,
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_MINIMISE`, `WORKSPACE_CLOSE_OTHERS`, `WORKSPACE_PANE_GET`:
/// one pane.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PaneArgs {
    pub pane: String,
}

/// `WORKSPACE_RESTORE`: a minimised float back.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RestoreArgs {
    pub pane: String,
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_WINDOW_SET`.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct WindowSetArgs {
    pub window: String,
    /// Screen points (not the main window's: the platform places it).
    #[facet(default)]
    pub bounds: Option<Rect>,
    #[facet(default)]
    pub maximized: Option<bool>,
}

/// `WORKSPACE_REOPEN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct ReopenArgs {
    #[facet(default)]
    pub focus: bool,
}

/// `WORKSPACE_CLOSE`: these panes, each kept to reopen.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct CloseArgs {
    pub panes: Vec<String>,
}

/// `WORKSPACE_CLOSE_WINDOW`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct WindowArgs {
    pub window: String,
}

/// `WORKSPACE_NEIGHBOR`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NeighborArgs {
    pub pane: String,
    pub direction: Toward,
}

/// `SCREENSET_RECALL`: by `id`, or by `slot` (1–9).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RecallArgs {
    #[facet(default)]
    pub id: Option<String>,
    #[facet(default)]
    pub slot: Option<u8>,
}

/// `SCREENSET_SAVE_AS`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SaveAsArgs {
    pub name: String,
    #[facet(default)]
    pub slot: Option<u8>,
}

/// `SCREENSET_DELETE`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeleteArgs {
    pub id: String,
}

/// Why a write changed nothing. A write that changed nothing is not an
/// error: it is safe to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum Reason {
    /// It is already as asked.
    Unchanged,
    /// A pane of that id is open already.
    AlreadyOpen,
    /// It is the active pane already.
    AlreadyFocused,
    AlreadyZoomed,
    AlreadyUnzoomed,
    /// The center is one group: nothing to zoom out of.
    SingleGroup,
    /// The pane is its group's only tab: splitting it off leaves the
    /// layout as it was.
    OnlyTab,
    /// No pane lies that way.
    NoNeighbor,
    /// Nothing to go back or forward to.
    NoHistory,
    /// Nothing was closed to reopen.
    NothingClosed,
    /// The dock is in its own window already.
    AlreadyOut,
    /// The dock is in the main window already.
    DockNotOut,
    /// The dock has nothing to pop out.
    DockEmpty,
    /// This app opens no other windows here (a browser).
    NoWindows,
    /// The pane is not a minimised float.
    NotMinimised,
    /// No screenset is active.
    NoScreenset,
    /// A screenset the app ships: it can be changed and reset, not
    /// deleted.
    Builtin,
}

/// What every workspace write answers: whether it changed anything (and
/// if not, why), what it acted on, and the layout after it.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Answer {
    pub changed: bool,
    /// Why nothing changed, when nothing did.
    #[facet(default)]
    pub reason: Option<Reason>,
    /// The pane it acted on or made.
    #[facet(default)]
    pub pane: Option<String>,
    /// The group that pane is in now (or the group acted on).
    #[facet(default)]
    pub group: Option<u64>,
    /// The window it is in now (or the window acted on).
    #[facet(default)]
    pub window: Option<String>,
    /// The screenset acted on or made.
    #[facet(default)]
    pub screenset: Option<String>,
    pub layout: Arrangement,
}
