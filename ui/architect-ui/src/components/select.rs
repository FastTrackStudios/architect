//! Select — primitive-backed single-value select dropdown.

use dioxus::prelude::*;
use dioxus_primitives::select::{
    Select as PrimitiveSelect, SelectGroup as PrimitiveSelectGroup, SelectItemIndicator,
    SelectList as PrimitiveSelectList, SelectMulti as PrimitiveSelectMulti,
    SelectOption as PrimitiveSelectOption, SelectTrigger as PrimitiveSelectTrigger,
    SelectValue as PrimitiveSelectValue,
};
use architect_story_runtime::story;
use lucide_dioxus::{Check, ChevronsUpDown};

#[derive(Props, Clone, PartialEq)]
pub struct SelectProps {
    /// The currently selected value.
    pub value: Signal<String>,
    #[props(default)]
    pub on_change: Option<Callback<String>>,
    #[props(default = "Select...".to_string())]
    pub placeholder: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Select(props: SelectProps) -> Element {
    let mut value = props.value;
    let selected: ReadSignal<Option<String>> = use_memo(move || {
        let value = value();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    })
    .into();

    rsx! {
        document::Style {
            r#"
                .fts-select-item[aria-selected="true"] {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}

                .fts-select-item:focus {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}

                .fts-select-item[aria-disabled="true"] {{
                    pointer-events: none;
                    opacity: 0.5;
                }}
            "#
        }
        PrimitiveSelect::<String> {
            value: Some(selected),
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            on_value_change: move |next: Option<String>| {
                let next = next.unwrap_or_default();
                value.set(next.clone());
                if let Some(callback) = &props.on_change {
                    callback.call(next);
                }
            },
            class: crate::cn::merge_slice(&["relative w-full", props.class.as_str()]),
            SelectTrigger { placeholder: props.placeholder.clone() }
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectTriggerProps {
    #[props(default = "Select...".to_string())]
    pub placeholder: String,
    #[props(default)]
    pub class: String,
    #[props(default = false)]
    pub custom_content: bool,
    #[props(default)]
    pub children: Element,
}

#[component]
pub fn SelectTrigger(props: SelectTriggerProps) -> Element {
    rsx! {
        PrimitiveSelectTrigger {
            class: crate::cn::merge(format!(
                "flex h-9 w-full items-center justify-between gap-2 rounded-lg border border-input bg-transparent px-3 py-2 text-sm whitespace-nowrap shadow-xs outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 {}",
                props.class
            )),
            if props.custom_content {
                {props.children}
            } else {
                PrimitiveSelectValue {
                    placeholder: props.placeholder,
                    class: "truncate text-left data-[placeholder=true]:text-muted-foreground",
                }
                ChevronsUpDown { class: "size-4 shrink-0 text-muted-foreground opacity-50" }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SelectContent(props: SelectContentProps) -> Element {
    rsx! {
        PrimitiveSelectList {
            id: props.id,
            class: crate::cn::merge(format!(
                "absolute left-0 top-full z-50 mt-1 max-h-80 min-w-full w-fit overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-md animate-fade-in p-1 {}",
                props.class
            )),
            div {
                class: "max-h-72 min-w-48 overflow-y-auto",
                {props.children}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectGroupProps {
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SelectGroup(props: SelectGroupProps) -> Element {
    rsx! {
        PrimitiveSelectGroup {
            disabled: props.disabled,
            id: props.id,
            class: crate::cn::merge_slice(&["py-1", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectItemProps {
    pub value: String,
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub text_value: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SelectItem(props: SelectItemProps) -> Element {
    rsx! {
        PrimitiveSelectOption::<String> {
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            text_value: props.text_value,
            class: crate::cn::merge(format!(
                "fts-select-item relative flex w-full cursor-default select-none items-center gap-2 rounded-md py-1.5 pr-8 pl-2 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground {}",
                props.class
            )),
            span { class: "flex-1", {props.children} }
            SelectItemIndicator {
                span { class: "absolute right-2 flex size-3.5 items-center justify-center",
                    Check { class: "size-4 text-current" }
                }
            }
        }
    }
}

#[component]
pub fn SelectSeparator() -> Element {
    rsx! {
        div { class: "-mx-1 my-1 h-px bg-border" }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectLabelProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SelectLabel(props: SelectLabelProps) -> Element {
    rsx! {
        div {
            id: props.id,
            class: crate::cn::merge_slice(&["px-2 py-1.5 text-xs font-medium text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── SelectMulti ─────────────────────────────────────────────────────────────
//
// Multi-value variant added upstream in PR #224. Same sub-components
// (`SelectTrigger`, `SelectContent`, `SelectItem`, ...) compose under it.

#[derive(Props, Clone, PartialEq)]
pub struct SelectMultiWrapperProps {
    /// The currently selected values (two-way bound).
    pub values: Signal<Vec<String>>,
    #[props(default)]
    pub on_change: Option<Callback<Vec<String>>>,
    #[props(default = "Select...".to_string())]
    pub placeholder: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SelectMulti(props: SelectMultiWrapperProps) -> Element {
    let mut values = props.values;
    let selected: ReadSignal<Option<Vec<String>>> = use_memo(move || Some(values())).into();

    rsx! {
        document::Style {
            r#"
                .fts-select-item[aria-selected="true"] {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}

                .fts-select-item:focus {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}

                .fts-select-item[aria-disabled="true"] {{
                    pointer-events: none;
                    opacity: 0.5;
                }}
            "#
        }
        PrimitiveSelectMulti::<String> {
            values: selected,
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            on_values_change: move |next: Vec<String>| {
                values.set(next.clone());
                if let Some(callback) = &props.on_change {
                    callback.call(next);
                }
            },
            class: crate::cn::merge_slice(&["relative w-full", props.class.as_str()]),
            SelectTrigger { placeholder: props.placeholder.clone() }
            {props.children}
        }
    }
}

/// Select states: empty (placeholder showing) / selected / disabled.
#[story(category = "Select", name = "states")]
pub fn select_states() -> Element {
    let empty = use_signal(String::new);
    let selected = use_signal(|| "banana".to_string());
    let disabled = use_signal(|| "apple".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4 w-64",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Empty" }
                Select {
                    value: empty,
                    placeholder: "Choose a fruit...".to_string(),
                    SelectContent {
                        SelectItem { value: "apple".to_string(), index: 0, "Apple" }
                        SelectItem { value: "banana".to_string(), index: 1, "Banana" }
                        SelectItem { value: "cherry".to_string(), index: 2, "Cherry" }
                    }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Selected" }
                Select {
                    value: selected,
                    placeholder: "Choose a fruit...".to_string(),
                    SelectContent {
                        SelectItem { value: "apple".to_string(), index: 0, "Apple" }
                        SelectItem { value: "banana".to_string(), index: 1, "Banana" }
                        SelectItem { value: "cherry".to_string(), index: 2, "Cherry" }
                    }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-1", "Disabled" }
                Select {
                    value: disabled,
                    placeholder: "Choose a fruit...".to_string(),
                    disabled: true,
                    SelectContent {
                        SelectItem { value: "apple".to_string(), index: 0, "Apple" }
                        SelectItem { value: "banana".to_string(), index: 1, "Banana" }
                    }
                }
            }
        }
    }
}

/// Default select with a list of fruit options.
#[story(
    category = "Select",
    name = "default",
    knobs(placeholder = "Choose a fruit...")
)]
pub fn select_default(placeholder: &str) -> Element {
    let value = use_signal(String::new);
    rsx! {
        div { class: "p-6 bg-background text-foreground w-64",
            Select {
                value,
                placeholder: placeholder.to_string(),
                SelectContent {
                    SelectGroup {
                        SelectLabel { "Fruits" }
                        SelectItem { value: "apple".to_string(), index: 0, "Apple" }
                        SelectItem { value: "banana".to_string(), index: 1, "Banana" }
                        SelectItem { value: "cherry".to_string(), index: 2, "Cherry" }
                    }
                    SelectSeparator {}
                    SelectGroup {
                        SelectLabel { "Vegetables" }
                        SelectItem { value: "carrot".to_string(), index: 3, "Carrot" }
                        SelectItem { value: "potato".to_string(), index: 4, "Potato" }
                    }
                }
            }
        }
    }
}
