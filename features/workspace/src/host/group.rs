//! A tree of splits and tab groups: the dividers, the tab bars (names,
//! icons, or none), stacked tabs, and the pane on show.

use dioxus::prelude::*;
use lucide_dioxus::{ArrowLeft, ArrowRight, ChevronDown, Ellipsis, Pin, Plus, X};

use super::{Drag, Menu, Mount, Op, ThisPane, point, use_host};
use crate::model::{Axis, Chrome, Group, Node, PaneId, Split};

/// How far a named tab gives way before the bar clips, points.
const TAB_MIN: f32 = 72.0;
/// How wide a named tab grows, points (Obsidian's).
const TAB_MAX: f32 = 200.0;
/// How wide a stacked pane is, points.
const STACKED_W: f32 = 420.0;
/// A stacked pane's spine, points.
const SPINE: f32 = 32.0;

/// Which of the window's edges a tree touches: a group on the top edge
/// draws its tab bar as the title row (Obsidian's), kept clear of the
/// window's buttons at the left end and the app's at the right.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Edges {
    pub top: bool,
    pub left: bool,
    pub right: bool,
    /// How much of the left clearance a ribbon or rail already covers.
    pub covered: f32,
}

impl Edges {
    /// What child `index` of `count` along `axis` touches.
    const fn child(self, axis: Axis, index: usize, count: usize) -> Self {
        let first = index == 0;
        let last = index.saturating_add(1) == count;
        match axis {
            Axis::Row => Self {
                top: self.top,
                left: self.left && first,
                right: self.right && last,
                covered: self.covered,
            },
            Axis::Column => Self {
                top: self.top && first,
                left: self.left,
                right: self.right,
                covered: self.covered,
            },
        }
    }
}

/// A split or a group.
#[component]
pub(super) fn NodeView(node: Node, #[props(default)] edges: Edges) -> Element {
    match node {
        Node::Group(group) => rsx! { GroupView { key: "{group.id.0}", group, edges } },
        Node::Split(split) => rsx! { SplitView { key: "s{split.id.0}", split, edges } },
    }
}

#[component]
fn SplitView(split: Split, edges: Edges) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let id = split.id;
    let axis = split.axis;
    let dir = if axis == Axis::Row { "row" } else { "column" };
    let cursor = if axis == Axis::Row {
        "ew-resize"
    } else {
        "ns-resize"
    };
    let (hair_line, hair_grip) = hairline(axis, s.divider, t.rule, cursor);
    let shares: Vec<f32> = split.children.iter().map(|c| c.share).collect();
    let count = split.children.len();
    let last = count.saturating_sub(1);
    rsx! {
        div {
            style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:{dir};",
            onmounted: move |e| host.mount(Mount::Split(id), e.data()),
            for (index, child) in split.children.into_iter().enumerate() {
                div {
                    key: "{index}",
                    style: "flex:{child.share} 1 0; min-width:0; min-height:0; display:flex; \
                            position:relative; overflow:hidden;",
                    NodeView { node: child.node, edges: edges.child(axis, index, count) }
                }
                if index < last {
                    div {
                        key: "d{index}",
                        style: "{hair_line}",
                        div {
                        style: "{hair_grip}",
                        onpointerdown: {
                            let shares = shares.clone();
                            move |_| {
                                let shares = shares.clone();
                                spawn(async move {
                                    if let Some(r) = host.rect(Mount::Split(id)).await {
                                        let (origin, length) = match axis {
                                            Axis::Row => (r.x, r.w),
                                            Axis::Column => (r.y, r.h),
                                        };
                                        host.drag.set(Some(Drag::Split { id, index, axis, origin, length, shares }));
                                    }
                                });
                            }
                        },
                        ondoubleclick: move |_| {
                            let _ = host.edit(|w| w.even(id));
                        },
                        }
                    }
                }
            }
        }
    }
}

/// A group: its tab bar (unless bare) over its pane, or its panes side by
/// side when stacked.
#[component]
fn GroupView(group: Group, edges: Edges) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let Some(current) = group.current_tab().cloned() else {
        return rsx! {};
    };
    let id = group.id;
    rsx! {
        div {
            style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:column; \
                    background:{t.sheet};",
            onmounted: move |e| host.mount(Mount::Group(id), e.data()),
            if group.chrome != Chrome::Bare && !group.stacked {
                TabBar { group: group.clone(), current: current.clone(), edges }
            }
            div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative; overflow:hidden;",
                if group.stacked {
                    Stacked { group: group.clone(), current: current.clone() }
                } else {
                    div { style: "flex:1 1 0; width:100%; min-width:0; min-height:0; display:flex; flex-direction:column;",
                        if group.chrome == Chrome::Tabs {
                            PaneHeader { key: "h{current.0}", pane: current.clone() }
                        }
                        PaneSlot { key: "{current.0}", pane: current.clone() }
                    }
                }
                if host.arranging() {
                    Grip { group, current }
                }
            }
        }
    }
}

