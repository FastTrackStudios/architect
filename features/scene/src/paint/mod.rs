//! The `paint` kind (feature `paint`;
//! `docs/content/architecture/embedded-views.md`, "Paint views"): a view
//! that records a scene each frame, drawn in-process by the host's own
//! renderer in the pane's place.
//!
//! A GPU view, a Frame surface or a chart draws through it.
//!
//! A [`PaintView`] is given the pane's [`ViewCtx`] — its size, its input
//! (IME too), the frame clock, the theme's tokens, the host's typed data
//! — and answers with an `anyrender` [`Scene`] in its own logical pixels.
//! [`Painter`] is the host's side: it hands the view its input, asks for
//! a frame at the pane's size and scale, and times each one
//! ([`FrameStats`]). [`Marks`] is a display list a view can draw and
//! hit-test from, so what is drawn and what a pointer finds are one list.
//!
//! A view that returns layers ([`PaintView::layers`]) may say how they
//! move ([`anim`]): the painter keeps the layers it was given and draws
//! them at each frame's clocks ([`Clocks`]) without asking the view, for
//! as long as nothing the view draws changed. The same evaluation draws
//! the same layers from a view in a process.
//!
//! This module is the model: no Dioxus, wasm-clean. The Blitz host —
//! `PaintPane`, a pane mounting a `PaintWidget` — is feature `paint-ui`.

#[cfg(feature = "paint-ui")]
mod widget;

#[cfg(all(feature = "paint-web", target_arch = "wasm32"))]
pub mod web;

#[cfg(all(feature = "paint-web-ui", target_arch = "wasm32"))]
pub mod web_ui;

// Plain arithmetic, so its tests run natively without the web stack
// (`cargo xtask feature-tests`).
#[cfg(any(all(feature = "paint-web", target_arch = "wasm32"), test))]
mod web_geometry;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

// What a view records with, at the versions the host draws: a crate
// that paints needs no anyrender or peniko of its own.
pub use crate::Layer;
pub use crate::anim::{self, Clocks, Readings};
use crate::anim::{Name, Space, Values};
pub use crate::layers::Key as LayerKey;
pub use crate::region::{self, Cursor, RegionEvent};
pub use crate::wake;
pub use anyrender::{PaintScene, Scene};
pub use peniko;
use peniko::kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};

#[cfg(feature = "paint-ui")]
pub use widget::{
    Chrome, PaintMount, PaintPane, PaintWake, PaintWidget, PaneChrome, WHEEL_LINE, left_view,
    view_input,
};

use crate::view::{Geometry, ThemeTokens, ViewCtx, ViewInput};
use architect_views::meter::percentile;

/// A view of the [`PAINT`](crate::PAINT) kind.
pub trait PaintView: 'static {
    /// One input event, in its logical pixels: whether it changed what it
    /// draws (the host draws a frame when it did).
    fn input(&mut self, ctx: &ViewCtx, event: &ViewInput) -> bool;

    /// This frame, recorded into `scene` in its logical pixels over
    /// `ctx.geometry` (the host scales it to the display's).
    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene);

    /// This frame as layers, for a view that keeps what it draws: `None`
    /// (the default) and it is [`Self::paint`]'s one scene.
    ///
    /// Each layer is content in its own space and where it is drawn. Keep
    /// a layer's `commands` the same `Arc` while its content is the same:
    /// a pan or a zoom is then a new transform and nothing is recorded
    /// again, here or in a host the view runs outside of, where only what
    /// changed crosses. What moves every frame (a playhead, a scroll)
    /// should move by its layer's animations ([`Layer::animated`]): its
    /// host runs them and the view is not asked. A view that returns
    /// layers is not asked to `paint`, and is asked for layers only when
    /// input, a resize or [`Painter::invalidate`] may have changed them,
    /// or while it says it is [`Self::animating`].
    fn layers(&mut self, ctx: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        let _ = ctx;
        None
    }

    /// Its named values ([`anim::Values`]) that a layer's animation may
    /// read: asked with its layers, and only its own (`view.` names).
    fn values(&mut self, ctx: &ViewCtx) -> Vec<(Name, f64)> {
        let _ = ctx;
        Vec::new()
    }

    /// What a pointer did to one of its layers' regions
    /// ([`region::RegionEvent`]): enter, leave, press, release, or a
    /// drag's value (at most once a frame). Whether what it draws
    /// changed (it is asked for its layers again). Its host handles the
    /// hover, the cursor, the highlight and the drag itself; a press on a
    /// region is the region's, not a [`ViewInput`].
    fn region(&mut self, ctx: &ViewCtx, event: &RegionEvent) -> bool {
        let _ = (ctx, event);
        false
    }

    /// What is under `at` (logical pixels): the id of what it drew there.
    /// A host asks it for a tooltip, a context menu or an agent's look.
    fn hit(&self, ctx: &ViewCtx, at: Point) -> Option<u64> {
        let _ = (ctx, at);
        None
    }

    /// Whether it must be asked for every frame, not only after input.
    /// Layers that move by their animations need not be: their host
    /// draws them every frame a clock they read runs.
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
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
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
            .map(peniko::color::DynamicColor::to_alpha_color)
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

