//! The deck: every source on one app, the one that is active, and what
//! may move each (#589's contract).
//!
//! Pure: time comes in as `now` (ms since the epoch), so it is tested
//! without a clock and runs the same on a desktop, a phone and in a
//! browser. The app keeps one ([`crate::deck`]); a clock that is heard (the
//! web app's audio element, later an engine) follows it and reports back
//! ([`crate::clock`]).

use serde_json::{Value, json};

use crate::model::{Item, Spec};

/// Who asks: the app itself (its bar, its Action List, the person), or
/// an extension, named by the host, never by its own arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The app: acts on the source it names, else the active one.
    App,
    /// Extension `id`: acts on its own source only.
    Extension(String),
}

/// The invocation sources that are the app's own (a person's press in
/// its UI, a key, ⌘K, a menu, an agent over MCP acting as the person):
/// only these, with no extension named, act as [`Caller::App`].
pub const APP_SOURCES: &[&str] = &[
    "app",
    "palette",
    "menu",
    "context_menu",
    "key",
    "agent",
    "toolbar",
];

/// Whether `id` is a source only the app may hold: `vault`, `app`,
/// `playhead`, or anything outside the extension-id grammar (the app's
/// own sources are `app:<name>`, which no extension id can be).
#[must_use]
pub fn reserved(id: &str) -> bool {
    matches!(id, "vault" | "app" | "playhead")
        || id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

impl Caller {
    /// The caller an action's invocation context names, failing closed.
    ///
    /// `caller` (which the host adds from the call's principal) is an
    /// extension. With none, the app only when the invocation is the
    /// app's own ([`APP_SOURCES`]); a hook, a script, or a context that
    /// says nothing is refused (#660 H2).
    ///
    /// # Errors
    ///
    /// Who called is not known to be the app or an extension.
    pub fn of_context(cx: &Value) -> Result<Self, String> {
        if let Some(id) = cx
            .get("caller")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Ok(Self::Extension(id.to_owned()));
        }
        match cx.get("source").and_then(Value::as_str) {
            Some(s) if APP_SOURCES.contains(&s) => Ok(Self::App),
            Some(s) => Err(format!(
                "a {s} run names no extension: the playhead does not act for it"
            )),
            None => Err("who called is not known: the playhead does not act for it".to_owned()),
        }
    }
}

/// What may be asked of the deck.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// Set, replace or (an empty queue) remove the caller's source.
    Set(Spec),
    /// Play (the source becomes the active one).
    Play,
    /// Pause.
    Pause,
    /// To `t` seconds in the current item.
    Seek(f64),
    /// The next item.
    Next,
    /// The current item's start, or (near its start) the previous item.
    Previous,
    /// Item `index` (or the one with that `id`), from its start.
    Item(Pick, bool),
    /// The app only: show this source in the bar.
    Activate,
}

/// An item, by place or by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Its place in the queue.
    Index(usize),
    /// Its id.
    Id(String),
}

/// Before this many seconds in, "previous" goes to the previous item.
pub const RESTART_AFTER: f64 = 3.0;

/// Where a source is: `time` seconds at `at` (ms since the epoch),
/// playing or not, at `rate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    /// Seconds into the current item at `at`.
    pub time: f64,
    /// When `time` was read, ms since the epoch.
    pub at: f64,
    /// Whether it moves.
    pub playing: bool,
    /// How fast.
    pub rate: f64,
}

impl Transport {
    const fn still(time: f64, at: f64) -> Self {
        Self {
            time,
            at,
            playing: false,
            rate: 1.0,
        }
    }

    /// Where it is at `now`, never past `duration` (when known).
    #[must_use]
    pub fn at(&self, now: f64, duration: f64) -> f64 {
        let t = if self.playing {
            ((now - self.at) / 1000.0).mul_add(self.rate, self.time)
        } else {
            self.time
        };
        if duration > 0.0 {
            t.clamp(0.0, duration)
        } else {
            t.max(0.0)
        }
    }
}

