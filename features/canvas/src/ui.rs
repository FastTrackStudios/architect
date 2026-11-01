//! Layers and windows on Blitz (feature `ui`).
//!
//! - [`ShapesView`] draws shapes as absolutely placed boxes in their
//!   parent, taking no pointer (Blitz draws boxes, borders, rotations and
//!   text; there is no `<canvas>`).
//! - [`LayerView`] is one layer over a view, mounted as the view's child
//!   where its plane is (its last child for `over` and `top`, under its
//!   items for `under`), like the workspace's other overlays: covering the
//!   view, clipped to it, passing the pointer through except over the
//!   regions it claims, which report the pointer in the view's points.
//! - [`WindowView`] is one immediate-mode window, filling its pane, giving
//!   back the pointer in its points and its size.
//! - [`window_view`] is the workspace view a window is docked, tabbed or
//!   floated as: kind [`WINDOW_VIEW`], one pane per window, the pane's
//!   state the window's id.
//!
//! With `luau` too, [`use_script_canvas`] runs a script's
//! [`Session`](crate::luau::Session) from Dioxus: a frame each time
//! something it draws from changes, the pointer's presses and releases
//! each a frame of their own. Its canvases go in a [`Canvases`] in
//! context, where the views find them: [`CanvasLayers`] draws every
//! canvas's layers over a view, [`Canvases::publish`] gives them its
//! coordinate spaces, and [`use_tool_windows`] opens a floating pane for
//! each window a script begins, and tells the script when its pane is
//! closed.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use architect_shell::ShellTheme;
use architect_workspace::{PaneId, View};

use crate::draw::{DrawList, Shape};
use crate::geometry::{Color, Point, Size};
use crate::imgui::{Style, WindowFrame};
use crate::layer::{Layer, ViewPointer};

/// The kind of the workspace view a script's window is shown in.
pub const WINDOW_VIEW: &str = "canvas.window";

/// Over the view's own content, under the shell's edges and floats.
const LAYER_Z: u8 = 3;

/// A pane id for window `id`: stable, so opening it again finds it.
#[must_use]
pub fn window_pane(id: &str) -> PaneId {
    PaneId::new(format!("{WINDOW_VIEW}:{id}"))
}

/// The style the shell's theme gives windows: its sheet behind, its pills
/// as frames, its ink, its accent for grabs and checks.
#[must_use]
pub fn style_from_theme(theme: &ShellTheme) -> Style {
    let base = Style::default();
    let pick = |css: &str, or: Color| Color::from_css(css).unwrap_or(or);
    let accent = pick(theme.accent, base.slider_grab);
    let rule = pick(theme.rule, base.frame_hovered);
    Style {
        text: pick(theme.ink, base.text),
        text_disabled: pick(theme.ink_3, base.text_disabled),
        window_bg: pick(theme.sheet, base.window_bg),
        frame_bg: pick(theme.pill, base.frame_bg),
        frame_hovered: rule,
        button: rule,
        button_active: accent.faded(0.5),
        slider_grab: accent,
        check_mark: accent,
        separator: rule,
        plot: accent,
        ..base
    }
}

fn px(v: f32) -> String {
    format!("{v:.1}px")
}

fn shape_style(shape: &Shape) -> String {
    match shape {
        Shape::Line {
            from,
            to,
            color,
            thickness,
        } => {
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            let length = dx.hypot(dy);
            let angle = dy.atan2(dx);
            format!(
                "left:{}; top:{}; width:{}; height:{}; background:{}; \
                 transform-origin:0 50%; transform:rotate({angle:.4}rad);",
                px(from.x),
                px(from.y - thickness / 2.0),
                px(length),
                px(*thickness),
                color.css(),
            )
        }
        Shape::Rect {
            rect,
            color,
            rounding,
            fill,
            thickness,
        } => {
            let paint = if *fill {
                format!("background:{};", color.css())
            } else {
                format!(
                    "border:{} solid {}; box-sizing:border-box;",
                    px(*thickness),
                    color.css()
                )
            };
            format!(
                "left:{}; top:{}; width:{}; height:{}; border-radius:{}; {paint}",
                px(rect.x),
                px(rect.y),
                px(rect.w),
                px(rect.h),
                px(*rounding),
            )
        }
        Shape::Circle {
            center,
            radius,
            color,
            fill,
            thickness,
        } => {
            let paint = if *fill {
                format!("background:{};", color.css())
            } else {
                format!(
                    "border:{} solid {}; box-sizing:border-box;",
                    px(*thickness),
                    color.css()
                )
            };
            format!(
                "left:{}; top:{}; width:{d}; height:{d}; border-radius:50%; {paint}",
                px(center.x - radius),
                px(center.y - radius),
                d = px(radius * 2.0),
            )
        }
        Shape::Text {
            at, color, size, ..
        } => format!(
            "left:{}; top:{}; font-size:{}; line-height:{}; color:{}; white-space:nowrap;",
            px(at.x),
            px(at.y),
            px(*size),
            px(*size),
            color.css(),
        ),
    }
}

