//! Skeleton — loading placeholder, shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonProps {
    #[props(default)]
    pub class: String,
}

/// Generic skeleton block.
#[component]
pub fn Skeleton(props: SkeletonProps) -> Element {
    let base = "bg-muted rounded-xl animate-pulse";

    rsx! {
        div { class: crate::cn::merge_slice(&[base, props.class.as_str()]) }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonTextProps {
    #[props(default)]
    pub class: String,
}

/// Single-line text skeleton.
#[component]
pub fn SkeletonText(props: SkeletonTextProps) -> Element {
    let base = "bg-muted rounded-xl animate-pulse h-4";

    rsx! {
        div { class: crate::cn::merge_slice(&[base, props.class.as_str()]) }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonCircleProps {
    #[props(default)]
    pub class: String,
}

/// Circular skeleton (e.g. avatar placeholder).
#[component]
pub fn SkeletonCircle(props: SkeletonCircleProps) -> Element {
    let base = "bg-muted rounded-full animate-pulse";

    rsx! {
        div { class: crate::cn::merge_slice(&[base, props.class.as_str()]) }
    }
}

#[story(category = "Skeleton", name = "skeleton_default")]
pub fn skeleton_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-start gap-3 w-96",
            SkeletonCircle { class: "size-10".to_string() }
            div { class: "flex-1 flex flex-col gap-2",
                SkeletonText {}
                Skeleton { class: "h-4 w-2/3".to_string() }
                Skeleton { class: "h-20 w-full".to_string() }
            }
        }
    }
}
