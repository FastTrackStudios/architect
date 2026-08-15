//! Textarea — shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct TextareaProps {
    /// Current value (two-way binding).
    pub value: Signal<String>,

    #[props(default)]
    pub placeholder: String,

    #[props(default = false)]
    pub disabled: bool,

    #[props(default = false)]
    pub readonly: bool,

    #[props(default)]
    pub rows: Option<u32>,

    #[props(default)]
    pub on_change: Option<Callback<FormEvent>>,

    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: cn-textarea
#[component]
pub fn Textarea(props: TextareaProps) -> Element {
    let base = "w-full bg-input/30 border-input focus-visible:border-ring focus-visible:ring-ring/50 resize-none rounded-xl border px-3 py-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50 placeholder:text-muted-foreground";

    let rows = props.rows.unwrap_or(3);
    let mut value = props.value;

    rsx! {
        textarea {
            class: crate::cn::merge_slice(&[base, props.class.as_str()]),
            value: "{value}",
            placeholder: "{props.placeholder}",
            disabled: if props.disabled { Some(true) } else { None },
            readonly: if props.readonly { Some(true) } else { None },
            rows: "{rows}",
            oninput: move |e: FormEvent| {
                value.set(e.value());
                if let Some(cb) = &props.on_change {
                    cb.call(e);
                }
            },
        }
    }
}

/// Textarea states: default, disabled, error (via aria-invalid).
#[story(category = "Textarea", name = "states")]
pub fn textarea_states() -> Element {
    let a = use_signal(|| "Default content".to_string());
    let b = use_signal(|| "Disabled content".to_string());
    let c = use_signal(|| "Error content".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4 w-96",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Default" }
                Textarea { value: a, placeholder: "Type a message...".to_string(), rows: 3 }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Disabled" }
                Textarea { value: b, placeholder: "Type a message...".to_string(), rows: 3, disabled: true }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Error" }
                Textarea {
                    value: c,
                    placeholder: "Type a message...".to_string(),
                    rows: 3,
                    class: "border-destructive aria-invalid:ring-destructive/20 aria-invalid:border-destructive".to_string(),
                }
            }
        }
    }
}

/// Default textarea with multi-line content.
#[story(
    category = "Textarea",
    name = "default",
    knobs(placeholder = "Type a longer message...")
)]
pub fn textarea_default(placeholder: &str) -> Element {
    let value = use_signal(|| "Multi-line\nsecond line".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground w-96",
            Textarea { value, placeholder: placeholder.to_string(), rows: 4 }
        }
    }
}
