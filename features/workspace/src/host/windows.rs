//! Pop-out windows: the workspace's windows after the main one, each in
//! an OS window of its own where the platform gives one.
//!
//! Blitz draws one document per `VirtualDom`, so a pop-out is a second
//! dom whose root, [`PopoutView`], draws its window of the same
//! `Signal<Workspace>` the main window's host has: a signal owned by one
//! dom redraws what reads it in another (`tests/windows.rs`). The platform
//! side — making the OS window, driving its document, telling the host
//! when the person closes or moves it — is a [`WindowSystem`]; the
//! `native` feature's launcher is the one for a desk.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use architect_shell::{ShellTheme, StatusItem};
use dioxus::prelude::*;
use futures_util::StreamExt as _;

use super::dock::DockView;
use super::group::NodeView;
use super::overlay::{DragLayer, DropHighlight, MenuLayer};
use super::{Command, Drag, Host, Mount, Op, Sizes, Views, overlay, point, use_host};
use crate::model::{Docks, GroupId, PaneId, Rect, Side, Window, WindowId, Workspace};
use crate::screenset::{FormClass, Screensets};

/// What the platform does with windows.
///
/// Open one for a [`Popout`], close it, and say where windows are on
/// screen (so a tab let go outside one window lands in the window under
/// the pointer, or in a new one there). Points are logical, the screen's
/// top left the origin.
pub trait WindowSystem {
    /// Open an OS window drawing `popout`, or bring its window forward if
    /// one is open for its id.
    fn open(&self, popout: Popout);
    /// Close the OS window for `window`, if one is open (its tabs have
    /// gone back already: this is the layout's change, not the person's).
    fn close(&self, window: &WindowId);
    /// Where `window`'s content (its document's top left) is on screen.
    fn origin(&self, window: &WindowId) -> Option<(f32, f32)>;
    /// Where `window`'s content is on screen, and its size (a dock popped
    /// out goes over where it was, as tall as the window). `None` when the
    /// platform does not say.
    fn content(&self, window: &WindowId) -> Option<Rect> {
        let _ = window;
        None
    }
    /// The window whose content is under the screen point, front first.
    fn window_at(&self, x: f32, y: f32) -> Option<WindowId>;
    /// Call `release` with the point (`window`'s logical points) where the
    /// pointer's button went up outside `window`'s content: the document
    /// there never hears it, and a tab dragged out ends there. It may be
    /// called outside any dom (send it on, don't draw from it).
    fn on_release_outside(&self, window: &WindowId, release: Rc<dyn Fn(f32, f32)>);
}

/// Hears where a drag let go outside this host's window, from the
/// platform, and ends the drag there.
pub(super) fn use_release_outside(host: Host) {
    let heard = use_coroutine(move |mut rx: UnboundedReceiver<(f32, f32)>| async move {
        while let Some(at) = rx.next().await {
            overlay::release_outside(host, at);
        }
    });
    use_hook(move || {
        if let Some(windows) = host.windows.peek().clone() {
            let tx = heard.tx();
            let released = host.released.peek().clone();
            windows.0.on_release_outside(
                &host.window_id(),
                Rc::new(move |x, y| {
                    released.set(Some((x, y)));
                    let _ = tx.unbounded_send((x, y));
                }),
            );
        }
    });
}

/// The platform's windows, in context for every host (the `native`
/// launcher puts one at the root).
#[derive(Clone)]
pub struct Windows(pub Rc<dyn WindowSystem>);

impl Windows {
    pub fn new(system: impl WindowSystem + 'static) -> Self {
        Self(Rc::new(system))
    }
}

impl PartialEq for Windows {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Contexts carried from the main window into a pop-out.
///
/// What the app's views read with `use_context` (the shell, the app's own
/// state). Captured where the main window's host is drawn, provided again at the
/// pop-out's root.
#[derive(Clone, Default)]
pub struct Bridge(Rc<Vec<Rc<dyn Fn()>>>);

impl PartialEq for Bridge {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Bridge {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Carry the `T` in context here (if there is one) into pop-outs. Call
    /// it in a component, where the context is.
    #[must_use]
    pub fn with<T: Clone + 'static>(self, value: Option<T>) -> Self {
        let Some(value) = value else {
            return self;
        };
        let mut all = (*self.0).clone();
        all.push(Rc::new(move || {
            provide_context(value.clone());
        }));
        Self(Rc::new(all))
    }

