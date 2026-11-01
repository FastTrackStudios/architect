//! The shell's Dioxus half (feature `ui`): [`AppShell`] and everything it
//! draws.

mod bars;
mod motion;
mod page;
mod workspaces;

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use lucide_dioxus::{PanelLeft, PanelLeftClose, PanelRight};

pub use crate::screen::{FormFactor, Insets, Screen, ShellStyle};
use bars::{BottomBar, Rail, RoundButton, SideTab};
pub use page::{
    Breadcrumb, Crumb, Menu, MenuEntry, MenuItem, Page, PageBar, Tool, ToolGroup, Toolbar,
};
use workspaces::{Place, Switcher};
pub use workspaces::{Workspace, WorkspaceAction, Workspaces};

#[doc(hidden)]
pub use dioxus::prelude::rsx as __rsx;

/// How an icon is drawn where the shell puts it. Blitz takes an SVG's
/// colour as an attribute, not from CSS, so the shell says it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Icon {
    pub size: usize,
    pub color: &'static str,
}

/// Draws an icon at the size and colour the shell asks for. Write one with
/// [`icon!`](crate::icon): `icon!(House)`.
pub type IconFn = fn(Icon) -> Element;

/// An [`IconFn`] for a lucide (or any `size`/`color`) icon component:
/// `icon!(House)`.
#[macro_export]
macro_rules! icon {
    ($name:ident) => {
        (|i: $crate::Icon| $crate::__rsx! { $name { size: i.size, color: i.color } })
            as $crate::IconFn
    };
}

/// One of the app's sections. The first four are the bottom bar.
#[derive(Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: IconFn,
    /// Something waiting there ("3"); `None` for nothing.
    pub badge: Option<String>,
}

impl Tab {
    #[must_use]
    pub fn new(id: &'static str, label: &'static str, icon: IconFn) -> Self {
        Self {
            id,
            label,
            icon,
            badge: None,
        }
    }

    /// Mark the tab with what is waiting there.
    #[must_use]
    // By value, so a count reads as `.badge(5)`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn badge(mut self, badge: impl ToString) -> Self {
        self.badge = Some(badge.to_string());
        self
    }
}

/// A button: the separate one at the bottom, or a page's main action.
#[derive(Clone, PartialEq)]
pub struct Action {
    pub label: String,
    pub icon: IconFn,
    pub on_press: Callback<()>,
    /// Say the label beside the icon ("Process 5"), not just show it.
    pub show_label: bool,
}

impl Action {
    pub fn new(label: impl Into<String>, icon: IconFn, on_press: impl FnMut(()) + 'static) -> Self {
        Self {
            label: label.into(),
            icon,
            on_press: Callback::new(on_press),
            show_label: false,
        }
    }

    /// Say the label beside the icon.
    #[must_use]
    pub const fn with_label(mut self) -> Self {
        self.show_label = true;
        self
    }
}

/// Who is signed in, at the bottom left: a row at the foot of a docked
/// sidebar and of a phone's drawer, the avatar alone at the foot of a rail.
#[derive(Clone, PartialEq)]
pub struct Account {
    pub name: String,
    /// A second line: the org, an email.
    pub detail: Option<String>,
    /// The avatar's letters (from the name unless given).
    pub initials: String,
    /// The avatar's fill.
    pub color: &'static str,
    /// What a press on it does (the account menu, settings).
    pub on_press: Option<Callback<()>>,
}

impl Account {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let initials = name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase();
        Self {
            name,
            detail: None,
            initials,
            color: "#1e3a8a",
            on_press: None,
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub const fn color(mut self, color: &'static str) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn on_press(mut self, on_press: impl FnMut(()) + 'static) -> Self {
        self.on_press = Some(Callback::new(on_press));
        self
    }
}

/// A desk window that draws its own title bar.
///
/// macOS: the content under a transparent title bar, the window's buttons
/// over it. The shell keeps
/// the buttons' corner clear, lines its top bars up with them, and lets
/// their empty space move the window. The host owns the window: it passes
/// what dragging and zooming it are.
#[derive(Clone, PartialEq)]
pub struct WindowChrome {
    /// The title bar's height: the row the window's buttons sit centred in.
    pub height: f64,
    /// How far from the left edge the window's buttons reach.
    pub buttons: f64,
    /// Start moving the window (call from the press itself).
    pub on_drag: Callback<()>,
    /// Zoom the window (a double click on the bar).
    pub on_zoom: Callback<()>,
}

impl WindowChrome {
    /// macOS with a unified (toolbar-height) title bar: the traffic lights
    /// centred in a 52pt row, reaching 78pt in.
    pub fn mac(on_drag: impl FnMut(()) + 'static, on_zoom: impl FnMut(()) + 'static) -> Self {
        Self {
            height: 52.0,
            buttons: 78.0,
            on_drag: Callback::new(on_drag),
            on_zoom: Callback::new(on_zoom),
        }
    }
}

/// The shell's colours. The default is `FastTrackStudio`'s dark.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShellTheme {
    /// Under everything.
    pub desk: &'static str,
    /// The sidebars, the drawers, the menus.
    pub sheet: &'static str,
    /// The pills.
    pub pill: &'static str,
    /// A pill's edge, a sidebar's edge.
    pub rule: &'static str,
    /// The current tab's fill, a pressed row's.
    pub active: &'static str,
    pub ink: &'static str,
    pub ink_2: &'static str,
    pub ink_3: &'static str,
    /// Badges.
    pub accent: &'static str,
    pub accent_ink: &'static str,
    /// A destructive menu row.
    pub danger: &'static str,
    /// Over the page while a drawer or a menu is open.
    pub scrim: &'static str,
    pub font: &'static str,
}

