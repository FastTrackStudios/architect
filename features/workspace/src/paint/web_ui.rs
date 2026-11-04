//! Dioxus canvas host for a browser paint view.
//!
//! Input and browser animation frames stay on the browser thread. Keep one
//! [`WebPaintMount`] per pane, and use its wake after host data changes.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{HtmlCanvasElement, KeyboardEvent, PointerEvent, WheelEvent};

use super::{Painter, web::WebPaintSurface};
use crate::{Button, Geometry, ThemeTokens, ViewInput};

type RuntimeRef = Rc<RefCell<Option<Runtime>>>;

struct Runtime {
    canvas: HtmlCanvasElement,
    make: Rc<dyn Fn() -> Painter>,
    surface: Option<WebPaintSurface>,
}

struct WindowResize {
    window: web_sys::Window,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for WindowResize {
    fn drop(&mut self) {
        let _ = self
            .window
            .remove_event_listener_with_callback("resize", self.callback.as_ref().unchecked_ref());
    }
}

fn watch_window_resize(core: &Rc<WakeCore>) -> Option<WindowResize> {
    let window = web_sys::window()?;
    let weak = Rc::downgrade(core);
    let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
        if let Some(core) = weak.upgrade() {
            resize(&core);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    window
        .add_event_listener_with_callback("resize", callback.as_ref().unchecked_ref())
        .ok()?;
    Some(WindowResize { window, callback })
}

#[derive(Default)]
struct WakeCore {
    runtime: RefCell<Weak<RefCell<Option<Runtime>>>>,
    pending: Cell<bool>,
    scheduled: Cell<bool>,
    theme: RefCell<Option<ThemeTokens>>,
    error: RefCell<Option<Signal<Option<String>>>>,
}

/// A cloneable, browser-thread wake for one paint pane.
#[derive(Clone, Default)]
pub struct WebPaintWake(Rc<WakeCore>);

impl WebPaintWake {
    /// Mark external data dirty and coalesce a browser animation frame.
    pub fn wake(&self) {
        self.0.pending.set(true);
        schedule(&self.0);
    }
}

/// A painter factory and wake handle for one web pane.
#[derive(Clone)]
pub struct WebPaintMount {
    make: Rc<dyn Fn() -> Painter>,
    wake: WebPaintWake,
}

impl PartialEq for WebPaintMount {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.make, &other.make) && Rc::ptr_eq(&self.wake.0, &other.wake.0)
    }
}

impl WebPaintMount {
    /// Create a mount. The factory supplies the pane's `ViewCtx` and view.
    pub fn new(make: impl Fn() -> Painter + 'static) -> Self {
        Self {
            make: Rc::new(make),
            wake: WebPaintWake::default(),
        }
    }

    /// The handle for requesting a frame after host data changes.
    #[must_use]
    pub fn wake_handle(&self) -> WebPaintWake {
        self.wake.clone()
    }
}

fn report(core: &WakeCore, message: String) {
    if let Some(mut error) = *core.error.borrow() {
        error.set(Some(message));
    }
}

fn schedule(core: &Rc<WakeCore>) {
    if core.scheduled.replace(true) {
        return;
    }
    let Some(window) = web_sys::window() else {
        core.scheduled.set(false);
        report(core, "No browser window is available".to_owned());
        return;
    };
    let weak = Rc::downgrade(core);
    let callback = Closure::once_into_js(move |_: f64| {
        if let Some(core) = weak.upgrade() {
            frame(&core);
        }
    });
    if window
        .request_animation_frame(callback.unchecked_ref())
        .is_err()
    {
        core.scheduled.set(false);
        report(core, "Could not request an animation frame".to_owned());
    }
}

fn frame(core: &Rc<WakeCore>) {
    core.scheduled.set(false);
    let Some(runtime) = core.runtime.borrow().upgrade() else {
        return;
    };
    let mut slot = runtime.borrow_mut();
    let Some(runtime) = slot.as_mut() else { return };
    let size = geometry(&runtime.canvas);
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }
    let Some(surface) = runtime.surface.as_mut() else {
        return;
    };
    if core.pending.replace(false) {
        surface.invalidate();
    }
    if let Some(theme) = core.theme.borrow().as_ref() {
        surface.set_theme(theme.clone());
    }
    if let Err(error) = surface.draw() {
        report(core, error.to_string());
        return;
    }
    let again = surface.wants_frame() || core.pending.get();
    drop(slot);
    if again {
        schedule(core);
    }
}

fn geometry(canvas: &HtmlCanvasElement) -> Geometry {
    let rect = canvas.get_bounding_client_rect();
    let scale = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio());
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    Geometry {
        width: rect.width() as f32,
        height: rect.height() as f32,
        scale: scale as f32,
    }
}

fn resize(core: &Rc<WakeCore>) {
    let Some(runtime) = core.runtime.borrow().upgrade() else {
        return;
    };
    let mut slot = runtime.borrow_mut();
    let Some(runtime) = slot.as_mut() else { return };
    let size = geometry(&runtime.canvas);
    // Hidden or not laid out yet: wait for a nonzero resize instead of failing.
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }
    let result = if let Some(surface) = runtime.surface.as_mut() {
        surface.resize(size)
    } else {
        WebPaintSurface::new(runtime.canvas.clone(), (runtime.make)(), size).map(|surface| {
            runtime.surface = Some(surface);
            core.pending.set(true);
        })
    };
    if let Err(error) = result {
        report(core, error.to_string());
        return;
    }
    drop(slot);
    schedule(core);
}