    /// Carry the `T` in context in the current component, if any.
    #[must_use]
    pub fn context<T: Clone + 'static>(self) -> Self {
        self.with(try_consume_context::<T>())
    }

    /// The contexts of both.
    #[must_use]
    fn and(self, other: &Self) -> Self {
        let mut all = (*self.0).clone();
        all.extend(other.0.iter().cloned());
        Self(Rc::new(all))
    }

    /// Provide every carried context in the current component.
    fn provide(&self) {
        for give in self.0.iter() {
            give();
        }
    }
}

/// A pop-out window, for a [`WindowSystem`] to open: its id, title and
/// place, the document to draw in it, and what to tell the host.
#[derive(Clone)]
pub struct Popout {
    props: PopoutProps,
    title: String,
    bounds: Option<Rect>,
    on_closed: Rc<dyn Fn()>,
    on_moved: Rc<dyn Fn(Rect)>,
}

impl Popout {
    /// The window's id (carried to the platform: a window manager can
    /// place it by it).
    #[must_use]
    pub const fn id(&self) -> &WindowId {
        &self.props.window
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Where it was last, on screen; `None` for the platform to place it.
    #[must_use]
    pub const fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// The document to draw in the window: its own dom, sharing the main
    /// window's state.
    #[must_use]
    pub fn dom(&self) -> VirtualDom {
        VirtualDom::new_with_props(PopoutView, self.props.clone())
    }

    /// The person closed the window: its tabs go back to the main window.
    /// Call it inside the pop-out's dom (`VirtualDom::in_runtime`), before
    /// dropping it.
    pub fn closed(&self) {
        (self.on_closed)();
    }

    /// The window moved or was resized (in its dom, as for
    /// [`closed`](Self::closed)).
    pub fn moved(&self, bounds: Rect) {
        (self.on_moved)(bounds);
    }
}

/// What a pop-out's root is given: the main window's state.
#[derive(Props, Clone)]
pub(super) struct PopoutProps {
    window: WindowId,
    workspace: Signal<Workspace>,
    views: Views,
    theme: ShellTheme,
    sizes: Sizes,
    status: Signal<Vec<(PaneId, u64, Vec<StatusItem>)>>,
    status_serial: CopyValue<u64>,
    screensets: Signal<Screensets>,
    windows: Windows,
    bridge: Bridge,
    /// What a popped-out dock's ribbon may pin, and where a press goes.
    commands: Vec<Command>,
    pressed: Signal<Option<String>>,
}

impl PartialEq for PopoutProps {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window
            && self.workspace == other.workspace
            && self.views == other.views
            && self.windows == other.windows
    }
}

/// In the main window: an OS window for each pop-out in the workspace,
/// opened as one appears and closed as one goes.
#[component]
pub(super) fn PopoutSync(bridge: Bridge) -> Element {
    let host = use_host();
    // The contexts every view may read, and the app's own.
    let carried = use_hook(|| {
        Bridge::new()
            .context::<Signal<architect_shell::Shell>>()
            .context::<architect_commands::Commands>()
            .and(&bridge)
    });
    let opened = use_hook(|| Rc::new(RefCell::new(HashMap::<WindowId, String>::new())));
    let w = host.workspace.read();
    let wanted: Vec<(WindowId, String, Option<Rect>)> = w
        .windows
        .iter()
        .skip(1)
        .map(|win| (win.id.clone(), window_title(host, &w, win), win.bounds))
        .collect();
    drop(w);
    let Some(windows) = host.windows.peek().clone() else {
        return rsx! {};
    };
    let mut open = opened.borrow_mut();
    let gone: Vec<WindowId> = open
        .keys()
        .filter(|id| !wanted.iter().any(|(w, _, _)| w == *id))
        .cloned()
        .collect();
    for id in gone {
        open.remove(&id);
        windows.0.close(&id);
    }
    for (id, title, bounds) in wanted {
        if open.contains_key(&id) {
            continue;
        }
        open.insert(id.clone(), title.clone());
        let closed_id = id.clone();
        let moved_id = id.clone();
        let opened_too = opened.clone();
        let popout = Popout {
            props: PopoutProps {
                window: id,
                workspace: host.workspace,
                views: host.views.peek().clone(),
                theme: *host.theme.peek(),
                sizes: *host.sizes.peek(),
                status: host.status,
                status_serial: host.status_serial,
                screensets: host.screensets,
                windows: windows.clone(),
                bridge: carried.clone(),
                commands: host.commands.peek().clone(),
                pressed: host.pressed,
            },
            title,
            bounds,
            on_closed: Rc::new(move || {
                opened_too.borrow_mut().remove(&closed_id);
                host.run(Op::CloseWindow(closed_id.clone()));
            }),
            on_moved: Rc::new(move |r| {
                let mut workspace = host.workspace;
                let differs = workspace
                    .peek()
                    .window(&moved_id)
                    .is_some_and(|w| w.bounds != Some(r));
                if differs {
                    let _ = workspace.write().set_window_bounds(&moved_id, r);
                }
            }),
        };
        windows.0.open(popout);
    }
    drop(open);
    rsx! {}
}

