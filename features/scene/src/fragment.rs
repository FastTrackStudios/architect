//! Fragments: a list of drawing commands as bytes.
//!
//! [`Encoder::encode`] writes anyrender's commands; [`decode`] reads them
//! back, checked. A fragment starts with [`MAGIC`] and a count, then each
//! command as a byte that says which and what it needs. Numbers are
//! little-endian. A transform is written as what it is (the identity, a
//! translation, or all six terms), a path as a rectangle when it is one,
//! and a path's coordinates as `f32` when that loses nothing, else `f64`.

use std::collections::HashMap;

use anyrender::recording::{
    BoxShadowCommand, ClipCommand, FillCommand, GlyphRunCommand, LayerCommand, RenderCommand,
    StrokeCommand,
};
use anyrender::{Glyph, Paint};
use peniko::color::{ColorSpaceTag, DynamicColor, HueDirection, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, PathEl, Point, Rect, Stroke, Vec2};
use peniko::{
    BlendMode, Color, ColorStop, ColorStops, Compose, Extend, Fill, FontData, Gradient,
    GradientKind, ImageBrush, ImageQuality, ImageSampler, InterpolationAlphaSpace,
    LinearGradientPosition, Mix, RadialGradientPosition, Style, SweepGradientPosition,
};

use crate::bytes::{Reader, Writer, narrow};
use crate::resource::{Resource, Resources};
use crate::{
    Error, Id, MAX_COMMANDS, MAX_DEPTH, MAX_FRAGMENT_BYTES, MAX_GLYPHS, MAX_PATH_ELEMENTS,
};

/// How a fragment starts: its format and that format's version.
pub const MAGIC: [u8; 4] = *b"ASF\x01";

const FILL: u8 = 0;
const STROKE: u8 = 1;
const CLIP: u8 = 2;
const LAYER: u8 = 3;
const POP: u8 = 4;
const SHADOW: u8 = 5;
const GLYPHS: u8 = 6;

const ALL_MIX: [Mix; 16] = [
    Mix::Normal,
    Mix::Multiply,
    Mix::Screen,
    Mix::Overlay,
    Mix::Darken,
    Mix::Lighten,
    Mix::ColorDodge,
    Mix::ColorBurn,
    Mix::HardLight,
    Mix::SoftLight,
    Mix::Difference,
    Mix::Exclusion,
    Mix::Hue,
    Mix::Saturation,
    Mix::Color,
    Mix::Luminosity,
];

const ALL_COMPOSE: [Compose; 14] = [
    Compose::Clear,
    Compose::Copy,
    Compose::Dest,
    Compose::SrcOver,
    Compose::DestOver,
    Compose::SrcIn,
    Compose::DestIn,
    Compose::SrcOut,
    Compose::DestOut,
    Compose::SrcAtop,
    Compose::DestAtop,
    Compose::Xor,
    Compose::Plus,
    Compose::PlusLighter,
];

const ALL_SPACES: [ColorSpaceTag; 16] = [
    ColorSpaceTag::Srgb,
    ColorSpaceTag::LinearSrgb,
    ColorSpaceTag::Lab,
    ColorSpaceTag::Lch,
    ColorSpaceTag::Hsl,
    ColorSpaceTag::Hwb,
    ColorSpaceTag::Oklab,
    ColorSpaceTag::Oklch,
    ColorSpaceTag::DisplayP3,
    ColorSpaceTag::A98Rgb,
    ColorSpaceTag::ProphotoRgb,
    ColorSpaceTag::Rec2020,
    ColorSpaceTag::AcesCg,
    ColorSpaceTag::XyzD50,
    ColorSpaceTag::XyzD65,
    ColorSpaceTag::Aces2065_1,
];

const ALL_HUES: [HueDirection; 4] = [
    HueDirection::Shorter,
    HueDirection::Longer,
    HueDirection::Increasing,
    HueDirection::Decreasing,
];

/// A fieldless `repr(u8)` enum's value as its byte.
macro_rules! tag {
    ($v:expr) => {{
        #[allow(clippy::as_conversions)]
        let tag = $v as u8;
        tag
    }};
}

/// The one of `all` whose byte is `tag`.
fn one_of<T: Copy>(
    all: &[T],
    tag: u8,
    byte: impl Fn(T) -> u8,
    what: &'static str,
) -> Result<T, Error> {
    all.iter()
        .copied()
        .find(|v| byte(*v) == tag)
        .ok_or(Error::Unknown(what, tag))
}