impl Default for ShellTheme {
    fn default() -> Self {
        Self {
            desk: "#09090b",
            sheet: "#0e0e11",
            pill: "#1a1a1f",
            rule: "#2b2b31",
            active: "rgba(255,255,255,0.10)",
            ink: "#e4e4e7",
            ink_2: "#a1a1aa",
            ink_3: "#80808a",
            accent: "#2563eb",
            accent_ink: "#ffffff",
            danger: "#f87171",
            scrim: "rgba(0,0,0,0.55)",
            font: "-apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif",
        }
    }
}

// ── The shell's state ───────────────────────────────────────────────────────

/// A side of the shell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

/// What the shell tells its pages, and what they may ask of it: read with
/// [`use_shell`].
#[derive(Clone, PartialEq)]
pub struct Shell {
    pub screen: Screen,
    pub style: ShellStyle,
    pub theme: ShellTheme,
    /// The side open over the page (a phone's drawer, or a side that is
    /// not docked at this size), if any.
    pub drawer: Signal<Option<Side>>,
    /// Whether the docked left sidebar is folded to a rail (`None`: as the
    /// size has it).
    fold: Signal<Option<bool>>,
    /// Whether the docked right side is put away.
    right_away: Signal<bool>,
    has_left: bool,
    has_right: bool,
    separate: Option<Action>,
    account: Option<Account>,
    workspaces: Option<Workspaces>,
    window: Option<WindowChrome>,
    /// The app's brand, leading a phone's root pages.
    brand: Option<Element>,
    /// Whether the separate button floats at the page's bottom right (a
    /// tablet) rather than sitting in a bar or a rail.
    floats_separate: bool,
}

impl Shell {
    #[must_use]
    pub fn form(&self) -> FormFactor {
        self.screen.form()
    }

    /// Open a side over the page, or close it with `None`.
    pub fn open(&self, side: Option<Side>) {
        let mut drawer = self.drawer;
        drawer.set(side);
    }

    #[must_use]
    pub const fn has_side(&self, side: Side) -> bool {
        match side {
            Side::Left => self.has_left,
            Side::Right => self.has_right,
        }
    }

    /// Whether `side` is docked beside the page at this size (as opposed to
    /// opening over it).
    #[must_use]
    pub fn docked(&self, side: Side) -> bool {
        match (side, frame(self.screen, self.style)) {
            (_, Frame::Bottom | Frame::Rail) => false,
            (Side::Left, Frame::Sidebar) => !self.folded(),
            (Side::Right, Frame::Sidebar) => self.screen.width >= DOCK_RIGHT,
        }
    }

    /// Whether the docked left sidebar is folded to a rail.
    #[must_use]
    pub fn folded(&self) -> bool {
        (self.fold)().unwrap_or(self.screen.width < FOLD_BELOW)
    }

    /// The side's button: a docked side folds away or comes back; one that
    /// is not docked opens over the page, or closes.
    pub fn toggle(&self, side: Side) {
        let mut drawer = self.drawer;
        if *drawer.peek() == Some(side) {
            drawer.set(None);
            return;
        }
        match (side, frame(self.screen, self.style)) {
            (Side::Left, Frame::Sidebar) => {
                let mut fold = self.fold;
                fold.set(Some(!self.folded()));
            }
            (Side::Right, Frame::Sidebar) if self.docked(Side::Right) => {
                let mut away = self.right_away;
                away.toggle();
            }
            _ => drawer.set(Some(side)),
        }
    }

    /// Whether this is a desk: its sidebar hides rather than folding to a
    /// rail, its sides' buttons sit fixed in the title row, its separate
    /// button floats at the bottom right.
    #[must_use]
    pub fn desk(&self) -> bool {
        self.form() == FormFactor::Desk
    }

