//! Radio group — shadcn v4 maia style radio buttons.

use dioxus::prelude::*;
use dioxus_primitives::radio_group::{
    RadioGroup as PrimitiveRadioGroup, RadioItem as PrimitiveRadioItem,
};
use architect_story_runtime::story;

// ---------------------------------------------------------------------------
// Context shared between RadioGroup and RadioGroupItem
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct RadioGroupContext {
    value: Signal<String>,
    next_index: Signal<usize>,
}

// ---------------------------------------------------------------------------
// RadioGroup
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct RadioGroupProps {
    /// The currently selected value (two-way bound).
    pub value: Signal<String>,

    /// Called when the user picks a different option.
    #[props(default)]
    pub on_change: Option<Callback<String>>,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: radio-group
#[component]
pub fn RadioGroup(props: RadioGroupProps) -> Element {
    let mut value = props.value;
    let on_change = props.on_change;
    let next_index = use_signal(|| 0usize);

    use_context_provider(|| RadioGroupContext { value, next_index });

    rsx! {
        PrimitiveRadioGroup {
            value: Some(value()),
            class: crate::cn::merge_slice(&["grid gap-3", props.class.as_str()]),
            on_value_change: move |next: String| {
                value.set(next.clone());
                if let Some(cb) = &on_change {
                    cb.call(next);
                }
            },
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// RadioGroupItem
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct RadioGroupItemProps {
    /// The value this radio item represents.
    pub value: String,

    /// Whether this item is disabled.
    #[props(default = false)]
    pub disabled: bool,

    /// Extra CSS classes on the button.
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: radio-group-item
#[component]
pub fn RadioGroupItem(props: RadioGroupItemProps) -> Element {
    let mut ctx: RadioGroupContext = use_context();
    let index = use_hook(|| {
        let mut next_index = ctx.next_index.write();
        let index = *next_index;
        *next_index += 1;
        index
    });
    let is_checked = *ctx.value.read() == props.value;

    let outer_class = if is_checked {
        "border-primary bg-primary"
    } else {
        "border-input dark:bg-input/30"
    };

    rsx! {
        PrimitiveRadioItem {
            value: props.value,
            index,
            disabled: props.disabled,
            class: crate::cn::merge(format!(
                "relative flex size-4 items-center justify-center rounded-full border focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 {outer_class} {}",
                props.class
            )),
            if is_checked {
                span {
                    class: "absolute bg-primary-foreground size-2 rounded-full",
                }
            }
        }
    }
}

/// Two radio groups side by side: a standard group and one with a disabled item.
#[story(category = "RadioGroup", name = "states")]
pub fn radio_group_states() -> Element {
    let standard = use_signal(|| "a".to_string());
    let with_disabled = use_signal(|| "a".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-6",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "Standard" }
                RadioGroup { value: standard,
                    div { class: "flex flex-col gap-2",
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "a".to_string() }
                            span { class: "text-sm", "Option A" }
                        }
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "b".to_string() }
                            span { class: "text-sm", "Option B" }
                        }
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "c".to_string() }
                            span { class: "text-sm", "Option C" }
                        }
                    }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "With disabled item" }
                RadioGroup { value: with_disabled,
                    div { class: "flex flex-col gap-2",
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "a".to_string() }
                            span { class: "text-sm", "Available" }
                        }
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "b".to_string(), disabled: true }
                            span { class: "text-sm text-muted-foreground", "Disabled option" }
                        }
                        div { class: "flex items-center gap-2",
                            RadioGroupItem { value: "c".to_string() }
                            span { class: "text-sm", "Also available" }
                        }
                    }
                }
            }
        }
    }
}

/// Default radio group with three options.
#[story(category = "RadioGroup", name = "default")]
pub fn radio_group_default() -> Element {
    let value = use_signal(|| "a".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            RadioGroup { value,
                div { class: "flex items-center gap-4",
                    div { class: "flex items-center gap-2",
                        RadioGroupItem { value: "a".to_string() }
                        span { class: "text-sm", "Option A" }
                    }
                    div { class: "flex items-center gap-2",
                        RadioGroupItem { value: "b".to_string() }
                        span { class: "text-sm", "Option B" }
                    }
                    div { class: "flex items-center gap-2",
                        RadioGroupItem { value: "c".to_string() }
                        span { class: "text-sm", "Option C" }
                    }
                }
            }
        }
    }
}
