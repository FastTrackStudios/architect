//! Native select — platform select control with architect-ui styling.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct NativeSelectProps {
    pub value: Signal<String>,
    #[props(default)]
    pub on_change: Option<Callback<FormEvent>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub required: bool,
    #[props(default)]
    pub name: String,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn NativeSelect(props: NativeSelectProps) -> Element {
    let mut value = props.value;

    rsx! {
        select {
            class: crate::cn::merge_slice(&[
                "flex h-9 w-full appearance-none items-center justify-between rounded-lg border border-input bg-transparent px-3 py-1 pr-8 text-sm shadow-xs transition-colors focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50",
                props.class.as_str(),
            ]),
            value: "{value}",
            name: props.name,
            disabled: if props.disabled { Some(true) } else { None },
            required: if props.required { Some(true) } else { None },
            onchange: move |e: FormEvent| {
                value.set(e.value());
                if let Some(callback) = &props.on_change {
                    callback.call(e);
                }
            },
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct NativeSelectOptionProps {
    pub value: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn NativeSelectOption(props: NativeSelectOptionProps) -> Element {
    rsx! {
        option {
            class: props.class,
            value: props.value,
            disabled: if props.disabled { Some(true) } else { None },
            {props.children}
        }
    }
}

/// Default native select with three options.
#[story(category = "NativeSelect", name = "default")]
pub fn native_select_default() -> Element {
    let value = use_signal(|| "apple".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground w-64",
            NativeSelect { value,
                NativeSelectOption { value: "apple".to_string(), "Apple" }
                NativeSelectOption { value: "banana".to_string(), "Banana" }
                NativeSelectOption { value: "cherry".to_string(), "Cherry" }
            }
        }
    }
}
