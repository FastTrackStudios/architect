//! `APP_SHOT` (feature `native`): the running app draws its own window to
//! a PNG — no screen capture, no permission, covered or not (#268 H3).
//!
//! The launcher ([`crate::native::launch`]) keeps the main window's Blitz
//! document in context ([`LiveDocument`]). A shot is that document — its
//! styles and layout as the last frame resolved them — painted again by
//! the CPU rasteriser (`anyrender_vello_cpu`, what `ui-snapshot` and the
//! guide's films paint with) at the window's own pixel size and scale, on
//! the window's thread, between two frames. The GPU's framebuffer is not
//! read back: the window's renderer keeps no copy of a presented frame.
//!
//! What the CPU paints is the page; what draws itself on the GPU (a
//! custom paint widget with a texture of its own) may be missing from it.
//! Pop-out windows are not shot (their documents are the launcher's own).
//!
//! `pane` crops to one workspace pane: its rectangle is the host's
//! measure of its group (or its float), `WORKSPACE_PANE_GET`'s `rect`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use blitz_dom::BaseDocument;
use dioxus::prelude::*;
use facet::Facet;
use futures_util::StreamExt as _;

use crate::control::{PaneInfo, ids};
use crate::model::Rect;

/// `APP_SHOT`'s id.
pub const APP_SHOT: &str = "APP_SHOT";

/// The main window's document, as the launcher puts it in context (a
/// host of its own — a test's headless document — may put one there too).
#[derive(Clone)]
pub struct LiveDocument(Rc<RefCell<BaseDocument>>);

impl LiveDocument {
    /// The document `inner` (a `DioxusDocument`'s `inner`).
    #[must_use]
    pub const fn new(inner: Rc<RefCell<BaseDocument>>) -> Self {
        Self(inner)
    }
}

/// `APP_SHOT`'s arguments.
#[derive(Debug, Clone, Default, Facet)]
pub struct ShotArgs {
    /// Where to write the PNG: an absolute path (a relative one is the
    /// app's working folder's). `<temp>/app-shot.png` unless given.
    #[facet(default)]
    pub path: Option<String>,
    /// Only this pane (a workspace pane id): the window cropped to it.
    #[facet(default)]
    pub pane: Option<String>,
}

/// What `APP_SHOT` answers.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Shot {
    /// The PNG's absolute path.
    pub path: String,
    /// Its size, pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels per point (the window's scale).
    pub scale: f64,
    /// The pane it was cropped to, if any.
    #[facet(default)]
    pub pane: Option<String>,
}

