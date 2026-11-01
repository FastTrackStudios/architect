//! Who is here, and where: presence in any view's coordinate space.
//!
//! The wire is a document's presence channel ([`DocPresence`], a Loro
//! `EphemeralStore`: last-writer-wins per key, expiring). This module is
//! what goes on it. Each person owns three keys, split by how often they
//! change, so the pointer (almost all of the traffic) never drags the
//! rest along with it:
//!
//! | key              | what                                            | rate              |
//! |------------------|-------------------------------------------------|-------------------|
//! | `<peer>/state`   | name, colour, what they look at, selections, caret | on change      |
//! | `<peer>/pointer` | the pointer, as a [`Place`] in a view's space   | on move, ≤ 30 a second |
//! | `<peer>/play`    | a playhead: playing, where, and when that was   | on change         |
//!
//! **A pointer is never pixels.** It is a [`Place`]: a view's coordinate
//! space by name (`session.arrange`, `chart`, `note`) and named
//! coordinates in it (`time: 12.5, track: "Kick"`), so it lands on the
//! same bar of the same track however differently two people have zoomed
//! and scrolled. A view places a pointer in its own pixels; one in a space
//! the view does not show is not drawn there.
//!
//! **Smooth:** a remote pointer is a [`PointerTrail`] of samples stamped
//! with the sender's clock, re-anchored to the receiver's on arrival and
//! drawn [`INTERPOLATION_DELAY_MS`] behind, interpolated — a 30 Hz stream
//! drawn as motion, no jumps, no lag that grows.
//!
//! Sending is [`Here`] (throttled, over any [`PresenceSink`]); receiving
//! is a [`Roster`] folded from the channel's states. Both are plain data
//! and time is a parameter (`now_ms`), so the same code runs in a test, a
//! desktop and a browser. The design is
//! `docs/content/architecture/documents.md`, "Presence and cursors".
//!
//! [`DocPresence`]: crate::sync::DocPresence

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use loro::LoroValue;

/// The part of a person's presence that changes on change: [`PeerState`].
pub const STATE: &str = "state";
/// The part that streams: a [`Place`] and the sender's time.
pub const POINTER: &str = "pointer";
/// A playhead: [`PlayState`].
pub const PLAY: &str = "play";

/// How far behind real time remote pointers are drawn, so there is almost
/// always a sample on each side of the moment being drawn.
pub const INTERPOLATION_DELAY_MS: f64 = 100.0;

/// The pointer's rate limit: thirty a second.
pub const POINTER_INTERVAL_MS: f64 = 33.0;

/// The state's: a caret moving with every keystroke goes out settled.
pub const STATE_INTERVAL_MS: f64 = 150.0;

/// The key `peer` publishes `part` under.
#[must_use]
pub fn key(peer: &str, part: &str) -> String {
    format!("{peer}/{part}")
}

/// Split a presence key into `(peer, part)`.
#[must_use]
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.rsplit_once('/')
}

// ── places ──────────────────────────────────────────────────────────────

/// One coordinate of a [`Place`]: a number (a time, a fraction of a
/// width) or a key (a track, a section, a measure).
#[derive(Debug, Clone, PartialEq)]
pub enum Coord {
    Num(f64),
    Key(String),
}

impl From<f64> for Coord {
    fn from(v: f64) -> Self {
        Self::Num(v)
    }
}

impl From<&str> for Coord {
    fn from(v: &str) -> Self {
        Self::Key(v.to_owned())
    }
}

impl From<String> for Coord {
    fn from(v: String) -> Self {
        Self::Key(v)
    }
}

/// Named coordinates in a space.
pub type Coords = BTreeMap<String, Coord>;

/// A place in a view's coordinate space: where a pointer is.
///
/// ```ignore
/// Place::new("session.arrange").with("time", 12.5).with("track", "Kick")
/// Place::new("chart").with("key", "verse-2").with("u", 0.4).with("v", 0.5)
/// ```
///
/// What the coordinates mean is the space's: the view that publishes a
/// space is the one that places it in pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Place {
    pub space: String,
    pub at: Coords,
}

impl Place {
    #[must_use]
    pub fn new(space: impl Into<String>) -> Self {
        Self {
            space: space.into(),
            at: Coords::new(),
        }
    }