/// One extension's queue on the deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// Its extension's id.
    pub id: String,
    /// Its name in the bar.
    pub name: String,
    /// Its items.
    pub queue: Vec<Item>,
    /// The current one.
    pub index: usize,
    /// Where it is.
    pub transport: Transport,
    /// The current item's length as its media says, once known.
    pub media_duration: Option<f64>,
    /// Moves on with each jump (a seek, another item): a clock that
    /// follows the deck seeks when it changes.
    pub epoch: u64,
    /// When it was last made active (the deck's count).
    active_seq: u64,
    /// Items it skipped or stopped at: media nobody can hear here and no
    /// known length (#660 M4). Cleared when its queue is set again.
    pub unplayable: Vec<String>,
}

impl Source {
    /// Its current item.
    #[must_use]
    pub fn item(&self) -> Option<&Item> {
        self.queue.get(self.index)
    }

    /// Whether its current item has media but no length known: it can be
    /// timed only by a clock that hears it.
    #[must_use]
    pub fn length_unknown(&self) -> bool {
        self.duration() <= 0.0
    }

    /// The current item's length: its media's, else as given.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.media_duration
            .or_else(|| self.item().and_then(|i| i.duration))
            .unwrap_or(0.0)
    }

    /// Where it is at `now`.
    #[must_use]
    pub fn time(&self, now: f64) -> f64 {
        self.transport.at(now, self.duration())
    }

    fn halt(&mut self, now: f64) {
        let t = self.time(now);
        self.transport = Transport::still(t, now);
    }

    const fn jump(&mut self, index: usize, time: f64, now: f64) {
        if index != self.index {
            self.media_duration = None;
        }
        self.index = index;
        self.transport = Transport {
            time,
            at: now,
            ..self.transport
        };
        self.epoch = self.epoch.wrapping_add(1);
    }
}

/// Every source on one app, and the active one.
#[derive(Debug, Clone, Default)]
pub struct Deck {
    sources: Vec<Source>,
    active: Option<String>,
    seq: u64,
    revision: u64,
    /// Whether this app has a clock that is heard (the web app's audio
    /// element): an item with media then plays on it (`clock: "media"`).
    pub audible: bool,
}

impl Deck {
    /// An empty deck.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves on with every change.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// The active source's id.
    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Every source, in the order they came.
    #[must_use]
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Source `id`.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    fn source_mut(&mut self, id: &str) -> Option<&mut Source> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    /// The active source.
    #[must_use]
    pub fn current(&self) -> Option<&Source> {
        self.active.as_deref().and_then(|a| self.source(a))
    }

    /// Whom `caller` acts on, naming `source` or not.
    ///
    /// # Errors
    ///
    /// An extension naming another's source; nothing to act on.
    pub fn target(&self, caller: &Caller, source: Option<&str>) -> Result<String, String> {
        match caller {
            Caller::Extension(me) if reserved(me) => Err("not your source".to_owned()),
            Caller::Extension(me) => match source {
                Some(s) if s != me => Err("not your source".to_owned()),
                _ => Ok(me.clone()),
            },
            Caller::App => source
                .map(str::to_owned)
                .or_else(|| self.active.clone())
                .ok_or_else(|| "no source".to_owned()),
        }
    }

