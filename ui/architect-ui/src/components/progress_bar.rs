//! ProgressBar — colored fill bar with optional label overlay.
//!
//! Supports two orientations:
//! - **Horizontal** (default): a compact bar with overlaid text label, used in song/section cards.
//! - **Vertical**: a thin fill indicator for sidebars.
//!
//! This is distinct from the lumen-blocks `Progress` re-export, which is a plain
//! semantic `<progress>` element.

use architect_story_runtime::story;
use dioxus::prelude::*;

/// Orientation for the progress bar.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum ProgressBarOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    /// Progress percentage (0–100).
    pub progress: f64,
    /// Fill color (CSS hex or color value).
    pub bright_color: String,
    /// Background color (CSS hex or color value).
    pub muted_color: String,
    /// Bar orientation.
    #[props(default)]
    pub orientation: ProgressBarOrientation,
    /// Whether to show a selection ring.
    #[props(default = false)]
    pub is_selected: bool,
    /// When true, hides the fill and shows an inactive state.
    #[props(default = false)]
    pub is_inactive: bool,
    /// Forces a dark background with a colored stripe on the right (horizontal only).
    #[props(default = false)]
    pub always_black_bg: bool,
    /// Text label overlaid on the bar (horizontal only).
    #[props(default)]
    pub label: Option<String>,
    /// Italic comment shown after the label (horizontal only).
    #[props(default)]
    pub comment: Option<String>,
    /// CSS height — vertical: required; horizontal: defaults to `"3rem"`.
    #[props(default)]
    pub height: Option<String>,
    /// Click handler.
    #[props(default)]
    pub on_click: Option<Callback<MouseEvent>>,
    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

/// Colored progress bar with horizontal and vertical variants.
#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    match props.orientation {
        ProgressBarOrientation::Horizontal => render_horizontal(props),
        ProgressBarOrientation::Vertical => render_vertical(props),
    }
}

fn render_horizontal(props: ProgressBarProps) -> Element {
    let height = props.height.as_deref().unwrap_or("3rem");
    let selected_class = if props.is_selected {
        "ring-2 ring-primary"
    } else {
        ""
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[
                "relative rounded-lg overflow-hidden cursor-pointer transition-all",
                selected_class,
                props.class.as_str(),
            ]),
            style: format!("height: {};", height),
            onclick: move |e| {
                if let Some(callback) = &props.on_click {
                    callback.call(e);
                }
            },
            // Background
            div {
                class: "absolute inset-0",
                style: if props.always_black_bg || props.is_inactive {
                    "background-color: var(--color-card);".to_string()
                } else {
                    format!("background-color: {}; opacity: 0.3;", props.muted_color)
                },
            }
            // Filled portion
            if !props.is_inactive {
                div {
                    class: "absolute left-0 top-0 h-full transition-all duration-100 ease-linear",
                    style: format!(
                        "width: {}%; background-color: {}; opacity: 0.8;",
                        props.progress.clamp(0.0, 100.0),
                        props.bright_color
                    ),
                }
            }
            // Colored stripe on right (when always_black_bg)
            if props.always_black_bg {
                div {
                    class: "absolute right-0 top-0 bottom-0 w-1",
                    style: format!("background-color: {};", props.bright_color),
                }
            }
            // Label text
            if props.label.is_some() || props.comment.is_some() {
                div {
                    class: "absolute inset-0 flex items-center px-3 text-sm font-medium pointer-events-none",
                    style: if props.always_black_bg {
                        // Theme-adaptive: card-foreground reads correctly on the
                        // card-token background in BOTH light (Task) and dark
                        // (desktop Signal stage) themes — the old hardcoded
                        // `white` rendered as an invisible white box in light mode.
                        "color: var(--color-card-foreground); text-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);".to_string()
                    } else if props.is_inactive {
                        format!("color: {};", props.bright_color)
                    } else {
                        // Theme-adaptive so the label reads on the subtle
                        // muted-color tint used by the non-stage (plain) selection.
                        "color: var(--color-foreground); text-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);".to_string()
                    },
                    if let Some(ref label) = props.label {
                        span { "{label}" }
                    }
                    if let Some(ref comment_text) = props.comment {
                        span {
                            class: "ml-2 italic opacity-70",
                            "{comment_text}"
                        }
                    }
                }
            }
        }
    }
}

fn render_vertical(props: ProgressBarProps) -> Element {
    let height = props.height.as_deref().unwrap_or("4rem");

    rsx! {
        div {
            class: crate::cn::merge_slice(&["relative w-1 flex-shrink-0 mr-2", props.class.as_str()]),
            style: format!("height: {};", height),
            // Background (unfilled)
            div {
                class: "absolute inset-0 rounded-full",
                style: format!(
                    "background-color: {}; opacity: 0.3;",
                    props.muted_color
                ),
            }
            // Filled portion
            div {
                class: "absolute top-0 left-0 right-0 rounded-full transition-all duration-300 ease-in-out",
                style: format!(
                    "height: {}%; background-color: {}; opacity: 0.8;",
                    props.progress.clamp(0.0, 100.0),
                    props.bright_color
                ),
            }
        }
    }
}

#[story(category = "ProgressBar", name = "progress_bar_default")]
pub fn progress_bar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3 w-96",
            ProgressBar {
                progress: 65.0,
                bright_color: "#3b82f6".to_string(),
                muted_color: "#1e3a8a".to_string(),
                label: "Loading".to_string(),
            }
            ProgressBar {
                progress: 35.0,
                bright_color: "#10b981".to_string(),
                muted_color: "#064e3b".to_string(),
                label: "Render".to_string(),
                comment: "step 2/5".to_string(),
            }
        }
    }
}
