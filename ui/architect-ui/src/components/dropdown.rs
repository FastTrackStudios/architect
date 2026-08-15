//! Dropdown menu — primitive-backed shadcn-style menu.

use dioxus::prelude::*;
use dioxus_primitives::dropdown_menu::{
    DropdownMenu as PrimitiveDropdown, DropdownMenuContent as PrimitiveDropdownContent,
    DropdownMenuItem as PrimitiveDropdownItem, DropdownMenuTrigger as PrimitiveDropdownTrigger,
};
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct DropdownProps {
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
pub fn Dropdown(props: DropdownProps) -> Element {
    let disabled_class = if props.disabled {
        "opacity-50 pointer-events-none"
    } else {
        ""
    };

    rsx! {
        PrimitiveDropdown {
            open: props.open,
            default_open: props.default_open,
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            on_open_change: move |open| {
                if let Some(callback) = &props.on_open_change {
                    callback.call(open);
                }
            },
            class: crate::cn::merge_slice(&["relative inline-block text-left", disabled_class, props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownTriggerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn DropdownTrigger(props: DropdownTriggerProps) -> Element {
    rsx! {
        PrimitiveDropdownTrigger {
            class: crate::cn::merge_slice(&["cursor-pointer", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DropdownSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownContentProps {
    #[props(default)]
    pub id: Option<String>,
    /// Cross-axis alignment: `"start"` (default) / `"center"` / `"end"`.
    #[props(default = String::from("start"))]
    pub align: String,
    /// Which side of the trigger to open on: `"bottom"` (default) /
    /// `"top"` / `"right"` / `"left"`. Use `"top"` for triggers near the
    /// bottom of a scroll container so the menu doesn't run off screen.
    #[props(default = String::from("bottom"))]
    pub side: String,
    #[props(default = String::from("w-56"))]
    pub width: String,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn DropdownContent(props: DropdownContentProps) -> Element {
    let anchor = match (props.side.as_str(), props.align.as_str()) {
        ("bottom", "start") => "top-full mt-2 left-0 origin-top-left",
        ("bottom", "center") => "top-full mt-2 left-1/2 -translate-x-1/2 origin-top",
        ("bottom", "end") => "top-full mt-2 right-0 origin-top-right",
        ("top", "start") => "bottom-full mb-2 left-0 origin-bottom-left",
        ("top", "center") => "bottom-full mb-2 left-1/2 -translate-x-1/2 origin-bottom",
        ("top", "end") => "bottom-full mb-2 right-0 origin-bottom-right",
        ("right", "start") => "left-full ml-2 top-0 origin-top-left",
        ("right", "center") => "left-full ml-2 top-1/2 -translate-y-1/2 origin-left",
        ("right", "end") => "left-full ml-2 bottom-0 origin-bottom-left",
        ("left", "start") => "right-full mr-2 top-0 origin-top-right",
        ("left", "center") => "right-full mr-2 top-1/2 -translate-y-1/2 origin-right",
        ("left", "end") => "right-full mr-2 bottom-0 origin-bottom-right",
        // Unknown combos fall back to bottom-start.
        _ => "top-full mt-2 left-0 origin-top-left",
    };

    rsx! {
        PrimitiveDropdownContent {
            id: props.id,
            class: crate::cn::merge(format!(
                "absolute z-[100] min-w-48 rounded-lg bg-popover text-popover-foreground border border-border shadow-md p-1 {} {anchor} {}",
                props.width, props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownItemProps {
    pub value: String,
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub destructive: bool,
    #[props(default)]
    pub on_select: Option<Callback<String>>,
    #[props(default)]
    pub icon: Option<Element>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn DropdownItem(props: DropdownItemProps) -> Element {
    let variant_class = if props.destructive {
        "text-destructive hover:bg-destructive/10"
    } else {
        "hover:bg-accent hover:text-accent-foreground"
    };

    rsx! {
        PrimitiveDropdownItem::<String> {
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            on_select: move |value| {
                if let Some(callback) = &props.on_select {
                    callback.call(value);
                }
            },
            class: crate::cn::merge(format!(
                "relative flex cursor-pointer select-none items-center rounded-xl px-3 py-2 text-sm transition-colors gap-2.5 [&_svg:not([class*='size-'])]:size-4 {variant_class} {}",
                props.class
            )),
            if let Some(icon) = &props.icon {
                span { class: "flex-shrink-0", {icon} }
            }
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownLabelProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn DropdownLabel(props: DropdownLabelProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["text-muted-foreground px-3 py-2.5 text-xs", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownSeparatorProps {
    #[props(default)]
    pub class: String,
}

#[component]
pub fn DropdownSeparator(props: DropdownSeparatorProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["bg-border/50 -mx-1 my-1 h-px", props.class.as_str()]),
            role: "separator",
        }
    }
}

/// Dropdown forced open with a few menu items.
#[story(category = "Dropdown", name = "dropdown default")]
pub fn dropdown_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-start justify-center min-h-[18rem]",
            Dropdown {
                open: open(),
                on_open_change: move |o| open.set(o),
                DropdownTrigger {
                    button { class: "h-9 px-3 rounded-lg border border-border text-sm", "Open menu" }
                }
                DropdownContent {
                    DropdownLabel { "Account" }
                    DropdownItem { value: "profile".to_string(), index: 0, "Profile" }
                    DropdownItem { value: "settings".to_string(), index: 1, "Settings" }
                    DropdownSeparator {}
                    DropdownItem { value: "logout".to_string(), index: 2, destructive: true, "Sign out" }
                }
            }
        }
    }
}