    /// The same place with `name` set to `value`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<Coord>) -> Self {
        self.at.insert(name.into(), value.into());
        self
    }

    /// A numeric coordinate.
    #[must_use]
    pub fn num(&self, name: &str) -> Option<f64> {
        match self.at.get(name)? {
            Coord::Num(v) => Some(*v),
            Coord::Key(_) => None,
        }
    }

    /// A key coordinate.
    #[must_use]
    pub fn key(&self, name: &str) -> Option<&str> {
        match self.at.get(name)? {
            Coord::Key(v) => Some(v),
            Coord::Num(_) => None,
        }
    }

    /// Encoded with the sender's clock, `t_ms`, for interpolation.
    #[must_use]
    pub fn encode(&self, t_ms: f64) -> LoroValue {
        map(vec![
            ("space", LoroValue::from(self.space.as_str())),
            ("t", LoroValue::Double(t_ms)),
            ("at", encode_coords(&self.at)),
        ])
    }

    /// The place and the sender's time.
    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<(Self, f64)> {
        let f = Fields::of(v)?;
        Some((
            Self {
                space: f.string("space")?,
                at: f.0.get("at").map(decode_coords).unwrap_or_default(),
            },
            f.f64("t")?,
        ))
    }
}

/// What a person has selected in a space: things by key (tracks, items,
/// tasks), a range between two places (a time selection), or both.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Selection {
    pub space: String,
    pub keys: Vec<String>,
    pub range: Option<(Coords, Coords)>,
}

impl Selection {
    fn encode(&self) -> LoroValue {
        let (from, to) = self
            .range
            .as_ref()
            .map_or((LoroValue::Null, LoroValue::Null), |(a, b)| {
                (encode_coords(a), encode_coords(b))
            });
        map(vec![
            ("space", LoroValue::from(self.space.as_str())),
            ("keys", strings(&self.keys)),
            ("from", from),
            ("to", to),
        ])
    }

    fn decode(v: &LoroValue) -> Option<Self> {
        let f = Fields::of(v)?;
        let coords = |k: &str| match f.0.get(k) {
            Some(v @ LoroValue::Map(_)) => Some(decode_coords(v)),
            _ => None,
        };
        Some(Self {
            space: f.string("space")?,
            keys: f.strings("keys"),
            range: coords("from").zip(coords("to")),
        })
    }
}

/// A caret and its selection in one text.
///
/// Both ends are encoded Loro stable cursors
/// (`loro::cursor::Cursor::encode`): they stay on the right character
/// while others type. `text` names the text container (its container id),
/// so a caret in one note is not drawn in another. Minted and resolved by
/// [`TextBinding`](crate::text::TextBinding).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextCaret {
    pub text: String,
    pub anchor: Vec<u8>,
    pub head: Vec<u8>,
}

/// The slow-changing part of a person's presence.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeerState {
    pub name: String,
    /// `0xRRGGBB`, what their pointer, caret and selections are drawn in.
    pub color: u32,
    /// What they are looking at: a view's space, a document, a pane
    /// (the roster's "looking at").
    pub looking_at: Option<String>,
    pub selections: Vec<Selection>,
    pub caret: Option<TextCaret>,
    /// The app's own, beyond name and colour.
    pub extra: BTreeMap<String, LoroValue>,
}

