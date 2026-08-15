//! Toolbar — primitive-backed grouped controls.

use dioxus::prelude::*;
use dioxus_primitives::toolbar::{
    Toolbar as PrimitiveToolbar, ToolbarButton as PrimitiveToolbarButton,
    ToolbarSeparator as PrimitiveToolbarSeparator,
};
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct ToolbarProps {
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub horizontal: bool,
    #[props(default)]
    pub aria_label: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Toolbar(props: ToolbarProps) -> Element {
    rsx! {
        PrimitiveToolbar {
            disabled: props.disabled,
            horizontal: props.horizontal,
            aria_label: props.aria_label,
            class: crate::cn::merge_slice(
                &["inline-flex items-center gap-1 rounded-lg border border-border bg-card p-1 text-card-foreground shadow-xs", props.class.as_str()],
            ),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToolbarButtonProps {
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub on_click: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ToolbarButton(props: ToolbarButtonProps) -> Element {
    rsx! {
        PrimitiveToolbarButton {
            index: props.index,
            disabled: props.disabled,
            on_click: move |_| {
                if let Some(callback) = &props.on_click {
                    callback.call(());
                }
            },
            class: crate::cn::merge_slice(&["inline-flex size-8 items-center justify-center rounded-md text-sm hover:bg-muted disabled:opacity-50", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToolbarSeparatorProps {
    #[props(default)]
    pub horizontal: Option<bool>,
    #[props(default = false)]
    pub decorative: bool,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn ToolbarSeparator(props: ToolbarSeparatorProps) -> Element {
    rsx! {
        PrimitiveToolbarSeparator {
            horizontal: props.horizontal,
            decorative: props.decorative,
            class: crate::cn::merge_slice(&["shrink-0 bg-border data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-6 data-[orientation=vertical]:w-px", props.class.as_str()]),
        }
    }
}

#[story(category = "Toolbar", name = "default")]
pub fn toolbar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Toolbar { aria_label: "Formatting".to_string(),
                ToolbarButton { index: 0, lucide_dioxus::Bold { size: 14 } }
                ToolbarButton { index: 1, lucide_dioxus::Italic { size: 14 } }
                ToolbarButton { index: 2, lucide_dioxus::Underline { size: 14 } }
                ToolbarSeparator {}
                ToolbarButton { index: 3, lucide_dioxus::AlignLeft { size: 14 } }
                ToolbarButton { index: 4, lucide_dioxus::AlignCenter { size: 14 } }
                ToolbarButton { index: 5, lucide_dioxus::AlignRight { size: 14 } }
            }
        }
    }
}
