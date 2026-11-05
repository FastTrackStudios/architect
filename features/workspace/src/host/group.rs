//! A tree of splits and tab groups: the dividers, the tab bars (names,
//! icons, or none), stacked tabs, and the pane on show.

use architect_shell::motion::{Motion, Spring};
use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;
use lucide_dioxus::{ArrowLeft, ArrowRight, ChevronDown, Ellipsis, Pin, Plus, X};

use super::{Drag, Host, LONG_PRESS_MS, Menu, Mount, Op, ThisPane, point, use_host};
use crate::kinds::ViewHost;
use crate::layout::{scroll_to, slides};
use crate::model::{Axis, Chrome, Group, Node, PaneId, Split};

/// How far a named tab gives way before the bar clips, points.
const TAB_MIN: f32 = 72.0;
/// How much wider a badged tab stays (its badges), points.
const BADGE_ROOM: f32 = 36.0;
/// How much of a badged tab's name always shows, points.
const NAME_LEAST: f32 = 20.0;
/// A split's child's box, filling its share.
const FILL: &str = "flex:1 1 0; min-width:0; min-height:0; display:flex;";
/// How wide a named tab grows, points (Obsidian's).
const TAB_MAX: f32 = 200.0;
/// How wide a stacked pane is, points (narrower where the strip is).
const STACKED_W: f32 = 420.0;
/// The narrowest a stacked pane gets, points.
const STACKED_MIN: f32 = 240.0;
/// A stacked pane's spine, points.
const SPINE: f32 = 32.0;
/// A stacked strip's scrollbar, points.
const STRIP_BAR: f32 = 8.0;
/// How far a wheel's notch scrolls a stacked strip, points.
const WHEEL_LINE: f32 = 40.0;

/// A tab (or a spine) pressed: it may become a drag; under a finger or a
/// pen, held still long enough, it opens the tab's menu (a right click's).
fn press(mut host: Host, pane: &PaneId, e: &PointerEvent) {
    // A right-press is its context menu's, not a drag's.
    if e.data().trigger_button() == Some(dioxus::html::input_data::MouseButton::Secondary) {
        return;
    }
    let (x, y) = point(e);
    let pressed = Drag::Pressed {
        pane: pane.clone(),
        x,
        y,
    };
    host.drag.set(Some(pressed.clone()));
    let touch = e.data().pointer_type() != "mouse" || host.sizes.peek().tabs > 40.0;
    if !touch {
        return;
    }
    let pane = pane.clone();
    spawn(async move {
        futures_timer::Delay::new(std::time::Duration::from_millis(LONG_PRESS_MS)).await;
        // Still held where it was pressed: not a drag, not let go.
        if host.drag.peek().as_ref() == Some(&pressed) {
            host.drag.set(None);
            host.open_menu(Menu::Pane(pane), (x, y));
        }
    });
}

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

