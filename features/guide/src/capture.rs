//! The film (feature `capture`): a run's frames as media.
//!
//! Each frame kept, every window is painted by Blitz on the CPU
//! rasteriser (no GPU, as `architect-story-snapshots` and `ui-snapshot`
//! paint), put on one canvas — the main window alone, or a desktop with
//! each window at its place under a title bar — with the pointer and a
//! press's ripple drawn over it (the film's own layer; the app never sees
//! it), cropped to `capture.crop` and scaled to `capture.width`. Frames go
//! to a directory as PNGs; [`encode`] makes them a GIF (gifski), `WebM` and
//! MP4 (ffmpeg) and a poster PNG (the last frame). Both tools are run as
//! programs, from the dev shell (`flake.nix`).

// Geometry reads plainer as `a + b * c`; its precision is a pixel's.
#![allow(clippy::suboptimal_flops)]

use std::path::{Path, PathBuf};
use std::process::Command;

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use architect_workspace::Rect;
use blitz_paint::paint_scene;
use image::{Rgba, RgbaImage, imageops};
use kurbo::Affine;
use peniko::{Color, Fill};

use crate::run::{Desktop, Film, Pointer, Resolver, device_px};
use crate::{Output, Scenario};

/// Why media could not be made.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("{what}: {source}")]
    Io {
        what: String,
        source: std::io::Error,
    },
    #[error("writing frame {0}: {1}")]
    Frame(PathBuf, image::ImageError),
    #[error("{tool} is not on PATH (it comes with the dev shell: `nix develop`)")]
    Missing { tool: &'static str },
    #[error("{tool} failed making {out}: {status}")]
    Tool {
        tool: &'static str,
        out: PathBuf,
        status: std::process::ExitStatus,
    },
    #[error("the run kept no frames (every step hidden?)")]
    NoFrames,
}

/// The desktop's ground, and a window's title bar.
const DESK: Rgba<u8> = Rgba([28, 30, 36, 255]);
const TITLE: Rgba<u8> = Rgba([44, 46, 54, 255]);
const EDGE: Rgba<u8> = Rgba([70, 72, 82, 255]);
/// A title bar's height, points.
pub const TITLE_BAR: f32 = 28.0;
/// What shows where a page paints nothing.
const PAGE: Color = Color::from_rgba8(9, 9, 11, 255);

/// The film's crop: not worked out yet, or fixed (none: the whole
/// canvas).
#[derive(Debug, Clone, Copy)]
enum Crop {
    Unknown,
    Fixed(Option<(u32, u32, u32, u32)>),
}

/// A [`Film`] writing each frame to `frames` as a numbered PNG.
pub struct Recorder {
    frames: PathBuf,
    count: u32,
    desktop: Option<(u32, u32)>,
    cursor: bool,
    crop_target: Option<crate::Target>,
    pad: f32,
    width: Option<u32>,
    resolvers: Vec<Resolver>,
    /// The crop, in canvas pixels, fixed at the first frame.
    crop: Crop,
    /// The first failure, reported when the run ends.
    failed: Option<CaptureError>,
}

impl Recorder {
    /// A recorder for `scenario`, writing into `frames` (made, and
    /// emptied of earlier frames).
    pub fn new(
        scenario: &Scenario,
        frames: &Path,
        resolvers: Vec<Resolver>,
    ) -> Result<Self, CaptureError> {
        if frames.exists() {
            std::fs::remove_dir_all(frames).map_err(|source| CaptureError::Io {
                what: format!("emptying {}", frames.display()),
                source,
            })?;
        }
        std::fs::create_dir_all(frames).map_err(|source| CaptureError::Io {
            what: format!("making {}", frames.display()),
            source,
        })?;
        let c = &scenario.capture;
        Ok(Self {
            frames: frames.to_owned(),
            count: 0,
            desktop: c.desktop.map(|d| (d.width, d.height)),
            cursor: c.cursor,
            crop_target: c.crop.clone(),
            pad: c.pad,
            width: c.width,
            resolvers,
            crop: Crop::Unknown,
            failed: None,
        })
    }

    /// The frames written, in order.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// The frames' directory.
    #[must_use]
    pub fn frames(&self) -> &Path {
        &self.frames
    }

    /// The first frame that could not be written, if one could not.
    pub fn finish(self) -> Result<u32, CaptureError> {
        match self.failed {
            Some(e) => Err(e),
            None if self.count == 0 => Err(CaptureError::NoFrames),
            None => Ok(self.count),
        }
    }

