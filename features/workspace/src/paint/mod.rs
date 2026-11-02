//! The `paint` kind (feature `paint`;
//! `docs/content/architecture/embedded-views.md`, "Paint views"): a view
//! that records a scene each frame, drawn in-process by the host's own
//! renderer in the pane's place. A GPU view, a Frame surface or a chart
//! draws through it.
//!
//! A [`PaintView`] is given the pane's [`ViewCtx`] — its size, its input
//! (IME too), the frame clock, the theme's tokens, the host's typed data
//! — and answers with an `anyrender` [`Scene`] in its own logical pixels.
//! [`Painter`] is the host's side: it hands the view its input, asks for
//! a frame at the pane's size and scale, and times each one
//! ([`FrameStats`]). [`Marks`] is a display list a view can draw and
//! hit-test from, so what is drawn and what a pointer finds are one list.
//!
//! This module is the model: no Dioxus, wasm-clean. The Blitz host —
//! `PaintPane`, a pane mounting a `PaintWidget` — is feature `paint-ui`.

#[cfg(feature = "paint-ui")]
mod widget;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

// What a view records with, at the versions the host draws: a crate
// that paints needs no anyrender or peniko of its own.
pub use anyrender::{PaintScene, Scene};
pub use peniko;
use peniko::kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};

#[cfg(feature = "paint-ui")]
pub use widget::{PaintMount, PaintPane, PaintWake, PaintWidget, shell_tokens};

use crate::ctx::{Geometry, ThemeTokens, ViewCtx, ViewInput};

/// A view of the [`PAINT`](crate::PAINT) kind.
pub trait PaintView: 'static {
    /// One input event, in its logical pixels: whether it changed what it
    /// draws (the host draws a frame when it did).
    fn input(&mut self, ctx: &ViewCtx, event: &ViewInput) -> bool;

    /// This frame, recorded into `scene` in its logical pixels over
    /// `ctx.geometry` (the host scales it to the display's).
    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene);

    /// What is under `at` (logical pixels): the id of what it drew there.
    /// A host asks it for a tooltip, a context menu or an agent's look.
    fn hit(&self, ctx: &ViewCtx, at: Point) -> Option<u64> {
        let _ = (ctx, at);
        None
    }

    /// Whether it draws every frame (it is moving), not only after input.
    fn animating(&self, ctx: &ViewCtx) -> bool {
        let _ = ctx;
        false
    }
}

/// One rectangle a view draws, by its id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub id: u64,
    pub rect: Rect,
    pub fill: Color,
}

/// A display list: rectangles in paint order, the last on top. A view
/// that paints from it and hit-tests from it can't draw one thing and
/// find another under the pointer.
#[derive(Debug, Clone, Default)]
pub struct Marks(Vec<Mark>);

impl Marks {
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, mark: Mark) {
        self.0.push(mark);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// In paint order.
    #[must_use]
    pub fn as_slice(&self) -> &[Mark] {
        &self.0
    }

    /// To restyle in place (a hover, a selection), in paint order.
    pub fn as_mut_slice(&mut self) -> &mut [Mark] {
        &mut self.0
    }

    /// Every mark into `scene`, under `transform`.
    pub fn paint(&self, scene: &mut Scene, transform: Affine) {
        for mark in &self.0 {
            scene.fill(
                Fill::NonZero,
                transform,
                mark.fill,
                Option::<Affine>::None,
                &mark.rect,
            );
        }
    }

    /// The topmost mark at `at`: the one drawn there.
    #[must_use]
    pub fn hit(&self, at: Point) -> Option<&Mark> {
        self.0.iter().rev().find(|m| m.rect.contains(at))
    }
}

impl FromIterator<Mark> for Marks {
    fn from_iter<I: IntoIterator<Item = Mark>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl ThemeTokens {
    /// The token `name` as a colour (any CSS colour), if it is one.
    #[must_use]
    pub fn color(&self, name: &str) -> Option<Color> {
        let value = self.get(name)?;
        peniko::color::parse_color(value)
            .ok()
            .map(|c| c.to_alpha_color())
    }
}

/// How long a view's last frames took: the last [`FrameStats::KEPT`],
/// on the host's clock. Cheap to clone; every clone is the same record.
#[derive(Clone, Default)]
pub struct FrameStats(Arc<Mutex<Times>>);

#[derive(Default)]
struct Times {
    last: VecDeque<Duration>,
    frames: u64,
}

/// [`FrameStats`] now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameSummary {
    /// Frames ever drawn.
    pub frames: u64,
    /// Of the last ones kept: the median, the 99th percentile and the
    /// slowest.
    pub p50: Duration,
    pub p99: Duration,
    pub max: Duration,
}

