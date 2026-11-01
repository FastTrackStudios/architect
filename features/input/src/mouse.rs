//! Mouse gesture patterns and per-mode binding tables.

use std::fmt;

use crate::context::{InputContext, WhenExpr};
use crate::event::{MouseAction, MouseButton, MouseEvent};
use crate::key::{KeyCode, Modifiers};

/// A mouse gesture a binding matches: `[held key +] [modifiers +]
/// button.gesture` — `Z+Left.Drag`, `Alt+Left.Drag`, `Left.DoubleClick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MousePattern {
    pub button: MouseButton,
    pub action: MouseAction,
    pub modifiers: Modifiers,
    /// A key held as a modifier for it (hold `Z` + drag): the gesture
    /// matches only while that key is held, and no other gesture does.
    pub held: Option<KeyCode>,
}

impl MousePattern {
    #[must_use]
    pub const fn new(button: MouseButton, action: MouseAction, modifiers: Modifiers) -> Self {
        Self {
            button,
            action,
            modifiers,
            held: None,
        }
    }

    /// This gesture while `key` is held.
    #[must_use]
    pub fn held(mut self, key: KeyCode) -> Self {
        self.held = Some(key);
        self
    }

    /// Whether `event` is this gesture, with no key held.
    #[must_use]
    pub fn matches(&self, event: &MouseEvent) -> bool {
        self.matches_held(event, None)
    }

    /// Whether `event` is this gesture while `held` is the key held (if
    /// any).
    #[must_use]
    pub fn matches_held(&self, event: &MouseEvent, held: Option<&KeyCode>) -> bool {
        self.button == event.button
            && self.action == event.action
            && self.modifiers == event.modifiers
            && self.held.as_ref() == held
    }
}

/// `Z+Alt+Left.Drag`: what [`crate::config::parse_mouse_pattern`] reads.
impl fmt::Display for MousePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(key) = &self.held {
            write!(f, "{}+", crate::key::KeyChord::plain(key.clone()))?;
        }
        for (held, name) in self.modifiers.named() {
            if held {
                write!(f, "{name}+")?;
            }
        }
        let button = match self.button {
            MouseButton::Left => "Left",
            MouseButton::Right => "Right",
            MouseButton::Middle => "Middle",
        };
        let action = match self.action {
            MouseAction::Press => "Press",
            MouseAction::Release => "Release",
            MouseAction::Click => "Click",
            MouseAction::DoubleClick => "DoubleClick",
            MouseAction::Drag => "Drag",
            MouseAction::LongPress => "LongPress",
        };
        write!(f, "{button}.{action}")
    }
}

/// One mode's mouse bindings, in order: the first match wins.
#[derive(Debug, Clone)]
pub struct MouseBindingTable<A = String> {
    bindings: Vec<(MousePattern, WhenExpr, A)>,
}

impl<A> Default for MouseBindingTable<A> {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }
}

impl<A: Clone> MouseBindingTable<A> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, pattern: MousePattern, when: WhenExpr, action: A) {
        self.bindings.push((pattern, when, action));
    }

    /// The raw entries, in match order.
    #[must_use]
    pub fn bindings(&self) -> &[(MousePattern, WhenExpr, A)] {
        &self.bindings
    }

    /// The action of the first entry matching `event` whose clause holds.
    #[must_use]
    pub fn match_event(&self, event: &MouseEvent, ctx: &InputContext) -> Option<A> {
        self.match_held(event, None, ctx)
    }

    /// [`match_event`](Self::match_event) while `held` is the key held.
    #[must_use]
    pub fn match_held(
        &self,
        event: &MouseEvent,
        held: Option<&KeyCode>,
        ctx: &InputContext,
    ) -> Option<A> {
        self.bindings
            .iter()
            .find(|(pattern, when, _)| pattern.matches_held(event, held) && when.evaluate(ctx))
            .map(|(_, _, action)| action.clone())
    }
}
