//! Checkbox — shadcn v4 maia style, standalone (no lumen-blocks).

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::checkbox::{
    Checkbox as PrimitiveCheckbox, CheckboxIndicator, CheckboxState,
};

#[derive(Props, Clone, PartialEq)]
pub struct CheckboxProps {
    pub checked: Signal<bool>,
    #[props(default)]
    pub on_change: Option<Callback<bool>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: cn-checkbox
#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    let checked = (props.checked)();
    let state = if checked {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    };

    // shadcn v4 maia: cn-checkbox
    let base = "border-input dark:bg-input/30 data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground dark:data-[state=checked]:bg-primary data-[state=checked]:border-primary focus-visible:border-ring focus-visible:ring-ring/50 flex size-4 items-center justify-center rounded-[6px] border transition-shadow focus-visible:ring-[3px]";

    let disabled_cls = if props.disabled {
        "opacity-50 cursor-not-allowed"
    } else {
        "cursor-pointer"
    };

    let class = crate::cn::merge_slice(&[base, disabled_cls, props.class.as_str()]);

    rsx! {
        PrimitiveCheckbox {
            checked: Some(state),
            disabled: props.disabled,
            class,
            on_checked_change: move |next_state| {
                let next = !matches!(next_state, CheckboxState::Unchecked);
                if next != checked {
                    let mut sig = props.checked;
                    sig.set(next);
                    if let Some(cb) = &props.on_change {
                        cb.call(next);
                    }
                }
            },
            CheckboxIndicator {
                svg {
                    class: "size-3.5",
                    xmlns: "http://www.w3.org/2000/svg",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    path { d: "M20 6 9 17l-5-5" }
                }
            }
        }
    }
}

/// Matrix of Checkbox states: unchecked / checked × enabled / disabled.
/// (Indeterminate is not exposed by this wrapper.)
#[story(category = "Checkbox", name = "states")]
pub fn checkbox_states() -> Element {
    let off = use_signal(|| false);
    let on = use_signal(|| true);
    let off_d = use_signal(|| false);
    let on_d = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            table { class: "border-collapse",
                thead {
                    tr { class: "text-xs uppercase tracking-wider text-muted-foreground",
                        th { class: "px-3 py-2 text-left", "State" }
                        th { class: "px-3 py-2 text-left", "Enabled" }
                        th { class: "px-3 py-2 text-left", "Disabled" }
                    }
                }
                tbody {
                    tr {
                        td { class: "px-3 py-2 text-sm text-muted-foreground", "Unchecked" }
                        td { class: "px-3 py-2", Checkbox { checked: off } }
                        td { class: "px-3 py-2", Checkbox { checked: off_d, disabled: true } }
                    }
                    tr {
                        td { class: "px-3 py-2 text-sm text-muted-foreground", "Checked" }
                        td { class: "px-3 py-2", Checkbox { checked: on } }
                        td { class: "px-3 py-2", Checkbox { checked: on_d, disabled: true } }
                    }
                }
            }
        }
    }
}

/// Default checkbox in checked, unchecked, and disabled states.
#[story(category = "Checkbox", name = "default")]
pub fn checkbox_default() -> Element {
    let off = use_signal(|| false);
    let on = use_signal(|| true);
    let dis = use_signal(|| false);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-4",
            Checkbox { checked: off }
            Checkbox { checked: on }
            Checkbox { checked: dis, disabled: true }
        }
    }
}
