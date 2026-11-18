//! Hit regions: what a pointer does over a layer, handled by its host.
//!
//! A layer may carry regions: shapes in its content space, each with an
//! id the view chose, a cursor, and what the host draws over it while the
//! pointer is on it or presses it. The host hit-tests them itself, with
//! each layer placed as its animations place it now ([`Layers::hit`]),
//! so hover, the cursor and a press's highlight cost the view nothing.
//! The view hears what happened by the region's id ([`RegionEvent`]):
//! enter, leave, press, release.
//!
//! A region may also be dragged ([`Drag`]): the host turns the pointer's
//! movement along an axis into a `host.` value ([`crate::anim::Values`]),
//! scaled, clamped and stepped, and a layer bound to that value (a fader's
//! knob, a drag box) follows the pointer at the display's rate. The view
//! hears the value at most once a frame, and the release.
//!
//! The rules ([`Interaction`]) are pure: a host feeds it its pointer and
//! draws and sends what it says. Every region is checked as an animation
//! is ([`check`]), at most [`MAX_REGIONS`] in a pane.

use anyrender::PaintScene;
use peniko::kurbo::{Affine, Circle, Point, Rect, RoundedRect, Shape as _};
use peniko::{Color, Fill};

use crate::anim::{self, Name, Readings, Space, Values};
use crate::bytes::{Reader, Writer};
use crate::layers::{Key, Layers};
use crate::{Error, MAX_COORDINATE};

/// The most regions a pane's layers may hold, over all of them.
pub const MAX_REGIONS: usize = 4096;

/// The most regions one layer may hold.
pub const MAX_LAYER_REGIONS: usize = 1024;

/// Where a region is, in its layer's content space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Rect(Rect),
    /// A rectangle with corners of a radius.
    Rounded(Rect, f64),
    Circle(Point, f64),
}

impl Shape {
    fn contains(&self, p: Point) -> bool {
        match *self {
            Self::Rect(r) => r.contains(p),
            Self::Rounded(r, radius) => RoundedRect::from_rect(r, radius).contains(p),
            Self::Circle(c, radius) => Circle::new(c, radius).contains(p),
        }
    }

    fn fill(&self, scene: &mut impl PaintScene, at: Affine, color: Color) {
        match *self {
            Self::Rect(r) => scene.fill(Fill::NonZero, at, color, None, &r),
            Self::Rounded(r, radius) => {
                scene.fill(
                    Fill::NonZero,
                    at,
                    color,
                    None,
                    &RoundedRect::from_rect(r, radius),
                );
            }
            Self::Circle(c, radius) => {
                scene.fill(Fill::NonZero, at, color, None, &Circle::new(c, radius));
            }
        }
    }
}

/// The pointer's look over a region: CSS's names for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Grab,
    Grabbing,
    Move,
    EwResize,
    NsResize,
    ColResize,
    RowResize,
    Crosshair,
    Text,
    NotAllowed,
}

const CURSORS: [Cursor; 12] = [
    Cursor::Default,
    Cursor::Pointer,
    Cursor::Grab,
    Cursor::Grabbing,
    Cursor::Move,
    Cursor::EwResize,
    Cursor::NsResize,
    Cursor::ColResize,
    Cursor::RowResize,
    Cursor::Crosshair,
    Cursor::Text,
    Cursor::NotAllowed,
];

impl Cursor {
    /// Its CSS name (`cursor-icon`'s too).
    #[must_use]
    pub const fn css(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Pointer => "pointer",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::Move => "move",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
            Self::ColResize => "col-resize",
            Self::RowResize => "row-resize",
            Self::Crosshair => "crosshair",
            Self::Text => "text",
            Self::NotAllowed => "not-allowed",
        }
    }
}

/// Which way a drag reads the pointer: right, or down, in the region's
/// content space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
}

/// A region dragged: the pointer's movement along `axis` (content units)
/// times `scale`, added to the value it had when pressed, clamped to
/// `[min, max]` and, with a `step`, rounded to one from `min`.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    /// What it writes: a `host.` value.
    pub value: Name,
    pub axis: Axis,
    pub scale: f64,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
    /// Its value until the pointer moves it, and whenever the view sends
    /// another (automation): the view's say over a host's value.
    pub initial: f64,
    /// Inertia: let go while moving, it coasts on, slowing with this time
    /// constant (seconds, at most [`MAX_COAST`]), on the host's wall clock
    /// ([`Interaction::coast`]). A scrolled list or arrangement flicked.
    pub coast: Option<f64>,
}

