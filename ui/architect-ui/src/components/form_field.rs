//! Form field — label + input wrapper for modal/form layouts.
//!
//! Provides consistent label styling and spacing for form inputs, textareas,
//! and custom form controls.

use dioxus::prelude::*;

/// A labeled wrapper for form inputs.
///
/// ```rust,ignore
/// FormField {
///     label: "Name",
///     Input { value: name, placeholder: "Enter preset name..." }
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct FormFieldProps {
    /// Field label text (rendered as uppercase).
    pub label: String,

    /// Optional hint/help text below the label.
    #[props(default)]
    pub hint: String,

    /// Whether the field is required (shows a subtle indicator).
    #[props(default = false)]
    pub required: bool,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,

    /// The form control (Input, textarea, custom element).
    pub children: Element,
}

#[component]
pub fn FormField(props: FormFieldProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-1.5", props.class.as_str()]),

            label {
                class: "text-xs font-semibold text-muted-foreground uppercase tracking-wider",

                "{props.label}"
                if props.required {
                    span { class: "text-destructive ml-0.5", "*" }
                }
            }

            if !props.hint.is_empty() {
                p { class: "text-[10px] text-muted-foreground -mt-0.5", "{props.hint}" }
            }

            {props.children}
        }
    }
}
