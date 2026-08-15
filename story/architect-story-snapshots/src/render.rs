//! Render a `Story` to a PNG via Blitz's CPU rasteriser.
//!
//! Mirrors the design of `blitz/examples/screenshot.rs` but feeds in a
//! Dioxus component (via `DioxusDocument`) instead of an HTML string.
//! The "current snapshot" — render thunk + knob values — is stashed in
//! a thread-local so the zero-arg `Component` we hand to `VirtualDom`
//! can recover it without forcing the user-written stories to be
//! generic over a props type.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyrender::{render_to_buffer, PaintScene as _};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::util::Color;
use blitz_dom::{Document as _, DocumentConfig};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use architect_story_runtime::{KnobValue, Story};
use architect_story_shell::Lookbook;
use peniko::kurbo::Rect;
use peniko::Fill;

/// Configuration for a single render.
#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// Output width in CSS pixels.
    pub width: u32,
    /// Output height in CSS pixels.
    pub height: u32,
    /// HiDPI scale (e.g. `2.0` for retina).
    pub scale: f32,
    /// Light vs dark color-scheme for `prefers-color-scheme` media queries.
    pub color_scheme: ColorScheme,
    /// Maximum settling iterations (resolve → poll → resolve loop).
    pub max_settle_iters: usize,
    /// Animation clock (in seconds) to use for the **final** resolve.
    /// Stylo's animation engine uses this as `current_time_for_animations`
    /// — anything that reaches that timestamp without finishing renders at
    /// the interpolated value for that instant. The default of `0.125s`
    /// lands a 1s linear `animate-spin` keyframe at 45° (a visibly-rotated
    /// frame for parity diffs against wry).
    ///
    /// Set to `0.0` to capture the un-animated initial state.
    pub animation_time_secs: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.0,
            color_scheme: ColorScheme::Dark,
            max_settle_iters: 64,
            animation_time_secs: 0.125,
        }
    }
}

/// Render `story` with the supplied knob overrides into a PNG byte
/// buffer. Returns `Ok(png)` or `Err(panic_message)` on hard failure.
///
/// **Panics in debug builds** because Stylo and Parley produce
/// visually-incorrect output under `debug_assertions`.
///
/// Internally this routes through
/// [`architect_story_shell::Lookbook`]`{ chrome: false, initial_story: ... }`
/// so the rendered tree is identical to what `apps/desktop`'s
/// `--snapshot=<story>` mode hands wry. Knob overrides aren't yet
/// honoured by the chromeless Lookbook path; the parameter is kept
/// for forward-compat with a future API.
pub fn render_story(
    story: &'static Story,
    knobs: HashMap<&'static str, KnobValue>,
    cfg: &RenderConfig,
) -> Vec<u8> {
    if cfg!(debug_assertions) {
        panic!(
            "architect-story-snapshots: render_story called in a debug build. \
             Stylo/Parley produce incorrect renders under debug_assertions; \
             run with `cargo test --release` (or use the Docker harness)."
        );
    }

    install_snapshot(story.name.to_string(), knobs);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_inner(cfg)));
    clear_snapshot();
    match result {
        Ok(png) => png,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn render_inner(cfg: &RenderConfig) -> Vec<u8> {
    let render_w = ((cfg.width as f32) * cfg.scale) as u32;
    let render_h = ((cfg.height as f32) * cfg.scale) as u32;

    let viewport = Viewport::new(render_w, render_h, cfg.scale, cfg.color_scheme);
    let vdom = VirtualDom::new(snapshot_component);
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        },
    );

    doc.initial_build();

    // Settling loop — keep polling until the runtime quiesces or we hit
    // the iteration cap. This drains:
    //  - dioxus async tasks (effects / spawn'd futures)
    //  - mounted-event flushes
    //  - layout/style resolution against any newly-inserted nodes
    //
    // Animation-clock convention: settle iters all use t=0.0 so animations
    // get **registered** in the Stylo animation set with a `start_time`
    // anchored at zero. Stylo only inserts animations on a restyle pass
    // where it sees the keyframe rule; `start_time` is captured from the
    // `current_time_for_animations` of that pass. Bumping the clock during
    // settling would offset every animation's start by an inconsistent
    // amount (depending on which iter happened to insert it) and produce
    // non-deterministic captures.
    for _ in 0..cfg.max_settle_iters {
        let progressed = doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
        if !progressed {
            break;
        }
    }

    // Final resolve at the configured animation time. With every
    // animation's `start_time = 0.0` from the settle loop, this single
    // jump deterministically advances each one by `animation_time_secs`
    // — e.g. a 1s linear `animate-spin` lands at
    // `animation_time_secs * 360°` of rotation.
    doc.inner.borrow_mut().resolve(cfg.animation_time_secs);

    // Render the painted scene into a CPU buffer via vello-cpu.
    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            // White-on-light or near-black-on-dark base so transparent
            // backgrounds don't render against undefined memory.
            let bg = match cfg.color_scheme {
                ColorScheme::Dark => Color::from_rgba8(11, 15, 23, 255),
                ColorScheme::Light => Color::WHITE,
            };
            scene.fill(
                Fill::NonZero,
                Default::default(),
                bg,
                Default::default(),
                &Rect::new(0.0, 0.0, render_w as f64, render_h as f64),
            );
            paint_scene(
                scene,
                &mut doc.inner.borrow_mut(),
                cfg.scale as f64,
                render_w,
                render_h,
                0,
                0,
            );
        },
        render_w,
        render_h,
    );

    encode_png(&buffer, render_w, render_h)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    // Match blitz's screenshot example for parity with the broader
    // ecosystem (144 DPI scaled to pixels-per-meter).
    const PPM: u32 = (144.0 * 39.3701) as u32;

    let mut out = Vec::with_capacity(rgba.len() + 1024);
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: PPM,
            yppu: PPM,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder.write_header().expect("png header write");
        writer.write_image_data(rgba).expect("png pixel data write");
        writer.finish().expect("png finish");
    }
    out
}