/// The longest a coast's time constant may be, in seconds.
pub const MAX_COAST: f64 = 10.0;

/// How long after the pointer's last move a release still flicks: a
/// pointer held still, then let go, does not coast.
const FLICK_WITHIN: f64 = 0.1;

impl Drag {
    fn settle(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        let stepped = self.step.map_or(v, |step| {
            ((v - self.min) / step).round().mul_add(step, self.min)
        });
        stepped.clamp(self.min, self.max)
    }
}

/// One region of a layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// What the view knows it by.
    pub id: u64,
    pub shape: Shape,
    pub cursor: Cursor,
    /// Drawn over its shape while the pointer is on it.
    pub hover: Option<Color>,
    /// Drawn over its shape while it is pressed.
    pub press: Option<Color>,
    /// A layer of the pane's drawn while the pointer is on it, in this
    /// region's layer's space, over it: what it shows hovered, recorded
    /// once (a notehead in the hover colour). The view sends that layer
    /// with `visible: false`; the host shows it with no word to the view.
    pub hover_layer: Option<Key>,
    /// The layer drawn while it is pressed, the same way (else its
    /// `hover_layer`).
    pub press_layer: Option<Key>,
    /// What dragging it writes: at most one along each axis (a fader
    /// one, a box that moves anywhere two).
    pub drags: Vec<Drag>,
}

impl Region {
    /// `shape`, known as `id`, with the default cursor and no highlight.
    #[must_use]
    pub const fn new(id: u64, shape: Shape) -> Self {
        Self {
            id,
            shape,
            cursor: Cursor::Default,
            hover: None,
            press: None,
            hover_layer: None,
            press_layer: None,
            drags: Vec::new(),
        }
    }

    /// While hovered, layer `key` (sent hidden) is drawn over it in its
    /// layer's space: its content redrawn as hovered, not a box over it.
    #[must_use]
    pub const fn hover_layer(mut self, key: Key) -> Self {
        self.hover_layer = Some(key);
        self
    }

    /// While pressed, layer `key` (sent hidden) is drawn over it.
    #[must_use]
    pub const fn press_layer(mut self, key: Key) -> Self {
        self.press_layer = Some(key);
        self
    }

    /// Whether it says what to draw with layers: a batch that says so is
    /// of [`crate::ops::VERSION`] 3.
    #[must_use]
    pub const fn swaps(&self) -> bool {
        self.hover_layer.is_some() || self.press_layer.is_some()
    }

