//! Dioxus DOM events as input events.

use crate::{
    InputEvent, KeyChord, KeyCode, KeyEvent, Modifiers, MouseAction, MouseButton, ScrollEvent,
};
use dioxus::prelude::{
    InteractionLocation, Key, KeyboardEvent, Modifiers as DioxusModifiers, ModifiersInteraction,
    MouseEvent, NamedKey, PointerInteraction, WheelEvent,
};

/// Convert a Dioxus key value to an input key code.
#[must_use]
pub fn convert_key(key: &Key) -> Option<KeyCode> {
    let Key::Named(named) = key else {
        let Key::Character(c) = key else { return None };
        return Some(KeyCode::Character(c.to_lowercase()));
    };
    match named {
        NamedKey::ArrowUp => Some(KeyCode::ArrowUp),
        NamedKey::ArrowDown => Some(KeyCode::ArrowDown),
        NamedKey::ArrowLeft => Some(KeyCode::ArrowLeft),
        NamedKey::ArrowRight => Some(KeyCode::ArrowRight),
        NamedKey::Enter => Some(KeyCode::Enter),
        NamedKey::Escape => Some(KeyCode::Escape),
        NamedKey::Tab => Some(KeyCode::Tab),
        NamedKey::Backspace => Some(KeyCode::Backspace),
        NamedKey::Delete => Some(KeyCode::Delete),
        NamedKey::F1 => Some(KeyCode::F(1)),
        NamedKey::F2 => Some(KeyCode::F(2)),
        NamedKey::F3 => Some(KeyCode::F(3)),
        NamedKey::F4 => Some(KeyCode::F(4)),
        NamedKey::F5 => Some(KeyCode::F(5)),
        NamedKey::F6 => Some(KeyCode::F(6)),
        NamedKey::F7 => Some(KeyCode::F(7)),
        NamedKey::F8 => Some(KeyCode::F(8)),
        NamedKey::F9 => Some(KeyCode::F(9)),
        NamedKey::F10 => Some(KeyCode::F(10)),
        NamedKey::F11 => Some(KeyCode::F(11)),
        NamedKey::F12 => Some(KeyCode::F(12)),
        _ => None,
    }
}

/// Convert Dioxus modifiers to input modifiers.
#[must_use]
pub fn convert_modifiers(m: &DioxusModifiers) -> Modifiers {
    Modifiers {
        ctrl: m.ctrl(),
        alt: m.alt(),
        shift: m.shift(),
        meta: m.meta(),
    }
}

/// Convert a Dioxus keyboard event to an input keyboard event.
///
/// Returns `None` for unrecognized keys.
#[must_use]
pub fn convert_keyboard_event(e: &KeyboardEvent) -> Option<InputEvent> {
    let key = convert_key(&e.key())?;
    Some(InputEvent::Key(KeyEvent {
        key,
        modifiers: convert_modifiers(&e.modifiers()),
    }))
}

/// The chord of a Dioxus keyboard event (a keydown or a keyup).
///
/// Returns `None` for unrecognized keys.
#[must_use]
pub fn keyboard_chord(e: &KeyboardEvent) -> Option<KeyChord> {
    Some(KeyChord::new(
        convert_key(&e.key())?,
        convert_modifiers(&e.modifiers()),
    ))
}

/// Convert a Dioxus mouse event to an input mouse event with explicit action.
#[must_use]
pub fn convert_mouse_event(e: &MouseEvent, action: MouseAction) -> InputEvent {
    let coords = e.client_coordinates();
    let button =
        e.trigger_button()
            .map_or(MouseButton::Left, |b| match format!("{b:?}").as_str() {
                "Secondary" => MouseButton::Right,
                "Auxiliary" | "Middle" => MouseButton::Middle,
                _ => MouseButton::Left,
            });

    InputEvent::Mouse(crate::MouseEvent {
        button,
        action,
        x: coords.x,
        y: coords.y,
        modifiers: convert_modifiers(&e.modifiers()),
        from: None,
    })
}

/// Convert a Dioxus wheel event to an input scroll event.
#[must_use]
pub fn convert_wheel_event(e: &WheelEvent) -> InputEvent {
    let delta = e.delta().strip_units();
    InputEvent::Scroll(ScrollEvent {
        delta_x: delta.x,
        delta_y: delta.y,
        modifiers: convert_modifiers(&e.modifiers()),
    })
}
