//! ColorPicker — primitive-backed HSV picker with styled area + hue slider.
//!
//! Wraps `dioxus_primitives::color_picker` with FTS tokens. The primitive
//! controls saturation/value via `ColorArea`; this wrapper pairs it with
//! a horizontal hue `Slider` so callers get a complete inline picker for
//! a `Signal<Hsv<Srgb, f64>>`. For a popover-triggered variant, compose
//! `ColorPickerInline` inside your own `Popover`.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::color_picker::{
    AreaThumb, AreaThumbSaturationInput, AreaThumbValueInput, AreaTrack, ColorArea,
    ColorPicker as PrimitiveColorPicker,
};
use dioxus_primitives::slider::{Slider as PrimitiveSlider, SliderRange, SliderThumb, SliderTrack};
use palette::{encoding, Hsv, RgbHue};

pub use dioxus_primitives::color_picker::Color;

// ── Hsv helpers ──────────────────────────────────────────────────────────────

/// Convenience constructor: HSV in degrees / 0..1 / 0..1.
pub fn hsv(h: f64, s: f64, v: f64) -> Hsv<encoding::Srgb, f64> {
    Hsv::<encoding::Srgb, f64>::new(RgbHue::new(h), s, v)
}

fn hue_degrees(c: Hsv<encoding::Srgb, f64>) -> f64 {
    c.hue.into_positive_degrees()
}

fn with_hue(c: Hsv<encoding::Srgb, f64>, h: f64) -> Hsv<encoding::Srgb, f64> {
    Hsv::<encoding::Srgb, f64>::new(RgbHue::new(h), c.saturation, c.value)
}

// ── ColorPicker ──────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ColorPickerProps {
    /// HSV color (two-way bound). Use `hsv(h, s, v)` to construct.
    pub value: Signal<Hsv<encoding::Srgb, f64>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
}

/// Inline HSV picker: 2-D saturation/value area + horizontal hue slider.
#[component]
pub fn ColorPicker(props: ColorPickerProps) -> Element {
    let mut value = props.value;
    let color_signal: ReadSignal<Hsv<encoding::Srgb, f64>> = use_memo(move || value()).into();

    let hue = use_memo(move || hue_degrees(value()));

    rsx! {
        document::Style {
            r#"
                .fts-color-area {{
                    background:
                        linear-gradient(to top, #000, transparent),
                        linear-gradient(to right, #fff, hsl(var(--fts-cp-hue, 0), 100%, 50%));
                }}
                .fts-color-hue {{
                    background: linear-gradient(
                        to right,
                        hsl(0,100%,50%) 0%,
                        hsl(60,100%,50%) 17%,
                        hsl(120,100%,50%) 33%,
                        hsl(180,100%,50%) 50%,
                        hsl(240,100%,50%) 67%,
                        hsl(300,100%,50%) 83%,
                        hsl(360,100%,50%) 100%
                    );
                }}
            "#
        }
        PrimitiveColorPicker {
            color: color_signal,
            disabled: props.disabled,
            on_color_change: move |c: Hsv<encoding::Srgb, f64>| {
                value.set(c);
            },
            class: crate::cn::merge_slice(&["flex flex-col gap-3 w-full max-w-xs", props.class.as_str()]),

            // ── Saturation/Value area ─────────────────────────────────
            ColorArea {
                AreaTrack {
                    class: "fts-color-area relative w-full aspect-square rounded-lg overflow-hidden border border-border touch-none select-none cursor-crosshair",
                    style: "--fts-cp-hue: {hue}",
                    AreaThumb {
                        class: "absolute size-4 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow-md ring-1 ring-black/30 pointer-events-none",
                        AreaThumbSaturationInput {}
                        AreaThumbValueInput {}
                    }
                }
            }

            // ── Hue slider ────────────────────────────────────────────
            PrimitiveSlider {
                value: Some(hue()),
                min: 0.0,
                max: 360.0,
                step: 1.0,
                disabled: props.disabled,
                label: Some("Hue".to_string()),
                class: "relative flex w-full touch-none select-none items-center",
                on_value_change: move |h: f64| {
                    let cur = value();
                    value.set(with_hue(cur, h));
                },
                SliderTrack {
                    class: "fts-color-hue relative h-3 w-full rounded-full overflow-hidden",
                    SliderRange {
                        class: "absolute h-full opacity-0",
                    }
                    SliderThumb {
                        class: "absolute top-1/2 size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-transparent shadow ring-1 ring-black/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    }
                }
            }
        }
    }
}

// ── Story ────────────────────────────────────────────────────────────────────

#[story(category = "ColorPicker", name = "default")]
pub fn color_picker_default() -> Element {
    let value = use_signal(|| hsv(280.0, 0.6, 0.9));
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            ColorPicker { value }
        }
    }
}