    // Geometry: x, y, w, h, r read as what they are.
    #[allow(clippy::many_single_char_names)]
    fn canvas(&self, desk: &Desktop, pointer: Pointer) -> RgbaImage {
        let scale = desk.scale();
        let main = desk
            .windows()
            .first()
            .map_or(Rect::new(0.0, 0.0, 1.0, 1.0), |w| w.rect);
        // Without a desktop the canvas is the main window, edge to edge.
        let (origin, w, h) = self
            .desktop
            .map_or(((main.x, main.y), main.w, main.h), |(w, h)| {
                ((0.0, 0.0), points(w), points(h))
            });
        let mut canvas = RgbaImage::from_pixel(device_px(w, scale), device_px(h, scale), DESK);
        let at = |x: f32, y: f32| ((x - origin.0) * scale, (y - origin.1) * scale);
        for window in desk.windows() {
            let r = window.rect;
            if self.desktop.is_some() {
                let (x, y) = at(r.x - 1.0, r.y - TITLE_BAR - 1.0);
                fill_rect(
                    &mut canvas,
                    x,
                    y,
                    (r.w + 2.0) * scale,
                    (r.h + TITLE_BAR + 2.0) * scale,
                    EDGE,
                );
                let (x, y) = at(r.x, r.y - TITLE_BAR);
                fill_rect(&mut canvas, x, y, r.w * scale, TITLE_BAR * scale, TITLE);
                for (i, dot) in [[255, 95, 87], [254, 188, 46], [40, 200, 64]]
                    .iter()
                    .enumerate()
                {
                    let cx = x + (14.0 + 20.0 * index_f32(i)) * scale;
                    let cy = y + TITLE_BAR * 0.5 * scale;
                    disc(
                        &mut canvas,
                        cx,
                        cy,
                        6.0 * scale,
                        Rgba([dot[0], dot[1], dot[2], 255]),
                        1.0,
                    );
                }
            }
            let pixels = paint(window, scale);
            let (x, y) = at(r.x, r.y);
            imageops::overlay(&mut canvas, &pixels, round_i64(x), round_i64(y));
        }
        if self.cursor {
            let (x, y) = at(pointer.x, pointer.y);
            if let Some(p) = pointer.ripple {
                let radius = (8.0 + 22.0 * p) * scale;
                ring(
                    &mut canvas,
                    x,
                    y,
                    radius,
                    2.5 * scale,
                    Rgba([147, 197, 253, 255]),
                    0.8 * (1.0 - p),
                );
            }
            if pointer.down {
                disc(
                    &mut canvas,
                    x,
                    y,
                    9.0 * scale,
                    Rgba([147, 197, 253, 255]),
                    0.35,
                );
            }
            arrow(&mut canvas, x, y, scale);
        }
        canvas
    }

    fn crop_for(&self, desk: &Desktop, canvas: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
        let target = self.crop_target.as_ref()?;
        let r = desk.find(target, &self.resolvers)?;
        let scale = desk.scale();
        let main = desk
            .windows()
            .first()
            .map_or(Rect::new(0.0, 0.0, 1.0, 1.0), |w| w.rect);
        let origin = if self.desktop.is_some() {
            (0.0, 0.0)
        } else {
            (main.x, main.y)
        };
        let left = ((r.x - self.pad - origin.0) * scale).max(0.0);
        let top = ((r.y - self.pad - origin.1) * scale).max(0.0);
        let right = ((r.x + r.w + self.pad - origin.0) * scale).min(points(canvas.width()));
        let bottom = ((r.y + r.h + self.pad - origin.1) * scale).min(points(canvas.height()));
        (right > left && bottom > top).then(|| {
            (
                round_u32(left),
                round_u32(top),
                round_u32(right - left).max(1),
                round_u32(bottom - top).max(1),
            )
        })
    }
}

impl Film for Recorder {
    fn frame(&mut self, desk: &Desktop, pointer: Pointer) {
        if self.failed.is_some() {
            return;
        }
        let mut canvas = self.canvas(desk, pointer);
        let crop = if let Crop::Fixed(c) = self.crop {
            c
        } else {
            let c = self.crop_for(desk, &canvas);
            self.crop = Crop::Fixed(c);
            c
        };
        if let Some((x, y, w, h)) = crop {
            canvas = imageops::crop_imm(&canvas, x, y, w, h).to_image();
        }
        if let Some(width) = self.width.filter(|w| *w > 0 && *w != canvas.width()) {
            let height = scaled_height(canvas.width(), canvas.height(), width);
            canvas = imageops::resize(&canvas, width, height, imageops::FilterType::Lanczos3);
        }
        let path = self.frames.join(format!("{:05}.png", self.count));
        if let Err(e) = canvas.save(&path) {
            self.failed = Some(CaptureError::Frame(path, e));
            return;
        }
        self.count = self.count.saturating_add(1);
    }
}

