//! What sits over the workspace: floats, the drag layer and its drop
//! highlight, the menus, and a phone's one-pane body, switcher and sheet.

use architect_shell::layer;
use architect_shell::motion::{Spring, use_motion};
use dioxus::prelude::*;
use lucide_dioxus::{Layers, Minus, PanelRight, X};

use super::group::{NodeView, PaneSlot};
use super::{
    DRAG_SLOP, Drag, FLOAT_AT, Geometry, Host, Menu, Mount, Op, groups_of, narrow, point, use_host,
};
use crate::drop::{Bar, contains, drop_in, float_rect, snap_at};
use crate::model::{Area, Axis, Chrome, Node, PaneId, Rect, Side};
use crate::ops::Dest;

use super::slide::Landing;

/// A float in the main window: a title bar to drag (to an edge to snap
/// it), its group, a resize corner.
#[component]
pub(super) fn FloatView(index: usize, pane: PaneId) -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
    let workspace = host.workspace.read();
    let Some(float) = workspace
        .windows
        .first()
        .and_then(|w| w.floats.get(index))
        .cloned()
    else {
        return rsx! {};
    };
    let title = workspace
        .pane(&pane)
        .map(|p| host.title(p))
        .unwrap_or_default();
    drop(workspace);
    // A snapped float fills a share of the host, measured on mount and
    // at every drag.
    let area = (host.float_area)().unwrap_or(Rect::new(0.0, 0.0, 1200.0, 800.0));
    let b = float_rect(&float, Rect::new(0.0, 0.0, area.w, area.h));
    let grab = pane.clone();
    let size = pane.clone();
    let dock = pane.clone();
    let minimise = pane.clone();
    let node = Node::Group(float.group.clone());
    let radius = if float.snap.is_some() { "0" } else { "8px" };
    // Floats stack in the order they were made, the last on top.
    let float_z = layer::FLOAT.saturating_add(u8::try_from(index).unwrap_or(9).min(9));
    rsx! {
        div {
            style: "position:absolute; left:{b.x}px; top:{b.y}px; width:{b.w}px; height:{b.h}px; \
                    display:flex; flex-direction:column; background:{t.sheet}; \
                    border:1px solid {t.rule}; border-radius:{radius}; overflow:hidden; \
                    box-shadow:{t.shadow_float}; z-index:{float_z};",
            div {
                style: "flex:0 0 28px; display:flex; align-items:center; gap:4px; padding:0 4px 0 10px; \
                        background:{t.pill}; color:{t.ink_2}; cursor:grab; touch-action:none; \
                        user-select:none; font-size:12px;",
                onpointerdown: move |e| {
                    let (x, y) = point(&e);
                    let grab = grab.clone();
                    spawn(async move {
                        if let Some(r) = host.rect(Mount::Host).await {
                            host.float_area.set(Some(r));
                            host.drag.set(Some(Drag::Float {
                                pane: grab,
                                host: r,
                                dx: x - r.x - b.x,
                                dy: y - r.y - b.y,
                                x,
                                y,
                            }));
                        }
                    });
                },
                ondoubleclick: move |_| {
                    let maximised = matches!(float.snap, Some(crate::Snap::Maximised));
                    let snap = if maximised { None } else { Some(crate::Snap::Maximised) };
                    let _ = host.edit(|w| w.snap(&pane, snap));
                },
                div { style: "flex:1; min-width:0; overflow:hidden; white-space:nowrap;", "{title}" }
                FloatButton { label: "Minimise", icon: FloatIcon::Minimise, onpress: move |()| {
                    let _ = host.edit(|w| w.minimise(&minimise));
                } }
                FloatButton { label: "Dock it", icon: FloatIcon::Dock, onpress: move |()| {
                    host.run(Op::Move(dock.clone(), Dest::Dock(Side::Right)));
                } }
            }
            div { style: "flex:1 1 0; min-height:0; display:flex; position:relative;",
                NodeView { node }
            }
            div {
                style: "position:absolute; right:0; bottom:0; width:16px; height:16px; \
                        cursor:nwse-resize; touch-action:none;",
                onpointerdown: move |_| {
                    let size = size.clone();
                    spawn(async move {
                        if let Some(r) = host.rect(Mount::Host).await {
                            host.float_area.set(Some(r));
                            host.drag.set(Some(Drag::FloatSize { pane: size, host: r }));
                        }
                    });
                },
            }
        }
    }
}

