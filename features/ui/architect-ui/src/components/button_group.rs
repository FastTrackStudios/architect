//! Button group — shadcn-style grouped actions.

use architect_story_runtime::story;
use dioxus::prelude::*;

/// Button group axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonGroupOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonGroupProps {
    #[props(default)]
    pub orientation: ButtonGroupOrientation,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ButtonGroup(props: ButtonGroupProps) -> Element {
    let orientation = match props.orientation {
        ButtonGroupOrientation::Horizontal => {
            "flex-row [&>*:not(:first-child)]:-ml-px [&>*:not(:first-child)]:rounded-l-none [&>*:not(:last-child)]:rounded-r-none"
        }
        ButtonGroupOrientation::Vertical => {
            "flex-col [&>*:not(:first-child)]:-mt-px [&>*:not(:first-child)]:rounded-t-none [&>*:not(:last-child)]:rounded-b-none"
        }
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[
                "inline-flex isolate w-fit rounded-lg shadow-xs [&>*]:relative [&>*]:focus-visible:z-10",
                orientation,
                props.class.as_str(),
            ]),
            role: "group",
            "aria-disabled": if props.disabled { "true" } else { "false" },
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonGroupTextProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ButtonGroupText(props: ButtonGroupTextProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&[
                "inline-flex h-9 items-center justify-center border border-input bg-muted px-3 text-sm text-muted-foreground",
                props.class.as_str(),
            ]),
            {props.children}
        }
    }
}

#[story(category = "ButtonGroup", name = "button_group_default")]
pub fn button_group_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4",
            ButtonGroup {
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Left" }
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Middle" }
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Right" }
            }
            ButtonGroup { orientation: ButtonGroupOrientation::Vertical,
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Top" }
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Mid" }
                crate::components::Button { variant: crate::components::ButtonVariant::Outline, "Bot" }
            }
        }
    }
}
