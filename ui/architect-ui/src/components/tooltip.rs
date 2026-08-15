//! Tooltip — shadcn v4 maia style CSS-only tooltip.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::{
    tooltip::{
        Tooltip as PrimitiveTooltip, TooltipContent as PrimitiveTooltipContent,
        TooltipTrigger as PrimitiveTooltipTrigger,
    },
    ContentAlign, ContentSide,
};

// ---------------------------------------------------------------------------
// TooltipSide
// ---------------------------------------------------------------------------

/// Which side the tooltip content appears on relative to the trigger.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum TooltipSide {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl TooltipSide {
    fn primitive_side(self) -> ContentSide {
        match self {
            Self::Top => ContentSide::Top,
            Self::Bottom => ContentSide::Bottom,
            Self::Left => ContentSide::Left,
            Self::Right => ContentSide::Right,
        }
    }

    fn position_class(self, align: ContentAlign) -> &'static str {
        match (self, align) {
            (Self::Top, ContentAlign::Start) => "bottom-full left-0 mb-2",
            (Self::Top, ContentAlign::Center) => "bottom-full left-1/2 -translate-x-1/2 mb-2",
            (Self::Top, ContentAlign::End) => "bottom-full right-0 mb-2",
            (Self::Bottom, ContentAlign::Start) => "top-full left-0 mt-2",
            (Self::Bottom, ContentAlign::Center) => "top-full left-1/2 -translate-x-1/2 mt-2",
            (Self::Bottom, ContentAlign::End) => "top-full right-0 mt-2",
            (Self::Left, ContentAlign::Start) => "right-full top-0 mr-2",
            (Self::Left, ContentAlign::Center) => "right-full top-1/2 -translate-y-1/2 mr-2",
            (Self::Left, ContentAlign::End) => "right-full bottom-0 mr-2",
            (Self::Right, ContentAlign::Start) => "left-full top-0 ml-2",
            (Self::Right, ContentAlign::Center) => "left-full top-1/2 -translate-y-1/2 ml-2",
            (Self::Right, ContentAlign::End) => "left-full bottom-0 ml-2",
        }
    }
}

// ---------------------------------------------------------------------------
// Tooltip (wrapper)
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct TooltipProps {
    #[props(default)]
    pub open: Option<bool>,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default)]
    pub on_open_change: Option<Callback<bool>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: tooltip wrapper — uses CSS `group-hover` for show/hide.
#[component]
pub fn Tooltip(props: TooltipProps) -> Element {
    rsx! {
        PrimitiveTooltip {
            open: props.open,
            default_open: props.default_open,
            disabled: props.disabled,
            on_open_change: move |open| {
                if let Some(callback) = &props.on_open_change {
                    callback.call(open);
                }
            },
            class: crate::cn::merge_slice(&["relative inline-flex group", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// TooltipTrigger
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct TooltipTriggerProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: tooltip trigger — renders children directly.
#[component]
pub fn TooltipTrigger(props: TooltipTriggerProps) -> Element {
    rsx! {
        PrimitiveTooltipTrigger {
            id: props.id,
            class: props.class,
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// TooltipContent
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct TooltipContentProps {
    /// Which side the tooltip appears on.
    #[props(default)]
    pub side: TooltipSide,
    #[props(default = ContentAlign::Center)]
    pub align: ContentAlign,
    #[props(default)]
    pub id: Option<String>,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: tooltip content popup
#[component]
pub fn TooltipContent(props: TooltipContentProps) -> Element {
    let position_class = props.side.position_class(props.align);

    rsx! {
        PrimitiveTooltipContent {
            id: props.id,
            side: props.side.primitive_side(),
            align: props.align,
            class: crate::cn::merge_slice(&[
                "absolute z-50 inline-flex items-center gap-1.5 rounded-lg bg-primary text-primary-foreground px-3 py-1.5 text-xs shadow-md whitespace-nowrap pointer-events-none",
                position_class,
                props.class.as_str(),
            ]),
            {props.children}
        }
    }
}

/// Tooltip forced open showing content above the trigger.
#[story(category = "Tooltip", name = "tooltip default")]
pub fn tooltip_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center justify-center min-h-[12rem]",
            Tooltip {
                open: open(),
                on_open_change: move |o| open.set(o),
                TooltipTrigger {
                    button { class: "h-9 px-3 rounded-lg border border-border text-sm", "Hover" }
                }
                TooltipContent { side: TooltipSide::Top, "Tooltip text" }
            }
        }
    }
}