    #[must_use]
    pub const fn cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = cursor;
        self
    }

    #[must_use]
    pub const fn hover(mut self, color: Color) -> Self {
        self.hover = Some(color);
        self
    }

    #[must_use]
    pub const fn press(mut self, color: Color) -> Self {
        self.press = Some(color);
        self
    }

    #[must_use]
    pub fn drag(mut self, drag: Drag) -> Self {
        self.drags.push(drag);
        self
    }
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// Whether `regions` are ones a host handles.
///
/// At most [`MAX_LAYER_REGIONS`], every number finite and in range, shapes the
/// right way round, and a drag that writes a `host.` value between ends
/// in order, with a step above zero and its initial value between them.
///
/// # Errors
///
/// The first thing wrong.
pub fn check(regions: &[Region]) -> Result<(), Error> {
    if regions.len() > MAX_LAYER_REGIONS {
        return Err(Error::BadRegion("too many regions on one layer"));
    }
    for r in regions {
        let numbers: Vec<f64> = match r.shape {
            Shape::Rect(b) => vec![b.x0, b.y0, b.x1, b.y1],
            Shape::Rounded(b, radius) => vec![b.x0, b.y0, b.x1, b.y1, radius],
            Shape::Circle(c, radius) => vec![c.x, c.y, radius],
        };
        if !numbers.into_iter().all(in_range) {
            return Err(Error::OutOfRange);
        }
        let backwards = match r.shape {
            Shape::Rect(b) => b.x0 > b.x1 || b.y0 > b.y1,
            Shape::Rounded(b, radius) => b.x0 > b.x1 || b.y0 > b.y1 || radius < 0.0,
            Shape::Circle(_, radius) => radius < 0.0,
        };
        if backwards {
            return Err(Error::BadRegion("a shape the wrong way round"));
        }
        if r.drags.len() > 2 || matches!(r.drags.as_slice(), [a, b] if a.axis == b.axis) {
            return Err(Error::BadRegion("more than one drag along an axis"));
        }
        for d in &r.drags {
            if d.value.space() != Space::Host {
                return Err(Error::Denied("drag", d.value.to_string()));
            }
            let numbers = [
                Some(d.scale),
                Some(d.min),
                Some(d.max),
                d.step,
                Some(d.initial),
            ];
            if !numbers.into_iter().flatten().all(in_range) {
                return Err(Error::OutOfRange);
            }
            if d.coast.is_some_and(|c| c <= 0.0 || c > MAX_COAST) {
                return Err(Error::BadRegion("a coast of nothing, or past its limit"));
            }
            if d.min > d.max || d.step.is_some_and(|s| s <= 0.0) {
                return Err(Error::BadRegion(
                    "a drag's ends swapped, or a step of nothing",
                ));
            }
            if !(d.min..=d.max).contains(&d.initial) {
                return Err(Error::BadRegion("a drag's value outside its ends"));
            }
        }
    }
    Ok(())
}

/// The host's values for `new`'s drags: each one's initial value when
/// the host has none, or when the view sent another than it did before
/// (`old`): automation moves a fader the pointer is not holding.
///
/// # Errors
///
/// The pane holds as many values as it may.
pub fn seed(old: Option<&[Region]>, new: &[Region], values: &mut Values) -> Result<(), Error> {
    for d in new.iter().flat_map(|r| r.drags.iter()) {
        let before = old.and_then(|old| {
            old.iter()
                .flat_map(|r| r.drags.iter())
                .find(|o| o.value == d.value)
                .map(|o| o.initial)
        });
        let moved = before.is_some_and(|b| b.to_bits() != d.initial.to_bits());
        if values.get(&d.value).is_none() || moved {
            values.write(Space::Host, &d.value, d.initial)?;
        }
    }
    Ok(())
}

/// A region under the pointer: its layer and its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hit {
    pub key: Key,
    pub id: u64,
}

/// What the host draws over a region: hovered, or pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lit {
    pub hit: Hit,
    pub pressed: bool,
}

impl Layers {
    /// The region at `at` (the pane's logical pixels): the topmost
    /// drawn layer's last region there, each layer placed by its
    /// animations at `readings` and `values`, within its clip.
    #[must_use]
    pub fn hit(&self, at: Point, readings: &Readings, values: &Values) -> Option<(Hit, &Region)> {
        if self.region_count() == 0 {
            return None;
        }
        self.in_order().rev().find_map(|(key, layer)| {
            let regions = layer.regions.as_deref()?;
            let placed = anim::place(layer, readings, values);
            if !placed.visible || placed.opacity <= 0.0 {
                return None;
            }
            if layer.clip.is_some_and(|clip| !clip.contains(at)) {
                return None;
            }
            let inside = content_point(placed.transform, at)?;
            regions
                .iter()
                .rev()
                .find(|r| r.shape.contains(inside))
                .map(|r| (Hit { key, id: r.id }, r))
        })
    }

    /// The region `hit` names, if its layer still has it.
    #[must_use]
    pub fn region(&self, hit: Hit) -> Option<&Region> {
        self.get(hit.key)?
            .regions
            .as_deref()?
            .iter()
            .find(|r| r.id == hit.id)
    }
}

/// `at` (pane pixels) in the content space `transform` takes to them.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
fn content_point(transform: Affine, at: Point) -> Option<Point> {
    if transform.determinant().abs() < 1e-12 {
        return None;
    }
    let p = transform.inverse() * at;
    (p.x.is_finite() && p.y.is_finite()).then_some(p)
}