#[component]
fn FloatButton(label: &'static str, icon: FloatIcon, onpress: EventHandler<()>) -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    rsx! {
        button {
            style: "flex:0 0 auto; width:24px; height:24px; border:none; border-radius:5px; \
                    background:transparent; display:flex; align-items:center; justify-content:center; cursor:pointer;",
            title: label,
            "aria-label": label,
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |_| onpress.call(()),
            match icon {
                FloatIcon::Minimise => rsx! { Minus { size: 14, color: t.ink_3.to_string() } },
                FloatIcon::Dock => rsx! { PanelRight { size: 14, color: t.ink_3.to_string() } },
            }
        }
    }
}

/// A float title bar's buttons.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FloatIcon {
    Minimise,
    Dock,
}

/// While anything drags: a layer over everything that shows where a pane
/// would land. The pointer itself is followed at the host's root (Blitz
/// sends a held pointer's moves to the element pressed; they bubble up).
#[component]
pub(super) fn DragLayer() -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let Some(drag) = (host.drag)() else {
        return rsx! {};
    };
    // A grip pressed is not yet a drag: nothing covers it.
    if matches!(drag, Drag::DockGrip { moved: false, .. }) {
        return rsx! {};
    }
    // Along its own bar the tab itself follows the pointer: no label.
    let on_own = (host.lifted)().is_some_and(|(_, own)| own);
    let label = match &drag {
        Drag::Pane {
            pane,
            x,
            y,
            geometry: Some(Geometry { host: r, .. }),
            ..
        } if !on_own => host
            .workspace
            .read()
            .pane(pane)
            .map(|p| (host.title(p), x - r.x + 12.0, y - r.y + 12.0)),
        _ => None,
    };
    let cursor = match drag {
        Drag::Pane { .. } | Drag::Float { .. } | Drag::DockGrip { moved: true, .. } => "grabbing",
        Drag::Split {
            axis: Axis::Row, ..
        }
        | Drag::Dock {
            side: Side::Left | Side::Right,
            ..
        } => "ew-resize",
        Drag::Split { .. } | Drag::Dock { .. } => "ns-resize",
        _ => "default",
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:{layer::DRAG}; touch-action:none; cursor:{cursor};",
            if let Some((title, x, y)) = label {
                div {
                    style: "position:absolute; left:{x}px; top:{y}px; padding:4px 10px; border-radius:6px; \
                            background:{t.pill}; border:1px solid {t.rule}; color:{t.ink}; font-size:12px; \
                            pointer-events:none;",
                    "{title}"
                }
            }
        }
    }
}

/// Where a drag would land, lit: a split's half, a group, a dock's band,
/// a float's snap. (A place in a tab bar shows as the gap the tabs make.)
fn highlight_of(drag: Option<&Drag>) -> Option<Rect> {
    match drag? {
        Drag::Pane {
            at: Some(d),
            geometry: Some(Geometry { host: r, .. }),
            ..
        } if !matches!(d.dest, Dest::Group { index: Some(_), .. }) => Some(Rect::new(
            d.highlight.x - r.x,
            d.highlight.y - r.y,
            d.highlight.w,
            d.highlight.h,
        )),
        Drag::Float { host: r, x, y, .. } => snap_at(*x, *y, *r).map(|snap| {
            let ghost = crate::Float {
                group: crate::Group::new(crate::GroupId(0)),
                rect: Rect::new(0.0, 0.0, 0.0, 0.0),
                snap: Some(snap),
                pin: None,
                minimised: false,
            };
            float_rect(&ghost, Rect::new(0.0, 0.0, r.w, r.h))
        }),
        _ => None,
    }
}

/// The drop's highlight: it fades in where a drag would land, follows the
/// pointer from place to place on a spring, and fades out when the drag
/// ends (over the split that grows in under it).
#[component]
pub(super) fn DropHighlight() -> Element {
    let host = use_host();
    let t = host.theme.read().clone();
    let x = use_motion(0.0, Spring::TRACK, 0.25);
    let y = use_motion(0.0, Spring::TRACK, 0.25);
    let w = use_motion(0.0, Spring::TRACK, 0.25);
    let h = use_motion(0.0, Spring::TRACK, 0.25);
    let shown = use_motion(0.0, Spring::TRACK, 0.005);
    let target = highlight_of((host.drag)().as_ref());
    use_effect(use_reactive!(|target| {
        let Some(r) = target else {
            shown.to(0.0);
            return;
        };
        // Coming in from nothing it starts where it is, else it travels.
        let fresh = shown.peek() < 0.05 && shown.target() < 0.5;
        for (m, v) in [(x, r.x), (y, r.y), (w, r.w), (h, r.h)] {
            if fresh {
                m.jump(f64::from(v));
            } else {
                m.to(f64::from(v));
            }
        }
        shown.to(1.0);
    }));
    let alpha = shown.value();
    if alpha < 0.01 {
        return rsx! {};
    }
    rsx! {
        div {
            style: "position:absolute; left:{x.value()}px; top:{y.value()}px; width:{w.value()}px; \
                    height:{h.value()}px; box-sizing:border-box; background:rgba(191,219,254,0.12); \
                    border:2px solid {t.ink_2}; border-radius:4px; pointer-events:none; \
                    opacity:{alpha}; z-index:{layer::DRAG};",
        }
    }
}

