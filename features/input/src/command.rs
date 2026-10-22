//! What the processor asks the app to do.
//!
//! `A` is the app's action id: by default a `String` naming an Action
//! List action (`architect::ActionList::run_json`), but any type built
//! from the id strings in a keymap works.

use crate::event::InputEvent;
use crate::mode::ModeId;

/// A command produced by [`crate::InputProcessor`] for one input event.
#[derive(Debug, Clone, PartialEq)]
pub enum InputCommand<A = String> {
    /// Run an action.
    Action(A),
    /// Run an action with composed arguments (a count, an operator and
    /// its motion or text object).
    ActionWithArgs { action: A, args: InputArgs },
    /// The base mode changed.
    SwitchMode(ModeId),
    /// A sub-mode was pushed.
    PushMode(ModeId),
    /// The top sub-mode was popped.
    PopMode,
    /// Insert literal text (a passthrough-text mode such as insert).
    InsertText(String),
    /// No binding took the event; the app may handle it itself.
    Unhandled(InputEvent),
    /// Keys are buffered waiting for more; `display` is what was typed.
    Pending { display: String },
}

impl<A> InputCommand<A> {
    /// The action this command runs, if it runs one.
    #[must_use]
    pub const fn action(&self) -> Option<&A> {
        match self {
            Self::Action(action) | Self::ActionWithArgs { action, .. } => Some(action),
            _ => None,
        }
    }
}

/// Arguments composed for an action by vim-style input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputArgs {
    /// A count typed before the key (`3j`).
    pub count: Option<u32>,
    /// The pending operator (`d` in `dw`).
    pub operator: Option<String>,
    /// The motion that completed the operator (`w` in `dw`).
    pub motion: Option<String>,
    /// The text object that completed the operator (`iw` in `diw`).
    pub text_object: Option<String>,
    /// A register (`"a`).
    pub register: Option<char>,
}

/// What an action id must be to drive the processor.
///
/// It is cloned into commands, compared in tests, and built from the
/// action strings of a [`crate::KeymapConfig`] (and the `operator.<op>`
/// ids of operator composition).
///
/// `String` is the default and what an Action List id is. An app with a
/// typed id implements `From<String>` for it (mapping unknown ids to an
/// "unbound" variant rather than failing).
pub trait ActionKey: Clone + PartialEq + std::fmt::Debug + From<String> {}

impl<T: Clone + PartialEq + std::fmt::Debug + From<String>> ActionKey for T {}
