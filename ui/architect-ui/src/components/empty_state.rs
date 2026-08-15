//! Empty state — dashed-border placeholder with centered text.
//!
//! Used when a list, panel, or section has no content to display.

use architect_story_runtime::story;
use dioxus::prelude::*;

/// A dashed-border container with centered placeholder text.
///
/// ```rust,ignore
/// EmptyState { message: "No setlists defined yet" }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct EmptyStateProps {
    /// The placeholder message.
    pub message: String,

    /// Optional icon or illustration above the message.
    #[props(default)]
    pub icon: Option<Element>,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn EmptyState(props: EmptyStateProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge(format!(
                "flex flex-col items-center justify-center gap-2 p-8 rounded-lg border border-dashed border-border {}",
                props.class
            )),

            if let Some(icon) = &props.icon {
                {icon}
            }

            p { class: "text-sm text-muted-foreground", "{props.message}" }
        }
    }
}

#[story(category = "EmptyState", name = "default")]
pub fn empty_state_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            EmptyState { message: "No items to show".to_string() }
        }
    }
}
