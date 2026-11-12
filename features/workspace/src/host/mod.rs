//! The host (feature `ui`): draws a [`Workspace`] on Blitz and turns
//! pointer drags, tab presses and menu rows into its operations
//! (`docs/content/architecture/workspace.md`).
//!
//! - **Inline styles only**, nothing Blitz drops: no `position: fixed`
//!   (every overlay is absolute against the host's root), no
//!   `overflow: auto`, no drag-and-drop events (tabs, dividers and floats
//!   drag on pointer events, measured with `get_client_rect`).
//! - **Context, not globals**: [`WorkspaceView`] puts a [`Host`] in
//!   context, so two windows (or a plugin editor) each host their own.
//! - **A pane is a [`View`]'s render function** given its [`PaneId`]; it
//!   reads itself with [`use_pane`] and keeps its state in
//!   [`Pane::state`](crate::Pane) through [`PaneHandle::set_state`].
//!
//! On a phone (`phone: true`) one pane shows at a time: the active
//! group's current tab, with a switcher listing every pane.
//!
//! A pop-out window is a second document (Blitz draws one per
//! `VirtualDom`) reading the same `Signal<Workspace>`: where the app runs
//! on [`Windows`] (feature `native`'s launcher), a tab dragged out of the
//! window, or "Move to a new window", opens one; elsewhere a pop-out kept
//! in a layout shows as a chip that brings its tabs back.

// Dioxus is single-threaded: its tasks are not `Send`, and a [`Host`] is a
// handful of signals copied into each handler. The `for` loops of `rsx!`
// iterate rows collected so the workspace's read is let go before drawing.
#![allow(
    clippy::future_not_send,
    clippy::large_types_passed_by_value,
    clippy::needless_collect
)]

mod actions;
mod commands;
mod deck;
mod dock;
mod group;
mod manage;
mod overlay;
#[cfg(feature = "presence")]
mod pane_presence;
#[cfg(not(feature = "presence"))]
#[path = "pane_presence_off.rs"]
mod pane_presence;
mod presence;
mod screens;
mod slide;
mod switcher;
mod tab_kinds;
mod windows;

use std::collections::HashMap;
use std::rc::Rc;

use architect_shell::motion::{Motion, Spring};
use architect_shell::{IconFn, ShellTheme, StatusItem};
use dioxus::prelude::*;

use crate::drop::{Bar, Drop};
use crate::kinds::{KindError, KindId, Placement, ViewHost, ViewKinds};
use crate::model::{
    Area, Axis, GroupId, Node, Pane, PaneId, Rect, Shown, Side, SplitId, ViewKind, WindowId,
    Workspace,
};
use crate::ops::{Dest, Target};
use crate::screenset::{FormClass, Screensets};

use dock::DockView;
use group::NodeView;
use overlay::{DragLayer, DropHighlight, FloatView, MenuLayer, PhoneBody, SheetLayer};
#[cfg(feature = "presence")]
pub use pane_presence::{PaneRoom, use_pane_room, use_pane_space, use_pane_subject};
pub use presence::{PresenceLayer, PresenceMarksLayer, PresenceOverlay, use_presence_overlay};

pub use actions::{Opened, WorkspaceActions};
pub use deck::{
    DeckHandle, DeckResolve, DeckStores, DecksConfig, try_use_decks, use_deck_context, use_decks,
};
pub use screens::WorkspaceStore;
pub use tab_kinds::{TabKindsUi, TabRender, TabRenderCtx, try_use_tab_kinds, use_tab_kinds};
pub use windows::{Bridge, Popout, WindowSystem, Windows};

/// What a pane can show: registered once, like the shell's tabs.
#[derive(Clone)]
pub struct View {
    pub kind: ViewKind,
    pub title: String,
    pub icon: Option<IconFn>,
    /// Where one opens when nothing says otherwise.
    pub home: Target,
    /// At most one open: its pane id is its kind, so opening it again
    /// brings it forward.
    pub single: bool,
    /// Its icon in a phone's tab switcher, not a live preview: a view
    /// that is costly to draw twice, or that says so to others as it
    /// mounts and goes (a published canvas, a remote process's view).
    pub still: bool,
    /// Its pane's header row (back, forward, title, menu) is drawn; a view
    /// whose tab already names it turns that off ([`View::headless`]).
    pub header: bool,
    /// What draws it: the built-in `dioxus` kind unless a kind's
    /// provider made it ([`View::drawn_by`]). A pane whose view's kind is
    /// not in [`Views::kinds`] shows why in its place.
    pub draws: KindId,
    render: Rc<dyn Fn(PaneId) -> Element>,
}

impl View {
    /// A view of `kind` titled `title`, drawn by `render`. Return a
    /// component (`rsx! { Mixer { pane } }`) so the pane's hooks are its
    /// own.
    pub fn new(
        kind: impl Into<String>,
        title: impl Into<String>,
        render: impl Fn(PaneId) -> Element + 'static,
    ) -> Self {
        Self {
            kind: ViewKind::new(kind),
            title: title.into(),
            icon: None,
            home: Target::Tab,
            single: true,
            still: false,
            header: true,
            draws: KindId::dioxus(),
            render: Rc::new(render),
        }
    }

    #[must_use]
    pub const fn icon(mut self, icon: IconFn) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub const fn home(mut self, home: Target) -> Self {
        self.home = home;
        self
    }

    /// More than one may be open (a document; each gets a fresh id).
    #[must_use]
    pub const fn many(mut self) -> Self {
        self.single = false;
        self
    }

    /// Shows its icon in a phone's tab switcher instead of a live preview
    /// (see [`View::still`](Self#structfield.still)); a view that draws
    /// itself twice safely can check [`PaneHandle::is_preview`] instead.
    #[must_use]
    pub const fn still(mut self) -> Self {
        self.still = true;
        self
    }

    /// No header row over its pane (back, forward, title, menu): for a view
    /// whose tab already names it, so the pane is just its content.
    #[must_use]
    pub const fn headless(mut self) -> Self {
        self.header = false;
        self
    }

    /// Drawn by `kind`, not the built-in `dioxus` one: what a kind's
    /// provider returns, its render mounting the kind in the pane.
    #[must_use]
    pub fn drawn_by(mut self, kind: impl Into<String>) -> Self {
        self.draws = KindId::new(kind);
        self
    }
}

/// The views an app offers, and the kinds that draw them.
#[derive(Clone, Default)]
pub struct Views(Rc<Vec<View>>, Rc<ViewKinds>);

impl PartialEq for Views {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) && Rc::ptr_eq(&self.1, &other.1)
    }
}

impl Views {
    /// No views yet; the built-in `dioxus` kind.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn view(mut self, view: View) -> Self {
        Rc::make_mut(&mut self.0).push(view);
        self
    }

    /// Draws with `kinds` (the built-in `dioxus` one and those the app's
    /// linked crates registered) in place of the built-in one alone.
    #[must_use]
    pub fn with_kinds(mut self, kinds: ViewKinds) -> Self {
        self.1 = Rc::new(kinds);
        self
    }

    /// The kinds it draws with.
    #[must_use]
    pub fn kinds(&self) -> &ViewKinds {
        &self.1
    }

    #[must_use]
    pub fn get(&self, kind: &str) -> Option<&View> {
        self.0.iter().find(|v| v.kind.0 == kind)
    }

    /// View `kind` and what draws it, mounted in a pane of `host`.
    ///
    /// # Errors
    ///
    /// Its kind is not provided here, or not for `host` or a pane. `None`
    /// when there is no such view.
    #[must_use]
    pub fn resolve(&self, kind: &str, host: ViewHost) -> Option<Result<&View, KindError>> {
        let view = self.get(kind)?;
        Some(
            self.1
                .resolve_for(view.draws.as_str(), host, Placement::Pane)
                .map(|_| view),
        )
    }

    pub fn iter(&self) -> impl Iterator<Item = &View> {
        self.0.iter()
    }
}