/// Draws `lit`'s highlight over its region, its layer at `at`: its
/// colour over its shape, then the layer it shows (`swap`, the commands
/// of [`Region::hover_layer`] or [`Region::press_layer`], when there is
/// one), in the same space.
pub(crate) fn draw_lit(
    scene: &mut impl PaintScene,
    region: &Region,
    at: Affine,
    pressed: bool,
    swap: Option<&[anyrender::recording::RenderCommand]>,
) {
    let color = if pressed {
        region.press.or(region.hover)
    } else {
        region.hover
    };
    if let Some(color) = color {
        region.shape.fill(scene, at, color);
    }
    if let Some(commands) = swap {
        crate::layers::replay(scene, commands, at);
    }
}

/// The layer `region` shows while lit (pressed or not), if it names one.
#[must_use]
pub const fn shown_layer(region: &Region, pressed: bool) -> Option<Key> {
    if pressed {
        match region.press_layer {
            Some(k) => Some(k),
            None => region.hover_layer,
        }
    } else {
        region.hover_layer
    }
}

/// The modifier keys held as a region was pressed or let go, by the
/// names a view's input gives them (`"Mod"`, `"Shift"`, `"Alt"`,
/// `"Ctrl"`; `"Mod"` is ⌘ on a Mac and Ctrl elsewhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Self = Self(0);
    const MOD: u8 = 1;
    const SHIFT: u8 = 2;
    const ALT: u8 = 4;
    const CTRL: u8 = 8;
    const NAMES: [(u8, &'static str); 4] = [
        (Self::MOD, "Mod"),
        (Self::SHIFT, "Shift"),
        (Self::ALT, "Alt"),
        (Self::CTRL, "Ctrl"),
    ];

    /// The keys held, as a host's pointer event says them: ⌘ is `"Mod"`
    /// on a Mac, Ctrl is `"Mod"` (and `"Ctrl"`) elsewhere.
    #[must_use]
    // A pointer event's four keys, as every host's event gives them.
    #[allow(clippy::fn_params_excessive_bools)]
    pub const fn held(shift: bool, alt: bool, ctrl: bool, meta: bool) -> Self {
        let command = if cfg!(target_os = "macos") {
            meta
        } else {
            ctrl
        };
        let mut bits = 0;
        if command {
            bits |= Self::MOD;
        }
        if shift {
            bits |= Self::SHIFT;
        }
        if alt {
            bits |= Self::ALT;
        }
        if ctrl {
            bits |= Self::CTRL;
        }
        Self(bits)
    }

    /// From names as a view's input gives them; unknown names are none.
    #[must_use]
    pub fn from_names(names: &[&str]) -> Self {
        Self(
            Self::NAMES
                .iter()
                .filter(|(_, n)| names.contains(n))
                .fold(0, |bits, (bit, _)| bits | bit),
        )
    }

    /// Their names, in `"Mod"`, `"Shift"`, `"Alt"`, `"Ctrl"` order.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, n)| *n)
            .collect()
    }

    /// Whether `name` (`"Shift"`, `"Mod"`) is held.
    #[must_use]
    pub fn has(self, name: &str) -> bool {
        Self::NAMES
            .iter()
            .any(|(bit, n)| *n == name && self.0 & bit != 0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// What happened to a region, as its view hears it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegionEvent {
    Enter(u64),
    Leave(u64),
    /// Pressed, with the modifiers held (a Shift-click is a range, a
    /// ⌘-click a toggle).
    Press(u64, Mods),
    /// Let go, with the modifiers held then.
    Release(u64, Mods),
    /// A drag's value along an axis: at most once a frame, and once more
    /// before its release.
    Value(u64, Axis, f64),
}

impl RegionEvent {
    /// The region it is about.
    #[must_use]
    pub const fn id(&self) -> u64 {
        match *self {
            Self::Enter(id)
            | Self::Leave(id)
            | Self::Press(id, _)
            | Self::Release(id, _)
            | Self::Value(id, _, _) => id,
        }
    }
}

/// What a pointer event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The host handled it: it is not the view's as a pointer event.
    pub consumed: bool,
    /// What is drawn changed (a highlight, a dragged value).
    pub redraw: bool,
    /// The cursor to show now, when it changed ([`Cursor::Default`]:
    /// the pane's own).
    pub cursor: Option<Cursor>,
}