/// The host's side of a paint view: its ctx, the view, its frame times.
pub struct Painter {
    ctx: ViewCtx,
    view: Box<dyn PaintView>,
    stats: FrameStats,
    dirty: bool,
    clocks: Clocks,
    /// What a view of layers last returned, drawn again at each frame's
    /// clocks without it.
    held: Option<Held>,
    /// Why its view failed: it is drawn as nothing and asked nothing
    /// again, as a view in a process that sent what its host refused.
    failed: Option<crate::Error>,
    /// A cursor its regions asked for since the host last looked.
    cursor: Option<Cursor>,
    /// The engine's values its layers read, when granted them.
    engine: Option<Arc<crate::engine::EngineTable>>,
}

/// A view's layers and values as its painter keeps them.
#[derive(Debug, Default)]
struct Held {
    layers: crate::Layers,
    values: Values,
    /// The clocks it was last drawn at.
    drawn: Readings,
    /// Its values' revision when it was last drawn.
    drawn_values: u64,
    /// What it highlighted when last drawn.
    drawn_lit: Option<region::Lit>,
    /// The pointer over its regions.
    interaction: region::Interaction,
}

impl Held {
    /// `layers` and `values` kept, checked as a host checks what a view
    /// in a process sends it ([`crate::ops::Receiver`]): the
    /// same limits and the same errors, so a view fails the same way
    /// linked in or not.
    fn keep(
        &mut self,
        layers: Vec<(LayerKey, Layer)>,
        values: Vec<(Name, f64)>,
    ) -> Result<(), crate::Error> {
        use crate::Error;
        if layers.len() > crate::ops::MAX_LAYERS {
            return Err(Error::TooManyLayers(crate::ops::MAX_LAYERS));
        }
        let mut animations = 0_usize;
        for (_, layer) in &layers {
            if let Some(a) = layer.animations.as_deref() {
                anim::check(a)?;
                animations = animations.saturating_add(a.len());
            }
        }
        if animations > anim::MAX_ANIMATIONS {
            return Err(Error::TooManyAnimations(anim::MAX_ANIMATIONS));
        }
        let mut regions = 0_usize;
        for (key, layer) in &layers {
            if let Some(r) = layer.regions.as_deref() {
                region::check(r)?;
                regions = regions.saturating_add(r.len());
                let old = self.layers.get(*key).and_then(|l| l.regions.as_deref());
                region::seed(old, r, &mut self.values)?;
            }
        }
        if regions > region::MAX_REGIONS {
            return Err(Error::TooManyRegions(region::MAX_REGIONS));
        }
        for (name, value) in values {
            self.values.write(Space::View, &name, value)?;
        }
        self.layers = layers.into_iter().collect();
        Ok(())
    }
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
        // Wall time is the pane's frame clock: one clock for the view
        // and its animations.
        let frames = ctx.clock.clone();
        Self {
            ctx,
            view,
            stats: FrameStats::new(),
            dirty: true,
            clocks: Clocks::app().with_wall(move || frames.now().as_secs_f64()),
            held: None,
            failed: None,
            cursor: None,
            engine: None,
        }
    }

    /// Its layers read the engine's values from `table` (its extension
    /// was granted `engine`): drawn at the level the engine last wrote,
    /// every frame while it writes.
    #[must_use]
    pub fn with_engine(mut self, table: Arc<crate::engine::EngineTable>) -> Self {
        if let Some(held) = self.held.as_mut() {
            held.values.set_engine(Some(Arc::clone(&table)));
        }
        self.engine = Some(table);
        self
    }

    /// The engine's table its layers read, if granted one.
    #[must_use]
    pub const fn engine(&self) -> Option<&Arc<crate::engine::EngineTable>> {
        self.engine.as_ref()
    }

    /// A pointer event, handled by its layers' regions first: whether it
    /// was theirs (a press on a region, a drag), and so not the view's.
    /// Hover and drags are drawn without asking the view; what happened
    /// reaches it at the next frame ([`PaintView::region`]).
    pub fn pointer(&mut self, input: &ViewInput) -> bool {
        let Some(held) = self.held.as_mut().filter(|_| self.failed.is_none()) else {
            return false;
        };
        let now = self.clocks.read();
        let (layers, values, i) = (&held.layers, &mut held.values, &mut held.interaction);
        let at = |x: f32, y: f32| Point::new(f64::from(x), f64::from(y));
        let outcome = match *input {
            ViewInput::PointerMove { x, y, .. } => i.moved(layers, &now, values, at(x, y)),
            ViewInput::PointerDown { x, y, ref mods, .. } => i.pressed_with(
                layers,
                &now,
                values,
                at(x, y),
                region::Mods::from_names(mods),
            ),
            ViewInput::PointerUp { x, y, ref mods, .. } => i.released_with(
                layers,
                &now,
                values,
                at(x, y),
                region::Mods::from_names(mods),
            ),
            ViewInput::Touch { x, y, phase, .. } => match phase {
                "start" => i.pressed(layers, &now, values, at(x, y)),
                "move" => i.moved(layers, &now, values, at(x, y)),
                "end" => i.released(layers, &now, values, at(x, y)),
                _ => i.left(),
            },
            ViewInput::Focus(false) | ViewInput::PointerLeave => i.left(),
            _ => return false,
        };
        if let Some(cursor) = outcome.cursor {
            self.cursor = Some(cursor);
        }
        outcome.consumed
    }

    /// The cursor its regions asked for since last asked, if it changed
    /// ([`Cursor::Default`]: the pane's own).
    pub const fn take_cursor(&mut self) -> Option<Cursor> {
        self.cursor.take()
    }

    /// Why its view failed, if it did: what it returned broke a rule a
    /// host enforces on a view in a process.
    #[must_use]
    pub const fn failed(&self) -> Option<&crate::Error> {
        self.failed.as_ref()
    }

    /// When what it holds next changes on its own: every frame, after a
    /// while (a step), or never.
    #[must_use]
    pub fn next(&self) -> anim::Next {
        self.held
            .as_ref()
            .filter(|_| self.failed.is_none())
            .map_or(anim::Next::Never, |h| h.layers.next(&self.clocks.read()))
    }

    /// Its animations run on `clocks` (the app's, with the pane's frame
    /// clock as wall time, unless told).
    #[must_use]
    pub fn with_clocks(mut self, clocks: Clocks) -> Self {
        self.clocks = clocks;
        self
    }

    /// Its view's named values, once it returned layers.
    #[must_use]
    pub fn values(&self) -> Option<&Values> {
        self.held.as_ref().map(|h| &h.values)
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
    pub const fn invalidate(&mut self) {
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
    /// is waiting, it is moving, or a clock its layers read moved.
    #[must_use]
    pub fn wants_frame(&self) -> bool {
        if self.failed.is_some() {
            return false;
        }
        let pointed = self.held.as_ref().is_some_and(|h| {
            h.interaction.pending()
                || h.interaction.lit() != h.drawn_lit
                || h.values.revision() != h.drawn_values
        });
        pointed
            || self.dirty
            || !self.ctx.input.is_empty()
            || self.view.animating(&self.ctx)
            || self
                .held
                .as_ref()
                .is_some_and(|h| h.layers.stale(&h.drawn, &self.clocks.read()))
    }

    /// Whether it is moving (it draws every frame): the view says so, or
    /// its layers move on a running clock.
    #[must_use]
    pub fn animating(&self) -> bool {
        if self.failed.is_some() {
            return false;
        }
        self.view.animating(&self.ctx)
            || self.held.as_ref().is_some_and(|h| {
                h.layers.moving(&self.clocks.read())
                    || h.interaction.coasting()
                    || (h.values.engine().is_some() && h.values.revision() != h.drawn_values)
            })
    }

    /// Whether the next frame asks the view, or draws what it holds.
    fn asks(&self, resized: bool) -> bool {
        self.dirty || resized || self.held.is_none() || self.view.animating(&self.ctx)
    }

    /// What is under `at`, in logical pixels ([`PaintView::hit`]).
    #[must_use]
    pub fn hit(&self, at: Point) -> Option<u64> {
        self.view.hit(&self.ctx, at)
    }

    /// One frame at `geometry`: the view's scene, scaled to physical
    /// pixels. Its time — the view's input and its recording, on the
    /// host's clock — goes into the stats.
    ///
    /// A view of layers is asked only when something it draws may have
    /// changed (input, a resize, [`Self::invalidate`], or it says it is
    /// [`PaintView::animating`]); otherwise its layers are drawn again
    /// as their animations place them now.
    pub fn frame(&mut self, geometry: Geometry) -> Scene {
        let began = self.ctx.clock.now();
        let resized = self.ctx.geometry != geometry;
        self.ctx.geometry = geometry;
        self.pump();
        let scale = f64::from(geometry.scale);
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let mut scene = Scene::new();
        if self.failed.is_some() {
            self.dirty = false;
            return scene;
        }
        // What its regions heard reaches the view first: what it answers
        // may change what it draws.
        if let Some(held) = self.held.as_mut() {
            for event in held.interaction.take() {
                self.dirty |= self.view.region(&self.ctx, &event);
            }
        }
        let asked = if self.asks(resized) {
            Some(self.view.layers(&self.ctx))
        } else {
            None
        };
        let held = match asked {
            Some(Some(layers)) => {
                let values = self.view.values(&self.ctx);
                let engine = self.engine.clone();
                let held = self.held.get_or_insert_with(|| Held {
                    values: engine
                        .map_or_else(Values::default, |t| Values::default().with_engine(t)),
                    ..Held::default()
                });
                if let Err(why) = held.keep(layers, values) {
                    self.failed = Some(why);
                    self.held = None;
                    self.dirty = false;
                    return scene;
                }
                Some(held)
            }
            // A view that paints: no layers to hold.
            Some(None) => {
                self.held = None;
                None
            }
            None => self.held.as_mut(),
        };
        if let Some(held) = held {
            // Composed from where each layer's content is kept, at the
            // clocks now.
            let now = self.clocks.read();
            // A flicked value coasts on: the host's, as a drag is.
            held.interaction.coast(&now, &mut held.values);
            let lit = held.interaction.lit();
            held.layers
                .compose_lit(&mut scene, Affine::scale(scale), &now, &held.values, lit);
            held.drawn = now;
            held.drawn_values = held.values.revision();
            held.drawn_lit = lit;
        } else {
            self.view.paint(&self.ctx, &mut scene);
            if (scale - 1.0).abs() > f64::EPSILON {
                let mut scaled = Scene::new();
                scaled.append_scene(scene, Affine::scale(scale));
                scene = scaled;
            }
        }
        self.stats
            .record(self.ctx.clock.now().saturating_sub(began));
        self.dirty = false;
        scene
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod anim_tests;