/// A command a dock's ribbon can pin: pressing it hands its id to the
/// app's `on_command` (an Action List id, once commands land).
#[derive(Clone, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub icon: IconFn,
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: IconFn) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
        }
    }
}

/// Something the host measures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Mount {
    Host,
    Group(GroupId),
    Split(SplitId),
    /// A group's tab bar.
    Bar(GroupId),
    /// A tab in a bar.
    Tab(PaneId),
    /// A stacked group's strip.
    Strip(GroupId),
    /// A phone's tab switcher's layer.
    Switcher,
}

/// What a pane drag measured when it began: the host, its groups (the
/// topmost first) and their tab bars.
#[derive(Debug, Clone, PartialEq)]
struct Geometry {
    host: Rect,
    groups: Vec<(GroupId, Rect)>,
    bars: Vec<Bar>,
    /// The bars again, each tab named: what makes room as the tab moves.
    rows: Vec<slide::Row>,
}

/// A drag in progress.
#[derive(Debug, Clone, PartialEq)]
enum Drag {
    /// A tab pressed, not yet moved far enough to drag.
    Pressed { pane: PaneId, x: f32, y: f32 },
    /// A tab being dragged from `x0` (where it was pressed); `geometry`
    /// is `None` until measured.
    Pane {
        pane: PaneId,
        geometry: Option<Geometry>,
        at: Option<Drop>,
        x0: f32,
        x: f32,
        y: f32,
    },
    /// The divider after child `index` of a split laid out from `origin`,
    /// `length` long, its shares `shares` when grabbed.
    Split {
        id: SplitId,
        index: usize,
        axis: Axis,
        origin: f32,
        length: f32,
        shares: Vec<f32>,
    },
    /// A dock's divider, in a host at `host`.
    Dock { side: Side, host: Rect },
    /// A float's title bar, grabbed `dx, dy` from its corner.
    Float {
        pane: PaneId,
        host: Rect,
        dx: f32,
        dy: f32,
        x: f32,
        y: f32,
    },
    /// A float's resize corner.
    FloatSize { pane: PaneId, host: Rect },
    /// A dock's grip, pressed at `x, y`, the pointer last at `last`;
    /// `moved` once past the slop (a drag, not a press).
    DockGrip {
        side: Side,
        x: f32,
        y: f32,
        last: (f32, f32),
        moved: bool,
    },
    /// A stacked strip's scrollbar, grabbed at `x` with the strip at
    /// `from`; a point of the bar is `per_point` of the strip.
    Strip {
        group: GroupId,
        x: f32,
        from: f32,
        per_point: f32,
        most: f32,
    },
}

/// A menu on show.
#[derive(Debug, Clone, PartialEq)]
enum Menu {
    /// What can be done with a pane.
    Pane(PaneId),
    /// Which view to open.
    Open,
    /// Every tab in a group, to pick one.
    Tabs(GroupId),
    /// What others add for a target a view shows (a file in a file tree):
    /// [`PaneHandle::context_menu`].
    Target {
        target: String,
        id: String,
        pane: PaneId,
    },
}

/// The sizes the pointer asks for: a mouse's, or a finger's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sizes {
    tabs: f32,
    divider: f32,
    ribbon: f32,
    button: f32,
}

impl Sizes {
    const FINE: Self = Self {
        // Obsidian's 40pt tab header: its tabs 34pt, 6pt below its top.
        tabs: 40.0,
        divider: 5.0,
        ribbon: 40.0,
        button: 28.0,
    };
    const COARSE: Self = Self {
        tabs: 44.0,
        divider: 10.0,
        ribbon: 52.0,
        button: 44.0,
    };
}

/// How many times a busy measurement is asked again.
const MEASURE_TRIES: u32 = 40;
/// The center's least width: the docks give way first, points.
const CENTER_MIN: f32 = 240.0;
/// How far a pressed tab moves before it is dragged, points.
const DRAG_SLOP: f32 = 6.0;
/// How long a finger holds a tab for its menu (a right click's), ms.
const LONG_PRESS_MS: u64 = 500;
/// A collapsed dock's rail without a ribbon, points.
const RAIL: f32 = 36.0;
/// Where a pane floated from a menu goes.
const FLOAT_AT: Rect = Rect {
    x: 96.0,
    y: 72.0,
    w: 480.0,
    h: 320.0,
};

/// The window's title row, for a workspace that reaches the window's top.
///
/// The tab bars along the top edge become it (Obsidian's): as tall, clear
/// of the window's buttons on the left and the app's on the right, their
/// empty part moving the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleRow {
    pub height: f32,
    /// Kept clear at the left end (the window's buttons), points.
    pub left: f32,
    /// Kept clear at the right end (the app's buttons), points.
    pub right: f32,
    /// Kept clear at the left end of the shell's sidebar (the window's
    /// buttons and the sidebar's), for a left dock the shell hosts.
    pub sidebar_left: f32,
    /// Start moving the window (called from the press itself).
    pub on_drag: Option<Callback<()>>,
    /// Zoom the window (a double click on the row).
    pub on_zoom: Option<Callback<()>>,
}

/// The host's state, in context under a [`WorkspaceView`].
#[derive(Clone, Copy)]
pub struct Host {
    pub workspace: Signal<Workspace>,
    views: CopyValue<Views>,
    commands: CopyValue<Vec<Command>>,
    /// The commands an empty main area offers, by id ([`WorkspaceView`]'s
    /// `empty`).
    empty: CopyValue<Vec<String>>,
    on_command: CopyValue<Option<EventHandler<String>>>,
    /// The colours, a signal so every pane, tab bar and dock repaints when
    /// the app's theme changes (a `CopyValue` here kept them on the theme
    /// the host was first drawn with).
    theme: Signal<ShellTheme>,
    sizes: Signal<Sizes>,
    phone: Signal<bool>,
    title_row: Signal<Option<TitleRow>>,
    /// The app's decks, when it has them: carried into what the shell
    /// draws for the host.
    decks: CopyValue<Option<deck::DeckHandle>>,
    in_shell: Signal<bool>,
    /// A phone shows its own bar over the pane (the title, the switcher).
    phone_header: Signal<bool>,
    drag: Signal<Option<Drag>>,
    menu: Signal<Option<Menu>>,
    /// A phone's pane opened over the one on show (a docked one).
    sheet: Signal<Option<PaneId>>,
    switcher: Signal<bool>,
    /// The switcher's zoom, grid and gestures.
    switch: switcher::Switch,
    /// The host's rectangle, as last measured (a snapped float's area).
    float_area: Signal<Option<Rect>>,
    /// Where the menu on show was asked for (window points); `None` puts it
    /// in the middle.
    menu_at: Signal<Option<(f32, f32)>>,
    mounts: CopyValue<HashMap<Mount, Rc<MountedData>>>,
    /// The arrange key is held: every pane can be grabbed.
    held: Signal<bool>,
    /// Arrange is turned on (a touch screen has no key to hold).
    latched: Signal<bool>,
    /// The window this host draws: the main one, or a pop-out.
    window: CopyValue<WindowId>,
    /// What each pane has put on the status line, shared by every window:
    /// by pane, and by who put them there (`0` for [`PaneHandle::set_status`],
    /// else a [`use_pane_status`]'s own number).
    status: Signal<Vec<(PaneId, u64, Vec<StatusItem>)>>,
    /// The last number given a [`use_pane_status`].
    status_serial: CopyValue<u64>,
    /// The screensets, and the class of screen they are recalled for.
    screensets: Signal<Screensets>,
    form: Signal<FormClass>,
    /// Each stacked group's scroll along its strip, points: where it was
    /// last sent, for a strip drawn afresh.
    scrolls: Signal<HashMap<GroupId, f32>>,
    /// Each stacked strip's scroll as it moves (a scrollbar's drag holds it).
    strips: CopyValue<HashMap<GroupId, Motion>>,
    /// Each tab's shift along its bar while a tab is dragged (`slide`).
    tab_shift: CopyValue<HashMap<PaneId, Motion>>,
    /// Where a tab landed from another bar starts, from its new place.
    tab_from: CopyValue<HashMap<PaneId, f64>>,
    /// The tab being dragged, and whether it is over its own bar (drawn
    /// there, following the pointer) or off it (its place closed up).
    lifted: Signal<Option<(PaneId, bool)>>,
    /// A split's new group growing in from the edge it was dropped on, or
    /// a closing one giving its room up: which, and how far (0–1).
    growing: Signal<Option<GroupId>>,
    grow: Motion,
    /// The size the growing group is laid out at (it is clipped, not
    /// squeezed), and the edge it is held to (`None`: its own far edge).
    grow_size: CopyValue<Option<(Rect, Option<Side>)>>,
    /// Each group's size in its split, as its box last said.
    split_sizes: CopyValue<HashMap<GroupId, Rect>>,
    /// The pane whose close waits for its group to give its room up.
    closing: CopyValue<Option<PaneId>>,
    /// The platform's windows, where pop-outs are real OS windows.
    windows: CopyValue<Option<Windows>>,
    /// A ribbon's pinned command pressed in a pop-out, for the main
    /// window's host to hand to the app's `on_command` (a handler runs in
    /// the dom that made it).
    pressed: Signal<Option<String>>,
    /// Where the platform last said the pointer let go outside this
    /// window, set as it says so (before the document hears the release),
    /// for a grip's drag to end there.
    released: CopyValue<Rc<std::cell::Cell<Option<(f32, f32)>>>>,
    /// The layout's revision, and the list its changes are told on.
    revisions: CopyValue<actions::Revisions>,
    /// The host's measure of its area and groups.
    measured: CopyValue<crate::tree::Measured>,
    /// What the app calls a saved layout ("Workspace").
    sets_noun: Signal<String>,
    /// The sheet that manages the saved layouts is open.
    managing: Signal<bool>,
    /// The screensets the app gave before anything was read from its
    /// store: the built-in ones as shipped, for the escape hatch
    /// (`WORKSPACES_RESTORE_BUILTINS`).
    shipped: CopyValue<Vec<crate::screenset::Screenset>>,
}