fn input(core: &Rc<WakeCore>, event: ViewInput) {
    let Some(runtime) = core.runtime.borrow().upgrade() else {
        return;
    };
    if let Some(surface) = runtime
        .borrow()
        .as_ref()
        .and_then(|runtime| runtime.surface.as_ref())
    {
        surface.input(event);
    }
    schedule(core);
}

fn pointer(core: &Rc<WakeCore>, event: PointerEvent, phase: &'static str) {
    let Some(runtime) = core.runtime.borrow().upgrade() else {
        return;
    };
    let slot = runtime.borrow();
    let Some(runtime) = slot.as_ref() else { return };
    let rect = runtime.canvas.get_bounding_client_rect();
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    let (x, y) = (
        (f64::from(event.client_x()) - rect.left()) as f32,
        (f64::from(event.client_y()) - rect.top()) as f32,
    );
    let button = match event.button() {
        1 => Button::Middle,
        2 => Button::Secondary,
        _ => Button::Primary,
    };
    let view = if event.pointer_type() == "touch" {
        ViewInput::Touch {
            id: u64::try_from(event.pointer_id()).unwrap_or_default(),
            x,
            y,
            phase,
        }
    } else {
        match phase {
            "start" => ViewInput::PointerDown { x, y, button },
            "end" => ViewInput::PointerUp { x, y, button },
            _ => ViewInput::PointerMove { x, y },
        }
    };
    if phase == "start" {
        let _ = runtime.canvas.focus();
        let _ = runtime.canvas.set_pointer_capture(event.pointer_id());
    }
    drop(slot);
    input(core, view);
}

fn wheel(core: &Rc<WakeCore>, event: WheelEvent) {
    let unit = match event.delta_mode() {
        1 => 20.0,
        2 => 800.0,
        _ => 1.0,
    };
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    input(
        core,
        ViewInput::Wheel {
            dx: (event.delta_x() * unit) as f32,
            dy: (event.delta_y() * unit) as f32,
        },
    );
}

fn key(core: &Rc<WakeCore>, event: KeyboardEvent, down: bool) {
    let mut modifiers = Vec::new();
    if event.ctrl_key() || event.meta_key() {
        modifiers.push("Mod");
    }
    if event.shift_key() {
        modifiers.push("Shift");
    }
    if event.alt_key() {
        modifiers.push("Alt");
    }
    if event.ctrl_key() {
        modifiers.push("Ctrl");
    }
    input(
        core,
        ViewInput::Key {
            key: event.key(),
            down,
            modifiers,
        },
    );
}

/// A Dioxus WebGL2 canvas. `theme` is applied live without remounting.
///
/// Place it in a sized parent; it fills the available width and height. The
/// canvas accepts focus, pointer, wheel, keyboard, composition and touch
/// input. On failure an alert is shown. A wake handle is browser-thread only.
#[component]
pub fn WebPaintPane(mount: WebPaintMount, #[props(default)] theme: ThemeTokens) -> Element {
    let core = mount.wake.0.clone();
    let runtime: RuntimeRef = use_hook(|| Rc::new(RefCell::new(None)));
    let _window_resize = use_hook(|| Rc::new(watch_window_resize(&core)));
    let error = use_signal(|| None::<String>);
    *core.error.borrow_mut() = Some(error);
    if core.theme.borrow().as_ref() != Some(&theme) {
        *core.theme.borrow_mut() = Some(theme);
        mount.wake.wake();
    }
    let teardown = core.clone();
    use_drop(move || {
        *teardown.runtime.borrow_mut() = Weak::new();
        *teardown.error.borrow_mut() = None;
    });
    let mounted_core = core.clone();
    let mounted_runtime = runtime.clone();
    let make = mount.make.clone();
    rsx! {
        div { style: "position:relative;width:100%;height:100%;min-width:0;min-height:0;",
            canvas {
                style: "display:block;width:100%;height:100%;touch-action:none;",
                tabindex: "0",
                onmounted: move |event| {
                    let Some(element) = event.data().try_as_web_event() else { report(&mounted_core, "Canvas is unavailable".into()); return };
                    let Ok(canvas) = element.dyn_into::<HtmlCanvasElement>() else { report(&mounted_core, "Mounted element is not a canvas".into()); return };
                    *mounted_runtime.borrow_mut() = Some(Runtime { canvas, make: make.clone(), surface: None });
                    *mounted_core.runtime.borrow_mut() = Rc::downgrade(&mounted_runtime);
                    resize(&mounted_core);
                },
                onresize: { let core = core.clone(); move |_| resize(&core) },
                onpointerdown: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { pointer(&core, event, "start"); } },
                onpointermove: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { pointer(&core, event, "move"); } },
                onpointerup: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { pointer(&core, event, "end"); } },
                onpointercancel: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { pointer(&core, event, "cancel"); } },
                onwheel: { let core = core.clone(); move |e| { e.prevent_default(); if let Some(event) = e.data().try_as_web_event() { wheel(&core, event); } } },
                onkeydown: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { key(&core, event, true); } },
                onkeyup: { let core = core.clone(); move |e| if let Some(event) = e.data().try_as_web_event() { key(&core, event, false); } },
                onfocus: { let core = core.clone(); move |_| input(&core, ViewInput::Focus(true)) },
                onblur: { let core = core.clone(); move |_| input(&core, ViewInput::Focus(false)) },
                oncompositionupdate: { let core = core.clone(); move |e| input(&core, ViewInput::Ime { preedit: e.data().data(), commit: None }) },
                oncompositionend: { let core = core.clone(); move |e| input(&core, ViewInput::Ime { preedit: String::new(), commit: Some(e.data().data()) }) },
            }
            if let Some(message) = error() { div { role: "alert", "{message}" } }
        }
    }
}
