//! Item — flexible content row/card primitive.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ItemVariant {
    #[default]
    Default,
    Outline,
    Muted,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ItemSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemGroupProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemGroup(props: ItemGroupProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["grid gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemProps {
    #[props(default)]
    pub variant: ItemVariant,
    #[props(default)]
    pub size: ItemSize,
    #[props(default = false)]
    pub interactive: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Item(props: ItemProps) -> Element {
    let variant = match props.variant {
        ItemVariant::Default => "",
        ItemVariant::Outline => "border border-border bg-card shadow-xs",
        ItemVariant::Muted => "bg-muted/50",
    };
    let size = match props.size {
        ItemSize::Small => "gap-2 rounded-lg p-2",
        ItemSize::Medium => "gap-3 rounded-lg p-3",
        ItemSize::Large => "gap-4 rounded-xl p-4",
    };
    let interactive = if props.interactive {
        "cursor-pointer transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
    } else {
        ""
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[
                "group/item flex w-full min-w-0 items-start text-left",
                variant,
                size,
                interactive,
                props.class.as_str(),
            ]),
            tabindex: if props.interactive { "0" },
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemMediaProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemMedia(props: ItemMediaProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&[
                "flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted text-muted-foreground [&_svg]:size-5",
                props.class.as_str(),
            ]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemContentProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemContent(props: ItemContentProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["grid min-w-0 flex-1 gap-1", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemTitleProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemTitle(props: ItemTitleProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["truncate text-sm font-medium leading-none", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemDescriptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemDescription(props: ItemDescriptionProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["line-clamp-2 text-sm text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemActionsProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ItemActions(props: ItemActionsProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["ml-auto flex shrink-0 items-center gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[story(category = "Item", name = "item_default")]
pub fn item_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            ItemGroup {
                Item { variant: ItemVariant::Outline,
                    ItemMedia { lucide_dioxus::House { size: 18 } }
                    ItemContent {
                        ItemTitle { "Project Alpha" }
                        ItemDescription { "Outline item with media, title, description." }
                    }
                    ItemActions {
                        crate::components::Button { size: crate::components::ButtonSize::Small, "Open" }
                    }
                }
                Item { variant: ItemVariant::Muted, interactive: true,
                    ItemContent {
                        ItemTitle { "Interactive muted item" }
                        ItemDescription { "Hover to see accent." }
                    }
                }
            }
        }
    }
}