struct Dragging {
    drag: Drag,
    /// Content space at the press: the region's layer may move with the
    /// value it drags, and the pointer's movement is read as it was.
    inverse: Affine,
    from: f64,
    start: f64,
    last: f64,
    /// The last two moves: wall seconds and the value they gave, for the
    /// speed a release flicks it at.
    moves: [Option<(f64, f64)>; 2],
}

/// A value coasting after a flick.
struct Coast {
    id: u64,
    drag: Drag,
    tau: f64,
    from: f64,
    /// Value units a second when let go.
    speed: f64,
    began: f64,
    last: f64,
}

struct Held {
    hit: Hit,
    dragging: Vec<Dragging>,
}

/// A pane's pointer over its regions: the host's side of hover, press
/// and drag.
#[derive(Default)]
pub struct Interaction {
    hovered: Option<(Hit, Cursor)>,
    held: Option<Held>,
    queue: Vec<RegionEvent>,
    coasting: Vec<Coast>,
}

impl std::fmt::Debug for Interaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interaction")
            .field("hovered", &self.hovered)
            .field("held", &self.held.as_ref().map(|h| h.hit))
            .finish_non_exhaustive()
    }
}

impl Interaction {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a region is pressed (the pointer is the host's until it is
    /// released).
    #[must_use]
    pub const fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// What to highlight.
    #[must_use]
    pub const fn lit(&self) -> Option<Lit> {
        match (&self.held, &self.hovered) {
            (Some(h), _) => Some(Lit {
                hit: h.hit,
                pressed: true,
            }),
            (None, Some((hit, _))) => Some(Lit {
                hit: *hit,
                pressed: false,
            }),
            (None, None) => None,
        }
    }

    /// Whether it has events the view has not heard.
    #[must_use]
    pub const fn pending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// What the view has not heard, in order, a drag's values coalesced
    /// to the last of each run.
    pub fn take(&mut self) -> Vec<RegionEvent> {
        let mut out: Vec<RegionEvent> = Vec::with_capacity(self.queue.len());
        for e in self.queue.drain(..) {
            // The run of a drag's values since the press, latest of each.
            if let RegionEvent::Value(id, axis, _) = e {
                let run = out
                    .iter()
                    .rposition(|o| !matches!(o, RegionEvent::Value(i, _, _) if *i == id))
                    .map_or(0, |i| i.saturating_add(1));
                if let Some(at) = out.get(run..).and_then(|tail| {
                    tail.iter()
                        .position(|o| matches!(o, RegionEvent::Value(_, a, _) if *a == axis))
                }) {
                    out.remove(run.saturating_add(at));
                }
            }
            out.push(e);
        }
        out
    }

    fn hover(&mut self, now: Option<(Hit, Cursor)>) -> Outcome {
        let before = self.hovered.map(|(h, _)| h);
        let after = now.map(|(h, _)| h);
        let cursor_was = self.cursor();
        self.hovered = now;
        let mut out = Outcome::default();
        if before != after {
            if let Some(h) = before {
                self.queue.push(RegionEvent::Leave(h.id));
            }
            if let Some(h) = after {
                self.queue.push(RegionEvent::Enter(h.id));
            }
            out.redraw = true;
        }
        if cursor_was != self.cursor() {
            out.cursor = Some(self.cursor());
        }
        out
    }

    /// The pointer moved to `at` (pane pixels), no button held or one
    /// held on a region.
    pub fn moved(
        &mut self,
        layers: &Layers,
        readings: &Readings,
        values: &mut Values,
        at: Point,
    ) -> Outcome {
        if let Some(held) = &mut self.held {
            let mut out = Outcome {
                consumed: true,
                ..Outcome::default()
            };
            for d in &mut held.dragging {
                // kurbo's transforms compose with `*`: no overflow.
                #[allow(clippy::arithmetic_side_effects)]
                let p = d.inverse * at;
                let along = match d.drag.axis {
                    Axis::X => p.x,
                    Axis::Y => p.y,
                };
                let v = d
                    .drag
                    .settle((along - d.from).mul_add(d.drag.scale, d.start));
                // A sample a moment: the release's own move at the last
                // move's time adds nothing.
                if let Some(t) = readings.wall
                    && d.moves[1].is_none_or(|(last, _)| t > last)
                {
                    d.moves = [d.moves[1], Some((t, v))];
                }
                if v.to_bits() != d.last.to_bits()
                    && values.write(Space::Host, &d.drag.value, v).is_ok()
                {
                    d.last = v;
                    self.queue
                        .push(RegionEvent::Value(held.hit.id, d.drag.axis, v));
                    out.redraw = true;
                }
            }
            return out;
        }
        let now = layers
            .hit(at, readings, values)
            .map(|(hit, region)| (hit, region.cursor));
        self.hover(now)
    }

    /// A button went down at `at`: on a region, it is the host's.
    pub fn pressed(
        &mut self,
        layers: &Layers,
        readings: &Readings,
        values: &mut Values,
        at: Point,
    ) -> Outcome {
        self.pressed_with(layers, readings, values, at, Mods::NONE)
    }

    /// [`Self::pressed`] with `mods` held: what the view hears with the
    /// press.
    pub fn pressed_with(
        &mut self,
        layers: &Layers,
        readings: &Readings,
        values: &mut Values,
        at: Point,
        mods: Mods,
    ) -> Outcome {
        let mut out = self.moved(layers, readings, values, at);
        if self.held.is_some() {
            return out;
        }
        let Some((hit, region)) = layers.hit(at, readings, values) else {
            return out;
        };
        let dragging = region.drags.iter().filter_map(|drag| {
            let layer = layers.get(hit.key)?;
            let placed = anim::place(layer, readings, values);
            if placed.transform.determinant().abs() < 1e-12 {
                return None;
            }
            let inverse = placed.transform.inverse();
            // kurbo's transforms compose with `*`: no overflow.
            #[allow(clippy::arithmetic_side_effects)]
            let p = inverse * at;
            let start = values.get(&drag.value).unwrap_or(drag.initial);
            Some(Dragging {
                drag: drag.clone(),
                inverse,
                from: match drag.axis {
                    Axis::X => p.x,
                    Axis::Y => p.y,
                },
                start,
                last: start,
                moves: [None, readings.wall.map(|t| (t, start))],
            })
        });
        let dragging: Vec<Dragging> = dragging.collect();
        // A press stops what it coasts.
        self.coasting
            .retain(|c| !dragging.iter().any(|d| d.drag.value == c.drag.value));
        self.queue.push(RegionEvent::Press(hit.id, mods));
        self.held = Some(Held { hit, dragging });
        out.consumed = true;
        out.redraw = true;
        out
    }

    /// The button came up at `at`: the region held hears its release, and
    /// what is under the pointer now is hovered.
    pub fn released(
        &mut self,
        layers: &Layers,
        readings: &Readings,
        values: &mut Values,
        at: Point,
    ) -> Outcome {
        self.released_with(layers, readings, values, at, Mods::NONE)
    }

    /// [`Self::released`] with `mods` held.
    pub fn released_with(
        &mut self,
        layers: &Layers,
        readings: &Readings,
        values: &mut Values,
        at: Point,
        mods: Mods,
    ) -> Outcome {
        let mut out = self.moved(layers, readings, values, at);
        let Some(held) = self.held.take() else {
            return out;
        };
        self.queue.push(RegionEvent::Release(held.hit.id, mods));
        // Let go while moving: a drag with inertia coasts on.
        for d in held.dragging {
            let (Some(tau), Some(now), [Some((t0, v0)), Some((t1, v1))]) =
                (d.drag.coast, readings.wall, d.moves)
            else {
                continue;
            };
            let dt = t1 - t0;
            if dt <= 0.0 || now - t1 > FLICK_WITHIN {
                continue;
            }
            let speed = (v1 - v0) / dt;
            if speed.abs() > f64::EPSILON && speed.is_finite() {
                self.coasting.push(Coast {
                    id: held.hit.id,
                    tau,
                    from: d.last,
                    speed,
                    began: now,
                    last: d.last,
                    drag: d.drag,
                });
                out.redraw = true;
            }
        }
        let hover = self.hover(
            layers
                .hit(at, readings, values)
                .map(|(hit, region)| (hit, region.cursor)),
        );
        out.consumed = true;
        out.redraw = true;
        out.cursor = hover.cursor.or(out.cursor);
        out
    }

    /// The pointer left the pane, or its press was lost: let go of what
    /// it held, and of the hover.
    pub fn left(&mut self) -> Outcome {
        let mut out = Outcome::default();
        if let Some(held) = self.held.take() {
            self.queue
                .push(RegionEvent::Release(held.hit.id, Mods::NONE));
            out.redraw = true;
        }
        let hover = self.hover(None);
        out.redraw |= hover.redraw;
        out.cursor = hover.cursor;
        out
    }

    /// The cursor to show now.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.hovered.map(|(_, c)| c).unwrap_or_default()
    }

