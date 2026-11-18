//! Whether a text field has the keyboard focus anywhere in the app: a
//! linked view's (Dioxus's own count, feature `paint-ui`) or a field in a
//! document the app hosts itself (a process view's), which says so
//! through a [`FieldFocus`].
//!
//! A pane's view keys (`[[keys]] when = "view:…"`) ask it first: a plain
//! letter typed into a field is the field's, not a binding's.

use std::sync::atomic::{AtomicUsize, Ordering};

static HELD: AtomicUsize = AtomicUsize::new(0);

/// Whether a text field has the focus now.
#[must_use]
pub fn field_focused() -> bool {
    #[cfg(feature = "paint-ui")]
    if dioxus_native_dom::text_input_focused() {
        return true;
    }
    HELD.load(Ordering::Relaxed) > 0
}

/// One hosted document's share of [`field_focused`]: set it each time
/// its focus may have moved; dropped, it lets go.
#[derive(Debug, Default)]
pub struct FieldFocus {
    held: bool,
}

impl FieldFocus {
    #[must_use]
    pub const fn new() -> Self {
        Self { held: false }
    }

    /// Whether a text field in its document has the focus now.
    pub fn set(&mut self, now: bool) {
        if now == self.held {
            return;
        }
        self.held = now;
        if now {
            HELD.fetch_add(1, Ordering::Relaxed);
        } else {
            HELD.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for FieldFocus {
    fn drop(&mut self) {
        self.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_counts_while_its_field_has_the_focus() {
        let base = HELD.load(Ordering::Relaxed);
        let mut a = FieldFocus::new();
        a.set(true);
        a.set(true);
        assert_eq!(HELD.load(Ordering::Relaxed), base + 1);
        {
            let mut b = FieldFocus::new();
            b.set(true);
            assert_eq!(HELD.load(Ordering::Relaxed), base + 2);
        }
        assert_eq!(HELD.load(Ordering::Relaxed), base + 1);
        a.set(false);
        assert_eq!(HELD.load(Ordering::Relaxed), base);
    }
}
