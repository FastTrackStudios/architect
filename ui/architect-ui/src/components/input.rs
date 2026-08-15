//! Input — shadcn v4 maia style, replaces lumen-blocks wrapper.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Input size variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Input visual variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputVariant {
    #[default]
    Default,
    Error,
}

#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    /// Current value (two-way binding).
    pub value: Signal<String>,

    #[props(default)]
    pub size: InputSize,

    #[props(default)]
    pub variant: InputVariant,

    #[props(default)]
    pub placeholder: String,

    /// HTML input type ("text", "password", "email", …). Defaults to "text".
    #[props(default)]
    pub input_type: Option<String>,

    #[props(default = false)]
    pub disabled: bool,

    #[props(default = false)]
    pub readonly: bool,

    #[props(default)]
    pub on_change: Option<Callback<FormEvent>>,

    #[props(default)]
    pub class: String,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    // shadcn v4 maia: cn-input
    let base = "w-full bg-input/30 border-input focus-visible:border-ring focus-visible:ring-ring/50 border px-3 py-1 text-sm transition-colors focus-visible:outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50 placeholder:text-muted-foreground";

    let size_class = match props.size {
        InputSize::Small => "h-8 rounded-lg text-xs",
        InputSize::Medium => "h-9 rounded-lg",
        InputSize::Large => "h-10 rounded-lg",
    };

    let variant_class = match props.variant {
        InputVariant::Default => "",
        InputVariant::Error => {
            "aria-invalid:ring-destructive/20 aria-invalid:border-destructive border-destructive"
        }
    };

    let mut value = props.value;

    rsx! {
        input {
            class: crate::cn::merge_slice(&[base, size_class, variant_class, props.class.as_str()]),
            r#type: props.input_type.clone().unwrap_or_else(|| "text".to_string()),
            value: "{value}",
            placeholder: "{props.placeholder}",
            disabled: if props.disabled { Some(true) } else { None },
            readonly: if props.readonly { Some(true) } else { None },
            oninput: move |e: FormEvent| {
                value.set(e.value());
                if let Some(cb) = &props.on_change {
                    cb.call(e);
                }
            },
        }
    }
}

/// Default text input with placeholder.
#[story(
    category = "Input",
    name = "default",
    knobs(placeholder = "Type something...")
)]
pub fn input_default(placeholder: &str) -> Element {
    let value = use_signal(String::new);
    rsx! {
        div { class: "p-6 bg-background text-foreground w-72",
            Input { value, placeholder: placeholder.to_string() }
        }
    }
}

/// Input sizes (Small / Medium / Large) crossed with default/disabled.
#[story(category = "Input", name = "sizes")]
pub fn input_sizes() -> Element {
    let s = use_signal(|| "Small".to_string());
    let m = use_signal(|| "Medium".to_string());
    let l = use_signal(|| "Large".to_string());
    let s_d = use_signal(|| "Small".to_string());
    let m_d = use_signal(|| "Medium".to_string());
    let l_d = use_signal(|| "Large".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            table { class: "border-collapse",
                thead {
                    tr { class: "text-xs uppercase tracking-wider text-muted-foreground",
                        th { class: "px-3 py-2 text-left", "Size" }
                        th { class: "px-3 py-2 text-left", "Default" }
                        th { class: "px-3 py-2 text-left", "Disabled" }
                    }
                }
                tbody {
                    tr {
                        td { class: "px-3 py-2 text-sm text-muted-foreground", "Small" }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: s, size: InputSize::Small } } }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: s_d, size: InputSize::Small, disabled: true } } }
                    }
                    tr {
                        td { class: "px-3 py-2 text-sm text-muted-foreground", "Medium" }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: m, size: InputSize::Medium } } }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: m_d, size: InputSize::Medium, disabled: true } } }
                    }
                    tr {
                        td { class: "px-3 py-2 text-sm text-muted-foreground", "Large" }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: l, size: InputSize::Large } } }
                        td { class: "px-3 py-2", div { class: "w-48", Input { value: l_d, size: InputSize::Large, disabled: true } } }
                    }
                }
            }
        }
    }
}

/// Input sizes and variants (default, error, disabled).
#[story(category = "Input", name = "variants")]
pub fn input_variants() -> Element {
    let a = use_signal(|| "Small".to_string());
    let b = use_signal(|| "Medium".to_string());
    let c = use_signal(|| "Large".to_string());
    let err = use_signal(|| "Error".to_string());
    let dis = use_signal(|| "Disabled".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground w-72 flex flex-col gap-2",
            Input { value: a, size: InputSize::Small }
            Input { value: b, size: InputSize::Medium }
            Input { value: c, size: InputSize::Large }
            Input { value: err, variant: InputVariant::Error }
            Input { value: dis, disabled: true }
        }
    }
}