    /// Whether a flicked value is still coasting: its host draws every
    /// frame until it stops.
    #[must_use]
    pub const fn coasting(&self) -> bool {
        !self.coasting.is_empty()
    }

    /// Moves what coasts to where it is at `readings`' wall time: each
    /// `from + speed × tau × (1 − e^(−t/tau))`, settled as its drag
    /// settles a value. One stops at an end of its drag, or once it moves
    /// less than a thousandth of its range a second. Whether a value
    /// changed (a frame to draw); the view hears each at most once a
    /// frame, as a drag's.
    pub fn coast(&mut self, readings: &Readings, values: &mut Values) -> bool {
        let Some(now) = readings.wall else {
            return false;
        };
        let mut moved = false;
        let queue = &mut self.queue;
        self.coasting.retain_mut(|c| {
            let t = (now - c.began).max(0.0);
            let decay = (-t / c.tau).exp();
            let raw = (c.speed * c.tau).mul_add(1.0 - decay, c.from);
            let v = c.drag.settle(raw);
            if v.to_bits() != c.last.to_bits()
                && values.write(Space::Host, &c.drag.value, v).is_ok()
            {
                c.last = v;
                queue.push(RegionEvent::Value(c.id, c.drag.axis, v));
                moved = true;
            }
            let range = (c.drag.max - c.drag.min).abs();
            let slow = (c.speed * decay).abs() <= range * 1e-3;
            let at_end = v <= c.drag.min && c.speed < 0.0 || v >= c.drag.max && c.speed > 0.0;
            !(slow || at_end)
        });
        moved
    }
}

