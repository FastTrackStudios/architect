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

/// The one place a scope tab's name is written: the words a person reads
/// (`noun`, `title`, `plural`, `plural_title`; the tabs on show with no
/// scope tab active, `general` and `general_title`) and the actions'
/// prefix (`prefix`). The code calls a scope tab a deck ([`Deck`]);
/// renaming what a person reads is changing these.
macro_rules! scope_word {
    (noun) => {
        "scope tab"
    };
    (title) => {
        "Scope tab"
    };
    (plural) => {
        "scope tabs"
    };
    (plural_title) => {
        "Scope tabs"
    };
    (general) => {
        "general tabs"
    };
    (general_title) => {
        "General"
    };
    (prefix) => {
        "SCOPE_"
    };
}

#[cfg(feature = "ui")]
mod bar;
pub mod control;
#[cfg(feature = "ctx")]
mod ctx;
pub mod deck;
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
#[cfg(feature = "presence")]
pub mod pane_space;
mod presence;
mod repair;
mod screenset;
pub mod store;
mod switcher;
pub mod tab_kind;
pub mod tree;

#[cfg(feature = "ui")]
pub use bar::{TABS_COMMAND, use_tabs_button};
pub use deck::{
    ContextLayout, ContextProperty, Deck, DeckContext, DeckError, DeckId, DeckState, DeckTemplate,
    Decks, KindDefault, Load, Outcome, Unchanged,
};
pub use drop::{Bar, Drop, bar_drop, contains, drop_at, drop_in, float_rect, snap_at};
pub use tab_kind::{TabFace, TabKind, TabKindItem, TabKinds, TabWidth};
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

/// The context-menu targets the workspace owns, `(kind, what it is)`:
/// what an app's extension host declares for it
/// (`architect_extensions`' `Host::declare_context_targets`).
pub const CONTEXT_TARGETS: [(&str, &str); 3] = [
    ("pane", "every pane's tab; its id the pane's"),
    ("tab", "a tab, past its view kind's rows; its id the pane's"),
    (
        "view",
        "the tabs of one view kind's panes: `view:<kind>`; its id the pane's",
    ),
];
#[cfg(feature = "ui")]
pub use host::{
    Bridge, Command, ContextEntries, ContextEntry, DeckHandle, DeckResolve, DeckStores,
    DecksConfig, HeaderTools, Host, Opened, PaneHandle, Popout, PresenceLayer, PresenceMarksLayer,
    PresenceOverlay, TabBadges, TitleRow, View, Views, WindowSystem, Windows, WorkspaceActions,
    WorkspaceStore, WorkspaceView, context_invocation, try_use_decks, try_use_pane_of,
    use_deck_context, use_decks, use_host, use_pane, use_pane_of, use_pane_status,
    use_presence_overlay,
};
#[cfg(all(feature = "ui", feature = "presence"))]
pub use host::{PaneRoom, use_pane_room, use_pane_space, use_pane_subject};
#[cfg(feature = "ui")]
pub use host::{TabKindsUi, TabRender, TabRenderCtx, try_use_tab_kinds, use_tab_kinds};
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
pub use store::{ContextStore, MemoryContextStore, MemoryStore, Store};
pub use switcher::{Card, CardGrid, Place, reorder_dest};