pub(super) fn on_move(mut host: Host, (x, y): (f32, f32)) {
    let Some(drag) = host.drag.peek().clone() else {
        return;
    };
    match drag {
        Drag::Pressed { pane, x: x0, y: y0 } => {
            if (x - x0).hypot(y - y0) > DRAG_SLOP {
                host.drag.set(Some(Drag::Pane {
                    pane: pane.clone(),
                    geometry: None,
                    at: None,
                    x0,
                    x,
                    y,
                }));
                spawn(async move { measure_for_drop(host, pane).await });
            }
        }
        Drag::Pane {
            pane,
            geometry: Some(g),
            x0,
            ..
        } => {
            let at = drop_in(x, y, g.host, &g.bars, &g.groups);
            super::slide::during(host, &pane, &g.rows, at.map(|d| d.dest), x - x0);
            host.drag.set(Some(Drag::Pane {
                pane,
                geometry: Some(g),
                at,
                x0,
                x,
                y,
            }));
        }
        Drag::Pane { .. } => {}
        // Heard here when the drag layer is over the grip (a dock's own
        // window); in the shell's ribbon the grip hears it itself.
        Drag::DockGrip { .. } => super::dock::grip_moved(host, (x, y)),
        Drag::Split {
            id,
            index,
            axis,
            origin,
            length,
            shares,
        } => {
            if let Some(next) = drag_shares(&shares, index, axis, origin, length, (x, y)) {
                let _ = host.edit(|w| w.set_shares(id, &next));
            }
        }
        Drag::Dock { side, host: r } => {
            let points = match side {
                Side::Left => x - r.x,
                Side::Right => r.right() - x,
                Side::Top => y - r.y,
                Side::Bottom => r.bottom() - y,
            };
            let limit = if side.axis() == Axis::Row { r.w } else { r.h } * 0.8;
            let _ = host.edit(|w| w.set_dock_size(side, points.min(limit)));
        }
        Drag::Float {
            pane,
            host: r,
            dx,
            dy,
            ..
        } => {
            host.edit(|w| {
                let shown = w
                    .locate(&pane)
                    .and_then(|l| match l.area {
                        Area::Float(i) => w.windows.get(l.window).and_then(|win| win.floats.get(i)),
                        _ => None,
                    })
                    .map(|f| float_rect(f, Rect::new(0.0, 0.0, r.w, r.h)));
                let Some(b) = shown else {
                    return;
                };
                let nx = (x - r.x - dx).clamp(0.0, (r.w - 48.0).max(0.0));
                let ny = (y - r.y - dy).clamp(0.0, (r.h - 28.0).max(0.0));
                let _ = w.set_float_rect(&pane, Rect::new(nx, ny, b.w, b.h));
            });
            host.drag.set(Some(Drag::Float {
                pane,
                host: r,
                dx,
                dy,
                x,
                y,
            }));
        }
        Drag::Strip {
            group,
            x: x0,
            from,
            per_point,
            most,
        } => {
            let to = (x - x0).mul_add(per_point, from).clamp(0.0, most);
            host.scrolls.write().insert(group, to);
            if let Some(m) = host.strips.peek().get(&group) {
                m.hold(f64::from(to));
            }
        }
        Drag::FloatSize { pane, host: r } => {
            host.edit(|w| {
                let rect = w
                    .locate(&pane)
                    .and_then(|l| match l.area {
                        Area::Float(i) => w.windows.get(l.window).and_then(|win| win.floats.get(i)),
                        _ => None,
                    })
                    .map(|f| f.rect);
                if let Some(b) = rect {
                    let wide = (x - r.x - b.x).max(160.0);
                    let high = (y - r.y - b.y).max(96.0);
                    let _ = w.set_float_rect(&pane, Rect::new(b.x, b.y, wide, high));
                }
            });
        }
    }
}

