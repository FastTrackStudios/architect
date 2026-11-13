//! The window as the shell lays out for it: its size, whether the screen
//! is the pointer, and the safe areas (`env(safe-area-inset-*)`, a phone's
//! notch and home bar), kept as the page resizes or turns.

use architect_shell::{Insets, Screen};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The id of the probe element whose padding reads the safe areas.
const PROBE: &str = "architect-safe-area";

fn measure() -> Screen {
    let Some(w) = web_sys::window() else {
        return Screen {
            width: 1280.0,
            height: 800.0,
            touch: false,
            insets: Insets::default(),
        };
    };
    let px = |v: Result<JsValue, JsValue>| v.ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
    let touch = w
        .match_media("(pointer: coarse)")
        .ok()
        .flatten()
        .is_some_and(|m| m.matches());
    Screen {
        width: px(w.inner_width()),
        height: px(w.inner_height()),
        touch,
        insets: insets(&w),
    }
}

/// The page's safe areas, read from a hidden element padded with them.
fn insets(w: &web_sys::Window) -> Insets {
    let Some(doc) = w.document() else {
        return Insets::default();
    };
    let probe = doc.get_element_by_id(PROBE).or_else(|| {
        let el = doc.create_element("div").ok()?;
        el.set_id(PROBE);
        let _ = el.set_attribute(
            "style",
            "position: fixed; visibility: hidden; pointer-events: none; \
             padding: env(safe-area-inset-top) env(safe-area-inset-right) \
             env(safe-area-inset-bottom) env(safe-area-inset-left);",
        );
        doc.body()?.append_child(&el).ok()?;
        Some(el)
    });
    let Some(style) = probe.and_then(|p| w.get_computed_style(&p).ok().flatten()) else {
        return Insets::default();
    };
    let side = |name: &str| {
        style
            .get_property_value(name)
            .ok()
            .and_then(|v| v.trim_end_matches("px").parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    Insets {
        top: side("padding-top"),
        right: side("padding-right"),
        bottom: side("padding-bottom"),
        left: side("padding-left"),
    }
}

/// The window, published as the `Signal<Screen>` the shell reads (what
/// `use_window_screen` is on a desk), kept on every resize.
pub fn use_browser_screen() -> Signal<Screen> {
    let screen = use_context_provider(|| Signal::new(measure()));
    use_hook(move || {
        let on_resize = Closure::<dyn Fn()>::new(move || {
            let mut screen = screen;
            let now = measure();
            if *screen.peek() != now {
                screen.set(now);
            }
        });
        if let Some(w) = web_sys::window() {
            let f = on_resize.as_ref().unchecked_ref();
            let _ = w.add_event_listener_with_callback("resize", f);
            let _ = w.add_event_listener_with_callback("orientationchange", f);
        }
        // The page's listener for its life.
        on_resize.forget();
    });
    screen
}
