//! `architect-form` — typed, validated form state for Dioxus.
//!
//! Modelled on [effect-form](https://github.com/lucas-barake/effect-form),
//! and the form-shaped sibling of [`architect_atom`]: the *controlled*
//! improvement over the baseline Dioxus form (uncontrolled inputs read with
//! `evt.parsed_values()` on submit, no per-field validation/error state).
//!
//! - [`Field`] / [`use_field`] — one field: a string-encoded value plus a
//!   **parser that validates and decodes** into a typed `T`, with
//!   error / touched / dirty tracking and live re-validation once touched.
//! - [`validate`] — validator combinators (`required`, `parse`,
//!   `min_length`, `email`, `and`, …) to build a field's parser from.
//! - Ready-made field components — [`TextField`], [`TextArea`],
//!   [`SelectField`], [`Checkbox`] — label + control + inline error over
//!   any `Field`.
//!
//! Most forms aren't written by hand at all: `#[architect(form)]` on an
//! entity derives `<E>CreateFields` / `<E>UpdateFields` whose `submit()`
//! returns the typed wire payload, ready for the derived optimistic
//! mutations. These primitives are the floor for the forms that don't
//! mirror a payload (login, search, settings). Forms submit through the
//! app's vox client (architect's idiom), never a Dioxus server function —
//! see `docs/content/architecture/forms.md`.

mod components;
mod field;
pub mod validate;

pub use components::{Checkbox, SelectField, TextArea, TextField};
pub use field::{Field, Parser, use_field};
