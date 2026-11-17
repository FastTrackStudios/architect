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
//! The tree belongs to the pane, not to a window: a pane moved to a
//! window of its own is drawn from the same tree there.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyrender::PaintScene;
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
        }
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

    /// Puts `layer` at `key`, in place of what was there.
    pub fn set(&mut self, key: Key, layer: Layer) {
        let reorder = self.held.get(&key).is_none_or(|old| old.z != layer.z);
        self.held.insert(key, layer);
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
        let z = layer.z;
        change(layer);
        if layer.z != z {
            self.reorder();
        }
        self.changed();
        true
    }

    /// Takes the layer at `key` away: whether there was one.
    pub fn remove(&mut self, key: Key) -> bool {
        let had = self.held.remove(&key).is_some();
        if had {
            self.reorder();
            self.changed();
        }
        had
    }

    /// Takes every layer away.
    pub fn clear(&mut self) {
        if !self.held.is_empty() {
            self.held.clear();
            self.order.clear();
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
    pub fn in_order(&self) -> impl Iterator<Item = (Key, &Layer)> {
        self.order
            .iter()
            .filter_map(|key| self.held.get(key).map(|layer| (*key, layer)))
    }

    /// Draws every layer into `scene`, `root` taking the pane's logical
    /// pixels to the scene's.
    ///
    /// Nothing is copied: each command is drawn from where it is kept.
    // kurbo's transforms compose with `*`: no overflow to guard.
    #[allow(clippy::arithmetic_side_effects)]
    pub fn compose(&self, scene: &mut impl PaintScene, root: Affine) {
        for (_, layer) in self.in_order() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if let Some(clip) = layer.clip {
                scene.push_clip_layer(Fill::NonZero, root, &clip);
            }
            let faded = layer.opacity < 1.0;
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
                    layer.opacity,
                    Affine::IDENTITY,
                    &all,
                    None,
                    None,
                );
            }
            replay(scene, &layer.commands, root * layer.transform);
            if faded {
                scene.pop_layer();
            }
            if layer.clip.is_some() {
                scene.pop_layer();
            }
        }
    }
}

impl FromIterator<(Key, Layer)> for Layers {
    fn from_iter<I: IntoIterator<Item = (Key, Layer)>>(layers: I) -> Self {
        let mut out = Self::new();
        out.held = layers.into_iter().collect();
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
