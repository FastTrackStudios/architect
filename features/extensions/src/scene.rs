//! Scenes in a remote view (#175): an element an installed extension
//! paints as an anyrender scene, not as DOM.
//!
//! A scene crosses the boundary as two attributes of its element, so the
//! remote-view wire needs nothing new and Dioxus's own diffing decides
//! what is sent:
//!
//! - [`SCENE_ATTR`]: the scene's content, recorded in content space, as
//!   `<hash>\n<json>` the first time, and as `<hash>` alone after that
//!   while both sides still hold it ([`SceneCache`], the last
//!   [`CACHED`] scenes, the same FIFO on each side). It changes only
//!   when the content does.
//! - [`TRANSFORM_ATTR`]: content space to the element's, `a b c d e f`
//!   (a `kurbo::Affine`'s coefficients). Panning and zooming change only
//!   this, a few dozen bytes a frame, coalesced with the rest of the
//!   frame's edits.
//!
//! The host draws the element as a widget in its own window's scene
//! (Vello through Blitz on the desk; a canvas in a browser,
//! `crate::web`). When it is laid out at a new size it tells the view
//! with a [`RESIZE_EVENT`], a form value `{"width", "height", "scale"}`
//! (CSS pixels). The view's pointer, wheel and key handlers on the
//! element get the input, in the element's coordinates, as for any
//! element (#176). A press holds the pointer.
//!
//! The JSON is a compact form of anyrender's commands: fills, strokes,
//! layers, clips and box shadows, each path as SVG, each colour as RGBA
//! bytes. A gradient crosses as its first stop's colour; text should be
//! drawn as paths for now (a glyph run carries its font's bytes).

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

pub use anyrender::recording::RenderCommand;
pub use anyrender::{PaintScene, Scene};
pub use peniko::kurbo::Affine;

/// The attribute that carries an element's scene.
pub const SCENE_ATTR: &str = "data-architect-scene";

/// The attribute that carries its transform, content to element.
pub const TRANSFORM_ATTR: &str = "data-architect-transform";

/// The attribute that carries an element's layers (`architect_scene`).
///
/// Its value is `<n>\n<base64 of a batch>`, where `n` counts the batches so each is a
/// new value. A batch says what changed (`architect_scene::ops`): a font
/// or an image once, a fragment once, a layer when it comes or changes,
/// a moved layer's transform alone. An element with layers is drawn from
/// them and its [`SCENE_ATTR`] is not read.
pub const LAYERS_ATTR: &str = "data-architect-layers";

/// A batch as [`LAYERS_ATTR`] carries it, the `n`th of its view.
#[must_use]
pub fn layers_value(n: u64, batch: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "{n:x}\n{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(batch)
    )
}

/// A [`LAYERS_ATTR`] value that names batch `n` of its view's frame
/// alone: the batch's bytes cross beside the edits (`wire::BYTES_WIRE`,
/// [`outbox`]).
#[must_use]
pub fn layers_marker(n: u64) -> String {
    format!("{n:x}")
}

/// Whether a [`LAYERS_ATTR`] value names its batch by number
/// ([`layers_marker`]) rather than carrying it: its number.
#[must_use]
pub fn layers_named(value: &str) -> Option<u64> {
    if value.contains('\n') {
        return None;
    }
    u64::from_str_radix(value, 16).ok()
}

/// The layers batches a view's thread sends as bytes beside its edits
/// (`wire::BYTES_WIRE`): on only in a view its host opened that way.
///
/// The scene component puts each batch here ([`put`](outbox::put)) and
/// sets its element's [`LAYERS_ATTR`] to the number it is given; the
/// view's writer takes them with the frame's edits
/// ([`take`](outbox::take)). The bytes are moved, never encoded or
/// copied on the way.
pub mod outbox {
    use std::cell::RefCell;

    thread_local! {
        static OUT: RefCell<Option<(u64, Vec<(u64, Vec<u8>)>)>> = const { RefCell::new(None) };
    }

    /// This thread's view sends its batches as bytes.
    pub fn enable() {
        OUT.with(|o| *o.borrow_mut() = Some((0, Vec::new())));
    }

    /// Whether this thread's view sends its batches as bytes.
    #[must_use]
    pub fn enabled() -> bool {
        OUT.with(|o| o.borrow().is_some())
    }

    /// `batch` to send with the next frame: the value its element's
    /// attribute takes, or the batch back when this thread sends none
    /// as bytes.
    ///
    /// # Errors
    ///
    /// Bytes are not on for this thread: the batch, to send as before.
    pub fn put(batch: Vec<u8>) -> Result<String, Vec<u8>> {
        OUT.with(|o| match o.borrow_mut().as_mut() {
            Some((n, out)) => {
                *n = n.wrapping_add(1);
                out.push((*n, batch));
                Ok(super::layers_marker(*n))
            }
            None => Err(batch),
        })
    }

    /// The batches put since the last take, in order.
    #[must_use]
    pub fn take() -> Vec<(u64, Vec<u8>)> {
        OUT.with(|o| {
            o.borrow_mut()
                .as_mut()
                .map(|(_, out)| std::mem::take(out))
                .unwrap_or_default()
        })
    }
}

