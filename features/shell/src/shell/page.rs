//! A page: its bar (the title, the main action, the ⋯ [`Menu`]) over its
//! body, which scrolls and keeps clear of the screen's edges.

use dioxus::prelude::*;
use lucide_dioxus::{Check, ChevronLeft, Ellipsis, PanelRight};

use super::{Action, DragStrip, FormFactor, Icon, IconFn, ShellStyle, Side, use_shell};

// ── The ⋯ menu ──────────────────────────────────────────────────────────────

/// One row of a [`Menu`].
#[derive(Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<IconFn>,
    /// A trailing note: "2 on", a shortcut.
    pub note: Option<String>,
    /// Shown ticked (a choice among several, a setting that is on).
    pub checked: bool,
    /// Drawn in the theme's danger colour (Delete).
    pub danger: bool,
    pub on_select: Callback<()>,
}

/// What a [`Menu`] holds.
#[derive(Clone, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    /// A hairline between groups.
    Divider,
    /// A small heading over a group.
    Heading(String),
}

/// A page's ⋯ menu, built in a chain; a modifier (`note`, `danger`)
/// applies to the row just added:
///
/// ```rust,ignore
/// Menu::new()
///     .item("Filter", icon!(Funnel), move |_| filter.open()).note("2 on")
///     .divider()
///     .heading("Layout")
///     .check("Compact", compact(), move |_| compact.toggle())
///     .divider()
///     .item("Delete list", icon!(Trash), move |_| delete()).danger()
/// ```
///
/// On a desk the same rows are the page's commands for the app's palette
/// ([`Menu::items`]).
#[derive(Clone, PartialEq, Default)]
pub struct Menu {
    pub entries: Vec<MenuEntry>,
}

impl Menu {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A row: its label, its icon (`icon!(Funnel)`, or `None`), what
    /// picking it does.
    #[must_use]
    pub fn item(
        mut self,
        label: impl Into<String>,
        icon: impl Into<Option<IconFn>>,
        on_select: impl FnMut(()) + 'static,
    ) -> Self {
        self.entries.push(MenuEntry::Item(MenuItem {
            label: label.into(),
            icon: icon.into(),
            note: None,
            checked: false,
            danger: false,
            on_select: Callback::new(on_select),
        }));
        self
    }

    /// A row that is ticked when `on`.
    #[must_use]
    pub fn check(
        self,
        label: impl Into<String>,
        on: bool,
        on_select: impl FnMut(()) + 'static,
    ) -> Self {
        let mut menu = self.item(label, None, on_select);
        if let Some(MenuEntry::Item(row)) = menu.entries.last_mut() {
            row.checked = on;
        }
        menu
    }

    /// A note at the end of the last row.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        if let Some(MenuEntry::Item(row)) = self.entries.last_mut() {
            row.note = Some(note.into());
        }
        self
    }

    /// The last row in the danger colour.
    #[must_use]
    pub fn danger(mut self) -> Self {
        if let Some(MenuEntry::Item(row)) = self.entries.last_mut() {
            row.danger = true;
        }
        self
    }

    #[must_use]
    pub fn divider(mut self) -> Self {
        self.entries.push(MenuEntry::Divider);
        self
    }

    #[must_use]
    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.entries.push(MenuEntry::Heading(heading.into()));
        self
    }

    /// Rows only (for a command palette).
    pub fn items(&self) -> impl Iterator<Item = &MenuItem> {
        self.entries.iter().filter_map(|e| match e {
            MenuEntry::Item(row) => Some(row),
            _ => None,
        })
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── The breadcrumb ──────────────────────────────────────────────────────────

/// One step of a page's breadcrumb: where it sits (a project, a folder),
/// before the page's own title.
#[derive(Clone, PartialEq)]
pub struct Crumb {
    pub label: String,
    /// An icon before it (the first crumb's, usually: a project's mark).
    pub icon: Option<IconFn>,
    /// Going there; a crumb without one is only said.
    pub on_press: Option<Callback<()>>,
}

impl Crumb {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            on_press: None,
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: IconFn) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn on_press(mut self, on_press: impl FnMut(()) + 'static) -> Self {
        self.on_press = Some(Callback::new(on_press));
        self
    }
}

