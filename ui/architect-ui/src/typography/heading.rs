//! Heading — H1 through H6 with consistent typography styles.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// Heading level.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum HeadingLevel {
    H1,
    H2,
    #[default]
    H3,
    H4,
    H5,
    H6,
}

impl HeadingLevel {
    fn classes(self) -> &'static str {
        match self {
            Self::H1 => "scroll-m-20 text-4xl font-extrabold tracking-tight lg:text-5xl",
            Self::H2 => "scroll-m-20 text-3xl font-semibold tracking-tight",
            Self::H3 => "scroll-m-20 text-2xl font-semibold tracking-tight",
            Self::H4 => "scroll-m-20 text-xl font-semibold tracking-tight",
            Self::H5 => "scroll-m-20 text-lg font-semibold tracking-tight",
            Self::H6 => "scroll-m-20 text-base font-semibold tracking-tight",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct HeadingProps {
    /// Heading level (H1–H6).
    #[props(default)]
    pub level: HeadingLevel,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,

    pub children: Element,
}

#[component]
pub fn Heading(props: HeadingProps) -> Element {
    let class = crate::cn::merge_slice(&[props.level.classes(), props.class.as_str()]);

    match props.level {
        HeadingLevel::H1 => rsx! { h1 { class, {props.children} } },
        HeadingLevel::H2 => rsx! { h2 { class, {props.children} } },
        HeadingLevel::H3 => rsx! { h3 { class, {props.children} } },
        HeadingLevel::H4 => rsx! { h4 { class, {props.children} } },
        HeadingLevel::H5 => rsx! { h5 { class, {props.children} } },
        HeadingLevel::H6 => rsx! { h6 { class, {props.children} } },
    }
}

#[story(category = "Typography", name = "heading_levels")]
pub fn heading_levels() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-2",
            Heading { level: HeadingLevel::H1, "H1 Heading" }
            Heading { level: HeadingLevel::H2, "H2 Heading" }
            Heading { level: HeadingLevel::H3, "H3 Heading" }
            Heading { level: HeadingLevel::H4, "H4 Heading" }
            Heading { level: HeadingLevel::H5, "H5 Heading" }
            Heading { level: HeadingLevel::H6, "H6 Heading" }
        }
    }
}
