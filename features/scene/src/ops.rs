//! Operations: what a view tells its host about its layers.
//!
//! A [`Sender`] is the view's side. Each frame it is shown the layers the
//! view wants drawn and writes only what the host does not have yet: a
//! font or an image the first time it is drawn with, a fragment the first
//! time it is shown, a layer when it comes or changes, and for a layer
//! that only moved, its transform alone. A frame that changed nothing
//! writes nothing.
//!
//! A [`Receiver`] is the host's side: it reads a batch, checked, into the
//! pane's [`Layers`], which the host composes without the view.
//!
//! A layer that moves on its own ([`crate::anim`]) crosses once with its
//! animations (an `ANIMATE` after its `LAYER`), and its host runs them:
//! a frame of playback writes nothing. A named value a layer is bound to
//! crosses as its name and its number (`VALUE`), when it changes.
//!
//! A batch is bytes: [`MAGIC`], then operations until it ends. How a
//! batch travels (a process's socket, a component's call, a worker's
//! message) is its carrier's business, not this format's.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyrender::recording::RenderCommand;
use peniko::kurbo::{Affine, Rect};

use crate::anim::{self, Name, Space, Values};
use crate::bytes::{Reader, Writer};
use crate::fragment::{Encoder, decode};
use crate::layers::{Key, Layer, Layers};
use crate::region;
use crate::resource::{Resource, Resources};
use crate::{Error, Id};

/// How a batch starts: its format and that format's first version, which
/// a batch of the first seven operations alone still is.
pub const MAGIC: [u8; 4] = *b"ASO\x01";

/// The latest version of a batch this host reads and writes.
///
/// 2 added animations, values, hit regions and drags; 3 a region's
/// layers shown while it is hovered or pressed
/// ([`crate::region::Region::hover_layer`]). A batch is written at the
/// lowest version that holds what it says, so a host that reads only
/// the first refuses one it can't read whole, by its version
/// ([`Error::NotAFragment`] there, [`Error::Version`] here), before it
/// applies any of it.
pub const VERSION: u8 = 3;

/// The most bytes one batch may be.
pub const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

/// The most layers a pane may have.
pub const MAX_LAYERS: usize = 4096;

/// The most bytes of fragments and resources a pane's host keeps for it.
pub const MAX_HELD_BYTES: usize = 256 * 1024 * 1024;

const RESOURCE: u8 = 1;
const FRAGMENT: u8 = 2;
const LAYER: u8 = 3;
const MOVE: u8 = 4;
const REMOVE: u8 = 5;
const DROP: u8 = 6;
const CLEAR: u8 = 7;
/// A layer's animations, in place of those it had (none: it stops).
/// Added after the first seven, which read as they did: a batch without
/// one is the same bytes as before.
const ANIMATE: u8 = 8;
/// A named value the view writes ([`Space::View`]).
const VALUE: u8 = 9;
/// A layer's hit regions, in place of those it had (none: it has none).
const REGIONS: u8 = 10;

/// What the host was last told of a layer.
struct Sent {
    /// The content it was shown with, kept so the same content shown
    /// again is known by its pointer and not encoded again.
    content: Arc<Vec<RenderCommand>>,
    fragment: Id,
    layer: Layer,
}

/// The view's side: writes what changed since the last frame.
///
/// One per view that is open: a view opened again starts afresh, as its
/// host's [`Receiver`] does.
#[derive(Default)]
pub struct Sender {
    encoder: Encoder,
    resources: HashSet<Id>,
    fragments: HashSet<Id>,
    sent: BTreeMap<Key, Sent>,
    /// The values the host was last told, as their bits.
    values: BTreeMap<Name, u64>,
}

impl std::fmt::Debug for Sender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender")
            .field("layers", &self.sent.len())
            .field("fragments", &self.fragments.len())
            .field("resources", &self.resources.len())
            .finish_non_exhaustive()
    }
}

fn affine(w: &mut Writer, t: Affine) {
    for v in t.as_coeffs() {
        w.f64(v);
    }
}

#[allow(clippy::float_cmp)]
fn same_placement(a: &Layer, b: &Layer) -> bool {
    a.opacity == b.opacity && a.clip == b.clip && a.z == b.z && a.visible == b.visible
}

