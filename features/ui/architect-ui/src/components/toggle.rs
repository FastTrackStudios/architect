//! Toggle — shadcn v4 maia style.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::toggle::Toggle as PrimitiveToggle;

/// Variant for the toggle button.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum ToggleVariant {
    #[default]
    Default,
    Outline,
}

/// Size for the toggle button.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum ToggleSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ToggleSize {
    fn classes(self) -> &'static str {
        match self {
            Self::Small => "h-8 min-w-8 px-3",
            Self::Medium => "h-9 min-w-9 px-3",
            Self::Large => "h-10 min-w-10 px-4",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToggleProps {
    pub pressed: bool,
    pub on_toggle: Callback<bool>,
    #[props(default)]
    pub variant: ToggleVariant,
    #[props(default)]
    pub size: ToggleSize,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-toggle
#[component]
pub fn Toggle(props: ToggleProps) -> Element {
    let base = "inline-flex items-center justify-center font-medium transition-colors gap-1 rounded-full text-sm [&_svg:not([class*='size-'])]:size-4 hover:text-foreground";

    let pressed_class = if props.pressed { "bg-muted" } else { "" };

    let variant_class = match props.variant {
        ToggleVariant::Default => "bg-transparent",
        ToggleVariant::Outline => "border-input hover:bg-muted border bg-transparent",
    };

    let size_class = props.size.classes();

    let disabled_class = if props.disabled {
        "pointer-events-none opacity-50"
    } else {
        ""
    };

    let pressed = props.pressed;

    let class = crate::cn::merge(format!(
        "{base} {variant_class} {size_class} {pressed_class} {disabled_class} {}",
        props.class
    ));

    rsx! {
        PrimitiveToggle {
            pressed: Some(pressed),
            disabled: props.disabled,
            class,
            on_pressed_change: move |next| {
                props.on_toggle.call(next);
            },
            {props.children}
        }
    }
}

/// Default toggle button.
#[story(category = "Toggle", name = "default")]
pub fn toggle_default() -> Element {
    let mut pressed = use_signal(|| false);
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Toggle {
                pressed: pressed(),
                on_toggle: move |next| pressed.set(next),
                "Bold"
            }
        }
    }
}

/// Toggle variants and sizes side-by-side.
#[story(category = "Toggle", name = "variants")]
pub fn toggle_variants() -> Element {
    let mut a = use_signal(|| false);
    let mut b = use_signal(|| true);
    let mut c = use_signal(|| false);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-3",
            Toggle {
                pressed: a(),
                on_toggle: move |n| a.set(n),
                size: ToggleSize::Small,
                "S"
            }
            Toggle {
                pressed: b(),
                on_toggle: move |n| b.set(n),
                size: ToggleSize::Medium,
                "M"
            }
            Toggle {
                pressed: c(),
                on_toggle: move |n| c.set(n),
                variant: ToggleVariant::Outline,
                size: ToggleSize::Large,
                "L · Outline"
            }
        }
    }
}