/// The batch a [`LAYERS_ATTR`] value carries.
///
/// # Errors
///
/// It is not `<n>\n<base64>`.
pub fn layers_batch(value: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let (_, encoded) = value.split_once('\n').ok_or("layers with no batch")?;
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|e| format!("layers that are not base64: {e}"))
}

/// The event a host sends a scene's element when it is laid out at a new
/// size: a form value, `{"width": …, "height": …, "scale": …}`.
pub const RESIZE_EVENT: &str = "architect-resize";

/// The event a host sends a scene's element when a pointer did something
/// to one of its layers' hit regions (`architect_scene::region`).
///
/// A form value, `{"region": id, "kind": "enter" | "leave" | "press" |
/// "release" | "value", "axis": "x" | "y", "value": …}`. It arrives as the element's
/// `input`, as a [`RESIZE_EVENT`] does; a drag's values come at most once
/// a frame.
pub const REGION_EVENT: &str = "architect-region";

/// `event` as a [`REGION_EVENT`]'s form value.
#[must_use]
pub fn region_value(event: &architect_scene::region::RegionEvent) -> String {
    use architect_scene::region::{Axis, RegionEvent as E};
    let (kind, along, mods) = match *event {
        E::Enter(_) => ("enter", None, None),
        E::Leave(_) => ("leave", None, None),
        E::Press(_) => ("press", None, None),
        E::Release(_) => ("release", None, None),
        E::Modifiers(_, m) => ("modifiers", None, Some(m)),
        E::Value(_, axis, v) => ("value", Some((axis, v)), None),
    };
    let axis = along.map(|(a, _)| if a == Axis::X { "x" } else { "y" });
    // An id as a string: JSON's numbers lose a u64's last digits.
    serde_json::json!({
        "region": event.id().to_string(),
        "kind": kind,
        "axis": axis,
        "value": along.map(|(_, v)| v),
        "mods": mods.map(architect_scene::region::Mods::names).unwrap_or_default(),
    })
    .to_string()
}

/// The region event a [`REGION_EVENT`]'s form value says, if it is one.
#[must_use]
pub fn region_of_value(value: &str) -> Option<architect_scene::region::RegionEvent> {
    use architect_scene::region::{Axis, RegionEvent as E};
    let v: serde_json::Value = serde_json::from_str(value).ok()?;
    let id: u64 = v.get("region")?.as_str()?.parse().ok()?;
    // None from a host built before them.
    let names: Vec<&str> = v
        .get("mods")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let mods = architect_scene::region::Mods::from_names(&names);
    Some(match v.get("kind")?.as_str()? {
        "enter" => E::Enter(id),
        "leave" => E::Leave(id),
        "press" => E::Press(id),
        "release" => E::Release(id),
        // A process built before it reads past it (an unknown kind).
        "modifiers" => E::Modifiers(id, mods),
        "value" => E::Value(
            id,
            match v.get("axis")?.as_str()? {
                "x" => Axis::X,
                "y" => Axis::Y,
                _ => return None,
            },
            v.get("value")?.as_f64()?,
        ),
        _ => return None,
    })
}

/// How many scenes each side keeps by hash.
pub const CACHED: usize = 64;

/// A scene's content, shared.
pub type Commands = Arc<Vec<RenderCommand>>;

/// One command as it crosses: a path as SVG, a transform's six numbers, a
/// colour as RGBA bytes. A gradient crosses as its first stop's colour,
/// an image or a backend's own paint not at all, nor a glyph run (a
/// font's bytes don't cross: draw text as paths).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "k")]
enum Wire {
    #[serde(rename = "f")]
    Fill {
        t: [f64; 6],
        c: [u8; 4],
        p: String,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "s")]
    Stroke {
        t: [f64; 6],
        c: [u8; 4],
        p: String,
        w: f64,
        /// Join: 0 bevel, 1 miter, 2 round. Cap: 0 butt, 1 square, 2 round.
        #[serde(default)]
        j: u8,
        #[serde(default)]
        cap: u8,
    },
    #[serde(rename = "c")]
    Clip {
        t: [f64; 6],
        p: String,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "l")]
    Layer {
        t: [f64; 6],
        p: String,
        a: f32,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "o")]
    Pop,
    #[serde(rename = "b")]
    Shadow {
        t: [f64; 6],
        r: [f64; 4],
        c: [u8; 4],
        radius: f64,
        sd: f64,
    },
}

fn rgba(c: peniko::Color) -> [u8; 4] {
    c.to_rgba8().to_u8_array()
}

fn brush_rgba(p: &anyrender::Paint) -> [u8; 4] {
    match p {
        anyrender::Paint::Solid(c) => rgba(*c),
        anyrender::Paint::Gradient(g) => g
            .stops
            .first()
            .map_or([0; 4], |s| rgba(s.color.to_alpha_color())),
        _ => [0; 4],
    }
}

const fn color([r, g, b, a]: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(r, g, b, a)
}

const fn even_odd(fill: peniko::Fill) -> bool {
    matches!(fill, peniko::Fill::EvenOdd)
}