/// `draw`'s shapes, placed in their (positioned) parent, taking no
/// pointer.
#[component]
pub fn ShapesView(draw: DrawList) -> Element {
    rsx! {
        for (i, shape) in draw.shapes().iter().enumerate() {
            div {
                key: "{i}",
                style: "position:absolute; pointer-events:none; {shape_style(shape)}",
                if let Shape::Text { text, .. } = shape {
                    "{text}"
                }
            }
        }
    }
}

fn view_pointer(e: &PointerEvent, origin: Point, inside: bool) -> ViewPointer {
    let p = e.element_coordinates();
    // Points on a screen: far inside f32's range.
    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    let at = Point::new(p.x as f32, p.y as f32).offset(origin);
    ViewPointer {
        at: inside.then_some(at),
        down: e.held_buttons().contains(MouseButton::Primary),
        context: e.trigger_button() == Some(MouseButton::Secondary),
    }
}

/// One layer over a view: its shapes, and its regions taking the pointer
/// (`on_pointer`, in the view's points). Mount it as a child of the
/// (positioned) view.
#[component]
pub fn LayerView(layer: Layer, on_pointer: Option<EventHandler<ViewPointer>>) -> Element {
    let report = move |p: ViewPointer| {
        if let Some(h) = on_pointer {
            h.call(p);
        }
    };
    rsx! {
        div {
            style: "position:absolute; inset:0; pointer-events:none; overflow:hidden; z-index:{LAYER_Z};",
            ShapesView { draw: layer.draw.clone() }
            for region in layer.regions.iter().cloned() {
                div {
                    key: "{region.id}",
                    style: "position:absolute; pointer-events:auto; left:{px(region.rect.x)}; top:{px(region.rect.y)}; \
                            width:{px(region.rect.w)}; height:{px(region.rect.h)}; cursor:pointer; touch-action:none;",
                    onpointermove: move |e| report(view_pointer(&e, region.rect.origin(), true)),
                    onpointerdown: move |e| report(view_pointer(&e, region.rect.origin(), true)),
                    onpointerup: move |e| report(view_pointer(&e, region.rect.origin(), true)),
                    onpointerleave: move |e| report(view_pointer(&e, region.rect.origin(), false)),
                }
            }
        }
    }
}

/// What a [`WindowView`] reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowEvent {
    /// The pointer, in the window's points (`None`: it left), and whether
    /// the primary button is held.
    Pointer { at: Option<Point>, down: bool },
    /// Its pane's size.
    Size(Size),
}

/// One immediate-mode window, filling its (positioned) pane.
#[component]
pub fn WindowView(window: WindowFrame, on_event: EventHandler<WindowEvent>) -> Element {
    let pointer = move |e: &PointerEvent, inside: bool| {
        let p = e.element_coordinates();
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let at = Point::new(p.x as f32, p.y as f32);
        WindowEvent::Pointer {
            at: inside.then_some(at),
            down: e.held_buttons().contains(MouseButton::Primary),
        }
    };
    let measure = move |mounted: std::rc::Rc<MountedData>| async move {
        if let Ok(r) = mounted.get_client_rect().await {
            #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
            on_event.call(WindowEvent::Size(Size::new(
                r.size.width as f32,
                r.size.height as f32,
            )));
        }
    };
    let mut mount = use_signal(|| None::<std::rc::Rc<MountedData>>);
    rsx! {
        div {
            style: "position:absolute; inset:0; overflow:hidden; touch-action:none; user-select:none;",
            onmounted: move |e| {
                let data = e.data();
                mount.set(Some(data.clone()));
                spawn(measure(data));
            },
            onpointerenter: move |_| {
                if let Some(m) = mount() {
                    spawn(measure(m));
                }
            },
            onpointermove: move |e| on_event.call(pointer(&e, true)),
            onpointerdown: move |e| on_event.call(pointer(&e, true)),
            onpointerup: move |e| on_event.call(pointer(&e, true)),
            onpointerleave: move |e| on_event.call(pointer(&e, false)),
            ShapesView { draw: window.draw }
        }
    }
}