static META: ActionMeta = ActionMeta {
    id: APP_SHOT,
    trait_name: "",
    method_name: "",
    display_name: "Screenshot the window",
    description: "Renders the window as the app last laid it out (or only pane `pane`) to a PNG at `path` (an absolute \
                  path ending in .png) and answers its path and size in pixels. The app's own rendering: no screen \
                  capture, works while the window is covered.",
    category: "App",
    group: "Agent tools",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// A shot to take on the window's thread.
struct Job {
    path: PathBuf,
    crop: Option<(Rect, String)>,
    answer: futures_channel::oneshot::Sender<Result<Shot, String>>,
}

/// Puts `APP_SHOT` on `list` while the calling component is mounted, in
/// a window the launcher opened (no [`LiveDocument`], no action).
pub fn use_app_shot(list: &Arc<ActionList>) {
    let live = try_use_context::<LiveDocument>();
    let runner = use_coroutine(move |mut jobs: UnboundedReceiver<Job>| {
        let live = live.clone();
        async move {
            while let Some(job) = jobs.next().await {
                let Some(live) = &live else {
                    let _ = job
                        .answer
                        .send(Err("there is no window to shoot".to_owned()));
                    continue;
                };
                let shot = take(live, &job).await;
                let _ = job.answer.send(shot);
            }
        }
    });
    let registered = use_hook({
        let list = Arc::clone(list);
        let has_window = try_consume_context::<LiveDocument>().is_some();
        move || {
            if !has_window {
                return None;
            }
            let tx = runner.tx();
            let lookup = Arc::clone(&list);
            list.register_async_with(
                &META,
                Effect::ReadOnly,
                move |_: &ActionCaller, args: ShotArgs| {
                    let tx = tx.clone();
                    let lookup = Arc::clone(&lookup);
                    async move {
                        let path = target(args.path.as_deref())?;
                        let crop = match args.pane {
                            Some(pane) => Some((pane_rect(&lookup, &pane)?, pane)),
                            None => None,
                        };
                        let (answer, answered) = futures_channel::oneshot::channel();
                        tx.unbounded_send(Job { path, crop, answer })
                            .map_err(|_| "the window is gone".to_owned())?;
                        answered
                            .await
                            .map_err(|_| "the window is gone".to_owned())?
                    }
                },
            )
            .ok()?;
            Some(list)
        }
    });
    use_drop(move || {
        if let Some(list) = registered {
            list.unregister(APP_SHOT);
        }
    });
}

/// Where a shot goes: `path` made absolute, ending in `.png`.
fn target(path: Option<&str>) -> Result<PathBuf, String> {
    let path = path.map_or_else(|| std::env::temp_dir().join("app-shot.png"), PathBuf::from);
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err(format!("{}: a shot's path ends in .png", path.display()));
    }
    std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Pane `pane`'s rectangle in the window, points: the host's measure.
fn pane_rect(list: &ActionList, pane: &str) -> Result<Rect, String> {
    #[derive(Facet)]
    struct Ask {
        pane: String,
    }
    let ask = facet_json::to_string(&Ask {
        pane: pane.to_owned(),
    })
    .map_err(|e| e.to_string())?;
    let info = list
        .run_json(ids::PANE_GET, ask.as_bytes())
        .map_err(|e| e.to_string())?;
    let info: PaneInfo = facet_json::from_slice(&info).map_err(|e| e.to_string())?;
    if !info.showing {
        return Err(format!(
            "pane {pane} is not showing (its group shows another tab)"
        ));
    }
    info.rect
        .filter(|r| r.w > 0.0 && r.h > 0.0)
        .ok_or_else(|| format!("pane {pane} has not been measured yet"))
}

/// Paints the document as the window would and writes the PNG. On the
/// window's thread: the document is the window's own, not `Send`.
#[allow(clippy::future_not_send)]
async fn take(live: &LiveDocument, job: &Job) -> Result<Shot, String> {
    // The document is busy while the window applies a frame's changes:
    // try again between two frames.
    let mut tries = 0_u32;
    let (pixels, width, height, scale) = loop {
        if let Ok(mut doc) = live.0.try_borrow_mut() {
            break paint(&mut doc);
        }
        tries = tries.saturating_add(1);
        if tries > 200 {
            return Err("the window's document stayed busy".to_owned());
        }
        futures_timer::Delay::new(Duration::from_millis(5)).await;
    };
    let (pixels, width, height) = match &job.crop {
        Some((rect, _)) => crop(&pixels, width, height, *rect, scale)?,
        None => (pixels, width, height),
    };
    write_png(&job.path, &pixels, width, height)?;
    Ok(Shot {
        path: job.path.display().to_string(),
        width,
        height,
        scale,
        pane: job.crop.as_ref().map(|(_, pane)| pane.clone()),
    })
}

/// The document painted at its window's size and scale: RGBA8 pixels.
fn paint(doc: &mut BaseDocument) -> (Vec<u8>, u32, u32, f64) {
    use anyrender::PaintScene as _;
    let (width, height) = doc.viewport().window_size;
    let scale = doc.viewport().scale_f64();
    let (width, height) = (width.max(1), height.max(1));
    let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |scene| {
            // What the window shows where the page paints nothing.
            scene.fill(
                peniko::Fill::NonZero,
                peniko::kurbo::Affine::IDENTITY,
                peniko::Color::WHITE,
                Option::<peniko::kurbo::Affine>::None,
                &peniko::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
            blitz_paint::paint_scene(scene, doc, scale, width, height, 0, 0);
        },
        width,
        height,
    );
    (pixels, width, height, scale)
}

/// `rect` (points) of an image `width` × `height` at `scale`.
fn crop(
    pixels: &[u8],
    width: u32,
    height: u32,
    rect: Rect,
    scale: f64,
) -> Result<(Vec<u8>, u32, u32), String> {
    let px = |v: f32| {
        let p = (f64::from(v) * scale).round();
        // Clamped to the image's size below; a negative or NaN one is 0.
        #[allow(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let p = if p.is_finite() {
            p.clamp(0.0, 65_535.0) as u32
        } else {
            0
        };
        p
    };
    let (x, y) = (px(rect.x).min(width), px(rect.y).min(height));
    let (w, h) = (
        px(rect.w).min(width.saturating_sub(x)),
        px(rect.h).min(height.saturating_sub(y)),
    );
    if w == 0 || h == 0 {
        return Err("the pane is outside the window".to_owned());
    }
    let size = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    let stride = size(width).saturating_mul(4);
    let line_len = size(w).saturating_mul(4);
    let mut out = Vec::with_capacity(line_len.saturating_mul(size(h)));
    for row in y..y.saturating_add(h) {
        let start = size(row)
            .saturating_mul(stride)
            .saturating_add(size(x).saturating_mul(4));
        let line = pixels
            .get(start..start.saturating_add(line_len))
            .ok_or("the shot is smaller than its window")?;
        out.extend_from_slice(line);
    }
    Ok((out, w, h))
}

/// The PNG of `pixels`, encoded before the file is opened (a failure
/// leaves no half a file), then put in place whole.
fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(pixels).map_err(|e| e.to_string())?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let name = path
        .file_name()
        .map_or_else(|| "shot".into(), |n| n.to_string_lossy().into_owned());
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, &bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {e}", path.display())
    })
}