const fn fill_of(eo: bool) -> peniko::Fill {
    if eo {
        peniko::Fill::EvenOdd
    } else {
        peniko::Fill::NonZero
    }
}

fn path_of(svg: &str) -> peniko::kurbo::BezPath {
    peniko::kurbo::BezPath::from_svg(svg).unwrap_or_default()
}

fn to_wire(c: &RenderCommand) -> Option<Wire> {
    use peniko::kurbo::{Cap, Join};
    Some(match c {
        RenderCommand::Fill(f) => Wire::Fill {
            t: f.transform.as_coeffs(),
            c: brush_rgba(&f.brush),
            p: f.shape.to_svg(),
            eo: even_odd(f.fill),
        },
        RenderCommand::Stroke(s) => Wire::Stroke {
            t: s.transform.as_coeffs(),
            c: brush_rgba(&s.brush),
            p: s.shape.to_svg(),
            w: s.style.width,
            j: match s.style.join {
                Join::Bevel => 0,
                Join::Miter => 1,
                Join::Round => 2,
            },
            cap: match s.style.start_cap {
                Cap::Butt => 0,
                Cap::Square => 1,
                Cap::Round => 2,
            },
        },
        RenderCommand::PushClipLayer(l) => Wire::Clip {
            t: l.transform.as_coeffs(),
            p: l.clip.to_svg(),
            eo: even_odd(l.fill),
        },
        RenderCommand::PushLayer(l) => Wire::Layer {
            t: l.transform.as_coeffs(),
            p: l.clip.to_svg(),
            a: l.alpha,
            eo: even_odd(l.fill),
        },
        RenderCommand::PopLayer => Wire::Pop,
        RenderCommand::BoxShadow(b) => Wire::Shadow {
            t: b.transform.as_coeffs(),
            r: [b.rect.x0, b.rect.y0, b.rect.x1, b.rect.y1],
            c: rgba(b.brush),
            radius: b.radius,
            sd: b.std_dev,
        },
        RenderCommand::GlyphRun(_) => return None,
    })
}

fn from_wire(w: Wire) -> RenderCommand {
    use anyrender::recording::{
        BoxShadowCommand, ClipCommand, FillCommand, LayerCommand, StrokeCommand,
    };
    use peniko::kurbo::{Cap, Join, Rect, Stroke};
    match w {
        Wire::Fill { t, c, p, eo } => RenderCommand::Fill(FillCommand {
            fill: fill_of(eo),
            transform: Affine::new(t),
            brush: anyrender::Paint::Solid(color(c)),
            brush_transform: None,
            shape: path_of(&p),
        }),
        Wire::Stroke { t, c, p, w, j, cap } => {
            let cap = match cap {
                1 => Cap::Square,
                2 => Cap::Round,
                _ => Cap::Butt,
            };
            let join = match j {
                0 => Join::Bevel,
                2 => Join::Round,
                _ => Join::Miter,
            };
            RenderCommand::Stroke(StrokeCommand {
                style: Stroke::new(w).with_join(join).with_caps(cap),
                transform: Affine::new(t),
                brush: anyrender::Paint::Solid(color(c)),
                brush_transform: None,
                shape: path_of(&p),
            })
        }
        Wire::Clip { t, p, eo } => RenderCommand::PushClipLayer(ClipCommand {
            fill: fill_of(eo),
            transform: Affine::new(t),
            clip: path_of(&p),
        }),
        Wire::Layer { t, p, a, eo } => RenderCommand::PushLayer(LayerCommand {
            fill: fill_of(eo),
            blend: peniko::BlendMode::default(),
            alpha: a,
            transform: Affine::new(t),
            clip: path_of(&p),
            filter: None,
            backdrop_filter: None,
        }),
        Wire::Pop => RenderCommand::PopLayer,
        Wire::Shadow {
            t,
            r: [x0, y0, x1, y1],
            c,
            radius,
            sd,
        } => RenderCommand::BoxShadow(BoxShadowCommand {
            transform: Affine::new(t),
            rect: Rect::new(x0, y0, x1, y1),
            brush: color(c),
            radius,
            std_dev: sd,
        }),
    }
}

/// A scene's content as JSON, and its hash.
#[must_use]
pub fn encode(commands: &[RenderCommand]) -> (u64, String) {
    let wire: Vec<Wire> = commands.iter().filter_map(to_wire).collect();
    let json = serde_json::to_string(&wire).unwrap_or_else(|_| "[]".to_owned());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut h);
    (h.finish(), json)
}

/// A transform as [`TRANSFORM_ATTR`] carries it.
#[must_use]
pub fn transform_attr(transform: Affine) -> String {
    transform
        .as_coeffs()
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`TRANSFORM_ATTR`]'s value as a transform; the identity when it is not
/// six numbers.
#[must_use]
pub fn parse_transform(value: &str) -> Affine {
    let numbers: Vec<f64> = value
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    match <[f64; 6]>::try_from(numbers.as_slice()) {
        Ok(coeffs) if coeffs.iter().all(|v| v.is_finite()) => Affine::new(coeffs),
        _ => Affine::IDENTITY,
    }
}

