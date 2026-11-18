//! Animation: what moves without the view.
//!
//! At 240 frames a second a frame is about 4 ms, and nothing that moves
//! every frame may wait on the view that drew it. So a layer can say how
//! it moves, as data its host evaluates each frame: a property (where it
//! is, how large, how opaque, whether it is drawn) as a function of an
//! input. The view says it once; the host runs it for as long as it
//! holds the layer, and the view is asked again only when what it draws
//! changes.
//!
//! - An [`Input`] is a named [`Clock`] (wall time, or the app's playhead,
//!   with a clock to fall back on when the first has nothing to read), or
//!   a named value ([`Values`]): a meter, a fader, a level.
//! - A [`Map`] takes the input to the property: a line ([`Linear`]:
//!   `scale × input + offset`, wrapped and clamped), or steps
//!   ([`Steps`]: a value from each of a few inputs on).
//! - The host reads its clocks once a frame ([`Readings`], from
//!   [`Clocks`]) and evaluates every animated layer with them
//!   ([`place`]). A paused, seeking or tempo-changed playhead is a new
//!   reading, so the animation follows the clock whatever it does, with
//!   no word from the view.
//!
//! An animation is data, never code, and is checked as a fragment is
//! ([`check`]): every number finite and in range, the steps in order, at
//! most [`MAX_ANIMATIONS`] in a pane, and every value it reads one the
//! pane may read ([`Access`]). What fails the check fails the view that
//! sent it, never the app.
//!
//! Nothing here depends on which window draws: a pane's layers, values
//! and the app's clocks are the same wherever the pane is shown.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use peniko::kurbo::Affine;

use crate::bytes::{Reader, Writer};
use crate::layers::Layer;
use crate::{Error, MAX_COORDINATE};

/// The most animations a pane's layers may hold, over all of them.
pub const MAX_ANIMATIONS: usize = 4096;

/// The most steps one [`Steps`] may have.
pub const MAX_STEPS: usize = 256;

/// The most named values a pane may hold.
pub const MAX_VALUES: usize = 1024;

/// The longest a value's name may be, in bytes.
pub const MAX_NAME: usize = 64;

/// What an animation sets: one per property per layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Property {
    /// A move right, in the layer's content space.
    X,
    /// A move down, in the layer's content space.
    Y,
    /// A scale across, about the content's origin.
    ScaleX,
    /// A scale down, about the content's origin.
    ScaleY,
    /// 0 to 1: multiplies the layer's own opacity.
    Opacity,
    /// Drawn when half or more.
    Visible,
}

/// The properties, by their number on the wire.
const PROPERTIES: [Property; 6] = [
    Property::X,
    Property::Y,
    Property::ScaleX,
    Property::ScaleY,
    Property::Opacity,
    Property::Visible,
];

/// A clock the host reads each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Clock {
    /// Seconds on the app's monotonic clock: the same in every pane and
    /// every window of the app ([`wall_seconds`]).
    Wall,
    /// Seconds into what the app's playhead plays, when something is on
    /// it: paused, seeking and its rate are the playhead's, not the
    /// view's.
    Playhead,
}

const CLOCKS: [Clock; 2] = [Clock::Wall, Clock::Playhead];

/// What an animation reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A clock, or `fallback` when `clock` has nothing to read (nothing
    /// is on the playhead).
    Clock {
        clock: Clock,
        fallback: Option<Clock>,
    },
    /// A named value of the pane ([`Values`]).
    Value(Name),
}

impl Input {
    /// `clock`, with nothing to fall back on.
    #[must_use]
    pub const fn clock(clock: Clock) -> Self {
        Self::Clock {
            clock,
            fallback: None,
        }
    }

    /// `clock`, or `fallback` when it has nothing to read.
    #[must_use]
    pub const fn clock_or(clock: Clock, fallback: Clock) -> Self {
        Self::Clock {
            clock,
            fallback: Some(fallback),
        }
    }