/// One window's document painted, as pixels.
fn paint(window: &crate::run::Window, scale: f32) -> RgbaImage {
    let (w, h) = (
        device_px(window.rect.w, scale),
        device_px(window.rect.h, scale),
    );
    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                PAGE,
                Option::<Affine>::None,
                &kurbo::Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
            );
            paint_scene(
                scene,
                &mut window.doc.inner.borrow_mut(),
                f64::from(scale),
                w,
                h,
                0,
                0,
            );
        },
        w,
        h,
    );
    RgbaImage::from_raw(w, h, buffer).unwrap_or_else(|| RgbaImage::new(w, h))
}

/// Makes the media `scenario` asks for out of the frames in `frames`,
/// into `out` as `<id>.<ext>`. Gives back the files written.
pub fn encode(
    scenario: &Scenario,
    frames: &Path,
    count: u32,
    out: &Path,
) -> Result<Vec<PathBuf>, CaptureError> {
    std::fs::create_dir_all(out).map_err(|source| CaptureError::Io {
        what: format!("making {}", out.display()),
        source,
    })?;
    let fps = scenario.capture.fps.max(1).to_string();
    let pattern = frames.join("%05d.png");
    let mut written = Vec::new();
    for output in scenario.capture.outputs() {
        let file = out.join(format!("{}.{}", scenario.id, output.extension()));
        match output {
            Output::Png => {
                let last = frames.join(format!("{:05}.png", count.saturating_sub(1)));
                std::fs::copy(&last, &file).map_err(|source| CaptureError::Io {
                    what: format!("writing {}", file.display()),
                    source,
                })?;
            }
            Output::Gif => {
                let mut cmd = Command::new("gifski");
                cmd.arg("--quiet")
                    .arg("--fps")
                    .arg(&fps)
                    .arg("-o")
                    .arg(&file);
                for n in 0..count {
                    cmd.arg(frames.join(format!("{n:05}.png")));
                }
                tool("gifski", &mut cmd, &file)?;
            }
            Output::Webm => {
                let mut cmd = Command::new("ffmpeg");
                cmd.args(["-y", "-loglevel", "error", "-framerate", &fps, "-i"])
                    .arg(&pattern)
                    .args([
                        "-c:v",
                        "libvpx-vp9",
                        "-pix_fmt",
                        "yuv420p",
                        "-b:v",
                        "0",
                        "-crf",
                        "32",
                    ])
                    .arg(&file);
                tool("ffmpeg", &mut cmd, &file)?;
            }
            Output::Mp4 => {
                let mut cmd = Command::new("ffmpeg");
                cmd.args(["-y", "-loglevel", "error", "-framerate", &fps, "-i"])
                    .arg(&pattern)
                    .args([
                        "-c:v",
                        "libx264",
                        "-pix_fmt",
                        "yuv420p",
                        "-vf",
                        "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                        "-movflags",
                        "+faststart",
                    ])
                    .arg(&file);
                tool("ffmpeg", &mut cmd, &file)?;
            }
        }
        written.push(file);
    }
    Ok(written)
}

fn tool(name: &'static str, cmd: &mut Command, out: &Path) -> Result<(), CaptureError> {
    let status = cmd.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CaptureError::Missing { tool: name }
        } else {
            CaptureError::Io {
                what: format!("running {name}"),
                source: e,
            }
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(CaptureError::Tool {
            tool: name,
            out: out.to_owned(),
            status,
        })
    }
}

// ── Drawing on the canvas ─────────────────────────────────────────────

/// Blends `color` at `alpha` (`0`–`1`) over the pixel at `(x, y)`.
fn blend(canvas: &mut RgbaImage, x: i64, y: i64, color: Rgba<u8>, alpha: f32) {
    let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
        return;
    };
    if x >= canvas.width() || y >= canvas.height() || alpha <= 0.0 {
        return;
    }
    let a = alpha.min(1.0);
    let px = canvas.get_pixel_mut(x, y);
    for c in 0..3 {
        if let (Some(dst), Some(src)) = (px.0.get_mut(c), color.0.get(c)) {
            *dst = channel(f32::from(*src).mul_add(a, f32::from(*dst) * (1.0 - a)));
        }
    }
}