/// The most bytes one [`SCENE_ATTR`] value may hold: past it the view
/// fails, not the app (a host decodes what it is sent).
pub const MAX_SCENE_BYTES: usize = 4 * 1024 * 1024;

/// The most commands one scene may hold.
pub const MAX_COMMANDS: usize = 100_000;

/// The most path elements (moves, lines, curves) one scene may hold.
pub const MAX_PATH_ELEMENTS: usize = 1_000_000;

/// The most layers and clips a scene may have open at once.
pub const MAX_DEPTH: usize = 64;

/// The largest coordinate, transform term or size a scene may use: past
/// it (or not finite) a renderer can stall or overflow.
pub const MAX_COORDINATE: f64 = 1.0e7;

/// The most bytes of scenes each side keeps ([`SceneCache`]), counted as
/// their JSON's length on both sides.
pub const CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Splits a [`SCENE_ATTR`] value into its hash and its content (empty
/// when it names a scene sent before).
///
/// # Errors
///
/// It has no hash, or its content is past [`MAX_SCENE_BYTES`].
pub fn split(value: &str) -> Result<(u64, &str), String> {
    if value.len() > MAX_SCENE_BYTES {
        return Err(format!(
            "a scene of {} bytes: past its limit of {MAX_SCENE_BYTES}",
            value.len()
        ));
    }
    let (hash, json) = value.split_once('\n').unwrap_or((value, ""));
    let hash =
        u64::from_str_radix(hash.trim(), 16).map_err(|_| "a scene with no hash".to_owned())?;
    Ok((hash, json))
}

fn finite(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

fn affine_ok(t: &[f64; 6]) -> bool {
    t.iter().all(|v| finite(*v))
}

/// A scene's content, decoded and checked.
///
/// Within the limits above, its layers balanced (a pop with none open is refused; layers left open
/// are closed), every number finite and in range. Pure, so a host can run
/// it off its UI thread.
///
/// # Errors
///
/// What is wrong with it; the view that sent it fails.
pub fn decode(json: &str) -> Result<Commands, String> {
    if json.len() > MAX_SCENE_BYTES {
        return Err(format!(
            "a scene of {} bytes: past its limit of {MAX_SCENE_BYTES}",
            json.len()
        ));
    }
    let wire: Vec<Wire> =
        serde_json::from_str(json).map_err(|e| format!("a scene that isn't one: {e}"))?;
    if wire.len() > MAX_COMMANDS {
        return Err(format!(
            "a scene of {} commands: past its limit of {MAX_COMMANDS}",
            wire.len()
        ));
    }
    let mut depth = 0_usize;
    let mut elements = 0_usize;
    let mut out = Vec::with_capacity(wire.len());
    for w in wire {
        let t = match &w {
            Wire::Fill { t, .. }
            | Wire::Stroke { t, .. }
            | Wire::Clip { t, .. }
            | Wire::Layer { t, .. } => Some(t),
            Wire::Shadow {
                t, r, radius, sd, ..
            } => {
                if !r.iter().chain([radius, sd]).all(|v| finite(*v)) {
                    return Err("a box shadow out of range".to_owned());
                }
                Some(t)
            }
            Wire::Pop => None,
        };
        if t.is_some_and(|t| !affine_ok(t)) {
            return Err("a transform out of range".to_owned());
        }
        if let Wire::Stroke { w, .. } = &w
            && !finite(*w)
        {
            return Err("a stroke width out of range".to_owned());
        }
        match &w {
            Wire::Clip { .. } | Wire::Layer { .. } => {
                depth = depth.saturating_add(1);
                if depth > MAX_DEPTH {
                    return Err(format!("layers nested past {MAX_DEPTH}"));
                }
            }
            Wire::Pop => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("a layer popped that it never pushed")?;
            }
            _ => {}
        }
        let command = from_wire(w);
        let shape = match &command {
            RenderCommand::Fill(f) => Some(&f.shape),
            RenderCommand::Stroke(s) => Some(&s.shape),
            RenderCommand::PushClipLayer(c) => Some(&c.clip),
            RenderCommand::PushLayer(l) => Some(&l.clip),
            _ => None,
        };
        if let Some(shape) = shape {
            use peniko::kurbo::{PathEl, Point};
            elements = elements.saturating_add(shape.elements().len());
            if elements > MAX_PATH_ELEMENTS {
                return Err(format!(
                    "a scene of more than {MAX_PATH_ELEMENTS} path elements"
                ));
            }
            let at = |p: &Point| finite(p.x) && finite(p.y);
            let in_range = shape.elements().iter().all(|e| match e {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => at(p),
                PathEl::QuadTo(a, b) => at(a) && at(b),
                PathEl::CurveTo(a, b, c) => at(a) && at(b) && at(c),
                PathEl::ClosePath => true,
            });
            if !in_range {
                return Err("a path out of range".to_owned());
            }
        }
        out.push(command);
    }
    // Layers left open are closed: nothing it sends reaches the host's.
    out.extend(std::iter::repeat_n(RenderCommand::PopLayer, depth));
    Ok(Arc::new(out))
}

/// The last scenes one side has sent or received, by hash.
///
/// At most [`CACHED`] of them and [`CACHE_BYTES`] of their JSON. Both sides insert
/// only full scenes, count each by its JSON's length and evict the oldest
/// first, so the sender knows what the receiver still holds.
///
/// The guest's side is per view thread (`scene_attr`), as a view's
/// document on the host is per view: a view that opens again starts both
/// afresh, together.
#[derive(Debug, Default)]
pub struct SceneCache {
    order: VecDeque<(u64, usize)>,
    held: HashMap<u64, Commands>,
    bytes: usize,
}

impl SceneCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether it holds `hash`.
    #[must_use]
    pub fn holds(&self, hash: u64) -> bool {
        self.held.contains_key(&hash)
    }

    /// How many bytes of scenes it holds, as counted.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    fn insert(&mut self, hash: u64, commands: Commands, size: usize) {
        if self.held.insert(hash, commands).is_some() {
            return;
        }
        self.order.push_back((hash, size));
        self.bytes = self.bytes.saturating_add(size);
        while self.order.len() > CACHED || (self.bytes > CACHE_BYTES && self.order.len() > 1) {
            let Some((old, size)) = self.order.pop_front() else {
                break;
            };
            self.held.remove(&old);
            self.bytes = self.bytes.saturating_sub(size);
        }
    }

    /// The sender's side: the [`SCENE_ATTR`] value for `commands`, the
    /// content only when the receiver doesn't hold it.
    pub fn send(&mut self, commands: &[RenderCommand]) -> String {
        let (hash, json) = encode(commands);
        if self.holds(hash) {
            return format!("{hash:x}");
        }
        self.insert(hash, Arc::new(Vec::new()), json.len());
        format!("{hash:x}\n{json}")
    }

    /// The receiver's side: the scene a [`SCENE_ATTR`] value names,
    /// decoded here ([`decode`]).
    ///
    /// # Errors
    ///
    /// It names a scene not held, or its content is not a scene within
    /// the limits.
    pub fn receive(&mut self, value: &str) -> Result<Commands, String> {
        self.receive_with(value, decode)
    }

    /// [`Self::receive`], its content decoded by `decoded` (a host's that
    /// decoded it already, off its UI thread).
    ///
    /// # Errors
    ///
    /// As [`Self::receive`].
    pub fn receive_with(
        &mut self,
        value: &str,
        decoded: impl FnOnce(&str) -> Result<Commands, String>,
    ) -> Result<Commands, String> {
        let (hash, json) = split(value)?;
        if json.is_empty() {
            return self
                .held
                .get(&hash)
                .cloned()
                .ok_or_else(|| format!("a scene {hash:x} it was never sent, or no longer holds"));
        }
        let commands = decoded(json)?;
        self.insert(hash, Arc::clone(&commands), json.len());
        Ok(commands)
    }
}

