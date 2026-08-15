//! Menubar — primitive-backed horizontal menu bar.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::menubar::{
    Menubar as PrimitiveMenubar, MenubarContent as PrimitiveMenubarContent,
    MenubarItem as PrimitiveMenubarItem, MenubarMenu as PrimitiveMenubarMenu,
    MenubarTrigger as PrimitiveMenubarTrigger,
};

#[derive(Props, Clone, PartialEq)]
pub struct MenubarProps {
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Menubar(props: MenubarProps) -> Element {
    rsx! {
        PrimitiveMenubar {
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            class: crate::cn::merge(format!(
                "inline-flex items-center h-9 rounded-lg border border-border bg-card p-1 gap-1 text-card-foreground shadow-xs {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarMenuProps {
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarMenu(props: MenubarMenuProps) -> Element {
    rsx! {
        PrimitiveMenubarMenu {
            index: props.index,
            disabled: props.disabled,
            class: crate::cn::merge_slice(&["relative", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarTriggerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarTrigger(props: MenubarTriggerProps) -> Element {
    rsx! {
        PrimitiveMenubarTrigger {
            class: crate::cn::merge(format!(
                "inline-flex items-center justify-center rounded-xl px-2 py-0.5 text-sm font-medium hover:bg-muted cursor-pointer select-none transition-colors data-[state=open]:bg-muted {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarContent(props: MenubarContentProps) -> Element {
    rsx! {
        PrimitiveMenubarContent {
            id: props.id,
            class: crate::cn::merge(format!(
                "absolute left-0 top-full z-[100] mt-1 min-w-48 rounded-lg bg-popover text-popover-foreground border border-border shadow-md p-1 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarItemProps {
    pub value: String,
    pub index: usize,
    #[props(default)]
    pub on_select: Option<Callback<String>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarItem(props: MenubarItemProps) -> Element {
    rsx! {
        PrimitiveMenubarItem {
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            on_select: move |value| {
                if let Some(callback) = &props.on_select {
                    callback.call(value);
                }
            },
            class: crate::cn::merge(format!(
                "flex cursor-pointer select-none items-center rounded-xl px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground transition-colors gap-2.5 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarSeparatorProps {
    #[props(default)]
    pub class: String,
}

#[component]
pub fn MenubarSeparator(props: MenubarSeparatorProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["bg-border/50 -mx-1 my-1 h-px", props.class.as_str()]),
            role: "separator",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarLabelProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarLabel(props: MenubarLabelProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["text-muted-foreground px-3 py-2.5 text-xs", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MenubarShortcutProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn MenubarShortcut(props: MenubarShortcutProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&["ml-auto text-xs tracking-widest text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Menubar with a single menu rendered. The menubar itself is always visible;
/// individual menus open on click and are not forced open here.
#[story(category = "Menubar", name = "menubar default")]
pub fn menubar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Menubar {
                MenubarMenu { index: 0,
                    MenubarTrigger { "File" }
                    MenubarContent {
                        MenubarItem { value: "new".to_string(), index: 0, "New" }
                        MenubarItem { value: "open".to_string(), index: 1, "Open" }
                        MenubarSeparator {}
                        MenubarItem { value: "quit".to_string(), index: 2, "Quit" }
                    }
                }
                MenubarMenu { index: 1,
                    MenubarTrigger { "Edit" }
                    MenubarContent {
                        MenubarItem { value: "undo".to_string(), index: 0, "Undo" }
                        MenubarItem { value: "redo".to_string(), index: 1, "Redo" }
                    }
                }
                MenubarMenu { index: 2,
                    MenubarTrigger { "View" }
                    MenubarContent {
                        MenubarItem { value: "zoom_in".to_string(), index: 0, "Zoom in" }
                        MenubarItem { value: "zoom_out".to_string(), index: 1, "Zoom out" }
                    }
                }
            }
        }
    }
}