    /// The title row's height: the window's buttons' row, or 52.
    #[must_use]
    pub fn title_row(&self) -> f64 {
        self.window.as_ref().map_or(TITLE_ROW, |w| w.height)
    }

    /// Where a desk's sidebar button sits: just past the window's buttons.
    #[must_use]
    pub fn controls_left(&self) -> f64 {
        self.screen.insets.left + self.window.as_ref().map_or(12.0, |w| w.buttons + 12.0)
    }

    /// Where a desk's title-row content starts, clear of the window's
    /// buttons and the sidebar button.
    #[must_use]
    pub fn content_left(&self) -> f64 {
        self.controls_left() + CONTROL + 10.0
    }

    /// The window's own title bar, when the app draws it.
    #[must_use]
    pub const fn window_chrome(&self) -> Option<&WindowChrome> {
        self.window.as_ref()
    }

    /// What a desk page that draws its own title row keeps clear, in
    /// points from the left and the right edge: the window's buttons and
    /// the sidebar's (unless a sidebar is docked beside the page), and the
    /// right side's button when it is over the page.
    #[must_use]
    pub fn title_clearance(&self) -> (f64, f64) {
        let left = if self.has_side(Side::Left) && self.docked(Side::Left) {
            0.0
        } else if self.has_side(Side::Left) {
            self.content_left()
        } else if self.window.is_some() {
            self.controls_left()
        } else {
            0.0
        };
        let right = if self.right_button_over_page() {
            12.0 + CONTROL + 14.0
        } else {
            0.0
        };
        (left, right + self.screen.insets.right)
    }

    /// Whether the desk's right side's button is over the page's title row
    /// (the side is not docked and showing), so the page keeps clear of it.
    #[must_use]
    pub fn right_button_over_page(&self) -> bool {
        self.desk() && self.has_right && !(self.docked(Side::Right) && !(self.right_away)())
    }

    /// Unfold the docked left sidebar from its rail.
    pub fn unfold(&self) {
        let mut fold = self.fold;
        fold.set(Some(false));
    }
}

/// The shell around this page (kept current through a resize).
#[must_use]
pub fn use_shell() -> Shell {
    use_context::<Signal<Shell>>()()
}

/// How the bars and the rail are laid out for this screen and style.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frame {
    /// The tabs along the bottom.
    Bottom,
    /// The tabs down a rail on the left (a phone on its side, `RailOnly`).
    Rail,
    /// The tabs at the head of a docked left sidebar, which may fold.
    Sidebar,
}

fn frame(screen: Screen, style: ShellStyle) -> Frame {
    match screen.form() {
        FormFactor::Phone if style == ShellStyle::RailOnly || screen.landscape() => Frame::Rail,
        FormFactor::Phone => Frame::Bottom,
        FormFactor::Tablet | FormFactor::Desk => Frame::Sidebar,
    }
}

/// How far a stroke goes before it is a sideways swipe or a pull.
const DECIDE: f64 = 12.0;
/// A sideways stroke this long (or this fast) opens or closes a drawer.
const SWIPE: f64 = 70.0;
/// A pull this long, from the top, is the app's [`AppShell::on_pull`].
const PULL: f64 = 90.0;
/// Where a pull must start: this far below the top inset.
const PULL_ZONE: f64 = 80.0;
/// From this width a tablet or desk docks the right side.
const DOCK_RIGHT: f64 = 1100.0;
/// Below this width the docked left sidebar starts folded to a rail.
const FOLD_BELOW: f64 = 1000.0;
/// A desk's title row without a window of its own: T3 Code's.
const TITLE_ROW: f64 = 52.0;
/// A desk's title-row controls (the sides' buttons).
const CONTROL: f64 = 28.0;
/// How far the page follows an opening drawer, as a share of its width.
const PARALLAX: f64 = 0.22;
/// The docked sidebars' widths: the start, and the range a drag keeps to.
const LEFT_WIDTH: f64 = 264.0;
const RIGHT_WIDTH: f64 = 320.0;
const MIN_WIDTH: f64 = 200.0;
const MAX_WIDTH: f64 = 440.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stroke {
    Undecided,
    Sideways,
    Down,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Touch {
    x0: f64,
    y0: f64,
    dx: f64,
    dy: f64,
    stroke: Stroke,
    /// The drawer open when the stroke began.
    open: Option<Side>,
}

/// A docked sidebar's edge being dragged: which, from where, from what width.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Resize {
    side: Side,
    x0: f64,
    w0: f64,
}