    /// Does `control` for `caller` (on `source`, when it names one) at
    /// `now`: the id of the source it acted on.
    ///
    /// # Errors
    ///
    /// The contract's refusals: `not your source`, `only the app
    /// activates a source`, `no source`, `t is outside 0..=duration`,
    /// `no item …`; a `SET` without `source` from the app.
    pub fn apply(
        &mut self,
        caller: &Caller,
        control: Control,
        source: Option<&str>,
        now: f64,
    ) -> Result<String, String> {
        if matches!(control, Control::Activate) && *caller != Caller::App {
            return Err("only the app activates a source".to_owned());
        }
        if matches!(control, Control::Set(_)) && *caller == Caller::App && source.is_none() {
            return Err("give `source`: whose queue this is".to_owned());
        }
        let id = self.target(caller, source)?;
        if let Control::Set(spec) = control {
            self.set(&id, spec, now);
            self.revision = self.revision.wrapping_add(1);
            return Ok(id);
        }
        if self.source(&id).is_none() {
            return Err("no source".to_owned());
        }
        match control {
            Control::Play => self.play(&id, now),
            Control::Pause => {
                if let Some(s) = self.source_mut(&id) {
                    s.halt(now);
                }
            }
            Control::Seek(t) => {
                let s = self.source_mut(&id).ok_or("no source")?;
                let d = s.duration();
                if d <= 0.0 {
                    return Err("its length is not known yet".to_owned());
                }
                if !(t.is_finite() && t >= 0.0) {
                    return Err("t is outside 0..=duration".to_owned());
                }
                let i = s.index;
                s.jump(i, t.min(d), now);
            }
            Control::Next => {
                let s = self.source_mut(&id).ok_or("no source")?;
                if s.index.saturating_add(1) < s.queue.len() {
                    s.jump(s.index.saturating_add(1), 0.0, now);
                } else {
                    let d = s.duration();
                    s.transport = Transport::still(d, now);
                    s.epoch = s.epoch.wrapping_add(1);
                }
            }
            Control::Previous => {
                let s = self.source_mut(&id).ok_or("no source")?;
                let to = if s.time(now) > RESTART_AFTER || s.index == 0 {
                    s.index
                } else {
                    s.index.saturating_sub(1)
                };
                s.jump(to, 0.0, now);
            }
            Control::Item(pick, play) => {
                let s = self.source_mut(&id).ok_or("no source")?;
                let to = match &pick {
                    Pick::Index(n) => (*n < s.queue.len()).then_some(*n),
                    Pick::Id(item) => s.queue.iter().position(|i| &i.id == item),
                }
                .ok_or_else(|| match pick {
                    Pick::Index(n) => format!("no item {n}"),
                    Pick::Id(item) => format!("no item {item}"),
                })?;
                s.jump(to, 0.0, now);
                if play {
                    self.play(&id, now);
                }
            }
            Control::Activate => {
                if self.active.as_deref() != Some(&id) {
                    if let Some(old) = self.active.clone()
                        && let Some(s) = self.source_mut(&old)
                    {
                        s.halt(now);
                    }
                    self.make_active(&id);
                }
            }
            Control::Set(_) => {}
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(id)
    }

    fn make_active(&mut self, id: &str) {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        if let Some(s) = self.source_mut(id) {
            s.active_seq = seq;
        }
        self.active = Some(id.to_owned());
    }

    fn play(&mut self, id: &str, now: f64) {
        if self.active.as_deref() != Some(id) {
            if let Some(old) = self.active.clone()
                && let Some(s) = self.source_mut(&old)
            {
                s.halt(now);
            }
            self.make_active(id);
        }
        if let Some(s) = self.source_mut(id) {
            let d = s.duration();
            let mut t = s.time(now);
            if d > 0.0 && t >= d {
                t = 0.0;
                s.epoch = s.epoch.wrapping_add(1);
            }
            s.transport = Transport {
                time: t,
                at: now,
                playing: true,
                rate: s.transport.rate,
            };
        }
    }

    fn set(&mut self, id: &str, spec: Spec, now: f64) {
        if spec.queue.is_empty() {
            self.sources.retain(|s| s.id != id);
            if self.active.as_deref() == Some(id) {
                self.active = None;
                // The most recently active of the rest, paused.
                let next = self
                    .sources
                    .iter()
                    .max_by_key(|s| s.active_seq)
                    .map(|s| s.id.clone());
                if let Some(n) = next {
                    if let Some(s) = self.source_mut(&n) {
                        s.halt(now);
                    }
                    self.make_active(&n);
                }
            }
            return;
        }
        let name = spec.name.clone().unwrap_or_else(|| id.to_owned());
        let new_current = spec.queue.get(spec.index).map(|i| i.id.clone());
        match self.source_mut(id) {
            Some(s) => {
                let carry = spec.time.is_none() && s.item().map(|i| &i.id) == new_current.as_ref();
                s.name = name;
                s.queue = spec.queue;
                s.unplayable.clear();
                if carry {
                    s.index = spec.index;
                } else {
                    s.index = spec.index;
                    s.media_duration = None;
                    s.transport = Transport::still(spec.time.unwrap_or(0.0), now);
                    s.epoch = s.epoch.wrapping_add(1);
                }
                let d = s.duration();
                if d > 0.0 && s.transport.time > d {
                    s.transport.time = d;
                }
            }
            None => self.sources.push(Source {
                id: id.to_owned(),
                name,
                queue: spec.queue,
                index: spec.index,
                transport: Transport::still(spec.time.unwrap_or(0.0), now),
                media_duration: None,
                epoch: 0,
                active_seq: 0,
                unplayable: Vec::new(),
            }),
        }
        if self.active.is_none() {
            self.make_active(id);
        }
        if spec.play {
            self.play(id, now);
        }
    }

    /// What a clock that is heard read for the active source: its time
    /// (while it is really moving), its media's length, whether it ended.
    /// Returns whether anything changed for the views.
    pub fn report(&mut self, reading: &crate::clock::Reading, now: f64) -> bool {
        let Some(id) = self.active.clone() else {
            return false;
        };
        // Not heard: the deck's own clock keeps the time and the ends; an
        // item it cannot time is passed over.
        let heard = !reading.failed;
        let flipped = self.audible != heard;
        self.audible = heard;
        if !heard {
            let passed = self.pass_unplayable(&id, now);
            if flipped && !passed {
                self.revision = self.revision.wrapping_add(1);
            }
            return flipped || passed;
        }
        let Some(s) = self.source_mut(&id) else {
            return flipped;
        };
        // A length only from media the app resolved for this item: never
        // for a path it would not stream (#589's media requirement).
        let length = reading
            .duration
            .filter(|_| reading.resolved)
            .filter(|d| d.is_finite() && *d > 0.0)
            .filter(|d| s.media_duration.is_none_or(|m| (m - d).abs() > 0.01));
        let changed = length.is_some();
        if length.is_some() {
            s.media_duration = length;
        }
        if reading.moving && s.transport.playing {
            s.transport.time = reading.time;
            s.transport.at = now;
        }
        if reading.ended && s.transport.playing {
            return self.ended(&id, now) || changed;
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    /// Source `id`, playing an item nobody hears here whose length is not
    /// known: marked unplayable and passed over (the next item plays), or
    /// at the last, stopped there. Whether it did.
    fn pass_unplayable(&mut self, id: &str, now: f64) -> bool {
        let Some(s) = self.source_mut(id) else {
            return false;
        };
        if !s.transport.playing || !s.length_unknown() {
            return false;
        }
        let Some(item) = s.item().map(|i| i.id.clone()) else {
            return false;
        };
        if !s.unplayable.contains(&item) {
            s.unplayable.push(item);
        }
        if s.index.saturating_add(1) < s.queue.len() {
            s.jump(s.index.saturating_add(1), 0.0, now);
            s.transport.playing = true;
        } else {
            s.transport = Transport::still(0.0, now);
            s.epoch = s.epoch.wrapping_add(1);
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn ended(&mut self, id: &str, now: f64) -> bool {
        let Some(s) = self.source_mut(id) else {
            return false;
        };
        if s.index.saturating_add(1) < s.queue.len() {
            s.jump(s.index.saturating_add(1), 0.0, now);
            s.transport.playing = true;
        } else {
            let d = s.duration();
            s.transport = Transport::still(d, now);
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// At `now`: the active source, playing on a clock nobody hears, past
    /// its item's end goes on to the next (or stops at the last). Whether
    /// it changed.
    pub fn tick(&mut self, now: f64) -> bool {
        let Some(id) = self.active.clone() else {
            return false;
        };
        let audible = self.audible;
        let Some(s) = self.source(&id) else {
            return false;
        };
        let heard = audible && s.item().is_some_and(|i| i.media.is_some());
        if heard || !s.transport.playing {
            return false;
        }
        // Nobody hears it and its length is not known: it never ends on
        // its own, so it is passed over (#660 M4).
        if s.length_unknown() {
            return self.pass_unplayable(&id, now);
        }
        if s.time(now) < s.duration() {
            return false;
        }
        self.ended(&id, now)
    }

    /// What clock source `s` plays on.
    fn clock_of(&self, s: &Source) -> &'static str {
        let heard = self.audible
            && self.active.as_deref() == Some(s.id.as_str())
            && s.item().is_some_and(|i| i.media.is_some());
        if heard { "media" } else { "silent" }
    }

    /// Source `id`'s state, the object every view gets (#589).
    #[must_use]
    pub fn state(&self, id: &str, now: f64) -> Option<Value> {
        let s = self.source(id)?;
        let item = s.item();
        let time = s.time(now);
        let clock = self.clock_of(s);
        // Media nobody hears here: the bar says so rather than "playing".
        let unheard = clock == "silent" && item.is_some_and(|i| i.media.is_some());
        Some(json!({
            "source": s.id,
            "name": s.name,
            "active": self.active.as_deref() == Some(id),
            "playing": s.transport.playing,
            "time": time,
            "at": now,
            "rate": s.transport.rate,
            "duration": s.duration(),
            "index": s.index,
            "tracks": s.queue.len(),
            "clock": clock,
            "unheard": unheard,
            "unplayable": s.unplayable,
            "item": item,
            "queue": s.queue,
            "title": item.map(|i| i.title.clone()),
            "subtitle": item.and_then(|i| i.subtitle.clone()),
            "artist": item.and_then(|i| i.subtitle.clone()),
            "sections": item.map_or_else(Vec::new, |i| i.sections.clone()),
        }))
    }

    /// What `PLAYHEAD_SOURCES` answers `caller`: for the app, every source
    /// (the bar's switcher, [`Self::listing`]); for an extension, its own
    /// entry, if it has one, and whether it is the active one, nothing of
    /// any other source (#660 M1).
    #[must_use]
    pub fn listing_for(&self, caller: &Caller) -> Value {
        match caller {
            Caller::App => self.listing(),
            Caller::Extension(me) => {
                let mine = |s: &&Source| s.id == *me && !reserved(me);
                json!({
                    "active": self.active.as_deref() == Some(me.as_str()) && !reserved(me),
                    "sources": self.sources.iter().filter(mine).map(Self::entry).collect::<Vec<_>>(),
                })
            }
        }
    }

    fn entry(s: &Source) -> Value {
        json!({
            "source": s.id,
            "name": s.name,
            "playing": s.transport.playing,
            "title": s.item().map(|i| i.title.clone()),
            "subtitle": s.item().and_then(|i| i.subtitle.clone()),
            "index": s.index,
            "tracks": s.queue.len(),
        })
    }

    /// Every source, for the bar's switcher (the app's `PLAYHEAD_SOURCES`).
    #[must_use]
    pub fn listing(&self) -> Value {
        json!({
            "active": self.active,
            "sources": self.sources.iter().map(|s| json!({
                "source": s.id,
                "name": s.name,
                "playing": s.transport.playing,
                "title": s.item().map(|i| i.title.clone()),
                "subtitle": s.item().and_then(|i| i.subtitle.clone()),
                "index": s.index,
                "tracks": s.queue.len(),
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::clock::Reading;

    fn spec(v: &Value) -> Control {
        Control::Set(Spec::parse(v).unwrap())
    }

    fn two() -> Value {
        json!({"name": "Studio", "queue": [
            {"id": "a", "title": "A", "subtitle": "Newton", "duration": 100},
            {"id": "b", "title": "B", "duration": 50}
        ]})
    }

    fn ext(id: &str) -> Caller {
        Caller::Extension(id.to_owned())
    }

    #[test]
    fn a_source_plays_seeks_and_moves_through_its_queue() {
        let mut d = Deck::new();
        d.apply(&ext("studio"), spec(&two()), None, 0.0).unwrap();
        assert_eq!(d.active(), Some("studio"), "the first source is active");
        d.apply(&ext("studio"), Control::Play, None, 0.0).unwrap();
        let s = d.state("studio", 10_000.0).unwrap();
        assert_eq!(s["time"], 10.0);
        assert_eq!(s["playing"], true);
        assert_eq!(s["title"], "A");
        assert_eq!(s["artist"], "Newton");
        assert_eq!(s["tracks"], 2);
        assert_eq!(s["clock"], "silent");
        d.apply(&ext("studio"), Control::Seek(40.0), None, 10_000.0)
            .unwrap();
        assert_eq!(d.state("studio", 11_000.0).unwrap()["time"], 41.0);
        // Past the end: clamped to it; before 0, or not a number: refused.
        d.apply(&ext("studio"), Control::Seek(101.0), None, 0.0)
            .unwrap();
        assert_eq!(d.state("studio", 0.0).unwrap()["time"], 100.0);
        for t in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                d.apply(&ext("studio"), Control::Seek(t), None, 0.0)
                    .unwrap_err(),
                "t is outside 0..=duration"
            );
        }
        // Previous, more than 3 s in: this item's start.
        d.apply(&ext("studio"), Control::Previous, None, 11_000.0)
            .unwrap();
        assert_eq!(d.source("studio").unwrap().index, 0);
        assert_eq!(d.state("studio", 11_000.0).unwrap()["time"], 0.0);
        d.apply(&ext("studio"), Control::Next, None, 11_000.0)
            .unwrap();
        assert_eq!(d.state("studio", 11_000.0).unwrap()["title"], "B");
        // Previous near the start: the previous item.
        d.apply(&ext("studio"), Control::Previous, None, 12_000.0)
            .unwrap();
        assert_eq!(d.state("studio", 12_000.0).unwrap()["title"], "A");
        d.apply(
            &ext("studio"),
            Control::Item(Pick::Id("b".into()), true),
            None,
            12_000.0,
        )
        .unwrap();
        // Past the last item's end: it stops there.
        assert!(d.tick(70_000.0));
        let s = d.state("studio", 70_000.0).unwrap();
        assert_eq!(
            (s["playing"].clone(), s["time"].clone()),
            (json!(false), json!(50.0))
        );
        // Play at the end starts it again.
        d.apply(&ext("studio"), Control::Play, None, 70_000.0)
            .unwrap();
        assert_eq!(d.state("studio", 71_000.0).unwrap()["time"], 1.0);
        assert_eq!(
            d.apply(
                &ext("studio"),
                Control::Item(Pick::Index(5), false),
                None,
                0.0
            )
            .unwrap_err(),
            "no item 5"
        );
    }

    #[test]
    fn the_end_of_an_item_goes_on_to_the_next() {
        let mut d = Deck::new();
        d.apply(&ext("x"), spec(&two()), None, 0.0).unwrap();
        d.apply(&ext("x"), Control::Play, None, 0.0).unwrap();
        assert!(!d.tick(99_000.0));
        assert!(d.tick(100_000.0));
        let s = d.state("x", 101_000.0).unwrap();
        assert_eq!(
            (s["index"].clone(), s["playing"].clone()),
            (json!(1), json!(true))
        );
        assert_eq!(s["time"], 1.0);
    }

    #[test]
    fn an_extension_controls_only_its_own_source() {
        let mut d = Deck::new();
        d.apply(&ext("studio"), spec(&two()), None, 0.0).unwrap();
        d.apply(&ext("other"), spec(&two()), None, 0.0).unwrap();
        assert_eq!(
            d.apply(&ext("other"), Control::Play, Some("studio"), 0.0)
                .unwrap_err(),
            "not your source"
        );
        assert_eq!(
            d.apply(&ext("other"), Control::Activate, Some("other"), 0.0)
                .unwrap_err(),
            "only the app activates a source"
        );
        // Without `source`, its own; never another's.
        d.apply(&ext("other"), Control::Play, None, 0.0).unwrap();
        assert_eq!(d.active(), Some("other"));
        assert!(!d.source("studio").unwrap().transport.playing);
        // An extension with no source has nothing to move.
        assert_eq!(
            d.apply(&ext("third"), Control::Play, None, 0.0)
                .unwrap_err(),
            "no source"
        );
        // The app sets a source only by naming whose it is.
        assert!(
            d.apply(&Caller::App, spec(&two()), None, 0.0)
                .unwrap_err()
                .contains("source")
        );
    }

    #[test]
    fn two_sources_one_active_and_the_app_switches_between_them() {
        let mut d = Deck::new();
        d.apply(&ext("studio"), spec(&two()), None, 0.0).unwrap();
        d.apply(
            &ext("memo"),
            spec(&json!({"queue": [{"id": "m", "title": "Memo", "duration": 30}]})),
            None,
            0.0,
        )
        .unwrap();
        assert_eq!(d.active(), Some("studio"), "a set does not take the bar");
        d.apply(&Caller::App, Control::Play, None, 0.0).unwrap();
        assert!(d.source("studio").unwrap().transport.playing);
        // Playing another pauses the active one and takes the bar.
        d.apply(&ext("memo"), Control::Play, None, 5_000.0).unwrap();
        assert_eq!(d.active(), Some("memo"));
        let studio = d.state("studio", 9_000.0).unwrap();
        assert_eq!(
            (studio["playing"].clone(), studio["time"].clone()),
            (json!(false), json!(5.0))
        );
        // The app's pause is the active source's.
        d.apply(&Caller::App, Control::Pause, None, 6_000.0)
            .unwrap();
        assert!(!d.source("memo").unwrap().transport.playing);
        // The bar's switcher: paused, at its own place.
        d.apply(&Caller::App, Control::Activate, Some("studio"), 7_000.0)
            .unwrap();
        assert_eq!(d.active(), Some("studio"));
        assert_eq!(d.state("studio", 9_000.0).unwrap()["time"], 5.0);
        let list = d.listing();
        assert_eq!(list["active"], "studio");
        assert_eq!(list["sources"].as_array().unwrap().len(), 2);
        // The active one removed: the most recently active one takes over.
        d.apply(&ext("studio"), spec(&json!({"queue": []})), None, 8_000.0)
            .unwrap();
        assert_eq!(d.active(), Some("memo"));
        assert!(d.source("studio").is_none());
        d.apply(&ext("memo"), spec(&json!({"queue": []})), None, 8_000.0)
            .unwrap();
        assert_eq!(d.active(), None);
        assert_eq!(
            d.apply(&Caller::App, Control::Play, None, 0.0).unwrap_err(),
            "no source"
        );
    }

    #[test]
    fn a_new_queue_keeps_what_plays_when_its_item_stays_current() {
        let mut d = Deck::new();
        d.apply(&ext("s"), spec(&two()), None, 0.0).unwrap();
        d.apply(&ext("s"), Control::Play, None, 0.0).unwrap();
        let reordered = json!({"queue": [
            {"id": "b", "title": "B", "duration": 50},
            {"id": "a", "title": "A", "duration": 100}
        ], "index": 1});
        d.apply(&ext("s"), spec(&reordered), None, 10_000.0)
            .unwrap();
        let s = d.state("s", 12_000.0).unwrap();
        assert_eq!(
            (s["title"].clone(), s["time"].clone(), s["playing"].clone()),
            (json!("A"), json!(12.0), json!(true))
        );
        // Another item current: from its start, paused.
        d.apply(&ext("s"), spec(&two()), None, 12_000.0).unwrap();
        d.apply(
            &ext("s"),
            Control::Item(Pick::Index(1), false),
            None,
            12_000.0,
        )
        .unwrap();
        d.apply(
            &ext("s"),
            spec(&json!({"queue": [{"id": "c", "title": "C", "duration": 9}]})),
            None,
            13_000.0,
        )
        .unwrap();
        let s = d.state("s", 14_000.0).unwrap();
        assert_eq!(
            (s["title"].clone(), s["time"].clone(), s["playing"].clone()),
            (json!("C"), json!(0.0), json!(false))
        );
    }

    #[test]
    fn a_clock_that_is_heard_reports_its_time_its_length_and_its_end() {
        let mut d = Deck {
            audible: true,
            ..Deck::new()
        };
        d.apply(
            &ext("s"),
            spec(&json!({"queue": [
                {"id": "a", "title": "A", "media": "R/a.m4a"},
                {"id": "b", "title": "B", "media": "R/b.m4a"}
            ]})),
            None,
            0.0,
        )
        .unwrap();
        assert_eq!(d.state("s", 0.0).unwrap()["clock"], "media");
        d.apply(&ext("s"), Control::Play, None, 0.0).unwrap();
        // Its length once loaded; its time only while it really moves.
        assert!(d.report(
            &Reading {
                time: 0.0,
                duration: Some(120.0),
                moving: false,
                ended: false,
                failed: false,
                resolved: true,
            },
            500.0
        ));
        assert_eq!(d.state("s", 500.0).unwrap()["duration"], 120.0);
        d.report(
            &Reading {
                time: 2.0,
                duration: Some(120.0),
                moving: true,
                ended: false,
                failed: false,
                resolved: true,
            },
            3_000.0,
        );
        assert_eq!(d.state("s", 4_000.0).unwrap()["time"], 3.0);
        // Media it cannot hear: the deck's own clock again.
        assert!(d.report(
            &Reading {
                failed: true,
                ..Reading::default()
            },
            3_500.0
        ));
        assert_eq!(d.state("s", 3_500.0).unwrap()["clock"], "silent");
        d.report(
            &Reading {
                time: 2.5,
                duration: Some(120.0),
                moving: true,
                resolved: true,
                ..Reading::default()
            },
            3_500.0,
        );
        assert_eq!(d.state("s", 3_500.0).unwrap()["clock"], "media");
        // A heard item's end is the clock's to say, not the deck's.
        assert!(!d.tick(500_000.0));
        assert!(d.report(
            &Reading {
                time: 120.0,
                duration: Some(120.0),
                moving: false,
                ended: true,
                failed: false,
                resolved: true,
            },
            6_000.0
        ));
        let s = d.state("s", 6_000.0).unwrap();
        assert_eq!(
            (
                s["index"].clone(),
                s["playing"].clone(),
                s["duration"].clone()
            ),
            (json!(1), json!(true), json!(0.0))
        );
    }

    #[test]
    fn the_caller_comes_from_the_hosts_context() {
        assert_eq!(
            Caller::of_context(&json!({"source": "script", "caller": "studio"})),
            Ok(ext("studio"))
        );
        assert_eq!(
            Caller::of_context(&json!({"source": "app"})),
            Ok(Caller::App)
        );
        assert_eq!(
            Caller::of_context(&json!({"source": "palette"})),
            Ok(Caller::App)
        );
        // Fails closed: a hook, a script, an opener or nothing said is
        // never the app (#660 H2).
        for cx in [
            json!({"source": "hook", "event": "startup"}),
            json!({"source": "script"}),
            json!({"source": "open", "path": "a.md"}),
            json!({}),
            Value::Null,
        ] {
            assert!(Caller::of_context(&cx).is_err(), "{cx}");
        }
    }
}