/// The shares after dragging the divider after child `index` to the
/// pointer: the two children beside it trade, the rest keep theirs.
fn drag_shares(
    shares: &[f32],
    index: usize,
    axis: Axis,
    origin: f32,
    length: f32,
    (x, y): (f32, f32),
) -> Option<Vec<f32>> {
    if length <= 0.0 {
        return None;
    }
    let total: f32 = shares.iter().sum();
    let before: f32 = shares.iter().take(index).sum();
    let a = *shares.get(index)?;
    let b = *shares.get(index.checked_add(1)?)?;
    let pair = a + b;
    let at = match axis {
        Axis::Row => x,
        Axis::Column => y,
    };
    let along = ((at - origin) / length).mul_add(total, -before);
    let least = pair * 0.08;
    let first = along.clamp(least, pair - least);
    let mut next = shares.to_vec();
    if let Some(s) = next.get_mut(index) {
        *s = first;
    }
    if let Some(s) = next.get_mut(index.checked_add(1)?) {
        *s = pair - first;
    }
    Some(next)
}

async fn measure_for_drop(mut host: Host, pane: PaneId) {
    let measured = host.rect(Mount::Host).await;
    // Floats sit over the docks and the center: the last drawn first.
    let mut ids = Vec::new();
    // Each group's tabs, for its bar.
    let mut tabs: Vec<(crate::GroupId, Vec<PaneId>)> = Vec::new();
    {
        let workspace = host.workspace.peek();
        let here = host.window_index(&workspace);
        if let Some(main) = here.and_then(|i| workspace.windows.get(i)) {
            for float in main.floats.iter().rev().filter(|f| !f.minimised) {
                ids.push(float.group.id);
            }
            // The left dock in the shell's sidebar shows only while the
            // sidebar does: folded, its groups are not on screen (their
            // last measures are stale) and take no drop.
            let shell_folded = *host.in_shell.peek()
                && try_consume_context::<Signal<architect_shell::Shell>>()
                    .is_some_and(|s| s.peek().folded());
            for (side, dock) in [
                (Side::Left, &main.docks.left),
                (Side::Right, &main.docks.right),
                (Side::Bottom, &main.docks.bottom),
            ] {
                if side == Side::Left && shell_folded {
                    continue;
                }
                if dock.shown == crate::Shown::Open {
                    if let Some(root) = &dock.root {
                        groups_of(root, &mut ids);
                    }
                }
            }
            if let Some(center) = &main.center {
                groups_of(center, &mut ids);
            }
        }
        for id in &ids {
            let named = workspace
                .group(*id)
                .filter(|g| g.chrome != crate::Chrome::Bare && !g.stacked);
            if let Some(g) = named {
                tabs.push((*id, g.tabs.clone()));
            }
        }
    }
    let mut groups = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(g) = host.rect(Mount::Group(id)).await {
            groups.push((id, g));
        }
    }
    let mut bars = Vec::with_capacity(tabs.len());
    let mut rows = Vec::with_capacity(tabs.len());
    for (group, panes) in tabs {
        let Some(rect) = host.rect(Mount::Bar(group)).await else {
            continue;
        };
        let mut named = Vec::with_capacity(panes.len());
        for p in panes {
            if let Some(r) = host.rect(Mount::Tab(p.clone())).await {
                // Where it stands in the layout, not where a shift draws it.
                let shift = host
                    .tab_shift
                    .peek()
                    .get(&p)
                    .map_or(0.0, |m| narrow(m.peek()));
                named.push((p, Rect::new(r.x - shift, r.y, r.w, r.h)));
            }
        }
        bars.push(Bar {
            group,
            rect,
            tabs: named.iter().map(|(_, r)| *r).collect(),
        });
        rows.push(super::slide::Row {
            group,
            bar: rect,
            tabs: named,
        });
    }
    // The host's own rectangle, else what its groups cover.
    let Some(r) = measured.or_else(|| cover(&groups)) else {
        host.drag.set(None);
        return;
    };
    let still = matches!(host.drag.peek().as_ref(), Some(Drag::Pane { pane: p, .. }) if *p == pane);
    if still {
        let (x0, x, y) = match host.drag.peek().as_ref() {
            Some(Drag::Pane { x0, x, y, .. }) => (*x0, *x, *y),
            _ => (0.0, 0.0, 0.0),
        };
        host.drag.set(Some(Drag::Pane {
            pane,
            geometry: Some(Geometry {
                host: r,
                groups,
                bars,
                rows,
            }),
            at: None,
            x0,
            x,
            y,
        }));
    }
}

/// The rectangle covering every group.
fn cover(groups: &[(crate::GroupId, Rect)]) -> Option<Rect> {
    let mut rects = groups.iter().map(|(_, r)| *r);
    let first = rects.next()?;
    Some(rects.fold(first, |a, b| {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        Rect::new(
            x,
            y,
            a.right().max(b.right()) - x,
            a.bottom().max(b.bottom()) - y,
        )
    }))
}