// ── Thread-local snapshot ────────────────────────────────────────────────────

thread_local! {
    static CURRENT: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    static WRAPPER: RefCell<Option<fn(Element) -> Element>> = const { RefCell::new(None) };
}

struct Snapshot {
    /// Story name passed to `Lookbook`'s `initial_story` prop.
    story_name: String,
    /// Reserved for future per-story knob overrides; the Lookbook
    /// chromeless path currently always uses each KnobSpec.default.
    #[allow(dead_code)]
    knobs: Rc<HashMap<&'static str, KnobValue>>,
}

/// Install a wrapper fn that is applied to every snapshot-rendered
/// element. Lets the consumer wrap stories in a `ThemeProvider`,
/// `Router`, or any other context the bare component fn would
/// otherwise see. Stays installed until [`clear_wrapper`] is called or
/// another wrapper is installed.
///
/// The fn receives the story's rendered `Element` and must return the
/// wrapped one. Typical use:
///
/// ```ignore
/// fn theme_wrap(child: Element) -> Element {
///     let state = use_signal(|| ThemeState::new(default_theme_preset(), ThemeMode::Dark));
///     rsx! { ThemeProvider { state, {child} } }
/// }
/// architect_story_snapshots::install_wrapper(theme_wrap);
/// ```
pub fn install_wrapper(wrap: fn(Element) -> Element) {
    WRAPPER.with(|cell| *cell.borrow_mut() = Some(wrap));
}

pub fn clear_wrapper() {
    WRAPPER.with(|cell| *cell.borrow_mut() = None);
}

fn install_snapshot(story_name: String, knobs: HashMap<&'static str, KnobValue>) {
    CURRENT.with(|cell| {
        *cell.borrow_mut() = Some(Snapshot {
            story_name,
            knobs: Rc::new(knobs),
        });
    });
}

fn clear_snapshot() {
    CURRENT.with(|cell| *cell.borrow_mut() = None);
}

/// Component handed to `VirtualDom::new`. Pulls the story name from
/// the thread-local, mounts the same `Lookbook { chrome: false }`
/// host that `apps/desktop`'s `--snapshot=<story>` mode uses, then
/// applies any installed wrapper (typically `ThemeProvider` plus
/// inline Tailwind stylesheet).
fn snapshot_component() -> Element {
    let story_name = CURRENT.with(|cell| cell.borrow().as_ref().map(|s| s.story_name.clone()));
    let story_name = match story_name {
        Some(s) => s,
        None => {
            return rsx! {
                div { "architect-story-snapshots: no snapshot installed; this is a bug." }
            };
        }
    };
    let element = rsx! {
        Lookbook {
            chrome: false,
            initial_story: story_name,
        }
    };
    let wrapper = WRAPPER.with(|cell| *cell.borrow());
    match wrapper {
        Some(wrap) => wrap(element),
        None => element,
    }
}