/// A group's tabs, Obsidian's way: rounded at the top, the current one the
/// colour of its pane, the rest on the bar with a hairline between them,
/// then a `+` and the group's menu. On the window's top edge the bar is
/// the title row: as tall, clear of the window's buttons and the app's,
/// and a press on its empty part moves the window.
#[component]
fn TabBar(group: Group, current: PaneId, edges: Edges) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let icons = group.chrome == Chrome::Icons;
    let row = (host.title_row)().filter(|_| edges.top);
    let height = row.as_ref().map_or(s.tabs, |r| r.height.max(s.tabs));
    let pad_left = row
        .as_ref()
        .filter(|_| edges.left)
        .map_or(4.0, |r| (r.left - edges.covered).max(4.0));
    let pad_right = row
        .as_ref()
        .filter(|_| edges.right)
        .map_or(0.0, |r| r.right);
    let on_drag = row.as_ref().and_then(|r| r.on_drag);
    let on_zoom = row.as_ref().and_then(|r| r.on_zoom);
    let lift = (height - s.tabs + 6.0).max(6.0);
    // On the title row the bar is the window's title bar, its rule under
    // every tab but the current one (which runs on into its pane).
    let bar = if row.is_some() { t.title_bar } else { t.desk };
    let ruled = row.is_some();
    let current_index = group.tabs.iter().position(|p| *p == current);
    // The group the keyboard acts on: its current tab carries the accent.
    let focused = host.workspace.read().active_group() == Some(group.id);
    let many = group.tabs.len() >= 3;
    let workspace = host.workspace.read();
    let tabs: Vec<(PaneId, String, bool, Option<architect_shell::IconFn>)> = group
        .tabs
        .iter()
        .filter_map(|id| workspace.pane(id))
        .map(|p| (p.id.clone(), host.title(p), p.pinned, host.icon(p)))
        .collect();
    drop(workspace);
    let menu_for = current;
    let button = format!(
        "flex:0 0 auto; width:{w}px; height:{w}px; align-self:center; border:none; border-radius:6px; \
         background:transparent; color:{ink}; font-size:16px; display:flex; align-items:center; \
         justify-content:center;",
        w = s.button.min(height - 4.0),
        ink = t.ink_3,
    );
    rsx! {
        div {
            style: "flex:0 0 {height}px; height:{height}px; box-sizing:border-box; display:flex; \
                    align-items:stretch; padding:0 {pad_right}px 0 {pad_left}px; overflow:hidden; \
                    min-width:0; background:{bar}; position:relative;",
            // The bar's rule, under every tab but the current one (drawn over it).
            if ruled {
                div { style: "position:absolute; left:0; right:0; bottom:0; height:1px; background:{t.rule};" }
            }
            div { style: "flex:0 1 auto; min-width:0; display:flex; align-items:flex-end; overflow:hidden; \
                          padding-top:{lift}px; gap:1px;",
                for (index, (pane, title, pinned, icon)) in tabs.into_iter().enumerate() {
                    Tab {
                        key: "{pane.0}",
                        on: Some(index) == current_index,
                        focused,
                        // A hairline after a tab unless it or the next is current.
                        rule: Some(index) != current_index && Some(index.saturating_add(1)) != current_index,
                        pane,
                        title,
                        pinned,
                        icon,
                        icons,
                    }
                }
            }
            if !icons {
                button {
                    style: "{button}",
                    title: "New tab",
                    "aria-label": "New tab",
                    onclick: move |_| host.show_open_menu(),
                    Plus { size: 16, color: t.ink_3 }
                }
            }
            // The empty part of the bar: on the title row, the window's.
            div {
                style: "flex:1 1 0; min-width:8px;",
                onpointerdown: move |_| {
                    if let Some(drag) = on_drag {
                        drag.call(());
                    }
                },
                ondoubleclick: move |_| {
                    if let Some(zoom) = on_zoom {
                        zoom.call(());
                    }
                },
            }
            if !icons && many {
                button {
                    style: "{button}",
                    title: "Every tab",
                    "aria-label": "Every tab",
                    onclick: move |e: MouseEvent| host.open_menu(Menu::Tabs(group.id), super::click_point(&e)),
                    ChevronDown { size: 16, color: t.ink_3 }
                }
            } else {
                button {
                    style: "{button}",
                    title: "Tab menu",
                    "aria-label": "Tab menu",
                    onclick: move |e: MouseEvent| host.open_menu(Menu::Pane(menu_for.clone()), super::click_point(&e)),
                    Ellipsis { size: 16, color: t.ink_3 }
                }
            }
        }
    }
}