/// The platform saw the pointer let go at `(x, y)`, outside the window: a
/// tab being dragged ends there (in another window, or a new one).
pub(super) fn release_outside(mut host: Host, (x, y): (f32, f32)) {
    // A dock's grip let go outside: it pops out there, or docks back.
    let drag = host.drag.peek().clone();
    if let Some(Drag::DockGrip { side, .. }) = drag {
        host.drag.set(None);
        if let Some(op) = super::windows::dock_outside(host, side, (x, y)) {
            host.run(op);
        }
        return;
    }
    let Some(Drag::Pane {
        pane, geometry, x0, ..
    }) = drag
    else {
        return;
    };
    host.drag.set(Some(Drag::Pane {
        pane,
        geometry,
        at: None,
        x0,
        x,
        y,
    }));
    on_up(host);
}

pub(super) fn on_up(mut host: Host) {
    let drag = host.drag.peek().clone();
    if matches!(drag, Some(Drag::DockGrip { .. })) {
        super::dock::grip_up(host);
        return;
    }
    host.drag.set(None);
    // The tabs that made room settle into their places (`super::slide`).
    let land = |pane: &PaneId, g: &Geometry, x0: f32, x: f32, landing| {
        let left = g
            .rows
            .iter()
            .flat_map(|r| r.tabs.iter())
            .find(|(p, _)| p == pane)
            .map_or(x, |(_, r)| r.x);
        super::slide::landed(host, pane, &g.rows, landing, x - x0 + left);
    };
    match drag {
        Some(Drag::Pane {
            pane,
            at: Some(d),
            geometry,
            x0,
            x,
            ..
        }) => {
            if let Some(g) = &geometry {
                land(&pane, g, x0, x, Landing::To(Some(d.dest)));
            }
            // Where it lit is the size a split it makes grows in at.
            host.move_pane(&pane, d.dest, Some(d.highlight));
        }
        // Let go outside the window: the window under it, or a new one.
        Some(Drag::Pane {
            pane,
            at: None,
            geometry: Some(g),
            x0,
            x,
            y,
        }) if !contains(g.host, x, y) => {
            let from = host
                .workspace
                .peek()
                .locate(&pane)
                .and_then(|l| g.groups.iter().find(|(id, _)| *id == l.group).copied());
            let to = super::windows::drop_outside(host, &pane, (x, y), from);
            land(
                &pane,
                &g,
                x0,
                x,
                if to.is_some() {
                    Landing::To(None)
                } else {
                    Landing::Back
                },
            );
            if let Some(op) = to {
                host.run(op);
            }
        }
        Some(Drag::Pane { .. }) => super::slide::cancel(host),
        // A scrollbar let go moving: the strip coasts on.
        Some(Drag::Strip { group, most, .. }) => {
            if let Some(m) = host.strips.peek().get(&group) {
                m.coast(0.0, f64::from(most));
            }
        }
        Some(Drag::Float {
            pane,
            host: r,
            x,
            y,
            ..
        }) => {
            if let Some(snap) = snap_at(x, y, r) {
                let _ = host.edit(|w| w.snap(&pane, Some(snap)));
            }
        }
        _ => {}
    }
}

/// A menu's width, and how tall one may run before it opens upward
/// (points).
const MENU_W: f32 = 248.0;
const MENU_TALL: f32 = 360.0;
/// The menus' layer, over the floats and the drag layer.
const Z_MENU: u8 = layer::MENU;

