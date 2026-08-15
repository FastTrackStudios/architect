//! HoverCard — primitive-backed hover/focus card.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::{
    hover_card::{
        HoverCard as PrimitiveHoverCard, HoverCardContent as PrimitiveHoverCardContent,
        HoverCardTrigger as PrimitiveHoverCardTrigger,
    },
    ContentAlign, ContentSide,
};

/// Which side the hover card appears on.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum HoverCardSide {
    Top,
    #[default]
    Bottom,
}

impl HoverCardSide {
    fn to_content_side(self) -> ContentSide {
        match self {
            Self::Top => ContentSide::Top,
            Self::Bottom => ContentSide::Bottom,
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct HoverCardProps {
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

#[component]
pub fn HoverCard(props: HoverCardProps) -> Element {
    rsx! {
        PrimitiveHoverCard {
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

#[derive(Props, Clone, PartialEq)]
pub struct HoverCardTriggerProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn HoverCardTrigger(props: HoverCardTriggerProps) -> Element {
    rsx! {
        PrimitiveHoverCardTrigger {
            id: props.id,
            class: props.class,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct HoverCardContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub side: HoverCardSide,
    #[props(default = ContentAlign::Center)]
    pub align: ContentAlign,
    #[props(default = true)]
    pub force_mount: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn HoverCardContent(props: HoverCardContentProps) -> Element {
    rsx! {
        PrimitiveHoverCardContent {
            id: props.id,
            side: props.side.to_content_side(),
            align: props.align,
            force_mount: props.force_mount,
            class: crate::cn::merge(format!(
                "absolute z-50 w-64 rounded-lg bg-popover text-popover-foreground border border-border shadow-md p-4 text-sm data-[side=top]:bottom-full data-[side=top]:left-1/2 data-[side=top]:-translate-x-1/2 data-[side=top]:mb-2 data-[side=bottom]:top-full data-[side=bottom]:left-1/2 data-[side=bottom]:-translate-x-1/2 data-[side=bottom]:mt-2 {}",
                props.class
            )),
            {props.children}
        }
    }
}

/// HoverCard forced open with content rendered for snapshots.
#[story(category = "HoverCard", name = "hover card default")]
pub fn hover_card_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center justify-center min-h-[18rem]",
            HoverCard {
                open: open(),
                on_open_change: move |o| open.set(o),
                HoverCardTrigger {
                    button { class: "h-9 px-3 rounded-lg border border-border text-sm", "Hover me" }
                }
                HoverCardContent {
                    div { class: "flex flex-col gap-1",
                        span { class: "text-sm font-medium", "@username" }
                        span { class: "text-xs text-muted-foreground", "Bio shown on hover/focus." }
                    }
                }
            }
        }
    }
}