/// Where the page is, T3 Code's way: the crumbs muted and pressable, a
/// muted "/" between, the page's title last in full ink. The title and
/// the space after it move the window, when the shell has one to move.
#[component]
pub fn Breadcrumb(crumbs: Vec<Crumb>, #[props(into)] current: String) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let (drag, zoom) = shell
        .window
        .as_ref()
        .map_or((None, None), |c| (Some(c.on_drag), Some(c.on_zoom)));
    rsx! {
        nav {
            "aria-label": "Breadcrumb",
            style: "flex: 1; min-width: 0; align-self: stretch; display: flex; align-items: center; gap: 8px; \
                    font-family: {t.font}; font-size: 14px; font-weight: 500;",
            for (i, crumb) in crumbs.into_iter().enumerate() {
                CrumbItem { key: "{i}", crumb }
            }
            span {
                "aria-current": "page",
                style: "flex: 0 1 auto; min-width: 0; white-space: nowrap; overflow: hidden; color: {t.ink}; font-weight: 600;",
                onmousedown: move |_| if let Some(d) = drag { d.call(()) },
                ondoubleclick: move |_| if let Some(z) = zoom { z.call(()) },
                "{current}"
            }
            div {
                style: "flex: 1; min-width: 12px; align-self: stretch;",
                onmousedown: move |_| if let Some(d) = drag { d.call(()) },
                ondoubleclick: move |_| if let Some(z) = zoom { z.call(()) },
            }
        }
    }
}

#[component]
fn CrumbItem(crumb: Crumb) -> Element {
    let t = use_shell().theme;
    let press = crumb.on_press;
    rsx! {
        button {
            class: "lp",
            style: "flex-shrink: 1; min-width: 0; max-width: 200px; height: 26px; display: flex; align-items: center; gap: 6px; \
                    padding: 0 4px; margin: 0 -4px; border: none; border-radius: 6px; background: transparent; \
                    color: {t.ink_2}; font-family: {t.font}; font-size: 14px; font-weight: 500; white-space: nowrap; overflow: hidden;",
            onclick: move |_| if let Some(p) = press { p.call(()) },
            if let Some(icon) = crumb.icon {
                {icon(Icon { size: 14, color: t.ink_3 })}
            }
            span { style: "min-width: 0; overflow: hidden;", "{crumb.label}" }
        }
        span { style: "flex-shrink: 0; color: {t.ink_3};", "/" }
    }
}

// ── The toolbar ─────────────────────────────────────────────────────────────

/// One button of a page's [`Toolbar`].
#[derive(Clone, PartialEq)]
pub enum Tool {
    /// Press it, it acts.
    Action(Action),
    /// Press it, its menu opens: the ⋯, a filter, a scope.
    Menu {
        label: String,
        icon: IconFn,
        menu: Menu,
        /// How many things it narrows by, marked on it.
        badge: usize,
    },
}

impl Tool {
    #[must_use]
    pub const fn action(action: Action) -> Self {
        Self::Action(action)
    }

    /// A button that opens `menu`.
    pub fn menu(label: impl Into<String>, icon: IconFn, menu: Menu) -> Self {
        Self::Menu {
            label: label.into(),
            icon,
            menu,
            badge: 0,
        }
    }

    /// The ⋯: everything else the page offers.
    #[must_use]
    pub fn more(menu: Menu) -> Self {
        Self::menu(
            "More",
            |i| rsx! { Ellipsis { size: i.size, color: i.color } },
            menu,
        )
    }

    /// Mark a menu button with what narrows the view.
    #[must_use]
    pub fn badge(self, n: usize) -> Self {
        match self {
            Self::Menu {
                label, icon, menu, ..
            } => Self::Menu {
                label,
                icon,
                menu,
                badge: n,
            },
            Self::Action(action) => Self::Action(action),
        }
    }
}

impl From<Action> for Tool {
    fn from(action: Action) -> Self {
        Self::Action(action)
    }
}