impl PeerState {
    /// A person named `name`, in the colour their id gives them.
    #[must_use]
    pub fn new(peer: &str, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: color_for(peer),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn encode(&self) -> LoroValue {
        let caret = self.caret.as_ref().map_or(LoroValue::Null, |c| {
            map(vec![
                ("text", LoroValue::from(c.text.as_str())),
                ("anchor", LoroValue::from(c.anchor.clone())),
                ("head", LoroValue::from(c.head.clone())),
            ])
        });
        let extra: HashMap<String, LoroValue> = self
            .extra
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        map(vec![
            ("name", LoroValue::from(self.name.as_str())),
            ("color", LoroValue::I64(i64::from(self.color))),
            (
                "looking_at",
                self.looking_at
                    .as_deref()
                    .map_or(LoroValue::Null, LoroValue::from),
            ),
            (
                "selections",
                LoroValue::from(
                    self.selections
                        .iter()
                        .map(Selection::encode)
                        .collect::<Vec<_>>(),
                ),
            ),
            ("caret", caret),
            ("extra", LoroValue::from(extra)),
        ])
    }

    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<Self> {
        let f = Fields::of(v)?;
        let color = match f.0.get("color") {
            Some(LoroValue::I64(c)) => u32::try_from(*c).unwrap_or(0),
            _ => 0,
        };
        let selections = match f.0.get("selections") {
            Some(LoroValue::List(l)) => l.iter().filter_map(Selection::decode).collect(),
            _ => Vec::new(),
        };
        let caret = f.0.get("caret").and_then(Fields::of).and_then(|c| {
            Some(TextCaret {
                text: c.string("text")?,
                anchor: c.binary("anchor")?,
                head: c.binary("head")?,
            })
        });
        let extra = match f.0.get("extra") {
            Some(LoroValue::Map(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => BTreeMap::new(),
        };
        Some(Self {
            name: f.string("name").unwrap_or_default(),
            color,
            looking_at: f.string("looking_at"),
            selections,
            caret,
            extra,
        })
    }
}

/// A playhead, at a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayState {
    pub playing: bool,
    /// Where it was at `at_ms`, in the space's own unit (seconds for a
    /// DAW).
    pub position: f64,
    /// When `position` was true, in the clock the two sides share.
    pub at_ms: f64,
    pub rate: f64,
}

impl PlayState {
    /// Where the playhead is at `now_ms` (the shared clock): not streamed,
    /// extrapolated.
    #[must_use]
    pub fn position_at(&self, now_ms: f64) -> f64 {
        if self.playing {
            ((now_ms - self.at_ms) / 1000.0).mul_add(self.rate, self.position)
        } else {
            self.position
        }
    }

    #[must_use]
    pub fn encode(&self) -> LoroValue {
        map(vec![
            ("playing", LoroValue::Bool(self.playing)),
            ("position", LoroValue::Double(self.position)),
            ("at", LoroValue::Double(self.at_ms)),
            ("rate", LoroValue::Double(self.rate)),
        ])
    }

    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<Self> {
        let f = Fields::of(v)?;
        Some(Self {
            playing: f.bool("playing")?,
            position: f.f64("position")?,
            at_ms: f.f64("at")?,
            rate: f.f64("rate").unwrap_or(1.0),
        })
    }
}

// ── sending ─────────────────────────────────────────────────────────────

/// Rate-limits a stream of values to at most one per `interval_ms`.
///
/// Never drops the last one: a pointer that stops moving ends where it
/// stopped, not at the last sample the throttle happened to let through.
#[derive(Debug, Clone)]
pub struct Throttle<T> {
    interval_ms: f64,
    last_sent_ms: Option<f64>,
    pending: Option<T>,
}

impl<T> Throttle<T> {
    #[must_use]
    pub const fn new(interval_ms: f64) -> Self {
        Self {
            interval_ms,
            last_sent_ms: None,
            pending: None,
        }
    }

    /// Offer a value at `now_ms`. Returns it if it should go out now;
    /// otherwise it is held for [`Self::flush`].
    pub fn offer(&mut self, value: T, now_ms: f64) -> Option<T> {
        match self.last_sent_ms {
            Some(last) if now_ms - last < self.interval_ms => {
                self.pending = Some(value);
                None
            }
            _ => {
                self.last_sent_ms = Some(now_ms);
                self.pending = None;
                Some(value)
            }
        }
    }

    /// The held value, once its turn has come. Call on a timer.
    pub fn flush(&mut self, now_ms: f64) -> Option<T> {
        let due = self
            .last_sent_ms
            .is_none_or(|last| now_ms - last >= self.interval_ms);
        if due && let Some(value) = self.pending.take() {
            self.last_sent_ms = Some(now_ms);
            return Some(value);
        }
        None
    }

    /// Forget a held value (the thing it would have sent is gone).
    pub fn clear(&mut self) {
        self.pending = None;
    }
}

/// Where presence is written: a document's presence channel
/// ([`PresencePeer`](crate::sync::PresencePeer), the Dioxus
/// [`Presence`](crate::hooks::Presence) handle) or a test's map.
pub trait PresenceSink {
    fn set(&self, key: &str, value: LoroValue);
    fn delete(&self, key: &str);
}

#[cfg(feature = "vox")]
impl PresenceSink for crate::sync::PresencePeer {
    fn set(&self, key: &str, value: LoroValue) {
        Self::set(self, key, value);
    }
    fn delete(&self, key: &str) {
        Self::delete(self, key);
    }
}

/// This person's presence, written to a [`PresenceSink`]: their state
/// (on change, settled), their pointer (thirty a second, the last one
/// never dropped) and their playhead. Cloning shares the throttles.
///
/// Time is a parameter: pass the local clock's milliseconds (`clock_ms`
/// with the `vox` feature), and call [`Here::flush`] on a timer so a held
/// value goes out.
#[derive(Clone)]
pub struct Here<S> {
    sink: S,
    me: String,
    sending: Arc<Mutex<Sending>>,
}

struct Sending {
    state: PeerState,
    state_throttle: Throttle<()>,
    pointer: Throttle<Place>,
}

impl<S: PresenceSink> Here<S> {
    /// `me` is this person's presence id (per device or session — two
    /// tabs of one person are two pointers); `state` their first state,
    /// published at once.
    pub fn new(sink: S, me: impl Into<String>, state: PeerState) -> Self {
        let me = me.into();
        sink.set(&key(&me, STATE), state.encode());
        Self {
            sink,
            me,
            sending: Arc::new(Mutex::new(Sending {
                state,
                state_throttle: Throttle::new(STATE_INTERVAL_MS),
                pointer: Throttle::new(POINTER_INTERVAL_MS),
            })),
        }
    }

    /// This person's presence id.
    #[must_use]
    pub fn me(&self) -> &str {
        &self.me
    }

    /// Their state as last set.
    #[must_use]
    pub fn state(&self) -> PeerState {
        architect::lock(&self.sending).state.clone()
    }

    /// Change their state (a selection, a caret, what they look at). A
    /// change that changes nothing sends nothing.
    pub fn edit_state(&self, now_ms: f64, edit: impl FnOnce(&mut PeerState)) {
        let mut sending = architect::lock(&self.sending);
        let before = sending.state.clone();
        edit(&mut sending.state);
        if sending.state != before && sending.state_throttle.offer((), now_ms).is_some() {
            self.sink.set(&key(&self.me, STATE), sending.state.encode());
        }
    }

    /// The pointer moved to `place`.
    pub fn pointer(&self, place: Place, now_ms: f64) {
        let sent = architect::lock(&self.sending).pointer.offer(place, now_ms);
        if let Some(place) = sent {
            self.sink.set(&key(&self.me, POINTER), place.encode(now_ms));
        }
    }

    /// The pointer left every space (out of the window).
    pub fn no_pointer(&self) {
        architect::lock(&self.sending).pointer.clear();
        self.sink.delete(&key(&self.me, POINTER));
    }

    /// The playhead changed (play, stop, a seek). Not throttled: it
    /// changes rarely and the receiver extrapolates it.
    pub fn play(&self, play: &PlayState) {
        self.sink.set(&key(&self.me, PLAY), play.encode());
    }

    /// Publish the state again, unthrottled: the channel (re)joined and
    /// holds nothing of this person yet.
    pub fn announce(&self) {
        let state = architect::lock(&self.sending).state.encode();
        self.sink.set(&key(&self.me, STATE), state);
    }

    /// Send what a throttle held, once its turn has come.
    pub fn flush(&self, now_ms: f64) {
        let mut sending = architect::lock(&self.sending);
        if sending.state_throttle.flush(now_ms).is_some() {
            self.sink.set(&key(&self.me, STATE), sending.state.encode());
        }
        if let Some(place) = sending.pointer.flush(now_ms) {
            self.sink.set(&key(&self.me, POINTER), place.encode(now_ms));
        }
    }

    /// Withdraw everything (leaving the document, signing out).
    pub fn leave(&self) {
        for part in [STATE, POINTER, PLAY] {
            self.sink.delete(&key(&self.me, part));
        }
    }
}

/// The local clock in milliseconds (monotonic, from the first call): what
/// [`Here`] stamps pointers with and a [`Roster`] anchors them to.
#[cfg(feature = "vox")]
#[must_use]
pub fn clock_ms() -> f64 {
    static BASE: std::sync::OnceLock<architect::platform::Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(architect::platform::now)
        .elapsed()
        .as_secs_f64()
        * 1000.0
}

// ── receiving ───────────────────────────────────────────────────────────

/// A remote pointer's recent samples, for smooth drawing.
///
/// Times are the SENDER's clock. The trail anchors that clock to the
/// receiver's on arrival, so neither needs to agree with the other.
#[derive(Debug, Clone, Default)]
pub struct PointerTrail {
    samples: VecDeque<(f64, Place)>,
    /// Receiver time minus sender time, from the newest sample.
    offset_ms: f64,
}

impl PointerTrail {
    const KEEP: usize = 16;

    /// A sample sent at `sent_ms` (sender clock) arrived at `local_ms`.
    pub fn push(&mut self, place: Place, sent_ms: f64, local_ms: f64) {
        if self.samples.back().is_some_and(|(t, _)| sent_ms <= *t) {
            return; // out of order or repeated
        }
        self.offset_ms = local_ms - sent_ms;
        self.samples.push_back((sent_ms, place));
        while self.samples.len() > Self::KEEP {
            self.samples.pop_front();
        }
    }

    /// The latest place (what space it is in now).
    #[must_use]
    pub fn latest(&self) -> Option<&Place> {
        self.samples.back().map(|s| &s.1)
    }

    /// Whether the trail still moves at `local_ms`: its newest sample is
    /// not drawn yet. A view animates while any trail does.
    #[must_use]
    pub fn moving(&self, local_ms: f64) -> bool {
        self.samples
            .back()
            .is_some_and(|(t, _)| t + self.offset_ms + INTERPOLATION_DELAY_MS > local_ms)
    }

    /// The two samples around the moment drawn at `local_ms`, and how far
    /// between them it is (`0..1`); one sample alone before the first or
    /// after the last.
    fn around(&self, local_ms: f64) -> Option<(&Place, Option<(&Place, f64)>)> {
        let t = local_ms - self.offset_ms - INTERPOLATION_DELAY_MS;
        let mut before: Option<&(f64, Place)> = None;
        for sample in &self.samples {
            if sample.0 <= t {
                before = Some(sample);
                continue;
            }
            let Some(prev) = before else {
                return Some((&sample.1, None));
            };
            let k = (t - prev.0) / (sample.0 - prev.0);
            return Some((&prev.1, Some((&sample.1, k))));
        }
        before.map(|s| (&s.1, None))
    }

    /// Where to draw the pointer at `local_ms`, in THIS view's pixels:
    /// each of the two samples around the moment is placed with `place`,
    /// then the pixels are interpolated — so a pointer moving from one
    /// track to the next, or from one view to another, glides instead of
    /// jumping. `place` answers `None` for a place this view does not
    /// show; then the newer sample decides alone.
    #[must_use]
    pub fn screen_at(
        &self,
        local_ms: f64,
        place: impl Fn(&Place) -> Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        match self.around(local_ms)? {
            (only, None) => place(only),
            (a, Some((b, k))) => match (place(a), place(b)) {
                (Some(a), Some(b)) => {
                    Some(((b.0 - a.0).mul_add(k, a.0), (b.1 - a.1).mul_add(k, a.1)))
                }
                // Into a place this view cannot show: gone at once, not
                // left where it last was.
                (_, b) => b,
            },
        }
    }

    /// Where the pointer is at `local_ms`, in its space: interpolated
    /// between two samples in one space whose keys agree; otherwise the
    /// newer one (halfway between two tracks is on neither).
    #[must_use]
    pub fn at(&self, local_ms: f64) -> Option<Place> {
        match self.around(local_ms)? {
            (only, None) => Some(only.clone()),
            (a, Some((b, k))) => Some(lerp(a, b, k)),
        }
    }
}

fn lerp(a: &Place, b: &Place, k: f64) -> Place {
    let same_keys = a.space == b.space
        && b.at.iter().all(|(name, coord)| match coord {
            Coord::Key(_) => a.at.get(name) == Some(coord),
            Coord::Num(_) => true,
        });
    if !same_keys {
        return b.clone();
    }
    let at =
        b.at.iter()
            .map(|(name, coord)| {
                let mixed = match (a.at.get(name), coord) {
                    (Some(Coord::Num(start)), Coord::Num(end)) => {
                        Coord::Num((end - start).mul_add(k, *start))
                    }
                    _ => coord.clone(),
                };
                (name.clone(), mixed)
            })
            .collect();
    Place {
        space: b.space.clone(),
        at,
    }
}

/// One other person, as the UI draws them.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub state: Option<PeerState>,
    pub trail: PointerTrail,
    pub play: Option<PlayState>,
}

/// A remote pointer placed in a view's pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenPointer {
    pub peer: String,
    pub name: String,
    pub color: u32,
    pub x: f64,
    pub y: f64,
}

