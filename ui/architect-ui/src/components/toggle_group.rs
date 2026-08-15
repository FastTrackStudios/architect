//! Toggle group — shadcn v4 maia style.

use dioxus::prelude::*;
use dioxus_primitives::toggle_group::{
    ToggleGroup as PrimitiveToggleGroup, ToggleItem as PrimitiveToggleItem,
};
use architect_story_runtime::story;
use std::collections::HashSet;

#[derive(Props, Clone, PartialEq)]
pub struct ToggleGroupProps {
    #[props(default)]
    pub pressed: Option<HashSet<usize>>,
    #[props(default)]
    pub default_pressed: HashSet<usize>,
    #[props(default)]
    pub on_pressed_change: Option<Callback<HashSet<usize>>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub allow_multiple_pressed: bool,
    #[props(default = true)]
    pub horizontal: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-toggle-group
#[component]
pub fn ToggleGroup(props: ToggleGroupProps) -> Element {
    rsx! {
        PrimitiveToggleGroup {
            pressed: props.pressed,
            default_pressed: props.default_pressed,
            disabled: props.disabled,
            allow_multiple_pressed: props.allow_multiple_pressed,
            horizontal: props.horizontal,
            roving_loop: props.roving_loop,
            on_pressed_change: move |pressed| {
                if let Some(callback) = &props.on_pressed_change {
                    callback.call(pressed);
                }
            },
            class: crate::cn::merge_slice(&["inline-flex items-center gap-1", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToggleGroupItemProps {
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-toggle-group-item
#[component]
pub fn ToggleGroupItem(props: ToggleGroupItemProps) -> Element {
    rsx! {
        PrimitiveToggleItem {
            index: props.index,
            disabled: props.disabled,
            class: props.class,
            {props.children}
        }
    }
}

/// Toggle group selection variants: single vs multi-selection.
#[story(category = "ToggleGroup", name = "variants")]
pub fn toggle_group_variants() -> Element {
    let item_class = "inline-flex items-center justify-center h-9 px-3 text-sm rounded-md border border-input bg-transparent hover:bg-muted data-[pressed=true]:bg-primary data-[pressed=true]:text-primary-foreground transition-colors".to_string();
    let mut single_default = HashSet::new();
    single_default.insert(1usize);
    let mut multi_default = HashSet::new();
    multi_default.insert(0usize);
    multi_default.insert(2usize);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "Single (only one pressed)" }
                ToggleGroup {
                    default_pressed: single_default,
                    allow_multiple_pressed: false,
                    ToggleGroupItem { index: 0, class: item_class.clone(), "Left" }
                    ToggleGroupItem { index: 1, class: item_class.clone(), "Center" }
                    ToggleGroupItem { index: 2, class: item_class.clone(), "Right" }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "Multi (any combination)" }
                ToggleGroup {
                    default_pressed: multi_default,
                    allow_multiple_pressed: true,
                    ToggleGroupItem { index: 0, class: item_class.clone(), "Bold" }
                    ToggleGroupItem { index: 1, class: item_class.clone(), "Italic" }
                    ToggleGroupItem { index: 2, class: item_class.clone(), "Underline" }
                }
            }
        }
    }
}

/// Default toggle group with three items.
#[story(category = "ToggleGroup", name = "default")]
pub fn toggle_group_default() -> Element {
    let item_class = "inline-flex items-center justify-center h-9 px-3 text-sm rounded-md border border-input bg-transparent hover:bg-muted data-[pressed=true]:bg-primary data-[pressed=true]:text-primary-foreground transition-colors".to_string();
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            ToggleGroup {
                ToggleGroupItem { index: 0, class: item_class.clone(), "Left" }
                ToggleGroupItem { index: 1, class: item_class.clone(), "Center" }
                ToggleGroupItem { index: 2, class: item_class.clone(), "Right" }
            }
        }
    }
}
