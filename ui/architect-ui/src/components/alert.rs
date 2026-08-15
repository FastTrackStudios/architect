//! Alert — shadcn v4 maia style.

use architect_story_runtime::story;
use dioxus::prelude::*;

/// Alert visual variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AlertVariant {
    #[default]
    Default,
    Destructive,
}

impl AlertVariant {
    fn classes(self) -> &'static str {
        match self {
            Self::Default => "bg-card text-card-foreground",
            Self::Destructive => "text-destructive bg-card",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertProps {
    #[props(default)]
    pub variant: AlertVariant,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: cn-alert
#[component]
pub fn Alert(props: AlertProps) -> Element {
    let base =
        "grid gap-0.5 rounded-lg border border-border px-4 py-3 text-left text-sm [&>svg]:size-4";
    let variant_class = props.variant.classes();

    rsx! {
        div {
            role: "alert",
            class: crate::cn::merge_slice(&[base, variant_class, props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertTitleProps {
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// Alert title line.
#[component]
pub fn AlertTitle(props: AlertTitleProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDescriptionProps {
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// Alert description text.
#[component]
pub fn AlertDescription(props: AlertDescriptionProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["text-muted-foreground text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[story(category = "Alert", name = "default")]
pub fn alert_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3",
            Alert {
                AlertTitle { "Heads up" }
                AlertDescription { "Default alert with description." }
            }
        }
    }
}

#[story(category = "Alert", name = "variants")]
pub fn alert_variants() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3",
            Alert {
                AlertTitle { "Default" }
                AlertDescription { "Standard alert." }
            }
            Alert { variant: AlertVariant::Destructive,
                AlertTitle { "Error" }
                AlertDescription { "Something went wrong." }
            }
        }
    }
}