/// What a fragment could not carry, counted: it was drawn without them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Layers whose filter or backdrop filter was left off.
    pub filters: u32,
    /// Brushes that were a backend's own paint, drawn transparent.
    pub paints: u32,
}

/// A fragment, ready to cross.
#[derive(Debug, Clone)]
pub struct Encoded {
    /// Its name: the hash of [`Self::bytes`].
    pub id: Id,
    pub bytes: Vec<u8>,
    /// The resources it draws, by name: what its receiver has to hold
    /// before it can read it.
    pub resources: Vec<(Id, Resource)>,
    pub dropped: Dropped,
}

/// What an [`Encoder`] remembers a resource by: its blob, and for an
/// image its size and format.
type Seen = (u8, u64, u32, u32, u8, u8);

/// How many resources an [`Encoder`] remembers the names of.
const REMEMBERED: usize = 1024;

/// Writes fragments.
///
/// One per sender: it remembers the name of each font and image it has
/// hashed, by the blob they were drawn from, so a font's file is hashed
/// once and not for every frame that draws with it.
#[derive(Debug, Default)]
pub struct Encoder {
    named: HashMap<Seen, Id>,
}

struct Pass<'e> {
    w: Writer,
    named: &'e mut HashMap<Seen, Id>,
    resources: Vec<(Id, Resource)>,
    dropped: Dropped,
}

impl Encoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `commands` as a fragment.
    pub fn encode(&mut self, commands: &[RenderCommand]) -> Encoded {
        let mut pass = Pass {
            w: Writer::default(),
            named: &mut self.named,
            resources: Vec::new(),
            dropped: Dropped::default(),
        };
        pass.w.bytes(&MAGIC);
        pass.w.count(commands.len());
        for command in commands {
            pass.command(command);
        }
        let bytes = pass.w.out;
        Encoded {
            id: Id::of(&[&bytes]),
            bytes,
            resources: pass.resources,
            dropped: pass.dropped,
        }
    }
}

