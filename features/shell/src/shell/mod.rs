//! The shell's Dioxus half (feature `ui`): [`AppShell`] and everything it
//! draws.

mod bar;
mod bars;
mod keyboard;
#[cfg(target_os = "macos")]
mod mac;
mod page;
mod status;
mod workspaces;

use std::cell::Cell;
use std::rc::Rc;

use crate::motion::{Spring, use_motion};
use dioxus::prelude::*;
use lucide_dioxus::{ChevronsUpDown, PanelLeft, PanelLeftClose, PanelRight};

pub use crate::screen::{FormFactor, Insets, Screen, ShellStyle};
pub use bar::{
    BAR_MAX, BarAction, CommandRunner, EditingFocus, use_bar_action, use_editing_actions,
};
use bar::{BarModel, PhoneFoot};
use bars::{Rail, RoundButton, SideTab};
pub use keyboard::{Keyboard, install_keyboard, keyboard, set_keyboard_height, set_typing};
#[cfg(target_os = "macos")]
pub use mac::fit_traffic_lights;
pub use page::{
    Breadcrumb, Crumb, Menu, MenuEntry, MenuItem, Page, PageBar, Tool, ToolGroup, Toolbar,
};
pub use status::{STATUS_BADGE, STATUS_BAR, StatusItem, StatusSide, StatusStyle, use_status};
use status::{StatusBadge, StatusBar};
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
    /// What a press on it does, when it has no [`menu`](Self::menu).
    pub on_press: Option<Callback<()>>,
    /// A picture, in place of the initials.
    pub image: Option<String>,
    /// A presence dot on the avatar, in this colour (online, away, busy).
    pub presence: Option<&'static str>,
    /// What a press opens, above the row: headed by who this is.
    pub menu: Option<Menu>,
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
            image: None,
            presence: None,
            menu: None,
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

    #[must_use]
    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(url.into());
        self
    }

    #[must_use]
    pub const fn presence(mut self, colour: &'static str) -> Self {
        self.presence = Some(colour);
        self
    }

    /// The account menu (profile, settings, sign out), opened above it.
    #[must_use]
    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
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
    /// A Mac title row's height, points.
    pub const MAC_ROW: f64 = 40.0;
    /// How far in a Mac window's first button sits, points.
    pub const MAC_LIGHTS_LEFT: f64 = 14.0;

    /// macOS, Obsidian's way: a 40pt title row, the traffic lights centred
    /// in it 14pt in (placed there by [`fit_traffic_lights`]), reaching
    /// 74pt in.
    pub fn mac(on_drag: impl FnMut(()) + 'static, on_zoom: impl FnMut(()) + 'static) -> Self {
        Self {
            height: Self::MAC_ROW,
            buttons: 74.0,
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
    /// A desk's title bar, across the whole window over the sidebars: the
    /// window's buttons, the sidebar's head, the page's bar or its tabs.
    pub title_bar: &'static str,
    /// The pills.
    pub pill: &'static str,
    /// A pill's edge, a sidebar's edge.
    pub rule: &'static str,
    /// The current tab's fill, a pressed row's.
    pub active: &'static str,
    pub ink: &'static str,
    pub ink_2: &'static str,
    /// The quietest text (a row's detail, a count): at least 4.5:1 on
    /// every fill it sits on, the pill and the accent's wash too.
    pub ink_3: &'static str,
    /// Badges.
    pub accent: &'static str,
    pub accent_ink: &'static str,
    /// A destructive menu row.
    pub danger: &'static str,
    /// Over the page while a drawer or a menu is open.
    pub scrim: &'static str,
    /// The accent, faint: the fill behind something current (the ribbon's
    /// section, a selected row).
    pub accent_soft: &'static str,
    /// A pill or a button over the page.
    pub shadow_lift: &'static str,
    /// What floats over everything: a menu, a popover, a floating pane.
    pub shadow_float: &'static str,
    /// The face, a CSS stack. Its first, Inter, is the app's to bundle
    /// (`architect_workspace::native::launch_with_fonts`, as the lab does);
    /// without it the system's sans is used.
    pub font: &'static str,
}

impl Default for ShellTheme {
    fn default() -> Self {
        Self {
            desk: "#09090b",
            sheet: "#0e0e11",
            title_bar: "#17171b",
            pill: "#1a1a1f",
            rule: "#2b2b31",
            active: "rgba(255,255,255,0.10)",
            ink: "#e4e4e7",
            ink_2: "#a1a1aa",
            ink_3: "#8f8f99",
            // Signal teal: a meter's green, FastTrackStudio's accent.
            accent: "#2dd4bf",
            accent_ink: "#04201c",
            danger: "#f87171",
            scrim: "rgba(0,0,0,0.55)",
            accent_soft: "rgba(45,212,191,0.14)",
            shadow_lift: "0 6px 20px rgba(0,0,0,0.35)",
            shadow_float: "0 16px 44px rgba(0,0,0,0.55)",
            // FastTrackStudio's face where the app bundles it (the lab
            // does) or the system has it; the system's sans elsewhere.
            font: "Inter, -apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif",
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
    /// What the page puts in the left sidebar instead of the app's `left`
    /// (a workspace's left dock): [`use_shell_slots`].
    page_left: Signal<Option<Element>>,
    /// What the page adds to a desk's ribbon, under the app's sections.
    rail_extra: Signal<Option<Element>>,
    /// What the page draws over the whole shell, its bars too:
    /// [`use_shell_cover`].
    cover: Signal<Option<Element>>,
    /// The page's status items: [`use_status`].
    page_status: Signal<Vec<StatusItem>>,
    pub status_style: StatusStyle,
    /// The app's buttons on a phone's action bar.
    bar_actions: Vec<BarAction>,
    /// What the pages and editors put on the bar: [`use_bar_action`],
    /// [`use_editing_actions`].
    bars: Signal<BarModel>,
}

/// Puts the page's own content in the shell's left side while the page is
/// mounted.
///
/// `left` goes in place of the app's sidebar content, `rail` in a desk's
/// ribbon under the app's sections. Give elements that read their
/// state themselves (they are set once, on mount); `None` leaves the
/// app's.
pub fn use_shell_slots(left: Option<Element>, rail: Option<Element>) {
    let shell = try_use_context::<Signal<Shell>>();
    use_hook(move || {
        if let Some(shell) = shell {
            let (mut l, mut r) = {
                let s = shell.peek();
                (s.page_left, s.rail_extra)
            };
            l.set(left);
            r.set(rail);
        }
    });
    use_drop(move || {
        if let Some(shell) = shell {
            let (mut l, mut r) = {
                let s = shell.peek();
                (s.page_left, s.rail_extra)
            };
            l.set(None);
            r.set(None);
        }
    });
}

/// Puts the page's own layer over the whole shell (the page's bar, the
/// action bar and the tabs too) while the page is mounted: a phone's tab
/// switcher, which takes the screen as Obsidian's does.
///
/// The layer is laid out against the shell's own box, which is the
/// window's: place it with `position: absolute` and keep clear of the
/// screen's insets ([`Shell::screen`]). Give an element that reads its
/// state itself and draws nothing while it is closed; it is set when
/// `cover` comes or goes (`None` takes it away), not on every render.
pub fn use_shell_cover(cover: Option<Element>) {
    let shell = try_use_context::<Signal<Shell>>();
    let mut had = use_hook(|| CopyValue::new(false));
    let has = cover.is_some();
    if let Some(shell) = shell
        && *had.peek() != has
    {
        had.set(has);
        let mut slot = shell.peek().cover;
        slot.set(cover);
    }
    use_drop(move || {
        if let Some(shell) = shell
            && let Ok(s) = shell.try_peek()
        {
            let mut slot = s.cover;
            if slot.try_peek().is_ok_and(|c| c.is_some()) {
                slot.set(None);
            }
        }
    });
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

    /// Whether to show keys beside what they do (`⌘O`, "↵ open"): not on
    /// a touch screen, where there may be no keyboard to press them on.
    #[must_use]
    pub const fn shows_keys(&self) -> bool {
        !self.screen.touch
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
        // A desk's page starts past the ribbon.
        let ribbon = if self.desk() { RIBBON } else { 0.0 };
        let left = if self.has_side(Side::Left) && self.docked(Side::Left) {
            0.0
        } else if self.has_side(Side::Left) {
            self.content_left() - ribbon
        } else if self.window.is_some() {
            self.controls_left() - ribbon
        } else {
            0.0
        };
        let left = left.max(0.0);
        let right = if self.right_button_over_page() {
            12.0 + CONTROL + 14.0
        } else {
            0.0
        };
        (left, right + self.screen.insets.right)
    }

    /// What the head of a desk's sidebar keeps clear in the title row, from
    /// the sidebar's left edge (past the ribbon): the window's buttons and
    /// the sidebar's.
    #[must_use]
    pub fn sidebar_title_left(&self) -> f64 {
        (self.content_left() - if self.desk() { RIBBON } else { 0.0 }).max(0.0)
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

/// The shell around this page, if there is one.
#[must_use]
pub fn try_use_shell() -> Option<Shell> {
    try_use_context::<Signal<Shell>>().map(|s| s())
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
/// The shell's stacking order, for the shell and what it hosts (a
/// workspace's floats and sheets, the command palette): one scale, so two
/// layers never meet at the same number.
pub mod layer {
    /// A resize edge over its side.
    pub const EDGE: u8 = 5;
    /// Floating panes (10 and up, the last drawn on top).
    pub const FLOAT: u8 = 10;
    /// Minimised floats' chips.
    pub const CHIPS: u8 = 20;
    /// The status badge, the "arranging" chip.
    pub const STATUS: u8 = 25;
    /// The title row's fixed buttons.
    pub const CONTROLS: u8 = 30;
    /// A sheet over the page and its scrim (a phone's pane switcher).
    pub const SHEET: u8 = 35;
    /// The layer that follows a drag.
    pub const DRAG: u8 = 40;
    /// Menus and their click-away layer.
    pub const MENU: u8 = 50;
    /// The workspace switcher's list.
    pub const POPOVER: u8 = 60;
    /// The command palette and quick open.
    pub const PALETTE: u8 = 90;
}

/// What the shell's inline styles cannot say: a pressable thing shows a
/// pointer, and what the keyboard is on shows the accent's ring, in every
/// app on the shell.
fn shell_css(t: &ShellTheme) -> String {
    // A tip: an icon-only control's name, drawn by the shell on hover
    // (or keyboard focus) after a beat (Blitz draws no `title`). Its host is
    // `position: relative` and places it.
    format!(
        "button, [role=tab], [role=button] {{ cursor: pointer; }} \
         :focus-visible {{ outline: 2px solid {accent}; outline-offset: 2px; }} \
         .tip {{ position: absolute; z-index: {z}; white-space: nowrap; padding: 4px 8px; border-radius: 6px; \
                 background: {pill}; border: 1px solid {rule}; color: {ink}; font-family: {font}; font-size: 12px; \
                 font-weight: 500; pointer-events: none; opacity: 0; transition: opacity 100ms ease-out; }} \
         .tip-host:hover > .tip, .tip-host:focus-visible > .tip {{ opacity: 1; transition: opacity 120ms ease-out 450ms; }}",
        accent = t.accent,
        z = layer::POPOVER,
        pill = t.pill,
        rule = t.rule,
        ink = t.ink,
        font = t.font,
    )
}

/// A desk's ribbon, at the window's left edge, points (Obsidian's).
pub const RIBBON: f64 = 44.0;
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
    /// The app's status items (a page adds its own: [`use_status`]).
    #[props(default)]
    status: Vec<StatusItem>,
    /// How a desk shows them: a bar along the foot, or a corner badge.
    #[props(default)]
    status_style: StatusStyle,
    /// The app's buttons on a phone's action bar, by command id (a page
    /// adds its own: [`use_bar_action`]); at most [`BAR_MAX`] show.
    #[props(default)]
    actions: Vec<BarAction>,
    children: Element,
) -> Element {
    let mut drawer = use_signal(|| None::<Side>);
    let fold = use_signal(|| None::<bool>);
    let right_away = use_signal(|| false);
    let left_w = use_signal(|| LEFT_WIDTH);
    let right_w = use_signal(|| RIGHT_WIDTH);
    let page_left = use_signal(|| None::<Element>);
    let page_status = use_signal(Vec::<StatusItem>::new);
    // The page's items first (its own, at a bar's left), then the app's.
    let all_status: Vec<StatusItem> = page_status().into_iter().chain(status).collect();
    let rail_extra = use_signal(|| None::<Element>);
    let cover = use_signal(|| None::<Element>);
    let bars = use_signal(BarModel::default);
    // A page that claims the sidebar draws it from the top (its tabs in the
    // title row). On a desk the switcher sits at the sidebar's foot either
    // way, so it stays put when such a page comes and goes.
    let page_side = page_left.read().is_some();
    let left = page_left().or(left);
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
        page_left,
        rail_extra,
        cover,
        page_status,
        status_style,
        bar_actions: actions,
        bars,
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
    let motion = use_motion(0.0, Spring::SHEET, 0.001);
    let mut shown = use_signal(|| None::<Side>);
    use_effect(move || {
        let target = drawer();
        if let Some(side) = target {
            if *shown.peek() != Some(side) {
                shown.set(Some(side));
            }
            motion.to(1.0);
        } else {
            motion.to_then(0.0, move || shown.set(None));
        }
    });
    // The docked sides open and fold on a spring: 0 folded away, 1 open.
    let docked_now = (
        frame == Frame::Sidebar && !shell.folded(),
        has_right && shell.docked(Side::Right) && !right_away(),
    );
    let left_open = use_motion(f64::from(u8::from(docked_now.0)), Spring::PANEL, 0.001);
    let right_open = use_motion(f64::from(u8::from(docked_now.1)), Spring::PANEL, 0.001);
    use_effect(use_reactive!(|docked_now| {
        let (left, right) = docked_now;
        left_open.to(f64::from(u8::from(left)));
        right_open.to(f64::from(u8::from(right)));
    }));

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
                    motion.to_then(to, move || {
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
            motion.to_then(if open.is_some() { 1.0 } else { 0.0 }, move || {
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
    let right_x = drawer_w.mul_add(-amount, screen.width);
    // On a phone the page gives way a little to the drawer coming in.
    let page_shift = match shown_side {
        Some(Side::Left) if phone => drawer_w * PARALLAX * amount,
        Some(Side::Right) if phone => -drawer_w * PARALLAX * amount,
        _ => 0.0,
    };

    // A side on its way open or folded is drawn whole and clipped to its
    // share of its width: its content does not reflow on the way.
    let (left_amount, right_amount) = (left_open.value(), right_open.value());
    let docked_left = frame == Frame::Sidebar && (docked_now.0 || left_amount > 0.001);
    let docked_right = has_right && (docked_now.1 || right_amount > 0.001);
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
    // A tablet folds its sidebar to its rail, a desk all the way.
    let rail_w = if form == FormFactor::Desk {
        0.0
    } else {
        76.0 + screen.insets.left
    };
    // Their padding (the safe area) is outside their widths.
    let left_clip = (lw + screen.insets.left - rail_w).mul_add(left_amount, rail_w);
    let right_clip = (rw + screen.insets.right) * right_amount;
    let resizable = form == FormFactor::Desk;
    // On a desk the title bar runs across the top of the sides too: their
    // edges start under it, and their heads draw their part of it.
    let desk = form == FormFactor::Desk;
    // A corner badge sits under the floating button: lift it clear.
    let badge_room = if desk && status_style == StatusStyle::Badge && !all_status.is_empty() {
        STATUS_BADGE
    } else {
        0.0
    };
    let bar_h = shell.title_row() + screen.insets.top;
    let (left_edge, left_pad_top) = if desk {
        (String::new(), 0.0)
    } else {
        (
            format!("border-right: 1px solid {};", t.rule),
            screen.insets.top,
        )
    };
    let (right_edge, right_pad_top) = if desk {
        (String::new(), 0.0)
    } else {
        (
            format!("border-left: 1px solid {};", t.rule),
            screen.insets.top,
        )
    };

    rsx! {
        div {
            style: "position: relative; width: 100%; height: 100%; overflow: hidden; display: flex; \
                    flex-direction: row; background: {t.desk}; color: {t.ink}; font-family: {t.font};",
            onpointerdown,
            onpointermove,
            onpointerup,
            onpointercancel,
            document::Style { {shell_css(&t)} }
            crate::motion::MotionFrames {}
            if frame == Frame::Sidebar && desk {
                DeskRibbon { tabs: tabs.clone(), current, on_tab: pick }
            }
            // Everything right of the ribbon, over a desk's status bar.
            div { style: "flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column;",
            div { style: "flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: {direction};",
            match frame {
                Frame::Rail => rsx! {
                    Rail { tabs: tabs.clone(), current, on_tab: pick, screen: side_screen }
                },
                Frame::Sidebar if !docked_left && desk => rsx! {},
                Frame::Sidebar if !docked_left => rsx! {
                    Rail { tabs: tabs.clone(), current, on_tab: pick, screen: side_screen }
                },
                Frame::Sidebar => rsx! {
                    div { style: "position: relative; width: {left_clip}px; flex-shrink: 0; overflow: hidden;",
                    div {
                        style: "position: absolute; top: 0; bottom: 0; left: 0; width: {lw}px; \
                                display: flex; flex-direction: column; \
                                background: {t.sheet}; {left_edge} \
                                padding-top: {left_pad_top}px; padding-left: {screen.insets.left}px;",
                        if desk && page_side {
                            div { style: "position: absolute; top: {bar_h}px; bottom: 0; right: 0; width: 1px; background: {t.rule};" }
                        } else if desk {
                            DeskSidebarHead { title: title }
                            // The sidebar's edge, under the title bar (which runs over it).
                            div { style: "position: absolute; top: {bar_h}px; bottom: 0; right: 0; width: 1px; background: {t.rule};" }
                        } else {
                            SidebarHead { title: title }
                        }
                        // On a desk the sections are the ribbon's; a tablet lists them here.
                        if !desk {
                            div { style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 10px 6px;",
                                for tab in tabs.iter().cloned() {
                                    SideTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab: pick }
                                }
                            }
                        }
                        if let Some(left) = left.clone() {
                            if !desk {
                                div { style: "height: 1px; background: {t.rule}; margin: 8px 18px;" }
                            }
                            div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column; overflow-y: scroll;",
                                {left}
                            }
                        } else {
                            span { style: "flex: 1;" }
                        }
                        if desk {
                            if let Some(workspaces) = workspaces.clone() {
                                div { style: "display: flex; align-items: center; padding: 6px 10px; border-top: 1px solid {t.rule};",
                                    Switcher { workspaces, place: Place::Foot }
                                }
                            }
                        }
                        if let Some(account) = account.clone() {
                            AccountRow { account, bottom: screen.insets.bottom }
                        }
                        if resizable {
                            ResizeEdge { side: Side::Left, on_start: move |e| start_left(Side::Left, e) }
                        }
                    }
                    }
                },
                Frame::Bottom => rsx! {},
            }
            // The page. It gives way by `left`, not `translateX`, which
            // Blitz moves by device pixels (a third as far on a 3x phone).
            div {
                style: "position: relative; left: {page_shift}px; flex: 1; min-width: 0; min-height: 0; \
                        display: flex; flex-direction: column;",
                {children}
                if float_separate {
                    if let Some(sep) = separate.clone() {
                        div {
                            style: "position: absolute; right: {20.0 + screen.insets.right}px; \
                                    bottom: {20.0 + screen.insets.bottom + badge_room}px;",
                            RoundButton { action: sep }
                        }
                    }
                }
            }
            if docked_right {
                if let Some(right) = right.clone() {
                    div { style: "position: relative; width: {right_clip}px; flex-shrink: 0; overflow: hidden;",
                    div {
                        style: "position: absolute; top: 0; bottom: 0; right: 0; width: {rw}px; \
                                display: flex; flex-direction: column; \
                                overflow-y: scroll; background: {t.sheet}; {right_edge} \
                                padding-top: {right_pad_top}px; padding-right: {screen.insets.right}px;",
                        // A desk's title row, under the side's fixed button:
                        // the title bar, running on over the side.
                        if form == FormFactor::Desk {
                            div { style: "height: {bar_h}px; flex-shrink: 0; display: flex; box-sizing: border-box; \
                                          padding-top: {screen.insets.top}px; background: {t.title_bar}; \
                                          border-bottom: 1px solid {t.rule};",
                                if let Some(w) = window {
                                    DragStrip { chrome: w, width: None }
                                }
                            }
                            div { style: "position: absolute; top: {bar_h}px; bottom: 0; left: 0; width: 1px; background: {t.rule};" }
                        }
                        {right}
                        if resizable {
                            ResizeEdge { side: Side::Right, on_start: move |e| start_right(Side::Right, e) }
                        }
                    }
                    }
                }
            }
            if in_bar {
                PhoneFoot { tabs: tabs, current, on_tab: pick, separate: separate, screen }
            }
            }
            if desk && status_style == StatusStyle::Bar && !all_status.is_empty() {
                StatusBar { items: all_status.clone() }
            }
            }
            if desk && status_style == StatusStyle::Badge {
                StatusBadge { items: all_status }
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
            // The page's layer over everything (`use_shell_cover`).
            {cover()}
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
/// the window). The workspace switcher is at the sidebar's foot.
#[component]
fn DeskSidebarHead(title: Option<String>) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let row = shell.title_row();
    let top = shell.screen.insets.top;
    let start = shell.sidebar_title_left();
    let chrome = shell.window;
    rsx! {
        div { style: "height: {row + top}px; flex-shrink: 0; box-sizing: border-box; display: flex; align-items: center; \
                      padding: {top}px 10px 0 {start}px; background: {t.title_bar}; border-bottom: 1px solid {t.rule};",
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

/// The account at the foot of a sidebar (shadcn's `NavUser`, Linear's):
/// an inset row — a rounded avatar with its presence, the name over the
/// email, ⇅ — that opens the account menu above it, headed by who this is.
#[component]
fn AccountRow(account: Account, bottom: f64) -> Element {
    let t = use_shell().theme;
    let mut open = use_signal(|| false);
    let press = account.on_press;
    let menu = account.menu.clone();
    let has_menu = menu.is_some();
    let fill = if open() { t.active } else { "transparent" };
    rsx! {
        div { style: "position: relative; flex-shrink: 0; padding: 6px 8px {6.0 + bottom}px; border-top: 1px solid {t.rule};",
            button {
                class: "lp",
                "aria-label": "Account",
                style: "width: 100%; height: 48px; box-sizing: border-box; display: flex; align-items: center; gap: 10px; \
                        padding: 0 8px; border: none; border-radius: 8px; background: {fill}; font-family: {t.font}; \
                        text-align: left;",
                onclick: move |_| {
                    if has_menu {
                        open.toggle();
                    } else if let Some(p) = press {
                        p.call(());
                    }
                },
                Avatar { account: account.clone(), size: 32.0 }
                div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px;",
                    span { style: "font-size: 13px; font-weight: 600; color: {t.ink}; white-space: nowrap; overflow: hidden;",
                        "{account.name}"
                    }
                    if let Some(d) = account.detail.clone() {
                        span { style: "font-size: 12px; color: {t.ink_3}; white-space: nowrap; overflow: hidden;", "{d}" }
                    }
                }
                if has_menu {
                    ChevronsUpDown { size: 15, color: t.ink_3 }
                }
            }
            if let (true, Some(menu)) = (open(), menu) {
                // Far past every edge: Blitz has no `position: fixed`.
                div {
                    style: "position: absolute; top: -4000px; left: -4000px; width: 8000px; height: 8000px; z-index: {layer::MENU};",
                    onclick: move |_| open.set(false),
                }
                div {
                    style: "position: absolute; left: 8px; right: 8px; bottom: {58.0 + bottom}px; z-index: {layer::MENU}; \
                            display: flex; flex-direction: column; padding: 6px 0; border-radius: 12px; \
                            background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                    div { style: "display: flex; align-items: center; gap: 10px; padding: 6px 12px 10px; \
                                  border-bottom: 1px solid {t.rule}; margin-bottom: 6px; font-family: {t.font};",
                        Avatar { account: account.clone(), size: 32.0 }
                        div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px;",
                            span { style: "font-size: 13px; font-weight: 600; color: {t.ink}; white-space: nowrap; overflow: hidden;",
                                "{account.name}"
                            }
                            if let Some(d) = account.detail {
                                span { style: "font-size: 12px; color: {t.ink_3}; white-space: nowrap; overflow: hidden;", "{d}" }
                            }
                        }
                    }
                    for (i, entry) in menu.entries.iter().cloned().enumerate() {
                        page::MenuRow { key: "{i}", entry, open }
                    }
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
            style: "height: 52px; border: none; border-radius: 12px; background: transparent; \
                    display: flex; align-items: center; justify-content: center;",
            onclick: move |_| if let Some(p) = press { p.call(()) },
            Avatar { account, size: 36.0 }
        }
    }
}

/// A rounded square (a person's picture or initials on their colour,
/// lit from the top left), its presence a dot at the corner ringed in the
/// sidebar's colour.
#[component]
fn Avatar(account: Account, size: f64) -> Element {
    let t = use_shell().theme;
    let font = (size * 0.4).round();
    let radius = (size * 0.25).round();
    let dot = (size * 0.3).round();
    let fill = format!(
        "linear-gradient(135deg, {c} 0%, {c}b3 100%)",
        c = account.color
    );
    rsx! {
        span { style: "position: relative; width: {size}px; height: {size}px; flex-shrink: 0; display: flex;",
            if let Some(src) = account.image.clone() {
                img { src: "{src}", style: "width: {size}px; height: {size}px; border-radius: {radius}px; object-fit: cover;" }
            } else {
                span {
                    style: "width: {size}px; height: {size}px; border-radius: {radius}px; background: {fill}; \
                            display: flex; align-items: center; justify-content: center; color: #ffffff; \
                            font-family: {t.font}; font-size: {font}px; font-weight: 650; letter-spacing: 0.02em;",
                    "{account.initials}"
                }
            }
            if let Some(colour) = account.presence {
                span { style: "position: absolute; right: -2px; bottom: -2px; width: {dot}px; height: {dot}px; \
                               box-sizing: border-box; border-radius: {dot / 2.0}px; background: {colour}; \
                               border: 2px solid {t.sheet};" }
            }
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
            style: "position: absolute; top: 0; {edge} width: 8px; height: 100%; cursor: col-resize; z-index: {layer::EDGE};",
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

/// A desk's ribbon: the window's left edge under the title bar, the app's
/// sections as icons, then what the page adds (a workspace's views and
/// pinned commands). Its head is its part of the title bar.
#[component]
fn DeskRibbon(
    tabs: Vec<Tab>,
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let bar_h = shell.title_row() + shell.screen.insets.top;
    let extra = (shell.rail_extra)();
    let chrome = shell.window;
    rsx! {
        div {
            style: "width: {RIBBON}px; flex-shrink: 0; display: flex; flex-direction: column; background: {t.desk};",
            div {
                style: "height: {bar_h}px; flex-shrink: 0; box-sizing: border-box; display: flex; \
                        background: {t.title_bar}; border-bottom: 1px solid {t.rule};",
                if let Some(c) = chrome {
                    DragStrip { chrome: c, width: None }
                }
            }
            div {
                style: "flex: 1; min-height: 0; display: flex; flex-direction: column; align-items: center; \
                        gap: 4px; padding: 8px 0; border-right: 1px solid {t.rule};",
                for tab in tabs {
                    RibbonTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab }
                }
                if let Some(extra) = extra {
                    div { style: "width: 60%; height: 1px; background: {t.rule}; margin: 4px 0;" }
                    {extra}
                }
            }
        }
    }
}

/// An app section in a desk's ribbon: its icon, its name on hover.
#[component]
fn RibbonTab(tab: Tab, on: bool, on_tab: EventHandler<&'static str>) -> Element {
    let t = use_shell().theme;
    // The current section: the accent, faint behind it and full in it.
    let (fill, colour) = if on {
        (t.accent_soft, t.accent)
    } else {
        ("transparent", t.ink_3)
    };
    let id = tab.id;
    rsx! {
        button {
            class: "lp tip-host",
            "aria-label": tab.label,
            style: "position: relative; width: 32px; height: 32px; flex-shrink: 0; border: none; border-radius: 7px; \
                    background: {fill}; display: flex; align-items: center; justify-content: center;",
            onclick: move |_| on_tab.call(id),
            {(tab.icon)(Icon { size: 18, color: colour })}
            span { class: "tip", style: "left: 40px; top: 5px;", "{tab.label}" }
            if tab.badge.is_some() {
                span { style: "position: absolute; top: 5px; right: 5px; width: 6px; height: 6px; border-radius: 3px; \
                               background: {t.accent};" }
            }
        }
    }
}
