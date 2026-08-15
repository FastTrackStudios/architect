//! Ready-made field components — the rendering half of a [`Field`].
//!
//! A [`Field`](crate::Field) owns the state (value, validation, error,
//! touched); [`TextField`] is the standard way to render one: label +
//! controlled input + inline error, wired to `set` / `blur`. Generic over
//! the decoded type, so a `Field<u32>` renders exactly like a
//! `Field<String>` — the parser is the difference.
//!
//! ```ignore
//! let fields = use_example_create_fields();          // derive-generated
//! rsx! {
//!     TextField { field: fields.name, label: "Name" }
//!     TextField { field: fields.description, label: "Description", placeholder: "optional" }
//! }
//! ```
//!
//! Apps with their own design system write their own equivalent — the
//! `Field` surface (`value`/`set`/`blur`/`error`) is the contract.

use dioxus::prelude::*;

use crate::field::Field;

/// Label + controlled text input + inline validation error for one
/// [`Field`].
#[component]
pub fn TextField<T: 'static>(
    field: Field<T>,
    label: String,
    #[props(default)] placeholder: String,
) -> Element {
    rsx! {
        label { class: "form-field",
            "{label}"
            input {
                placeholder,
                value: field.value(),
                oninput: move |evt| field.set(evt.value()),
                onblur: move |_| field.blur(),
            }
            if let Some(err) = field.error() {
                span { class: "field-error", "{err}" }
            }
        }
    }
}

/// Label + controlled multi-line textarea + inline error for one
/// [`Field`].
#[component]
pub fn TextArea<T: 'static>(
    field: Field<T>,
    label: String,
    #[props(default)] placeholder: String,
    #[props(default = 4)] rows: u32,
) -> Element {
    rsx! {
        label { class: "form-field",
            "{label}"
            textarea {
                placeholder,
                rows: "{rows}",
                value: field.value(),
                oninput: move |evt| field.set(evt.value()),
                onblur: move |_| field.blur(),
            }
            if let Some(err) = field.error() {
                span { class: "field-error", "{err}" }
            }
        }
    }
}

/// Label + `<select>` + inline error. `options` are `(value, display)`
/// pairs; the field's parser validates/decodes the selected value (use
/// `validate::parse::<YourEnum>` with a `FromStr` impl for typed enums).
#[component]
pub fn SelectField<T: 'static>(
    field: Field<T>,
    label: String,
    options: Vec<(String, String)>,
) -> Element {
    rsx! {
        label { class: "form-field",
            "{label}"
            select {
                value: field.value(),
                onchange: move |evt| {
                    field.set(evt.value());
                    field.blur();
                },
                for (value, display) in options {
                    option { value: "{value}", selected: value == field.value(), "{display}" }
                }
            }
            if let Some(err) = field.error() {
                span { class: "field-error", "{err}" }
            }
        }
    }
}

/// Checkbox over a `Field<bool>` (string-encoded `"true"`/`"false"`,
/// parsed with `validate::parse::<bool>` — seed the field with `"false"`).
#[component]
pub fn Checkbox(field: Field<bool>, label: String) -> Element {
    rsx! {
        label { class: "form-field form-field--checkbox",
            input {
                r#type: "checkbox",
                checked: field.value() == "true",
                onchange: move |evt| {
                    field.set(if evt.checked() { "true" } else { "false" });
                    field.blur();
                },
            }
            "{label}"
            if let Some(err) = field.error() {
                span { class: "field-error", "{err}" }
            }
        }
    }
}