/// Every scene with content in `edits`, decoded ([`decode`]): what a host
/// does off its UI thread before it applies them.
#[must_use]
pub fn predecode(edits: &[crate::wire::Edit]) -> Vec<(u64, Result<Commands, String>)> {
    edits
        .iter()
        .filter_map(|e| match e {
            crate::wire::Edit::SetAttribute {
                name,
                value: crate::wire::Value::Text(v),
                ..
            } if name == SCENE_ATTR => split(v)
                .ok()
                .filter(|(_, json)| !json.is_empty())
                .map(|(hash, json)| (hash, decode(json))),
            _ => None,
        })
        .collect()
}

/// `commands` drawn at `transform` into a scene.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
#[must_use]
pub fn scene_of(commands: &[RenderCommand], transform: Affine) -> Scene {
    let mut scene = Scene::new();
    scene.append_scene(
        Scene {
            commands: commands.to_vec(),
            ..Scene::new()
        },
        transform,
    );
    scene
}

/// What an element's widget draws.
///
/// `commands` at `transform`, scaled by `scale`, in a box `width` × `height` (device pixels). Clipped to the
/// box unless the extension may draw over the whole app (`overlay`,
/// `capabilities.ui`), so a view can't smear over the app by mistake.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
#[must_use]
pub fn framed(
    commands: &[RenderCommand],
    transform: Affine,
    scale: f64,
    (width, height): (u32, u32),
    overlay: bool,
) -> Scene {
    if overlay {
        return scene_of(commands, Affine::scale(scale) * transform);
    }
    let mut scene = Scene::new();
    scene.push_clip_layer(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        &peniko::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
    );
    scene.append_scene(
        Scene {
            commands: commands.to_vec(),
            ..Scene::new()
        },
        Affine::scale(scale) * transform,
    );
    scene.pop_layer();
    scene
}