/// The shell: mount once at the app's root. It fills its parent, which
/// must be positioned (the window's root, `inset: 0`).
#[component]
pub fn AppShell(
    screen: Screen,
    #[props(default)] style: ShellStyle,
    #[props(default)] theme: ShellTheme,
    /// The app's name, at the head of a docked sidebar.
    #[props(into)]
    title: Option<String>,
    tabs: Vec<Tab>,
    /// The current tab's id; `None` when the page is under none of them.
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
    separate: Option<Action>,
    /// Who is signed in, at the bottom left.
    account: Option<Account>,
    /// The organization / workspace switcher, at the head of the sidebar.
    workspaces: Option<Workspaces>,
    /// The app's left side: what is beside the tabs in the sidebar, and
    /// a phone's left drawer.
    left: Option<Element>,
    /// The app's right side: docked on a wide screen, over the page
    /// otherwise.
    right: Option<Element>,
    /// A pull down from the top of a phone's screen (the command palette).
    on_pull: Option<EventHandler<()>>,
    /// A desk window that draws its own title bar ([`WindowChrome`]).
    window: Option<WindowChrome>,
    /// The app's brand (its mark and name): at the top left of a phone's
    /// root pages, in place of the page's title (T3 Code's).
    brand: Option<Element>,
    children: Element,
) -> Element {
    let mut drawer = use_signal(|| None::<Side>);
    let fold = use_signal(|| None::<bool>);
    let right_away = use_signal(|| false);
    let left_w = use_signal(|| LEFT_WIDTH);
    let right_w = use_signal(|| RIGHT_WIDTH);
    let has_left = left.is_some();
    let has_right = right.is_some();
    let shell = Shell {
        screen,
        style,
        theme,
        drawer,
        fold,
        right_away,
        has_left,
        has_right,
        separate: separate.clone(),
        account: account.clone(),
        workspaces: workspaces.clone(),
        window: window.clone(),
        brand,
        floats_separate: frame(screen, style) == Frame::Sidebar,
    };
    let mut ctx = use_context_provider(|| Signal::new(shell.clone()));
    if *ctx.peek() != shell {
        ctx.set(shell.clone());
    }

    let form = screen.form();
    let phone = form == FormFactor::Phone;
    let frame = frame(screen, style);
    let t = theme;

    // The drawer's travel, 0 (away) to 1 (open), and which side it is:
    // driven by the finger while a swipe lasts, animated otherwise.
    let motion = motion::use_motion();
    let mut shown = use_signal(|| None::<Side>);
    use_effect(move || {
        let target = drawer();
        if let Some(side) = target {
            if *shown.peek() != Some(side) {
                shown.set(Some(side));
            }
            motion.to(1.0, move || {});
        } else {
            motion.to(0.0, move || shown.set(None));
        }
    });

    // A press on a tab also closes a drawer.
    let pick = move |id: &'static str| {
        drawer.set(None);
        on_tab.call(id);
    };

    // The stroke under way, heard as pointer events: Blitz makes them of a
    // finger and of a mouse alike, so a phone-sized desk window takes the
    // same swipes. Kept out of signals: a tap must not redraw the shell
    // between its press and its release.
    let touch = use_hook(|| Rc::new(Cell::new(None::<Touch>)));
    let resize = use_hook(|| Rc::new(Cell::new(None::<Resize>)));
    let drawer_w = if phone {
        (screen.width * 0.86).min(340.0)
    } else {
        340.0
    };

    let (touch_at, resize_at) = (touch.clone(), resize.clone());
    let onpointerdown = move |e: PointerEvent| {
        if !phone || resize_at.get().is_some() {
            return;
        }
        let c = e.client_coordinates();
        touch_at.set(Some(Touch {
            x0: c.x,
            y0: c.y,
            dx: 0.0,
            dy: 0.0,
            stroke: Stroke::Undecided,
            open: *drawer.peek(),
        }));
    };
    let (touch_mv, resize_mv) = (touch.clone(), resize.clone());
    let onpointermove = move |e: PointerEvent| {
        let c = e.client_coordinates();
        if let Some(r) = resize_mv.get() {
            let dx = c.x - r.x0;
            let (mut w, grow) = match r.side {
                Side::Left => (left_w, dx),
                Side::Right => (right_w, -dx),
            };
            w.set((r.w0 + grow).clamp(MIN_WIDTH, MAX_WIDTH));
            return;
        }
        let Some(mut s) = touch_mv.get() else { return };
        s.dx = c.x - s.x0;
        s.dy = c.y - s.y0;
        if s.stroke == Stroke::Undecided && s.dx.hypot(s.dy) >= DECIDE {
            s.stroke = if s.dx.abs() > s.dy.abs() {
                Stroke::Sideways
            } else if s.dy > 0.0 && s.y0 < screen.insets.top + PULL_ZONE && s.open.is_none() {
                Stroke::Down
            } else {
                // Up or down the page: the page's own scroll.
                touch_mv.set(None);
                return;
            };
        }
        touch_mv.set(Some(s));
        if s.stroke == Stroke::Sideways {
            // The drawer under the finger: opening from the closed side,
            // or closing the one that is open.
            let (side, amount) = match (s.open, s.dx > 0.0) {
                (Some(Side::Left), _) => (Side::Left, 1.0 + s.dx.min(0.0) / drawer_w),
                (Some(Side::Right), _) => (Side::Right, 1.0 - s.dx.max(0.0) / drawer_w),
                (None, true) if has_left => (Side::Left, s.dx / drawer_w),
                (None, false) if has_right => (Side::Right, -s.dx / drawer_w),
                _ => return,
            };
            if *shown.peek() != Some(side) {
                shown.set(Some(side));
            }
            motion.hold(amount.clamp(0.0, 1.0));
        }
    };
    let (touch_end, resize_end) = (touch.clone(), resize.clone());
    let onpointerup = move |_: PointerEvent| {
        if resize_end.take().is_some() {
            return;
        }
        let Some(s) = touch_end.take() else { return };
        match s.stroke {
            Stroke::Sideways => {
                let target = match (s.open, s.dx) {
                    (Some(Side::Left), dx) if dx <= -SWIPE => None,
                    (Some(Side::Right), dx) if dx >= SWIPE => None,
                    (None, dx) if dx >= SWIPE && has_left => Some(Side::Left),
                    (None, dx) if dx <= -SWIPE && has_right => Some(Side::Right),
                    (open, _) => open,
                };
                if *drawer.peek() == target {
                    // Not far enough: back to where it was.
                    let to = if target.is_some() { 1.0 } else { 0.0 };
                    motion.to(to, move || {
                        if target.is_none() {
                            shown.set(None);
                        }
                    });
                } else {
                    drawer.set(target);
                }
            }
            Stroke::Down if s.dy >= PULL => {
                if let Some(pull) = on_pull {
                    pull.call(());
                }
            }
            _ => {}
        }
    };
    let (touch_cancel, resize_cancel) = (touch, resize.clone());
    let onpointercancel = move |_: PointerEvent| {
        resize_cancel.set(None);
        if touch_cancel
            .take()
            .is_some_and(|s| s.stroke == Stroke::Sideways)
        {
            let open = *drawer.peek();
            motion.to(if open.is_some() { 1.0 } else { 0.0 }, move || {
                if open.is_none() {
                    shown.set(None);
                }
            });
        }
    };
    let start_resize = move |side: Side, e: PointerEvent| {
        let w0 = match side {
            Side::Left => *left_w.peek(),
            Side::Right => *right_w.peek(),
        };
        resize.set(Some(Resize {
            side,
            x0: e.client_coordinates().x,
            w0,
        }));
    };
    let start_left = start_resize.clone();
    let start_right = start_resize;

    let amount = motion.value();
    let shown_side = shown();
    let over = shown_side.is_some() && amount > 0.001;
    let left_x = -drawer_w * (1.0 - amount);
    let right_x = screen.width - drawer_w * amount;
    // On a phone the page gives way a little to the drawer coming in.
    let page_shift = match shown_side {
        Some(Side::Left) if phone => drawer_w * PARALLAX * amount,
        Some(Side::Right) if phone => -drawer_w * PARALLAX * amount,
        _ => 0.0,
    };

    let docked_left = frame == Frame::Sidebar && !shell.folded();
    let docked_right = has_right && shell.docked(Side::Right) && !right_away();
    let float_separate = shell.floats_separate;
    let in_bar = frame == Frame::Bottom;
    let direction = if in_bar { "column" } else { "row" };
    // The sidebar's and the rail's top, clear of the window's controls.
    // The rail's top, clear of the window's buttons. (A docked sidebar puts
    // them in its head's first row instead.)
    let chrome_h = window.as_ref().map_or(0.0, |w| w.height);
    let side_screen = Screen {
        insets: Insets {
            top: screen.insets.top.max(chrome_h),
            ..screen.insets
        },
        ..screen
    };
    let lw = left_w();
    let rw = right_w();
    let resizable = form == FormFactor::Desk;

    rsx! {
        div {
            style: "position: relative; width: 100%; height: 100%; overflow: hidden; display: flex; \
                    flex-direction: {direction}; background: {t.desk}; color: {t.ink}; font-family: {t.font};",
            onpointerdown,
            onpointermove,
            onpointerup,
            onpointercancel,
            match frame {
                Frame::Rail => rsx! {
                    Rail { tabs: tabs.clone(), current, on_tab: pick, screen: side_screen }
                },
                Frame::Sidebar if !docked_left && form == FormFactor::Desk => rsx! {},
                Frame::Sidebar if !docked_left => rsx! {
                    Rail { tabs: tabs.clone(), current, on_tab: pick, screen: side_screen }
                },
                Frame::Sidebar => rsx! {
                    div {
                        style: "position: relative; width: {lw}px; flex-shrink: 0; display: flex; flex-direction: column; \
                                background: {t.sheet}; border-right: 1px solid {t.rule}; \
                                padding-top: {screen.insets.top}px; padding-left: {screen.insets.left}px;",
                        if form == FormFactor::Desk {
                            DeskSidebarHead { title: title }
                        } else {
                            SidebarHead { title: title }
                        }
                        div { style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 10px 6px;",
                            for tab in tabs.iter().cloned() {
                                SideTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab: pick }
                            }
                        }
                        if let Some(left) = left.clone() {
                            div { style: "height: 1px; background: {t.rule}; margin: 8px 18px;" }
                            div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column; overflow-y: scroll;",
                                {left}
                            }
                        } else {
                            span { style: "flex: 1;" }
                        }
                        if let Some(account) = account.clone() {
                            AccountRow { account, bottom: screen.insets.bottom }
                        }
                        if resizable {
                            ResizeEdge { side: Side::Left, on_start: move |e| start_left(Side::Left, e) }
                        }
                    }
                },
                Frame::Bottom => rsx! {},
            }
            // The page.
            div {
                style: "position: relative; flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; \
                        transform: translateX({page_shift}px);",
                {children}
                if float_separate {
                    if let Some(sep) = separate.clone() {
                        div {
                            style: "position: absolute; right: {20.0 + screen.insets.right}px; \
                                    bottom: {20.0 + screen.insets.bottom}px;",
                            RoundButton { action: sep }
                        }
                    }
                }
            }
            if docked_right {
                if let Some(right) = right.clone() {
                    div {
                        style: "position: relative; width: {rw}px; flex-shrink: 0; display: flex; flex-direction: column; \
                                overflow-y: scroll; background: {t.sheet}; border-left: 1px solid {t.rule}; \
                                padding-top: {screen.insets.top}px; padding-right: {screen.insets.right}px;",
                        // A desk's title row, under the side's fixed button.
                        if form == FormFactor::Desk {
                            div { style: "height: {shell.title_row()}px; flex-shrink: 0; display: flex;",
                                if let Some(w) = window {
                                    DragStrip { chrome: w, width: None }
                                }
                            }
                        }
                        {right}
                        if resizable {
                            ResizeEdge { side: Side::Right, on_start: move |e| start_right(Side::Right, e) }
                        }
                    }
                }
            }
            if in_bar {
                BottomBar { tabs: tabs, current, on_tab: pick, separate: separate, screen }
            }
            // A desk's sides' buttons, fixed in the title row whether the
            // side is showing or not (T3 Code's): the left one just past the
            // window's buttons, the right one at the top right.
            if form == FormFactor::Desk {
                if has_left {
                    TitleControl { side: Side::Left }
                }
                if has_right {
                    TitleControl { side: Side::Right }
                }
            }
            // A side open over the page.
            if over {
                div {
                    style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; \
                            background: {t.scrim}; opacity: {amount.clamp(0.0, 1.0)};",
                    onclick: move |_| drawer.set(None),
                }
                match shown_side {
                    Some(Side::Left) => rsx! {
                        if let Some(left) = left {
                            Drawer { side: Side::Left, x: left_x, width: drawer_w, screen,
                                if let Some(workspaces) = workspaces {
                                    div { style: "display: flex; padding: 0 10px 8px;",
                                        Switcher { workspaces, place: Place::Head }
                                    }
                                }
                                {left}
                                if let Some(account) = account {
                                    span { style: "flex: 1;" }
                                    AccountRow { account, bottom: 0.0 }
                                }
                            }
                        }
                    },
                    Some(Side::Right) => rsx! {
                        if let Some(right) = right {
                            Drawer { side: Side::Right, x: right_x, width: drawer_w, screen, {right} }
                        }
                    },
                    None => rsx! {},
                }
            }
        }
    }
}

