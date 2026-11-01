//! Keys heard from the window (feature `native`): Blitz hands a key to the
//! DOM only through what has the focus, and at first nothing has — so a
//! native app hears its chords from winit and runs them here, whatever
//! has the focus.

use dioxus::prelude::*;
use dioxus_native::winit::event::{ElementState, WindowEvent};
use dioxus_native::winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::ui::try_use_commands;

/// Hears the window's keys for the [`Commands`](crate::Commands) above:
/// a chord with ⌘ or ⌃, or Esc, F1–F12, runs its command. Plain keys stay
/// the page's (typing, a held-letter tool).
#[component]
pub fn WindowKeys() -> Element {
    let commands = try_use_commands();
    let mut held = use_signal(ModifiersState::default);
    dioxus_native::use_window_event(move |event, _| match event {
        WindowEvent::ModifiersChanged(m) => held.set(m.state()),
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
            let Some(commands) = commands else {
                return;
            };
            let m = *held.peek();
            let key = match &event.logical_key {
                Key::Character(c) if c == " " => "Space".to_owned(),
                Key::Character(c) => c.to_uppercase(),
                Key::Named(NamedKey::Tab) => "Tab".to_owned(),
                Key::Named(NamedKey::Enter) => "↵".to_owned(),
                Key::Named(NamedKey::Escape) => "Esc".to_owned(),
                Key::Named(NamedKey::Backspace) => "⌫".to_owned(),
                Key::Named(NamedKey::ArrowLeft) => "←".to_owned(),
                Key::Named(NamedKey::ArrowRight) => "→".to_owned(),
                Key::Named(NamedKey::ArrowUp) => "↑".to_owned(),
                Key::Named(NamedKey::ArrowDown) => "↓".to_owned(),
                _ => return,
            };
            let chorded = m.control_key() || m.meta_key();
            // Arrows and ↵ in the palette are its field's; plain letters
            // are the page's.
            if !chorded && key != "Esc" {
                return;
            }
            let shifted_char = matches!(&event.logical_key, Key::Character(c) if !c.chars().all(char::is_alphabetic));
            let mut chord = String::new();
            if m.control_key() {
                chord.push('⌃');
            }
            if m.alt_key() {
                chord.push('⌥');
            }
            if m.shift_key() && !shifted_char {
                chord.push('⇧');
            }
            if m.meta_key() {
                chord.push('⌘');
            }
            chord.push_str(&key);
            let _ = commands.handle_chord(&chord);
        }
        _ => {}
    });
    rsx! {}
}
