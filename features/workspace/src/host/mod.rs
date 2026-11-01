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

// Dioxus is single-threaded: its tasks are not `Send`, and a [`Host`] is a
// handful of signals copied into each handler. The `for` loops of `rsx!`
// iterate rows collected so the workspace's read is let go before drawing.
#![allow(
    clippy::future_not_send,
    clippy::large_types_passed_by_value,
    clippy::needless_collect
)]

mod commands;
mod dock;
mod group;
mod overlay;

use std::collections::HashMap;
use std::rc::Rc;

use architect_shell::{IconFn, ShellTheme};
use dioxus::prelude::*;

use crate::drop::Drop;
use crate::model::{
    Area, Axis, GroupId, Node, Pane, PaneId, Rect, Shown, Side, SplitId, ViewKind, Workspace,
};
use crate::ops::{Dest, Target};

use dock::DockView;
use group::NodeView;
use overlay::{DragLayer, FloatView, MenuLayer, PhoneBody, SheetLayer};

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
}

/// The views an app offers.
#[derive(Clone, Default)]
pub struct Views(Rc<Vec<View>>);

impl PartialEq for Views {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Views {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn view(mut self, view: View) -> Self {
        Rc::make_mut(&mut self.0).push(view);
        self
    }

    #[must_use]
    pub fn get(&self, kind: &str) -> Option<&View> {
        self.0.iter().find(|v| v.kind.0 == kind)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Mount {
    Host,
    Group(GroupId),
    Split(SplitId),
}

/// A drag in progress.
#[derive(Debug, Clone, PartialEq)]
enum Drag {
    /// A tab pressed, not yet moved far enough to drag.
    Pressed { pane: PaneId, x: f32, y: f32 },
    /// A tab being dragged; `geometry` is `None` until measured.
    Pane {
        pane: PaneId,
        geometry: Option<(Rect, Vec<(GroupId, Rect)>)>,
        at: Option<Drop>,
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
        tabs: 36.0,
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
    on_command: CopyValue<Option<EventHandler<String>>>,
    theme: CopyValue<ShellTheme>,
    sizes: Signal<Sizes>,
    phone: Signal<bool>,
    title_row: Signal<Option<TitleRow>>,
    in_shell: Signal<bool>,
    /// A phone shows its own bar over the pane (the title, the switcher).
    phone_header: Signal<bool>,
    drag: Signal<Option<Drag>>,
    menu: Signal<Option<Menu>>,
    /// A phone's pane opened over the one on show (a docked one).
    sheet: Signal<Option<PaneId>>,
    switcher: Signal<bool>,
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
}

/// The [`Host`] a [`WorkspaceView`] above put in context.
#[must_use]
pub fn use_host() -> Host {
    use_context()
}

/// The pane a slot draws, kept current: Dioxus may hand a slot another
/// pane (a tab closed beside it) without making it afresh.
#[derive(Clone, Copy)]
struct ThisPane(Signal<PaneId>);

/// The pane this component draws, inside a [`View`]'s render.
#[must_use]
pub fn use_pane() -> PaneHandle {
    PaneHandle {
        id: use_context::<ThisPane>().0(),
        host: use_host(),
    }
}

/// A pane, from inside it.
#[derive(Clone)]
pub struct PaneHandle {
    pub id: PaneId,
    host: Host,
}

impl PaneHandle {
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
}

impl Host {
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

    /// Runs a menu row or a gesture's end.
    fn run(&self, op: Op) {
        let mut sheet = self.sheet;
        match op {
            Op::Move(pane, dest) => {
                let _ = self.edit(|w| w.move_pane(&pane, dest));
            }
            Op::Close(pane) => {
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
}

/// A measured length, as the model's points (window sizes fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
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
    #[props(default)] theme: ShellTheme,
) -> Element {
    let mut host = use_context_provider(|| Host {
        workspace,
        views: CopyValue::new(views.clone()),
        commands: CopyValue::new(commands.clone()),
        on_command: CopyValue::new(on_command),
        theme: CopyValue::new(theme),
        sizes: Signal::new(Sizes::FINE),
        phone: Signal::new(phone),
        title_row: Signal::new(None),
        in_shell: Signal::new(false),
        phone_header: Signal::new(phone_header),
        drag: Signal::new(None),
        menu: Signal::new(None),
        sheet: Signal::new(None),
        switcher: switcher.unwrap_or_else(|| Signal::new(false)),
        float_area: Signal::new(None),
        menu_at: Signal::new(None),
        mounts: CopyValue::new(HashMap::new()),
        held: Signal::new(false),
        latched: Signal::new(false),
    });
    // Props change under a live host: keep it current.
    host.views.set(views);
    host.commands.set(commands);
    host.on_command.set(on_command);
    host.theme.set(theme);
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
    let t = theme;
    let body = if phone {
        rsx! { PhoneBody {} }
    } else {
        rsx! { DeskBody {} }
    };
    rsx! {
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
                if host.drag.peek().is_some() {
                    overlay::on_move(host, point(&e));
                }
            },
            onpointerup: move |_| {
                if host.drag.peek().is_some() {
                    overlay::on_up(host);
                }
            },
            onkeydown: move |e| {
                let m = e.modifiers();
                let plain = !(m.meta() || m.ctrl() || m.alt());
                if plain && arrange_key == Some(e.code()) {
                    host.set_arranging(true);
                }
                if e.key() == Key::Escape {
                    host.drag.set(None);
                    host.menu.set(None);
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
            DragLayer {}
            MenuLayer {}
            SheetLayer {}
        }
    }
}

/// The docks around the center, the floats over them.
#[component]
fn DeskBody() -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let workspace = host.workspace.read();
    let Some(main) = workspace.windows.first() else {
        return rsx! {};
    };
    let center = main.center.clone();
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
        if !minimised.is_empty() {
            div {
                style: "position:absolute; left:12px; bottom:12px; z-index:{architect_shell::layer::CHIPS}; display:flex; gap:6px;",
                for pane in minimised {
                    MinimisedChip { key: "{pane.0}", pane }
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

/// A minimised float, a press away from coming back.
#[component]
fn MinimisedChip(pane: PaneId) -> Element {
    let host = use_host();
    let t = *host.theme.read();
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

#[component]
fn EmptyCenter() -> Element {
    let host = use_host();
    let t = *host.theme.read();
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
    children
}
