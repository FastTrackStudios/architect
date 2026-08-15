//! ui-snapshot — headless rendering + fuzzy PNG diff for architect-ui components.
//!
//! This library exposes the rendering infrastructure so both the CLI (`bin`)
//! and integration tests (`tests/`) can drive it. See `src/main.rs` for the
//! `check` / `update` / `render` command surface, and `tests/pixel_probes.rs`
//! for the sampled-pixel regression gates.

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{Document, DocumentConfig};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::DioxusDocument;
use kurbo::Rect;
use peniko::{Color as PColor, Fill};

pub mod scenes;

// Generated Tailwind output, vendored at the monorepo extraction. It used to
// be `include_str!`'d out of apps/extensions/reaper-fts-extensions/assets/ —
// an accident of monorepo convenience, and an invisible dependency (a file
// path, so it never showed up in the cargo graph). Regenerate it from the
// architect-ui sources rather than copying it back from a consumer.
const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");
const FTS_THEME_CSS: &str = include_str!("../../architect-ui/assets/fts-theme.css");
const COLOR_SCHEME_LIGHT: &str = ":root { color-scheme: light; }";

/// Scene descriptor — pairs a scene id with the component that paints it.
pub struct Scene {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    /// Background: we paint a theme-aware background first, then the tree.
    pub background: PColor,
    pub render: fn() -> Element,
}

pub const SCENES: &[Scene] = &[
    Scene {
        name: "icons-default",
        width: 800,
        height: 400,
        background: PColor::WHITE,
        render: scenes::icons_default,
    },
    Scene {
        name: "theme-probes",
        width: 400,
        height: 160,
        background: PColor::WHITE,
        render: scenes::theme_probes,
    },
];

/// Render a Dioxus component to a tightly-packed RGBA8 buffer via Blitz + Vello CPU.
pub fn render_scene(scene: &Scene) -> Vec<u8> {
    let width = scene.width;
    let height = scene.height;

    let page_component = scene.render;
    let vdom = VirtualDom::new_with_props(
        Page,
        PageProps {
            inner: page_component,
        },
    );

    let viewport = Viewport::new(width, height, 1.0, ColorScheme::Light);
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        },
    );

    doc.initial_build();
    doc.inner_mut().resolve(0.0);

    let bg = scene.background;
    render_to_buffer::<VelloCpuImageRenderer, _>(
        |canvas| {
            canvas.fill(
                Fill::NonZero,
                Default::default(),
                bg,
                Default::default(),
                &Rect::new(0.0, 0.0, width as f64, height as f64),
            );
            paint_scene(canvas, &mut doc.inner_mut(), 1.0, width, height, 0, 0);
        },
        width,
        height,
    )
}

/// Sample one RGBA pixel out of a tightly-packed RGBA8 buffer.
pub fn sample_pixel(buffer: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [
        buffer[idx],
        buffer[idx + 1],
        buffer[idx + 2],
        buffer[idx + 3],
    ]
}

#[derive(Clone, Props)]
struct PageProps {
    inner: fn() -> Element,
}

// Dioxus Props derive requires PartialEq; function-pointer equality is
// imprecise but fine here — every scene gets a fresh VDom, so the memo
// comparison is never load-bearing.
#[allow(unpredictable_function_pointer_comparisons)]
impl PartialEq for PageProps {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

#[component]
fn Page(props: PageProps) -> Element {
    let inner = props.inner;
    rsx! {
        document::Style { {TAILWIND_CSS} }
        document::Style { {FTS_THEME_CSS} }
        document::Style { {COLOR_SCHEME_LIGHT} }
        {inner()}
    }
}