    /// The clock it reads now, and that clock's reading.
    fn reading(&self, r: &Readings) -> Option<Reading> {
        match self {
            Self::Clock { clock, fallback } => r
                .clock(*clock)
                .or_else(|| fallback.and_then(|f| r.clock(f))),
            Self::Value(_) => None,
        }
    }

    fn read(&self, r: &Readings, values: &Values) -> Option<f64> {
        match self {
            Self::Clock { .. } => self.reading(r).map(|reading| reading.time),
            Self::Value(name) => values.get(name),
        }
    }
}

/// An input to a property: `scale × input + offset`, then wrapped into
/// `[0, wrap)`, then clamped to `[min, max]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linear {
    pub scale: f64,
    pub offset: f64,
    /// Wraps it, so it repeats every `wrap`: a loop.
    pub wrap: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Linear {
    /// `scale × input + offset`, unwrapped and unclamped.
    #[must_use]
    pub const fn new(scale: f64, offset: f64) -> Self {
        Self {
            scale,
            offset,
            wrap: None,
            min: None,
            max: None,
        }
    }

    #[must_use]
    pub const fn wrap(mut self, every: f64) -> Self {
        self.wrap = Some(every);
        self
    }

    #[must_use]
    pub const fn clamp(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    fn apply(&self, x: f64) -> f64 {
        let mut y = self.scale.mul_add(x, self.offset);
        if let Some(every) = self.wrap {
            y = y.rem_euclid(every);
        }
        if let Some(min) = self.min {
            y = y.max(min);
        }
        if let Some(max) = self.max {
            y = y.min(max);
        }
        y
    }
}

/// An input to a property in steps: `before` until the first step's
/// input, then each step's value from its input on. A lyric line is
/// visible from its start to its end: `before` 0, then `(start, 1)`,
/// `(end, 0)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Steps {
    pub before: f64,
    /// `(from, value)`, `from` strictly increasing.
    pub steps: Vec<(f64, f64)>,
}

impl Steps {
    fn apply(&self, x: f64) -> f64 {
        let past = self.steps.partition_point(|(from, _)| *from <= x);
        past.checked_sub(1)
            .and_then(|i| self.steps.get(i))
            .map_or(self.before, |(_, value)| *value)
    }
}

/// How an input becomes a property.
#[derive(Debug, Clone, PartialEq)]
pub enum Map {
    Linear(Linear),
    Steps(Steps),
}

impl Map {
    fn apply(&self, x: f64) -> f64 {
        let y = match self {
            Self::Linear(l) => l.apply(x),
            Self::Steps(s) => s.apply(x),
        };
        // Whatever the input, the property stays one a renderer takes.
        if y.is_nan() {
            0.0
        } else {
            y.clamp(-MAX_COORDINATE, MAX_COORDINATE)
        }
    }
}

/// One property of a layer, as a function of an input.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub property: Property,
    pub input: Input,
    pub map: Map,
}

impl Animation {
    #[must_use]
    pub const fn new(property: Property, input: Input, map: Map) -> Self {
        Self {
            property,
            input,
            map,
        }
    }

    /// `property` as `scale × input + offset`.
    #[must_use]
    pub const fn linear(property: Property, input: Input, linear: Linear) -> Self {
        Self::new(property, input, Map::Linear(linear))
    }
}

/// One clock's reading: where it is, and how fast it moves on its own.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// Seconds.
    pub time: f64,
    /// Its seconds a second of wall time: 1 for wall time, a playhead's
    /// rate while it plays, 0 while it stands (paused, or at the end of
    /// what it plays).
    pub rate: f64,
}

impl Reading {
    /// A clock standing at `time`.
    #[must_use]
    pub const fn still(time: f64) -> Self {
        Self { time, rate: 0.0 }
    }

    /// A clock at `time`, moving at `rate`.
    #[must_use]
    pub const fn moving(time: f64, rate: f64) -> Self {
        Self { time, rate }
    }