/// The [`Host`] a [`WorkspaceView`] above put in context.
#[must_use]
pub fn use_host() -> Host {
    use_context()
}

impl Host {
    /// Whether a pane's header row is drawn: not for a view that left it
    /// out ([`View::headless`]).
    fn pane_header(&self, pane: &PaneId) -> bool {
        let workspace = self.workspace.read();
        let Some(p) = workspace.pane(pane) else {
            return true;
        };
        self.views.read().get(&p.view.0).map_or(true, |v| v.header)
    }
}

/// The pane a slot draws, kept current: Dioxus may hand a slot another
/// pane (a tab closed beside it) without making it afresh.
#[derive(Clone, Copy)]
struct ThisPane(Signal<PaneId>);

/// Under a card in a phone's tab switcher: the view drawn again, scaled
/// down, while the pane itself may be drawn too.
#[derive(Clone, Copy)]
struct Previewing;

/// The pane this component draws, inside a [`View`]'s render.
#[must_use]
pub fn use_pane() -> PaneHandle {
    PaneHandle {
        id: use_context::<ThisPane>().0(),
        host: use_host(),
        preview: try_use_context::<Previewing>().is_some(),
    }
}

/// Pane `id`'s handle, from anywhere in the workspace (its header, a
/// toolbar in it): what [`use_pane`] gives inside its view.
#[must_use]
pub fn use_pane_of(id: PaneId) -> PaneHandle {
    PaneHandle {
        id,
        host: use_host(),
        preview: false,
    }
}

/// [`use_pane_of`] where there may be no workspace (a toolbar drawn over
/// a phone's keyboard, on a page of the shell's): `None` outside one.
#[must_use]
pub fn try_use_pane_of(id: PaneId) -> Option<PaneHandle> {
    try_use_context::<Host>().map(|host| PaneHandle {
        id,
        host,
        preview: false,
    })
}

/// A pane, from inside it.
#[derive(Clone)]
pub struct PaneHandle {
    pub id: PaneId,
    host: Host,
    preview: bool,
}

impl PaneHandle {
    /// Opens the context menu of `target` (`file`, `doc:<kind>`) `id` (a
    /// file's path) where `e` was pressed: what others add for it
    /// ([`ContextEntries`]), each row run told the target, this pane and
    /// its view. A view's `oncontextmenu` calls it for what it shows (a
    /// file tree's row).
    pub fn context_menu(&self, target: impl Into<String>, id: impl Into<String>, e: &MouseEvent) {
        e.prevent_default();
        self.host.open_menu(
            Menu::Target {
                target: target.into(),
                id: id.into(),
                pane: self.id.clone(),
            },
            click_point(e),
        );
    }

    /// Whether this is its card's preview in a phone's tab switcher (the
    /// view drawn a second time, scaled down): a view that tells others
    /// it is there as it mounts or goes leaves that to the pane itself.
    #[must_use]
    pub const fn is_preview(&self) -> bool {
        self.preview
    }

    /// Its saved state ("" when it has none). Read in a render, the pane
    /// redraws when it changes.
    #[must_use]
    pub fn state(&self) -> String {
        self.host
            .workspace
            .read()
            .pane(&self.id)
            .map(|p| p.state.clone())
            .unwrap_or_default()
    }

    /// Its tab's title.
    #[must_use]
    pub fn title(&self) -> String {
        self.host
            .workspace
            .read()
            .pane(&self.id)
            .map(|p| self.host.title(p))
            .unwrap_or_default()
    }

    /// The whole workspace, to open a pane of a given id or look around.
    #[must_use]
    pub const fn workspace(&self) -> Signal<Workspace> {
        self.host.workspace
    }

    /// Keeps its state. Write it when a gesture ends, not on every frame
    /// of one: each write is a workspace change.
    pub fn set_state(&self, state: impl Into<String>) {
        let state = state.into();
        let same = self
            .host
            .workspace
            .peek()
            .pane(&self.id)
            .is_some_and(|p| p.state == state);
        if !same {
            self.host.edit(|w| {
                if let Some(p) = w.pane_mut(&self.id) {
                    p.state = state;
                }
            });
        }
    }

    /// Its saved state as a `T` (kept as `T`'s JSON): `None` when it has
    /// none or what it has is not a `T`.
    #[must_use]
    pub fn state_as<T: for<'a> facet::Facet<'a>>(&self) -> Option<T> {
        let state = self.state();
        if state.is_empty() {
            return None;
        }
        facet_json::from_str(&state).ok()
    }

    /// Keeps `state` as its state, typed ([`state_as`](Self::state_as)
    /// reads it back); as [`set_state`](Self::set_state), once a gesture
    /// ends.
    pub fn set_state_as<T: for<'a> facet::Facet<'a>>(&self, state: &T) {
        if let Ok(json) = facet_json::to_string(state) {
            self.set_state(json);
        }
    }

    /// Names its tab.
    pub fn set_title(&self, title: impl Into<String>) {
        let title = title.into();
        let same = self
            .host
            .workspace
            .peek()
            .pane(&self.id)
            .is_some_and(|p| p.title == title);
        if !same {
            self.host.edit(|w| {
                if let Some(p) = w.pane_mut(&self.id) {
                    p.title = title;
                }
            });
        }
    }

    /// Opens a view of `kind` at `target`.
    #[must_use = "the pane opened, if the view is known"]
    pub fn open(&self, kind: &str, target: Target) -> Option<PaneId> {
        self.host.open_at(kind, target)
    }