/// Over a named tab's pane: back and forward through what it showed, its
/// title, and its menu (Obsidian's view header).
#[component]
fn PaneHeader(pane: PaneId) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let workspace = host.workspace.read();
    let Some(p) = workspace.pane(&pane) else {
        return rsx! {};
    };
    let title = host.title(p);
    let back = !p.history.back.is_empty();
    let forward = !p.history.forward.is_empty();
    drop(workspace);
    let nav = move |on: bool| {
        format!(
            "flex:0 0 auto; width:{w}px; height:{w}px; border:none; border-radius:6px; background:transparent; \
             color:{ink}; font-size:15px; display:flex; align-items:center; justify-content:center;",
            w = s.button,
            ink = if on { t.ink_2 } else { t.rule },
        )
    };
    let (go_back, go_forward, menu) = (pane.clone(), pane.clone(), pane);
    rsx! {
        div {
            style: "flex:0 0 auto; height:{s.button + 12.0}px; display:flex; align-items:center; gap:2px; \
                    padding:0 8px; background:{t.sheet}; color:{t.ink_2}; font-size:13px;",
            button {
                style: nav(back),
                title: "Back",
                "aria-label": "Back",
                onclick: move |_| {
                    let _ = host.edit(|w| w.go(&go_back, false));
                },
                ArrowLeft { size: 16, color: if back { t.ink_2 } else { t.rule } }
            }
            button {
                style: nav(forward),
                title: "Forward",
                "aria-label": "Forward",
                onclick: move |_| {
                    let _ = host.edit(|w| w.go(&go_forward, true));
                },
                ArrowRight { size: 16, color: if forward { t.ink_2 } else { t.rule } }
            }
            div { style: "flex:1 1 0; min-width:0; overflow:hidden; white-space:nowrap; text-align:center;", "{title}" }
            button {
                style: nav(true),
                title: "More options",
                "aria-label": "More options",
                onclick: move |e: MouseEvent| host.open_menu(Menu::Pane(menu.clone()), super::click_point(&e)),
                Ellipsis { size: 16, color: t.ink_2 }
            }
        }
    }
}

#[component]
fn Tab(
    pane: PaneId,
    title: String,
    on: bool,
    focused: bool,
    rule: bool,
    pinned: bool,
    icon: Option<architect_shell::IconFn>,
    icons: bool,
) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let ink = if on { t.ink } else { t.ink_3 };
    let fill = if on { t.sheet } else { "transparent" };
    let pad = if icons { "0 8px" } else { "0 6px 0 12px" };
    let least = if icons { 0.0 } else { TAB_MIN };
    let (grow, most) = if icons { (0, 48.0) } else { (1, TAB_MAX) };
    let hairline = if rule { t.rule } else { "transparent" };
    let tab_h = s.tabs - 6.0;
    let press = pane.clone();
    let menu = pane.clone();
    let close = pane;
    let icon_size = if s.tabs > 40.0 { 18 } else { 14 };
    let fade_to = if on { t.sheet } else { bar_colour(&host) };
    let fade_from = transparent_of(fade_to);
    let key_open = menu.clone();
    let label = if pinned {
        format!("{title}, pinned")
    } else {
        title.clone()
    };
    rsx! {
        div {
            style: "flex:{grow} 1 {most}px; max-width:{most}px; min-width:{least}px; height:{tab_h}px; \
                    box-sizing:border-box; display:flex; align-items:center; gap:6px; padding:{pad}; \
                    border-radius:8px 8px 0 0; white-space:nowrap; overflow:hidden; touch-action:none; \
                    user-select:none; color:{ink}; background:{fill}; border-right:1px solid {hairline}; \
                    position:relative; z-index:{u8::from(on)}; cursor:pointer;",
            title: "{title}",
            role: "tab",
            tabindex: "0",
            "aria-label": "{label}",
            "aria-selected": "{on}",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) {
                    e.prevent_default();
                    let _ = host.edit(|w| w.activate(&key_open));
                }
            },
            onpointerdown: move |e| {
                let (x, y) = point(&e);
                let _ = host.edit(|w| w.activate(&press));
                host.drag.set(Some(Drag::Pressed { pane: press.clone(), x, y }));
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                host.drag.set(None);
                host.open_menu(Menu::Pane(menu.clone()), super::click_point(&e));
            },
            // The focused group's current tab: the accent along its top.
            if on && focused {
                div { style: "position:absolute; left:8px; right:8px; top:0; height:2px; border-radius:0 0 2px 2px; \
                              background:{t.accent};" }
            }
            if let Some(icon) = icon {
                {icon(architect_shell::Icon { size: icon_size, color: ink })}
            }
            if !icons || icon.is_none() {
                // A long name fades out at its end (Blitz has no ellipsis).
                span { style: "position:relative; flex:1 1 0; min-width:0; overflow:hidden;",
                    "{title}"
                    span { style: "position:absolute; top:0; right:0; bottom:0; width:16px; \
                                   background:linear-gradient(to right, {fade_from}, {fade_to});" }
                }
            }
            if !icons {
                if pinned {
                    Pin { size: 12, color: t.ink_3 }
                } else if on || s.tabs > 40.0 {
                    button {
                        style: "flex:0 0 auto; width:{tab_h - 10.0}px; height:{tab_h - 10.0}px; \
                                border:none; border-radius:4px; background:transparent; display:flex; \
                                align-items:center; justify-content:center; cursor:pointer;",
                        title: "Close {title}",
                        "aria-label": "Close {title}",
                        onpointerdown: move |e| e.stop_propagation(),
                        onclick: move |e| {
                            e.stop_propagation();
                            host.run(Op::Close(close.clone()));
                        },
                        X { size: 14, color: t.ink_3 }
                    }
                }
            }
        }
    }
}