/// Everyone else on a presence channel, keyed by presence id: who is
/// here, what they look at, their pointers, selections, carets and
/// playheads.
#[derive(Debug, Clone, Default)]
pub struct Roster {
    pub peers: BTreeMap<String, Peer>,
}

impl Roster {
    /// Apply one presence entry that arrived at `local_ms`. `me` is this
    /// person's id, whose own entries are skipped. `None` removes the part
    /// (they left, or timed out). Keys that are not a person's three parts
    /// (a document-wide entry) are not somebody and are skipped.
    pub fn apply(&mut self, me: &str, key: &str, value: Option<&LoroValue>, local_ms: f64) {
        let Some((peer, part)) = split_key(key) else {
            return;
        };
        if peer == me || ![STATE, POINTER, PLAY].contains(&part) {
            return;
        }
        let Some(value) = value else {
            if part == STATE {
                self.peers.remove(peer);
            } else if let Some(p) = self.peers.get_mut(peer) {
                if part == POINTER {
                    p.trail = PointerTrail::default();
                } else {
                    p.play = None;
                }
            }
            return;
        };
        let entry = self.peers.entry(peer.to_owned()).or_default();
        match part {
            STATE => entry.state = PeerState::decode(value),
            POINTER => {
                if let Some((place, sent)) = Place::decode(value) {
                    entry.trail.push(place, sent, local_ms);
                }
            }
            _ => entry.play = PlayState::decode(value),
        }
    }