#[cfg(feature = "remote")]
impl Painted {
    /// Draws it into `scene` at `at`: its layers, placed by their
    /// animations at the clocks now, or its one scene.
    // kurbo's transforms compose with `*`: no overflow to guard.
    #[allow(clippy::arithmetic_side_effects)]
    pub(crate) fn draw(&mut self, scene: &mut Scene, at: Affine) {
        match &self.layers {
            Some(_) => {
                let now = self.clocks.read();
                // A flicked value coasts on: the host's, as a drag is.
                if let Some(receiver) = self.layers.as_mut() {
                    self.interaction.coast(&now, receiver.parts_mut().1);
                }
                let Some(layers) = &self.layers else { return };
                let lit = self.interaction.lit();
                layers
                    .layers()
                    .compose_lit(scene, at * self.transform, &now, layers.values(), lit);
                self.drawn = now;
                self.drawn_lit = lit;
                self.drawn_values = layers.values().revision();
                if let Some(alarm) = &self.alarm {
                    let _ = alarm.drawn(layers.layers().next(&now));
                }
            }
            None => scene.append_shared(&self.commands, at * self.transform),
        }
    }

    /// A pointer event at `at` (the element's CSS pixels) for its
    /// layers' regions: what it did ([`architect_scene::region::Outcome`]).
    // kurbo's transforms compose with `*`: no overflow to guard.
    #[allow(clippy::arithmetic_side_effects)]
    pub(crate) fn pointer(
        &mut self,
        kind: PointerKind,
        at: peniko::kurbo::Point,
        mods: architect_scene::region::Mods,
    ) -> architect_scene::region::Outcome {
        let Some(receiver) = self.layers.as_mut() else {
            return architect_scene::region::Outcome::default();
        };
        if self.transform.determinant().abs() < 1e-12 {
            return architect_scene::region::Outcome::default();
        }
        let at = self.transform.inverse() * at;
        let now = self.clocks.read();
        let (layers, values) = receiver.parts_mut();
        let i = &mut self.interaction;
        match kind {
            PointerKind::Move => i.moved(layers, &now, values, at),
            PointerKind::Down => i.pressed_with(layers, &now, values, at, mods),
            PointerKind::Up => i.released_with(layers, &now, values, at, mods),
            PointerKind::Gone => i.left(),
        }
    }

    /// Whether a layer moves on a running clock, or the engine's values
    /// it reads moved since it was drawn: drawn every frame.
    pub(crate) fn moving(&self) -> bool {
        self.layers.as_ref().is_some_and(|l| {
            l.layers().moving(&self.clocks.read())
                || self.interaction.coasting()
                || (l.values().engine().is_some() && l.values().revision() != self.drawn_values)
        })
    }

    /// Whether a frame now would draw something else than the last one:
    /// a batch came, a layer moves, or a clock one reads was moved.
    pub(crate) fn stale(&self) -> bool {
        self.dirty
            || self.interaction.pending()
            || self.interaction.lit() != self.drawn_lit
            || self
                .layers
                .as_ref()
                .is_some_and(|l| l.values().revision() != self.drawn_values)
            || self
                .layers
                .as_ref()
                .is_some_and(|l| l.layers().stale(&self.drawn, &self.clocks.read()))
    }
}

/// What a pointer did, for a scene's regions.
#[cfg(feature = "remote")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerKind {
    Move,
    Down,
    Up,
    /// It left the element, or its press was lost.
    Gone,
}

/// An element's scene as its widget and its document share it: updated
/// in place, so a new transform keeps the size the widget last told.
#[cfg(feature = "remote")]
#[derive(Debug)]
pub(crate) struct Painted {
    pub(crate) commands: Commands,
    /// Its layers, once its view sent any ([`LAYERS_ATTR`]): drawn in
    /// place of `commands`.
    pub(crate) layers: Option<architect_scene::ops::Receiver>,
    /// What its layers' animations read: the app's clocks, the same in
    /// every window.
    pub(crate) clocks: architect_scene::Clocks,
    /// The clocks it was last drawn at.
    pub(crate) drawn: architect_scene::Readings,
    /// Wakes its window at its layers' next step and when a clock jumps.
    pub(crate) alarm: Option<Arc<architect_scene::wake::Alarm>>,
    /// The pointer over its layers' regions.
    pub(crate) interaction: architect_scene::region::Interaction,
    /// What it highlighted, and its values' revision, when last drawn.
    pub(crate) drawn_lit: Option<architect_scene::region::Lit>,
    pub(crate) drawn_values: u64,
    pub(crate) transform: Affine,
    pub(crate) element: u64,
    pub(crate) dirty: bool,
    /// Where its element's top left is in the window, CSS pixels: where
    /// an [`OverlayLayer`] draws it.
    pub(crate) origin: (f64, f64),
}

/// What draws a view's scenes over the whole app.
///
/// For an extension granted `overlay`: every scene of the view, at its element's place in the
/// window, unclipped. Its widget ([`OverlayWidget`]) fills the window in
/// the host's own document, above the panes, and takes no input; the
/// scene's element in the pane still has its size and its input.
#[cfg(feature = "remote")]
#[derive(Clone, Default)]
pub struct OverlayLayer {
    pub(crate) scenes: Arc<std::sync::Mutex<Vec<Arc<std::sync::Mutex<Painted>>>>>,
    pub(crate) version: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "remote")]