/// A stacked group: every pane side by side, each behind a spine with its
/// name, the strip scrolled sideways (Obsidian's stacked tabs).
#[component]
fn Stacked(group: Group, current: PaneId) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let workspace = host.workspace.read();
    let panes: Vec<(PaneId, String, Option<architect_shell::IconFn>)> = group
        .tabs
        .iter()
        .filter_map(|id| workspace.pane(id))
        .map(|p| (p.id.clone(), host.title(p), host.icon(p)))
        .collect();
    drop(workspace);
    rsx! {
        div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:row; overflow-x:scroll; overflow-y:hidden;",
            for (pane, title, icon) in panes {
                div {
                    key: "{pane.0}",
                    style: "flex:0 0 {STACKED_W}px; width:{STACKED_W}px; min-height:0; display:flex; \
                            flex-direction:row; border-right:1px solid {t.rule};",
                    div {
                        style: "flex:0 0 {SPINE}px; display:flex; flex-direction:column; align-items:center; \
                                gap:8px; padding-top:8px; touch-action:none; user-select:none; \
                                background:{spine_fill(pane == current, t)}; \
                                border-right:1px solid {t.rule}; color:{t.ink_3};",
                        title: "{title}",
                        "aria-label": "{title}",
                        onpointerdown: {
                            let pane = pane.clone();
                            move |e: PointerEvent| {
                                let (x, y) = point(&e);
                                let _ = host.edit(|w| w.activate(&pane));
                                host.drag.set(Some(Drag::Pressed { pane: pane.clone(), x, y }));
                            }
                        },
                        oncontextmenu: {
                            let pane = pane;
                            move |e: MouseEvent| {
                                e.prevent_default();
                                host.drag.set(None);
                                host.open_menu(Menu::Pane(pane.clone()), super::click_point(&e));
                            }
                        },
                        if let Some(icon) = icon {
                            {icon(architect_shell::Icon { size: 14, color: t.ink_3 })}
                        }
                        for ch in title.chars().take(14) {
                            span { style: "font-size:11px; line-height:12px;", "{ch}" }
                        }
                    }
                    div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative; overflow:hidden;",
                        PaneSlot { pane: pane.clone() }
                    }
                }
            }
        }
    }
}

