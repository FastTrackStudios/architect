//! A dock on a side of the main window: its ribbon (the views it switches
//! between, then the commands pinned to it), its groups, and the divider
//! that sizes it. Collapsed, a dock is its ribbon, or a rail of its tabs'
//! icons when it has none.

use architect_shell::{Icon, IconFn};
use dioxus::prelude::*;

use super::{Drag, Host, Mount, Op, use_host};
use crate::model::{Area, Axis, Dock, Node, PaneId, Shown, Side};
use crate::ops::Target;

/// How far a dock gives way to a narrow window, points.
const DOCK_MIN: f32 = 140.0;

#[component]
pub(super) fn DockView(side: Side, dock: Dock) -> Element {
    let host = use_host();
    if dock.shown == Shown::Hidden || (dock.root.is_none() && dock.ribbon.is_none()) {
        return rsx! {};
    }
    let across = side.axis() == Axis::Row;
    let ribbon = dock
        .ribbon
        .clone()
        .map(|_| rsx! { Ribbon { side, dock: dock.clone() } });
    let body = match (dock.shown, dock.root.clone()) {
        (Shown::Open, Some(root)) => {
            let size = if across {
                format!(
                    "flex:0 1 {}px; width:{}px; min-width:{DOCK_MIN}px;",
                    dock.size, dock.size
                )
            } else {
                format!(
                    "flex:0 1 {}px; height:{}px; min-height:{DOCK_MIN}px;",
                    dock.size, dock.size
                )
            };
            let divider = rsx! { DockDivider { side } };
            // A side dock reaches the window's top; the bottom one does not.
            let ribbon_w = if dock.ribbon.is_some() {
                (host.sizes)().ribbon
            } else {
                0.0
            };
            let edges = super::group::Edges {
                top: side != Side::Bottom,
                left: side == Side::Left,
                right: side == Side::Right && dock.ribbon.is_none(),
                covered: ribbon_w,
            };
            let tree = rsx! {
                div { style: "{size} display:flex; position:relative; overflow:hidden;",
                    super::NodeView { node: root, edges }
                }
            };
            if side.leads() {
                rsx! { {tree} {divider} }
            } else {
                rsx! { {divider} {tree} }
            }
        }
        (_, Some(root)) if dock.ribbon.is_none() => rsx! { Rail { side, root } },
        _ => rsx! {},
    };
    // The ribbon sits on the window's edge, the dock inside it.
    match side {
        Side::Left => rsx! { {ribbon} {body} },
        _ => rsx! { {body} {ribbon} },
    }
}

/// The two-stage sidebar's first stage: the dock's views, then its pinned
/// commands.
#[component]
fn Ribbon(side: Side, dock: Dock) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let Some(ribbon) = dock.ribbon.clone() else {
        return rsx! {};
    };
    let views: Vec<(String, String, Option<IconFn>)> = ribbon
        .views
        .iter()
        .filter(|k| !ribbon.hidden.contains(k))
        .filter_map(|k| {
            host.views
                .read()
                .get(&k.0)
                .map(|v| (k.0.clone(), v.title.clone(), v.icon))
        })
        .collect();
    let commands: Vec<(String, String, IconFn)> = ribbon
        .commands
        .iter()
        .filter_map(|id| {
            host.commands
                .read()
                .iter()
                .find(|c| &c.id == id)
                .map(|c| (c.id.clone(), c.label.clone(), c.icon))
        })
        .collect();
    // The view on show in this dock, to light its icon.
    let workspace = host.workspace.read();
    let showing: Option<String> = dock
        .root
        .as_ref()
        .and_then(Node::first_group)
        .and_then(|g| g.current_tab())
        .and_then(|p| workspace.pane(p))
        .map(|p| p.view.0.clone());
    drop(workspace);
    let open = dock.shown == Shown::Open;
    let (dir, rule) = if side.axis() == Axis::Row {
        (
            "column",
            if side == Side::Left {
                "border-right"
            } else {
                "border-left"
            },
        )
    } else {
        ("row", "border-top")
    };
    let across = if side.axis() == Axis::Row {
        format!("width:{}px;", s.ribbon)
    } else {
        format!("height:{}px;", s.ribbon)
    };
    // Under a title row the ribbon starts below it, as the dock's tabs do.
    let top = (host.title_row)()
        .filter(|_| side != Side::Bottom)
        .map_or(6.0, |r| r.height.max(s.tabs) + 6.0);
    rsx! {
        div {
            style: "flex:0 0 auto; {across} display:flex; flex-direction:{dir}; align-items:center; \
                    gap:4px; padding:{top}px 0 6px; background:{t.desk}; {rule}:1px solid {t.rule};",
            for (kind, title, icon) in views {
                RibbonButton {
                    key: "{kind}",
                    label: title,
                    icon,
                    on: open && showing.as_deref() == Some(kind.as_str()),
                    onpress: move |()| press_view(&host, side, &kind),
                }
            }
            if !commands.is_empty() {
                div { style: "flex:0 0 1px; width:60%; height:1px; background:{t.rule}; margin:4px 0;" }
            }
            for (id, label, icon) in commands {
                RibbonButton {
                    key: "c{id}",
                    label,
                    icon: Some(icon),
                    on: false,
                    onpress: move |()| {
                        if let Some(on) = *host.on_command.read() {
                            on.call(id.clone());
                        }
                    },
                }
            }
        }
    }
}

