//! Kbd — keyboard shortcut badge, shadcn v4 maia style.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct KbdProps {
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: cn-kbd
#[component]
pub fn Kbd(props: KbdProps) -> Element {
    let base = "inline-flex items-center justify-center bg-muted text-muted-foreground h-5 w-fit min-w-5 gap-1 rounded-sm px-1 font-sans text-xs font-medium [&_svg:not([class*='size-'])]:size-3";

    rsx! {
        kbd {
            class: crate::cn::merge_slice(&[base, props.class.as_str()]),
            {props.children}
        }
    }
}

#[story(category = "Kbd", name = "default")]
pub fn kbd_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-2",
            Kbd { "Ctrl" }
            span { class: "text-muted-foreground", "+" }
            Kbd { "K" }
            span { class: "text-muted-foreground ml-3", "or" }
            Kbd { "Enter" }
            Kbd { "Esc" }
        }
    }
}