/// A side open over the page: a sheet the full height of the screen, its
/// edge shadowed, its content kept clear of the status bar and the home
/// indicator.
#[component]
fn Drawer(side: Side, x: f64, width: f64, screen: Screen, children: Element) -> Element {
    let t = use_shell().theme;
    let ins = screen.insets;
    let (pad_l, pad_r) = match side {
        Side::Left => (ins.left, 0.0),
        Side::Right => (0.0, ins.right),
    };
    rsx! {
        div {
            style: "position: absolute; top: 0; left: {x}px; width: {width}px; height: 100%; \
                    display: flex; flex-direction: column; overflow-y: scroll; box-sizing: border-box; \
                    background: {t.sheet}; box-shadow: 0 0 48px rgba(0,0,0,0.55); \
                    padding: {ins.top + 8.0}px {pad_r}px {ins.bottom + 8.0}px {pad_l}px;",
            {children}
        }
    }
}

/// The docked sidebar's head: the workspace switcher (or the app's name)
/// and the button that folds the sidebar to a rail. Under a window that
/// draws its own title bar, the fold button shares the window buttons' row
/// (whose empty space moves the window) and the switcher goes below it.
#[component]
fn SidebarHead(title: Option<String>) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let folder = shell.clone();
    let fold = rsx! {
        button {
            class: "lp",
            "aria-label": "Fold the sidebar",
            style: "width: 36px; height: 36px; flex-shrink: 0; border: none; border-radius: 10px; background: transparent; \
                    display: flex; align-items: center; justify-content: center;",
            onclick: move |_| folder.toggle(Side::Left),
            PanelLeftClose { size: 18, color: t.ink_3 }
        }
    };
    let name = rsx! {
        if let Some(workspaces) = shell.workspaces.clone() {
            Switcher { workspaces, place: Place::Head }
        } else {
            span { style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; padding-left: 10px; \
                           font-size: 17px; font-weight: 750; letter-spacing: -0.01em; color: {t.ink};",
                if let Some(title) = title { "{title}" }
            }
        }
    };
    if let Some(chrome) = shell.window {
        return rsx! {
            div { style: "height: {chrome.height}px; display: flex; align-items: center; padding: 0 10px 0 0;",
                DragStrip { chrome: chrome.clone(), width: None }
                {fold}
            }
            div { style: "display: flex; align-items: center; padding: 0 10px 4px;", {name} }
        };
    }
    rsx! {
        div { style: "height: 52px; display: flex; align-items: center; gap: 4px; padding: 0 10px;",
            {name}
            {fold}
        }
    }
}