impl Pass<'_> {
    fn command(&mut self, command: &RenderCommand) {
        match command {
            RenderCommand::Fill(f) => {
                self.w.u8(FILL);
                self.w.u8(tag!(f.fill));
                self.transform(f.transform);
                self.brush(&f.brush);
                self.optional_transform(f.brush_transform);
                self.path(&f.shape);
            }
            RenderCommand::Stroke(s) => {
                self.w.u8(STROKE);
                self.stroke(&s.style);
                self.transform(s.transform);
                self.brush(&s.brush);
                self.optional_transform(s.brush_transform);
                self.path(&s.shape);
            }
            RenderCommand::PushClipLayer(c) => {
                self.w.u8(CLIP);
                self.w.u8(tag!(c.fill));
                self.transform(c.transform);
                self.path(&c.clip);
            }
            RenderCommand::PushLayer(l) => {
                self.w.u8(LAYER);
                self.w.u8(tag!(l.fill));
                self.w.u8(tag!(l.blend.mix));
                self.w.u8(tag!(l.blend.compose));
                self.w.f32(l.alpha);
                self.transform(l.transform);
                self.path(&l.clip);
                let filters = u32::from(l.filter.is_some())
                    .saturating_add(u32::from(l.backdrop_filter.is_some()));
                self.dropped.filters = self.dropped.filters.saturating_add(filters);
            }
            RenderCommand::PopLayer => self.w.u8(POP),
            RenderCommand::BoxShadow(b) => {
                self.w.u8(SHADOW);
                self.transform(b.transform);
                for v in [b.rect.x0, b.rect.y0, b.rect.x1, b.rect.y1] {
                    self.w.f64(v);
                }
                self.color(b.brush);
                self.w.f64(b.radius);
                self.w.f64(b.std_dev);
            }
            RenderCommand::GlyphRun(g) => self.glyphs(g),
        }
    }

    fn glyphs(&mut self, g: &GlyphRunCommand) {
        self.w.u8(GLYPHS);
        let font = self.resource(
            (crate::resource::FONT, g.font_data.data.id(), 0, 0, 0, 0),
            || Resource::Font(g.font_data.data.clone()),
        );
        self.w.bytes(&font.0);
        self.w.u32(g.font_data.index);
        self.w.f32(g.font_size);
        self.w.bool(g.hint);
        self.w
            .u16(u16::try_from(g.normalized_coords.len()).unwrap_or(u16::MAX));
        for coord in g.normalized_coords.iter().take(usize::from(u16::MAX)) {
            self.w.i16(*coord);
        }
        self.w.f64(g.embolden.x);
        self.w.f64(g.embolden.y);
        match &g.style {
            Style::Fill(fill) => {
                self.w.u8(0);
                self.w.u8(tag!(*fill));
            }
            Style::Stroke(stroke) => {
                self.w.u8(1);
                self.stroke(stroke);
            }
        }
        self.brush(&g.brush);
        self.w.f32(g.brush_alpha);
        self.transform(g.transform);
        self.optional_transform(g.glyph_transform);
        self.w.count(g.glyphs.len());
        for glyph in &g.glyphs {
            self.w.u32(glyph.id);
            self.w.f32(glyph.x);
            self.w.f32(glyph.y);
        }
    }

    /// The name of the resource seen as `seen`, hashed the first time.
    fn resource(&mut self, seen: Seen, make: impl FnOnce() -> Resource) -> Id {
        if let Some(id) = self.named.get(&seen).copied() {
            if !self.resources.iter().any(|(held, _)| *held == id) {
                self.resources.push((id, make()));
            }
            return id;
        }
        let resource = make();
        let id = resource.id();
        if self.named.len() >= REMEMBERED {
            self.named.clear();
        }
        self.named.insert(seen, id);
        if !self.resources.iter().any(|(held, _)| *held == id) {
            self.resources.push((id, resource));
        }
        id
    }

    #[allow(clippy::many_single_char_names)]
    fn transform(&mut self, t: Affine) {
        let [a, b, c, d, e, f] = t.as_coeffs();
        #[allow(clippy::float_cmp)]
        let translation = a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0;
        #[allow(clippy::float_cmp)]
        let identity = translation && e == 0.0 && f == 0.0;
        if identity {
            self.w.u8(0);
        } else if translation {
            self.w.u8(1);
            self.w.f64(e);
            self.w.f64(f);
        } else {
            self.w.u8(2);
            for v in [a, b, c, d, e, f] {
                self.w.f64(v);
            }
        }
    }

    fn optional_transform(&mut self, t: Option<Affine>) {
        match t {
            None => self.w.u8(3),
            Some(t) => self.transform(t),
        }
    }

    #[allow(clippy::many_single_char_names)]
    fn color(&mut self, c: Color) {
        let bytes = c.to_rgba8().to_u8_array();
        let [r, g, b, a] = bytes;
        #[allow(clippy::float_cmp)]
        let exact = Color::from_rgba8(r, g, b, a).components == c.components;
        if exact {
            self.w.u8(0);
            self.w.bytes(&bytes);
        } else {
            self.w.u8(1);
            for v in c.components {
                self.w.f32(v);
            }
        }
    }

    fn brush(&mut self, brush: &Paint) {
        match brush {
            Paint::Solid(c) => self.color(*c),
            Paint::Gradient(g) => {
                self.w.u8(2);
                self.gradient(g);
            }
            Paint::Image(image) => {
                self.w.u8(3);
                let data = &image.image;
                let seen = (
                    crate::resource::IMAGE,
                    data.data.id(),
                    data.width,
                    data.height,
                    tag!(data.format),
                    tag!(data.alpha_type),
                );
                let id = self.resource(seen, || Resource::Image(data.clone()));
                self.w.bytes(&id.0);
                self.w.u8(tag!(image.sampler.x_extend));
                self.w.u8(tag!(image.sampler.y_extend));
                self.w.u8(tag!(image.sampler.quality));
                self.w.f32(image.sampler.alpha);
            }
            // A renderer's own paint has no bytes to send.
            Paint::Resource(_) | Paint::Custom(_) => {
                self.dropped.paints = self.dropped.paints.saturating_add(1);
                self.color(Color::TRANSPARENT);
            }
        }
    }

    fn gradient(&mut self, g: &Gradient) {
        match g.kind {
            GradientKind::Linear(p) => {
                self.w.u8(0);
                for v in [p.start.x, p.start.y, p.end.x, p.end.y] {
                    self.w.f64(v);
                }
            }
            GradientKind::Radial(p) => {
                self.w.u8(1);
                self.w.f64(p.start_center.x);
                self.w.f64(p.start_center.y);
                self.w.f32(p.start_radius);
                self.w.f64(p.end_center.x);
                self.w.f64(p.end_center.y);
                self.w.f32(p.end_radius);
            }
            GradientKind::Sweep(p) => {
                self.w.u8(2);
                self.w.f64(p.center.x);
                self.w.f64(p.center.y);
                self.w.f32(p.start_angle);
                self.w.f32(p.end_angle);
            }
        }
        self.w.u8(tag!(g.extend));
        self.w.u8(tag!(g.interpolation_cs));
        self.w.u8(tag!(g.hue_direction));
        self.w.u8(tag!(g.interpolation_alpha_space));
        self.w.u16(u16::try_from(g.stops.len()).unwrap_or(u16::MAX));
        for stop in g.stops.iter().take(usize::from(u16::MAX)) {
            self.w.f32(stop.offset);
            // A stop crosses as sRGB: what it is drawn in.
            for v in stop.color.to_alpha_color::<Srgb>().components {
                self.w.f32(v);
            }
        }
    }

    fn stroke(&mut self, s: &Stroke) {
        self.w.f64(s.width);
        self.w.u8(join_tag(s.join));
        self.w.f64(s.miter_limit);
        self.w.u8(cap_tag(s.start_cap));
        self.w.u8(cap_tag(s.end_cap));
        self.w.f64(s.dash_offset);
        self.w
            .u16(u16::try_from(s.dash_pattern.len()).unwrap_or(u16::MAX));
        for dash in s.dash_pattern.iter().take(usize::from(u16::MAX)) {
            self.w.f64(*dash);
        }
    }

    fn path(&mut self, path: &BezPath) {
        let elements = path.elements();
        if let Some(rect) = as_rect(elements) {
            let sides = [rect.x0, rect.y0, rect.x1, rect.y1];
            if let [Some(a), Some(b), Some(c), Some(d)] = sides.map(narrow) {
                self.w.u8(0);
                for v in [a, b, c, d] {
                    self.w.f32(v);
                }
            } else {
                self.w.u8(1);
                for v in sides {
                    self.w.f64(v);
                }
            }
            return;
        }
        let wide = elements.iter().any(|e| {
            points(e)
                .iter()
                .any(|p| narrow(p.x).is_none() || narrow(p.y).is_none())
        });
        self.w.u8(if wide { 3 } else { 2 });
        self.w.count(elements.len());
        for e in elements {
            self.w.u8(match e {
                PathEl::MoveTo(_) => 0,
                PathEl::LineTo(_) => 1,
                PathEl::QuadTo(..) => 2,
                PathEl::CurveTo(..) => 3,
                PathEl::ClosePath => 4,
            });
        }
        for e in elements {
            for p in points(e) {
                for v in [p.x, p.y] {
                    match narrow(v) {
                        Some(n) if !wide => self.w.f32(n),
                        _ => self.w.f64(v),
                    }
                }
            }
        }
    }
}

