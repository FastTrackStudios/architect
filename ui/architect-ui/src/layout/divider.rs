//! Divider — shadcn v4 maia separator style.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::separator::Separator;

/// Orientation of the divider.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum DividerOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Props, Clone, PartialEq)]
pub struct DividerProps {
    #[props(default)]
    pub orientation: DividerOrientation,
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: cn-separator
#[component]
pub fn Divider(props: DividerProps) -> Element {
    let orientation_class = match props.orientation {
        DividerOrientation::Horizontal => "h-px w-full",
        DividerOrientation::Vertical => "h-full w-px",
    };
    let horizontal = matches!(props.orientation, DividerOrientation::Horizontal);

    rsx! {
        Separator {
            horizontal,
            class: crate::cn::merge_slice(&["shrink-0 bg-border", orientation_class, props.class.as_str()]),
        }
    }
}

#[story(category = "Divider", name = "divider_default")]
pub fn divider_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3 w-80",
            div { class: "text-sm", "Above" }
            Divider {}
            div { class: "text-sm", "Below" }
            div { class: "flex items-center gap-3 h-8",
                span { class: "text-sm", "Left" }
                Divider { orientation: DividerOrientation::Vertical }
                span { class: "text-sm", "Right" }
            }
        }
    }
}