/// The workspace view windows are shown in ([`WINDOW_VIEW`]): one pane a
/// window, found by its state among the [`Canvases`] in context.
#[must_use]
pub fn window_view() -> View {
    View::new(WINDOW_VIEW, "Tool window", |_| rsx! { WindowPane {} })
        .home(architect_workspace::Target::Float(
            architect_workspace::Rect::new(80.0, 80.0, 320.0, 160.0),
        ))
        .many()
}

#[component]
fn WindowPane() -> Element {
    let pane = architect_workspace::use_pane();
    let id = pane.state();
    let found = windows::find(&id);
    match found {
        Some((window, on_event)) => rsx! { WindowView { window, on_event } },
        None => rsx! {
            div { style: "padding:12px; font-size:12px; opacity:0.7;",
                "This window's script is not running."
            }
        },
    }
}

#[cfg(feature = "luau")]
mod windows {
    pub(super) use super::script::find;
}

#[cfg(not(feature = "luau"))]
mod windows {
    use dioxus::prelude::*;

    use super::WindowEvent;
    use crate::imgui::WindowFrame;

    /// Without scripts, no window is drawn.
    pub(super) const fn find(_: &str) -> Option<(WindowFrame, EventHandler<WindowEvent>)> {
        None
    }
}

#[cfg(feature = "luau")]
pub use script::{
    CanvasLayers, Canvases, ScriptCanvas, Status, use_canvases, use_script_canvas, use_tool_windows,
};

#[cfg(feature = "luau")]
mod script {
    use std::collections::BTreeSet;

    use architect::action::ActionCaller;
    use architect_scripting::ScriptError;
    use architect_scripting::luau::Runtime;
    use architect_workspace::{Pane, Rect as PaneRect, Target, Workspace};
    use dioxus::prelude::*;
    use futures::StreamExt;
    use futures::channel::mpsc::UnboundedReceiver;
    use serde_json::Value;

    use super::{LayerView, WINDOW_VIEW, WindowEvent, window_pane};
    use crate::Output;
    use crate::dock::Docking;
    use crate::geometry::{Point, Size};
    use crate::imgui::{Input, Style, WindowFrame, WindowPointer};
    use crate::layer::{Plane, ViewPointer};
    use crate::luau::Session;
    use crate::space::Spaces;

