//! ContextMenu — primitive-backed shadcn-style menu.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::context_menu::{
    ContextMenu as PrimitiveContextMenu, ContextMenuContent as PrimitiveContextMenuContent,
    ContextMenuItem as PrimitiveContextMenuItem, ContextMenuTrigger as PrimitiveContextMenuTrigger,
};

#[derive(Props, Clone, PartialEq)]
pub struct ContextMenuProps {
    #[props(default)]
    pub open: Option<bool>,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default)]
    pub on_open_change: Option<Callback<bool>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ContextMenu(props: ContextMenuProps) -> Element {
    rsx! {
        PrimitiveContextMenu {
            open: props.open,
            default_open: props.default_open,
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            on_open_change: move |open| {
                if let Some(callback) = &props.on_open_change {
                    callback.call(open);
                }
            },
            class: crate::cn::merge_slice(&["relative", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ContextMenuTriggerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ContextMenuTrigger(props: ContextMenuTriggerProps) -> Element {
    rsx! {
        PrimitiveContextMenuTrigger {
            class: props.class,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ContextMenuContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ContextMenuContent(props: ContextMenuContentProps) -> Element {
    rsx! {
        PrimitiveContextMenuContent {
            id: props.id,
            class: crate::cn::merge(format!(
                "fixed z-50 min-w-48 rounded-lg bg-popover text-popover-foreground border border-border shadow-md p-1 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ContextMenuItemProps {
    pub value: String,
    pub index: usize,
    #[props(default)]
    pub on_select: Option<Callback<String>>,
    #[props(default = false)]
    pub destructive: bool,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ContextMenuItem(props: ContextMenuItemProps) -> Element {
    let base = "flex cursor-pointer select-none items-center rounded-xl px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground transition-colors gap-2.5";
    let variant = if props.destructive {
        "text-destructive hover:bg-destructive/10"
    } else {
        ""
    };

    rsx! {
        PrimitiveContextMenuItem {
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            on_select: move |value| {
                if let Some(callback) = &props.on_select {
                    callback.call(value);
                }
            },
            class: crate::cn::merge_slice(&[base, variant, props.class.as_str()]),
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuSeparator() -> Element {
    rsx! {
        div { class: "bg-border/50 -mx-1 my-1 h-px" }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ContextMenuLabelProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ContextMenuLabel(props: ContextMenuLabelProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["text-muted-foreground px-3 py-2.5 text-xs", props.class.as_str()]),
            {props.children}
        }
    }
}

/// ContextMenu forced open showing items. (Note: positioned at fixed coords.)
#[story(category = "ContextMenu", name = "context menu default")]
pub fn context_menu_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[18rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm mb-3",
                    onclick: move |_| open.set(true),
                    "Open context menu"
                }
            }
            ContextMenu {
                open: open(),
                on_open_change: move |o| open.set(o),
                ContextMenuTrigger {
                    div { class: "h-24 rounded-lg border border-dashed border-border flex items-center justify-center text-sm text-muted-foreground",
                        "Right-click area"
                    }
                }
                ContextMenuContent {
                    ContextMenuLabel { "Actions" }
                    ContextMenuItem { value: "copy".to_string(), index: 0, "Copy" }
                    ContextMenuItem { value: "paste".to_string(), index: 1, "Paste" }
                    ContextMenuSeparator {}
                    ContextMenuItem { value: "delete".to_string(), index: 2, destructive: true, "Delete" }
                }
            }
        }
    }
}
