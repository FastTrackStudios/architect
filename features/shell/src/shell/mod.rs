//! The shell's Dioxus half (feature `ui`): [`AppShell`] and everything it
//! draws.

mod bar;
mod bars;
mod busy;
mod chrome;
mod favourites;
mod input_debug;
mod keyboard;
#[cfg(target_os = "macos")]
mod mac;
mod page;
#[cfg(test)]
mod snapshot_tests;
mod status;
mod typing;
mod workspaces;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::motion::{Spring, use_motion};
use dioxus::prelude::*;
use lucide_dioxus::{ChevronsUpDown, Minus, PanelLeft, PanelLeftClose, PanelRight, Square, X};

pub use crate::screen::{FormFactor, Insets, Screen, ShellStyle};
pub use bar::{
    BAR_MAX, BarAction, CommandRunner, EditingFocus, use_bar_action, use_editing_actions,
    use_typing_scope,
};
use bar::{BarModel, PhoneFoot};
use bars::{Rail, RoundButton, SideTab};
pub use busy::BusyBadge;
pub use chrome::{Chrome, Filler, Slot, intern, named_icon, use_chrome};
pub use favourites::{
    Declared, Favourites, PlaceOpened, Presses, SLOTS, Shows, declare, declared_tab_id, place_kept,
    resolve_favourites, set_note, use_presses,
};
pub use input_debug::{foot_bottom, input_debug, set_input_debug};
pub use keyboard::{
    Foot, Keyboard, KeyboardTaps, TAP_SLOP, TapStroke, hide_keyboard, install_keyboard, keyboard,
    set_keyboard_height, set_soft_keyboard, set_typing, show_keyboard, soft_keyboard, tap_raises,
    use_keyboard_taps,
};
#[cfg(target_os = "macos")]
pub use mac::fit_traffic_lights;
pub use page::{
    Breadcrumb, Crumb, Menu, MenuEntry, MenuItem, Page, PageBar, TabFlares, Tool, ToolGroup,
    Toolbar,
};
pub use status::{STATUS_BADGE, STATUS_BAR, StatusItem, StatusSide, StatusStyle, use_status};
use status::{StatusBadge, StatusBar};
pub use typing::{
    Typing, TypingButton, TypingContent, TypingItem, TypingMenu, TypingTools, TypingWidget,
};
use workspaces::{Place, Switcher};
pub use workspaces::{Workspace, WorkspaceAction, Workspaces};

#[doc(hidden)]
pub use dioxus::prelude::rsx as __rsx;

/// How an icon is drawn where the shell puts it. Blitz takes an SVG's
/// colour as an attribute, not from CSS, so the shell says it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Icon {
    pub size: usize,
    /// A CSS colour, usually a [`ShellTheme`] field (cheap to clone).
    pub color: Arc<str>,
}

/// Draws an icon at the size and colour the shell asks for. Write one with
/// [`icon!`](crate::icon): `icon!(House)`.
pub type IconFn = fn(Icon) -> Element;

/// An [`IconFn`] for a lucide (or any `size`/`color`) icon component:
/// `icon!(House)`.
#[macro_export]
macro_rules! icon {
    ($name:ident) => {
        (|i: $crate::Icon| $crate::__rsx! { $name { size: i.size, color: i.color.to_string() } })
            as $crate::IconFn
    };
}

/// One of the app's sections. The first four are the bottom bar.
#[derive(Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: IconFn,
    /// A Lucide icon by its name (`list-music`), drawn in place of
    /// `icon`: a tab named in data (a manifest's favourites).
    pub glyph: Option<&'static str>,
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
            glyph: None,
            badge: None,
        }
    }

    /// A tab whose icon is Lucide's `glyph` (`list-music`), by name: one
    /// named in data. A name Lucide has not draws [`named_icon`]'s puzzle.
    #[must_use]
    pub fn named(id: &'static str, label: &'static str, glyph: &'static str) -> Self {
        Self {
            glyph: Some(glyph),
            ..Self::new(id, label, named_icon(glyph))
        }
    }

    /// Its icon, drawn at `icon`'s size and colour.
    #[must_use]
    pub fn draw(&self, icon: Icon) -> Element {
        draw_icon(self.icon, self.glyph, icon)
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
    /// A Lucide icon by its name, drawn in place of `icon` (as
    /// [`Tab::glyph`]).
    pub glyph: Option<&'static str>,
    pub on_press: Callback<()>,
    /// Say the label beside the icon ("Process 5"), not just show it.
    pub show_label: bool,
    /// What pressing and sliding it up does instead (Task's capture
    /// button: a timer), on the round separate button.
    pub swipe_up: Option<SwipeUp>,
    /// A count on it ("3" captures to process), on the round button.
    pub badge: Option<String>,
}

/// An [`Action`]'s press-and-slide-up: the round button lifts with the
/// finger, takes this icon and the accent once it has gone far enough
/// ([`SWIPE_UP`]), and lets go into `on_swipe` instead of a press.
#[derive(Clone, PartialEq)]
pub struct SwipeUp {
    pub label: String,
    pub icon: IconFn,
    pub on_swipe: Callback<()>,
}

/// How far the round button travels up to mean its swipe (Task's 36pt).
pub const SWIPE_UP: f64 = 36.0;

impl Action {
    pub fn new(label: impl Into<String>, icon: IconFn, on_press: impl FnMut(()) + 'static) -> Self {
        Self {
            label: label.into(),
            icon,
            glyph: None,
            on_press: Callback::new(on_press),
            show_label: false,
            swipe_up: None,
            badge: None,
        }
    }

    /// Press and slide up to do `on_swipe` (the round separate button).
    #[must_use]
    pub fn on_swipe_up(
        mut self,
        label: impl Into<String>,
        icon: IconFn,
        on_swipe: impl FnMut(()) + 'static,
    ) -> Self {
        self.swipe_up = Some(SwipeUp {
            label: label.into(),
            icon,
            on_swipe: Callback::new(on_swipe),
        });
        self
    }

    /// A count on the round button: none at 0, `99+` past 99.
    #[must_use]
    pub fn with_badge(mut self, count: usize) -> Self {
        self.badge = (count > 0).then(|| {
            if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            }
        });
        self
    }

    /// Say the label beside the icon.
    #[must_use]
    pub const fn with_label(mut self) -> Self {
        self.show_label = true;
        self
    }

    /// Its icon Lucide's `glyph` (`play`), by name (as [`Tab::named`]).
    #[must_use]
    pub fn glyph(mut self, glyph: &'static str) -> Self {
        self.icon = named_icon(glyph);
        self.glyph = Some(glyph);
        self
    }

    /// Its icon, drawn at `icon`'s size and colour.
    #[must_use]
    pub fn draw(&self, icon: Icon) -> Element {
        draw_icon(self.icon, self.glyph, icon)
    }
}