// The bytes of an `ops::REGIONS`.

const RECT: u8 = 0;
const ROUNDED: u8 = 1;
const CIRCLE: u8 = 2;
const HOVER: u8 = 1;
const PRESS: u8 = 2;
const DRAG: u8 = 4;
/// A layer drawn while hovered: batch version 3.
const HOVER_LAYER: u8 = 8;
/// A layer drawn while pressed: batch version 3.
const PRESS_LAYER: u8 = 16;
const STEP: u8 = 1;
const COAST: u8 = 2;

fn color(w: &mut Writer, c: Color) {
    w.bytes(&c.to_rgba8().to_u8_array());
}

fn read_color(r: &mut Reader<'_>) -> Result<Color, Error> {
    let b = r.take(4)?;
    match *b {
        [red, green, blue, alpha] => Ok(Color::from_rgba8(red, green, blue, alpha)),
        _ => Err(Error::Truncated),
    }
}

fn rect(w: &mut Writer, b: Rect) {
    for v in [b.x0, b.y0, b.x1, b.y1] {
        w.f64(v);
    }
}

fn read_rect(r: &mut Reader<'_>) -> Result<Rect, Error> {
    Ok(Rect::new(r.f64()?, r.f64()?, r.f64()?, r.f64()?))
}

/// `regions` as bytes: a count, then each.
pub(crate) fn write(w: &mut Writer, regions: &[Region]) {
    w.u16(u16::try_from(regions.len()).unwrap_or(u16::MAX));
    for region in regions {
        w.u64(region.id);
        match region.shape {
            Shape::Rect(b) => {
                w.u8(RECT);
                rect(w, b);
            }
            Shape::Rounded(b, radius) => {
                w.u8(ROUNDED);
                rect(w, b);
                w.f64(radius);
            }
            Shape::Circle(c, radius) => {
                w.u8(CIRCLE);
                w.f64(c.x);
                w.f64(c.y);
                w.f64(radius);
            }
        }
        w.u8(CURSORS
            .iter()
            .position(|c| *c == region.cursor)
            .and_then(|i| u8::try_from(i).ok())
            .unwrap_or_default());
        let flags = [
            (region.hover.is_some(), HOVER),
            (region.press.is_some(), PRESS),
            (!region.drags.is_empty(), DRAG),
            (region.hover_layer.is_some(), HOVER_LAYER),
            (region.press_layer.is_some(), PRESS_LAYER),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit);
        w.u8(flags);
        for c in [region.hover, region.press].into_iter().flatten() {
            color(w, c);
        }
        for key in [region.hover_layer, region.press_layer]
            .into_iter()
            .flatten()
        {
            w.u64(key);
        }
        if !region.drags.is_empty() {
            w.u8(u8::try_from(region.drags.len()).unwrap_or(u8::MAX));
        }
        for d in &region.drags {
            anim::write_name(w, &d.value);
            w.u8(match d.axis {
                Axis::X => 0,
                Axis::Y => 1,
            });
            for v in [d.scale, d.min, d.max] {
                w.f64(v);
            }
            w.u8(
                if d.step.is_some() { STEP } else { 0 } | if d.coast.is_some() { COAST } else { 0 }
            );
            for v in [d.step, d.coast].into_iter().flatten() {
                w.f64(v);
            }
            w.f64(d.initial);
        }
    }
}