    /// Puts the pane's own items on the status line, shown while it is the
    /// current document (a note's word count, a mixer's CPU). Call it on
    /// every render with what is true now; [`use_pane_status`] also takes
    /// them away when the pane goes.
    pub fn set_status(&self, items: Vec<StatusItem>) {
        self.put_status(0, items);
    }

    /// The pane's items, put there by `by`.
    fn put_status(&self, by: u64, items: Vec<StatusItem>) {
        let mut status = self.host.status;
        let same = status
            .peek()
            .iter()
            .find(|(p, _, _)| *p == self.id)
            .is_some_and(|(_, who, now)| *who == by && *now == items);
        if same {
            return;
        }
        let mut all = status.write();
        all.retain(|(p, _, _)| *p != self.id);
        if !items.is_empty() {
            all.push((self.id.clone(), by, items));
        }
    }

    /// Take the pane's items away, if `by` put them there (a slot drawn
    /// afresh may have put its own since).
    /// A pane in a pop-out goes with its window, whose host (and its
    /// status) may be dropped first: then there is nothing to take.
    fn take_status(&self, by: u64) {
        let mut status = self.host.status;
        let mine = status
            .try_peek()
            .is_ok_and(|all| all.iter().any(|(p, who, _)| *p == self.id && *who == by));
        if mine {
            if let Ok(mut all) = status.try_write() {
                all.retain(|(p, _, _)| *p != self.id);
            }
        }
    }
}

/// The pane's own status items (see [`PaneHandle::set_status`]), taken
/// away when the pane is no longer drawn.
pub fn use_pane_status(items: Vec<StatusItem>) {
    let pane = use_pane();
    let by = use_hook(|| {
        let mut next = pane.host.status_serial;
        let n = next.peek().saturating_add(1);
        next.set(n);
        n
    });
    // A card's preview leaves them to the pane.
    let preview = pane.preview;
    if !preview {
        pane.put_status(by, items);
    }
    use_drop(move || {
        if !preview {
            pane.take_status(by);
        }
    });
}

impl Host {
    /// The status items of the current document (the pane a command for
    /// "the document" acts on): what [`use_pane_status`] gave.
    #[must_use]
    pub fn status_items(&self) -> Vec<StatusItem> {
        let current = self.workspace.read().current_document().cloned();
        current
            .and_then(|c| {
                self.status
                    .read()
                    .iter()
                    .find(|(p, _, _)| *p == c)
                    .map(|(_, _, items)| items.clone())
            })
            .unwrap_or_default()
    }

    /// The window this host draws.
    #[must_use]
    pub fn window_id(&self) -> WindowId {
        self.window.peek().clone()
    }

    /// The index of this host's window in the workspace.
    fn window_index(&self, w: &Workspace) -> Option<usize> {
        let id = self.window.peek();
        w.windows.iter().position(|win| win.id == *id)
    }

    /// Whether pop-outs open as OS windows here.
    fn has_windows(&self) -> bool {
        self.windows.peek().is_some()
    }

    /// Changes the workspace.
    pub fn edit<R>(&self, f: impl FnOnce(&mut Workspace) -> R) -> R {
        let mut workspace = self.workspace;
        let mut workspace = workspace.write();
        f(&mut workspace)
    }

    /// Opens a view of `kind` where it lives.
    #[must_use = "the pane opened, if the view is known"]
    pub fn open(&self, kind: &str) -> Option<PaneId> {
        let home = self.views.read().get(kind)?.home;
        self.open_at(kind, home)
    }

    /// Opens a view of `kind` at `target` (a single view already open is
    /// brought forward there instead).
    #[must_use = "the pane opened, if the view is known"]
    pub fn open_at(&self, kind: &str, target: Target) -> Option<PaneId> {
        let view = self.views.read().get(kind)?.clone();
        self.edit(|w| {
            let id = if view.single {
                PaneId::new(kind)
            } else {
                PaneId::new(format!("{kind}-{}", w.alloc()))
            };
            let pane = Pane::new(id.0, kind, view.title.clone());
            w.open(pane, target).ok()
        })
    }

    /// Whether panes can be grabbed: the arrange key is held, or Arrange
    /// is on.
    #[must_use]
    pub fn arranging(&self) -> bool {
        (self.held)() || (self.latched)()
    }

    /// Holds or lets go of arranging, for an app that reads the keyboard
    /// itself.
    pub fn set_arranging(&self, on: bool) {
        let mut held = self.held;
        if *held.peek() != on {
            held.set(on);
        }
    }

    /// Turns Arrange on or off (the button for touch screens).
    pub fn toggle_arrange(&self) {
        let mut latched = self.latched;
        let on = !*latched.peek();
        latched.set(on);
    }

    /// Shows `menu` beside the point it was asked for at (window
    /// points); a phone shows it in the middle.
    fn open_menu(&self, menu: Menu, at: (f32, f32)) {
        let mut m = self.menu;
        let mut a = self.menu_at;
        a.set(Some(at));
        m.set(Some(menu));
        // The menu is placed within the host, measured now: measured only
        // when mounted, Blitz may have had no layout yet and given a zero
        // rect, and every menu went to the host's top left.
        let host = *self;
        spawn(async move {
            if let Some(r) = host.rect(Mount::Host).await
                && r.w > 0.0
            {
                let mut area = host.float_area;
                area.set(Some(r));
            }
        });
    }

    /// Shows `menu` in the middle (asked for by a key, not a pointer).
    fn show_menu(&self, menu: Menu) {
        let mut a = self.menu_at;
        a.set(None);
        let mut m = self.menu;
        m.set(Some(menu));
    }

    /// Shows the menu of views to open.
    pub fn show_open_menu(&self) {
        let mut a = self.menu_at;
        a.set(None);
        let mut menu = self.menu;
        menu.set(Some(Menu::Open));
    }

    /// Shows a phone's switcher of every pane.
    pub fn show_switcher(&self) {
        let mut switcher = self.switcher;
        switcher.set(true);
    }

    /// The pane's tab label: its title, else its view's.
    fn title(&self, pane: &Pane) -> String {
        if !pane.title.is_empty() {
            return pane.title.clone();
        }
        self.views
            .read()
            .get(&pane.view.0)
            .map_or_else(|| pane.view.0.clone(), |v| v.title.clone())
    }

    fn icon(&self, pane: &Pane) -> Option<IconFn> {
        self.views.read().get(&pane.view.0).and_then(|v| v.icon)
    }

    fn mount(&self, at: Mount, data: Rc<MountedData>) {
        let mut mounts = self.mounts;
        mounts.write().insert(at, data);
    }

    /// Measures `at`, in window points. Blitz's document is busy while
    /// events stream in (a drag), and a measurement then fails: it is
    /// asked again, a few milliseconds later, a few times.
    async fn rect(&self, at: Mount) -> Option<Rect> {
        let mounted = self.mounts.read().get(&at).cloned()?;
        for _ in 0..MEASURE_TRIES {
            if let Ok(r) = mounted.get_client_rect().await {
                return Some(Rect::new(
                    narrow(r.origin.x),
                    narrow(r.origin.y),
                    narrow(r.size.width),
                    narrow(r.size.height),
                ));
            }
            futures_timer::Delay::new(std::time::Duration::from_millis(4)).await;
        }
        None
    }

    /// A ribbon's pinned command pressed: the app's `on_command` hears it,
    /// from the main window's dom.
    fn press_command(&self, id: String) {
        if let Some(on) = *self.on_command.peek() {
            on.call(id);
        } else {
            let mut pressed = self.pressed;
            pressed.set(Some(id));
        }
    }

