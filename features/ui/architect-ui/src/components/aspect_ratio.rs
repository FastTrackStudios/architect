//! AspectRatio — shadcn v4 maia style CSS aspect-ratio wrapper.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::aspect_ratio::AspectRatio as PrimitiveAspectRatio;

#[derive(Props, Clone, PartialEq)]
pub struct AspectRatioProps {
    /// The aspect ratio expressed as width / height (e.g. 16.0/9.0).
    #[props(default = 16.0 / 9.0)]
    pub ratio: f64,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-aspect-ratio
///
/// Common ratios: 16/9, 4/3, 1/1, 21/9.
#[component]
pub fn AspectRatio(props: AspectRatioProps) -> Element {
    rsx! {
        PrimitiveAspectRatio {
            ratio: props.ratio,
            class: crate::cn::merge_slice(&["relative w-full overflow-hidden", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Default AspectRatio story showing common ratios side-by-side.
#[story(category = "AspectRatio", name = "default")]
pub fn aspect_ratio_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground grid grid-cols-1 sm:grid-cols-3 gap-4",
            div { class: "w-full max-w-xs",
                p { class: "text-xs text-muted-foreground mb-1", "16 / 9" }
                AspectRatio { ratio: 16.0 / 9.0, class: "rounded-lg border border-border bg-muted".to_string(),
                    div { class: "flex h-full w-full items-center justify-center text-sm text-muted-foreground",
                        "16:9"
                    }
                }
            }
            div { class: "w-full max-w-xs",
                p { class: "text-xs text-muted-foreground mb-1", "4 / 3" }
                AspectRatio { ratio: 4.0 / 3.0, class: "rounded-lg border border-border bg-muted".to_string(),
                    div { class: "flex h-full w-full items-center justify-center text-sm text-muted-foreground",
                        "4:3"
                    }
                }
            }
            div { class: "w-full max-w-xs",
                p { class: "text-xs text-muted-foreground mb-1", "1 / 1" }
                AspectRatio { ratio: 1.0, class: "rounded-lg border border-border bg-muted".to_string(),
                    div { class: "flex h-full w-full items-center justify-center text-sm text-muted-foreground",
                        "1:1"
                    }
                }
            }
        }
    }
}