/// Buttons side by side: joined in one pill, or each on its own.
#[derive(Clone, PartialEq)]
pub struct ToolGroup {
    pub tools: Vec<Tool>,
    pub joined: bool,
}

/// A page's buttons at the top right, in groups.
///
/// A group is **joined** (one pill, its buttons split by hairlines — T3
/// Code's thread actions) or each button is **separate** (its own round
/// button — T3 Code's search and scope):
///
/// ```rust,ignore
/// Toolbar::new()
///     .separate(Tool::menu("Scope", icon!(Funnel), scope_menu).badge(1))
///     .separate(Action::new("Search", icon!(Search), move |_| search.open()))
///     .joined([Tool::action(new_task), Tool::more(menu)])
/// ```
#[derive(Clone, PartialEq, Default)]
pub struct Toolbar {
    pub groups: Vec<ToolGroup>,
}

impl Toolbar {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One button on its own.
    #[must_use]
    pub fn separate(mut self, tool: impl Into<Tool>) -> Self {
        self.groups.push(ToolGroup {
            tools: vec![tool.into()],
            joined: false,
        });
        self
    }

    /// Several buttons in one pill.
    #[must_use]
    pub fn joined(mut self, tools: impl IntoIterator<Item = Tool>) -> Self {
        let tools: Vec<Tool> = tools.into_iter().collect();
        if !tools.is_empty() {
            self.groups.push(ToolGroup {
                tools,
                joined: true,
            });
        }
        self
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// What a page's `action`, `menu` and `badge` amount to: one pill of
    /// the action and the ⋯.
    fn of(action: Option<Action>, menu: Menu, badge: usize) -> Self {
        let mut tools: Vec<Tool> = action.into_iter().map(Tool::Action).collect();
        if !menu.is_empty() {
            tools.push(Tool::more(menu).badge(badge));
        }
        Self::new().joined(tools)
    }
}

/// How big the toolbar's buttons are: a finger's, or a desk's.
#[derive(Clone, Copy, PartialEq, Debug)]
struct ToolSize {
    /// A button's side.
    button: f64,
    icon: usize,
    /// A pill's or a separate button's corner.
    radius: f64,
    font: f64,
}

impl ToolSize {
    const TOUCH: Self = Self {
        button: 44.0,
        icon: 20,
        radius: 22.0,
        font: 15.0,
    };
    const DESK: Self = Self {
        button: 30.0,
        icon: 16,
        radius: 9.0,
        font: 13.0,
    };
}

/// The toolbar, its groups left to right.
#[component]
fn ToolbarView(toolbar: Toolbar, size: ToolSize) -> Element {
    rsx! {
        div { style: "flex-shrink: 0; display: flex; align-items: center; gap: 8px;",
            for (i, group) in toolbar.groups.into_iter().enumerate() {
                Group { key: "{i}", group, size }
            }
        }
    }
}

#[component]
fn Group(group: ToolGroup, size: ToolSize) -> Element {
    let t = use_shell().theme;
    let inner = size.button - 4.0;
    if group.joined {
        let n = group.tools.len();
        return rsx! {
            div {
                style: "height: {size.button}px; box-sizing: border-box; display: flex; align-items: center; padding: 0 1px; \
                        border-radius: {size.radius}px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: {t.shadow_lift};",
                for (i, tool) in group.tools.into_iter().enumerate() {
                    ToolButton { key: "{i}", tool, size, side: inner, framed: false }
                    if i + 1 < n {
                        span { key: "d{i}", style: "width: 1px; height: {size.button * 0.45}px; background: {t.rule};" }
                    }
                }
            }
        };
    }
    rsx! {
        for (i, tool) in group.tools.into_iter().enumerate() {
            ToolButton { key: "{i}", tool, size, side: size.button, framed: true }
        }
    }
}

/// One button: inside a pill (`framed: false`) or on its own.
#[component]
fn ToolButton(tool: Tool, size: ToolSize, side: f64, framed: bool) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    // A long menu scrolls inside the window rather than running past it.
    let menu_most = (shell.screen.height - shell.screen.insets.bottom - side - 72.0).max(160.0);
    let mut open = use_signal(|| false);
    let frame = if framed {
        format!(
            "border-radius: {r}px; background: {bg}; border: 1px solid {rule}; box-sizing: border-box; \
             box-shadow: {shadow};",
            r = size.radius,
            bg = t.pill,
            rule = t.rule,
            shadow = t.shadow_lift
        )
    } else {
        format!(
            "border-radius: {}px; background: transparent; border: none;",
            size.radius - 2.0
        )
    };
    let icon_only = format!("width: {side}px; padding: 0;");
    match tool {
        Tool::Action(action) => {
            let press = action.on_press;
            let pad = if action.show_label {
                "padding: 0 12px 0 9px;".to_owned()
            } else {
                icon_only
            };
            rsx! {
                button {
                    class: "lp",
                    "aria-label": "{action.label}",
                    style: "height: {side}px; {pad} {frame} flex-shrink: 0; display: flex; align-items: center; justify-content: center; \
                            gap: 6px; color: {t.ink}; font-family: {t.font}; font-size: {size.font}px; font-weight: 650;",
                    onclick: move |_| press.call(()),
                    {(action.icon)(Icon { size: size.icon, color: t.ink })}
                    if action.show_label { span { "{action.label}" } }
                }
            }
        }
        Tool::Menu {
            label,
            icon,
            menu,
            badge,
        } => {
            let fill = if open() { t.active } else { "transparent" };
            let fill_style = if framed {
                String::new()
            } else {
                format!("background: {fill};")
            };
            rsx! {
                div { style: "position: relative; flex-shrink: 0; display: flex;",
                    button {
                        class: "lp",
                        "aria-label": "{label}",
                        style: "position: relative; height: {side}px; {icon_only} {frame} {fill_style} display: flex; align-items: center; \
                                justify-content: center;",
                        onclick: move |_| open.toggle(),
                        {icon(Icon { size: size.icon, color: t.ink })}
                        if badge > 0 {
                            span {
                                style: "position: absolute; top: -3px; right: -3px; min-width: 16px; height: 16px; padding: 0 4px; \
                                        box-sizing: border-box; border-radius: 8px; display: flex; align-items: center; \
                                        justify-content: center; background: {t.accent}; color: {t.accent_ink}; \
                                        font-size: 10px; font-weight: 700;",
                                "{badge}"
                            }
                        }
                    }
                    if open() {
                        // Far past every edge: Blitz has no `position: fixed`.
                        div {
                            style: "position: absolute; top: -2000px; left: -4000px; width: 8000px; height: 8000px; z-index: 40;",
                            onclick: move |_| open.set(false),
                        }
                        div {
                            style: "position: absolute; right: -2px; top: {side + 8.0}px; z-index: {super::layer::MENU}; min-width: 248px; \
                                    max-height: {menu_most}px; overflow-y: scroll; box-sizing: border-box; \
                                    display: flex; flex-direction: column; padding: 6px 0; border-radius: 16px; \
                                    background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                            for (i, entry) in menu.entries.iter().cloned().enumerate() {
                                MenuRow { key: "{i}", entry, open }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── A page ──────────────────────────────────────────────────────────────────

/// `#rrggbb` as `rgba(r,g,b,a)`; anything else as it is.
fn with_alpha(hex: &str, alpha: f64) -> String {
    let h = hex.trim_start_matches('#');
    let channel = |i: usize| {
        h.get(i..i.saturating_add(2))
            .and_then(|c| u8::from_str_radix(c, 16).ok())
    };
    match (h.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => format!("rgba({r},{g},{b},{alpha})"),
        _ => hex.to_owned(),
    }
}

/// A page: its bar over its body. The body scrolls (unless `scroll:
/// false`, for a page that lays out its own panes) and keeps clear of the
/// screen's edges and of the bar at the foot.
///
/// On a touch screen the bar floats over the body, fading into it, so the
/// body scrolls up under it (T3 Code's frosted header; Blitz has no
/// backdrop blur, so the page's colour fades in instead). A root page — no
/// way back — under an app with a [`brand`](AppShell::brand) leads with the
/// brand, its title large at the top of the body.
#[component]
pub fn Page(
    #[props(into)] title: String,
    /// Something to lead with instead of the title.
    lead: Option<Element>,
    /// A way back: a chevron before the title. A page without one is a
    /// root page.
    on_back: Option<EventHandler<()>>,
    /// The main action (with `menu`, one pill); or say it all in `toolbar`.
    action: Option<Action>,
    /// How many things narrow the view, marked on the ⋯.
    #[props(default)]
    badge: usize,
    #[props(default)] menu: Menu,
    /// The page's buttons, grouped ([`Toolbar`]); overrides `action`/`menu`.
    toolbar: Option<Toolbar>,
    /// Where the page sits, before its title: on a desk, the breadcrumb.
    #[props(default)]
    crumbs: Vec<Crumb>,
    /// Anything else the app keeps at the bar's end.
    trailing: Option<Element>,
    #[props(default = true)] scroll: bool,
    children: Element,
) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let ins = shell.screen.insets;
    let toolbar = toolbar.unwrap_or_else(|| Toolbar::of(action, menu, badge));
    let phone = shell.form() == FormFactor::Phone;
    let upright_phone = phone && !shell.screen.landscape();
    // Beside a rail the rail keeps the left inset; on its own the page does.
    let (left, right) = if upright_phone {
        (20.0 + ins.left, 20.0 + ins.right)
    } else {
        (20.0, 20.0 + ins.right)
    };
    let overflow = if scroll {
        "overflow-y: scroll;"
    } else {
        "overflow: hidden;"
    };
    if shell.desk() {
        return rsx! {
            DeskBar { title, lead, on_back, toolbar, crumbs, trailing }
            div {
                style: "flex: 1; min-height: 0; {overflow} display: flex; flex-direction: column; padding: 4px {right}px 24px {left}px;",
                {children}
            }
        };
    }
    // The brand leads a root page; its title goes large into the body.
    let branded = on_back.is_none() && lead.is_none() && shell.brand.is_some() && phone;
    let bar_h = touch_bar_height(&shell);
    let fade_to = with_alpha(t.desk, 0.0);
    rsx! {
        div { style: "position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column;",
            div {
                style: "flex: 1; min-height: 0; {overflow} display: flex; flex-direction: column; \
                        padding: {bar_h + 4.0}px {right}px 24px {left}px;",
                if branded && !title.is_empty() {
                    h1 { style: "margin: 4px 0 12px; font-family: {t.font}; font-size: 30px; line-height: 1.15; \
                                 font-weight: 750; letter-spacing: -0.02em; color: {t.ink};",
                        "{title}"
                    }
                }
                {children}
            }
            // Over the body: solid behind the bar, fading out below it.
            div {
                style: "position: absolute; top: 0; left: 0; width: 100%; height: {bar_h + 18.0}px; pointer-events: none; \
                        background: linear-gradient(to bottom, {t.desk} 0%, {t.desk} 62%, {fade_to} 100%);",
            }
            div { style: "position: absolute; top: 0; left: 0; width: 100%;",
                TouchBar { title, lead, on_back, toolbar, trailing, branded }
            }
        }
    }
}

/// A page's top bar, for a page that lays out its body itself; [`Page`]
/// is the bar and the body together.
#[component]
pub fn PageBar(
    #[props(into)] title: String,
    lead: Option<Element>,
    on_back: Option<EventHandler<()>>,
    action: Option<Action>,
    #[props(default)] badge: usize,
    #[props(default)] menu: Menu,
    toolbar: Option<Toolbar>,
    #[props(default)] crumbs: Vec<Crumb>,
    trailing: Option<Element>,
) -> Element {
    let shell = use_shell();
    let toolbar = toolbar.unwrap_or_else(|| Toolbar::of(action, menu, badge));
    if shell.desk() {
        return rsx! { DeskBar { title, lead, on_back, toolbar, crumbs, trailing } };
    }
    rsx! { TouchBar { title, lead, on_back, toolbar, trailing, branded: false } }
}

/// A touch screen's bar: the brand (a root page) or the way back and the
/// title on the left, the toolbar on the right; on a tablet with a right
/// side, its button at the end.
#[component]
fn TouchBar(
    title: String,
    lead: Option<Element>,
    on_back: Option<EventHandler<()>>,
    toolbar: Toolbar,
    trailing: Option<Element>,
    branded: bool,
) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let screen = shell.screen;
    let phone = shell.form() == FormFactor::Phone;
    let upright_phone = phone && !screen.landscape() && shell.style != ShellStyle::RailOnly;
    let ins = screen.insets;
    let pad_left = if upright_phone { 20.0 + ins.left } else { 20.0 };
    let top = if upright_phone {
        ins.top + 4.0
    } else {
        ins.top
    };
    let height = touch_bar_height(&shell) - top;
    // A root page's title is large; a page pushed on top of one says its
    // name small beside the way back, as iOS does.
    let size = match (on_back.is_some(), phone && !screen.landscape()) {
        (true, _) => 17,
        (false, true) => 28,
        (false, false) => 22,
    };
    let weight = if on_back.is_some() { 650 } else { 750 };
    let right_button = !phone && shell.has_side(Side::Right);
    let brand = if branded { shell.brand.clone() } else { None };
    rsx! {
        div {
            style: "position: relative; flex-shrink: 0; box-sizing: border-box; height: {height + top}px; \
                    padding: {top}px {20.0 + ins.right}px 0 {pad_left}px; display: flex; align-items: center; gap: 10px;",
            if let Some(back) = on_back {
                button {
                    class: "lp",
                    "aria-label": "Back",
                    style: "width: 40px; height: 40px; margin-left: -10px; flex-shrink: 0; border: none; border-radius: 20px; \
                            background: transparent; display: flex; align-items: center; justify-content: center;",
                    onclick: move |_| back.call(()),
                    ChevronLeft { size: 26, color: t.ink }
                }
            }
            if let Some(brand) = brand {
                div { style: "flex: 1; min-width: 0; display: flex; align-items: center;", "aria-label": "{title}", {brand} }
            } else if let Some(lead) = lead {
                div { style: "flex: 1; min-width: 0;", "aria-label": "{title}", {lead} }
            } else {
                h1 {
                    style: "flex: 1; min-width: 0; margin: 0; white-space: nowrap; overflow: hidden; \
                            font-size: {size}px; line-height: 1.2; font-weight: {weight}; letter-spacing: -0.02em; color: {t.ink};",
                    "{title}"
                }
            }
            if let Some(trailing) = trailing { {trailing} }
            ToolbarView { toolbar, size: ToolSize::TOUCH }
            if right_button {
                ToolButton {
                    tool: Tool::action(Action::new("Details", |i| rsx! { PanelRight { size: i.size, color: i.color } }, move |()| shell.toggle(Side::Right))),
                    size: ToolSize::TOUCH,
                    side: ToolSize::TOUCH.button,
                    framed: true,
                }
            }
        }
    }
}

/// A touch screen's bar's whole height, its top inset in.
fn touch_bar_height(shell: &super::Shell) -> f64 {
    let screen = shell.screen;
    let upright_phone = shell.form() == FormFactor::Phone
        && !screen.landscape()
        && shell.style != ShellStyle::RailOnly;
    if upright_phone {
        screen.insets.top + 4.0 + 52.0
    } else {
        screen.insets.top + 56.0
    }
}

#[component]
pub(super) fn MenuRow(entry: MenuEntry, open: Signal<bool>) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    // A finger's row, or a mouse's.
    let (row_h, font) = if shell.screen.touch {
        (46, 16)
    } else {
        (34, 14)
    };
    match entry {
        MenuEntry::Divider => {
            rsx! { div { style: "height: 1px; margin: 4px 16px; background: {t.rule};" } }
        }
        MenuEntry::Heading(text) => rsx! {
            div { style: "padding: 10px 16px 4px; font-size: 12px; font-weight: 650; color: {t.ink_3};", "{text}" }
        },
        MenuEntry::Item(row) => {
            let colour = if row.danger { t.danger } else { t.ink };
            let pick = row.on_select;
            rsx! {
                button {
                    class: "lp",
                    style: "height: {row_h}px; flex-shrink: 0; display: flex; align-items: center; justify-content: flex-start; \
                            gap: 12px; padding: 0 16px; border: none; background: transparent; color: {colour}; \
                            font-family: {t.font}; font-size: {font}px; text-align: left;",
                    onclick: move |_| {
                        open.set(false);
                        pick.call(());
                    },
                    span { style: "width: 20px; display: flex; justify-content: center;",
                        if let Some(icon) = row.icon {
                            {icon(Icon { size: 18, color: if row.danger { t.danger } else { t.ink_3 } })}
                        } else if row.checked {
                            Check { size: 18, color: t.ink }
                        }
                    }
                    span { style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden;", "{row.label}" }
                    if row.icon.is_some() && row.checked {
                        Check { size: 16, color: t.ink }
                    }
                    if let Some(n) = row.note {
                        span { style: "font-size: 13px; color: {t.ink_3};", "{n}" }
                    }
                }
            }
        }
    }
}

/// A desk's page bar, Obsidian's way: the title row holds the page as a
/// tab (rounded at the top, the page's colour, running on into it), the
/// rest of the row moving the window; under it the page's header — the way
/// back, the breadcrumb, the page's toolbar. Clear at each end of the
/// sides' fixed buttons where they sit over the title row.
#[component]
fn DeskBar(
    title: String,
    lead: Option<Element>,
    on_back: Option<EventHandler<()>>,
    toolbar: Toolbar,
    crumbs: Vec<Crumb>,
    trailing: Option<Element>,
) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let ins = shell.screen.insets;
    let row = shell.title_row();
    let (left, right) = shell.title_clearance();
    let pad_left = left.max(4.0);
    let tab_h = (row - 6.0).max(24.0);
    let chrome = shell.window_chrome().cloned();
    rsx! {
        div {
            style: "position: relative; flex-shrink: 0; box-sizing: border-box; height: {row + ins.top}px; \
                    padding: {ins.top}px {right}px 0 {pad_left}px; display: flex; align-items: flex-end; \
                    background: {t.title_bar};",
            // The bar's rule, under everything but the tab (drawn over it).
            div { style: "position: absolute; left: 0; right: 0; bottom: 0; height: 1px; background: {t.rule};" }
            div {
                style: "position: relative; z-index: 1; flex: 0 1 200px; max-width: 200px; min-width: 72px; \
                        height: {tab_h}px; box-sizing: border-box; display: flex; align-items: center; \
                        padding: 0 12px; border-radius: 8px 8px 0 0; background: {t.desk}; color: {t.ink}; \
                        font-family: {t.font}; font-size: 13px; white-space: nowrap; overflow: hidden;",
                "{title}"
            }
            if let Some(c) = chrome {
                DragStrip { chrome: c, width: None }
            } else {
                span { style: "flex: 1;" }
            }
        }
        div {
            style: "position: relative; flex-shrink: 0; box-sizing: border-box; height: 44px; \
                    padding: 0 {20.0 + ins.right}px 0 20px; display: flex; align-items: center; gap: 10px;",
            if let Some(back) = on_back {
                button {
                    class: "lp",
                    "aria-label": "Back",
                    style: "width: 28px; height: 28px; margin-left: -6px; flex-shrink: 0; border: none; border-radius: 7px; \
                            background: transparent; display: flex; align-items: center; justify-content: center;",
                    onclick: move |_| back.call(()),
                    ChevronLeft { size: 18, color: t.ink_2 }
                }
            }
            if let Some(lead) = lead {
                div { style: "flex: 1; min-width: 0;", "aria-label": "{title}", {lead} }
            } else {
                Breadcrumb { crumbs, current: title }
            }
            if let Some(trailing) = trailing { {trailing} }
            ToolbarView { toolbar, size: ToolSize::DESK }
        }
    }
}
