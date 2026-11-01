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

// A view's icon is a fn pointer compared by Dioxus's props: a stale
// compare only redraws.
#![cfg_attr(feature = "ui", allow(unpredictable_function_pointer_comparisons))]

mod drop;
#[cfg(feature = "ui")]
mod host;
mod layout;
mod model;
mod ops;
mod repair;
mod screenset;

pub use drop::{Drop, contains, drop_at, float_rect, snap_at};
#[cfg(feature = "ui")]
pub use host::{
    Command, Host, PaneHandle, TitleRow, View, Views, WorkspaceView, use_host, use_pane,
};
pub use layout::{Direction, Placed, area_rects, neighbour};
pub use model::{
    Above, Area, Axis, Child, Chrome, ClosedPane, Corner, Dock, Docks, Float, Group, GroupId,
    History, LinkId, Location, Node, Pane, PaneId, Pin, Rect, Ribbon, Shown, Side, Snap, Split,
    SplitId, ViewKind, Window, WindowId, Workspace,
};
pub use ops::{Dest, Target, WorkspaceError};
pub use screenset::{FormClass, Layout, Screenset, Screensets};
