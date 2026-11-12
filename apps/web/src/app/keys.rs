//! The page's chords (⌘K, ⌘O, ⌘,, Esc…), whatever has the focus: what
//! `architect_commands::WindowKeys` hears from winit on a desk, heard here
//! from the document's `keydown`.
//!
//! A chord with ⌘ or ⌃, or Esc, runs its command and the browser's own
//! action for it is stopped (⌘O does not open a file, ⌘K stays the app's).
//! Plain keys stay the page's (typing). What the browser keeps for itself
//! (⌘W, ⌘T, ⌘N in most) never reaches the page.

use architect_commands::try_use_commands;
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::KeyboardEvent;

/// `event` as a chord the commands' keymap reads (`⇧⌘K`), or `None` for
/// a key that stays the page's.
fn chord(event: &KeyboardEvent) -> Option<String> {
    let key = event.key();
    let named = match key.as_str() {
        " " => "Space".to_owned(),
        "Tab" => "Tab".to_owned(),
        "Enter" => "↵".to_owned(),
        "Escape" => "Esc".to_owned(),
        "Backspace" => "⌫".to_owned(),
        "ArrowLeft" => "←".to_owned(),
        "ArrowRight" => "→".to_owned(),
        "ArrowUp" => "↑".to_owned(),
        "ArrowDown" => "↓".to_owned(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k if k.starts_with('F') && k.len() <= 3 => k.to_owned(),
        _ => return None,
    };
    let chorded = event.ctrl_key() || event.meta_key();
    if !chorded && named != "Esc" {
        return None;
    }
    // ⇧ is in the character itself for a symbol (`?`), as on a desk.
    let shifted_char = key.chars().count() == 1 && !key.chars().all(char::is_alphabetic);
    let mut out = String::new();
    if event.ctrl_key() {
        out.push('⌃');
    }
    if event.alt_key() {
        out.push('⌥');
    }
    if event.shift_key() && !shifted_char {
        out.push('⇧');
    }
    if event.meta_key() {
        out.push('⌘');
    }
    out.push_str(&named);
    Some(out)
}

/// Hears the document's keys for the commands above.
#[component]
pub fn WebKeys() -> Element {
    let commands = try_use_commands();
    use_hook(move || {
        let Some(commands) = commands else { return };
        let on_key = Closure::<dyn Fn(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            if event.is_composing() {
                return;
            }
            if let Some(c) = chord(&event)
                && commands.handle_chord(&c)
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