/// A desk's sidebar head: the title row (the window's buttons and the
/// sidebar's button over its start, then the app's name, the rest moving
/// the window), and the workspace switcher under it.
#[component]
fn DeskSidebarHead(title: Option<String>) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let row = shell.title_row();
    let start = shell.content_left();
    let chrome = shell.window.clone();
    rsx! {
        div { style: "height: {row}px; flex-shrink: 0; display: flex; align-items: center; padding-left: {start}px; padding-right: 10px;",
            if let Some(title) = title {
                span { style: "flex-shrink: 0; white-space: nowrap; font-size: 15px; font-weight: 700; \
                               letter-spacing: -0.01em; color: {t.ink};",
                    "{title}"
                }
            }
            if let Some(c) = chrome {
                DragStrip { chrome: c, width: None }
            }
        }
        if let Some(workspaces) = shell.workspaces {
            div { style: "display: flex; align-items: center; padding: 2px 10px 4px;",
                Switcher { workspaces, place: Place::Head }
            }
        }
    }
}

/// A desk's button for a side, fixed in the title row.
#[component]
fn TitleControl(side: Side) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let row = shell.title_row();
    let top = shell.screen.insets.top;
    let at = match side {
        Side::Left => format!("left: {}px;", shell.controls_left()),
        Side::Right => format!("right: {}px;", 12.0 + shell.screen.insets.right),
    };
    let label = match side {
        Side::Left => "Toggle the sidebar",
        Side::Right => "Toggle the details",
    };
    rsx! {
        div { style: "position: absolute; top: {top}px; {at} height: {row}px; display: flex; align-items: center; z-index: 30;",
            button {
                class: "lp",
                "aria-label": label,
                style: "width: {CONTROL}px; height: {CONTROL}px; border: none; border-radius: 7px; background: transparent; \
                        display: flex; align-items: center; justify-content: center;",
                onclick: move |_| shell.toggle(side),
                match side {
                    Side::Left => rsx! { PanelLeft { size: 17, color: t.ink_2 } },
                    Side::Right => rsx! { PanelRight { size: 17, color: t.ink_2 } },
                }
            }
        }
    }
}

