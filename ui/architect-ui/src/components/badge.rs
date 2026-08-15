//! Badge — shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Visual variant for the badge.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Destructive,
    Outline,
    Ghost,
}

impl BadgeVariant {
    // shadcn v4 maia: cn-badge-variant-*
    fn classes(self) -> &'static str {
        match self {
            Self::Default => "bg-primary text-primary-foreground",
            Self::Secondary => "bg-secondary text-secondary-foreground",
            Self::Destructive => "bg-destructive/10 text-destructive dark:bg-destructive/20",
            Self::Outline => "border-border text-foreground bg-input/30",
            Self::Ghost => "hover:bg-muted hover:text-muted-foreground dark:hover:bg-muted/50",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BadgeProps {
    #[props(default)]
    pub variant: BadgeVariant,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-badge
#[component]
pub fn Badge(props: BadgeProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge(format!(
                "inline-flex items-center h-5 gap-1 rounded-full border border-transparent px-2 py-0.5 text-xs font-medium transition-all [&>svg]:size-3 {} {}",
                props.variant.classes(),
                props.class
            )),
            {props.children}
        }
    }
}

/// All `BadgeVariant` values rendered with a configurable label.
#[story(category = "Badge", name = "variants", knobs(label = "Badge"))]
pub fn badge_variants(label: &str) -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-wrap gap-2",
            Badge { variant: BadgeVariant::Default, "{label}" }
            Badge { variant: BadgeVariant::Secondary, "{label}" }
            Badge { variant: BadgeVariant::Destructive, "{label}" }
            Badge { variant: BadgeVariant::Outline, "{label}" }
            Badge { variant: BadgeVariant::Ghost, "{label}" }
        }
    }
}