impl Sender {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The batch that makes the host's layers `layers`, or `None` when
    /// they already are.
    ///
    /// Content is known by its `Arc`: a layer shown with the same one as
    /// last frame is not encoded again. New content is encoded and
    /// hashed, and sent only if the host does not hold that fragment.
    pub fn frame(&mut self, layers: &[(Key, Layer)]) -> Option<Vec<u8>> {
        self.frame_with(layers, &[])
    }

    /// [`Self::frame`], and the values in `values` that changed since
    /// the host was last told (a value it no longer names keeps its last
    /// number there).
    pub fn frame_with(
        &mut self,
        layers: &[(Key, Layer)],
        values: &[(Name, f64)],
    ) -> Option<Vec<u8>> {
        let mut w = Writer::default();
        w.bytes(&MAGIC);
        let start = w.out.len();
        // The lowest version that holds what it says.
        let mut later = false;
        let mut swaps = false;
        for (key, layer) in layers {
            let known = self
                .sent
                .get(key)
                .filter(|sent| Arc::ptr_eq(&sent.content, &layer.commands))
                .map(|sent| sent.fragment);
            let fragment = known.unwrap_or_else(|| self.fragment(&mut w, &layer.commands));
            // Whether the host has this layer as it is but for where.
            let placed = self
                .sent
                .get(key)
                .filter(|sent| sent.fragment == fragment && same_placement(&sent.layer, layer))
                .map(|sent| sent.layer.transform);
            // What the host runs of it now, if it is told again.
            let animated_as = self
                .sent
                .get(key)
                .filter(|_| placed.is_some())
                .map(|sent| &sent.layer.animations);
            let regions_as = self
                .sent
                .get(key)
                .filter(|_| placed.is_some())
                .map(|sent| &sent.layer.regions);
            if regions_as.is_some_and(|was| *was != layer.regions) {
                later = true;
                w.u8(REGIONS);
                w.u64(*key);
                let regions = layer.regions.as_deref().unwrap_or_default();
                swaps |= regions.iter().any(region::Region::swaps);
                region::write(&mut w, regions);
            }
            if animated_as.is_some_and(|was| *was != layer.animations) {
                later = true;
                w.u8(ANIMATE);
                w.u64(*key);
                anim::write(&mut w, layer.animations.as_deref().unwrap_or_default());
            }
            match placed {
                Some(at) if at == layer.transform => {}
                Some(_) => {
                    w.u8(MOVE);
                    w.u64(*key);
                    affine(&mut w, layer.transform);
                }
                None => {
                    w.u8(LAYER);
                    w.u64(*key);
                    w.bytes(&fragment.0);
                    affine(&mut w, layer.transform);
                    w.f32(layer.opacity);
                    w.bool(layer.clip.is_some());
                    if let Some(clip) = layer.clip {
                        for v in [clip.x0, clip.y0, clip.x1, clip.y1] {
                            w.f64(v);
                        }
                    }
                    w.i32(layer.z);
                    w.bool(layer.visible);
                    // A layer comes still: it moves once told how, and
                    // has regions once told them.
                    if let Some(regions) = &layer.regions {
                        later = true;
                        w.u8(REGIONS);
                        w.u64(*key);
                        swaps |= regions.iter().any(region::Region::swaps);
                        region::write(&mut w, regions);
                    }
                    if let Some(animations) = &layer.animations {
                        later = true;
                        w.u8(ANIMATE);
                        w.u64(*key);
                        anim::write(&mut w, animations);
                    }
                }
            }
            self.sent.insert(
                *key,
                Sent {
                    content: Arc::clone(&layer.commands),
                    fragment,
                    layer: layer.clone(),
                },
            );
        }
        // Layers it no longer shows, then fragments none of them draws.
        let shown: HashSet<Key> = layers.iter().map(|(key, _)| *key).collect();
        let gone: Vec<Key> = self
            .sent
            .keys()
            .copied()
            .filter(|key| !shown.contains(key))
            .collect();
        for key in gone {
            self.sent.remove(&key);
            w.u8(REMOVE);
            w.u64(key);
        }
        let drawn: HashSet<Id> = self.sent.values().map(|sent| sent.fragment).collect();
        let unused: Vec<Id> = self.fragments.difference(&drawn).copied().collect();
        for id in unused {
            self.fragments.remove(&id);
            w.u8(DROP);
            w.bytes(&id.0);
        }
        for (name, value) in values {
            let bits = value.to_bits();
            if self.values.get(name) != Some(&bits) {
                self.values.insert(name.clone(), bits);
                later = true;
                w.u8(VALUE);
                anim::write_name(&mut w, name);
                w.f64(*value);
            }
        }
        let version = if swaps { Some(3) } else { later.then_some(2) };
        if let (Some(v), Some(at)) = (version, w.out.get_mut(MAGIC.len().saturating_sub(1))) {
            *at = v;
        }
        (w.out.len() > start).then_some(w.out)
    }