    /// Whether it moves on its own.
    #[must_use]
    pub fn running(&self) -> bool {
        self.rate != 0.0
    }
}

/// Every clock, read once for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Readings {
    /// [`Clock::Wall`]: `None` reads nothing (a host with no clocks).
    pub wall: Option<f64>,
    /// [`Clock::Playhead`]: `None` when nothing is on it.
    pub playhead: Option<Reading>,
}

impl Readings {
    /// No clock: every animation on one keeps its layer as it was sent.
    pub const NONE: Self = Self {
        wall: None,
        playhead: None,
    };

    #[must_use]
    pub const fn clock(&self, clock: Clock) -> Option<Reading> {
        match clock {
            Clock::Wall => match self.wall {
                Some(time) => Some(Reading::moving(time, 1.0)),
                None => None,
            },
            Clock::Playhead => self.playhead,
        }
    }
}

type WallSource = Arc<dyn Fn() -> f64 + Send + Sync>;
type PlayheadSource = Arc<dyn Fn() -> Option<Reading> + Send + Sync>;

/// Where a host reads its clocks: cheap to clone; every clone reads the
/// same sources.
#[derive(Clone)]
pub struct Clocks {
    wall: WallSource,
    playhead: PlayheadSource,
}

impl std::fmt::Debug for Clocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clocks").finish_non_exhaustive()
    }
}

static PLAYHEAD: RwLock<Option<PlayheadSource>> = RwLock::new(None);

/// Sets where the app's playhead is read ([`Clocks::app`]): the app that
/// has a playhead says so once.
pub fn install_playhead(source: impl Fn() -> Option<Reading> + Send + Sync + 'static) {
    *PLAYHEAD.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(source));
}

/// Time since the app first asked: the app's one wall clock, which a
/// pane's frame clock reads too.
#[must_use]
pub fn wall_elapsed() -> std::time::Duration {
    static EPOCH: OnceLock<web_time::Instant> = OnceLock::new();
    EPOCH.get_or_init(web_time::Instant::now).elapsed()
}

/// [`wall_elapsed`] in seconds.
#[must_use]
pub fn wall_seconds() -> f64 {
    wall_elapsed().as_secs_f64()
}

impl Clocks {
    /// Clocks read from `wall` and `playhead`.
    pub fn new(
        wall: impl Fn() -> f64 + Send + Sync + 'static,
        playhead: impl Fn() -> Option<Reading> + Send + Sync + 'static,
    ) -> Self {
        Self {
            wall: Arc::new(wall),
            playhead: Arc::new(playhead),
        }
    }

    /// The app's: [`wall_seconds`] and the playhead it installed
    /// ([`install_playhead`]), read each time, so every pane in every
    /// window reads the same.
    #[must_use]
    pub fn app() -> Self {
        Self::new(wall_seconds, || {
            let source = PLAYHEAD
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            source.and_then(|read| read())
        })
    }

    /// These, with wall time read from `wall`.
    #[must_use]
    pub fn with_wall(mut self, wall: impl Fn() -> f64 + Send + Sync + 'static) -> Self {
        self.wall = Arc::new(wall);
        self
    }

    /// Every clock, now.
    #[must_use]
    pub fn read(&self) -> Readings {
        Readings {
            wall: Some((self.wall)()).filter(|t| t.is_finite()),
            playhead: (self.playhead)().filter(|r| r.time.is_finite() && r.rate.is_finite()),
        }
    }
}

impl Default for Clocks {
    fn default() -> Self {
        Self::app()
    }
}

/// Who writes a value, and whose names it may write: a name's first
/// part says (`view.meter`, `host.fader.1`, `engine.track.3.peak`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Space {
    /// The view, by an operation it sends ([`crate::ops`]) or, linked,
    /// by [`Values::write`].
    View,
    /// The host: a pointer dragging a fader, a control surface.
    Host,
    /// The engine, lock-free. Read only by a pane granted it
    /// ([`Access::engine`]).
    Engine,
}