/// The view on show in the dock on `side`.
fn showing_kind(host: &Host, side: Side) -> Option<String> {
    let workspace = host.workspace.peek();
    workspace
        .windows
        .first()
        .and_then(|w| w.docks.get(side))
        .and_then(|d| d.root.as_ref())
        .and_then(Node::first_group)
        .and_then(|g| g.current_tab())
        .and_then(|p| workspace.pane(p))
        .map(|p| p.view.0.clone())
}

/// A ribbon's view pressed: the one on show folds the dock; another is
/// brought into the dock (or opened there) and the dock opens.
fn press_view(host: &Host, side: Side, kind: &str) {
    let open = host
        .workspace
        .peek()
        .windows
        .first()
        .and_then(|w| w.docks.get(side))
        .is_some_and(|d| d.shown == Shown::Open);
    if open && showing_kind(host, side).as_deref() == Some(kind) {
        host.run(Op::Dock(side, Shown::Collapsed));
        return;
    }
    let existing: Option<PaneId> = host
        .workspace
        .peek()
        .panes
        .iter()
        .find(|p| p.view.0 == kind)
        .map(|p| p.id.clone());
    match existing {
        Some(pane) => {
            let here = host
                .workspace
                .peek()
                .locate(&pane)
                .is_some_and(|l| l.area == Area::Dock(side));
            if here {
                host.run(Op::Show(pane));
            } else {
                host.run(Op::Move(pane, crate::ops::Dest::Dock(side)));
            }
            host.run(Op::Dock(side, Shown::Open));
        }
        None => {
            let _ = host.open_at(kind, Target::Dock(side));
        }
    }
}

#[component]
fn RibbonButton(
    label: String,
    icon: Option<IconFn>,
    on: bool,
    onpress: EventHandler<()>,
) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    let ink = if on { t.ink } else { t.ink_3 };
    let fill = if on { t.active } else { "transparent" };
    let short: String = label.chars().take(2).collect();
    rsx! {
        button {
            style: "flex:0 0 auto; width:{s.button}px; height:{s.button}px; border:none; border-radius:8px; \
                    display:flex; align-items:center; justify-content:center; background:{fill}; color:{ink}; \
                    font-size:10px; font-weight:700;",
            title: "{label}",
            onclick: move |_| onpress.call(()),
            {icon.map_or_else(|| rsx! { "{short}" }, |icon| icon(Icon { size: 18, color: ink }))}
        }
    }
}

/// A collapsed dock with no ribbon: its tabs' icons; a press opens it on
/// that tab.
#[component]
fn Rail(side: Side, root: Node) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let across = side.axis() == Axis::Row;
    let shape = if across {
        format!("width:{RAIL}px; flex-direction:column;", RAIL = super::RAIL)
    } else {
        format!("height:{RAIL}px; flex-direction:row;", RAIL = super::RAIL)
    };
    let mut groups = Vec::new();
    root.groups(&mut groups);
    let workspace = host.workspace.read();
    let tabs: Vec<(PaneId, String, Option<IconFn>)> = groups
        .iter()
        .flat_map(|g| g.tabs.iter())
        .filter_map(|id| workspace.pane(id))
        .map(|p| (p.id.clone(), host.title(p), host.icon(p)))
        .collect();
    drop(workspace);
    rsx! {
        div {
            style: "flex:0 0 auto; {shape} display:flex; align-items:center; gap:4px; padding:4px; \
                    background:{t.desk}; overflow:hidden;",
            for (pane, title, icon) in tabs {
                RibbonButton {
                    key: "{pane.0}",
                    label: title,
                    icon,
                    on: false,
                    onpress: move |()| {
                        host.run(Op::Dock(side, Shown::Open));
                        host.run(Op::Show(pane.clone()));
                    },
                }
            }
        }
    }
}

/// Between a dock and the center: dragged to size the dock, double-clicked
/// to fold it.
#[component]
fn DockDivider(side: Side) -> Element {
    let mut host = use_host();
    let t = *host.theme.read();
    let s = (host.sizes)();
    // A hairline on the dock's own edge, the handle wide enough to grab.
    let line = match side {
        Side::Left => "border-left",
        Side::Right => "border-right",
        Side::Top => "border-bottom",
        Side::Bottom => "border-top",
    };
    let (cursor, shape) = if side.axis() == Axis::Row {
        ("ew-resize", format!("width:{}px;", s.divider))
    } else {
        ("ns-resize", format!("height:{}px;", s.divider))
    };
    rsx! {
        div {
            style: "flex:0 0 auto; {shape} box-sizing:border-box; background:{t.desk}; {line}:1px solid {t.rule}; \
                    cursor:{cursor}; touch-action:none;",
            onpointerdown: move |_| {
                spawn(async move {
                    if let Some(rect) = host.rect(Mount::Host).await {
                        host.drag.set(Some(Drag::Dock { side, host: rect }));
                    }
                });
            },
            ondoubleclick: move |_| host.run(Op::Dock(side, Shown::Collapsed)),
        }
    }
}