    /// How a script's run went.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Status {
        /// It has a next frame.
        pub running: bool,
        /// Why it stopped, when it failed.
        pub error: Option<ScriptError>,
        /// What it printed, newest last (the last hundred lines).
        pub log: Vec<String>,
    }

    enum Msg {
        Spaces(String, Option<Spaces>),
        View(String, ViewPointer),
        Window(String, WindowEvent),
        Closed(String),
    }

    /// One script on the canvas: what it draws and how it is going.
    #[derive(Clone, Copy, PartialEq)]
    pub struct ScriptCanvas {
        output: Signal<Output>,
        status: Signal<Status>,
        tx: Coroutine<Msg>,
    }

    impl ScriptCanvas {
        /// What it drew last. Reactive.
        #[must_use]
        pub fn output(&self) -> Output {
            self.output.read().clone()
        }

        /// Reactive.
        #[must_use]
        pub fn status(&self) -> Status {
            self.status.read().clone()
        }

        /// A view of kind `view` shows with `spaces` (`None`: it is gone).
        pub fn publish(&self, view: &str, spaces: Option<Spaces>) {
            self.tx.send(Msg::Spaces(view.to_owned(), spaces));
        }

        /// The pointer over `view`'s layers.
        pub fn view_pointer(&self, view: &str, pointer: ViewPointer) {
            self.tx.send(Msg::View(view.to_owned(), pointer));
        }

        /// What window `id`'s pane reports.
        pub fn window_event(&self, id: &str, event: WindowEvent) {
            self.tx.send(Msg::Window(id.to_owned(), event));
        }

        /// The person closed window `id`'s pane.
        pub fn closed(&self, id: &str) {
            self.tx.send(Msg::Closed(id.to_owned()));
        }
    }

    /// Every script canvas under an app, for its views to find.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct Canvases(Signal<Vec<ScriptCanvas>>);

    impl Canvases {
        /// Reactive.
        #[must_use]
        pub fn all(&self) -> Vec<ScriptCanvas> {
            self.0.read().clone()
        }

        fn add(&self, canvas: ScriptCanvas) {
            let mut all = self.0;
            all.write().push(canvas);
        }

        fn remove(&self, canvas: ScriptCanvas) {
            let mut all = self.0;
            all.write().retain(|c| *c != canvas);
        }

        /// A view of kind `view` shows with `spaces`, to every canvas.
        pub fn publish(&self, view: &str, spaces: Option<&Spaces>) {
            for c in self.0.peek().iter() {
                c.publish(view, spaces.cloned());
            }
        }
    }

    /// The [`Canvases`] for everything under the caller.
    #[must_use]
    pub fn use_canvases() -> Canvases {
        use_context_provider(|| Canvases(Signal::new(Vec::new())))
    }

    /// Window `id`, from whichever canvas drew it, and where its events go.
    pub(in crate::ui) fn find(id: &str) -> Option<(WindowFrame, EventHandler<WindowEvent>)> {
        let canvases = try_consume_context::<Canvases>()?;
        canvases.all().into_iter().find_map(|c| {
            let window = c.output.read().window(id).cloned()?;
            let id = id.to_owned();
            let handler = EventHandler::new(move |e: WindowEvent| c.window_event(&id, e));
            Some((window, handler))
        })
    }

    /// Runs `source` as a live script on `runtime` as `caller`, its windows
    /// styled by `theme`, in the [`Canvases`] in context (when there are).
    pub fn use_script_canvas(
        runtime: Runtime,
        caller: ActionCaller,
        source: String,
        args: Value,
        step: String,
        style: Style,
    ) -> ScriptCanvas {
        let output = use_signal(Output::default);
        let status = use_signal(Status::default);
        let tx = use_coroutine(move |rx: UnboundedReceiver<Msg>| {
            let (runtime, caller, source, args, step, style) = (
                runtime.clone(),
                caller.clone(),
                source.clone(),
                args.clone(),
                step.clone(),
                style.clone(),
            );
            drive(
                Start {
                    runtime,
                    caller,
                    source,
                    args,
                    step,
                    style,
                },
                rx,
                output,
                status,
            )
        });
        let canvas = ScriptCanvas { output, status, tx };
        let canvases = try_use_context::<Canvases>();
        use_hook(move || {
            if let Some(c) = canvases {
                c.add(canvas);
            }
        });
        use_drop(move || {
            if let Some(c) = canvases {
                c.remove(canvas);
            }
        });
        canvas
    }

    struct Start {
        runtime: Runtime,
        caller: ActionCaller,
        source: String,
        args: Value,
        step: String,
        style: Style,
    }

    const LOG_KEPT: usize = 100;

    fn note(
        status: &mut Signal<Status>,
        report: &architect_scripting::luau::Report,
        running: bool,
    ) {
        let mut s = status.write();
        s.running = running;
        if let Err(e) = &report.result {
            s.error = Some(e.clone());
        }
        s.log.extend(report.log.iter().cloned());
        let over = s.log.len().saturating_sub(LOG_KEPT);
        s.log.drain(..over);
    }

    async fn drive(
        start: Start,
        mut rx: UnboundedReceiver<Msg>,
        mut output: Signal<Output>,
        mut status: Signal<Status>,
    ) {
        let (report, session) = Session::start(
            &start.runtime,
            &start.caller,
            &start.source,
            start.args,
            &start.step,
        )
        .await;
        let Some(mut session) = session else {
            note(&mut status, &report, false);
            return;
        };
        session.set_style(start.style);
        note(&mut status, &report, session.running());
        output.set(session.output().clone());
        let mut input = Input::default();
        while let Some(first) = rx.next().await {
            // Moves coalesce; a press or a release is a frame of its own.
            let mut next = Some(first);
            while let Some(msg) = next.take() {
                if apply(&session, &mut input, msg) {
                    break;
                }
                next = rx.try_recv().ok();
            }
            let report = session.frame(input.clone()).await;
            input.closed.clear();
            note(&mut status, &report, session.running());
            if *output.peek() != *session.output() {
                output.set(session.output().clone());
            }
            if !session.running() {
                return;
            }
        }
    }

    /// Takes `msg` in; whether it pressed or let go.
    fn apply(session: &Session, input: &mut Input, msg: Msg) -> bool {
        match msg {
            Msg::Spaces(view, spaces) => {
                session.publish(&view, spaces);
                false
            }
            Msg::View(view, pointer) => {
                session.pointer(&view, pointer);
                true
            }
            Msg::Window(id, WindowEvent::Size(size)) => {
                input.sizes.insert(id, size);
                false
            }
            Msg::Window(id, WindowEvent::Pointer { at, down }) => {
                let pressed = down != input.down;
                input.down = down;
                input.pointer = at.map(|at| WindowPointer { window: id, at });
                pressed
            }
            Msg::Closed(id) => {
                input.sizes.remove(&id);
                input.closed.insert(id);
                false
            }
        }
    }

    /// Every canvas's layers over views of kind `view` on `plane`. Mount it
    /// in the view where that plane is.
    #[component]
    pub fn CanvasLayers(view: String, plane: Plane) -> Element {
        let Some(canvases) = try_use_context::<Canvases>() else {
            return rsx! {};
        };
        let mut drawn = Vec::new();
        for canvas in canvases.all() {
            for layer in canvas.output().layers_on(&view, plane) {
                drawn.push((canvas, layer.clone()));
            }
        }
        rsx! {
            for (i, (canvas, layer)) in drawn.into_iter().enumerate() {
                LayerView {
                    key: "{i}-{layer.id}",
                    layer,
                    on_pointer: {
                        let view = view.clone();
                        move |p: ViewPointer| canvas.view_pointer(&view, p)
                    },
                }
            }
        }
    }

    /// Opens a floating pane in `workspace` for each window a script in
    /// context begins, closes it when the script stops drawing it, and
    /// tells the script when the person closed it.
    pub fn use_tool_windows(mut workspace: Signal<Workspace>) {
        let mut docking = use_hook(|| CopyValue::new(Docking::new()));
        let canvases = try_use_context::<Canvases>();
        use_effect(move || {
            let Some(canvases) = canvases else { return };
            let shown: Vec<(ScriptCanvas, WindowFrame)> = canvases
                .all()
                .into_iter()
                .flat_map(|c| c.output().windows.into_iter().map(move |w| (c, w)))
                .collect();
            let open: BTreeSet<String> = workspace
                .read()
                .panes
                .iter()
                .filter(|p| p.view.0 == WINDOW_VIEW)
                .map(|p| p.state.clone())
                .collect();
            let drawn: Vec<&str> = shown.iter().map(|(_, w)| w.id.as_str()).collect();
            let plan = docking.write().plan(&drawn, &open);
            for id in &plan.closed {
                if let Some((c, _)) = shown.iter().find(|(_, w)| &w.id == id) {
                    c.closed(id);
                }
            }
            for id in &plan.close {
                workspace.write().close(&window_pane(id)).ok();
            }
            // Begun: a pane floats it, sized to it, where it asked.
            for (_, w) in shown.iter().filter(|(_, w)| plan.open.contains(&w.id)) {
                let mut pane = Pane::new(window_pane(&w.id).0, WINDOW_VIEW, w.title.clone());
                pane.state.clone_from(&w.id);
                let at = w.pos.unwrap_or(Point::new(80.0, 80.0));
                let size = if w.content.w > 0.0 {
                    w.content
                } else {
                    Size::new(320.0, 160.0)
                };
                let rect = PaneRect::new(at.x, at.y, size.w.max(160.0), size.h.max(60.0));
                workspace.write().open(pane, Target::Float(rect)).ok();
            }
        });
    }
}