impl Space {
    const fn prefix(self) -> &'static str {
        match self {
            Self::View => "view.",
            Self::Host => "host.",
            Self::Engine => "engine.",
        }
    }
}

/// A value's name: `<space>.<path>`, the path of lowercase letters,
/// digits, `.`, `_` and `-`, at most [`MAX_NAME`] bytes in all.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(Arc<str>);

impl Name {
    /// `name`, checked.
    ///
    /// # Errors
    ///
    /// It is not a name.
    pub fn new(name: &str) -> Result<Self, Error> {
        let bad = || Error::BadName(name.chars().take(MAX_NAME).collect());
        if name.len() > MAX_NAME {
            return Err(bad());
        }
        let space = [Space::View, Space::Host, Space::Engine]
            .into_iter()
            .find(|s| name.starts_with(s.prefix()))
            .ok_or_else(bad)?;
        let path = name.get(space.prefix().len()..).unwrap_or_default();
        let fits =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
        if path.is_empty() || !path.chars().all(fits) {
            return Err(bad());
        }
        Ok(Self(Arc::from(name)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whose it is.
    #[must_use]
    pub fn space(&self) -> Space {
        [Space::Engine, Space::Host]
            .into_iter()
            .find(|s| self.0.starts_with(s.prefix()))
            .unwrap_or(Space::View)
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a pane may read: its own view's values and its host's always;
/// the engine's when the host grants it (a capability, later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Access {
    pub engine: bool,
}

impl Access {
    #[must_use]
    pub const fn may_read(self, space: Space) -> bool {
        match space {
            Space::View | Space::Host => true,
            Space::Engine => self.engine,
        }
    }

    /// Whether `writer` may write `name`: each its own space only.
    #[must_use]
    pub fn may_write(self, writer: Space, name: &Name) -> bool {
        name.space() == writer
    }
}

/// A pane's named values: what a layer's property can be bound to.
#[derive(Debug, Clone, Default)]
pub struct Values {
    held: BTreeMap<Name, f64>,
    revision: u64,
}

impl Values {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times one changed: what a host compares to know whether
    /// to draw again.
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
    pub fn get(&self, name: &Name) -> Option<f64> {
        self.held.get(name).copied()
    }

    /// `name` set to `value` by `writer`: whether it changed.
    ///
    /// # Errors
    ///
    /// `writer` may not write `name`, `value` is out of range, or the
    /// pane holds [`MAX_VALUES`] already.
    pub fn write(&mut self, writer: Space, name: &Name, value: f64) -> Result<bool, Error> {
        if !Access::default().may_write(writer, name) {
            return Err(Error::Denied("write", name.to_string()));
        }
        if !in_range(value) {
            return Err(Error::OutOfRange);
        }
        if self.held.get(name).map(|v| v.to_bits()) == Some(value.to_bits()) {
            return Ok(false);
        }
        if !self.held.contains_key(name) && self.held.len() >= MAX_VALUES {
            return Err(Error::TooManyValues(MAX_VALUES));
        }
        self.held.insert(name.clone(), value);
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    /// Every value, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&Name, f64)> {
        self.held.iter().map(|(name, v)| (name, *v))
    }
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// Whether `animations` are ones a host evaluates.
///
/// At most one per property, every number finite and in range, a wrap
/// above zero, a clamp's ends in order, steps in order and at most
/// [`MAX_STEPS`], and every value read one `access` lets the pane read.
///
/// # Errors
///
/// The first thing wrong.
pub fn check(animations: &[Animation], access: Access) -> Result<(), Error> {
    let mut seen = [false; PROPERTIES.len()];
    for a in animations {
        let at = PROPERTIES
            .iter()
            .position(|p| *p == a.property)
            .unwrap_or_default();
        let slot = seen.get_mut(at).ok_or(Error::BadAnimation("property"))?;
        if *slot {
            return Err(Error::BadAnimation("a property animated twice"));
        }
        *slot = true;
        if let Input::Value(name) = &a.input
            && !access.may_read(name.space())
        {
            return Err(Error::Denied("read", name.to_string()));
        }
        match &a.map {
            Map::Linear(l) => {
                let numbers = [Some(l.scale), Some(l.offset), l.wrap, l.min, l.max];
                if !numbers.into_iter().flatten().all(in_range) {
                    return Err(Error::OutOfRange);
                }
                if l.wrap.is_some_and(|w| w <= 0.0) {
                    return Err(Error::BadAnimation("a wrap of zero or less"));
                }
                if let (Some(min), Some(max)) = (l.min, l.max)
                    && min > max
                {
                    return Err(Error::BadAnimation("a clamp whose ends are swapped"));
                }
            }
            Map::Steps(s) => {
                if s.steps.len() > MAX_STEPS {
                    return Err(Error::BadAnimation("too many steps"));
                }
                let numbers = s.steps.iter().flat_map(|(from, v)| [*from, *v]);
                if !std::iter::once(s.before).chain(numbers).all(in_range) {
                    return Err(Error::OutOfRange);
                }
                if s.steps.windows(2).any(|w| match w {
                    [a, b] => a.0 >= b.0,
                    _ => false,
                }) {
                    return Err(Error::BadAnimation("steps out of order"));
                }
            }
        }
    }
    Ok(())
}

/// Where and how a layer is drawn now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub transform: Affine,
    pub opacity: f32,
    pub visible: bool,
}

/// `layer` as its animations place it with these clocks and values: as
/// it was sent where an input reads nothing.
///
/// Moves and scales are in the layer's content space, after its own
/// transform: a playhead's `X` is a time in the same units as the clips
/// it moves over, whatever the pan and zoom.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
#[must_use]
pub fn place(layer: &Layer, readings: &Readings, values: &Values) -> Placed {
    let mut placed = Placed {
        transform: layer.transform,
        opacity: layer.opacity,
        visible: layer.visible,
    };
    let Some(animations) = &layer.animations else {
        return placed;
    };
    let (mut x, mut y, mut sx, mut sy) = (0.0, 0.0, 1.0, 1.0);
    let mut moved = false;
    for a in animations.iter() {
        let Some(input) = a.input.read(readings, values) else {
            continue;
        };
        let v = a.map.apply(input);
        match a.property {
            Property::X => (x, moved) = (v, true),
            Property::Y => (y, moved) = (v, true),
            Property::ScaleX => (sx, moved) = (v, true),
            Property::ScaleY => (sy, moved) = (v, true),
            Property::Opacity => placed.opacity *= narrow(v.clamp(0.0, 1.0)),
            Property::Visible => placed.visible &= v >= 0.5,
        }
    }
    if moved {
        placed.transform =
            layer.transform * Affine::translate((x, y)) * Affine::scale_non_uniform(sx, sy);
    }
    placed
}

#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// When what an animation draws next changes on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Next {
    /// Every frame: a line on a running clock.
    Every,
    /// In this many seconds of wall time: the next step on a running
    /// clock, or a clamp the line reaches.
    In(f64),
    /// Not until something else changes: a still clock, a value, a line
    /// held at its clamp.
    Never,
}

impl Next {
    /// The sooner of the two.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Every, _) | (_, Self::Every) => Self::Every,
            (Self::In(a), Self::In(b)) => Self::In(a.min(b)),
            (Self::In(a), Self::Never) | (Self::Never, Self::In(a)) => Self::In(a),
            (Self::Never, Self::Never) => Self::Never,
        }
    }
}

impl Linear {
    // The input to output slope times the clock's rate.
    fn next(&self, x: f64, rate: f64) -> Next {
        let slope = self.scale * rate;
        if slope == 0.0 {
            return Next::Never;
        }
        if self.wrap.is_some() {
            return Next::Every;
        }
        let y = self.scale.mul_add(x, self.offset);
        // Held at a clamp it moves further into: still. Short of a clamp
        // it moves away from... it moves now.
        match (self.min, self.max) {
            (_, Some(max)) if y >= max && slope > 0.0 => Next::Never,
            (Some(min), _) if y <= min && slope < 0.0 => Next::Never,
            (_, Some(max)) if y > max => Next::In((max - y) / slope),
            (Some(min), _) if y < min => Next::In((min - y) / slope),
            _ => Next::Every,
        }
    }
}

impl Steps {
    fn next(&self, x: f64, rate: f64) -> Next {
        let crossing = if rate > 0.0 {
            // The first step past where it is.
            self.steps
                .iter()
                .find(|(from, _)| *from > x)
                .map(|(from, _)| (from - x) / rate)
        } else {
            // Going back: when it drops below the step it is on.
            self.steps
                .iter()
                .rev()
                .find(|(from, _)| *from <= x)
                .map(|(from, _)| (x - from) / -rate)
        };
        crossing.map_or(Next::Never, Next::In)
    }
}

impl Animation {
    /// When what it draws next changes on its own, at `readings`.
    #[must_use]
    pub fn next(&self, readings: &Readings) -> Next {
        let Some(r) = self.input.reading(readings) else {
            return Next::Never;
        };
        if !r.running() {
            return Next::Never;
        }
        match &self.map {
            Map::Linear(l) => l.next(r.time, r.rate),
            Map::Steps(s) => s.next(r.time, r.rate),
        }
    }

