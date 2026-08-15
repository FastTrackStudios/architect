//! Field — shadcn v4 maia style form field components.
//!
//! Provides `Field`, `FieldLabel`, `FieldDescription`, and `FieldMessage`
//! for consistent form layout and validation styling.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FieldProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Wrapper div for a form field group.
#[component]
pub fn Field(props: FieldProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["grid gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldLabelProps {
    #[props(default)]
    pub class: String,
    #[props(default = false)]
    pub required: bool,
    pub children: Element,
}

/// Label for a form field. Shows a red asterisk when `required` is true.
#[component]
pub fn FieldLabel(props: FieldLabelProps) -> Element {
    rsx! {
        label {
            class: crate::cn::merge_slice(&["text-sm font-medium leading-none", props.class.as_str()]),
            {props.children}
            if props.required {
                span { class: "text-destructive ml-0.5", "*" }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldDescriptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Help / hint text below a form field.
#[component]
pub fn FieldDescription(props: FieldDescriptionProps) -> Element {
    rsx! {
        p {
            class: crate::cn::merge_slice(&["text-muted-foreground text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldMessageProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Validation error message for a form field.
#[component]
pub fn FieldMessage(props: FieldMessageProps) -> Element {
    rsx! {
        p {
            class: crate::cn::merge_slice(&["text-destructive text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[story(category = "Field", name = "field_default")]
pub fn field_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground w-80",
            Field {
                FieldLabel { required: true, "Email" }
                input {
                    class: "h-9 w-full rounded-lg border border-input bg-transparent px-3 py-1 text-sm shadow-xs",
                    placeholder: "you@example.com",
                }
                FieldDescription { "We'll never share your email." }
                FieldMessage { "Required." }
            }
        }
    }
}