/// Regions from bytes, checked ([`check`]).
pub(crate) fn read(r: &mut Reader<'_>) -> Result<Vec<Region>, Error> {
    let n = usize::from(r.u16()?);
    if n > MAX_LAYER_REGIONS {
        return Err(Error::BadRegion("too many regions on one layer"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.u64()?;
        let shape = match r.u8()? {
            RECT => Shape::Rect(read_rect(r)?),
            ROUNDED => Shape::Rounded(read_rect(r)?, r.f64()?),
            CIRCLE => Shape::Circle(Point::new(r.f64()?, r.f64()?), r.f64()?),
            other => return Err(Error::Unknown("shape", other)),
        };
        let c = r.u8()?;
        let cursor = CURSORS
            .get(usize::from(c))
            .copied()
            .ok_or(Error::Unknown("cursor", c))?;
        let flags = r.u8()?;
        if flags & !(HOVER | PRESS | DRAG | HOVER_LAYER | PRESS_LAYER) != 0 {
            return Err(Error::Unknown("region", flags));
        }
        let hover = if flags & HOVER == 0 {
            None
        } else {
            Some(read_color(r)?)
        };
        let press = if flags & PRESS == 0 {
            None
        } else {
            Some(read_color(r)?)
        };
        let hover_layer = if flags & HOVER_LAYER == 0 {
            None
        } else {
            Some(r.u64()?)
        };
        let press_layer = if flags & PRESS_LAYER == 0 {
            None
        } else {
            Some(r.u64()?)
        };
        let count = if flags & DRAG == 0 { 0 } else { r.u8()? };
        if count > 2 {
            return Err(Error::BadRegion("more than one drag along an axis"));
        }
        let mut drags = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let value = anim::read_name(r)?;
            let axis = match r.u8()? {
                0 => Axis::X,
                1 => Axis::Y,
                other => return Err(Error::Unknown("axis", other)),
            };
            let (scale, min, max) = (r.f64()?, r.f64()?, r.f64()?);
            let flags = r.u8()?;
            if flags & !(STEP | COAST) != 0 {
                return Err(Error::Unknown("drag", flags));
            }
            let step = if flags & STEP == 0 {
                None
            } else {
                Some(r.f64()?)
            };
            let coast = if flags & COAST == 0 {
                None
            } else {
                Some(r.f64()?)
            };
            drags.push(Drag {
                value,
                axis,
                scale,
                min,
                max,
                step,
                initial: r.f64()?,
                coast,
            });
        }
        out.push(Region {
            id,
            shape,
            cursor,
            hover,
            press,
            hover_layer,
            press_layer,
            drags,
        });
    }
    check(&out)?;
    Ok(out)
}
