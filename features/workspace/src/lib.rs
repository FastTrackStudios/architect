//! The workspace: what a person arranges, as data.
//!
//! A [`Workspace`] is every window an app has open — the main window and
//! its pop-outs — each a tree of [`Split`]s ending in tab [`Group`]s, with
//! docks on its sides (the main window's) and panes floating over it; and
//! every [`Pane`] (one tab: a view and its state), wherever it is. The
//! operations ([`Workspace::open`], [`Workspace::move_pane`],
//! [`Workspace::close`], …) are what every gesture and every key comes
//! down to; after each one the workspace is [repaired](Workspace::repair)
//! to its invariants. A [`Screenset`] is a workspace kept under a name,
//! one per form class.
//!
//! The design is `docs/content/architecture/workspace.md`. This module is
//! the model only: no Dioxus, no I/O, no clock. The host that draws it and
//! turns pointer drags into operations is the crate's `ui` feature:
//! [`WorkspaceView`], given the app's [`Views`].
//!
//! What a view draws *with* is its kind ([`KindId`]): the built-in
//! `dioxus` one or one a linked crate registered in a [`ViewKinds`]
//! (`docs/content/architecture/embedded-views.md`). Feature `ctx` is the
//! contract a kind mounts through, [`ViewCtx`]; feature `paint` the
//! built-in kind that paints a scene each frame, drawn in-process by Blitz
//! under `paint-ui` (`src/paint/`).

// A view's icon is a fn pointer compared by Dioxus's props: a stale
// compare only redraws.
#![cfg_attr(feature = "ui", allow(unpredictable_function_pointer_comparisons))]

#[cfg(feature = "ui")]
mod bar;
pub mod control;
#[cfg(feature = "ctx")]
mod ctx;
mod drop;
pub mod guide;
#[cfg(feature = "home")]
pub mod home;
#[cfg(feature = "ui")]
mod host;
mod kinds;
mod layout;
pub mod meter;
mod model;
mod ops;
#[cfg(feature = "paint")]
pub mod paint;
mod presence;
mod repair;
mod screenset;
pub mod store;
mod switcher;
pub mod tree;

#[cfg(feature = "ui")]
pub use bar::{TABS_COMMAND, use_tabs_button};
pub use drop::{Bar, Drop, bar_drop, contains, drop_at, drop_in, float_rect, snap_at};
#[cfg(feature = "native")]
pub mod native;
#[cfg(feature = "native")]
pub mod shot;
#[cfg(feature = "ctx")]
pub use ctx::{
    Button, DocRef, Extras, Frame, FrameClock, Geometry, HostData, InputSink, Now, ThemeTokens,
    ViewCtx, ViewInput,
};
pub use guide::{ArrangedGroup, ArrangedSplit, ArrangedWindow, Arrangement, target_selector};
#[cfg(feature = "ui")]
pub use host::{
    Bridge, Command, ContextEntries, ContextEntry, HeaderTools, Host, Opened, PaneHandle, Popout,
    PresenceLayer, PresenceOverlay, TabBadges, TitleRow, View, Views, WindowSystem, Windows,
    WorkspaceActions, WorkspaceStore, WorkspaceView, context_invocation, use_host, use_pane,
    use_pane_status, use_presence_overlay,
};
pub use kinds::{
    Capabilities, Capability, DIOXUS, Hosts, KindError, KindId, KindSpec, PAINT, Part, Placement,
    Placements, ViewHost, ViewKinds,
};
pub use layout::{Direction, Placed, Slide, area_rects, neighbour, scroll_to, slides};
pub use meter::{FRAME_BUDGET, Meters, ViewHealth, ViewMeter, ViewReport, ViewSummary};
pub use model::{
    Above, Aliases, Area, Axis, Child, Chrome, ClosedPane, Corner, Dock, Docks, Float, Group,
    GroupAlias, GroupId, History, LinkId, Location, Node, Pane, PaneId, Pin, Rect, Ribbon, Shown,
    Side, Snap, Split, SplitId, ViewKind, Window, WindowAlias, WindowId, Workspace,
};
pub use ops::{Dest, Target, WorkspaceError};
#[cfg(feature = "paint")]
pub use paint::{FrameStats, FrameSummary, Mark, Marks, PaintView, Painter};
#[cfg(feature = "paint-ui")]
pub use paint::{PaintMount, PaintPane, PaintWake, PaintWidget, shell_tokens};
pub use presence::{PlayheadMark, PointerMark, PresenceMarks, WashMark, css_color};
pub use screenset::{
    FormClass, Layout, Recall, SavedScreensets, Screenset, ScreensetOptions, Screensets,
};
pub use store::{MemoryStore, Store};
pub use switcher::{Card, CardGrid, Place, reorder_dest};