/// Empty title-bar space: a press moves the window, a double click zooms
/// it. `width: None` fills what is left of its row.
#[component]
pub(super) fn DragStrip(chrome: WindowChrome, width: Option<f64>) -> Element {
    let size = width.map_or_else(
        || "flex: 1; min-width: 0;".to_owned(),
        |w| format!("width: {w}px; flex-shrink: 0;"),
    );
    let (drag, zoom) = (chrome.on_drag, chrome.on_zoom);
    rsx! {
        div {
            style: "{size} align-self: stretch; min-height: {chrome.height}px;",
            onmousedown: move |_| drag.call(()),
            ondoubleclick: move |_| zoom.call(()),
        }
    }
}

/// The button at the head of a rail that brings the left side out: the
/// docked sidebar unfolds, a phone's drawer opens.
#[component]
fn RailSideButton() -> Element {
    let shell = use_shell();
    let t = shell.theme;
    if !shell.has_side(Side::Left) {
        return rsx! {};
    }
    let unfold = frame(shell.screen, shell.style) == Frame::Sidebar;
    rsx! {
        button {
            class: "lp",
            "aria-label": "Show the sidebar",
            style: "height: 44px; border: none; border-radius: 12px; background: transparent; \
                    display: flex; align-items: center; justify-content: center;",
            onclick: move |_| if unfold { shell.unfold() } else { shell.open(Some(Side::Left)) },
            PanelLeft { size: 20, color: t.ink_3 }
        }
    }
}

