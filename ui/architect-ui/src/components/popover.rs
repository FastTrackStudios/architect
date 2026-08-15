//! Popover — shadcn v4 maia style positioned floating panel.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::{
    popover::{
        PopoverContent as PrimitivePopoverContent, PopoverRoot as PrimitivePopover,
        PopoverTrigger as PrimitivePopoverTrigger,
    },
    ContentAlign, ContentSide,
};

#[derive(Props, Clone, PartialEq)]
pub struct PopoverProps {
    #[props(default)]
    pub open: Option<bool>,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default = true)]
    pub is_modal: bool,
    #[props(default)]
    pub on_open_change: Option<Callback<bool>>,
    #[props(default)]
    pub on_close: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-popover
#[component]
pub fn Popover(props: PopoverProps) -> Element {
    rsx! {
        PrimitivePopover {
            open: props.open,
            default_open: props.default_open,
            is_modal: props.is_modal,
            on_open_change: move |open| {
                if let Some(callback) = &props.on_open_change {
                    callback.call(open);
                }
                if !open {
                    if let Some(callback) = &props.on_close {
                        callback.call(());
                    }
                }
            },
            class: crate::cn::merge_slice(&["relative inline-block", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PopoverTriggerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-popover-trigger
#[component]
pub fn PopoverTrigger(props: PopoverTriggerProps) -> Element {
    rsx! {
        PrimitivePopoverTrigger {
            class: crate::cn::merge_slice(&["cursor-pointer", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PopoverContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default = ContentSide::Bottom)]
    pub side: ContentSide,
    #[props(default = ContentAlign::Center)]
    pub align: ContentAlign,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Tailwind anchor classes derived from `(side, align)`.
///
/// The underlying dioxus-primitives `PopoverContent` only sets
/// `data-side` + `data-align` attributes — actual positioning is up to
/// the styling layer. This wrapper translates the typed props into
/// `position: absolute` anchor utilities so the popover renders on the
/// requested edge of its trigger:
///
/// - `side=Bottom` (default): pop below trigger, `top-full mt-2`
/// - `side=Top`: pop above trigger, `bottom-full mb-2`
/// - `side=Right`: pop to the right, `left-full ml-2 top-0`
/// - `side=Left`: pop to the left, `right-full mr-2 top-0`
///
/// `align` controls cross-axis: for vertical sides (top/bottom), align
/// the popover's start/center/end with the trigger's matching edge via
/// `left-0` / `left-1/2 -translate-x-1/2` / `right-0`. For horizontal
/// sides, similar with vertical anchors.
fn anchor_class(side: ContentSide, align: ContentAlign) -> &'static str {
    use ContentAlign::*;
    use ContentSide::*;
    match (side, align) {
        (Bottom, Start) => "top-full mt-2 left-0",
        (Bottom, Center) => "top-full mt-2 left-1/2 -translate-x-1/2",
        (Bottom, End) => "top-full mt-2 right-0",
        (Top, Start) => "bottom-full mb-2 left-0",
        (Top, Center) => "bottom-full mb-2 left-1/2 -translate-x-1/2",
        (Top, End) => "bottom-full mb-2 right-0",
        (Right, Start) => "left-full ml-2 top-0",
        (Right, Center) => "left-full ml-2 top-1/2 -translate-y-1/2",
        (Right, End) => "left-full ml-2 bottom-0",
        (Left, Start) => "right-full mr-2 top-0",
        (Left, Center) => "right-full mr-2 top-1/2 -translate-y-1/2",
        (Left, End) => "right-full mr-2 bottom-0",
    }
}

/// shadcn v4 maia: cn-popover-content
#[component]
pub fn PopoverContent(props: PopoverContentProps) -> Element {
    let anchor = anchor_class(props.side, props.align);
    rsx! {
        PrimitivePopoverContent {
            id: props.id,
            side: props.side,
            align: props.align,
            class: crate::cn::merge(format!(
                "bg-popover text-popover-foreground border border-border flex flex-col gap-4 rounded-lg p-4 text-sm shadow-md absolute z-50 {anchor} {}",
                props.class
            )),
            {props.children}
        }
    }
}

/// Popover forced open with simple content.
#[story(category = "Popover", name = "popover default")]
pub fn popover_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center justify-center min-h-[18rem]",
            Popover {
                open: open(),
                is_modal: false,
                on_open_change: move |o| open.set(o),
                PopoverTrigger {
                    button { class: "h-9 px-3 rounded-lg border border-border text-sm", "Open popover" }
                }
                PopoverContent {
                    div { class: "flex flex-col gap-1",
                        span { class: "text-sm font-medium", "Popover" }
                        span { class: "text-xs text-muted-foreground", "Anchored content next to the trigger." }
                    }
                }
            }
        }
    }
}