fn fill_rect(canvas: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, color: Rgba<u8>) {
    for py in round_i64(y)..round_i64(y + h) {
        for px in round_i64(x)..round_i64(x + w) {
            blend(canvas, px, py, color, 1.0);
        }
    }
}

/// How much of the pixel at `(px, py)` a shape covers, from 4×4 samples.
fn coverage(px: i64, py: i64, inside: impl Fn(f32, f32) -> bool) -> f32 {
    let mut hits = 0_u8;
    for sy in 0..4_u8 {
        for sx in 0..4_u8 {
            let x = whole(px) + (f32::from(sx) + 0.5) / 4.0;
            let y = whole(py) + (f32::from(sy) + 0.5) / 4.0;
            if inside(x, y) {
                hits = hits.saturating_add(1);
            }
        }
    }
    f32::from(hits) / 16.0
}

fn shade(
    canvas: &mut RgbaImage,
    bounds: (f32, f32, f32, f32),
    color: Rgba<u8>,
    alpha: f32,
    inside: impl Fn(f32, f32) -> bool,
) {
    let (x0, y0, x1, y1) = bounds;
    for py in round_i64(y0.floor())..=round_i64(y1.ceil()) {
        for px in round_i64(x0.floor())..=round_i64(x1.ceil()) {
            let c = coverage(px, py, &inside);
            if c > 0.0 {
                blend(canvas, px, py, color, c * alpha);
            }
        }
    }
}

fn disc(canvas: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: Rgba<u8>, alpha: f32) {
    shade(
        canvas,
        (cx - r, cy - r, cx + r, cy + r),
        color,
        alpha,
        |x, y| (x - cx).hypot(y - cy) <= r,
    );
}

fn ring(canvas: &mut RgbaImage, cx: f32, cy: f32, r: f32, width: f32, color: Rgba<u8>, alpha: f32) {
    let outer = r + width * 0.5;
    shade(
        canvas,
        (cx - outer, cy - outer, cx + outer, cy + outer),
        color,
        alpha,
        |x, y| ((x - cx).hypot(y - cy) - r).abs() <= width * 0.5,
    );
}

/// The pointer: the usual arrow, white with a dark edge, its tip at
/// `(x, y)`.
fn arrow(canvas: &mut RgbaImage, x: f32, y: f32, scale: f32) {
    const SHAPE: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 17.0),
        (4.2, 13.3),
        (7.0, 19.6),
        (9.8, 18.4),
        (7.1, 12.2),
        (12.4, 12.2),
    ];
    let at = |grow: f32| -> Vec<(f32, f32)> {
        // Grown about the shape's middle, for the edge drawn under it.
        let (mx, my) = (4.5, 10.0);
        SHAPE
            .iter()
            .map(|(px, py)| {
                (
                    ((px - mx).mul_add(grow, mx)).mul_add(scale, x),
                    ((py - my).mul_add(grow, my)).mul_add(scale, y),
                )
            })
            .collect()
    };
    let bounds = (
        x - 4.0 * scale,
        y - 4.0 * scale,
        x + 18.0 * scale,
        y + 26.0 * scale,
    );
    let edge = at(1.18);
    shade(canvas, bounds, Rgba([17, 17, 20, 255]), 0.95, |px, py| {
        inside(&edge, px, py)
    });
    let body = at(1.0);
    shade(canvas, bounds, Rgba([250, 250, 250, 255]), 1.0, |px, py| {
        inside(&body, px, py)
    });
}

/// Even-odd point in polygon.
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut on = false;
    let mut j = poly.len().saturating_sub(1);
    for (i, &(xi, yi)) in poly.iter().enumerate() {
        let (xj, yj) = poly.get(j).copied().unwrap_or((xi, yi));
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            on = !on;
        }
        j = i;
    }
    on
}

// ── Conversions the lints want named ──────────────────────────────────

#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn points(n: u32) -> f32 {
    n as f32
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn index_f32(n: usize) -> f32 {
    n as f32
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn whole(n: i64) -> f32 {
    n as f32
}

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn round_i64(v: f32) -> i64 {
    if v.is_finite() {
        v.round().clamp(-1.0e9, 1.0e9) as i64
    } else {
        0
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
fn round_u32(v: f32) -> u32 {
    if v.is_finite() && v > 0.0 {
        v.round().min(1.0e9) as u32
    } else {
        0
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
const fn channel(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// The height that keeps `w × h`'s shape at `width`.
fn scaled_height(w: u32, h: u32, width: u32) -> u32 {
    let scaled = u64::from(h)
        .saturating_mul(u64::from(width))
        .checked_div(u64::from(w.max(1)))
        .unwrap_or(1);
    u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
}