/// An element's points, in order.
fn points(e: &PathEl) -> Vec<Point> {
    match e {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![*p],
        PathEl::QuadTo(a, b) => vec![*a, *b],
        PathEl::CurveTo(a, b, c) => vec![*a, *b, *c],
        PathEl::ClosePath => Vec::new(),
    }
}

/// The rectangle `elements` are, when they are one as kurbo writes it.
#[allow(clippy::float_cmp)]
fn as_rect(elements: &[PathEl]) -> Option<Rect> {
    match elements {
        [
            PathEl::MoveTo(a),
            PathEl::LineTo(b),
            PathEl::LineTo(c),
            PathEl::LineTo(d),
            PathEl::ClosePath,
        ] if b.y == a.y && c.x == b.x && d.y == c.y && d.x == a.x => {
            Some(Rect::new(a.x, a.y, c.x, c.y))
        }
        _ => None,
    }
}

/// A rectangle as the path [`as_rect`] reads.
fn rect_path(r: Rect) -> BezPath {
    BezPath::from_vec(vec![
        PathEl::MoveTo(Point::new(r.x0, r.y0)),
        PathEl::LineTo(Point::new(r.x1, r.y0)),
        PathEl::LineTo(Point::new(r.x1, r.y1)),
        PathEl::LineTo(Point::new(r.x0, r.y1)),
        PathEl::ClosePath,
    ])
}

const fn join_tag(join: Join) -> u8 {
    match join {
        Join::Bevel => 0,
        Join::Miter => 1,
        Join::Round => 2,
    }
}

