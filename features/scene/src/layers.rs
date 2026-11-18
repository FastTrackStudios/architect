//! Layers: what a pane draws, kept by its host.
//!
//! A view says what it draws once, as layers by key, and after that only
//! what changed: one layer's content, or where it is. The host composes
//! the layers into its window's scene every frame without asking the
//! view, so a pan, a zoom or a moving playhead costs the view nothing.
//!
//! A layer's content is drawing commands, shared: a linked view puts in
//! what it recorded, and a view that runs elsewhere puts in what its
//! host read from a fragment ([`crate::decode`]). Either way the tree is
//! the same, and so is what is drawn.
//!
//! A layer may also say how it moves ([`crate::anim`]): its host then
//! draws it every frame its clocks move, from the same tree, without the
//! view ([`Layers::compose_at`]).
//!
//! The tree belongs to the pane, not to a window: a pane moved to a
//! window of its own is drawn from the same tree there.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyrender::PaintScene;

use crate::anim::{self, Animation, Readings, Values};
use crate::region::{self, Lit, Region};
use anyrender::recording::RenderCommand;
use peniko::kurbo::{Affine, Rect};
use peniko::{Fill, Mix};

/// What a layer is named by, within its pane.
pub type Key = u64;

/// One layer: content, and where and how it is drawn.
#[derive(Debug, Clone)]
pub struct Layer {
    /// What it draws, in its own content space.
    pub commands: Arc<Vec<RenderCommand>>,
    /// Content space to the pane's logical pixels.
    pub transform: Affine,
    /// 0 is not drawn, 1 is drawn as recorded.
    pub opacity: f32,
    /// What it is clipped to, in the pane's logical pixels.
    pub clip: Option<Rect>,
    /// Higher is drawn later, so on top; equal ones in the order of
    /// their keys.
    pub z: i32,
    pub visible: bool,
    /// How it moves without its view ([`crate::anim`]): at most one per
    /// property.
    pub animations: Option<Arc<[Animation]>>,
    /// What a pointer does over it, handled by its host
    /// ([`crate::region`]).
    pub regions: Option<Arc<[Region]>>,
}

impl Layer {
    /// A layer of `commands`, drawn as recorded.
    #[must_use]
    pub const fn new(commands: Arc<Vec<RenderCommand>>) -> Self {
        Self {
            commands,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            clip: None,
            z: 0,
            visible: true,
            animations: None,
            regions: None,
        }
    }

    /// With `regions` a pointer finds, hovers, presses and drags.
    #[must_use]
    pub fn regions(mut self, regions: impl Into<Arc<[Region]>>) -> Self {
        let regions = regions.into();
        self.regions = (!regions.is_empty()).then_some(regions);
        self
    }

    /// How many regions it has.
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.as_ref().map_or(0, |r| r.len())
    }

    /// Moving as `animations` say, evaluated by its host each frame.
    #[must_use]
    pub fn animated(mut self, animations: impl Into<Arc<[Animation]>>) -> Self {
        let animations = animations.into();
        self.animations = (!animations.is_empty()).then_some(animations);
        self
    }

    /// How many animations it has.
    #[must_use]
    pub fn animation_count(&self) -> usize {
        self.animations.as_ref().map_or(0, |a| a.len())
    }

    #[must_use]
    pub const fn at(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    #[must_use]
    pub const fn z(mut self, z: i32) -> Self {
        self.z = z;
        self
    }

    #[must_use]
    pub const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    #[must_use]
    pub const fn clipped(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }
}

/// A pane's layers.
#[derive(Debug, Clone, Default)]
pub struct Layers {
    held: BTreeMap<Key, Layer>,
    /// The keys in paint order; rebuilt when a layer comes, goes or
    /// changes its `z`.
    order: Vec<Key>,
    revision: u64,
    /// Animations over every layer.
    animations: usize,
    /// Regions over every layer.
    regions: usize,
}