    /// Where the dock on `side` goes when it pops out without a place
    /// given: over where it was in the main window, a little in, as tall as
    /// the main window (the host's own area, where the platform does not
    /// say).
    fn dock_bounds(&self, side: Side) -> Option<Rect> {
        let windows = self.windows.peek().clone()?;
        let main = WindowId::main();
        let (ox, oy, area) = if let Some(r) = windows.0.content(&main) {
            (r.x, r.y, Rect::new(0.0, 0.0, r.w, r.h))
        } else {
            let (ox, oy) = windows.0.origin(&main)?;
            let area = (*self.float_area.peek()).filter(|a| a.w > 0.0 && a.h > 0.0)?;
            (ox, oy, area)
        };
        let w = self.workspace.peek();
        let dock = w.dock(side)?;
        let ribbon = if dock.ribbon.is_some() {
            self.sizes.peek().ribbon
        } else {
            0.0
        };
        let across = (dock.size + ribbon).max(240.0);
        Some(match side {
            Side::Right => Rect::new(
                ox + area.right() - across - 24.0,
                oy + area.y + 24.0,
                across,
                (area.h - 48.0).max(320.0),
            ),
            Side::Bottom | Side::Top => Rect::new(
                ox + area.x + 24.0,
                oy + area.bottom() - across - 24.0,
                (area.w - 48.0).max(480.0),
                across,
            ),
            Side::Left => Rect::new(
                ox + area.x + 24.0,
                oy + area.y + 24.0,
                across,
                (area.h - 48.0).max(320.0),
            ),
        })
    }

    /// Runs a menu row or a gesture's end.
    fn run(&self, op: Op) {
        let mut sheet = self.sheet;
        match op {
            Op::Move(pane, dest) => self.move_pane(&pane, dest, None),
            Op::Close(pane) => {
                self.finish_closing();
                let _ = self.edit(|w| w.close(&pane));
                if sheet.peek().as_ref() == Some(&pane) {
                    sheet.set(None);
                }
            }
            Op::CloseOthers(pane) => {
                let _ = self.edit(|w| w.close_others(&pane));
            }
            Op::Reopen => {
                let _ = self.edit(Workspace::reopen);
            }
            Op::Pin(pane, on) => {
                let _ = self.edit(|w| w.set_pinned(&pane, on));
            }
            Op::Stacked(group, on) => {
                let _ = self.edit(|w| w.set_stacked(group, on));
            }
            Op::Chrome(group, chrome) => {
                let _ = self.edit(|w| w.set_chrome(group, chrome));
            }
            Op::Dock(side, shown) => {
                let _ = self.edit(|w| w.set_dock_shown(side, shown));
            }
            Op::Open(kind) => {
                let opened = self.open(&kind);
                let docked = opened.as_ref().is_some_and(|id| {
                    self.workspace
                        .peek()
                        .locate(id)
                        .is_some_and(|l| l.area != Area::Center)
                });
                if (self.phone)() && docked {
                    sheet.set(opened);
                }
            }
            Op::OpenMenu => self.show_open_menu(),
            Op::Show(pane) => {
                let _ = self.edit(|w| w.activate(&pane));
            }
            Op::ToMain(pane) => {
                let _ = self.edit(|w| w.to_main_window(&pane));
            }
            Op::CloseWindow(window) => {
                let _ = self.edit(|w| w.close_window(&window));
            }
            Op::PopOutDock(side, bounds) => {
                if self.has_windows() {
                    let bounds = bounds.or_else(|| self.dock_bounds(side));
                    let _ = self.edit(|w| w.pop_out_dock(side, bounds));
                }
            }
            Op::DockBack(side) => {
                let _ = self.edit(|w| w.dock_back(side));
            }
            Op::Recall(slot) => self.write(
                crate::control::load,
                crate::control::LoadArgs {
                    if_revision: None,
                    id: None,
                    name: None,
                    slot: Some(slot),
                },
            ),
            // Run where the commands are (the menu row does it).
            Op::Run { .. } => {}
        }
    }
}

impl Host {
    /// Moves a pane; a split's new group grows in from the edge it was
    /// dropped on, laid out at `size` (where the drop lit) on the way.
    fn move_pane(&self, pane: &PaneId, dest: Dest, size: Option<Rect>) {
        self.finish_closing();
        let edge = match dest {
            Dest::Edge { side, .. } => Some(side),
            _ => None,
        };
        let moved = self.edit(|w| w.move_pane(pane, dest)).is_ok();
        let group = self.workspace.peek().locate(pane).map(|l| l.group);
        if let (Some(side), true, Some(group)) = (edge, moved, group) {
            let mut grow_size = self.grow_size;
            grow_size.set(size.map(|r| (r, Some(side))));
            self.grow_in(group);
        }
    }

    /// A group that just took a split's place grows into it.
    fn grow_in(&self, group: GroupId) {
        let mut growing = self.growing;
        growing.set(Some(group));
        self.grow.jump(0.0);
        let mut done = self.growing;
        self.grow.to_then(1.0, move || done.set(None));
    }

    /// Closes `pane` from its tab's ✕: the last tab of a group beside
    /// others gives its room up first, then closes.
    fn close_from_tab(&self, pane: PaneId) {
        let lone = {
            let w = self.workspace.peek();
            w.locate(&pane).is_some_and(|l| {
                w.group(l.group).is_some_and(|g| g.tabs.len() == 1)
                    && w.windows.get(l.window).is_some_and(|win| {
                        l.area == Area::Center && matches!(win.center, Some(Node::Split(_)))
                    })
            })
        };
        let group = self.workspace.peek().locate(&pane).map(|l| l.group);
        let (true, Some(group)) = (lone, group) else {
            self.run(Op::Close(pane));
            return;
        };
        self.finish_closing();
        let mut closing = self.closing;
        closing.set(Some(pane));
        // Laid out at the size it has as it goes (a group drawn since the
        // split was made may not be measurable: its box's last size).
        let size = self
            .split_sizes
            .peek()
            .get(&group)
            .copied()
            .or_else(|| self.estimate(group));
        let mut grow_size = self.grow_size;
        grow_size.set(size.map(|r| (r, None)));
        let mut growing = self.growing;
        growing.set(Some(group));
        let host = *self;
        self.grow.to_then(0.0, move || {
            let mut growing = host.growing;
            growing.set(None);
            host.finish_closing();
        });
    }

    /// Where a center group sits by its splits' shares, in the host's area
    /// less the side docks open beside the center.
    fn estimate(&self, group: GroupId) -> Option<Rect> {
        let area = (*self.float_area.peek())?;
        let w = self.workspace.peek();
        let main = w.windows.first()?;
        let ribbon = self.sizes.peek().ribbon;
        let beside = |d: &crate::Dock| {
            let open = d.shown == Shown::Open && d.root.is_some();
            let strip = if d.ribbon.is_some() { ribbon } else { 0.0 };
            if open { d.size + strip } else { strip }
        };
        let side = if *self.in_shell.peek() {
            0.0
        } else {
            beside(&main.docks.left)
        };
        let width = area.w - side - beside(&main.docks.right);
        let center = main.center.as_ref()?;
        crate::layout::area_rects(center, Rect::new(0.0, 0.0, width, area.h))
            .into_iter()
            .find(|p| p.group == group)
            .map(|p| p.rect)
    }

    /// A close still waiting for its group to give its room up: now.
    fn finish_closing(&self) {
        let mut closing = self.closing;
        let pane = closing.write().take();
        if let Some(pane) = pane {
            let mut growing = self.growing;
            if growing.peek().is_some() {
                growing.set(None);
            }
            self.grow.jump(1.0);
            let _ = self.edit(|w| w.close(&pane));
        }
    }
}

