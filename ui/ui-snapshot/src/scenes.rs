//! Scenes — one Dioxus component per snapshot. Keep them self-contained and
//! deterministic (no time, no randomness, no animated content).

use dioxus::prelude::*;
use architect_ui::lucide_dioxus;

/// Grid of solid-color probes at fixed pixel positions. Paired with
/// `tests/pixel_probes.rs`, which renders this scene and asserts exact RGB
/// ranges at each block's center. This catches regressions in:
///
/// 1. Tailwind OKLCH → stylo → paint pipeline (`bg-*` utility classes).
/// 2. SVG `currentColor` substitution under a `text-*` class (exercises the
///    `color_to_svg_compatible` funnel in blitz-dom — see
///    `packages/blitz-dom/src/node/node.rs`). The filled rect in each
///    probe's SVG picks up its color from the parent's `color` cascade.
///
/// Layout (400×160):
/// ```text
///   row 0   (y=   0..80)  : 5 × 80px `bg-*` blocks
///   row 1   (y=  80..160) : 5 × 80px `text-*` + SVG rect fill="currentColor"
/// ```
/// Column centers: x = 40, 120, 200, 280, 360.
pub fn theme_probes() -> Element {
    // Inline styles so column widths are exact pixel sizes regardless of
    // Tailwind scan coverage (the JIT only emits classes it saw at compile
    // time, and arbitrary sizes like `w-[80px]` depend on source scanning).
    let row = "display: flex; flex-direction: row; margin: 0; padding: 0;";
    let block = "width: 80px; height: 80px;";
    // Literal OKLCH values matching fts-theme.css. These bypass the
    // Tailwind `var(--destructive)` chain entirely so the probe tests the
    // OKLCH → sRGB paint path directly. The SVG row below exercises the
    // `currentColor` substitution fix on top.
    let c_destructive = "oklch(0.5757 0.2352 27.92)";
    let c_primary = "oklch(0.205 0 0)";
    let c_chart_2 = "oklch(0.6 0.118 184.704)";
    let c_foreground = "oklch(0.145 0 0)";
    let c_background = "oklch(1 0 0)";
    rsx! {
        div { style: "margin: 0; padding: 0;",
            // Row 0 — inline `background-color: oklch(...)`.
            div { style: "{row}",
                div { style: "{block}; background-color: {c_destructive};" }
                div { style: "{block}; background-color: {c_primary};" }
                div { style: "{block}; background-color: {c_chart_2};" }
                div { style: "{block}; background-color: {c_foreground};" }
                div { style: "{block}; background-color: {c_background};" }
            }
            // Row 1 — inline `color: oklch(...)` wrapping an SVG rect with
            // `fill="currentColor"`. Exercises the node.rs serialization
            // fix in isolation (no Tailwind cascade in the loop).
            div { style: "{row}",
                ProbeSvg { oklch: c_destructive.to_string() }
                ProbeSvg { oklch: c_primary.to_string() }
                ProbeSvg { oklch: c_chart_2.to_string() }
                ProbeSvg { oklch: c_foreground.to_string() }
                ProbeSvg { oklch: c_background.to_string() }
            }
        }
    }
}

#[component]
fn ProbeSvg(oklch: String) -> Element {
    rsx! {
        div {
            style: "width: 80px; height: 80px; color: {oklch};",
            svg {
                width: "80",
                height: "80",
                view_box: "0 0 80 80",
                rect { x: "0", y: "0", width: "80", height: "80", fill: "currentColor" }
            }
        }
    }
}

/// Row of default-color icons + row of theme-tinted icons + sizes.
/// Exercises: SVG element rendering, currentColor attribute substitution,
/// CSS `color` cascade into the SVG source, per-path stroke-width.
pub fn icons_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            div { class: "flex flex-col gap-6",
                // Default (foreground) row
                div { class: "flex items-center gap-4",
                    lucide_dioxus::Check        { size: 24 }
                    lucide_dioxus::X            { size: 24 }
                    lucide_dioxus::Search       { size: 24 }
                    lucide_dioxus::House        { size: 24 }
                    lucide_dioxus::Settings     { size: 24 }
                    lucide_dioxus::Bell         { size: 24 }
                    lucide_dioxus::Heart        { size: 24 }
                    lucide_dioxus::Star         { size: 24 }
                    lucide_dioxus::ChevronRight { size: 24 }
                    lucide_dioxus::ChevronDown  { size: 24 }
                }
                // Theme-tinted row (tailwind utility)
                div { class: "flex items-center gap-4",
                    span { class: "text-destructive",       lucide_dioxus::CircleAlert   { size: 24 } }
                    span { class: "text-primary",           lucide_dioxus::Info          { size: 24 } }
                    span { class: "text-chart-2",           lucide_dioxus::CircleCheck   { size: 24 } }
                    span { class: "text-chart-4",           lucide_dioxus::TriangleAlert { size: 24 } }
                    span { class: "text-muted-foreground",  lucide_dioxus::Circle        { size: 24 } }
                }
                // Inline style (control — proves CSS → SVG chain works)
                div { class: "flex items-center gap-4",
                    span { style: "color: #dc2626;", lucide_dioxus::CircleAlert   { size: 24 } }
                    span { style: "color: #2563eb;", lucide_dioxus::Info          { size: 24 } }
                    span { style: "color: #16a34a;", lucide_dioxus::CircleCheck   { size: 24 } }
                }
                // Size scale (ensures stroke-width scales, viewBox correct)
                div { class: "flex items-end gap-4",
                    lucide_dioxus::Star { size: 12 }
                    lucide_dioxus::Star { size: 16 }
                    lucide_dioxus::Star { size: 24 }
                    lucide_dioxus::Star { size: 32 }
                    lucide_dioxus::Star { size: 48 }
                }
            }
        }
    }
}