/// The account as a row: avatar, name, detail.
#[component]
fn AccountRow(account: Account, bottom: f64) -> Element {
    let t = use_shell().theme;
    let press = account.on_press;
    rsx! {
        button {
            class: "lp",
            "aria-label": "Account",
            style: "flex-shrink: 0; display: flex; align-items: center; justify-content: flex-start; gap: 12px;                     padding: 12px 16px {12.0 + bottom}px; border: none; border-top: 1px solid {t.rule};                     background: transparent; font-family: {t.font}; text-align: left;",
            onclick: move |_| if let Some(p) = press { p.call(()) },
            Avatar { initials: account.initials.clone(), color: account.color, size: 34.0 }
            div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px;",
                span { style: "font-size: 15px; font-weight: 650; color: {t.ink}; white-space: nowrap; overflow: hidden;",
                    "{account.name}"
                }
                if let Some(d) = account.detail {
                    span { style: "font-size: 12px; color: {t.ink_3}; white-space: nowrap; overflow: hidden;", "{d}" }
                }
            }
        }
    }
}

/// The account as an avatar alone, at the foot of a rail.
#[component]
fn AccountChip(account: Account) -> Element {
    let press = account.on_press;
    rsx! {
        button {
            class: "lp",
            "aria-label": "Account",
            style: "height: 52px; border: none; border-radius: 12px; background: transparent;                     display: flex; align-items: center; justify-content: center;",
            onclick: move |_| if let Some(p) = press { p.call(()) },
            Avatar { initials: account.initials.clone(), color: account.color, size: 36.0 }
        }
    }
}

#[component]
fn Avatar(initials: String, color: &'static str, size: f64) -> Element {
    let t = use_shell().theme;
    let font = (size * 0.42).round();
    rsx! {
        span {
            style: "width: {size}px; height: {size}px; flex-shrink: 0; border-radius: {size / 2.0}px; background: {color};                     display: flex; align-items: center; justify-content: center; color: {t.ink};                     font-size: {font}px; font-weight: 700;",
            "{initials}"
        }
    }
}

/// The edge of a docked sidebar that a drag resizes.
#[component]
fn ResizeEdge(side: Side, on_start: EventHandler<PointerEvent>) -> Element {
    let edge = match side {
        Side::Left => "right: -4px;",
        Side::Right => "left: -4px;",
    };
    rsx! {
        div {
            style: "position: absolute; top: 0; {edge} width: 8px; height: 100%; cursor: col-resize; z-index: 5;",
            onpointerdown: move |e| on_start.call(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_factors() {
        assert_eq!(FormFactor::of(402.0, 874.0, true), FormFactor::Phone);
        assert_eq!(FormFactor::of(874.0, 402.0, true), FormFactor::Phone);
        assert_eq!(FormFactor::of(834.0, 1194.0, true), FormFactor::Tablet);
        assert_eq!(FormFactor::of(1440.0, 900.0, false), FormFactor::Desk);
    }

    #[test]
    fn frames() {
        let s = |w, h, touch| Screen {
            width: w,
            height: h,
            touch,
            insets: Insets::default(),
        };
        assert_eq!(
            frame(s(402.0, 874.0, true), ShellStyle::FloatingPill),
            Frame::Bottom
        );
        assert_eq!(
            frame(s(874.0, 402.0, true), ShellStyle::FloatingPill),
            Frame::Rail
        );
        assert_eq!(
            frame(s(402.0, 874.0, true), ShellStyle::RailOnly),
            Frame::Rail
        );
        assert_eq!(
            frame(s(834.0, 1194.0, true), ShellStyle::FloatingPill),
            Frame::Sidebar
        );
        assert_eq!(
            frame(s(1440.0, 900.0, false), ShellStyle::TabBar),
            Frame::Sidebar
        );
    }
}