/// What a menu row or a gesture does.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Move(PaneId, Dest),
    Close(PaneId),
    CloseOthers(PaneId),
    Reopen,
    Pin(PaneId, bool),
    Stacked(GroupId, bool),
    Chrome(GroupId, crate::model::Chrome),
    Dock(Side, Shown),
    Open(String),
    OpenMenu,
    Show(PaneId),
    /// Back to the main window, from a pop-out.
    ToMain(PaneId),
    /// A pop-out closed: its tabs come back.
    CloseWindow(WindowId),
    /// The dock on this side into a window of its own (there, on screen,
    /// or where it was).
    PopOutDock(Side, Option<Rect>),
    /// A popped-out dock back to its side.
    DockBack(Side),
    /// Recall the screenset in slot n (⌘n).
    Recall(u8),
    /// Run another's entry (an extension's context-menu row): its
    /// command, told what the menu was opened on (the pane, its view, a
    /// file).
    Run {
        command: String,
        invocation: architect::action::Invocation,
    },
}

/// One row others add to a target's context menu: its label and the
/// command it runs (told the target: [`architect::action::Invocation`]),
/// ticked when the command is a toggle that is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextEntry {
    pub label: String,
    pub command: String,
    /// Whether its command is on, when it is a toggle; `None` when not.
    pub checked: Option<bool>,
}

/// What others add to a context menu (an extension's `[[context]]` rows).
///
/// Asked for by target and id each time the menu opens: a tab's
/// (`view:<kind>` and `tab`, the pane's id), a target a view shows
/// ([`PaneHandle::context_menu`]: `file` and a path). Provide it above the
/// workspace.
#[derive(Clone)]
pub struct ContextEntries(pub Rc<dyn Fn(&str, &str) -> Vec<ContextEntry>>);

impl ContextEntries {
    pub fn new(f: impl Fn(&str, &str) -> Vec<ContextEntry> + 'static) -> Self {
        Self(Rc::new(f))
    }
}

/// What a tab draws after its name: icons and badges.
///
/// Asked for by its pane's view kind and id each time it draws
/// (`architect-extensions` gives it, for what extensions set on tabs):
/// `None` when it has none. A badged tab keeps room for them and a few of
/// its name's letters, its name fading where it is cut: the badge is never
/// what is pushed out. Provide it above the workspace.
#[derive(Clone)]
pub struct TabBadges(pub Rc<dyn Fn(&str, &str) -> Option<Element>>);

impl TabBadges {
    pub fn new(f: impl Fn(&str, &str) -> Option<Element> + 'static) -> Self {
        Self(Rc::new(f))
    }
}

/// What others put in a pane's header, between its title and its menu.
///
/// An extension's `[[toolbars]]` with `where = "pane"` or
/// `"view:<kind>"`, asked for by the pane's view kind and id each time
/// the header draws: `None` for nothing. Return a component, so its hooks
/// (what keeps a toggle's state live) are its own. Provide it above the
/// workspace.
#[derive(Clone)]
pub struct HeaderTools(pub Rc<dyn Fn(&str, &PaneId) -> Option<Element>>);

impl HeaderTools {
    pub fn new(f: impl Fn(&str, &PaneId) -> Option<Element> + 'static) -> Self {
        Self(Rc::new(f))
    }
}

/// The invocation a context-menu row runs with.
///
/// Opened on `target` (`view:<kind>`, `tab`, `file`) `id` (the pane's, a
/// path) in pane `pane` showing `view`; a file's path is its
/// [`Invocation::path`] too.
///
/// [`Invocation::path`]: architect::action::Invocation::path
#[must_use]
pub fn context_invocation(
    target: &str,
    id: &str,
    pane: &PaneId,
    view: &str,
) -> architect::action::Invocation {
    let file = target == "file" || target.starts_with("file:");
    architect::action::Invocation {
        pane: Some(pane.0.clone()),
        view: Some(view.to_owned()),
        path: file.then(|| id.to_owned()),
        ..architect::action::Invocation::new(architect::action::InvocationSource::ContextMenu)
            .on(target, id)
    }
}

/// A measured length, as the model's points (window sizes fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// A count as points (no layout holds more than an `f32` counts).
#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn count_f32(n: usize) -> f32 {
    n as f32
}

/// Where a click was, in window points.
fn click_point(e: &MouseEvent) -> (f32, f32) {
    let p = e.client_coordinates();
    (narrow(p.x), narrow(p.y))
}

fn point(e: &PointerEvent) -> (f32, f32) {
    let p = e.data().client_coordinates();
    (narrow(p.x), narrow(p.y))
}