/// The pane menu and the open-a-view menu.
#[component]
pub(super) fn MenuLayer() -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
    let Some(menu) = (host.menu)() else {
        return rsx! {};
    };
    let items = match menu {
        Menu::Pane(pane) => pane_items(host, &pane),
        Menu::Open => open_items(host),
        Menu::Tabs(group) => tab_items(host, group),
        Menu::Target { target, id, pane } => target_items(host, &target, &id, &pane),
    };
    // On a desk the menu opens where it was asked for (the ⋯, the
    // pointer), kept inside the window; on a phone, or from a key, in the
    // middle over a scrim.
    let area = (host.float_area)().unwrap_or(Rect::new(0.0, 0.0, 1200.0, 800.0));
    let anchored = (host.menu_at)().filter(|_| !(host.phone)());
    let (scrim, place) = anchored.map_or_else(
        || {
            (
                t.scrim.to_string(),
                "position:relative; margin:auto; max-width:90%; max-height:85%;".to_owned(),
            )
        },
        |(x, y)| {
            let left = (x - area.x).clamp(8.0, (area.w - MENU_W - 8.0).max(8.0));
            let top = (y - area.y + 6.0).clamp(8.0, (area.h - MENU_TALL).max(8.0));
            (
                "transparent".to_owned(),
                format!(
                    "position:absolute; left:{left}px; top:{top}px; max-height:{}px;",
                    area.h - top - 8.0
                ),
            )
        },
    );
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:{Z_MENU}; background:{scrim}; \
                    display:flex;",
            onclick: move |_| host.menu.set(None),
            div {
                style: "{place} width:{MENU_W}px; overflow-y:scroll; box-sizing:border-box; \
                        display:flex; flex-direction:column; padding:5px; border-radius:10px; \
                        background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float};",
                onclick: move |e| e.stop_propagation(),
                for (i, item) in items.into_iter().enumerate() {
                    MenuRow { key: "{i}", item }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum Item {
    Heading(String),
    Act(String, Op),
    /// Another's row: ticked when its command is a toggle that is on.
    Entry(String, Op, Option<bool>),
    /// A line that does nothing (a menu with no rows says so).
    Note(String),
}

#[component]
fn MenuRow(item: Item) -> Element {
    let mut host = use_host();
    let commands = architect_commands::try_use_commands();
    let t = host.theme.read().clone();
    let s = (host.sizes)();
    match item {
        Item::Heading(text) => rsx! {
            div { style: "padding:8px 10px 3px; color:{t.ink_2}; font-size:11px; font-weight:600;", "{text}" }
        },
        Item::Note(text) => rsx! {
            div { style: "padding:6px 12px; color:{t.ink_3}; font-size:12px;", "{text}" }
        },
        Item::Entry(label, op, checked) => {
            let row_font = if s.button > 40.0 { 15 } else { 13 };
            let tick = if checked == Some(true) { "✓" } else { "" };
            // A toggle's row keeps a column for its tick; another lines up
            // with the menu's own rows.
            let pad = if checked.is_some() { 6 } else { 12 };
            rsx! {
                button {
                    style: "display:flex; justify-content:flex-start; align-items:center; gap:6px; text-align:left; \
                            padding:0 12px 0 {pad}px; min-height:{s.button}px; border:none; border-radius:6px; cursor:pointer; \
                            background:transparent; color:{t.ink}; font-size:{row_font}px;",
                    role: if checked.is_some() { "menuitemcheckbox" } else { "menuitem" },
                    "aria-checked": checked.map(|on| on.to_string()),
                    onclick: move |_| {
                        host.menu.set(None);
                        host.switcher.set(false);
                        if let Op::Run { command, invocation } = &op
                            && let Some(c) = commands
                        {
                            let _ = c.run_invoked(command, None, invocation.clone());
                        }
                    },
                    if checked.is_some() {
                        span { style: "flex:0 0 auto; width:14px; text-align:center; color:{t.accent};", "{tick}" }
                    }
                    span { "{label}" }
                }
            }
        }
        Item::Act(label, op) => {
            // A mouse's rows (28pt, 13px) or a finger's (44pt, 15px).
            let row_font = if s.button > 40.0 { 15 } else { 13 };
            let danger = matches!(op, Op::Close(_) | Op::CloseOthers(_));
            let ink = if danger { &*t.danger } else { &*t.ink };
            rsx! {
                button {
                    style: "display:flex; justify-content:flex-start; text-align:left; padding:0 12px; \
                            min-height:{s.button}px; border:none; border-radius:6px; cursor:pointer; \
                            background:transparent; color:{ink}; font-size:{row_font}px;",
                    onclick: move |_| {
                        host.menu.set(None);
                        host.switcher.set(false);
                        host.run(op.clone());
                    },
                    "{label}"
                }
            }
        }
    }
}

fn act(label: &str, op: Op) -> Item {
    Item::Act(label.to_owned(), op)
}

fn pane_items(host: Host, pane: &PaneId) -> Vec<Item> {
    let workspace = host.workspace.peek();
    let (Some(p), Some(loc)) = (workspace.pane(pane), workspace.locate(pane)) else {
        return Vec::new();
    };
    let group = workspace.group(loc.group);
    let tabs = group.map_or(0, |g| g.tabs.len());
    let stacked = group.is_some_and(|g| g.stacked);
    let chrome = group.map_or(Chrome::Tabs, |g| g.chrome);
    let mut items = vec![Item::Heading(host.title(p))];
    if tabs > 1 {
        for (label, side) in [("Split right", Side::Right), ("Split down", Side::Bottom)] {
            items.push(act(
                label,
                Op::Move(
                    pane.clone(),
                    Dest::Edge {
                        group: loc.group,
                        side,
                    },
                ),
            ));
        }
    }
    for (label, side) in [
        ("Move to the left dock", Side::Left),
        ("Move to the right dock", Side::Right),
        ("Move to the bottom dock", Side::Bottom),
    ] {
        if loc.area != Area::Dock(side) {
            items.push(act(label, Op::Move(pane.clone(), Dest::Dock(side))));
        }
    }
    // A dock's tab: its whole dock to a window of its own, or back.
    if let Area::Dock(side) = loc.area {
        let name = match side {
            Side::Left => "sidebar",
            Side::Right => "right sidebar",
            _ => "bottom panel",
        };
        if loc.window > 0 {
            items.push(act(&format!("Dock the {name} back"), Op::DockBack(side)));
        } else if host.has_windows() && !*host.phone.peek() {
            items.push(act(
                &format!("Move the {name} to its own window"),
                Op::PopOutDock(side, None),
            ));
        }
    }
    let popped = loc.window > 0;
    if host.has_windows() && !*host.phone.peek() && (!popped || tabs > 1) {
        items.push(act(
            "Move to a new window",
            Op::Move(pane.clone(), Dest::Window(None)),
        ));
    }
    if popped {
        items.push(act("Move to the main window", Op::ToMain(pane.clone())));
    } else if loc.area == Area::Center {
        if !*host.phone.peek() {
            items.push(act("Float", Op::Move(pane.clone(), Dest::Float(FLOAT_AT))));
        }
    } else if let Some(center) = workspace
        .windows
        .first()
        .and_then(|w| w.center.as_ref())
        .and_then(Node::first_group)
    {
        items.push(act(
            "Move to the center",
            Op::Move(
                pane.clone(),
                Dest::Group {
                    group: center.id,
                    index: None,
                },
            ),
        ));
    }
    items.push(Item::Heading("Tabs".to_owned()));
    items.push(act(
        if p.pinned { "Unpin" } else { "Pin" },
        Op::Pin(pane.clone(), !p.pinned),
    ));
    items.push(act(
        if stacked {
            "Unstack tabs"
        } else {
            "Stack tabs"
        },
        Op::Stacked(loc.group, !stacked),
    ));
    for (label, next) in [
        ("Show names", Chrome::Tabs),
        ("Show icons", Chrome::Icons),
        ("Hide the tab bar", Chrome::Bare),
    ] {
        if chrome != next {
            items.push(act(label, Op::Chrome(loc.group, next)));
        }
    }
    if !workspace.closed.is_empty() {
        items.push(act("Reopen closed tab", Op::Reopen));
    }
    items.push(act("Open a view…", Op::OpenMenu));
    // What others add for this pane's view and its tab (an extension's
    // rows).
    let view = p.view.0.clone();
    for target in [format!("view:{view}"), "tab".to_owned()] {
        items.extend(entries(&target, &pane.0, pane, &view));
    }
    if tabs > 1 {
        items.push(act("Close others", Op::CloseOthers(pane.clone())));
    }
    items.push(act("Close", Op::Close(pane.clone())));
    items
}

/// What others add for `target` `id`, opened in `pane` showing `view`.
fn entries(target: &str, id: &str, pane: &PaneId, view: &str) -> Vec<Item> {
    let Some(entries) = try_consume_context::<super::ContextEntries>() else {
        return Vec::new();
    };
    (entries.0)(target, id)
        .into_iter()
        .map(|e| {
            Item::Entry(
                e.label,
                Op::Run {
                    command: e.command,
                    invocation: super::context_invocation(target, id, pane, view),
                },
                e.checked,
            )
        })
        .collect()
}

/// A target's menu, opened from inside `pane` (a file tree's row): its
/// name, and what others add for it.
fn target_items(host: Host, target: &str, id: &str, pane: &PaneId) -> Vec<Item> {
    let view = host
        .workspace
        .peek()
        .pane(pane)
        .map(|p| p.view.0.clone())
        .unwrap_or_default();
    let name = id.rsplit('/').next().unwrap_or(id).to_owned();
    let mut items = vec![Item::Heading(name)];
    let rows = entries(target, id, pane, &view);
    if rows.is_empty() {
        items.push(Item::Note("Nothing to do here".to_owned()));
    }
    items.extend(rows);
    items
}

fn tab_items(host: Host, group: crate::GroupId) -> Vec<Item> {
    let workspace = host.workspace.peek();
    let mut items = vec![Item::Heading("Tabs".to_owned())];
    if let Some(g) = workspace.group(group) {
        items.extend(
            g.tabs
                .iter()
                .filter_map(|id| workspace.pane(id))
                .map(|p| act(&host.title(p), Op::Show(p.id.clone()))),
        );
    }
    items
}

fn open_items(host: Host) -> Vec<Item> {
    let mut items = vec![Item::Heading("Open".to_owned())];
    items.extend(
        host.views
            .read()
            .iter()
            .map(|v| act(&v.title, Op::Open(v.kind.0.clone()))),
    );
    if !host.workspace.peek().closed.is_empty() {
        items.push(act("Reopen closed tab", Op::Reopen));
    }
    items
}

/// On a phone: the active pane alone, under a bar with its name and the
/// count of tabs (the switcher).
#[component]
pub(super) fn PhoneBody() -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
    let workspace = host.workspace.read();
    let pane = super::switcher::phone_pane(&workspace);
    let title = pane
        .as_ref()
        .and_then(|p| workspace.pane(p))
        .map(|p| host.title(p))
        .unwrap_or_default();
    let count = workspace.tab_count();
    drop(workspace);
    // The app may show the pane's title and the switcher in its own bar
    // (`phone_header: false`); else this one: the title on the page's
    // left edge, the switcher a labelled pill.
    let own_bar = (host.phone_header)();
    let header = super::switcher::PHONE_HEADER;
    let tabs = if count == 1 { "tab" } else { "tabs" };
    let grip = super::switcher::GRIP;
    rsx! {
        div {
            style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:column;",
            if own_bar {
                div {
                    style: "flex:0 0 {header}px; display:flex; align-items:center; gap:10px; padding:0 12px 0 20px; \
                            border-bottom:1px solid {t.rule}; background:{t.desk};",
                    div { style: "flex:1; min-width:0; overflow:hidden; white-space:nowrap; font-size:17px; font-weight:650;",
                        "{title}"
                    }
                    button {
                        style: "height:44px; display:flex; align-items:center; gap:7px; padding:0 14px; border-radius:22px; \
                                border:1px solid {t.rule}; background:{t.pill}; color:{t.ink}; font-size:14px; \
                                font-weight:600; cursor:pointer;",
                        "aria-label": "{count} {tabs}",
                        onclick: move |_| host.switcher.set(true),
                        Layers { size: 16, color: t.ink_2.to_string() }
                        "{count} {tabs}"
                    }
                }
            }
            div {
                style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative;",
                match pane {
                    Some(pane) => rsx! { PaneSlot { key: "{pane.0}", pane } },
                    None => rsx! { super::EmptyCenter {} },
                }
                // The grip along the pane's foot: a pull up from it opens
                // the switcher, the pane zooming out into its card as the
                // finger goes (the moves are the host's and the layer's).
                div {
                    "data-switcher": "grip",
                    "aria-label": "Show tabs",
                    style: "position:absolute; left:0; right:0; bottom:0; height:{grip}px; z-index:2; \
                            display:flex; align-items:center; justify-content:center; touch-action:none;",
                    onpointerdown: move |e| {
                        e.stop_propagation();
                        super::switcher::start_pull(host, super::point(&e).1);
                    },
                    div { style: "width:36px; height:4px; border-radius:2px; background:{t.ink_3}; opacity:0.6;" }
                }
            }
        }
    }
}

/// A phone's docked pane, opened over the one on show.
#[component]
pub(super) fn SheetLayer() -> Element {
    let mut host = use_host();
    let t = host.theme.read().clone();
    let Some(pane) = (host.sheet)() else {
        return rsx! {};
    };
    let Some(title) = host.workspace.read().pane(&pane).map(|p| host.title(p)) else {
        return rsx! {};
    };
    let sheet_top = layer::SHEET.saturating_add(1);
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; background:{t.scrim}; z-index:{layer::SHEET};",
            onclick: move |_| host.sheet.set(None),
        }
        div {
            style: "position:absolute; left:0; right:0; bottom:0; height:80%; z-index:{sheet_top}; display:flex; \
                    flex-direction:column; background:{t.sheet}; border-radius:16px 16px 0 0; overflow:hidden;",
            div {
                style: "flex:0 0 48px; display:flex; align-items:center; padding:0 6px 0 16px; \
                        border-bottom:1px solid {t.rule};",
                div { style: "flex:1; font-weight:600;", "{title}" }
                button {
                    style: "width:44px; height:44px; border:none; background:transparent; display:flex; \
                            align-items:center; justify-content:center; cursor:pointer;",
                    title: "Close",
                    "aria-label": "Close {title}",
                    onclick: move |_| host.sheet.set(None),
                    X { size: 18, color: t.ink_2.to_string() }
                }
            }
            div { style: "flex:1 1 0; min-height:0; display:flex; position:relative;",
                PaneSlot { key: "{pane.0}", pane }
            }
        }
    }
}