impl Layers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times it has changed: what a host compares to know
    /// whether to draw again.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.held.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: Key) -> Option<&Layer> {
        self.held.get(&key)
    }

    /// How many animations its layers have, over all of them.
    #[must_use]
    pub const fn animation_count(&self) -> usize {
        self.animations
    }

    /// How many regions its layers have, over all of them.
    #[must_use]
    pub const fn region_count(&self) -> usize {
        self.regions
    }

    /// Puts `layer` at `key`, in place of what was there.
    pub fn set(&mut self, key: Key, layer: Layer) {
        let reorder = self.held.get(&key).is_none_or(|old| old.z != layer.z);
        self.animations = self.animations.saturating_add(layer.animation_count());
        self.regions = self.regions.saturating_add(layer.region_count());
        if let Some(old) = self.held.insert(key, layer) {
            self.animations = self.animations.saturating_sub(old.animation_count());
            self.regions = self.regions.saturating_sub(old.region_count());
        }
        if reorder {
            self.reorder();
        }
        self.changed();
    }

    /// Changes the layer at `key` in place: whether there was one.
    pub fn update(&mut self, key: Key, change: impl FnOnce(&mut Layer)) -> bool {
        let Some(layer) = self.held.get_mut(&key) else {
            return false;
        };
        let (z, before, had) = (layer.z, layer.animation_count(), layer.region_count());
        change(layer);
        self.animations = self
            .animations
            .saturating_sub(before)
            .saturating_add(layer.animation_count());
        self.regions = self
            .regions
            .saturating_sub(had)
            .saturating_add(layer.region_count());
        if layer.z != z {
            self.reorder();
        }
        self.changed();
        true
    }

    /// Takes the layer at `key` away: whether there was one.
    pub fn remove(&mut self, key: Key) -> bool {
        let had = self.held.remove(&key);
        if let Some(old) = &had {
            self.animations = self.animations.saturating_sub(old.animation_count());
            self.regions = self.regions.saturating_sub(old.region_count());
            self.reorder();
            self.changed();
        }
        had.is_some()
    }

    /// Takes every layer away.
    pub fn clear(&mut self) {
        if !self.held.is_empty() {
            self.held.clear();
            self.order.clear();
            self.animations = 0;
            self.regions = 0;
            self.changed();
        }
    }

    const fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn reorder(&mut self) {
        self.order = self.held.keys().copied().collect();
        // Stable, and the keys come sorted: equal `z` keeps key order.
        self.order
            .sort_by_key(|key| self.held.get(key).map_or(0, |layer| layer.z));
    }

    /// The layers in paint order, the first at the back.
    pub fn in_order(&self) -> impl DoubleEndedIterator<Item = (Key, &Layer)> {
        self.order
            .iter()
            .filter_map(|key| self.held.get(key).map(|layer| (*key, layer)))
    }

    /// Draws every layer into `scene`, `root` taking the pane's logical
    /// pixels to the scene's, each where it was sent (its animations not
    /// run: [`Self::compose_at`] runs them).
    ///
    /// Nothing is copied: each command is drawn from where it is kept.
    pub fn compose(&self, scene: &mut impl PaintScene, root: Affine) {
        self.compose_at(scene, root, &Readings::NONE, &Values::default());
    }

    /// Draws every layer into `scene` as [`Self::compose`] does, each
    /// placed by its animations at these clocks and values
    /// ([`anim::place`]).
    pub fn compose_at(
        &self,
        scene: &mut impl PaintScene,
        root: Affine,
        readings: &Readings,
        values: &Values,
    ) {
        self.compose_lit(scene, root, readings, values, None);
    }

    /// Draws every layer as [`Self::compose_at`] does, and over the
    /// region `lit` names its hover or press highlight
    /// ([`crate::region`]).
    // kurbo's transforms compose with `*`: no overflow to guard.
    #[allow(clippy::arithmetic_side_effects)]
    pub fn compose_lit(
        &self,
        scene: &mut impl PaintScene,
        root: Affine,
        readings: &Readings,
        values: &Values,
        lit: Option<Lit>,
    ) {
        for (key, layer) in self.in_order() {
            let placed = anim::place(layer, readings, values);
            if !placed.visible || placed.opacity <= 0.0 {
                continue;
            }
            if let Some(clip) = layer.clip {
                scene.push_clip_layer(Fill::NonZero, root, &clip);
            }
            let faded = placed.opacity < 1.0;
            if faded {
                // A layer needs a shape: everything a scene may hold.
                let all = Rect::new(
                    -crate::MAX_COORDINATE,
                    -crate::MAX_COORDINATE,
                    crate::MAX_COORDINATE,
                    crate::MAX_COORDINATE,
                );
                scene.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    placed.opacity,
                    Affine::IDENTITY,
                    &all,
                    None,
                    None,
                );
            }
            replay(scene, &layer.commands, root * placed.transform);
            if let Some(lit) = lit.filter(|l| l.hit.key == key)
                && let Some(r) = layer
                    .regions
                    .as_deref()
                    .and_then(|rs| rs.iter().find(|r| r.id == lit.hit.id))
            {
                region::draw_lit(scene, r, root * placed.transform, lit.pressed);
            }
            if faded {
                scene.pop_layer();
            }
            if layer.clip.is_some() {
                scene.pop_layer();
            }
        }
    }

    fn animated(&self) -> impl Iterator<Item = &[Animation]> {
        self.held
            .values()
            .filter_map(|layer| layer.animations.as_deref())
    }

    /// When what it draws next changes on its own at `readings`: every
    /// frame while a layer moves on a running clock, at the next step or
    /// clamp, or never. Its host draws every frame only while it is
    /// [`anim::Next::Every`], and sleeps until the next change otherwise.
    #[must_use]
    pub fn next(&self, readings: &Readings) -> anim::Next {
        if self.animations == 0 {
            return anim::Next::Never;
        }
        self.animated()
            .fold(anim::Next::Never, |n, a| n.and(anim::next(a, readings)))
    }

    /// Whether a layer moves every frame at `readings`.
    #[must_use]
    pub fn moving(&self, readings: &Readings) -> bool {
        self.next(readings) == anim::Next::Every
    }

    /// Whether what it draws at `now` differs from what it drew at
    /// `drawn` by its clocks: a frame of playback, a step reached, a
    /// seek that moves something.
    #[must_use]
    pub fn stale(&self, drawn: &Readings, now: &Readings) -> bool {
        self.animations > 0 && self.animated().any(|a| anim::differs(a, drawn, now))
    }
}