/// The workspace's main window: its docks around its center, its floats
/// over them, and the drag, menu and sheet layers over everything.
#[component]
pub fn WorkspaceView(
    /// The workspace to draw and change; where it is kept is the app's.
    workspace: Signal<Workspace>,
    views: Views,
    /// The commands a ribbon may pin.
    #[props(default)]
    commands: Vec<Command>,
    /// A pinned command pressed: its id.
    on_command: Option<EventHandler<String>>,
    /// The ids of `commands` the main area offers when no tab is open
    /// (Obsidian's "No file is open": New note), beside Go to file (⌘O)
    /// and, for a group that is not the main area's last, Close.
    #[props(default)]
    empty: Vec<String>,
    /// One pane at a time, with a switcher.
    #[props(default)]
    phone: bool,
    /// A finger or a pen: 44pt targets, wider dividers.
    #[props(default)]
    coarse: bool,
    /// The key held to arrange (W by default); `None` for an app that
    /// calls [`Host::set_arranging`] itself.
    #[props(default = Some(Code::KeyW))]
    arrange_key: Option<Code>,
    /// The window's title row, when the workspace reaches the window's top.
    title_row: Option<TitleRow>,
    /// The app's shell hosts the left dock: its groups in the shell's
    /// sidebar, its ribbon in the shell's ribbon (one left side). Read on
    /// mount.
    #[props(default)]
    in_shell: bool,
    /// On a phone, the bar over the pane (its title, the switcher). An app
    /// that shows them in its own bar turns it off and opens the switcher
    /// through `switcher`.
    #[props(default = true)]
    phone_header: bool,
    /// The phone's pane switcher is open: the app's, to open it from its
    /// own bar.
    switcher: Option<Signal<bool>>,
    /// The screensets (the app's built-in ones to begin with): ⌘1–9
    /// recall them for this screen's class, and the live layout is kept
    /// in them per class.
    screensets: Option<Signal<Screensets>>,
    /// The class of screen layouts are recalled for; else a desk, or a
    /// phone's (upright or on its side) when `phone`.
    form: Option<FormClass>,
    /// Where the screensets and the live layout are kept: read once when
    /// the host mounts, written when the workspace settles.
    store: Option<WorkspaceStore>,
    /// Where this device's working copy is kept, apart from the saved sets
    /// in `store` (which then go where they are shared: a project's
    /// settings, the person's synced ones).
    device_store: Option<WorkspaceStore>,
    /// What the app calls a saved layout, in its commands and the manage
    /// sheet: "Workspace" unless it says (Studio: "Screenset").
    sets_noun: Option<String>,
    /// Kept filled with the current document's own status items
    /// ([`use_pane_status`]), for the app to put on its status line.
    status: Option<Signal<Vec<StatusItem>>>,
    /// Contexts the app's views read, carried into pop-out windows (each a
    /// document of its own). The shell's and the commands' are carried
    /// already.
    #[props(default)]
    bridge: Bridge,
    /// The app's Action List: every workspace action
    /// ([`crate::control::ids`]) goes on it while the host is mounted, run
    /// by this host, and each change of the arrangement is a
    /// `Changed { topic: "workspace" }` event.
    actions: Option<WorkspaceActions>,
    /// Its colours; left out, the shell's (which follow the app's theme
    /// store), else the default.
    #[props(default, into)]
    theme: Option<ShellTheme>,
) -> Element {
    // Asked for on every render: a hook in a branch (`phone && …`) moved
    // every hook after it when a window was resized across a phone's width.
    let shell_now = architect_shell::try_use_shell();
    let in_a_shell = shell_now.is_some();
    let theme = theme
        .or_else(|| shell_now.map(|s| s.theme))
        .unwrap_or_default();
    let os_windows = try_use_context::<Windows>();
    let decks_now = deck::try_use_decks();
    let mut host = use_context_provider(|| Host {
        workspace,
        views: CopyValue::new(views.clone()),
        commands: CopyValue::new(commands.clone()),
        empty: CopyValue::new(empty.clone()),
        on_command: CopyValue::new(on_command),
        theme: Signal::new(theme.clone()),
        sizes: Signal::new(Sizes::FINE),
        phone: Signal::new(phone),
        title_row: Signal::new(None),
        decks: CopyValue::new(decks_now),
        in_shell: Signal::new(false),
        phone_header: Signal::new(phone_header),
        drag: Signal::new(None),
        menu: Signal::new(None),
        sheet: Signal::new(None),
        switcher: switcher.unwrap_or_else(|| Signal::new(false)),
        switch: switcher::Switch::new(),
        float_area: Signal::new(None),
        menu_at: Signal::new(None),
        mounts: CopyValue::new(HashMap::new()),
        held: Signal::new(false),
        latched: Signal::new(false),
        window: CopyValue::new(WindowId::main()),
        status: Signal::new(Vec::new()),
        status_serial: CopyValue::new(0),
        shipped: CopyValue::new(screensets.map_or_else(Vec::new, |s| {
            s.peek()
                .sets
                .iter()
                .filter(|s| s.builtin)
                .cloned()
                .collect()
        })),
        screensets: screensets.unwrap_or_else(|| Signal::new(Screensets::default())),
        form: Signal::new(FormClass::Desk),
        scrolls: Signal::new(HashMap::new()),
        strips: CopyValue::new(HashMap::new()),
        tab_shift: CopyValue::new(HashMap::new()),
        tab_from: CopyValue::new(HashMap::new()),
        lifted: Signal::new(None),
        growing: Signal::new(None),
        grow: Motion::new(1.0, Spring::PANEL, 0.001),
        grow_size: CopyValue::new(None),
        split_sizes: CopyValue::new(HashMap::new()),
        closing: CopyValue::new(None),
        windows: CopyValue::new(os_windows.clone()),
        pressed: Signal::new(None),
        released: CopyValue::new(Rc::default()),
        revisions: CopyValue::new(actions::Revisions::default()),
        measured: CopyValue::new(crate::tree::Measured::default()),
        sets_noun: Signal::new("Workspace".to_owned()),
        managing: Signal::new(false),
    });
    // Props change under a live host: keep it current.
    host.views.set(views);
    host.commands.set(commands);
    host.on_command.set(on_command);
    if *host.theme.peek() != theme {
        host.theme.set(theme.clone());
    }
    let sizes = if coarse { Sizes::COARSE } else { Sizes::FINE };
    if *host.sizes.peek() != sizes {
        host.sizes.set(sizes);
    }
    if *host.phone.peek() != phone {
        host.phone.set(phone);
    }
    let title_row = title_row.filter(|_| !phone);
    if *host.title_row.peek() != title_row {
        host.title_row.set(title_row);
    }
    // The top tab bars are the window's title row: the shell's own row
    // gives way to them (one row, not two).
    architect_shell::use_title_row(title_row.is_some());
    // One left side: the shell draws the dock and its ribbon, from here.
    let slots = (in_shell && !phone).then(|| {
        (
            rsx! { HostScope { host, dock::ShellLeftDock {} } },
            rsx! { HostScope { host, dock::ShellRibbon {} } },
        )
    });
    let (slot_left, slot_rail) = slots.unzip();
    architect_shell::use_shell_slots(slot_left, slot_rail);
    if *host.in_shell.peek() != in_shell {
        host.in_shell.set(in_shell);
    }
    // A window resized across a phone's width (a desk browser narrowed to
    // one) follows the prop, as `in_shell` does: one bar over the pane.
    if *host.phone_header.peek() != phone_header {
        host.phone_header.set(phone_header);
    }
    // A phone's class turns with it: its area, as measured.
    let area = if phone { *host.float_area.read() } else { None };
    let class = form.unwrap_or_else(|| match (phone, area) {
        (true, Some(r)) => FormClass::of(r.w, r.h, true),
        (true, None) => FormClass::PhonePortrait,
        (false, _) => FormClass::Desk,
    });
    if *host.form.peek() != class {
        host.form.set(class);
    }
    screens::use_kept(host, store, device_store);
    let noun = sets_noun.unwrap_or_else(|| "Workspace".to_owned());
    if *host.sets_noun.peek() != noun {
        host.sets_noun.set(noun);
    }
    actions::use_actions(host, actions);
    windows::use_release_outside(host);
    use_pressed_in_popouts(host);
    use_sidebar_follows_left_dock(host, in_shell && !phone);
    // The current document's own status items, for the app's status line.
    let items = host.status_items();
    if let Some(mut out) = status {
        if *out.peek() != items {
            out.set(items);
        }
    }
    // A phone's tab switcher takes the whole screen in a shell (its bars
    // too), else the host.
    let shell_cover = phone && in_a_shell;
    architect_shell::use_shell_cover(
        shell_cover.then(|| rsx! { HostScope { host, switcher::SwitcherLayer { cover: true } } }),
    );
    let t = theme;
    let body = if phone {
        rsx! { PhoneBody {} }
    } else {
        rsx! { DeskBody {} }
    };
    rsx! {
        // The scope tabs' commands and keys, while the app has decks.
        if let Some(handle) = decks_now {
            deck::DeckCommands { handle }
        }
        div {
            style: "position:relative; width:100%; height:100%; min-width:0; min-height:0; \
                    display:flex; overflow:hidden; background:{t.desk}; color:{t.ink}; \
                    font-family:{t.font}; font-size:13px;",
            tabindex: "0",
            onmounted: move |e| {
                host.mount(Mount::Host, e.data());
                spawn(async move {
                    let r = host.rect(Mount::Host).await;
                    host.float_area.set(r);
                });
            },
            onresize: move |e| {
                if let Ok(size) = e.get_content_box_size() {
                    let (x, y) = host.float_area.peek().map_or((0.0, 0.0), |r| (r.x, r.y));
                    let r = Rect::new(x, y, narrow(size.width), narrow(size.height));
                    host.float_area.set(Some(r));
                }
            },
            onpointermove: move |e| {
                if switcher::pulling(host) {
                    switcher::pull_move(host, point(&e).1);
                } else if host.drag.peek().is_some() {
                    overlay::on_move(host, point(&e));
                }
            },
            onpointerup: move |_| {
                if switcher::pulling(host) {
                    switcher::pull_end(host, true);
                } else if host.drag.peek().is_some() {
                    overlay::on_up(host);
                }
            },
            onpointercancel: move |_| switcher::pull_end(host, false),
            onkeydown: move |e| {
                let m = e.modifiers();
                let plain = !(m.meta() || m.ctrl() || m.alt());
                if plain && arrange_key == Some(e.code()) {
                    host.set_arranging(true);
                }
                if e.key() == Key::Named(NamedKey::Escape) {
                    host.drag.set(None);
                    host.menu.set(None);
                    slide::cancel(host);
                }
            },
            onkeyup: move |e| {
                if arrange_key == Some(e.code()) {
                    host.set_arranging(false);
                }
            },
            onblur: move |_| host.set_arranging(false),
            {body}
            commands::WorkspaceCommands {}
            if os_windows.is_some() && !phone {
                windows::PopoutSync { bridge }
            }
            // Under the app's shell, the shell's keeps the frames coming.
            if !in_shell {
                architect_shell::motion::MotionFrames {}
            }
            DropHighlight {}
            DragLayer {}
            MenuLayer {}
            SheetLayer {}
            manage::ManageSheet {}
            if phone && !shell_cover {
                switcher::SwitcherLayer { cover: false }
            }
        }
    }
}