const fn cap_tag(cap: Cap) -> u8 {
    match cap {
        Cap::Butt => 0,
        Cap::Square => 1,
        Cap::Round => 2,
    }
}

/// What a decode has counted against the limits.
#[derive(Default)]
struct Counts {
    elements: usize,
    glyphs: usize,
    depth: usize,
}

/// A fragment's commands, read and checked.
///
/// Within the limits, every number finite and in range, its layers
/// balanced (a pop with none open is refused; layers left open are
/// closed), every font and image it names held by `resources`. Pure, so a
/// host can run it off its UI thread.
///
/// # Errors
///
/// What is wrong with it; the view that sent it fails.
pub fn decode(bytes: &[u8], resources: &Resources) -> Result<Vec<RenderCommand>, Error> {
    if bytes.len() > MAX_FRAGMENT_BYTES {
        return Err(Error::TooLarge(bytes.len(), MAX_FRAGMENT_BYTES));
    }
    let mut r = Reader::new(bytes);
    if r.take(MAGIC.len()).ok() != Some(&MAGIC) {
        return Err(Error::NotAFragment);
    }
    let count = r.count()?;
    if count > MAX_COMMANDS {
        return Err(Error::TooManyCommands(MAX_COMMANDS));
    }
    let mut counts = Counts::default();
    // A command is at least a byte: a count past what is left is a lie,
    // and nothing is reserved for it.
    let mut out = Vec::with_capacity(count.min(r.left()));
    for _ in 0..count {
        out.push(command(&mut r, resources, &mut counts)?);
    }
    if r.left() > 0 {
        return Err(Error::Trailing(r.left()));
    }
    // Layers left open are closed: nothing it sends reaches the host's.
    out.extend(std::iter::repeat_n(RenderCommand::PopLayer, counts.depth));
    Ok(out)
}

fn command(
    r: &mut Reader<'_>,
    resources: &Resources,
    counts: &mut Counts,
) -> Result<RenderCommand, Error> {
    Ok(match r.u8()? {
        FILL => {
            let fill = fill(r)?;
            let transform = transform(r)?;
            let brush = brush(r, resources)?;
            let brush_transform = optional_transform(r)?;
            RenderCommand::Fill(FillCommand {
                fill,
                transform,
                brush,
                brush_transform,
                shape: path(r, counts)?,
            })
        }
        STROKE => {
            let style = stroke(r)?;
            let transform = transform(r)?;
            let brush = brush(r, resources)?;
            let brush_transform = optional_transform(r)?;
            RenderCommand::Stroke(StrokeCommand {
                style,
                transform,
                brush,
                brush_transform,
                shape: path(r, counts)?,
            })
        }
        CLIP => {
            open(counts)?;
            RenderCommand::PushClipLayer(ClipCommand {
                fill: fill(r)?,
                transform: transform(r)?,
                clip: path(r, counts)?,
            })
        }
        LAYER => {
            open(counts)?;
            let fill = fill(r)?;
            let mix = one_of(&ALL_MIX, r.u8()?, |v| tag!(v), "blend mix")?;
            let compose = one_of(&ALL_COMPOSE, r.u8()?, |v| tag!(v), "blend compose")?;
            RenderCommand::PushLayer(LayerCommand {
                fill,
                blend: BlendMode { mix, compose },
                alpha: r.f32()?,
                transform: transform(r)?,
                clip: path(r, counts)?,
                filter: None,
                backdrop_filter: None,
            })
        }
        POP => {
            counts.depth = counts.depth.checked_sub(1).ok_or(Error::UnbalancedPop)?;
            RenderCommand::PopLayer
        }
        SHADOW => RenderCommand::BoxShadow(BoxShadowCommand {
            transform: transform(r)?,
            rect: Rect::new(r.f64()?, r.f64()?, r.f64()?, r.f64()?),
            brush: color(r, None)?,
            radius: r.f64()?,
            std_dev: r.f64()?,
        }),
        GLYPHS => RenderCommand::GlyphRun(glyphs(r, resources, counts)?),
        other => return Err(Error::Unknown("command", other)),
    })
}

const fn open(counts: &mut Counts) -> Result<(), Error> {
    counts.depth = counts.depth.saturating_add(1);
    if counts.depth > MAX_DEPTH {
        return Err(Error::TooDeep(MAX_DEPTH));
    }
    Ok(())
}