impl FromIterator<(Key, Layer)> for Layers {
    fn from_iter<I: IntoIterator<Item = (Key, Layer)>>(layers: I) -> Self {
        let mut out = Self::new();
        out.held = layers.into_iter().collect();
        out.animations = out
            .held
            .values()
            .map(Layer::animation_count)
            .fold(0, usize::saturating_add);
        out.regions = out
            .held
            .values()
            .map(Layer::region_count)
            .fold(0, usize::saturating_add);
        out.reorder();
        out
    }
}

/// Draws `commands` into `scene` at `transform`, from where they are.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
pub fn replay(scene: &mut impl PaintScene, commands: &[RenderCommand], transform: Affine) {
    for command in commands {
        match command {
            RenderCommand::PushLayer(l) => scene.push_layer(
                l.fill,
                l.blend,
                l.alpha,
                transform * l.transform,
                &l.clip,
                l.filter.clone(),
                l.backdrop_filter.clone(),
            ),
            RenderCommand::PushClipLayer(c) => {
                scene.push_clip_layer(c.fill, transform * c.transform, &c.clip);
            }
            RenderCommand::PopLayer => scene.pop_layer(),
            RenderCommand::Stroke(s) => scene.stroke(
                &s.style,
                transform * s.transform,
                s.brush.as_ref(),
                s.brush_transform,
                &s.shape,
            ),
            RenderCommand::Fill(f) => scene.fill(
                f.fill,
                transform * f.transform,
                f.brush.as_ref(),
                f.brush_transform,
                &f.shape,
            ),
            RenderCommand::GlyphRun(g) => scene.draw_glyphs(
                &g.font_data,
                g.font_size,
                g.hint,
                &g.normalized_coords,
                g.embolden,
                &g.style,
                g.brush.as_ref(),
                g.brush_alpha,
                transform * g.transform,
                g.glyph_transform,
                g.glyphs.iter().copied(),
            ),
            RenderCommand::BoxShadow(b) => scene.draw_box_shadow(
                transform * b.transform,
                b.rect,
                b.brush,
                b.radius,
                b.std_dev,
            ),
        }
    }
}
