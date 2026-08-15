//! Status indicators — dots and badges for conveying state.
//!
//! `StatusDot` — small colored circle used as a leading indicator in lists.
//! `StatusBadge` — pill with tinted background, icon, and label text.

use dioxus::prelude::*;
use architect_story_runtime::story;

// ---------------------------------------------------------------------------
// StatusDot
// ---------------------------------------------------------------------------

/// Semantic color for a status dot.
#[derive(Clone, PartialEq, Default)]
pub enum StatusDotColor {
    /// Green — running, connected, healthy.
    Success,
    /// Yellow — connecting, warning, degraded.
    Warning,
    /// Red — error, stopped, disconnected.
    Danger,
    /// Gray — inactive, unknown.
    #[default]
    Neutral,
    /// Custom CSS color value (used with inline `style`). Useful for
    /// domain-specific colors like module-type indicators.
    Custom(String),
}

impl StatusDotColor {
    fn class(&self) -> &str {
        match self {
            Self::Success => "bg-green-500",
            Self::Warning => "bg-yellow-500",
            Self::Danger => "bg-red-500",
            Self::Neutral => "bg-muted-foreground/40",
            Self::Custom(_) => "",
        }
    }

    fn style(&self) -> String {
        match self {
            Self::Custom(color) => format!("background-color: {color};"),
            _ => String::new(),
        }
    }
}

/// Size variants for the status dot.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum StatusDotSize {
    /// 1.5 (6px) — for tight lists.
    Small,
    /// 2.5 (10px) — standard size.
    #[default]
    Medium,
}

impl StatusDotSize {
    fn classes(self) -> &'static str {
        match self {
            Self::Small => "w-1.5 h-1.5",
            Self::Medium => "w-2.5 h-2.5",
        }
    }
}

/// A small colored circle indicating status.
#[derive(Props, Clone, PartialEq)]
pub struct StatusDotProps {
    /// Semantic color.
    #[props(default)]
    pub color: StatusDotColor,

    /// Dot size.
    #[props(default)]
    pub size: StatusDotSize,

    /// Shape — `true` for rounded-full (circle), `false` for rounded-sm (square).
    #[props(default = true)]
    pub round: bool,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn StatusDot(props: StatusDotProps) -> Element {
    let shape = if props.round {
        "rounded-full"
    } else {
        "rounded-sm"
    };

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "{} {} {} flex-shrink-0 {}",
                props.size.classes(),
                props.color.class(),
                shape,
                props.class
            )),
            style: "{props.color.style()}",
        }
    }
}

// ---------------------------------------------------------------------------
// StatusBadge
// ---------------------------------------------------------------------------

/// Semantic variant for a status badge.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum StatusBadgeVariant {
    /// Green tint — connected, healthy.
    Success,
    /// Yellow tint — connecting, degraded.
    Warning,
    /// Red tint — error, disconnected.
    Danger,
    /// Gray tint — inactive.
    #[default]
    Neutral,
}

impl StatusBadgeVariant {
    fn classes(self) -> (&'static str, &'static str) {
        match self {
            Self::Success => ("bg-green-500/20", "text-green-500"),
            Self::Warning => ("bg-yellow-500/20", "text-yellow-500"),
            Self::Danger => ("bg-red-500/20", "text-red-500"),
            Self::Neutral => ("bg-muted", "text-muted-foreground"),
        }
    }
}

/// A pill-shaped badge with tinted background, optional icon, and label.
///
/// ```rust,ignore
/// StatusBadge {
///     variant: StatusBadgeVariant::Success,
///     label: "Connected",
///     icon: rsx! { CircleCheck { size: 16 } },
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct StatusBadgeProps {
    /// Visual variant controlling background/text color.
    #[props(default)]
    pub variant: StatusBadgeVariant,

    /// Label text.
    pub label: String,

    /// Optional leading icon element.
    #[props(default)]
    pub icon: Option<Element>,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn StatusBadge(props: StatusBadgeProps) -> Element {
    let (bg, text) = props.variant.classes();

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "flex items-center gap-2 px-3 py-1.5 rounded-full text-sm font-medium {bg} {text} {}",
                props.class
            )),

            if let Some(icon) = &props.icon {
                {icon}
            }

            span { "{props.label}" }
        }
    }
}

/// Every `StatusDotColor` paired with every `StatusBadgeVariant` in a labelled grid.
#[story(category = "Status", name = "dots-and-badges")]
pub fn status_dots_and_badges() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-6",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "StatusDot — colors" }
                div { class: "flex items-center gap-6",
                    div { class: "flex items-center gap-2",
                        StatusDot { color: StatusDotColor::Success }
                        span { class: "text-sm", "Success" }
                    }
                    div { class: "flex items-center gap-2",
                        StatusDot { color: StatusDotColor::Warning }
                        span { class: "text-sm", "Warning" }
                    }
                    div { class: "flex items-center gap-2",
                        StatusDot { color: StatusDotColor::Danger }
                        span { class: "text-sm", "Danger" }
                    }
                    div { class: "flex items-center gap-2",
                        StatusDot { color: StatusDotColor::Neutral }
                        span { class: "text-sm", "Neutral" }
                    }
                    div { class: "flex items-center gap-2",
                        StatusDot { color: StatusDotColor::Custom("#a855f7".to_string()) }
                        span { class: "text-sm", "Custom" }
                    }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "StatusDot — sizes" }
                div { class: "flex items-center gap-4",
                    StatusDot { color: StatusDotColor::Success, size: StatusDotSize::Small }
                    StatusDot { color: StatusDotColor::Success, size: StatusDotSize::Medium }
                    StatusDot { color: StatusDotColor::Success, round: false }
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "StatusBadge — variants" }
                div { class: "flex items-center gap-2 flex-wrap",
                    StatusBadge { variant: StatusBadgeVariant::Success, label: "Connected".to_string() }
                    StatusBadge { variant: StatusBadgeVariant::Warning, label: "Degraded".to_string() }
                    StatusBadge { variant: StatusBadgeVariant::Danger, label: "Error".to_string() }
                    StatusBadge { variant: StatusBadgeVariant::Neutral, label: "Offline".to_string() }
                }
            }
        }
    }
}

#[story(category = "Status", name = "default")]
pub fn status_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3",
            div { class: "flex items-center gap-3",
                StatusDot { color: StatusDotColor::Success }
                StatusDot { color: StatusDotColor::Warning }
                StatusDot { color: StatusDotColor::Danger }
                StatusDot { color: StatusDotColor::Neutral }
            }
            div { class: "flex items-center gap-2",
                StatusBadge { variant: StatusBadgeVariant::Success, label: "Connected".to_string() }
                StatusBadge { variant: StatusBadgeVariant::Warning, label: "Degraded".to_string() }
                StatusBadge { variant: StatusBadgeVariant::Danger, label: "Error".to_string() }
                StatusBadge { variant: StatusBadgeVariant::Neutral, label: "Offline".to_string() }
            }
        }
    }
}