    /// What it sets its property to at these clocks, if it reads one.
    fn output(&self, readings: &Readings) -> Option<f64> {
        match &self.input {
            Input::Clock { .. } => self.input.reading(readings).map(|r| self.map.apply(r.time)),
            Input::Value(_) => None,
        }
    }
}

/// When what `animations` draw next changes on its own.
#[must_use]
pub fn next(animations: &[Animation], readings: &Readings) -> Next {
    animations
        .iter()
        .fold(Next::Never, |n, a| n.and(a.next(readings)))
}

/// Whether any of `animations` reads a clock that moves every frame: the
/// host draws every frame while one does.
#[must_use]
pub fn moving(animations: &[Animation], readings: &Readings) -> bool {
    next(animations, readings) == Next::Every
}

/// Whether any of `animations` on a clock draws something else at `now`
/// than at `drawn`: a frame of playback, a step reached, a seek past a
/// step. A seek that changes nothing drawn is not.
#[must_use]
pub fn differs(animations: &[Animation], drawn: &Readings, now: &Readings) -> bool {
    animations
        .iter()
        .any(|a| a.output(drawn).map(f64::to_bits) != a.output(now).map(f64::to_bits))
}

// The bytes of an `ops::ANIMATE` and an `ops::VALUE`.

const LINEAR: u8 = 0;
const STEPS: u8 = 1;
const FROM_CLOCK: u8 = 0;
const FROM_VALUE: u8 = 1;
const NO_CLOCK: u8 = 0xff;
const WRAP: u8 = 1;
const MIN: u8 = 2;
const MAX: u8 = 4;

fn index_of<T: PartialEq>(all: &[T], item: &T) -> u8 {
    all.iter()
        .position(|x| x == item)
        .and_then(|i| u8::try_from(i).ok())
        .unwrap_or(u8::MAX)
}

fn clock_byte(clock: Option<Clock>) -> u8 {
    clock.map_or(NO_CLOCK, |c| index_of(&CLOCKS, &c))
}

pub(crate) fn write_name(w: &mut Writer, name: &Name) {
    let bytes = name.as_str().as_bytes();
    w.u8(u8::try_from(bytes.len()).unwrap_or(u8::MAX));
    w.bytes(bytes);
}

pub(crate) fn read_name(r: &mut Reader<'_>) -> Result<Name, Error> {
    let len = usize::from(r.u8()?);
    if len > MAX_NAME {
        return Err(Error::BadName(String::new()));
    }
    let bytes = r.take(len)?;
    let text = std::str::from_utf8(bytes).map_err(|_| Error::BadName(String::new()))?;
    Name::new(text)
}

/// `animations` as bytes: a count, then each.
pub(crate) fn write(w: &mut Writer, animations: &[Animation]) {
    w.u8(u8::try_from(animations.len()).unwrap_or(u8::MAX));
    for a in animations {
        w.u8(index_of(&PROPERTIES, &a.property));
        match &a.input {
            Input::Clock { clock, fallback } => {
                w.u8(FROM_CLOCK);
                w.u8(clock_byte(Some(*clock)));
                w.u8(clock_byte(*fallback));
            }
            Input::Value(name) => {
                w.u8(FROM_VALUE);
                write_name(w, name);
            }
        }
        match &a.map {
            Map::Linear(l) => {
                w.u8(LINEAR);
                w.f64(l.scale);
                w.f64(l.offset);
                let flags = [(l.wrap, WRAP), (l.min, MIN), (l.max, MAX)]
                    .into_iter()
                    .filter(|(v, _)| v.is_some())
                    .fold(0, |acc, (_, bit)| acc | bit);
                w.u8(flags);
                for v in [l.wrap, l.min, l.max].into_iter().flatten() {
                    w.f64(v);
                }
            }
            Map::Steps(s) => {
                w.u8(STEPS);
                w.f64(s.before);
                w.u16(u16::try_from(s.steps.len()).unwrap_or(u16::MAX));
                for (from, v) in &s.steps {
                    w.f64(*from);
                    w.f64(*v);
                }
            }
        }
    }
}

fn read_clock(r: &mut Reader<'_>) -> Result<Option<Clock>, Error> {
    match r.u8()? {
        NO_CLOCK => Ok(None),
        n => CLOCKS
            .get(usize::from(n))
            .copied()
            .map(Some)
            .ok_or(Error::Unknown("clock", n)),
    }
}

/// Animations from bytes, checked ([`check`]).
pub(crate) fn read(r: &mut Reader<'_>, access: Access) -> Result<Vec<Animation>, Error> {
    let n = usize::from(r.u8()?);
    if n > PROPERTIES.len() {
        return Err(Error::BadAnimation("more animations than properties"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let p = r.u8()?;
        let property = PROPERTIES
            .get(usize::from(p))
            .copied()
            .ok_or(Error::Unknown("property", p))?;
        let input = match r.u8()? {
            FROM_CLOCK => {
                let clock = read_clock(r)?.ok_or(Error::Unknown("clock", NO_CLOCK))?;
                Input::Clock {
                    clock,
                    fallback: read_clock(r)?,
                }
            }
            FROM_VALUE => Input::Value(read_name(r)?),
            other => return Err(Error::Unknown("input", other)),
        };
        let map = match r.u8()? {
            LINEAR => {
                let (scale, offset) = (r.f64()?, r.f64()?);
                let flags = r.u8()?;
                if flags & !(WRAP | MIN | MAX) != 0 {
                    return Err(Error::Unknown("linear", flags));
                }
                let mut maybe = |bit: u8| -> Result<Option<f64>, Error> {
                    if flags & bit == 0 {
                        Ok(None)
                    } else {
                        r.f64().map(Some)
                    }
                };
                let (wrap, min, max) = (maybe(WRAP)?, maybe(MIN)?, maybe(MAX)?);
                Map::Linear(Linear {
                    scale,
                    offset,
                    wrap,
                    min,
                    max,
                })
            }
            STEPS => {
                let before = r.f64()?;
                let count = usize::from(r.u16()?);
                if count > MAX_STEPS {
                    return Err(Error::BadAnimation("too many steps"));
                }
                let mut steps = Vec::with_capacity(count);
                for _ in 0..count {
                    steps.push((r.f64()?, r.f64()?));
                }
                Map::Steps(Steps { before, steps })
            }
            other => return Err(Error::Unknown("map", other)),
        };
        out.push(Animation {
            property,
            input,
            map,
        });
    }
    check(&out, access)?;
    Ok(out)
}