impl OverlayLayer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Something it draws changed.
    pub(crate) fn touch(&self) {
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn add(&self, painted: Arc<std::sync::Mutex<Painted>>) {
        self.scenes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(painted);
        self.touch();
    }

    pub(crate) fn remove(&self, painted: &Arc<std::sync::Mutex<Painted>>) {
        self.scenes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|p| !Arc::ptr_eq(p, painted));
        self.touch();
    }

    /// Its widget, for an element filling the window.
    #[must_use]
    pub fn widget(&self) -> OverlayWidget {
        OverlayWidget {
            layer: self.clone(),
            drawn: u64::MAX,
        }
    }
}

/// An [`OverlayLayer`] as a Blitz widget over the whole window.
#[cfg(feature = "remote")]
pub struct OverlayWidget {
    layer: OverlayLayer,
    drawn: u64,
}

#[cfg(feature = "remote")]
impl OverlayWidget {
    /// Whether `test` holds of any scene it draws.
    fn any(&self, test: impl Fn(&Painted) -> bool) -> bool {
        self.layer
            .scenes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|p| test(&p.lock().unwrap_or_else(std::sync::PoisonError::into_inner)))
    }
}

#[cfg(feature = "remote")]
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
impl blitz_dom::Widget for OverlayWidget {
    fn needs_redraw(&self) -> bool {
        self.layer
            .version
            .load(std::sync::atomic::Ordering::Relaxed)
            != self.drawn
            || self.any(Painted::stale)
    }

    fn requires_redraw(&self) -> bool {
        self.any(Painted::moving)
    }

    fn paint(
        &mut self,
        _: &mut dyn anyrender::RenderContext,
        _: &blitz_dom::node::ComputedStyles,
        _: u32,
        _: u32,
        scale: f64,
    ) -> Scene {
        self.drawn = self
            .layer
            .version
            .load(std::sync::atomic::Ordering::Relaxed);
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let mut scene = Scene::new();
        let scenes = self
            .layer
            .scenes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for painted in scenes {
            let mut p = painted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = Affine::scale(scale) * Affine::translate(p.origin);
            p.draw(&mut scene, at);
        }
        scene
    }
}

/// The host's widget: a scene's content at its transform, scaled to the
/// display, telling the view when its size changes.
#[cfg(feature = "remote")]
pub(crate) struct SceneWidget {
    pub(crate) painted: Arc<std::sync::Mutex<Painted>>,
    pub(crate) overlay: bool,
    pub(crate) events: futures::channel::mpsc::UnboundedSender<crate::wire::ViewEvent>,
    pub(crate) resizes: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) size: Option<(u32, u32)>,
}

#[cfg(feature = "remote")]
impl blitz_dom::Widget for SceneWidget {
    // Drawn again when its view sent something, and every frame its
    // layers move on their own: the view is not asked for those.
    fn needs_redraw(&self) -> bool {
        self.painted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stale()
    }

    fn requires_redraw(&self) -> bool {
        self.painted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .moving()
    }

    fn paint(
        &mut self,
        _: &mut dyn anyrender::RenderContext,
        _: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let (drawn, element) = {
            let mut p = self
                .painted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            p.dirty = false;
            // Drawn over the whole app by its view's `OverlayLayer`.
            let mut scene = Scene::new();
            if !self.overlay {
                scene.push_clip_layer(
                    peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    &peniko::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                );
                p.draw(&mut scene, Affine::scale(scale));
                scene.pop_layer();
            }
            // What its regions heard reaches the view once a frame.
            for event in p.interaction.take() {
                let _ = self.events.unbounded_send(crate::wire::ViewEvent {
                    element: p.element,
                    name: REGION_EVENT.to_owned(),
                    bubbles: false,
                    data: crate::wire::EventData::Form(region_value(&event)),
                });
            }
            (scene, p.element)
        };
        if self.size != Some((width, height)) {
            self.size = Some((width, height));
            self.resizes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let size = serde_json::json!({
                "width": f64::from(width) / scale,
                "height": f64::from(height) / scale,
                "scale": scale,
            });
            let _ = self.events.unbounded_send(crate::wire::ViewEvent {
                element,
                name: RESIZE_EVENT.to_owned(),
                bubbles: false,
                data: crate::wire::EventData::Form(size.to_string()),
            });
        }
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::kurbo::{Rect, Shape as _};
    use peniko::{Color, Fill};

    fn square(x: f64) -> Vec<RenderCommand> {
        let mut s = Scene::new();
        s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(1, 2, 3),
            None,
            &Rect::new(x, 0.0, x + 10.0, 10.0),
        );
        s.commands
    }

    #[test]
    fn a_scene_crosses_once_then_by_its_hash() {
        let (mut sent, mut got) = (SceneCache::new(), SceneCache::new());
        let first = sent.send(&square(0.0));
        assert!(first.contains('\n'));
        let a = got.receive(&first).unwrap();
        let again = sent.send(&square(0.0));
        assert!(!again.contains('\n'), "the same scene is its hash: {again}");
        assert_eq!(got.receive(&again).unwrap(), a);
        assert_eq!(*a, square(0.0));
    }

