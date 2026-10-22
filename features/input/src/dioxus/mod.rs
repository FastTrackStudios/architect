//! Dioxus bindings (feature `dioxus`): DOM events in, a reactive handle on
//! an [`crate::InputProcessor`] out.
//!
//! The which-key popup is not rendered here: [`crate::InputProcessor::which_key`]
//! gives the model and the app draws it in its own design system.

pub mod convert;
pub mod hook;

pub use convert::{
    convert_key, convert_keyboard_event, convert_modifiers, convert_mouse_event,
    convert_wheel_event,
};
pub use hook::{InputHandle, use_input_processor};
