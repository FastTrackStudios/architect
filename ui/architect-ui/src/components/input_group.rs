//! InputGroup — input with prefix/suffix addon slots, shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct InputGroupProps {
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Container that wraps addons and an input control into a unified bordered group.
#[component]
pub fn InputGroup(props: InputGroupProps) -> Element {
    let base = "inline-flex items-center h-9 rounded-lg border border-input bg-input/30 transition-colors focus-within:border-ring focus-within:ring-ring/50 focus-within:ring-[3px]";

    let disabled_class = if props.disabled {
        "opacity-50 cursor-not-allowed"
    } else {
        ""
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[base, disabled_class, props.class.as_str()]),
            "data-disabled": if props.disabled { Some("true") } else { None },
            {props.children}
        }
    }
}

/// Alignment for the addon slot.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum InputGroupAlign {
    #[default]
    Start,
    End,
}

#[derive(Props, Clone, PartialEq)]
pub struct InputGroupAddonProps {
    #[props(default)]
    pub align: InputGroupAlign,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Prefix or suffix addon (icon, text, etc.) inside an InputGroup.
#[component]
pub fn InputGroupAddon(props: InputGroupAddonProps) -> Element {
    let base = "flex items-center text-muted-foreground text-sm font-medium [&>svg:not([class*='size-'])]:size-4";

    let align_class = match props.align {
        InputGroupAlign::Start => "pl-3",
        InputGroupAlign::End => "pr-3",
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[base, align_class, props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct InputGroupControlProps {
    /// Current value (two-way binding).
    pub value: Signal<String>,
    #[props(default)]
    pub placeholder: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub on_change: Option<Callback<FormEvent>>,
    #[props(default)]
    pub class: String,
}

/// The input element inside an InputGroup.
#[component]
pub fn InputGroupControl(props: InputGroupControlProps) -> Element {
    let base = "flex-1 h-full bg-transparent border-none px-3 text-sm placeholder:text-muted-foreground focus:outline-none";

    let mut value = props.value;

    rsx! {
        input {
            class: crate::cn::merge_slice(&[base, props.class.as_str()]),
            "data-slot": "input-group-control",
            value: "{value}",
            placeholder: "{props.placeholder}",
            disabled: if props.disabled { Some(true) } else { None },
            oninput: move |e: FormEvent| {
                value.set(e.value());
                if let Some(cb) = &props.on_change {
                    cb.call(e);
                }
            },
        }
    }
}

/// Input group with a prefix addon and a suffix addon.
#[story(category = "InputGroup", name = "default")]
pub fn input_group_default() -> Element {
    let value = use_signal(String::new);
    rsx! {
        div { class: "p-6 bg-background text-foreground w-80",
            InputGroup {
                InputGroupAddon { align: InputGroupAlign::Start, "$" }
                InputGroupControl { value, placeholder: "0.00".to_string() }
                InputGroupAddon { align: InputGroupAlign::End, "USD" }
            }
        }
    }
}