    #[test]
    fn both_sides_forget_the_same_scenes() {
        let (mut sent, mut got) = (SceneCache::new(), SceneCache::new());
        for i in 0..=u32::try_from(CACHED).unwrap_or(u32::MAX) {
            let v = sent.send(&square(f64::from(i)));
            got.receive(&v).unwrap();
        }
        // The first is gone on both sides: it is sent whole again.
        let v = sent.send(&square(0.0));
        assert!(v.contains('\n'));
        got.receive(&v).unwrap();
        assert!(got.receive("deadbeef").is_err());
    }

    #[test]
    fn a_transform_round_trips_and_nonsense_is_the_identity() {
        let t = Affine::translate((3.5, -2.0)) * Affine::scale(1.25);
        assert_eq!(parse_transform(&transform_attr(t)), t);
        assert_eq!(parse_transform("1 2 x"), Affine::IDENTITY);
        assert_eq!(parse_transform("1 0 0 1 NaN 0"), Affine::IDENTITY);
    }

    fn json(wire: &str) -> String {
        format!("[{wire}]")
    }

    const SQUARE: &str = r#"{"k":"f","t":[1,0,0,1,0,0],"c":[1,2,3,255],"p":"M0 0L10 0L10 10Z"}"#;
    const CLIP: &str = r#"{"k":"c","t":[1,0,0,1,0,0],"p":"M0 0L5 0L5 5Z"}"#;
    const POP: &str = r#"{"k":"o"}"#;

    #[test]
    fn a_pop_with_no_layer_open_is_refused_and_open_layers_are_closed() {
        let stray = decode(&json(&format!("{SQUARE},{POP}"))).unwrap_err();
        assert!(stray.contains("popped"), "{stray}");
        let open = decode(&json(&format!("{CLIP},{CLIP},{SQUARE}"))).unwrap();
        let pops = open
            .iter()
            .filter(|c| matches!(c, RenderCommand::PopLayer))
            .count();
        assert_eq!(pops, 2, "two layers left open are closed");
        let deep: Vec<&str> = std::iter::repeat_n(CLIP, MAX_DEPTH + 1).collect();
        let too_deep = decode(&json(&deep.join(","))).unwrap_err();
        assert!(too_deep.contains("nested"), "{too_deep}");
    }

    #[test]
    fn a_scene_past_its_limits_or_out_of_range_is_refused() {
        let huge = format!("ab\n{}", " ".repeat(MAX_SCENE_BYTES + 1));
        assert!(split(&huge).unwrap_err().contains("past its limit"));
        let many: Vec<&str> = std::iter::repeat_n(POP, MAX_COMMANDS + 1).collect();
        assert!(
            decode(&json(&many.join(",")))
                .unwrap_err()
                .contains("commands")
        );
        let far = r#"{"k":"f","t":[1,0,0,1,0,0],"c":[1,2,3,255],"p":"M0 0L99999999999 0L0 10Z"}"#;
        assert!(decode(&json(far)).unwrap_err().contains("path"));
        let shifted = r#"{"k":"f","t":[1,0,0,1,-1e300,0],"c":[1,2,3,255],"p":"M0 0L1 0Z"}"#;
        assert!(decode(&json(shifted)).unwrap_err().contains("transform"));
    }

    #[test]
    fn both_sides_forget_by_bytes_alike() {
        let (mut sent, mut got) = (SceneCache::new(), SceneCache::new());
        // Ten scenes of about 3.5 MB: past the byte budget, under the
        // count.
        let big = |n: u32| -> Vec<RenderCommand> {
            let mut s = Scene::new();
            let mut path = peniko::kurbo::BezPath::new();
            path.move_to((f64::from(n), 0.0));
            for i in 0..200_000_u32 {
                path.line_to((f64::from(i) * 1.000_1, 1.5));
            }
            s.fill(Fill::NonZero, Affine::IDENTITY, Color::BLACK, None, &path);
            s.commands
        };
        for n in 0..10 {
            let v = sent.send(&big(n));
            assert!(v.len() <= MAX_SCENE_BYTES, "{}", v.len());
            let (_, j) = split(&v).unwrap();
            // Decoding isn't the point here; the receiver keeps its size.
            got.receive_with(&v, |_| Ok(Arc::new(Vec::new()))).unwrap();
            assert!(!j.is_empty());
        }
        assert!(sent.bytes() <= CACHE_BYTES && got.bytes() <= CACHE_BYTES);
        assert_eq!(sent.bytes(), got.bytes());
        // The first went on both sides: it is sent whole again.
        assert!(sent.send(&big(0)).contains('\n'));
    }

    #[test]
    fn a_scene_is_clipped_to_its_box_unless_it_may_draw_over_the_app() {
        let clipped = framed(&square(0.0), Affine::IDENTITY, 2.0, (40, 30), false);
        assert!(matches!(
            clipped.commands.first(),
            Some(RenderCommand::PushClipLayer(c)) if c.clip.bounding_box() == Rect::new(0.0, 0.0, 40.0, 30.0)
        ));
        assert!(matches!(
            clipped.commands.last(),
            Some(RenderCommand::PopLayer)
        ));
        let over = framed(&square(0.0), Affine::IDENTITY, 2.0, (40, 30), true);
        assert!(
            !over
                .commands
                .iter()
                .any(|c| matches!(c, RenderCommand::PushClipLayer(_)))
        );
    }
}