/// `glyph` by name from Lucide's table when it has it, else `fallback`.
fn draw_icon(fallback: IconFn, glyph: Option<&'static str>, icon: Icon) -> Element {
    match glyph.filter(|g| crate::lucide::glyphs(g).is_some()) {
        Some(name) => rsx! {
            crate::lucide::LucideIcon {
                name: name.to_owned(),
                size: u32::try_from(icon.size).unwrap_or(16),
                color: icon.color.to_string(),
            }
        },
        None => fallback(icon),
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
    /// The window's buttons, when the shell draws them (off a Mac, where
    /// the app has no system title bar): traffic lights at the row's left.
    pub controls: Option<WindowControls>,
}

/// A window's close, minimize and maximize buttons, drawn by the shell as
/// a Mac's traffic lights at the title row's left (Loft's look).
#[derive(Clone, PartialEq)]
pub struct WindowControls {
    pub on_minimize: Callback<()>,
    pub on_maximize: Callback<()>,
    pub on_close: Callback<()>,
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
            controls: None,
        }
    }

    /// A window with no system title bar off a Mac: a Mac's 40pt row, the
    /// shell drawing its close, minimize and maximize as traffic lights
    /// where a Mac's sit (14pt in, reaching 66pt).
    pub fn framed(
        on_drag: impl FnMut(()) + 'static,
        on_zoom: impl FnMut(()) + 'static,
        on_minimize: impl FnMut(()) + 'static,
        on_close: impl FnMut(()) + 'static,
    ) -> Self {
        let on_zoom = Callback::new(on_zoom);
        Self {
            height: Self::MAC_ROW,
            buttons: 66.0,
            on_drag: Callback::new(on_drag),
            on_zoom,
            controls: Some(WindowControls {
                on_minimize: Callback::new(on_minimize),
                on_maximize: on_zoom,
                on_close: Callback::new(on_close),
            }),
        }
    }
}

/// The shell's colours. The default is `FastTrackStudio`'s dark.
///
/// Owned, so a theme can change at run time: an app with a theme store
/// (`architect_theme::use_theme_store`) gets it from there
/// ([`ShellTheme::from_resolved`], live), and [`AppShell`] follows the
/// store when it is given no `theme`. Clones are cheap (shared strings).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShellTheme {
    /// Under everything.
    pub desk: Arc<str>,
    /// The sidebars, the drawers, the menus.
    pub sheet: Arc<str>,
    /// A desk's title bar, across the whole window over the sidebars: the
    /// window's buttons, the sidebar's head, the page's bar or its tabs.
    pub title_bar: Arc<str>,
    /// The pills.
    pub pill: Arc<str>,
    /// A pill's edge, a sidebar's edge.
    pub rule: Arc<str>,
    /// The current tab's fill, a pressed row's.
    pub active: Arc<str>,
    pub ink: Arc<str>,
    pub ink_2: Arc<str>,
    /// The quietest text (a row's detail, a count): at least 4.5:1 on
    /// every fill it sits on, the pill and the accent's wash too.
    pub ink_3: Arc<str>,
    /// Badges.
    pub accent: Arc<str>,
    pub accent_ink: Arc<str>,
    /// A destructive menu row.
    pub danger: Arc<str>,
    /// Over the page while a drawer or a menu is open.
    pub scrim: Arc<str>,
    /// The accent, faint: the fill behind something current (the ribbon's
    /// section, a selected row).
    pub accent_soft: Arc<str>,
    /// A pill or a button over the page.
    pub shadow_lift: Arc<str>,
    /// What floats over everything: a menu, a popover, a floating pane.
    pub shadow_float: Arc<str>,
    /// The face, a CSS stack. Its first, Inter, is the app's to bundle
    /// (`architect_workspace::native::launch_with_fonts`, as the lab does);
    /// without it the system's sans is used.
    pub font: Arc<str>,
}

impl Default for ShellTheme {
    fn default() -> Self {
        Self {
            desk: "#09090b".into(),
            sheet: "#0e0e11".into(),
            title_bar: "#17171b".into(),
            pill: "#1a1a1f".into(),
            rule: "#2b2b31".into(),
            active: "rgba(255,255,255,0.10)".into(),
            ink: "#e4e4e7".into(),
            ink_2: "#a1a1aa".into(),
            ink_3: "#8f8f99".into(),
            // Signal teal: a meter's green, FastTrackStudio's accent.
            accent: "#2dd4bf".into(),
            accent_ink: "#04201c".into(),
            danger: "#f87171".into(),
            scrim: "rgba(0,0,0,0.55)".into(),
            accent_soft: "rgba(45,212,191,0.14)".into(),
            shadow_lift: "0 6px 20px rgba(0,0,0,0.35)".into(),
            shadow_float: "0 16px 44px rgba(0,0,0,0.55)".into(),
            // FastTrackStudio's face where the app bundles it (the lab
            // does) or the system has it; the system's sans elsewhere.
            font: "Inter, -apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif".into(),
        }
    }
}

/// The theme's tokens by the shell's names (`brand` for the accent: in a
/// resolved theme `accent` is architect-ui's hover wash).
impl architect_theme::FromResolved for ShellTheme {
    fn from_resolved(r: &architect_theme::Resolved) -> Self {
        let t = |name: &str| -> Arc<str> { r.value(name).into() };
        Self {
            desk: t("desk"),
            sheet: t("sheet"),
            title_bar: t("title-bar"),
            pill: t("pill"),
            rule: t("rule"),
            active: t("active"),
            ink: t("ink"),
            ink_2: t("ink-2"),
            ink_3: t("ink-3"),
            accent: t("brand"),
            accent_ink: t("brand-ink"),
            danger: t("danger"),
            scrim: t("scrim"),
            accent_soft: t("brand-soft"),
            shadow_lift: t("shadow-lift"),
            shadow_float: t("shadow-float"),
            font: t("font"),
        }
    }
}