/// A ribbon's command pressed in a pop-out reaches the app's `on_command`
/// here, in the main window's dom.
fn use_pressed_in_popouts(host: Host) {
    use_effect(move || {
        let mut pressed = host.pressed;
        let Some(id) = pressed() else {
            return;
        };
        pressed.set(None);
        if let Some(on) = *host.on_command.peek() {
            on.call(id);
        }
    });
}

/// Where the shell hosts the left dock: its sidebar folds while the dock
/// is in a window of its own, and unfolds when it docks back.
fn use_sidebar_follows_left_dock(host: Host, in_shell: bool) {
    let shell = try_use_context::<Signal<architect_shell::Shell>>();
    let mut was_out = use_hook(|| CopyValue::new(false));
    use_effect(move || {
        let out = host.workspace.read().dock_window(Side::Left).is_some();
        if out == *was_out.peek() {
            return;
        }
        was_out.set(out);
        let Some(shell) = shell.filter(|_| in_shell) else {
            return;
        };
        let shell = shell.peek().clone();
        if out && !shell.folded() {
            shell.toggle(architect_shell::Side::Left);
        } else if !out && shell.folded() {
            shell.unfold();
        }
    });
}

/// The docks around the center, the floats over them.
#[component]
fn DeskBody() -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let workspace = host.workspace.read();
    let Some(main) = workspace.windows.first() else {
        return rsx! {};
    };
    let center = shown_center(main);
    let docks = main.docks.clone();
    let floats: Vec<(usize, PaneId)> = main
        .floats
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.minimised)
        .filter_map(|(i, f)| f.group.current_tab().map(|p| (i, p.clone())))
        .collect();
    let minimised: Vec<PaneId> = main
        .floats
        .iter()
        .filter(|f| f.minimised)
        .filter_map(|f| f.group.current_tab().cloned())
        .collect();
    // Pop-outs with no OS window to show them in (a browser, an app not
    // on the native launcher): a chip each, a press brings their tabs back.
    let popouts: Vec<(WindowId, usize)> = if host.has_windows() {
        Vec::new()
    } else {
        workspace
            .windows
            .iter()
            .skip(1)
            .map(|w| {
                let mut groups = Vec::new();
                if let Some(c) = &w.center {
                    c.groups(&mut groups);
                }
                for side in crate::Docks::SIDES {
                    if let Some(root) = w.docks.get(side).and_then(|d| d.root.as_ref()) {
                        root.groups(&mut groups);
                    }
                }
                (w.id.clone(), groups.iter().map(|g| g.tabs.len()).sum())
            })
            .collect()
    };
    drop(workspace);
    let s = (host.sizes)();
    // What reaches the window's left and right edges at the top: an open
    // dock's tree, else the center (past a ribbon or a rail).
    let showing =
        |d: &crate::Dock| d.shown != Shown::Hidden && (d.root.is_some() || d.ribbon.is_some());
    let open_tree = |d: &crate::Dock| d.shown == Shown::Open && d.root.is_some();
    let strip = |d: &crate::Dock| match (showing(d), d.ribbon.is_some()) {
        (false, _) => 0.0,
        (true, true) => s.ribbon,
        (true, false) => RAIL,
    };
    let in_shell = (host.in_shell)();
    let center_edges = group::Edges {
        top: true,
        left: in_shell || !open_tree(&docks.left),
        right: !open_tree(&docks.right),
        covered: if in_shell { 0.0 } else { strip(&docks.left) },
        lead: true,
    };
    rsx! {
        div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:row;",
            if !in_shell {
                DockView { side: Side::Left, dock: docks.left }
            }
            div { style: "flex:1 1 0; min-width:{CENTER_MIN}px; min-height:0; display:flex; flex-direction:column;",
                div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative;",
                    {center.map_or_else(|| rsx! { EmptyCenter {} }, |node| rsx! { NodeView { node, edges: center_edges } })}
                }
                DockView { side: Side::Bottom, dock: docks.bottom }
            }
            DockView { side: Side::Right, dock: docks.right }
        }
        for (index, pane) in floats {
            FloatView { key: "{pane.0}", index, pane }
        }
        if !minimised.is_empty() || !popouts.is_empty() {
            div {
                style: "position:absolute; left:12px; bottom:12px; z-index:{architect_shell::layer::CHIPS}; display:flex; gap:6px;",
                for pane in minimised {
                    MinimisedChip { key: "{pane.0}", pane }
                }
                for (window, count) in popouts {
                    WindowChip { key: "{window.0}", window, count }
                }
            }
        }
        if host.arranging() {
            div {
                style: "position:absolute; right:12px; top:12px; z-index:{architect_shell::layer::STATUS}; padding:4px 10px; \
                        border-radius:999px; background:{t.accent}; color:{t.accent_ink}; \
                        font-size:11px; font-weight:700; pointer-events:none;",
                "Arranging"
            }
        }
    }
}

/// What a window's center draws: its tree, or its zoomed group alone.
fn shown_center(window: &crate::model::Window) -> Option<Node> {
    let center = window.center.as_ref()?;
    window
        .zoomed
        .and_then(|g| center.group(g).cloned())
        .map_or_else(|| Some(center.clone()), |g| Some(Node::Group(g)))
}

/// A minimised float, a press away from coming back.
#[component]
fn MinimisedChip(pane: PaneId) -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let title = host
        .workspace
        .read()
        .pane(&pane)
        .map(|p| host.title(p))
        .unwrap_or_default();
    rsx! {
        button {
            style: "min-height:32px; padding:0 12px; border-radius:8px; border:1px solid {t.rule}; \
                    background:{t.pill}; color:{t.ink}; font-size:12px;",
            onclick: move |_| {
                let _ = host.edit(|w| w.restore(&pane));
            },
            "{title}"
        }
    }
}

/// A pop-out with nowhere to show it: a press brings its tabs back.
#[component]
fn WindowChip(window: WindowId, count: usize) -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let tabs = if count == 1 { "tab" } else { "tabs" };
    rsx! {
        button {
            style: "min-height:32px; padding:0 12px; border-radius:8px; border:1px dashed {t.rule}; \
                    background:{t.pill}; color:{t.ink}; font-size:12px;",
            title: "Bring these tabs back",
            onclick: move |_| host.run(Op::CloseWindow(window.clone())),
            "Another window: {count} {tabs}"
        }
    }
}

#[component]
fn EmptyCenter() -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let s = (host.sizes)();
    rsx! {
        div {
            style: "flex:1; display:flex; align-items:center; justify-content:center; \
                    flex-direction:column; gap:12px; color:{t.ink_3};",
            div { "Nothing open" }
            button {
                style: "padding:0 16px; min-height:{s.button}px; border-radius:8px; \
                        border:1px solid {t.rule}; background:{t.pill}; color:{t.ink};",
                onclick: move |_| host.show_open_menu(),
                "Open a view"
            }
        }
    }
}

/// Every group in `node`, for the drop targets.
fn groups_of(node: &Node, out: &mut Vec<GroupId>) {
    let mut found = Vec::new();
    node.groups(&mut found);
    out.extend(found.into_iter().map(|g| g.id));
}

impl PartialEq for Host {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
    }
}

/// Puts `host` back in context for what is drawn elsewhere (the shell's
/// sidebar and ribbon, which are not under the [`WorkspaceView`]).
#[component]
fn HostScope(host: Host, children: Element) -> Element {
    use_context_provider(|| host);
    // The decks too: what the shell draws for the host (its sidebar, the
    // left dock's views) is outside the app's tree that holds them.
    let decks = *host.decks.peek();
    if let Some(decks) = decks {
        use_context_provider(|| decks);
    }
    children
}
