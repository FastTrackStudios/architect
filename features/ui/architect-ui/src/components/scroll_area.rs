//! ScrollArea — shadcn v4 maia style scroll container.
//!
//! CSS-only approach using Tailwind scrollbar utilities where available,
//! falling back to native OS scrollbars otherwise.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::scroll_area::ScrollArea as PrimitiveScrollArea;
pub use dioxus_primitives::scroll_area::{ScrollDirection, ScrollType};

#[derive(Props, Clone, PartialEq)]
pub struct ScrollAreaProps {
    #[props(default)]
    pub direction: ScrollDirection,
    #[props(default)]
    pub always_show_scrollbars: bool,
    #[props(default)]
    pub scroll_type: ScrollType,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-scroll-area
#[component]
pub fn ScrollArea(props: ScrollAreaProps) -> Element {
    rsx! {
        PrimitiveScrollArea {
            direction: props.direction,
            always_show_scrollbars: props.always_show_scrollbars,
            scroll_type: props.scroll_type,
            class: crate::cn::merge_slice(&["relative overflow-hidden", props.class.as_str()]),
            div {
                class: "h-full w-full overflow-auto scrollbar-thin scrollbar-thumb-rounded-full scrollbar-thumb-border scrollbar-track-transparent",
                {props.children}
            }
        }
    }
}

/// Default ScrollArea story rendering a fixed-height scrolling list.
#[story(category = "ScrollArea", name = "default")]
pub fn scroll_area_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            ScrollArea {
                always_show_scrollbars: true,
                class: "h-48 w-72 rounded-lg border border-border".to_string(),
                div { class: "p-4 flex flex-col gap-2 text-sm",
                    for i in 0..30 {
                        div { key: "{i}", class: "rounded-md border border-border px-3 py-2",
                            "Item {i + 1}"
                        }
                    }
                }
            }
        }
    }
}