impl ShellTheme {
    /// The shell's colours from a resolved theme.
    #[must_use]
    pub fn from_resolved(r: &architect_theme::Resolved) -> Self {
        <Self as architect_theme::FromResolved>::from_resolved(r)
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
    /// The docked left sidebar's width, and how open it is now (0 folded
    /// to 1 open, mid-fold too): where the page's left edge is.
    left_w: Signal<f64>,
    left_open: Option<crate::motion::Motion>,
    has_left: bool,
    has_right: bool,
    separate: Option<Action>,
    account: Option<Account>,
    workspaces: Option<Workspaces>,
    /// The app's buttons above the switcher at the sidebar's foot.
    foot: Vec<Action>,
    window: Option<WindowChrome>,
    /// The app's brand, leading a phone's root pages.
    brand: Option<Element>,
    /// Whether the separate button floats at the page's bottom right (a
    /// tablet) rather than sitting in a bar or a rail.
    floats_separate: bool,
    /// What the page puts in the left sidebar instead of the app's `left`
    /// (a workspace's left dock): [`use_shell_slots`].
    page_left: Signal<Option<Element>>,
    /// Whether there is anything to show in the left sidebar: the page's or
    /// the app's `left` (a desk folds a sidebar with nothing in it).
    has_content: bool,
    /// The current tab: the page kind a desk's sidebar toggle is kept for.
    current: Option<&'static str>,
    /// A desk's sidebar layout as the person last chose it, per page kind
    /// (the current tab's id; `""` under none).
    desk_layouts: Signal<BTreeMap<&'static str, SidebarLayout>>,
    /// The layout the page prefers when nobody has chosen one:
    /// [`use_sidebar_layout`].
    page_sidebar: Claim<SidebarLayout>,
    /// The page draws a desk's title row itself, its top edge in it:
    /// [`use_title_row`].
    page_title: Claim<()>,
    /// The page's tab on a desk, drawn in the title row beside the sidebar
    /// (its desk bar sets it).
    desk_tab: Signal<Option<String>>,
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
    /// [`use_editing_actions`], [`use_typing_scope`].
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

/// A desk's left side: the rail alone, or the rail and a sidebar of one or
/// two columns.
///
/// One and two are Notebook Navigator's single and dual pane: notes, or
/// folders beside notes. Every page may be shown in any of the three
/// (the sidebar's button, ⌘K's "Sidebar: …"); a page says which it
/// prefers ([`use_sidebar_layout`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarLayout {
    /// The rail (the ribbon) alone: the sidebar folded away.
    Rail,
    /// The rail and a sidebar of one column.
    #[default]
    One,
    /// The rail and a sidebar of two columns, as wide as both need.
    Two,
}

impl SidebarLayout {
    /// All three, rail first.
    pub const ALL: [Self; 3] = [Self::Rail, Self::One, Self::Two];

    /// How many columns the sidebar has: 0 for the rail alone.
    #[must_use]
    pub const fn columns(self) -> u8 {
        match self {
            Self::Rail => 0,
            Self::One => 1,
            Self::Two => 2,
        }
    }

    /// Its name, as a menu says it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rail => "Rail",
            Self::One => "One column",
            Self::Two => "Two columns",
        }
    }
}

/// A page's hold on a part of the shell: the hook's own number and what it
/// asks, so a page going away never takes back the next page's.
type Claim<T> = Signal<Option<(u64, T)>>;

/// Holds `value` in the shell's `pick` claim while the page is mounted
/// (`None` lets go), set only when it changes.
fn use_claim<T: Clone + PartialEq + 'static>(pick: fn(&Shell) -> Claim<T>, value: Option<T>) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let shell = try_use_context::<Signal<Shell>>();
    let me = use_hook(|| NEXT.fetch_add(1, Ordering::Relaxed));
    if let Some(shell) = shell {
        let mut slot = pick(&shell.peek());
        let now = slot.peek().clone();
        match value {
            Some(v) if now.as_ref() != Some(&(me, v.clone())) => slot.set(Some((me, v))),
            None if now.as_ref().is_some_and(|(id, _)| *id == me) => slot.set(None),
            _ => {}
        }
    }
    use_drop(move || {
        if let Some(shell) = shell
            && let Ok(s) = shell.try_peek()
        {
            let mut slot = pick(&s);
            if slot
                .try_peek()
                .is_ok_and(|c| c.as_ref().is_some_and(|(id, _)| *id == me))
            {
                slot.set(None);
            }
        }
    });
}

/// The layout the page prefers for a desk's left side
/// ([`SidebarLayout`]): the rail alone (a page with no use for a sidebar),
/// or a sidebar of one or two columns. The others stay a press away.
///
/// The person's own choice wins, and is kept per page kind (the current
/// tab). A page that says nothing gets one column when it has something
/// for the sidebar and the window is wide enough (1000pt), else the rail.
/// Phones and tablets keep their own.
pub fn use_sidebar_layout(preferred: SidebarLayout) {
    use_claim(|s| s.page_sidebar, Some(preferred));
}