/// Room either side of a bar's tabs for the current tab's curved corners.
const FLARE: f32 = 8.0;

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
    let t = host.theme.read().clone();
    let s = (host.sizes)();
    let id = split.id;
    let axis = split.axis;
    let dir = if axis == Axis::Row { "row" } else { "column" };
    let cursor = if axis == Axis::Row {
        "ew-resize"
    } else {
        "ns-resize"
    };
    let (hair_line, hair_grip) = hairline(axis, s.divider, &t.rule, cursor);
    let shares: Vec<f32> = split.children.iter().map(|c| c.share).collect();
    // A child group growing in (a split just made) or giving its room up
    // (closing) takes that share of its own.
    let growing = (host.growing)();
    // Where it is held while it grows: the drop's edge, else (closing) the
    // end away from the sibling that takes its room.
    let held = |index: usize, last: usize| -> Option<String> {
        let (r, side) = (*host.grow_size.peek())?;
        let side = side.unwrap_or(match (axis, index == last) {
            (Axis::Row, true) => crate::model::Side::Right,
            (Axis::Row, false) => crate::model::Side::Left,
            (Axis::Column, true) => crate::model::Side::Bottom,
            (Axis::Column, false) => crate::model::Side::Top,
        });
        Some(match side {
            crate::model::Side::Left => format!("left:0; top:0; bottom:0; width:{}px;", r.w),
            crate::model::Side::Right => format!("right:0; top:0; bottom:0; width:{}px;", r.w),
            crate::model::Side::Top => format!("top:0; left:0; right:0; height:{}px;", r.h),
            crate::model::Side::Bottom => format!("bottom:0; left:0; right:0; height:{}px;", r.h),
        })
    };
    let grown: Vec<f32> = split
        .children
        .iter()
        .map(|c| match (&c.node, growing) {
            (Node::Group(g), Some(id)) if g.id == id => c.share * super::narrow(host.grow.value()),
            _ => c.share,
        })
        .collect();
    // Flex shares that add up to less than one leave room unfilled: the
    // others take up what the growing one has not.
    let sum: f32 = grown.iter().sum();
    let grown: Vec<f32> = grown
        .iter()
        .map(|g| if sum > 0.0 { g / sum } else { *g })
        .collect();
    // Each divider names the panes before it, for a guide scenario's
    // `@split"<pane>"` (`crate::target_selector`).
    let befores: Vec<String> = split.children.iter().map(|c| pane_names(&c.node)).collect();
    let count = split.children.len();
    let last = count.saturating_sub(1);
    let ids: Vec<Option<crate::model::GroupId>> = split
        .children
        .iter()
        .map(|c| match &c.node {
            Node::Group(g) => Some(g.id),
            Node::Split(_) => None,
        })
        .collect();
    let boxes: Vec<String> = split
        .children
        .iter()
        .enumerate()
        .map(|(index, c)| match (&c.node, growing) {
            (Node::Group(g), Some(id)) if g.id == id => held(index, last).map_or_else(
                || FILL.to_owned(),
                |at| format!("position:absolute; {at} display:flex;"),
            ),
            _ => FILL.to_owned(),
        })
        .collect();
    rsx! {
        div {
            style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:{dir};",
            onmounted: move |e| host.mount(Mount::Split(id), e.data()),
            for (index, child) in split.children.into_iter().enumerate() {
                div {
                    key: "{index}",
                    onresize: {
                        let group = ids.get(index).copied().flatten();
                        move |e: Event<ResizeData>| {
                            if let (Some(g), Ok(size)) = (group, e.get_content_box_size()) {
                                let mut sizes = host.split_sizes;
                                let r = crate::model::Rect::new(0.0, 0.0, super::narrow(size.width), super::narrow(size.height));
                                sizes.write().insert(g, r);
                            }
                        }
                    },
                    style: "flex:{grown.get(index).copied().unwrap_or(child.share)} 1 0; min-width:0; min-height:0; display:flex; \
                            position:relative; overflow:hidden;",
                    // One box either way, so the tree is not drawn afresh
                    // when it has grown.
                    div {
                        style: boxes.get(index).cloned().unwrap_or_else(|| FILL.to_owned()),
                        NodeView { node: child.node, edges: edges.child(axis, index, count) }
                    }
                }
                if index < last {
                    div {
                        key: "d{index}",
                        style: "{hair_line}",
                        div {
                        style: "{hair_grip}",
                        "data-split-after": befores.get(index).cloned().unwrap_or_default(),
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
    let t = host.theme.read().clone();
    let Some(current) = group.current_tab().cloned() else {
        return rsx! {};
    };
    let id = group.id;
    let names = tab_names(&group);
    let header = host.pane_header(&current);
    rsx! {
        div {
            style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:column; \
                    background:{t.sheet};",
            "data-group": "{id.0}",
            "data-group-tabs": "{names}",
            onmounted: move |e| host.mount(Mount::Group(id), e.data()),
            if group.chrome != Chrome::Bare && !group.stacked {
                TabBar { group: group.clone(), current: current.clone(), edges }
            }
            div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative; overflow:hidden;",
                if group.stacked {
                    Stacked { group: group.clone(), current: current.clone() }
                } else {
                    div { style: "flex:1 1 0; width:100%; min-width:0; min-height:0; display:flex; flex-direction:column;",
                        if group.chrome == Chrome::Tabs && header {
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
    let t = host.theme.read().clone();
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
    // On the title row the bar is the window's title bar. Every bar has its
    // rule under every tab but the current one, which runs on into its pane
    // (Obsidian's, in every pane).
    let bar = if row.is_some() {
        &*t.title_bar
    } else {
        &*t.desk
    };
    let ruled = true;
    let current_index = group.tabs.iter().position(|p| *p == current);
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
    let names = tab_names(&group);
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
            // Above the bar's rule: positioned after it, so drawn after it
            // (no z-index: a raised strip is hit-tested ahead of its own
            // tabs, and a press on one missed it). The current tab covers
            // the rule and opens into its pane; the others show it.
            div { style: "flex:0 1 auto; min-width:0; display:flex; align-items:flex-end; overflow:hidden; \
                          padding:{lift}px {FLARE}px 0; gap:1px; position:relative;",
                "data-tab-bar": "{names}",
                onmounted: move |e| host.mount(Mount::Bar(group.id), e.data()),
                for (index, (pane, title, pinned, icon)) in tabs.into_iter().enumerate() {
                    Tab {
                        key: "{pane.0}",
                        on: Some(index) == current_index,
                        // A hairline after a tab unless it or the next is current.
                        rule: Some(index) != current_index && Some(index.saturating_add(1)) != current_index,
                        pane,
                        title,
                        pinned,
                        icon,
                        icons,
                        rule_bar: ruled,
                        bar: bar.to_owned(),
                    }
                }
            }
            if !icons {
                button {
                    style: "{button}",
                    title: "New tab",
                    "aria-label": "New tab",
                    onclick: move |_| host.show_open_menu(),
                    Plus { size: 16, color: t.ink_3.to_string() }
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
                    ChevronDown { size: 16, color: t.ink_3.to_string() }
                }
            } else {
                button {
                    style: "{button}",
                    title: "Tab menu",
                    "aria-label": "Tab menu",
                    onclick: move |e: MouseEvent| host.open_menu(Menu::Pane(menu_for.clone()), super::click_point(&e)),
                    Ellipsis { size: 16, color: t.ink_3.to_string() }
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
    let t = host.theme.read().clone();
    let s = (host.sizes)();
    let workspace = host.workspace.read();
    let Some(p) = workspace.pane(&pane) else {
        return rsx! {};
    };
    let title = host.title(p);
    let back = !p.history.back.is_empty();
    let forward = !p.history.forward.is_empty();
    let view = p.view.0.clone();
    drop(workspace);
    // What others put in it (an extension's toolbars), before its menu.
    let tools = try_consume_context::<super::HeaderTools>().and_then(|t| (t.0)(&view, &pane));
    let (nav_on, nav_off) = (t.ink_2.clone(), t.rule.clone());
    let nav = move |on: bool| {
        format!(
            "flex:0 0 auto; width:{w}px; height:{w}px; border:none; border-radius:6px; background:transparent; \
             color:{ink}; font-size:15px; display:flex; align-items:center; justify-content:center;",
            w = s.button,
            ink = if on { &nav_on } else { &nav_off },
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
                ArrowLeft { size: 16, color: if back { &*t.ink_2 } else { &*t.rule } }
            }
            button {
                style: nav(forward),
                title: "Forward",
                "aria-label": "Forward",
                onclick: move |_| {
                    let _ = host.edit(|w| w.go(&go_forward, true));
                },
                ArrowRight { size: 16, color: if forward { &*t.ink_2 } else { &*t.rule } }
            }
            div { style: "flex:1 1 0; min-width:0; overflow:hidden; white-space:nowrap; text-align:center;", "{title}" }
            if let Some(tools) = tools {
                {tools}
            }
            button {
                style: nav(true),
                title: "More options",
                "aria-label": "More options",
                onclick: move |e: MouseEvent| host.open_menu(Menu::Pane(menu.clone()), super::click_point(&e)),
                Ellipsis { size: 16, color: t.ink_2.to_string() }
            }
        }
    }
}

#[component]
fn Tab(
    pane: PaneId,
    title: String,
    on: bool,
    rule: bool,
    pinned: bool,
    icon: Option<architect_shell::IconFn>,
    icons: bool,
    /// The bar draws a rule along its foot.
    rule_bar: bool,
    /// The bar's colour, which the current tab's corners curve out into.
    bar: String,
) -> Element {
    let tab_id = pane.0.clone();
    let mut host = use_host();
    // Its shift along the bar while a tab is dragged (`super::slide`).
    let shift = use_hook(|| super::slide::tab_motion(host, &pane));
    let registered = pane.clone();
    use_drop(move || super::slide::forget(host, &registered, shift));
    use_effect(move || {
        if shift.peek().abs() > 0.01 && !shift.moving() {
            shift.to(0.0);
        }
    });
    let dx = shift.value();
    let lifted = (host.lifted)();
    let (dragged, off_bar) = match &lifted {
        Some((p, own)) if *p == pane => (true, !*own),
        _ => (false, false),
    };
    let lift = if dragged { 4 } else { u8::from(on) };
    let shown = u8::from(!off_bar);
    let t = host.theme.read().clone();
    let s = (host.sizes)();
    let ink = if on { &*t.ink } else { &*t.ink_3 };
    let fill = if on { &*t.sheet } else { "transparent" };
    let (pad_l, pad_r) = if icons { (8.0, 8.0) } else { (12.0, 6.0) };
    // What extensions put on it (`TabBadges`), after its name.
    let badges = try_use_context::<super::TabBadges>().and_then(|b| {
        let kind = host
            .workspace
            .peek()
            .pane(&pane)
            .map(|p| p.view.0.clone())?;
        (b.0)(&kind, &pane.0)
    });
    let badged = badges.is_some() && !icons;
    let least = if icons {
        0.0
    } else if badged {
        TAB_MIN + BADGE_ROOM
    } else {
        TAB_MIN
    };
    let name_least = if badged { NAME_LEAST } else { 0.0 };
    let (grow, most) = if icons { (0, 48.0) } else { (1, TAB_MAX) };
    // The current tab gives up its room last, so its name reads whole
    // while the others narrow.
    let shrink = if on { 0.2 } else { 1.0 };
    let hairline = if rule { &*t.rule } else { "transparent" };
    // The current tab is outlined on three sides and open into its pane
    // below, its corners curving out into the bar's rule (Obsidian's). The
    // curves are drawn in its own box (it clips: Blitz sizes a tab that
    // does not by its content): it reaches FLARE past each side over its
    // neighbours, its plate inset by as much.
    let flared = on && rule_bar;
    let (margin, inset) = if flared { (-FLARE, FLARE) } else { (0.0, 0.0) };
    let (pad_l, pad_r) = (pad_l + inset, pad_r + inset);
    let plain_fill = if flared { "transparent" } else { fill };
    let tab_h = s.tabs - 6.0;
    let press = pane.clone();
    let menu = pane.clone();
    let close = pane;
    let icon_size = if s.tabs > 40.0 { 18 } else { 14 };
    let fade_to = if on {
        t.sheet.clone()
    } else {
        bar_colour(&host)
    };
    let fade_from = transparent_of(&fade_to);
    let key_open = menu.clone();
    let mounted = menu.clone();
    let label = if pinned {
        format!("{title}, pinned")
    } else {
        title.clone()
    };
    rsx! {
        div {
            style: "flex:{grow} {shrink} {most}px; max-width:{most}px; min-width:{least}px; height:{tab_h}px; \
                    box-sizing:border-box; display:flex; align-items:center; gap:6px; \
                    padding:0 {pad_r}px 0 {pad_l}px; margin:0 {margin}px; overflow:hidden; \
                    border-radius:8px 8px 0 0; white-space:nowrap; touch-action:none; \
                    user-select:none; color:{ink}; background:{plain_fill}; \
                    position:relative; z-index:{lift}; cursor:pointer; opacity:{shown}; left:{dx}px;",
            title: "{title}",
            role: "tab",
            "data-tab": "{tab_id}",
            tabindex: "0",
            "aria-label": "{label}",
            "aria-selected": "{on}",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Enter) || e.key() == Key::Character(" ".into()) {
                    e.prevent_default();
                    let _ = host.edit(|w| w.activate(&key_open));
                }
            },
            onmounted: move |e| host.mount(Mount::Tab(mounted.clone()), e.data()),
            onpointerdown: move |e| {
                let _ = host.edit(|w| w.activate(&press));
                self::press(host, &press, &e);
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                host.drag.set(None);
                host.open_menu(Menu::Pane(menu.clone()), super::click_point(&e));
            },
            // The current tab's plate, outlined and open below, with its
            // curved corners either side; what it shows is drawn over it.
            if flared {
                div { style: "position:absolute; top:0; bottom:0; left:{inset}px; right:{inset}px; box-sizing:border-box; \
                              background:{fill}; border:1px solid {t.rule}; border-bottom:none; border-radius:8px 8px 0 0;",
                    architect_shell::TabFlares { fill: t.sheet.to_string(), bar, rule: t.rule.to_string() }
                }
            }
            // Obsidian's divider: a short line between two tabs that are not
            // the current one, half the tab's height.
            if rule {
                div { style: "position:absolute; right:0; top:25%; height:50%; width:1px; background:{hairline};" }
            }
            if let Some(icon) = icon {
                span { style: "position:relative; display:flex;",
                    {icon(architect_shell::Icon { size: icon_size, color: ink.into() })}
                }
            }
            if !icons || icon.is_none() {
                // A long name fades out at its end (Blitz has no ellipsis).
                span { "data-tab-name": "{tab_id}", style: "position:relative; flex:1 1 0; min-width:{name_least}px; overflow:hidden;",
                    "{title}"
                    span { style: "position:absolute; top:0; right:0; bottom:0; width:16px; \
                                   background:linear-gradient(to right, {fade_from}, {fade_to});" }
                }
                {badges}
            }
            if !icons {
                if pinned {
                    span { style: "position:relative; display:flex;",
                        Pin { size: 12, color: t.ink_3.to_string() }
                    }
                } else if on || s.tabs > 40.0 {
                    // Above the name: its overflowing text still takes
                    // presses past its clip (blitz-quirks.md).
                    button {
                        style: "flex:0 0 auto; width:{tab_h - 10.0}px; height:{tab_h - 10.0}px; \
                                border:none; border-radius:4px; background:transparent; display:flex; \
                                align-items:center; justify-content:center; cursor:pointer; \
                                position:relative; z-index:1;",
                        title: "Close {title}",
                        "aria-label": "Close {title}",
                        onpointerdown: move |e| e.stop_propagation(),
                        onclick: move |e| {
                            e.stop_propagation();
                            host.close_from_tab(close.clone());
                        },
                        X { size: 14, color: t.ink_3.to_string() }
                    }
                }
            }
        }
    }
}

/// A stacked group (Obsidian's stacked tabs): its panes side by side in
/// a strip, each behind a spine with its name. As the strip scrolls a
/// pane slides over the ones before it, which leave their spines piled at
/// the left; the panes still to come pile theirs at the right. Blitz has
/// no horizontal `position: sticky` and cannot scroll a box from code, so
/// the strip keeps its own scroll ([`slides`] places each pane from it):
/// a sideways wheel or trackpad, its scrollbar, or a press on a spine.
#[component]
fn Stacked(group: Group, current: PaneId) -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
    let workspace = host.workspace.read();
    let panes: Vec<(PaneId, String, Option<architect_shell::IconFn>)> = group
        .tabs
        .iter()
        .filter_map(|id| workspace.pane(id))
        .map(|p| (p.id.clone(), host.title(p), host.icon(p)))
        .collect();
    drop(workspace);
    let id = group.id;
    let mut view = use_signal(|| 0.0_f32);
    // The strip's scroll moves: a trackpad's every point at once, a wheel's
    // notch and a spine's press eased there, a flicked scrollbar coasting.
    let strip = use_hook(|| {
        let from = host.scrolls.peek().get(&id).copied().unwrap_or(0.0);
        let m = Motion::new(f64::from(from), Spring::SNAPPY, 0.25);
        let mut strips = host.strips;
        strips.write().insert(id, m);
        m
    });
    use_drop(move || {
        let mut strips = host.strips;
        if let Ok(mut all) = strips.try_write() {
            if all.get(&id) == Some(&strip) {
                all.remove(&id);
            }
        }
    });
    let shown = view();
    let count = panes.len();
    let width = STACKED_W.min(shown).max(STACKED_MIN);
    let total = width * super::count_f32(count);
    let most = (total - shown).max(0.0);
    let scroll = super::narrow(strip.value()).clamp(0.0, most);
    let placed = slides(count, width, SPINE, shown, scroll);
    // Kept for a strip drawn afresh (the group moved in the tree).
    let keep = move |to: f32| {
        let mut scrolls = host.scrolls;
        if scrolls.peek().get(&id) != Some(&to) {
            scrolls.write().insert(id, to);
        }
    };
    let set_scroll = move |to: f32| {
        let to = to.clamp(0.0, most);
        keep(to);
        strip.to(f64::from(to));
    };
    // How far each pane lies over the one before it, 0 to 1: its shadow
    // deepens as it slides over.
    let covers: Vec<f32> = placed
        .iter()
        .enumerate()
        .map(|(i, s)| {
            i.checked_sub(1)
                .and_then(|p| placed.get(p))
                .map_or(0.0, |prev| {
                    ((prev.x + width - s.x) / (width * 0.25)).clamp(0.0, 1.0)
                })
        })
        .collect();
    let thumb_w = if total > 0.0 {
        shown * shown / total
    } else {
        0.0
    };
    let thumb_x = if total > 0.0 {
        scroll * shown / total
    } else {
        0.0
    };
    rsx! {
        div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:column;",
            div {
                style: "flex:1 1 0; min-width:0; min-height:0; position:relative; overflow:hidden;",
                // Measured when drawn (Blitz may not report a first size),
                // and again as it resizes.
                onmounted: move |e| {
                    host.mount(Mount::Strip(id), e.data());
                    spawn(async move {
                        if let Some(r) = host.rect(Mount::Strip(id)).await {
                            if (*view.peek() - r.w).abs() > 0.5 {
                                view.set(r.w);
                            }
                        }
                    });
                },
                onresize: move |e| {
                    if let Ok(size) = e.get_content_box_size() {
                        let w = super::narrow(size.width);
                        if (*view.peek() - w).abs() > 0.5 {
                            view.set(w);
                        }
                    }
                },
                onwheel: move |e| {
                    let shift = e.modifiers().shift();
                    // Sideways, or a plain wheel turned with ⇧ held; signed
                    // as Blitz scrolls a box (a delta to the right brings
                    // the start into view), so a swipe moves it the way it
                    // moves everything else.
                    let along = |x: f64, y: f64| -super::narrow(if x == 0.0 && shift { y } else { x });
                    match e.data().delta() {
                        // A trackpad (and its coasting): point for point.
                        WheelDelta::Pixels(d) => {
                            let dx = along(d.x, d.y);
                            if dx != 0.0 {
                                let to = (super::narrow(strip.peek()) + dx).clamp(0.0, most);
                                keep(to);
                                strip.hold(f64::from(to));
                            }
                        }
                        // A wheel's notches: eased, from where it is headed.
                        WheelDelta::Lines(d) => {
                            let dx = along(d.x, d.y) * WHEEL_LINE;
                            if dx != 0.0 {
                                set_scroll(super::narrow(strip.target()) + dx);
                            }
                        }
                        WheelDelta::Pages(d) => {
                            let dx = along(d.x, d.y) * shown;
                            if dx != 0.0 {
                                set_scroll(super::narrow(strip.target()) + dx);
                            }
                        }
                    }
                },
                for (i, ((pane, title, icon), (slide, cover))) in panes.into_iter().zip(placed.iter().copied().zip(covers.iter().copied())).enumerate() {
                    div {
                        key: "{pane.0}",
                        style: "position:absolute; top:0; bottom:0; left:{slide.x}px; width:{width}px; \
                                z-index:{i}; display:flex; flex-direction:row; background:{t.sheet}; \
                                border-right:1px solid {t.rule}; box-shadow:{slide_shadow(cover)};",
                        div {
                            style: "flex:0 0 {SPINE}px; display:flex; flex-direction:column; align-items:center; \
                                    gap:8px; padding-top:8px; touch-action:none; user-select:none; cursor:pointer; \
                                    background:{spine_fill(pane == current, &t)}; \
                                    border-right:1px solid {t.rule}; color:{t.ink_3};",
                            title: "{title}",
                            "aria-label": "{title}",
                            "data-tab": "{pane.0}",
                            onpointerdown: {
                                let pane = pane.clone();
                                move |e: PointerEvent| {
                                    let _ = host.edit(|w| w.activate(&pane));
                                    press(host, &pane, &e);
                                }
                            },
                            // A press on a spine brings its pane into view.
                            onclick: move |_| set_scroll(scroll_to(i, width, SPINE)),
                            oncontextmenu: {
                                let pane = pane;
                                move |e: MouseEvent| {
                                    e.prevent_default();
                                    host.drag.set(None);
                                    host.open_menu(Menu::Pane(pane.clone()), super::click_point(&e));
                                }
                            },
                            if let Some(icon) = icon {
                                {icon(architect_shell::Icon { size: 14, color: t.ink_3.clone() })}
                            }
                            // The name down the spine, turned to read top to
                            // bottom as a book's does.
                            div { style: "position:relative; flex:1 1 0; width:100%; min-height:0; overflow:hidden;",
                                span {
                                    // Turned about its top left, the line lies left of
                                    // it: half a line right centres it on the spine
                                    // (a `translate` is in device pixels).
                                    style: "position:absolute; top:0; left:{SPINE / 2.0 + 7.0}px; transform-origin:0 0; \
                                            transform:rotate(90deg); white-space:nowrap; \
                                            font-size:12px; line-height:14px; color:{t.ink_2};",
                                    "{title}"
                                }
                            }
                        }
                        div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative; overflow:hidden;",
                            PaneSlot { pane: pane.clone() }
                        }
                    }
                }
            }
            if most > 0.0 {
                div { style: "flex:0 0 {STRIP_BAR}px; position:relative; background:{t.desk};",
                    div {
                        style: "position:absolute; top:1px; bottom:1px; left:{thumb_x}px; width:{thumb_w}px; \
                                border-radius:4px; background:{t.rule}; touch-action:none;",
                        onpointerdown: move |e| {
                            let (x, _) = point(&e);
                            host.drag.set(Some(Drag::Strip {
                                group: id,
                                x,
                                from: scroll,
                                per_point: total / shown.max(1.0),
                                most,
                            }));
                        },
                    }
                }
            }
        }
    }
}

/// A stacked pane lying over the one before it casts a shadow on it, the
/// deeper the further over it is (`cover`, 0 to 1).
fn slide_shadow(cover: f32) -> String {
    if cover <= 0.0 {
        "none".to_owned()
    } else {
        format!("-6px 0 12px rgba(0,0,0,{:.3})", 0.18 * cover)
    }
}

/// Over a group while arranging: its panes' names, grabbed to drag one,
/// right-clicked for the menu; and the switch between a tab bar and none.
#[component]
fn Grip(group: Group, current: PaneId) -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
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
            if on { &*t.active } else { &*t.sheet },
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
    let named = pane.0.clone();
    let host = use_host();
    let t = host.theme.read().clone();
    let this = use_context_provider(|| ThisPane(Signal::new(pane.clone())));
    if *this.0.peek() != pane {
        let mut current = this.0;
        current.set(pane.clone());
    }
    let kind = host.workspace.read().pane(&pane).map(|p| p.view.0.clone());
    // While a field in it is typed in, its view kind is a context the
    // phone's Typing Toolbar is in (a toolbar's `when = "view:<kind>"`).
    let typing =
        architect_shell::use_typing_scope(&format!("view:{}", kind.as_deref().unwrap_or_default()));
    // Its view, if the app has it, and what draws it, if that is here:
    // the built-in `dioxus` kind always is.
    let render = kind.as_ref().and_then(|k| {
        host.views
            .read()
            .resolve(k, ViewHost::Blitz)
            .map(|r| r.map(|v| v.render.clone()))
    });
    let focus = pane.clone();
    let body = match render {
        Some(Ok(render)) => render(pane),
        Some(Err(why)) => rsx! {
            div { style: "flex:1; display:flex; align-items:center; justify-content:center; \
                          text-align:center; padding:24px; color:{t.ink_3};",
                "{why}; the pane keeps its state until it comes back."
            }
        },
        None => {
            let name = kind.unwrap_or_default();
            rsx! {
                div { style: "flex:1; display:flex; align-items:center; justify-content:center; \
                              text-align:center; padding:24px; color:{t.ink_3};",
                    "No view called “{name}” in this app; the pane keeps its state until it comes back."
                }
            }
        }
    };
    rsx! {
        div {
            style: "flex:1 1 0; width:100%; min-width:0; min-height:0; display:flex; flex-direction:column; position:relative;",
            "data-pane": "{named}",
            onpointerdown: move |_| {
                let already = host.workspace.peek().active.as_ref() == Some(&focus);
                if !already {
                    let _ = host.edit(|w| w.activate(&focus));
                }
            },
            onfocusin: move |_| typing.focus(),
            onfocusout: move |_| typing.blur(),
            {body}
        }
    }
}

/// A group's panes, space-separated: what its element says it holds, for
/// a guide scenario's `@group"<pane>"` and `@tab-bar"<pane>"`.
fn tab_names(group: &Group) -> String {
    group
        .tabs
        .iter()
        .map(|p| p.0.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every pane under `node`, space-separated.
fn pane_names(node: &Node) -> String {
    let mut groups = Vec::new();
    node.groups(&mut groups);
    groups
        .iter()
        .map(|g| tab_names(g))
        .collect::<Vec<_>>()
        .join(" ")
}

fn spine_fill(on: bool, t: &architect_shell::ShellTheme) -> &str {
    if on { &t.sheet } else { &t.desk }
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
fn bar_colour(host: &super::Host) -> std::sync::Arc<str> {
    let t = host.theme.peek();
    if host.title_row.peek().is_some() {
        t.title_bar.clone()
    } else {
        t.desk.clone()
    }
}

/// `#rrggbb` fully transparent (`#rrggbb00`), for a fade into it.
fn transparent_of(hex: &str) -> String {
    if hex.len() == 7 && hex.starts_with('#') {
        format!("{hex}00")
    } else {
        "transparent".to_owned()
    }
}
