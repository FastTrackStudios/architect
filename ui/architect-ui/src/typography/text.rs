//! Text — body text with semantic variants.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Text style variant.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum TextVariant {
    /// Standard body text.
    #[default]
    Body,
    /// Smaller secondary text.
    Small,
    /// De-emphasized muted text.
    Muted,
    /// Inline code.
    Code,
}

impl TextVariant {
    fn classes(self) -> &'static str {
        match self {
            Self::Body => "text-sm",
            Self::Small => "text-xs",
            Self::Muted => "text-sm text-muted-foreground",
            Self::Code => {
                "relative rounded bg-muted px-[0.3rem] py-[0.2rem] font-mono text-sm font-semibold"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TextProps {
    /// Text variant.
    #[props(default)]
    pub variant: TextVariant,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,

    pub children: Element,
}

#[component]
pub fn Text(props: TextProps) -> Element {
    let class = crate::cn::merge_slice(&[props.variant.classes(), props.class.as_str()]);

    match props.variant {
        TextVariant::Code => rsx! { code { class, {props.children} } },
        _ => rsx! { p { class, {props.children} } },
    }
}

#[story(category = "Typography", name = "text_variants")]
pub fn text_variants() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-2",
            Text { "Body text — the standard paragraph style." }
            Text { variant: TextVariant::Small, "Small text — secondary detail." }
            Text { variant: TextVariant::Muted, "Muted text — de-emphasized." }
            Text { variant: TextVariant::Code, "Code text" }
        }
    }
}