/// Over a group while arranging: its panes' names, grabbed to drag one,
/// right-clicked for the menu; and the switch between a tab bar and none.
#[component]
fn Grip(group: Group, current: PaneId) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let workspace = host.workspace.read();
    let panes: Vec<(PaneId, String)> = group
        .tabs
        .iter()
        .filter_map(|id| workspace.pane(id))
        .map(|p| (p.id.clone(), host.title(p)))
        .collect();
    drop(workspace);
    let id = group.id;
    let chrome = group.chrome;
    let menu = current.clone();
    let grab = current.clone();
    let chip = move |on: bool| {
        format!(
            "min-height:{}px; display:flex; align-items:center; padding:0 12px; border-radius:8px; \
             white-space:nowrap; font-weight:600; color:{}; background:{}; border:1px solid {};",
            s.button,
            t.ink,
            if on { t.active } else { t.sheet },
            t.rule,
        )
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:5; display:flex; \
                    align-items:center; justify-content:center; background:rgba(191,219,254,0.10); \
                    border:2px solid {t.ink_3}; cursor:grab; touch-action:none; user-select:none;",
            onpointerdown: move |e| {
                let (x, y) = point(&e);
                host.drag.set(Some(Drag::Pressed { pane: grab.clone(), x, y }));
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                host.drag.set(None);
                host.open_menu(Menu::Pane(menu.clone()), super::click_point(&e));
            },
            div {
                style: "display:flex; flex-wrap:wrap; gap:6px; padding:8px; max-width:90%; \
                        justify-content:center; border-radius:10px; background:{t.pill};",
                for (pane, title) in panes {
                    div {
                        key: "{pane.0}",
                        style: chip(pane == current),
                        onpointerdown: {
                            let pane = pane;
                            move |e: PointerEvent| {
                                e.stop_propagation();
                                let (x, y) = point(&e);
                                let _ = host.edit(|w| w.activate(&pane));
                                host.drag.set(Some(Drag::Pressed { pane: pane.clone(), x, y }));
                            }
                        },
                        "{title}"
                    }
                }
                div {
                    style: chip(false),
                    onpointerdown: move |e| {
                        e.stop_propagation();
                        let next = if chrome == Chrome::Bare { Chrome::Tabs } else { Chrome::Bare };
                        host.run(Op::Chrome(id, next));
                    },
                    if chrome == Chrome::Bare { "Show tabs" } else { "Hide tabs" }
                }
            }
        }
    }
}

/// One pane, drawn by its view; a press in it makes it the active pane.
#[component]
pub(super) fn PaneSlot(pane: PaneId) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    use_context_provider(|| ThisPane(pane.clone()));
    let kind = host.workspace.read().pane(&pane).map(|p| p.view.0.clone());
    let render = kind
        .as_ref()
        .and_then(|k| host.views.read().get(k).map(|v| v.render.clone()));
    let focus = pane.clone();
    let body = render.map_or_else(
        || {
            let name = kind.unwrap_or_default();
            rsx! {
                div { style: "flex:1; display:flex; align-items:center; justify-content:center; \
                              text-align:center; padding:24px; color:{t.ink_3};",
                    "No view called “{name}” in this app; the pane keeps its state until it comes back."
                }
            }
        },
        |render| render(pane),
    );
    rsx! {
        div {
            style: "flex:1 1 0; width:100%; min-width:0; min-height:0; display:flex; flex-direction:column; position:relative;",
            onpointerdown: move |_| {
                let already = host.workspace.peek().active.as_ref() == Some(&focus);
                if !already {
                    let _ = host.edit(|w| w.activate(&focus));
                }
            },
            {body}
        }
    }
}

const fn spine_fill(on: bool, t: architect_shell::ShellTheme) -> &'static str {
    if on { t.sheet } else { t.desk }
}

/// A divider, Obsidian's: a one-point line in the layout, and over it an
/// invisible handle `grip` points across to grab (the panes either side
/// keep their colour right up to the line).
pub(super) fn hairline(axis: Axis, grip: f32, rule: &str, cursor: &str) -> (String, String) {
    let half = grip / 2.0;
    match axis {
        Axis::Row => (
            format!("flex:0 0 1px; width:1px; position:relative; z-index:6; background:{rule};"),
            format!(
                "position:absolute; top:0; bottom:0; left:{l}px; width:{grip}px; cursor:{cursor}; touch-action:none;",
                l = 0.5 - half
            ),
        ),
        Axis::Column => (
            format!("flex:0 0 1px; height:1px; position:relative; z-index:6; background:{rule};"),
            format!(
                "position:absolute; left:0; right:0; top:{t}px; height:{grip}px; cursor:{cursor}; touch-action:none;",
                t = 0.5 - half
            ),
        ),
    }
}

/// What an inactive tab sits on: the title bar's colour on the title row,
/// the desk's elsewhere.
fn bar_colour(host: &super::Host) -> &'static str {
    let t = *host.theme.peek();
    if host.title_row.peek().is_some() {
        t.title_bar
    } else {
        t.desk
    }
}

/// `#rrggbb` fully transparent (`#rrggbb00`), for a fade into it.
fn transparent_of(hex: &'static str) -> String {
    if hex.len() == 7 && hex.starts_with('#') {
        format!("{hex}00")
    } else {
        "transparent".to_owned()
    }
}