/// The page's top edge is a desk's title row (a workspace's top tab bars,
/// Obsidian's).
///
/// While `claim` holds, the page (and a left sidebar it fills,
/// [`use_shell_slots`]) starts at the window's top, over the shell's title
/// row, which stays under it beside them (the ribbon's corner, the app's
/// sidebar). One row, not two. The page keeps its own content clear of the
/// window's buttons and the sidebar's ([`Shell::title_clearance`]) and moves
/// the window from its empty part ([`WindowChrome::on_drag`]).
pub fn use_title_row(claim: bool) {
    use_claim(|s| s.page_title, claim.then_some(()));
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

    /// Whether the docked left sidebar is folded to a rail (on a desk, to
    /// the ribbon): [`Self::sidebar_layout`] is [`SidebarLayout::Rail`].
    #[must_use]
    pub fn folded(&self) -> bool {
        self.sidebar_layout() == SidebarLayout::Rail
    }

    /// A desk's left side now: as the person last chose it on this page
    /// kind, else as the page prefers ([`use_sidebar_layout`]), else one
    /// column when it has something for it and the window is wide enough.
    /// Off a desk, the rail or one column.
    #[must_use]
    pub fn sidebar_layout(&self) -> SidebarLayout {
        if self.desk() {
            if let Some(chosen) = self.desk_layouts.read().get(self.page_kind()) {
                return *chosen;
            }
            return self.preferred_sidebar();
        }
        if (self.fold)().unwrap_or(self.screen.width < FOLD_BELOW) {
            SidebarLayout::Rail
        } else {
            SidebarLayout::One
        }
    }

    /// The layout the page prefers ([`use_sidebar_layout`]), else one
    /// column when it has something for the sidebar and the window is wide
    /// enough, else the rail.
    #[must_use]
    pub fn preferred_sidebar(&self) -> SidebarLayout {
        if let Some((_, preferred)) = *self.page_sidebar.read() {
            return preferred;
        }
        if !self.has_content || self.screen.width < FOLD_BELOW {
            SidebarLayout::Rail
        } else {
            SidebarLayout::One
        }
    }

    /// Show a desk's left side as `layout`, kept for this page kind (off a
    /// desk, one or two columns open the sidebar).
    pub fn set_sidebar_layout(&self, layout: SidebarLayout) {
        if self.desk() {
            let mut layouts = self.desk_layouts;
            layouts.write().insert(self.page_kind(), layout);
        } else {
            let mut fold = self.fold;
            fold.set(Some(layout == SidebarLayout::Rail));
        }
    }

    /// The page kind a desk's sidebar toggle is kept for: the current tab.
    fn page_kind(&self) -> &'static str {
        self.current.unwrap_or("")
    }

    /// Fold the docked left sidebar (`true`) or open it: open, as the page
    /// prefers when that is a sidebar, else one column.
    fn set_folded(&self, folded: bool) {
        let layout = match (folded, self.preferred_sidebar()) {
            (true, _) => SidebarLayout::Rail,
            (false, SidebarLayout::Rail) => SidebarLayout::One,
            (false, preferred) => preferred,
        };
        self.set_sidebar_layout(layout);
    }

    /// Whether the page draws a desk's title row itself ([`use_title_row`]).
    #[must_use]
    pub fn page_draws_title_row(&self) -> bool {
        self.desk() && (self.page_title)().is_some()
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
            (Side::Left, Frame::Sidebar) => self.set_folded(!self.folded()),
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

    /// The title row's height: the window's buttons' row, or Obsidian's 40.
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
        let left = if self.desk() && self.has_side(Side::Left) {
            // Past the sidebar as it shows now, so the page's title row
            // gives way smoothly while the sidebar folds or opens.
            self.content_left() - ribbon - self.left_shown()
        } else if self.has_side(Side::Left) && self.docked(Side::Left) {
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

    /// How wide the docked left sidebar shows now on a desk, mid-fold too.
    #[must_use]
    pub fn left_shown(&self) -> f64 {
        let open = self.left_open.map_or_else(
            || if self.docked(Side::Left) { 1.0 } else { 0.0 },
            crate::motion::Motion::value,
        );
        ((self.left_w)() + self.screen.insets.left) * open
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
        self.set_folded(false);
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
/// A desk's title row without window buttons of its own (full screen, the
/// web app): Obsidian's header height (`--header-height: 40px`), the same
/// as the row with them ([`WindowChrome::MAC_ROW`]), so the row never jumps.
const TITLE_ROW: f64 = WindowChrome::MAC_ROW;
/// A desk's title-row controls (the sides' buttons).
const CONTROL: f64 = 28.0;
/// A traffic light the shell draws: its size and the gap between two.
const LIGHT: f64 = 12.0;
const LIGHT_GAP: f64 = 8.0;
/// How far the page follows an opening drawer, as a share of its width.
const PARALLAX: f64 = 0.22;
/// The docked sidebars' widths: the start, and the range a drag keeps to.
const LEFT_WIDTH: f64 = 264.0;
/// A two-column sidebar's width to start with: Notebook Navigator's
/// folders (200) and a list beside them.
const TWO_WIDTH: f64 = 480.0;
/// The shell's stacking order, for the shell and what it hosts (a
/// workspace's floats and sheets, the command palette): one scale, so two
/// layers never meet at the same number.
pub mod layer {
    /// A resize edge over its side.
    pub const EDGE: u8 = 5;
    /// A phone's foot (the action bar, the tabs, the editing bar).
    ///
    /// Over the page, whose positioned content Blitz hit-tests even where the
    /// page's `overflow: hidden` keeps it from being drawn (a calendar's
    /// events below the fold took the tabs' presses).
    pub const FOOT: u8 = 8;
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
///
/// Blitz never matches `:focus-visible` and draws its own browser-blue
/// outline on a focused `input` or `textarea` (its user-agent sheet), so
/// a text field's ring is set on `:focus` here (a browser shows a text
/// field's on any focus too), and a framed field whose bare input draws
/// none (a search box: `outline: none` on the input) takes the class
/// `focus-ring`, which rings the frame while what is inside has focus.
fn shell_css(t: &ShellTheme) -> String {
    // A tip: an icon-only control's name, drawn by the shell on hover
    // (or keyboard focus) after a beat (Blitz draws no `title`). Its host is
    // `position: relative` and places it.
    format!(
        "button, [role=tab], [role=button] {{ cursor: pointer; }} \
         :focus-visible {{ outline: 2px solid {accent}; outline-offset: 2px; }} \
         input:focus, textarea:focus, select:focus, .focus-ring:focus-within {{ outline: 2px solid {accent}; outline-offset: 2px; }} \
         .tip {{ position: absolute; z-index: {z}; white-space: nowrap; padding: 4px 8px; border-radius: 6px; \
                 background: {pill}; border: 1px solid {rule}; color: {ink}; font-family: {font}; font-size: 12px; \
                 font-weight: 500; pointer-events: none; opacity: 0; transition: opacity 100ms ease-out; }} \
         .tip-host:hover > .tip, .tip-host:focus-visible > .tip {{ opacity: 1; transition: opacity 120ms ease-out 450ms; }} \
         .lights svg {{ opacity: 0; }} .lights:hover svg {{ opacity: 1; }}",
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
/// What a sidebar dragged wider leaves the page between them: no fixed
/// maximum (Notebook Navigator's sidebar has none), only the main area's
/// room (#353).
const PAGE_ROOM: f64 = 240.0;

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
    /// Its colours; left out, the app's theme store's
    /// (`architect_theme::use_theme_store`), live, or the default.
    #[props(default, into)]
    theme: Option<ShellTheme>,
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
    /// The app's buttons at the sidebar's foot, above the workspace
    /// switcher and the account, top to bottom (Theme, Settings): rows in a
    /// docked sidebar, icons in a folded one's ribbon and in a rail.
    #[props(default)]
    foot: Vec<Action>,
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
    /// The Now Playing bar (`architect_playhead::ui::NowPlayingBar`):
    /// above a phone's tab bar, at a desk's or a tablet's page foot over
    /// the status bar. It draws nothing while nothing is queued.
    now_playing: Option<Element>,
    children: Element,
) -> Element {
    let stored = architect_theme::use_themed::<ShellTheme>();
    let theme = theme.unwrap_or_else(|| stored());
    // What others add and how the slots are filled (`use_chrome`); with
    // none above, the shell's own, as it always was.
    let chrome = try_use_context::<Signal<Chrome>>()
        .map(|c| c())
        .unwrap_or_default();
    let tabs: Vec<Tab> = if chrome.tabs.is_empty() {
        tabs
    } else {
        tabs.into_iter()
            .chain(chrome.tabs.iter().cloned())
            .collect()
    };
    let status: Vec<StatusItem> = status
        .into_iter()
        .chain(chrome.status.iter().cloned())
        .collect();
    let actions: Vec<BarAction> = actions
        .into_iter()
        .chain(chrome.bar.iter().cloned())
        .collect();
    let mut drawer = use_signal(|| None::<Side>);
    let fold = use_signal(|| None::<bool>);
    let right_away = use_signal(|| false);
    let left_w = use_signal(|| LEFT_WIDTH);
    let right_w = use_signal(|| RIGHT_WIDTH);
    let page_left = use_signal(|| None::<Element>);
    let desk_layouts = use_signal(BTreeMap::<&'static str, SidebarLayout>::new);
    let page_sidebar = use_signal(|| None::<(u64, SidebarLayout)>);
    let page_title = use_signal(|| None::<(u64, ())>);
    let desk_tab = use_signal(|| None::<String>);
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
    // A slot's filler replaces its content; a hidden one has none.
    let left = slot_content(&chrome, Slot::Left, left);
    let right = slot_content(&chrome, Slot::Right, right);
    let has_content = left.is_some();
    // A sidebar's button with the switcher or the account alone: a tablet's
    // sidebar holds them; on a desk, where they stay in the ribbon, every
    // page keeps its three layouts a press away (`SidebarLayout`).
    let has_left = has_content
        || (screen.form() == FormFactor::Desk && (account.is_some() || workspaces.is_some()));
    let has_right = right.is_some();
    let shell = Shell {
        screen,
        style,
        theme: theme.clone(),
        drawer,
        fold,
        right_away,
        left_w,
        left_open: None,
        has_left,
        has_right,
        separate: separate.clone(),
        account: account.clone(),
        workspaces: workspaces.clone(),
        foot: foot.clone(),
        window: window.clone(),
        brand,
        floats_separate: frame(screen, style) == Frame::Sidebar,
        page_left,
        has_content,
        current,
        desk_layouts,
        page_sidebar,
        page_title,
        desk_tab,
        rail_extra,
        cover,
        page_status,
        status_style,
        bar_actions: actions,
        bars,
    };
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
    // The docked sides: 0 folded away, 1 open. They open and fold on a
    // spring when the person folds or opens one (`Shell::toggle`,
    // `Shell::set_sidebar_layout`), and are simply there otherwise: the
    // sidebar's content arriving after load, the page's preferred layout,
    // the window's size. Sprung then, a sidebar would slide open just as
    // the app shows, its rows drawn but clipped, and a click on one would
    // land on the page beside it.
    let docked_now = (
        frame == Frame::Sidebar && !shell.folded(),
        has_right && shell.docked(Side::Right) && !right_away(),
    );
    let chosen_now = (fold(), right_away(), desk_layouts.read().clone());
    let left_open = use_motion(f64::from(u8::from(docked_now.0)), Spring::PANEL, 0.001);
    let right_open = use_motion(f64::from(u8::from(docked_now.1)), Spring::PANEL, 0.001);
    let chosen_before = use_hook(|| CopyValue::new(chosen_now.clone()));
    use_effect(use_reactive!(|docked_now, chosen_now| {
        let (left, right) = docked_now;
        let mut chosen_before = chosen_before;
        let by_the_person = *chosen_before.peek() != chosen_now;
        chosen_before.set(chosen_now);
        for (motion, open) in [(left_open, left), (right_open, right)] {
            let to = f64::from(u8::from(open));
            if by_the_person {
                motion.to(to);
            } else if (motion.target() - to).abs() > f64::EPSILON {
                motion.jump(to);
            }
        }
    }));
    // Two columns start as wide as both need; back to one, the width one
    // column had.
    let layout_now = shell.sidebar_layout();
    let one_w = use_hook(|| CopyValue::new(LEFT_WIDTH));
    // As if one column before: a page that starts on two gets their width.
    let last_layout = use_hook(|| CopyValue::new(SidebarLayout::One));
    use_effect(use_reactive!(|layout_now| {
        let (mut one_w, mut last_layout, mut left_w) = (one_w, last_layout, left_w);
        let was = *last_layout.peek();
        if was == layout_now {
            return;
        }
        let w = *left_w.peek();
        match (was, layout_now) {
            (SidebarLayout::One, SidebarLayout::Two) => {
                one_w.set(w);
                left_w.set(w.max(TWO_WIDTH));
            }
            (_, SidebarLayout::Two) => left_w.set(w.max(TWO_WIDTH)),
            (SidebarLayout::Two, SidebarLayout::One) => left_w.set(*one_w.peek()),
            _ => {}
        }
        last_layout.set(layout_now);
    }));
    // Pages read the sidebar's fold as it moves (the title row's clearance).
    let shell = Shell {
        left_open: Some(left_open),
        ..shell
    };
    let mut ctx = use_context_provider(|| Signal::new(shell.clone()));
    if *ctx.peek() != shell {
        ctx.set(shell.clone());
    }

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

    // The finger's stroke for the keyboard's sake: a tap that ends on a
    // field brings it up (the root's click, below), a scroll does not.
    let taps = keyboard::use_keyboard_taps();
    let (tap_down, tap_move, tap_up, tap_click) = (taps.clone(), taps.clone(), taps.clone(), taps);
    let onclick = move |_: MouseEvent| tap_click.click();
    let (touch_at, resize_at) = (touch.clone(), resize.clone());
    let onpointerdown = move |e: PointerEvent| {
        let c = e.client_coordinates();
        tap_down.down(&e);
        input_debug::hit("the page", &e);
        if !phone || resize_at.get().is_some() {
            return;
        }
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
        tap_move.moved(&e);
        if let Some(r) = resize_mv.get() {
            let dx = c.x - r.x0;
            let (mut w, grow, other) = match r.side {
                Side::Left => (left_w, dx, if docked_now.1 { *right_w.peek() } else { 0.0 }),
                Side::Right => (
                    right_w,
                    -dx,
                    if docked_now.0 { *left_w.peek() } else { 0.0 },
                ),
            };
            let room = (screen.width - RIBBON - PAGE_ROOM - other).max(MIN_WIDTH);
            w.set((r.w0 + grow).clamp(MIN_WIDTH, room));
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
    let onpointerup = move |e: PointerEvent| {
        tap_up.up(&e);
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
    // On a desk the title bar runs across the whole top, over the sides and
    // the ribbon: they all start under it.
    let desk = form == FormFactor::Desk;
    let root_dir = if desk { "column" } else { "row" };
    // The sidebar's title sits in the title bar while a sidebar is docked.
    let bar_title = if docked_left && !page_side {
        title.clone()
    } else {
        None
    };
    // The page's tab starts where the page's column does, past the docked
    // sidebar; otherwise clear of the window's and the sidebar's buttons.
    let tabs_left = if left_clip > 0.0 && has_left {
        // Mid-fold, never under the window's and the sidebar's buttons.
        (RIBBON + left_clip + 4.0).max(shell.content_left())
    } else if left_clip > 0.0 {
        RIBBON + left_clip + 4.0
    } else if has_left {
        shell.content_left()
    } else if shell.window_chrome().is_some() {
        shell.controls_left()
    } else {
        RIBBON + 4.0
    };
    // A page that draws the title row itself starts at the window's top
    // (one row): the shell draws no row across the window, only its piece
    // over what is beside the page (the ribbon, the app's sidebar, the
    // right side). Laid out, not lifted over the shell's row: Blitz finds
    // a press only inside its element's parents' boxes.
    let page_title = frame == Frame::Sidebar && shell.page_draws_title_row();
    let spacer_title = if page_title { bar_title.clone() } else { None };
    let show_status_bar = desk
        && status_style == StatusStyle::Bar
        && (chrome.filler(Slot::Status).is_some() || !all_status.is_empty());
    // A corner badge, or the status bar at the page's foot, sits under the
    // floating button: lift it clear.
    let badge_room = if desk && status_style == StatusStyle::Badge && !all_status.is_empty() {
        STATUS_BADGE
    } else if show_status_bar {
        STATUS_BAR
    } else {
        0.0
    };
    // The floating button's room under it, inside the page: the window's
    // inset and a corner badge only when nothing (a status bar, the Now
    // Playing bar) is at the page's foot below it already.
    let page_foot_room = if show_status_bar || (now_playing.is_some() && !in_bar) {
        20.0
    } else {
        20.0 + screen.insets.bottom + badge_room
    };
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
            // A stacking context (`z-index: 0`): the shell's layers stay
            // inside it, under whatever the app draws over the shell.
            style: "position: relative; z-index: 0; width: 100%; height: 100%; overflow: hidden; display: flex; \
                    flex-direction: {root_dir}; background: {t.desk}; color: {t.ink}; font-family: {t.font};",
            onpointerdown,
            onpointermove,
            onpointerup,
            onpointercancel,
            onclick,
            document::Style { {shell_css(&t)} }
            crate::motion::MotionFrames {}
            // A desk's title bar runs across the whole window, over the ribbon,
            // the sidebar and the page; the middle row starts under it.
            if frame == Frame::Sidebar && desk && !page_title {
                SlotView { slot: Slot::Title,
                    DeskTitleRow { title: bar_title, tabs_left }
                }
            }
            // The middle row: a desk's ribbon, then the sides and the page.
            div { style: "flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: {direction};",
            if frame == Frame::Sidebar && desk {
                SlotView { slot: Slot::Ribbon,
                    DeskRibbon { tabs: tabs.clone(), current, on_tab: pick, title_row: page_title }
                }
            }
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
                        if desk {
                            // The sidebar's edge, in the middle row under the title bar.
                            div { style: "position: absolute; top: 0; bottom: 0; right: 0; width: 1px; background: {t.rule};" }
                            // The app's sidebar beside a page that draws the
                            // title row: its piece of the row.
                            if page_title && !page_side {
                                TitlePiece { title: spacer_title, left: shell.sidebar_title_left() }
                            }
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
                        // A desk's account, switcher and foot buttons are the
                        // ribbon's (`DeskRibbon`); a tablet's are here.
                        if !desk {
                            if let Some(account) = account.clone() {
                                AccountRow { account, bottom: screen.insets.bottom }
                            }
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
                // The page itself, and the floating button over its corner:
                // above the Now Playing bar and the status bar, never
                // under them (#660: the bar's source chip covered it).
                div { style: "position: relative; flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column;",
                    {children}
                    if float_separate {
                        if let Some(sep) = separate.clone() {
                            div {
                                style: "position: absolute; right: {20.0 + screen.insets.right}px; \
                                        bottom: {page_foot_room}px;",
                                RoundButton { action: sep }
                            }
                        }
                    }
                }
                if !in_bar {
                    if let Some(np) = now_playing.clone() {
                        div { style: "position: relative; z-index: {layer::FOOT}; flex-shrink: 0;", {np} }
                    }
                }
                // A desk's status bar is the page's foot: it starts where the
                // left side (rail, sidebar) ends and stops at the right side,
                // so the sidebars run to the window's bottom (Obsidian's).
                if show_status_bar {
                    SlotView { slot: Slot::Status,
                        StatusBar { items: all_status.clone() }
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
                        // A desk's edge, in the middle row under the title bar.
                        if form == FormFactor::Desk {
                            div { style: "position: absolute; top: 0; bottom: 0; left: 0; width: 1px; background: {t.rule};" }
                        }
                        if page_title {
                            TitlePiece { title: None, left: 0.0 }
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
                // Its own layer: a press on it is its own, whatever the
                // page hides under it (`layer::FOOT`).
                div { style: "position: relative; z-index: {layer::FOOT}; flex-shrink: 0; display: flex; flex-direction: column;",
                    if let Some(np) = now_playing {
                        {np}
                    }
                    PhoneFoot { tabs: tabs, current, on_tab: pick, separate: separate, screen }
                }
            }
            }
            if desk && status_style == StatusStyle::Badge {
                StatusBadge { items: all_status }
            }
            // A desk's sides' buttons, fixed in the title row whether the
            // side is showing or not (T3 Code's): the left one just past the
            // window's buttons, the right one at the top right.
            if form == FormFactor::Desk {
                if let Some(controls) = window.as_ref().and_then(|w| w.controls.clone()) {
                    WindowButtons { controls }
                }
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
                    style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; z-index: {layer::SHEET}; \
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
                                span { style: "flex: 1;" }
                                if !foot.is_empty() {
                                    div { style: "display: flex; flex-direction: column; gap: 1px; padding: 6px 8px; border-top: 1px solid {t.rule};",
                                        for action in foot.iter().cloned() {
                                            FootRow { key: "{action.label}", action }
                                        }
                                    }
                                }
                                if let Some(account) = account {
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
            // What the shell hears of touches and the keyboard, when asked.
            if input_debug() {
                input_debug::InputDebugOverlay { screen }
            }
        }
    }
}

/// `content` for `slot` as the chrome has it: a filler's, none when it is
/// hidden, else as given.
fn slot_content(chrome: &Chrome, slot: Slot, content: Option<Element>) -> Option<Element> {
    if chrome.is_hidden(slot) {
        return None;
    }
    chrome.filler(slot).map_or_else(
        || content.map(|c| slot_look(chrome, slot, c)),
        |f| Some((f.render)()),
    )
}

/// `own` in `slot`'s look, when the chrome gives it one.
fn slot_look(chrome: &Chrome, slot: Slot, own: Element) -> Element {
    if chrome.looks.contains_key(&slot) {
        rsx! { SlotLook { slot, {own} } }
    } else {
        own
    }
}

/// A slot of the shell: its filler, nothing when it is hidden, or the
/// shell's own component (`children`, drawn as it always is — in the
/// slot's own look when the chrome gives one).
#[component]
fn SlotView(slot: Slot, children: Element) -> Element {
    let chrome = try_use_context::<Signal<Chrome>>()
        .map(|c| c())
        .unwrap_or_default();
    if chrome.is_hidden(slot) {
        return rsx! {};
    }
    chrome
        .filler(slot)
        .map_or_else(|| slot_look(&chrome, slot, children), |f| (f.render)())
}

/// The shell's own component in a slot's look: the shell below it reads
/// the workspace's theme with the slot's overrides on top (the cascade's
/// nearest node).
#[component]
fn SlotLook(slot: Slot, children: Element) -> Element {
    let outer = use_context::<Signal<Shell>>();
    let chrome = try_use_context::<Signal<Chrome>>()
        .map(|c| c())
        .unwrap_or_default();
    let store = architect_theme::try_use_theme().map(|t| t.store());
    let look = chrome.looks.get(&slot).cloned().unwrap_or_default();
    let theme = chrome::slot_theme(store.as_ref(), &look);
    let mut shell = outer();
    if let Some(theme) = theme {
        shell.theme = theme;
    }
    let mut inner = use_context_provider(|| Signal::new(shell.clone()));
    if *inner.peek() != shell {
        inner.set(shell);
    }
    rsx! { {children} }
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
            style: "position: absolute; top: 0; left: {x}px; width: {width}px; height: 100%; z-index: {layer::SHEET}; \
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
    let t = shell.theme.clone();
    let folder = shell.clone();
    let fold = rsx! {
        button {
            class: "lp",
            "aria-label": "Fold the sidebar",
            style: "width: 36px; height: 36px; flex-shrink: 0; border: none; border-radius: 10px; background: transparent; \
                    display: flex; align-items: center; justify-content: center;",
            onclick: move |_| folder.toggle(Side::Left),
            PanelLeftClose { size: 18, color: t.ink_3.to_string() }
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
        div { style: "height: {TITLE_ROW}px; display: flex; align-items: center; gap: 4px; padding: 0 10px;",
            {name}
            {fold}
        }
    }
}

/// A desk's title bar, across the window's top over the ribbon, the sides
/// and the page: the docked sidebar's name after the sidebar's button (the
/// window's buttons and the sidebar's button sit over its start), the rest
/// moving the window. The page's tab sits in it from `tabs_left`, over the
/// drag area (which runs on past it). The middle row starts under it.
#[component]
fn DeskTitleRow(title: Option<String>, tabs_left: f64) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let row = shell.title_row();
    let top = shell.screen.insets.top;
    // Past the window's buttons and the sidebar's button, where the name is.
    let start = if title.is_some() {
        shell.content_left()
    } else {
        0.0
    };
    let chrome = shell.window;
    let tab = (shell.desk_tab)();
    rsx! {
        div { style: "position: relative; height: {row + top}px; flex-shrink: 0; box-sizing: border-box; display: flex; align-items: center; \
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
            // The page's tab: as tall as the row, its tab centred in it. Only
            // as wide as the tab, so the drag area beside it still moves the window.
            if let Some(tab) = tab {
                // At the row's foot, over its rule (the tab opens into the page).
                div { style: "position: absolute; left: {tabs_left}px; top: {top}px; height: {row}px; display: flex; align-items: flex-end;",
                    SlotView { slot: Slot::Tabs,
                        page::DeskTab { title: tab }
                    }
                }
            }
        }
    }
}

/// A piece of a desk's title row over what is beside a page that draws
/// the row itself ([`use_title_row`]): as tall, the title bar's colour
/// over its rule, `title` (the app's, beside its docked sidebar) from
/// `left`, the rest moving the window.
#[component]
fn TitlePiece(title: Option<String>, left: f64) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let height = shell.title_row() + shell.screen.insets.top;
    rsx! {
        div { style: "height: {height}px; flex-shrink: 0; box-sizing: border-box; display: flex; align-items: center; \
                      padding: {shell.screen.insets.top}px 0 0 {left}px; background: {t.title_bar}; border-bottom: 1px solid {t.rule};",
            if let Some(title) = title {
                span { style: "flex-shrink: 0; white-space: nowrap; font-size: 15px; font-weight: 700; \
                               letter-spacing: -0.01em; color: {t.ink};",
                    "{title}"
                }
            }
            if let Some(c) = shell.window {
                DragStrip { chrome: c, width: None }
            }
        }
    }
}

/// A desk's button for a side, fixed in the title row.
#[component]
fn TitleControl(side: Side) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
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
                    Side::Left => rsx! { PanelLeft { size: 17, color: t.ink_2.to_string() } },
                    Side::Right => rsx! { PanelRight { size: 17, color: t.ink_2.to_string() } },
                }
            }
        }
    }
}

/// The window's close, minimize and maximize as a Mac's traffic lights,
/// at the title row's left where a Mac's sit (when the shell draws the
/// window's title bar off a Mac). Their glyphs show while the pointer is
/// over them.
#[component]
fn WindowButtons(controls: WindowControls) -> Element {
    let shell = use_shell();
    let row = shell.title_row();
    let top = shell.screen.insets.top;
    let left = shell.screen.insets.left + WindowChrome::MAC_LIGHTS_LEFT;
    let (close, minimize, maximize) = (
        controls.on_close,
        controls.on_minimize,
        controls.on_maximize,
    );
    let light = |colour: &str| {
        format!(
            "width: {LIGHT}px; height: {LIGHT}px; box-sizing: border-box; padding: 0; border: none; \
             border-radius: 50%; background: {colour}; display: flex; align-items: center; justify-content: center;"
        )
    };
    let (red, yellow, green) = (light("#ff5f57"), light("#febc2e"), light("#28c840"));
    rsx! {
        div { class: "lights", style: "position: absolute; top: {top}px; left: {left}px; height: {row}px; display: flex; \
                      align-items: center; gap: {LIGHT_GAP}px; z-index: 30;",
            button { "aria-label": "Close", style: "{red}", onclick: move |_| close.call(()),
                X { size: 8, color: "#4d0000".to_owned() }
            }
            button { "aria-label": "Minimize", style: "{yellow}", onclick: move |_| minimize.call(()),
                Minus { size: 8, color: "#5a3c00".to_owned() }
            }
            button { "aria-label": "Maximize", style: "{green}", onclick: move |_| maximize.call(()),
                Square { size: 6, color: "#003a00".to_owned() }
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
    let t = shell.theme.clone();
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
            PanelLeft { size: 20, color: t.ink_3.to_string() }
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
    let fill = if open() { &*t.active } else { "transparent" };
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
                    ChevronsUpDown { size: 15, color: t.ink_3.to_string() }
                }
            }
            if let (true, Some(menu)) = (open(), menu) {
                AccountMenu { account, menu, open, at: format!("left: 8px; right: 8px; bottom: {}px;", 58.0 + bottom) }
            }
        }
    }
}

/// The account's menu while `open`, placed by `at` against its trigger's
/// box, headed by who this is.
#[component]
fn AccountMenu(account: Account, menu: Menu, open: Signal<bool>, at: String) -> Element {
    let t = use_shell().theme;
    rsx! {
        // Far past every edge: Blitz has no `position: fixed`.
        div {
            style: "position: absolute; top: -4000px; left: -4000px; width: 8000px; height: 8000px; z-index: {layer::MENU};",
            onclick: move |_| open.set(false),
        }
        div {
            style: "position: absolute; {at} z-index: {layer::MENU}; \
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

/// The account as its avatar alone, at the foot of a desk's ribbon while
/// the sidebar is folded: the same menu as the sidebar's row, opening up
/// to its right.
#[component]
fn AccountMark(account: Account) -> Element {
    let t = use_shell().theme;
    let mut open = use_signal(|| false);
    let press = account.on_press;
    let menu = account.menu.clone();
    let has_menu = menu.is_some();
    let fill = if open() { &*t.active } else { "transparent" };
    rsx! {
        div { style: "position: relative; display: flex; flex-shrink: 0;",
            button {
                class: "lp tip-host",
                "aria-label": "Account: {account.name}",
                style: "position: relative; width: 36px; height: 36px; border: none; border-radius: 8px; background: {fill}; \
                        display: flex; align-items: center; justify-content: center;",
                onclick: move |_| {
                    if has_menu {
                        open.toggle();
                    } else if let Some(p) = press {
                        p.call(());
                    }
                },
                Avatar { account: account.clone(), size: 28.0 }
                span { class: "tip", style: "left: 44px; top: 8px;", "{account.name}" }
            }
            if let (true, Some(menu)) = (open(), menu) {
                AccountMenu { account: account.clone(), menu, open, at: "left: 44px; bottom: 0px; width: 260px;".to_owned() }
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
    fn the_default_theme_is_the_default_shell_theme() {
        assert_eq!(
            ShellTheme::from_resolved(&architect_theme::Resolved::default()),
            ShellTheme::default()
        );
    }

    #[test]
    fn a_focused_text_field_rings_in_the_accent_not_blitz_blue() {
        // Blitz never matches `:focus-visible`; its own sheet outlines a
        // focused input in #4D90FE. The shell's `:focus` rule replaces it.
        let t = ShellTheme::default();
        let css = shell_css(&t);
        let rule = format!(
            "input:focus, textarea:focus, select:focus, .focus-ring:focus-within {{ outline: 2px solid {};",
            t.accent
        );
        assert!(css.contains(&rule), "{css}");
    }

    thread_local! {
        static STORE: std::cell::RefCell<Option<architect_theme::ThemeStore>> =
            const { std::cell::RefCell::new(None) };
        static SEEN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn themed_app() -> Element {
        architect_theme::use_theme_store(|| STORE.with(|s| s.borrow().clone().unwrap()));
        let screen = Screen {
            width: 1000.0,
            height: 800.0,
            touch: false,
            insets: Insets::default(),
        };
        rsx! {
            AppShell { screen, tabs: vec![], current: None, on_tab: |_| (),
                Probe {}
            }
        }
    }

    #[component]
    fn Probe() -> Element {
        let t = use_shell().theme;
        SEEN.with(|s| s.borrow_mut().push(format!("{} {}", t.desk, t.accent)));
        rsx! {}
    }

    #[test]
    fn app_shell_follows_the_theme_store() {
        use architect_theme::{Layer, LayerKind, ThemeChoice, ThemeStore};

        let store = ThemeStore::default();
        STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
        let mut dom = VirtualDom::new(themed_app);
        dom.rebuild_in_place();
        assert_eq!(
            SEEN.with(|s| s.borrow().last().cloned()).unwrap(),
            "#09090b #2dd4bf"
        );

        store.set(
            LayerKind::Pick,
            Layer::default().choice(ThemeChoice::One("nord-light".into())),
        );
        futures::executor::block_on(dom.wait_for_work());
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
        let want = ShellTheme::from_resolved(&store.current());
        assert_eq!(
            SEEN.with(|s| s.borrow().last().cloned()).unwrap(),
            format!("{} {}", want.desk, want.accent)
        );
        assert_ne!(&*want.desk, "#09090b");
    }

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

/// A desk's ribbon: the window's left edge in the middle row, under the
/// title bar, the app's sections as icons, then what the page adds (a
/// workspace's views and pinned commands). Its foot is the app's foot
/// buttons (Theme, Settings), the workspace switcher and the account, as
/// marks, open sidebar or not: they never move into the sidebar (#353).
#[component]
fn DeskRibbon(
    tabs: Vec<Tab>,
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
    /// The page draws the title row: the ribbon starts at the window's top
    /// with its piece of the row.
    title_row: bool,
) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let extra = (shell.rail_extra)();
    let (workspaces, account, foot) = (shell.workspaces, shell.account, shell.foot);
    rsx! {
        div {
            style: "width: {RIBBON}px; flex-shrink: 0; display: flex; flex-direction: column; background: {t.desk};",
            if title_row {
                TitlePiece { title: None, left: 0.0 }
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
                if workspaces.is_some() || account.is_some() || !foot.is_empty() {
                    span { style: "flex: 1;" }
                    for action in foot {
                        FootIcon { key: "{action.label}", action }
                    }
                    if let Some(workspaces) = workspaces {
                        div { style: "display: flex; width: 36px;",
                            Switcher { workspaces, place: Place::Ribbon }
                        }
                    }
                    if let Some(account) = account {
                        AccountMark { account }
                    }
                }
            }
        }
    }
}

/// One of the app's foot buttons in a docked sidebar: its icon and label.
#[component]
fn FootRow(action: Action) -> Element {
    let t = use_shell().theme;
    let press = action.on_press;
    rsx! {
        button {
            class: "lp",
            "aria-label": "{action.label}",
            style: "height: 30px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border: none; \
                    border-radius: 7px; background: transparent; color: {t.ink_2}; font-family: {t.font}; \
                    font-size: 13px; font-weight: 550; text-align: left;",
            onclick: move |_| press.call(()),
            {action.draw(Icon { size: 16, color: t.ink_3.clone() })}
            span { "{action.label}" }
        }
    }
}

/// One of the app's foot buttons in a folded ribbon or a rail: its icon,
/// its label on hover.
#[component]
pub(super) fn FootIcon(action: Action) -> Element {
    let t = use_shell().theme;
    let press = action.on_press;
    rsx! {
        button {
            class: "lp tip-host",
            "aria-label": "{action.label}",
            style: "position: relative; width: 32px; height: 32px; flex-shrink: 0; align-self: center; border: none; \
                    border-radius: 7px; background: transparent; display: flex; align-items: center; justify-content: center;",
            onclick: move |_| press.call(()),
            {action.draw(Icon { size: 18, color: t.ink_3 })}
            span { class: "tip", style: "left: 40px; top: 5px;", "{action.label}" }
        }
    }
}

/// An app section in a desk's ribbon: its icon, its name on hover.
#[component]
fn RibbonTab(tab: Tab, on: bool, on_tab: EventHandler<&'static str>) -> Element {
    let t = use_shell().theme;
    // The current section: the accent, faint behind it and full in it.
    let (fill, colour) = if on {
        (&*t.accent_soft, &*t.accent)
    } else {
        ("transparent", &*t.ink_3)
    };
    let id = tab.id;
    rsx! {
        button {
            class: "lp tip-host",
            "aria-label": tab.label,
            "data-tab": id,
            "aria-current": if on { "page" } else { "false" },
            style: "position: relative; width: 32px; height: 32px; flex-shrink: 0; border: none; border-radius: 7px; \
                    background: {fill}; display: flex; align-items: center; justify-content: center;",
            onclick: move |_| on_tab.call(id),
            {tab.draw(Icon { size: 18, color: colour.into() })}
            span { class: "tip", style: "left: 40px; top: 5px;", "{tab.label}" }
            if tab.badge.is_some() {
                span { style: "position: absolute; top: 5px; right: 5px; width: 6px; height: 6px; border-radius: 3px; \
                               background: {t.accent};" }
            }
        }
    }
}
