//! Spinner — loading indicator, shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Spinner size variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SpinnerSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl SpinnerSize {
    fn classes(self) -> &'static str {
        match self {
            Self::Small => "size-4",
            Self::Medium => "size-6",
            Self::Large => "size-8",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SpinnerProps {
    #[props(default)]
    pub size: SpinnerSize,

    #[props(default)]
    pub class: String,
}

/// SVG circle spinner with animate-spin.
#[component]
pub fn Spinner(props: SpinnerProps) -> Element {
    let size_class = props.size.classes();

    rsx! {
        svg {
            class: crate::cn::merge_slice(&["animate-spin text-muted-foreground", size_class, props.class.as_str()]),
            xmlns: "http://www.w3.org/2000/svg",
            fill: "none",
            view_box: "0 0 24 24",
            circle {
                class: "opacity-25",
                cx: "12",
                cy: "12",
                r: "10",
                stroke: "currentColor",
                stroke_width: "4",
            }
            path {
                class: "opacity-75",
                fill: "currentColor",
                d: "M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z",
            }
        }
    }
}

#[story(category = "Spinner", name = "spinner_sizes")]
pub fn spinner_sizes() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-4",
            Spinner { size: SpinnerSize::Small }
            Spinner { size: SpinnerSize::Medium }
            Spinner { size: SpinnerSize::Large }
        }
    }
}
