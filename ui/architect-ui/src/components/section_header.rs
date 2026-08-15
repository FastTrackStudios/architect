//! Section header — uppercase label with optional trailing action.
//!
//! Used to label groups of content in sidebars, panels, and lists.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Size variants for the section header.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum SectionHeaderSize {
    /// Tiny — `text-[8px]` with wide tracking. Used inside dropdowns/pickers.
    Small,
    /// Standard — `text-sm`. Used for dashboard/panel sections.
    #[default]
    Medium,
}

impl SectionHeaderSize {
    fn classes(self) -> &'static str {
        match self {
            Self::Small => "text-[8px] font-semibold uppercase tracking-[0.2em]",
            Self::Medium => "text-sm font-medium uppercase tracking-wider",
        }
    }
}

/// An uppercase section label with an optional trailing element (button, count, etc.).
///
/// ```rust,ignore
/// SectionHeader {
///     label: "Instances",
///     trailing: rsx! { Button { "Import" } },
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct SectionHeaderProps {
    /// Section label text (rendered uppercase by CSS).
    pub label: String,

    /// Size variant.
    #[props(default)]
    pub size: SectionHeaderSize,

    /// Optional trailing element (action button, badge, count).
    #[props(default)]
    pub trailing: Option<Element>,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn SectionHeader(props: SectionHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex items-center justify-between", props.class.as_str()]),

            span {
                class: format!("text-muted-foreground {}", props.size.classes()),
                "{props.label}"
            }

            if let Some(trailing) = &props.trailing {
                {trailing}
            }
        }
    }
}

#[story(category = "SectionHeader", name = "section_header_default")]
pub fn section_header_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3 w-80",
            SectionHeader { label: "Instances".to_string() }
            SectionHeader {
                label: "Recent".to_string(),
                trailing: rsx! { crate::components::Badge { variant: crate::components::BadgeVariant::Secondary, "3" } },
            }
            SectionHeader { label: "Tiny header".to_string(), size: SectionHeaderSize::Small }
        }
    }
}
