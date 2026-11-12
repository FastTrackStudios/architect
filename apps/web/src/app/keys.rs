//! The page's chords (⌘K, ⌘O, ⌘,, Esc…), whatever has the focus: what
//! `architect_commands::WindowKeys` hears from winit on a desk, heard here
//! from the document's `keydown`.
//!
//! A chord with ⌘ or ⌃ (off Apple platforms Ctrl is tried as ⌘ too:
//! `crate::keys::chords`), or Esc, runs its command and the browser's own
//! action for it is stopped (⌘O does not open a file, ⌘K stays the app's).
//! Plain keys stay the page's (typing). What the browser keeps for itself
//! (⌘W, ⌘T, ⌘N in most) never reaches the page.

use architect_commands::try_use_commands;
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::KeyboardEvent;

/// Whether this browser is on an Apple platform: there ⌘ is Meta, and
/// Ctrl is Control.
fn apple() -> bool {
    let Some(nav) = web_sys::window().map(|w| w.navigator()) else {
        return false;
    };
    let data = js_sys::Reflect::get(&nav, &JsValue::from_str("userAgentData"))
        .ok()
        .filter(JsValue::is_object)
        .and_then(|d| js_sys::Reflect::get(&d, &JsValue::from_str("platform")).ok())
        .and_then(|p| p.as_string());
    let platform = data.or_else(|| nav.platform().ok()).unwrap_or_default();
    crate::keys::is_apple(&platform)
}

/// The chords `event` is tried as (`crate::keys::chords`).
fn chords(event: &KeyboardEvent, apple: bool) -> Vec<String> {
    let key = event.key();
    crate::keys::chords(
        crate::keys::Press {
            key: &key,
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            shift: event.shift_key(),
            meta: event.meta_key(),
        },
        apple,
    )
}

/// Hears the document's keys for the commands above.
#[component]
pub fn WebKeys() -> Element {
    let commands = try_use_commands();
    use_hook(move || {
        let Some(commands) = commands else { return };
        let apple = apple();
        let on_key = Closure::<dyn Fn(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            if event.is_composing() {
                return;
            }
            if chords(&event, apple)
                .iter()
                .any(|c| commands.handle_chord(c))
            {
                event.prevent_default();
            }
        });
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            let _ =
                doc.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref());
        }
        on_key.forget();
    });
    rsx! {}
}