    /// `commands` as a fragment the host holds: its name, with the
    /// fragment and the resources it draws written if they are new.
    fn fragment(&mut self, w: &mut Writer, commands: &[RenderCommand]) -> Id {
        let encoded = self.encoder.encode(commands);
        if self.fragments.contains(&encoded.id) {
            return encoded.id;
        }
        for (id, resource) in &encoded.resources {
            if self.resources.insert(*id) {
                w.u8(RESOURCE);
                w.u8(resource.kind());
                let bytes = resource.to_bytes();
                w.count(bytes.len());
                w.bytes(&bytes);
            }
        }
        w.u8(FRAGMENT);
        w.count(encoded.bytes.len());
        w.bytes(&encoded.bytes);
        self.fragments.insert(encoded.id);
        encoded.id
    }
}

/// The host's side: a pane's layers as its view last said them.
#[derive(Debug, Default)]
pub struct Receiver {
    resources: Resources,
    fragments: HashMap<Id, (Arc<Vec<RenderCommand>>, usize)>,
    fragment_bytes: usize,
    layers: Layers,
    values: Values,
}

impl Receiver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reading the engine's values from `table`: its extension was
    /// granted them (`None`: not, and an animation bound to one reads
    /// nothing).
    pub fn grant(&mut self, table: Option<Arc<crate::engine::EngineTable>>) {
        self.values.set_engine(table);
    }

    /// The pane's named values, as its view last wrote them.
    #[must_use]
    pub const fn values(&self) -> &Values {
        &self.values
    }

    /// The pane's values, for the host to write its own
    /// ([`Space::Host`]).
    pub const fn values_mut(&mut self) -> &mut Values {
        &mut self.values
    }

    /// Its layers to read and its values to write at once: what a host's
    /// pointer needs to drag a region ([`crate::region::Interaction`]).
    pub const fn parts_mut(&mut self) -> (&Layers, &mut Values) {
        (&self.layers, &mut self.values)
    }

    /// The pane's layers.
    #[must_use]
    pub const fn layers(&self) -> &Layers {
        &self.layers
    }

    /// How many bytes of fragments and resources it holds.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.fragment_bytes.saturating_add(self.resources.bytes())
    }

    /// Applies one batch.
    ///
    /// # Errors
    ///
    /// What is wrong with it. The layers keep what was applied before
    /// the fault; the view that sent it fails.
    pub fn apply(&mut self, batch: &[u8]) -> Result<(), Error> {
        if batch.len() > MAX_BATCH_BYTES {
            return Err(Error::TooLarge(batch.len(), MAX_BATCH_BYTES));
        }
        let mut r = Reader::new(batch);
        let version = match r.take(MAGIC.len()).map_err(|_| Error::NotAFragment)? {
            [b'A', b'S', b'O', v] if (1..=VERSION).contains(v) => *v,
            [b'A', b'S', b'O', v] if *v > VERSION => return Err(Error::Version(*v, VERSION)),
            _ => return Err(Error::NotAFragment),
        };
        while r.left() > 0 {
            self.op(&mut r, version)?;
        }
        Ok(())
    }

    fn op(&mut self, r: &mut Reader<'_>, version: u8) -> Result<(), Error> {
        let op = r.u8()?;
        if op > CLEAR && version < 2 {
            return Err(Error::Unknown("operation of version 1", op));
        }
        match op {
            RESOURCE => {
                let kind = r.u8()?;
                let len = r.count()?;
                self.resources.receive(kind, r.take(len)?.to_vec())?;
                self.within()?;
            }
            FRAGMENT => {
                let len = r.count()?;
                let bytes = r.take(len)?;
                let commands = decode(bytes, &self.resources)?;
                let id = Id::of(&[bytes]);
                if self
                    .fragments
                    .insert(id, (Arc::new(commands), len))
                    .is_none()
                {
                    self.fragment_bytes = self.fragment_bytes.saturating_add(len);
                }
                self.within()?;
            }
            LAYER => {
                let key = r.u64()?;
                let fragment = id(r)?;
                let (commands, _) = self
                    .fragments
                    .get(&fragment)
                    .ok_or(Error::MissingResource(fragment))?;
                let mut layer = Layer::new(Arc::clone(commands)).at(transform(r)?);
                layer.opacity = r.f32()?;
                if r.bool()? {
                    layer.clip = Some(Rect::new(r.f64()?, r.f64()?, r.f64()?, r.f64()?));
                }
                layer.z = r.i32()?;
                layer.visible = r.bool()?;
                if self.layers.get(key).is_none() && self.layers.len() >= MAX_LAYERS {
                    return Err(Error::TooManyLayers(MAX_LAYERS));
                }
                self.layers.set(key, layer);
            }
            MOVE => {
                let key = r.u64()?;
                let to = transform(r)?;
                // A move of a layer it does not have is a view's mistake,
                // not a reason to fail it.
                self.layers.update(key, |layer| layer.transform = to);
            }
            REMOVE => {
                self.layers.remove(r.u64()?);
            }
            DROP => {
                if let Some((_, len)) = self.fragments.remove(&id(r)?) {
                    self.fragment_bytes = self.fragment_bytes.saturating_sub(len);
                }
            }
            CLEAR => self.layers.clear(),
            ANIMATE => {
                let key = r.u64()?;
                let animations = anim::read(r)?;
                let had = self.layers.get(key).map_or(0, Layer::animation_count);
                let total = self
                    .layers
                    .animation_count()
                    .saturating_sub(had)
                    .saturating_add(animations.len());
                if total > anim::MAX_ANIMATIONS {
                    return Err(Error::TooManyAnimations(anim::MAX_ANIMATIONS));
                }
                // As a move: animating a layer it does not have is a
                // view's mistake, not a reason to fail it.
                self.layers.update(key, |layer| {
                    layer.animations = (!animations.is_empty()).then(|| Arc::from(animations));
                });
            }
            REGIONS => {
                let key = r.u64()?;
                let regions = region::read(r)?;
                let had = self.layers.get(key).map_or(0, Layer::region_count);
                let total = self
                    .layers
                    .region_count()
                    .saturating_sub(had)
                    .saturating_add(regions.len());
                if total > region::MAX_REGIONS {
                    return Err(Error::TooManyRegions(region::MAX_REGIONS));
                }
                if self.layers.get(key).is_none() {
                    // As a move: a view's mistake, not a reason to fail it.
                    return Ok(());
                }
                let old = self.layers.get(key).and_then(|l| l.regions.clone());
                region::seed(old.as_deref(), &regions, &mut self.values)?;
                self.layers.update(key, |layer| {
                    layer.regions = (!regions.is_empty()).then(|| Arc::from(regions));
                });
            }
            VALUE => {
                let name = anim::read_name(r)?;
                let value = r.f64()?;
                self.values.write(Space::View, &name, value)?;
            }
            other => return Err(Error::Unknown("operation", other)),
        }
        Ok(())
    }

    const fn within(&self) -> Result<(), Error> {
        if self.bytes() > MAX_HELD_BYTES {
            return Err(Error::TooLarge(self.bytes(), MAX_HELD_BYTES));
        }
        Ok(())
    }
}

fn id(r: &mut Reader<'_>) -> Result<Id, Error> {
    r.take(16)?.try_into().map(Id).map_err(|_| Error::Truncated)
}

fn transform(r: &mut Reader<'_>) -> Result<Affine, Error> {
    Ok(Affine::new([
        r.f64()?,
        r.f64()?,
        r.f64()?,
        r.f64()?,
        r.f64()?,
        r.f64()?,
    ]))
}

/// A resource as a [`Receiver`] would name it: what a host that shares
/// resources between panes keys them by.
#[must_use]
pub fn name_of(resource: &Resource) -> Id {
    resource.id()
}
