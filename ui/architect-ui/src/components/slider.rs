//! Slider — shadcn v4 maia style range slider.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::slider::{
    RangeSlider as PrimitiveRangeSlider, Slider as PrimitiveSlider, SliderRange, SliderThumb,
    SliderTrack,
};
use std::ops::Range;

// ---------------------------------------------------------------------------
// Slider
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SliderProps {
    /// The current value (two-way bound).
    pub value: Signal<f64>,

    /// Minimum value.
    #[props(default = 0.0)]
    pub min: f64,

    /// Maximum value.
    #[props(default = 100.0)]
    pub max: f64,

    /// Step increment.
    #[props(default = 1.0)]
    pub step: f64,

    /// Whether the slider is disabled.
    #[props(default = false)]
    pub disabled: bool,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: slider
#[component]
pub fn Slider(props: SliderProps) -> Element {
    let mut value = props.value;
    let min = props.min;
    let max = props.max;
    let step = props.step;

    rsx! {
        PrimitiveSlider {
            value: Some(value()),
            min,
            max,
            step,
            disabled: props.disabled,
            label: None::<String>,
            class: crate::cn::merge_slice(&["relative flex w-full touch-none select-none items-center", props.class.as_str()]),
            on_value_change: move |next: f64| {
                value.set(next);
            },
            SliderTrack {
                class: "relative h-3 w-full rounded-full bg-muted overflow-hidden",
                SliderRange {
                    class: "absolute h-full bg-primary rounded-full",
                }
                SliderThumb {
                    class: "absolute top-1/2 size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-primary bg-background shadow transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RangeSlider — two-thumb (start, end) variant from upstream PR #225.
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct RangeSliderProps {
    /// The current (start, end) range (two-way bound).
    pub value: Signal<Range<f64>>,
    #[props(default = 0.0)]
    pub min: f64,
    #[props(default = 100.0)]
    pub max: f64,
    #[props(default = 1.0)]
    pub step: f64,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: two-thumb range slider
#[component]
pub fn RangeSlider(props: RangeSliderProps) -> Element {
    let mut value = props.value;

    rsx! {
        PrimitiveRangeSlider {
            value: Some(value()),
            min: props.min,
            max: props.max,
            step: props.step,
            disabled: props.disabled,
            label: None::<String>,
            class: crate::cn::merge_slice(&["relative flex w-full touch-none select-none items-center", props.class.as_str()]),
            on_value_change: move |next: Range<f64>| {
                value.set(next);
            },
            SliderTrack {
                class: "relative h-3 w-full rounded-full bg-muted overflow-hidden",
                SliderRange {
                    class: "absolute h-full bg-primary rounded-full",
                }
                SliderThumb {
                    index: 0usize,
                    class: "absolute top-1/2 size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-primary bg-background shadow transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
                }
                SliderThumb {
                    index: 1usize,
                    class: "absolute top-1/2 size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-primary bg-background shadow transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
                }
            }
        }
    }
}

/// Two-thumb range slider with a start and end value.
#[story(category = "Slider", name = "range two-thumb")]
pub fn range_slider_default() -> Element {
    let value = use_signal(|| 25.0..75.0);
    rsx! {
        div { class: "p-6 bg-background text-foreground w-72",
            RangeSlider { value }
        }
    }
}

/// Slider at multiple fill levels plus a disabled state.
#[story(category = "Slider", name = "range")]
pub fn slider_range() -> Element {
    let v0 = use_signal(|| 0.0);
    let v25 = use_signal(|| 25.0);
    let v50 = use_signal(|| 50.0);
    let v75 = use_signal(|| 75.0);
    let v100 = use_signal(|| 100.0);
    let vd = use_signal(|| 50.0);
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-4 w-72",
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "0%" }
                Slider { value: v0 }
            }
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "25%" }
                Slider { value: v25 }
            }
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "50%" }
                Slider { value: v50 }
            }
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "75%" }
                Slider { value: v75 }
            }
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "100%" }
                Slider { value: v100 }
            }
            div { class: "flex items-center gap-3",
                span { class: "w-16 text-xs text-muted-foreground", "Disabled" }
                Slider { value: vd, disabled: true }
            }
        }
    }
}

/// Default slider with a single value thumb.
#[story(category = "Slider", name = "default")]
pub fn slider_default() -> Element {
    let value = use_signal(|| 50.0);
    rsx! {
        div { class: "p-6 bg-background text-foreground w-72",
            Slider { value }
        }
    }
}