fn id(r: &mut Reader<'_>) -> Result<Id, Error> {
    r.take(16)?.try_into().map(Id).map_err(|_| Error::Truncated)
}

fn glyphs(
    r: &mut Reader<'_>,
    resources: &Resources,
    counts: &mut Counts,
) -> Result<GlyphRunCommand, Error> {
    let font = resources.font(id(r)?)?;
    let index = r.u32()?;
    let font_size = r.f32()?;
    let hint = r.bool()?;
    let coords = usize::from(r.u16()?);
    let normalized_coords = (0..coords).map(|_| r.i16()).collect::<Result<_, _>>()?;
    let embolden = Vec2::new(r.f64()?, r.f64()?);
    let style = match r.u8()? {
        0 => Style::Fill(fill(r)?),
        1 => Style::Stroke(stroke(r)?),
        other => return Err(Error::Unknown("glyph style", other)),
    };
    let brush = brush(r, resources)?;
    let brush_alpha = r.f32()?;
    let transform = transform(r)?;
    let glyph_transform = optional_transform(r)?;
    let count = r.count()?;
    counts.glyphs = counts.glyphs.saturating_add(count);
    if counts.glyphs > MAX_GLYPHS {
        return Err(Error::TooManyGlyphs(MAX_GLYPHS));
    }
    let glyphs = (0..count)
        .map(|_| {
            Ok(Glyph {
                id: r.u32()?,
                x: r.f32()?,
                y: r.f32()?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(GlyphRunCommand {
        font_data: FontData::new(font, index),
        font_size,
        hint,
        normalized_coords,
        embolden,
        style,
        brush,
        brush_alpha,
        transform,
        glyph_transform,
        glyphs,
    })
}

fn fill(r: &mut Reader<'_>) -> Result<Fill, Error> {
    match r.u8()? {
        0 => Ok(Fill::NonZero),
        1 => Ok(Fill::EvenOdd),
        other => Err(Error::Unknown("fill rule", other)),
    }
}

fn extend(r: &mut Reader<'_>) -> Result<Extend, Error> {
    match r.u8()? {
        0 => Ok(Extend::Pad),
        1 => Ok(Extend::Repeat),
        2 => Ok(Extend::Reflect),
        other => Err(Error::Unknown("extend", other)),
    }
}

fn transform(r: &mut Reader<'_>) -> Result<Affine, Error> {
    optional_transform(r)?.ok_or(Error::Unknown("transform", 3))
}

fn optional_transform(r: &mut Reader<'_>) -> Result<Option<Affine>, Error> {
    Ok(Some(match r.u8()? {
        0 => Affine::IDENTITY,
        1 => Affine::translate((r.f64()?, r.f64()?)),
        2 => Affine::new([r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?]),
        3 => return Ok(None),
        other => return Err(Error::Unknown("transform", other)),
    }))
}

/// A colour; `tag` when its first byte was read already.
fn color(r: &mut Reader<'_>, tag: Option<u8>) -> Result<Color, Error> {
    let tag = match tag {
        Some(tag) => tag,
        None => r.u8()?,
    };
    match tag {
        0 => {
            let (red, green, blue, alpha) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            Ok(Color::from_rgba8(red, green, blue, alpha))
        }
        1 => Ok(Color::new([r.f32()?, r.f32()?, r.f32()?, r.f32()?])),
        other => Err(Error::Unknown("colour", other)),
    }
}

fn brush(r: &mut Reader<'_>, resources: &Resources) -> Result<Paint, Error> {
    Ok(match r.u8()? {
        2 => Paint::Gradient(gradient(r)?),
        3 => {
            let image = resources.image(id(r)?)?;
            let sampler = ImageSampler {
                x_extend: extend(r)?,
                y_extend: extend(r)?,
                quality: match r.u8()? {
                    0 => ImageQuality::Low,
                    1 => ImageQuality::Medium,
                    2 => ImageQuality::High,
                    other => return Err(Error::Unknown("image quality", other)),
                },
                alpha: r.f32()?,
            };
            Paint::Image(ImageBrush { image, sampler })
        }
        solid => Paint::Solid(color(r, Some(solid))?),
    })
}

fn gradient(r: &mut Reader<'_>) -> Result<Gradient, Error> {
    let kind = match r.u8()? {
        0 => GradientKind::Linear(LinearGradientPosition {
            start: Point::new(r.f64()?, r.f64()?),
            end: Point::new(r.f64()?, r.f64()?),
        }),
        1 => GradientKind::Radial(RadialGradientPosition {
            start_center: Point::new(r.f64()?, r.f64()?),
            start_radius: r.f32()?,
            end_center: Point::new(r.f64()?, r.f64()?),
            end_radius: r.f32()?,
        }),
        2 => GradientKind::Sweep(SweepGradientPosition {
            center: Point::new(r.f64()?, r.f64()?),
            start_angle: r.f32()?,
            end_angle: r.f32()?,
        }),
        other => return Err(Error::Unknown("gradient kind", other)),
    };
    let extend = extend(r)?;
    let interpolation_cs = one_of(&ALL_SPACES, r.u8()?, |v| tag!(v), "colour space")?;
    let hue_direction = one_of(&ALL_HUES, r.u8()?, |v| tag!(v), "hue direction")?;
    let interpolation_alpha_space = match r.u8()? {
        0 => InterpolationAlphaSpace::Premultiplied,
        1 => InterpolationAlphaSpace::Unpremultiplied,
        other => return Err(Error::Unknown("alpha space", other)),
    };
    let count = usize::from(r.u16()?);
    let mut stops = ColorStops::default();
    for _ in 0..count {
        let offset = r.f32()?;
        let color = Color::new([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
        stops.push(ColorStop {
            offset,
            color: DynamicColor::from_alpha_color(color),
        });
    }
    Ok(Gradient {
        kind,
        extend,
        interpolation_cs,
        hue_direction,
        interpolation_alpha_space,
        stops,
    })
}

fn stroke(r: &mut Reader<'_>) -> Result<Stroke, Error> {
    let width = r.f64()?;
    let join = match r.u8()? {
        0 => Join::Bevel,
        1 => Join::Miter,
        2 => Join::Round,
        other => return Err(Error::Unknown("stroke join", other)),
    };
    let miter_limit = r.f64()?;
    let (start_cap, end_cap) = (cap(r)?, cap(r)?);
    let dash_offset = r.f64()?;
    let dashes = usize::from(r.u16()?);
    let pattern = (0..dashes)
        .map(|_| r.f64())
        .collect::<Result<Vec<_>, _>>()?;
    let mut stroke = Stroke::new(width)
        .with_join(join)
        .with_miter_limit(miter_limit)
        .with_start_cap(start_cap)
        .with_end_cap(end_cap);
    if !pattern.is_empty() {
        stroke = stroke.with_dashes(dash_offset, pattern);
    }
    // An offset with no dashes draws nothing different, but it is what
    // was recorded.
    stroke.dash_offset = dash_offset;
    Ok(stroke)
}

fn cap(r: &mut Reader<'_>) -> Result<Cap, Error> {
    match r.u8()? {
        0 => Ok(Cap::Butt),
        1 => Ok(Cap::Square),
        2 => Ok(Cap::Round),
        other => Err(Error::Unknown("stroke cap", other)),
    }
}

fn path(r: &mut Reader<'_>, counts: &mut Counts) -> Result<BezPath, Error> {
    let kind = r.u8()?;
    let wide = match kind {
        0 | 2 => false,
        1 | 3 => true,
        other => return Err(Error::Unknown("path", other)),
    };
    let elements = if kind < 2 { 5 } else { r.count()? };
    counts.elements = counts.elements.saturating_add(elements);
    if counts.elements > MAX_PATH_ELEMENTS {
        return Err(Error::TooManyPathElements(MAX_PATH_ELEMENTS));
    }
    if kind < 2 {
        let rect = Rect::new(
            r.coord(wide)?,
            r.coord(wide)?,
            r.coord(wide)?,
            r.coord(wide)?,
        );
        return Ok(rect_path(rect));
    }
    let verbs = r.take(elements)?;
    let point = |r: &mut Reader<'_>| Ok::<_, Error>(Point::new(r.coord(wide)?, r.coord(wide)?));
    let mut out = Vec::with_capacity(elements);
    for verb in verbs {
        out.push(match verb {
            0 => PathEl::MoveTo(point(r)?),
            1 => PathEl::LineTo(point(r)?),
            2 => PathEl::QuadTo(point(r)?, point(r)?),
            3 => PathEl::CurveTo(point(r)?, point(r)?, point(r)?),
            4 => PathEl::ClosePath,
            other => return Err(Error::Unknown("path element", *other)),
        });
    }
    Ok(BezPath::from_vec(out))
}