impl std::fmt::Debug for FrameStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FrameStats").field(&self.summary()).finish()
    }
}

impl PartialEq for FrameStats {
    /// The same record, not an equal one.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl FrameStats {
    /// Frames kept for the percentiles.
    pub const KEPT: usize = 1024;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, took: Duration) {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if t.last.len() == Self::KEPT {
            t.last.pop_front();
        }
        t.last.push_back(took);
        t.frames = t.frames.saturating_add(1);
    }

    #[must_use]
    pub fn summary(&self) -> FrameSummary {
        let t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sorted: Vec<Duration> = t.last.iter().copied().collect();
        sorted.sort_unstable();
        FrameSummary {
            frames: t.frames,
            p50: percentile(&sorted, 50),
            p99: percentile(&sorted, 99),
            max: sorted.last().copied().unwrap_or_default(),
        }
    }
}

/// The nearest-rank `p`th percentile of `sorted`.
fn percentile(sorted: &[Duration], p: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = sorted.len().saturating_mul(p).div_ceil(100).max(1);
    sorted
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// The host's side of a paint view: its ctx, the view, its frame times.
pub struct Painter {
    ctx: ViewCtx,
    view: Box<dyn PaintView>,
    stats: FrameStats,
    dirty: bool,
}

impl std::fmt::Debug for Painter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Painter")
            .field("pane", &self.ctx.pane)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Painter {
    /// `view` mounted with `ctx`, its first frame wanted.
    pub fn new(ctx: ViewCtx, view: impl PaintView) -> Self {
        Self::boxed(ctx, Box::new(view))
    }

    #[must_use]
    pub fn boxed(ctx: ViewCtx, view: Box<dyn PaintView>) -> Self {
        Self {
            ctx,
            view,
            stats: FrameStats::new(),
            dirty: true,
        }
    }

    /// Recording its frames into `stats` (one the host reads elsewhere).
    #[must_use]
    pub fn with_stats(mut self, stats: FrameStats) -> Self {
        self.stats = stats;
        self
    }

    #[must_use]
    pub const fn ctx(&self) -> &ViewCtx {
        &self.ctx
    }

    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.stats.clone()
    }

    /// Hands the view what is waiting in its input: whether it wants a
    /// frame now.
    pub fn pump(&mut self) -> bool {
        for event in self.ctx.input.drain() {
            self.dirty |= self.view.input(&self.ctx, &event);
        }
        self.dirty
    }

    /// Request a frame after view data changes outside the input queue.
    /// The host must also wake its renderer if it is sleeping.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Update the tokens seen by input and paint, requesting a frame if changed.
    pub fn set_theme(&mut self, theme: ThemeTokens) {
        if self.ctx.theme != theme {
            self.ctx.theme = theme;
            self.invalidate();
        }
    }

    /// Whether a frame would show something new: input changed it, input
    /// is waiting, or it is moving.
    #[must_use]
    pub fn wants_frame(&self) -> bool {
        self.dirty || !self.ctx.input.is_empty() || self.view.animating(&self.ctx)
    }

    /// Whether it is moving (it draws every frame).
    #[must_use]
    pub fn animating(&self) -> bool {
        self.view.animating(&self.ctx)
    }

    /// What is under `at`, in logical pixels ([`PaintView::hit`]).
    #[must_use]
    pub fn hit(&self, at: Point) -> Option<u64> {
        self.view.hit(&self.ctx, at)
    }

    /// One frame at `geometry`: the view's scene, scaled to physical
    /// pixels. Its time — the view's input and its recording, on the
    /// host's clock — goes into the stats.
    pub fn frame(&mut self, geometry: Geometry) -> Scene {
        let began = self.ctx.clock.now();
        self.ctx.geometry = geometry;
        self.pump();
        let mut scene = Scene::new();
        self.view.paint(&self.ctx, &mut scene);
        let scale = f64::from(geometry.scale);
        if (scale - 1.0).abs() > f64::EPSILON && scale > 0.0 {
            let mut scaled = Scene::new();
            scaled.append_scene(scene, Affine::scale(scale));
            scene = scaled;
        }
        self.stats
            .record(self.ctx.clock.now().saturating_sub(began));
        self.dirty = false;
        scene
    }
}

#[cfg(test)]
mod tests;