    /// Fold the channel's whole picture in (what
    /// [`PresencePeer::states`](crate::sync::PresencePeer::states) reads):
    /// every entry applied — a pointer sample already seen is skipped, so
    /// this is safe on every change — and everyone the picture no longer
    /// holds removed.
    pub fn sync(&mut self, me: &str, states: &HashMap<String, LoroValue>, local_ms: f64) {
        for (key, value) in states {
            self.apply(me, key, Some(value), local_ms);
        }
        let ids: Vec<String> = self.peers.keys().cloned().collect();
        for id in ids {
            for part in [STATE, POINTER, PLAY] {
                let k = key(&id, part);
                if !states.contains_key(&k) {
                    self.apply(me, &k, None, local_ms);
                }
            }
        }
        // A pointer or playhead with no state yet is not someone to draw.
        self.peers.retain(|_, p| p.state.is_some());
    }

    /// Who is here: id and state, in id order.
    pub fn here(&self) -> impl Iterator<Item = (&str, &PeerState)> {
        self.peers
            .iter()
            .filter_map(|(id, p)| p.state.as_ref().map(|s| (id.as_str(), s)))
    }

    /// Everyone's pointer in a view's pixels at `local_ms`: `place` puts a
    /// [`Place`] in the view's pixels, `None` for a space it does not show
    /// (that pointer is not drawn here).
    #[must_use]
    pub fn pointers(
        &self,
        local_ms: f64,
        place: impl Fn(&Place) -> Option<(f64, f64)>,
    ) -> Vec<ScreenPointer> {
        self.peers
            .iter()
            .filter_map(|(id, p)| {
                let state = p.state.as_ref()?;
                let (x, y) = p.trail.screen_at(local_ms, &place)?;
                Some(ScreenPointer {
                    peer: id.clone(),
                    name: state.name.clone(),
                    color: state.color,
                    x,
                    y,
                })
            })
            .collect()
    }

