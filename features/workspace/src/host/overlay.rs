//! What sits over the workspace: floats, the drag layer and its drop
//! highlight, the menus, and a phone's one-pane body, switcher and sheet.

use dioxus::prelude::*;

use super::group::{NodeView, PaneSlot};
use super::{DRAG_SLOP, Drag, FLOAT_AT, Host, Menu, Mount, Op, groups_of, point, use_host};
use crate::drop::{drop_at, float_rect, snap_at};
use crate::model::{Area, Axis, Chrome, Node, PaneId, Rect, Side};
use crate::ops::Dest;

/// A float in the main window: a title bar to drag (to an edge to snap
/// it), its group, a resize corner.
#[component]
pub(super) fn FloatView(index: usize, pane: PaneId) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
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
    rsx! {
        div {
            style: "position:absolute; left:{b.x}px; top:{b.y}px; width:{b.w}px; height:{b.h}px; \
                    display:flex; flex-direction:column; background:{t.sheet}; \
                    border:1px solid {t.rule}; border-radius:{radius}; overflow:hidden; \
                    box-shadow:0 12px 32px rgba(0,0,0,0.5); z-index:10;",
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
                FloatButton { label: "Minimise", glyph: "–", onpress: move |()| {
                    let _ = host.edit(|w| w.minimise(&minimise));
                } }
                FloatButton { label: "Dock it", glyph: "⇲", onpress: move |()| {
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
fn FloatButton(label: &'static str, glyph: &'static str, onpress: EventHandler<()>) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    rsx! {
        button {
            style: "flex:0 0 auto; min-width:24px; height:22px; border:none; border-radius:4px; \
                    background:transparent; color:{t.ink_3};",
            title: label,
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |_| onpress.call(()),
            "{glyph}"
        }
    }
}

/// While anything drags: a layer over everything that shows where a pane
/// would land. The pointer itself is followed at the host's root (Blitz
/// sends a held pointer's moves to the element pressed; they bubble up).
#[component]
pub(super) fn DragLayer() -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let Some(drag) = (host.drag)() else {
        return rsx! {};
    };
    let highlight = match &drag {
        Drag::Pane {
            at: Some(d),
            geometry: Some((r, _)),
            ..
        } => Some(Rect::new(
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
    };
    let label = match &drag {
        Drag::Pane {
            pane,
            x,
            y,
            geometry: Some((r, _)),
            ..
        } => host
            .workspace
            .read()
            .pane(pane)
            .map(|p| (host.title(p), x - r.x + 12.0, y - r.y + 12.0)),
        _ => None,
    };
    let cursor = match drag {
        Drag::Pane { .. } | Drag::Float { .. } => "grabbing",
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
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:40; touch-action:none; cursor:{cursor};",
            if let Some(h) = highlight {
                div {
                    style: "position:absolute; left:{h.x}px; top:{h.y}px; width:{h.w}px; height:{h.h}px; \
                            background:rgba(191,219,254,0.12); border:2px solid {t.ink_2}; \
                            border-radius:4px; pointer-events:none;",
                }
            }
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
                    x,
                    y,
                }));
                spawn(async move { measure_for_drop(host, pane).await });
            }
        }
        Drag::Pane {
            pane,
            geometry: Some((r, groups)),
            ..
        } => {
            let at = drop_at(x, y, r, &groups);
            host.drag.set(Some(Drag::Pane {
                pane,
                geometry: Some((r, groups)),
                at,
                x,
                y,
            }));
        }
        Drag::Pane { .. } => {}
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
    {
        let workspace = host.workspace.peek();
        if let Some(main) = workspace.windows.first() {
            for float in main.floats.iter().rev().filter(|f| !f.minimised) {
                ids.push(float.group.id);
            }
            for dock in [&main.docks.left, &main.docks.right, &main.docks.bottom] {
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
    }
    let mut groups = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(g) = host.rect(Mount::Group(id)).await {
            groups.push((id, g));
        }
    }
    // The host's own rectangle, else what its groups cover.
    let Some(r) = measured.or_else(|| cover(&groups)) else {
        host.drag.set(None);
        return;
    };
    let still = matches!(host.drag.peek().as_ref(), Some(Drag::Pane { pane: p, .. }) if *p == pane);
    if still {
        let (x, y) = match host.drag.peek().as_ref() {
            Some(Drag::Pane { x, y, .. }) => (*x, *y),
            _ => (0.0, 0.0),
        };
        host.drag.set(Some(Drag::Pane {
            pane,
            geometry: Some((r, groups)),
            at: None,
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

pub(super) fn on_up(mut host: Host) {
    let drag = host.drag.peek().clone();
    host.drag.set(None);
    match drag {
        Some(Drag::Pane {
            pane, at: Some(d), ..
        }) => host.run(Op::Move(pane, d.dest)),
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

/// The pane menu and the open-a-view menu.
#[component]
pub(super) fn MenuLayer() -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let Some(menu) = (host.menu)() else {
        return rsx! {};
    };
    let items = match menu {
        Menu::Pane(pane) => pane_items(host, &pane),
        Menu::Open => open_items(host),
        Menu::Tabs(group) => tab_items(host, group),
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:50; background:{t.scrim}; \
                    display:flex; align-items:center; justify-content:center;",
            onclick: move |_| host.menu.set(None),
            div {
                style: "min-width:260px; max-width:90%; max-height:85%; overflow-y:scroll; \
                        display:flex; flex-direction:column; padding:6px; border-radius:12px; \
                        background:{t.pill}; border:1px solid {t.rule};",
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
}

#[component]
fn MenuRow(item: Item) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    match item {
        Item::Heading(text) => rsx! {
            div { style: "padding:10px 12px 4px; color:{t.ink_3}; font-size:11px; font-weight:600;", "{text}" }
        },
        Item::Act(label, op) => {
            let danger = matches!(op, Op::Close(_) | Op::CloseOthers(_));
            let ink = if danger { t.danger } else { t.ink };
            rsx! {
                button {
                    style: "display:flex; justify-content:flex-start; text-align:left; padding:0 12px; \
                            min-height:{s.button.max(36.0)}px; border:none; border-radius:8px; \
                            background:transparent; color:{ink}; font-size:14px;",
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
    if loc.area == Area::Center {
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
    if tabs > 1 {
        items.push(act("Close others", Op::CloseOthers(pane.clone())));
    }
    items.push(act("Close", Op::Close(pane.clone())));
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
/// count of panes (the switcher).
#[component]
pub(super) fn PhoneBody() -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let workspace = host.workspace.read();
    let pane = workspace
        .active
        .clone()
        .filter(|p| workspace.locate(p).is_some_and(|l| l.area == Area::Center))
        .or_else(|| workspace.current_document().cloned());
    let title = pane
        .as_ref()
        .and_then(|p| workspace.pane(p))
        .map(|p| host.title(p))
        .unwrap_or_default();
    let count = workspace.panes.len();
    drop(workspace);
    rsx! {
        div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:column;",
            div {
                style: "flex:0 0 44px; display:flex; align-items:center; gap:8px; padding:0 8px 0 14px; \
                        border-bottom:1px solid {t.rule}; background:{t.desk};",
                div { style: "flex:1; min-width:0; overflow:hidden; white-space:nowrap; font-weight:600;", "{title}" }
                button {
                    style: "min-width:44px; height:32px; padding:0 10px; border-radius:8px; \
                            border:1.5px solid {t.ink_3}; background:transparent; color:{t.ink}; \
                            font-size:13px; font-weight:700;",
                    title: "Every pane",
                    onclick: move |_| host.switcher.set(true),
                    "{count}"
                }
            }
            div { style: "flex:1 1 0; min-width:0; min-height:0; display:flex; position:relative;",
                match pane {
                    Some(pane) => rsx! { PaneSlot { key: "{pane.0}", pane } },
                    None => rsx! { super::EmptyCenter {} },
                }
            }
        }
        if (host.switcher)() {
            Switcher {}
        }
    }
}

/// A phone's list of every pane, by where it is: a press shows it (a
/// docked one as a sheet), a long list scrolls.
#[component]
fn Switcher() -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let workspace = host.workspace.read();
    let rows: Vec<(PaneId, String, &'static str)> = workspace
        .panes
        .iter()
        .filter_map(|p| {
            let place = match workspace.locate(&p.id)?.area {
                Area::Center => "Open",
                Area::Dock(Side::Left) => "Left side",
                Area::Dock(Side::Right) => "Right side",
                Area::Dock(_) => "Bottom",
                Area::Float(_) => "Floating",
            };
            Some((p.id.clone(), host.title(p), place))
        })
        .collect();
    drop(workspace);
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:30; background:{t.scrim};",
            onclick: move |_| host.switcher.set(false),
        }
        div {
            style: "position:absolute; left:0; right:0; bottom:0; max-height:80%; z-index:31; \
                    display:flex; flex-direction:column; padding:8px; border-radius:16px 16px 0 0; \
                    background:{t.sheet}; overflow-y:scroll;",
            for (pane, title, place) in rows {
                button {
                    key: "{pane.0}",
                    style: "display:flex; align-items:center; gap:8px; min-height:48px; padding:0 12px; \
                            border:none; border-radius:10px; background:transparent; color:{t.ink}; \
                            font-size:15px; text-align:left;",
                    onclick: move |_| {
                        host.switcher.set(false);
                        let docked = host
                            .workspace
                            .peek()
                            .locate(&pane)
                            .is_some_and(|l| l.area != Area::Center);
                        if docked {
                            host.sheet.set(Some(pane.clone()));
                        } else {
                            host.run(Op::Show(pane.clone()));
                        }
                    },
                    span { style: "flex:1; min-width:0; overflow:hidden; white-space:nowrap;", "{title}" }
                    span { style: "color:{t.ink_3}; font-size:12px;", "{place}" }
                }
            }
            button {
                style: "min-height:48px; margin-top:4px; border:none; border-radius:10px; \
                        background:{t.pill}; color:{t.ink}; font-size:15px;",
                onclick: move |e| {
                    e.stop_propagation();
                    host.switcher.set(false);
                    host.show_open_menu();
                },
                "Open a view"
            }
        }
    }
}

/// A phone's docked pane, opened over the one on show.
#[component]
pub(super) fn SheetLayer() -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let Some(pane) = (host.sheet)() else {
        return rsx! {};
    };
    let Some(title) = host.workspace.read().pane(&pane).map(|p| host.title(p)) else {
        return rsx! {};
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; background:{t.scrim}; z-index:32;",
            onclick: move |_| host.sheet.set(None),
        }
        div {
            style: "position:absolute; left:0; right:0; bottom:0; height:80%; z-index:33; display:flex; \
                    flex-direction:column; background:{t.sheet}; border-radius:16px 16px 0 0; overflow:hidden;",
            div {
                style: "flex:0 0 48px; display:flex; align-items:center; padding:0 6px 0 16px; \
                        border-bottom:1px solid {t.rule};",
                div { style: "flex:1; font-weight:600;", "{title}" }
                button {
                    style: "min-width:44px; min-height:44px; border:none; background:transparent; \
                            color:{t.ink}; font-size:18px;",
                    onclick: move |_| host.sheet.set(None),
                    "✕"
                }
            }
            div { style: "flex:1 1 0; min-height:0; display:flex; position:relative;",
                PaneSlot { key: "{pane.0}", pane }
            }
        }
    }
}
