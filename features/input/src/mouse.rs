//! Mouse gesture patterns and per-mode binding tables.

use crate::context::{InputContext, WhenExpr};
use crate::event::{MouseAction, MouseButton, MouseEvent};
use crate::key::Modifiers;

/// A mouse gesture a binding matches: button, what it did, modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MousePattern {
    pub button: MouseButton,
    pub action: MouseAction,
    pub modifiers: Modifiers,
}

impl MousePattern {
    #[must_use]
    pub const fn new(button: MouseButton, action: MouseAction, modifiers: Modifiers) -> Self {
        Self {
            button,
            action,
            modifiers,
        }
    }

    /// Whether `event` is this gesture.
    #[must_use]
    pub fn matches(&self, event: &MouseEvent) -> bool {
        self.button == event.button
            && self.action == event.action
            && self.modifiers == event.modifiers
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
        self.bindings
            .iter()
            .find(|(pattern, when, _)| pattern.matches(event) && when.evaluate(ctx))
            .map(|(_, _, action)| action.clone())
    }
}
