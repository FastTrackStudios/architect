//! Segmented control — pill-grouped toggle buttons.
//!
//! Replaces the repeated pattern of `div { class: "bg-zinc-800/50 rounded ..." }`
//! with toggle buttons inside. Supports multiple sizes for different density needs.

use dioxus::prelude::*;
use architect_story_runtime::story;

// ---------------------------------------------------------------------------
// Size
// ---------------------------------------------------------------------------

/// Size variants for the segmented control.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum SegmentedControlSize {
    /// Compact size — `text-[11px]`, tighter padding. Good for secondary selectors.
    Small,
    /// Standard size — `text-xs`. Good for primary mode tabs.
    #[default]
    Medium,
}

impl SegmentedControlSize {
    fn active_classes(self) -> &'static str {
        match self {
            Self::Small => {
                "px-2 py-0.5 text-[11px] font-medium rounded bg-accent text-accent-foreground"
            }
            Self::Medium => {
                "px-2.5 py-1 text-xs font-semibold rounded bg-accent text-accent-foreground"
            }
        }
    }

    fn inactive_classes(self) -> &'static str {
        match self {
            Self::Small => {
                "px-2 py-0.5 text-[11px] text-muted-foreground hover:text-foreground hover:bg-accent/50 rounded transition-colors"
            }
            Self::Medium => {
                "px-2.5 py-1 text-xs text-muted-foreground hover:text-foreground hover:bg-accent/50 rounded transition-colors"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// A group of mutually-exclusive toggle buttons rendered inside a pill container.
///
/// ```rust,ignore
/// SegmentedControl {
///     value: "song",
///     on_change: move |v| mode.set(v),
///     options: vec![
///         ("song".into(), "Song".into()),
///         ("profile".into(), "Profile".into()),
///         ("preset".into(), "Preset".into()),
///     ],
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct SegmentedControlProps {
    /// The currently selected value.
    pub value: String,

    /// Called when the user clicks a different option. Receives the option's value.
    pub on_change: Callback<String>,

    /// `(value, label)` pairs for each segment.
    pub options: Vec<(String, String)>,

    /// Size variant.
    #[props(default)]
    pub size: SegmentedControlSize,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn SegmentedControl(props: SegmentedControlProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge(format!(
                "flex items-center gap-0.5 bg-secondary/50 rounded px-0.5 py-0.5 {}",
                props.class
            )),

            for (val, label) in props.options.iter() {
                {
                    let is_active = *val == props.value;
                    let classes = if is_active {
                        props.size.active_classes()
                    } else {
                        props.size.inactive_classes()
                    };
                    let val = val.clone();
                    rsx! {
                        button {
                            key: "{val}",
                            class: "{classes}",
                            onclick: move |_| props.on_change.call(val.clone()),
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

/// Segmented control states: default vs visually-disabled (wrapped in opacity-50 + pointer-events-none).
#[story(category = "SegmentedControl", name = "states")]
pub fn segmented_control_states() -> Element {
    let mut a = use_signal(|| "song".to_string());
    let opts = || {
        vec![
            ("song".to_string(), "Song".to_string()),
            ("profile".to_string(), "Profile".to_string()),
            ("preset".to_string(), "Preset".to_string()),
        ]
    };
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4 items-start",
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "Default" }
                SegmentedControl {
                    value: a(),
                    on_change: move |v: String| a.set(v),
                    options: opts(),
                }
            }
            div {
                div { class: "text-xs uppercase tracking-wider text-muted-foreground mb-2", "Disabled" }
                div { class: "opacity-50 pointer-events-none",
                    SegmentedControl {
                        value: "song".to_string(),
                        on_change: Callback::new(|_: String| {}),
                        options: opts(),
                    }
                }
            }
        }
    }
}

/// Default segmented control with three options.
#[story(category = "SegmentedControl", name = "default")]
pub fn segmented_control_default() -> Element {
    let mut value = use_signal(|| "song".to_string());
    let options = vec![
        ("song".to_string(), "Song".to_string()),
        ("profile".to_string(), "Profile".to_string()),
        ("preset".to_string(), "Preset".to_string()),
    ];
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            SegmentedControl {
                value: value(),
                on_change: move |v: String| value.set(v),
                options,
            }
        }
    }
}

/// Segmented control sizes (small and medium).
#[story(category = "SegmentedControl", name = "sizes")]
pub fn segmented_control_sizes() -> Element {
    let mut a = use_signal(|| "one".to_string());
    let mut b = use_signal(|| "two".to_string());
    let opts = || {
        vec![
            ("one".to_string(), "One".to_string()),
            ("two".to_string(), "Two".to_string()),
            ("three".to_string(), "Three".to_string()),
        ]
    };
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-3 items-start",
            SegmentedControl {
                value: a(),
                on_change: move |v: String| a.set(v),
                options: opts(),
                size: SegmentedControlSize::Small,
            }
            SegmentedControl {
                value: b(),
                on_change: move |v: String| b.set(v),
                options: opts(),
                size: SegmentedControlSize::Medium,
            }
        }
    }
}