    /// Whether any pointer still moves at `local_ms` (keep animating).
    #[must_use]
    pub fn moving(&self, local_ms: f64) -> bool {
        self.peers.values().any(|p| p.trail.moving(local_ms))
    }

    /// Everyone's selections in `space`: id, state, selection.
    pub fn selections_in<'a>(
        &'a self,
        space: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a PeerState, &'a Selection)> + 'a {
        self.here().flat_map(move |(id, state)| {
            state
                .selections
                .iter()
                .filter(move |s| s.space == space)
                .map(move |s| (id, state, s))
        })
    }

    /// Everyone's caret in the text named `text` (a container id).
    pub fn carets_in<'a>(
        &'a self,
        text: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a PeerState, &'a TextCaret)> + 'a {
        self.here().filter_map(move |(id, state)| {
            state
                .caret
                .as_ref()
                .filter(|c| c.text == text)
                .map(|c| (id, state, c))
        })
    }
}

/// A person's colour from their id: a stable hue, bright enough to read
/// on a dark view and on a light one.
#[must_use]
pub fn color_for(peer: &str) -> u32 {
    let hash = peer.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    hsl_to_rgb(f64::from(hash % 360), 0.7, 0.55)
}

fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> u32 {
    let chroma = (1.0 - 2.0f64.mul_add(lightness, -1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector {
        s if s < 1.0 => (chroma, second, 0.0),
        s if s < 2.0 => (second, chroma, 0.0),
        s if s < 3.0 => (0.0, chroma, second),
        s if s < 4.0 => (0.0, second, chroma),
        s if s < 5.0 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = lightness - chroma / 2.0;
    let byte = |channel: f64| {
        let scaled = ((channel + base) * 255.0).round().clamp(0.0, 255.0);
        // In 0..=255 by the clamp above.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions
        )]
        let value = scaled as u32;
        value
    };
    (byte(red) << 16) | (byte(green) << 8) | byte(blue)
}

// ── codecs ──────────────────────────────────────────────────────────────

fn map(entries: Vec<(&str, LoroValue)>) -> LoroValue {
    let m: HashMap<String, LoroValue> = entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    LoroValue::from(m)
}

fn strings(v: &[String]) -> LoroValue {
    LoroValue::from(
        v.iter()
            .map(|s| LoroValue::from(s.as_str()))
            .collect::<Vec<_>>(),
    )
}

fn encode_coords(at: &Coords) -> LoroValue {
    let m: HashMap<String, LoroValue> = at
        .iter()
        .map(|(k, c)| {
            let v = match c {
                Coord::Num(n) => LoroValue::Double(*n),
                Coord::Key(s) => LoroValue::from(s.as_str()),
            };
            (k.clone(), v)
        })
        .collect();
    LoroValue::from(m)
}

fn decode_coords(v: &LoroValue) -> Coords {
    let LoroValue::Map(m) = v else {
        return Coords::new();
    };
    m.iter()
        .filter_map(|(k, v)| {
            let c = match v {
                LoroValue::Double(d) => Coord::Num(*d),
                LoroValue::I64(i) => Coord::Num(f64::from(i32::try_from(*i).ok()?)),
                LoroValue::String(s) => Coord::Key(s.to_string()),
                _ => return None,
            };
            Some((k.clone(), c))
        })
        .collect()
}

struct Fields(HashMap<String, LoroValue>);

impl Fields {
    fn of(v: &LoroValue) -> Option<Self> {
        match v {
            LoroValue::Map(m) => Some(Self(
                m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            )),
            _ => None,
        }
    }
    fn f64(&self, k: &str) -> Option<f64> {
        match self.0.get(k)? {
            LoroValue::Double(d) => Some(*d),
            LoroValue::I64(i) => i32::try_from(*i).ok().map(f64::from),
            _ => None,
        }
    }
    fn bool(&self, k: &str) -> Option<bool> {
        match self.0.get(k)? {
            LoroValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn string(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            LoroValue::String(s) => Some(s.to_string()),
            _ => None,
        }
    }
    fn binary(&self, k: &str) -> Option<Vec<u8>> {
        match self.0.get(k)? {
            LoroValue::Binary(b) => Some(b.to_vec()),
            _ => None,
        }
    }
    fn strings(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(LoroValue::List(l)) => l
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that is a map: what a presence channel's states would be.
    #[derive(Clone, Default)]
    struct MapSink(Arc<Mutex<HashMap<String, LoroValue>>>);

    impl PresenceSink for MapSink {
        fn set(&self, key: &str, value: LoroValue) {
            self.0.lock().unwrap().insert(key.to_owned(), value);
        }
        fn delete(&self, key: &str) {
            self.0.lock().unwrap().remove(key);
        }
    }

    impl MapSink {
        fn states(&self) -> HashMap<String, LoroValue> {
            self.0.lock().unwrap().clone()
        }
    }

    fn arrange(time: f64, track: &str) -> Place {
        Place::new("session.arrange")
            .with("time", time)
            .with("track", track)
            .with("y", 0.5)
    }

    #[test]
    fn a_state_survives_its_encoding() {
        let mut extra = BTreeMap::new();
        extra.insert("song".to_owned(), LoroValue::from("Hallelujah"));
        let state = PeerState {
            name: "Ann".into(),
            color: 0x12_34_56,
            looking_at: Some("session.arrange".into()),
            selections: vec![
                Selection {
                    space: "session.arrange".into(),
                    keys: vec!["Kick".into(), "Snare".into()],
                    range: Some((
                        arrange(1.0, "Kick").at,
                        Place::new("x").with("time", 4.0).at,
                    )),
                },
                Selection {
                    space: "tasks".into(),
                    keys: vec!["t1".into()],
                    range: None,
                },
            ],
            caret: Some(TextCaret {
                text: "cid:root-body:Text".into(),
                anchor: vec![1, 2, 3],
                head: vec![4, 5],
            }),
            extra,
        };
        assert_eq!(PeerState::decode(&state.encode()), Some(state));
    }

    #[test]
    fn a_place_survives_its_encoding_with_the_senders_time() {
        let place = Place::new("chart")
            .with("key", "verse-2")
            .with("u", 0.25)
            .with("v", 0.75);
        assert_eq!(Place::decode(&place.encode(42.0)), Some((place, 42.0)));
    }

    #[test]
    fn the_throttle_never_drops_the_last_value() {
        let mut t = Throttle::new(33.0);
        assert_eq!(t.offer(1, 0.0), Some(1));
        assert_eq!(t.offer(2, 10.0), None);
        assert_eq!(t.offer(3, 20.0), None);
        assert_eq!(t.flush(25.0), None, "not yet its turn");
        assert_eq!(t.flush(40.0), Some(3), "the last one, not the second");
        assert_eq!(t.flush(100.0), None);
    }

    #[test]
    fn a_trail_is_drawn_behind_and_between_samples() {
        let mut trail = PointerTrail::default();
        // The sender's clock is 1000ms behind ours.
        trail.push(arrange(0.0, "Kick"), 0.0, 1000.0);
        trail.push(arrange(10.0, "Kick"), 100.0, 1100.0);
        // At 1150 we draw the sender's 50: halfway.
        let at = trail.at(1150.0).unwrap();
        assert!((at.num("time").unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(at.key("track"), Some("Kick"));
        assert!(trail.moving(1150.0));
        assert!(!trail.moving(1300.0), "the last sample has been drawn");
        assert_eq!(trail.at(5000.0).unwrap().num("time"), Some(10.0));
    }

    #[test]
    fn across_keys_the_place_jumps_but_the_pixels_glide() {
        let mut trail = PointerTrail::default();
        trail.push(arrange(0.0, "Kick"), 0.0, 0.0);
        trail.push(arrange(0.0, "Snare"), 100.0, 100.0);
        assert_eq!(trail.at(150.0).unwrap().key("track"), Some("Snare"));
        // Kick's row at y 0, Snare's at y 100: halfway in pixels.
        let rows = |p: &Place| {
            let row = if p.key("track")? == "Kick" {
                0.0
            } else {
                100.0
            };
            Some((p.num("time")? * 10.0, row))
        };
        assert_eq!(trail.screen_at(150.0, rows), Some((0.0, 50.0)));
        // Into a space this view does not show: gone at once.
        trail.push(Place::new("mixer").with("x", 0.5), 200.0, 200.0);
        assert_eq!(trail.screen_at(250.0, rows), None);
    }

    #[test]
    fn here_and_a_roster_see_each_other() {
        let sink = MapSink::default();
        let ann = Here::new(sink.clone(), "ann", PeerState::new("ann", "Ann"));
        let bob = Here::new(sink.clone(), "bob", PeerState::new("bob", "Bob"));
        ann.pointer(arrange(3.0, "Kick"), 0.0);
        bob.pointer(Place::new("chart").with("u", 0.5).with("v", 0.5), 0.0);
        ann.edit_state(0.0, |s| {
            s.selections.push(Selection {
                space: "session.arrange".into(),
                keys: vec!["Kick".into()],
                range: None,
            });
        });

        let mut seen_by_bob = Roster::default();
        seen_by_bob.sync("bob", &sink.states(), 0.0);
        let who: Vec<&str> = seen_by_bob.here().map(|(_, s)| s.name.as_str()).collect();
        assert_eq!(who, ["Ann"], "Bob sees Ann, not himself");

        // The arrangement places only its own space: Ann's pointer is
        // drawn there, at her time and track, in this view's pixels.
        let arrangement = |p: &Place| {
            if p.space != "session.arrange" {
                return None;
            }
            Some((p.num("time")? * 100.0, p.num("y")? * 40.0))
        };
        let drawn = seen_by_bob.pointers(500.0, arrangement);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].name, "Ann");
        assert_eq!((drawn[0].x, drawn[0].y), (300.0, 20.0));
        assert_eq!(drawn[0].color, color_for("ann"));
        assert_eq!(
            seen_by_bob.selections_in("session.arrange").count(),
            1,
            "her selection is washed in the arrangement"
        );

        // Ann sees Bob's pointer only in the chart.
        let mut seen_by_ann = Roster::default();
        seen_by_ann.sync("ann", &sink.states(), 0.0);
        assert!(seen_by_ann.pointers(500.0, arrangement).is_empty());

        // Bob leaves: gone from Ann's roster on the next picture.
        bob.leave();
        seen_by_ann.sync("ann", &sink.states(), 600.0);
        assert_eq!(seen_by_ann.here().count(), 0);
    }

    #[test]
    fn a_state_change_inside_the_throttle_goes_out_on_flush() {
        let sink = MapSink::default();
        let ann = Here::new(sink.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.edit_state(0.0, |s| s.looking_at = Some("a".into()));
        ann.edit_state(10.0, |s| s.looking_at = Some("b".into()));
        let read = || {
            PeerState::decode(sink.states().get("ann/state").unwrap())
                .unwrap()
                .looking_at
        };
        assert_eq!(read().as_deref(), Some("a"));
        ann.flush(STATE_INTERVAL_MS + 1.0);
        assert_eq!(read().as_deref(), Some("b"));
    }

    #[test]
    fn colours_are_stable_per_person() {
        assert_eq!(color_for("ann"), color_for("ann"));
        assert_ne!(color_for("ann"), color_for("bob"));
    }
}