/// A window's title: its center's tab on show, or, for a dock's own
/// window, the dock's name (the OS window's title, for a person and a
/// script to find it by).
fn window_title(host: Host, w: &Workspace, win: &Window) -> String {
    if let Some(p) = win
        .center
        .as_ref()
        .and_then(crate::model::Node::first_group)
        .and_then(|g| g.current_tab())
        .and_then(|p| w.pane(p))
    {
        return host.title(p);
    }
    let side = Docks::SIDES
        .into_iter()
        .find(|&s| win.docks.get(s).is_some_and(|d| !d.is_empty()));
    match side {
        Some(Side::Left) => "Sidebar".to_owned(),
        Some(Side::Right) => "Right sidebar".to_owned(),
        Some(_) => "Bottom panel".to_owned(),
        None => String::new(),
    }
}

/// A pop-out's root: its window's tree of splits and groups, and a dock
/// popped out into it filling it (no floats), with its own drag and menu
/// layers.
#[allow(clippy::needless_pass_by_value)]
#[component]
fn PopoutView(props: PopoutProps) -> Element {
    use_hook(|| props.bridge.provide());
    let host = use_context_provider(|| Host {
        workspace: props.workspace,
        views: CopyValue::new(props.views.clone()),
        commands: CopyValue::new(props.commands.clone()),
        on_command: CopyValue::new(None),
        theme: CopyValue::new(props.theme),
        sizes: Signal::new(props.sizes),
        phone: Signal::new(false),
        title_row: Signal::new(None),
        in_shell: Signal::new(false),
        phone_header: Signal::new(false),
        drag: Signal::new(None),
        menu: Signal::new(None),
        sheet: Signal::new(None),
        switcher: Signal::new(false),
        switch: super::switcher::Switch::new(),
        float_area: Signal::new(None),
        menu_at: Signal::new(None),
        mounts: CopyValue::new(HashMap::new()),
        held: Signal::new(false),
        latched: Signal::new(false),
        window: CopyValue::new(props.window.clone()),
        status: props.status,
        status_serial: props.status_serial,
        screensets: props.screensets,
        form: Signal::new(FormClass::Desk),
        scrolls: Signal::new(HashMap::new()),
        strips: CopyValue::new(HashMap::new()),
        tab_shift: CopyValue::new(HashMap::new()),
        tab_from: CopyValue::new(HashMap::new()),
        lifted: Signal::new(None),
        growing: Signal::new(None),
        grow: architect_shell::motion::Motion::new(
            1.0,
            architect_shell::motion::Spring::PANEL,
            0.001,
        ),
        grow_size: CopyValue::new(None),
        split_sizes: CopyValue::new(HashMap::new()),
        closing: CopyValue::new(None),
        windows: CopyValue::new(Some(props.windows.clone())),
        pressed: props.pressed,
        released: CopyValue::new(Rc::default()),
        revisions: CopyValue::new(super::actions::Revisions::default()),
        measured: CopyValue::new(crate::tree::Measured::default()),
    });
    use_release_outside(host);
    let mut host = host;
    let t = props.theme;
    let w = host.workspace.read();
    let index = host.window_index(&w);
    let window = index.and_then(|i| w.windows.get(i));
    let center = window.and_then(super::shown_center);
    let title = window.map_or_else(String::new, |win| window_title(host, &w, win));
    let docks: Vec<(Side, crate::model::Dock)> = window
        .map(|win| {
            Docks::SIDES
                .into_iter()
                .filter_map(|s| {
                    win.docks
                        .get(s)
                        .filter(|d| !d.is_empty())
                        .map(|d| (s, d.clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    drop(w);
    let edges = super::group::Edges::default();
    let ground = format!(
        "html, body {{ margin: 0; padding: 0; height: 100%; background: {}; }}",
        t.desk
    );
    rsx! {
        document::Title { "{title}" }
        document::Style { {ground} }
        div {
            style: "position:relative; width:100vw; height:100vh; min-width:0; min-height:0; \
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
                if e.key() == Key::Named(NamedKey::Escape) {
                    host.drag.set(None::<Drag>);
                    host.menu.set(None);
                }
            },
            {center.map_or_else(|| rsx! {}, |node| rsx! { NodeView { node, edges } })}
            for (side, dock) in docks {
                div {
                    key: "{side:?}",
                    style: "flex:1 1 0; min-width:0; min-height:0; display:flex; flex-direction:{dock_flow(side)};",
                    DockView { side, dock, fill: true }
                }
            }
            architect_shell::motion::MotionFrames {}
            DropHighlight {}
            DragLayer {}
            MenuLayer {}
        }
    }
}

/// How a dock filling a window lays out its ribbon and its groups.
const fn dock_flow(side: Side) -> &'static str {
    match side {
        Side::Left | Side::Right => "row",
        Side::Top | Side::Bottom => "column",
    }
}

/// Where a tab let go at `(x, y)` (this host's window points) outside its
/// window goes: the window under the pointer on screen, or a new one there
/// the size of the group it left.
pub(super) fn drop_outside(
    host: Host,
    pane: &PaneId,
    (x, y): (f32, f32),
    from: Option<(GroupId, Rect)>,
) -> Option<Op> {
    let windows = host.windows.peek().clone()?;
    let here = host.window_id();
    let (ox, oy) = windows.0.origin(&here)?;
    let (sx, sy) = (ox + x, oy + y);
    match windows.0.window_at(sx, sy) {
        Some(there) if there == here => None,
        Some(there) if there == WindowId::main() => Some(Op::ToMain(pane.clone())),
        Some(there) => {
            let group = host
                .workspace
                .peek()
                .window(&there)
                .and_then(Window::first_group)
                .map(|g| g.id)?;
            Some(Op::Move(
                pane.clone(),
                crate::ops::Dest::Group { group, index: None },
            ))
        }
        None => {
            let (w, h) = from.map_or((640.0, 480.0), |(_, r)| (r.w.max(320.0), r.h.max(240.0)));
            // Under the pointer, as if it carried the tab by its top.
            let at = Rect::new(sx - 60.0, sy - 16.0, w, h);
            Some(Op::Move(pane.clone(), crate::ops::Dest::Window(Some(at))))
        }
    }
}

/// Where a dock's grip let go at `(x, y)` (this host's window points)
/// outside its window takes the dock: from the main window, a window of
/// its own there; from its own window, onto the main one, back to its side.
pub(super) fn dock_outside(host: Host, side: Side, (x, y): (f32, f32)) -> Option<Op> {
    let windows = host.windows.peek().clone()?;
    let here = host.window_id();
    let (ox, oy) = windows.0.origin(&here)?;
    let (sx, sy) = (ox + x, oy + y);
    let there = windows.0.window_at(sx, sy);
    if here == WindowId::main() {
        if there.is_some() {
            return None;
        }
        let (w, h) = host
            .dock_bounds(side)
            .map_or((300.0, 600.0), |r| (r.w, r.h));
        // Under the pointer, as if it carried the dock by its grip.
        return Some(Op::PopOutDock(
            side,
            Some(Rect::new(sx - 20.0, sy - 20.0, w, h)),
        ));
    }
    (there == Some(WindowId::main())).then_some(Op::DockBack(side))
}
