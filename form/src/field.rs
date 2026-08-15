//! [`Field`] — one typed, validated form field.
//!
//! The baseline Dioxus form is *uncontrolled*: you read `name`-keyed
//! strings off the `FormEvent` with `evt.parsed_values()` on submit, with
//! no per-field validation, error, or touched/dirty state. `Field` is the
//! *controlled* improvement: it owns a string-encoded `value` plus a
//! **parser that both validates and decodes** into a typed `T` (effect-form
//! uses an Effect `Schema` for this; Rust has no runtime schema, so the
//! parser closure plays that role), and tracks the validation `error`,
//! whether the field has been `touched`, and whether it's `dirty`.

use std::rc::Rc;

use dioxus::prelude::*;

/// A parser that validates a raw string and decodes it into `T`, returning
/// a human-readable message on failure. This is the field's "schema".
pub type Parser<T> = Rc<dyn Fn(&str) -> Result<T, String>>;

/// One form field. `Copy` (it wraps `Signal`s + a `CopyValue`), so it
/// moves into event handlers freely.
pub struct Field<T: 'static> {
    value: Signal<String>,
    initial: Signal<String>,
    error: Signal<Option<String>>,
    touched: Signal<bool>,
    parser: CopyValue<Parser<T>>,
}

impl<T: 'static> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Field<T> {}

// Identity comparison (do these handles point at the same field state?),
// so a `Field` can be a component prop — reactivity flows through the
// signals read *inside* the component, not through prop diffing.
impl<T: 'static> PartialEq for Field<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

/// Create a field with an initial string value and a parser/validator.
///
/// ```ignore
/// let name = use_field("", validate::required("Name"));
/// let age  = use_field("", |s: &str| s.parse::<u8>().map_err(|_| "must be a number".into()));
/// ```
pub fn use_field<T: 'static>(
    initial: impl Into<String>,
    parser: impl Fn(&str) -> Result<T, String> + 'static,
) -> Field<T> {
    let initial = initial.into();
    let value = use_signal({
        let initial = initial.clone();
        move || initial.clone()
    });
    let initial_sig = use_signal(move || initial.clone());
    let error = use_signal(|| None);
    let touched = use_signal(|| false);
    let parser = use_hook(|| CopyValue::new(Rc::new(parser) as Parser<T>));
    Field {
        value,
        initial: initial_sig,
        error,
        touched,
        parser,
    }
}

impl<T: 'static> Field<T> {
    /// The current raw (string-encoded) value. Bind this to the input's
    /// `value` attribute.
    pub fn value(&self) -> String {
        self.value.read().clone()
    }

    /// Update the value (an `oninput` handler). Re-validates live once the
    /// field has been touched, so an error clears as the user fixes it.
    pub fn set(&self, value: impl Into<String>) {
        let mut v = self.value;
        v.set(value.into());
        if *self.touched.read() {
            let _ = self.validate();
        }
    }

    /// Mark the field touched and validate (an `onblur` handler).
    pub fn blur(&self) {
        let mut touched = self.touched;
        touched.set(true);
        let _ = self.validate();
    }

    /// Run the parser, store the error (or clear it), and return the parsed
    /// value. Use the returned `Result` when collecting the whole form.
    pub fn validate(&self) -> Result<T, String> {
        let raw = self.value.peek().clone();
        let outcome = (self.parser.read())(&raw);
        let mut error = self.error;
        match &outcome {
            Ok(_) => error.set(None),
            Err(e) => error.set(Some(e.clone())),
        }
        outcome
    }

    /// The parsed value without touching error state — for reading a valid
    /// form's values.
    pub fn parsed(&self) -> Result<T, String> {
        (self.parser.read())(&self.value.peek())
    }

    /// The error to display: shown only once the field is touched (or
    /// after [`show_error`](Self::show_error) on a submit attempt), so a
    /// pristine field doesn't shout at the user.
    pub fn error(&self) -> Option<String> {
        if *self.touched.read() {
            self.error.read().clone()
        } else {
            None
        }
    }

    /// True once the value differs from its initial value.
    pub fn is_dirty(&self) -> bool {
        *self.value.read() != *self.initial.read()
    }

    /// True once the field has been blurred or force-shown.
    pub fn is_touched(&self) -> bool {
        *self.touched.read()
    }

    /// True if the current value parses cleanly (does not mutate state).
    pub fn is_valid(&self) -> bool {
        self.parsed().is_ok()
    }

    /// Reveal the field's error (mark touched + validate) — call on a
    /// submit attempt so a never-touched invalid field surfaces its error.
    pub fn show_error(&self) {
        let mut touched = self.touched;
        touched.set(true);
        let _ = self.validate();
    }

    /// Reset to the initial value and clear error/touched state.
    pub fn reset(&self) {
        let mut value = self.value;
        let mut error = self.error;
        let mut touched = self.touched;
        value.set(self.initial.peek().clone());
        error.set(None);
        touched.set(false);
    }
}
