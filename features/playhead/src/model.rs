//! What a source is made of, as an extension gives it (`PLAYHEAD_SET`).
//!
//! Its queue's items, their sections and timed marks, and the rules each
//! must meet before the deck takes it. Every limit here is the contract's
//! (#589): a `SET` that breaks one is refused whole.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most items a queue holds.
pub const MAX_QUEUE: usize = 500;
/// The most sections an item has.
pub const MAX_SECTIONS: usize = 200;
/// The most timed marks an item has.
pub const MAX_MARKS: usize = 2000;
/// The longest time the contract takes, seconds: 24 h.
///
/// An item's `duration`, a `time`, a section's `start` and `end`, a mark's
/// `at`, a tempo's `at`, and a length the media reports. Above it a `SET`
/// is refused, as other invalid values are, and a media length is not
/// taken.
pub const MAX_SECONDS: f64 = 86_400.0;
/// The slowest and fastest `bpm` a tempo has.
pub const BPM: std::ops::RangeInclusive<f64> = 20.0..=400.0;
/// The fewest and most beats a bar a tempo has.
pub const METER: std::ops::RangeInclusive<u8> = 1..=16;

/// A part of an item (a verse, a chorus): `start` to `end` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    /// Its name, at most 40 characters.
    pub name: String,
    /// Where it starts, seconds.
    pub start: f64,
    /// Where it ends, seconds.
    pub end: f64,
    /// `#rrggbb`, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// A timed mark of an item: a lyric line, a cue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// When, seconds.
    pub at: f64,
    /// What it says, at most 300 characters.
    pub text: String,
    /// What kind of mark (`line`, `cue`), at most 32 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// An item's beat: what the bar's Click sounds on.
///
/// Its first bar's downbeat at `at` seconds, `bpm` beats a minute from
/// there, `meter` beats a bar (the first of each accented). One tempo for
/// the whole item: a song whose tempo changes gives the one it starts in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tempo {
    /// Beats a minute, 20 to 400 ([`BPM`]).
    pub bpm: f64,
    /// Where its first downbeat is, seconds, 0 to [`MAX_SECONDS`].
    #[serde(default)]
    pub at: f64,
    /// Beats a bar, 1 to 16 ([`METER`]).
    #[serde(default = "four")]
    pub meter: u8,
}

const fn four() -> u8 {
    4
}

impl Tempo {
    /// The beats from `from` to `to` seconds: each beat's time, and
    /// whether it is a bar's first.
    ///
    /// Counted with a whole-number beat counter from the first downbeat,
    /// never by adding to a float: past 2^53 beats `n + 1.0 == n`, and a
    /// float count would never reach `to` (#667 H1). It always ends, and
    /// gives nothing for a tempo the contract refuses or a window that is
    /// not a number.
    pub fn beats(self, from: f64, to: f64) -> impl Iterator<Item = (f64, bool)> {
        let sane = self.bpm.is_finite()
            && self.bpm > 0.0
            && self.at.is_finite()
            && from.is_finite()
            && to.is_finite();
        let every = if sane { 60.0 / self.bpm } else { 1.0 };
        let meter = u32::from(self.meter.max(1));
        // The first whole beat at or after `from`, and the first at or
        // after `to`: the count is known before a beat is given. A `u32`
        // holds 24 h at 400 bpm many times over; the float-to-int `as`
        // saturates (and a NaN is 0), which is what is wanted here.
        #[allow(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let count = |t: f64| ((t - self.at) / every).ceil().max(0.0) as u32;
        let first = count(from);
        let end = if sane { count(to).max(first) } else { first };
        // The integer range alone bounds the beats: a cut on the beat's
        // time would drop one landing a rounding step past `to`, which the
        // next window (starting at `count(to)`) would not give again.
        (first..end).map(move |n| {
            let t = f64::from(n).mul_add(every, self.at);
            (t, n.is_multiple_of(meter))
        })
    }
}

/// What a source asks the bar to draw beside its play button (#589).
///
/// At most two, each once, in its order. The bar always draws `open`,
/// after the others when it is not asked for, so the detail stays
/// reachable from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Slot {
    /// Click on and off: a tick on each beat of the item's `tempo`, where
    /// the app can sound one.
    Click,
    /// The bar's detail: previous and next, the position, the sources.
    Open,
}

/// The most slots a source asks for.
pub const MAX_SLOTS: usize = 2;

/// One item of a queue: what plays, and what the bar and the views show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// The extension's own id for it, unique in its queue. Architect never
    /// reads anything into it.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its artist, or anything under the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// A vault path to its art.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
    /// Its length, seconds (required without `media`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// A vault path to what is heard. Never a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Its parts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
    /// Its timed marks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// Its beat, for Click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<Tempo>,
}

/// What `PLAYHEAD_SET` gives: a source's whole queue.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spec {
    /// Its name in the bar (else the source's id).
    pub name: Option<String>,
    /// Its items; none removes the source.
    pub queue: Vec<Item>,
    /// The current item; none keeps the one that is current when it is
    /// still in the queue (a view setting the same list again does not
    /// send the source back to its first item), else the first.
    pub index: Option<usize>,
    /// Where in it to start, seconds (else where it was, when it is the
    /// item that was current, or its start).
    pub time: Option<f64>,
    /// Play now (and take the bar).
    pub play: bool,
    /// What the bar draws beside its play button.
    pub slots: Vec<Slot>,
}

/// Whether `path` is a path in a vault, as `media` and `art` must be.
///
/// The one rule for both, shared by the deck and the web app's frame host
/// (`apps/web/src/ext_host.rs` re-exports it): 1 to 1024 bytes,
/// `/`-separated and relative. Refused: a scheme or drive (`:`), a
/// leading `/`, `\`, a query or fragment once it is a URL (`?`, `#`),
/// percent-encoding (`%`), control characters, invisible format
/// characters (zero-width, direction marks, the BOM, the soft hyphen),
/// slashes and dots that only look like `/` and `.` (U+FF0F, U+2215,
/// U+2044, U+FF0E, U+2024, U+FE52), and an empty, `.` or `..` segment or
/// one that ends in a dot or a space. Spaces inside a name are kept.
#[must_use]
pub fn vault_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.chars().any(refused_char)
        && path.split('/').all(|s| {
            !s.is_empty() && s != "." && s != ".." && !s.ends_with('.') && !s.ends_with(' ')
        })
}

/// A character no vault path holds: one with a meaning once the path is
/// a URL or a file name elsewhere, one nobody sees, or one that passes
/// for `/` or `.`.
const fn refused_char(c: char) -> bool {
    matches!(c, ':' | '\\' | '?' | '#' | '%')
        || c.is_control()
        // Look-alikes of `/` and `.`.
        || matches!(c, '\u{FF0F}' | '\u{2215}' | '\u{2044}' | '\u{29F8}' | '\u{FF0E}' | '\u{2024}' | '\u{FE52}')
        // Invisible format characters (Unicode's Cf, as names use them).
        || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

fn text(v: &Value, key: &str, max: usize, what: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
        Some(Value::String(_)) => Err(format!("{what}: {key} is longer than {max} characters")),
        Some(_) => Err(format!("{what}: {key} must be text")),
    }
}

fn seconds(v: &Value, key: &str, what: &str) -> Result<Option<f64>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_f64()
            .filter(|t| t.is_finite() && (0.0..=MAX_SECONDS).contains(t))
            .map(Some)
            .ok_or_else(|| format!("{what}: {key} must be seconds (0 to {MAX_SECONDS})")),
    }
}

fn color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c.chars().skip(1).all(|h| h.is_ascii_hexdigit())
}

fn item(v: &Value) -> Result<Item, String> {
    let id = text(v, "id", 256, "an item")?
        .filter(|s| !s.is_empty())
        .ok_or("an item needs an id")?;
    let what = format!("item {id}");
    let title = text(v, "title", 200, &what)?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{what} needs a title"))?;
    let subtitle = text(v, "subtitle", 200, &what)?.filter(|s| !s.is_empty());
    let art = text(v, "art", 1024, &what)?;
    let media = text(v, "media", 1024, &what)?;
    for p in art.iter().chain(media.iter()) {
        if !vault_path(p) {
            return Err(format!("{what}: media must be a vault path ({p:?} is not)"));
        }
    }
    let duration = seconds(v, "duration", &what)?.filter(|d| *d > 0.0);
    if media.is_none() && duration.is_none() {
        return Err(format!("{what}: give its duration (it has no media)"));
    }
    let sections: Vec<Section> = match v.get("sections") {
        None | Some(Value::Null) => Vec::new(),
        Some(s) => {
            serde_json::from_value(s.clone()).map_err(|e| format!("{what}: sections: {e}"))?
        }
    };
    if sections.len() > MAX_SECTIONS {
        return Err(format!("{what}: more than {MAX_SECTIONS} sections"));
    }
    for s in &sections {
        if s.name.chars().count() > 40
            || !s.start.is_finite()
            || !s.end.is_finite()
            || s.start < 0.0
            || s.end < s.start
            || s.end > MAX_SECONDS
            || s.color.as_deref().is_some_and(|c| !color(c))
        {
            return Err(format!("{what}: section {:?} is not one", s.name));
        }
    }
    let marks: Vec<Mark> = match v.get("marks") {
        None | Some(Value::Null) => Vec::new(),
        Some(m) => serde_json::from_value(m.clone()).map_err(|e| format!("{what}: marks: {e}"))?,
    };
    if marks.len() > MAX_MARKS {
        return Err(format!("{what}: more than {MAX_MARKS} marks"));
    }
    if marks.iter().any(|m| {
        !m.at.is_finite()
            || !(0.0..=MAX_SECONDS).contains(&m.at)
            || m.text.chars().count() > 300
            || m.kind.as_deref().is_some_and(|k| k.chars().count() > 32)
    }) {
        return Err(format!("{what}: a mark is not one"));
    }
    let tempo: Option<Tempo> = match v.get("tempo") {
        None | Some(Value::Null) => None,
        Some(t) => {
            Some(serde_json::from_value(t.clone()).map_err(|e| format!("{what}: tempo: {e}"))?)
        }
    };
    if tempo.is_some_and(|t| {
        !t.bpm.is_finite()
            || !BPM.contains(&t.bpm)
            || !t.at.is_finite()
            || !(0.0..=MAX_SECONDS).contains(&t.at)
            || !METER.contains(&t.meter)
    }) {
        return Err(format!(
            "{what}: tempo is not one (bpm 20 to 400, at 0 to {MAX_SECONDS}, meter 1 to 16)"
        ));
    }
    Ok(Item {
        id,
        title,
        subtitle,
        art,
        duration,
        media,
        sections,
        marks,
        tempo,
    })
}

impl Spec {
    /// A `PLAYHEAD_SET`'s arguments, checked whole.
    ///
    /// # Errors
    ///
    /// The first rule it breaks.
    pub fn parse(v: &Value) -> Result<Self, String> {
        if !v.is_object() {
            return Err("give the source: {name?, queue, index?, time?, play?}".to_owned());
        }
        let name = text(v, "name", 64, "the source")?.filter(|s| !s.is_empty());
        let queue = match v.get("queue") {
            Some(Value::Array(items)) => items,
            None | Some(Value::Null) => return Err("give its queue (empty removes it)".to_owned()),
            Some(_) => return Err("queue must be a list of items".to_owned()),
        };
        if queue.len() > MAX_QUEUE {
            return Err(format!("a queue holds at most {MAX_QUEUE} items"));
        }
        let queue = queue.iter().map(item).collect::<Result<Vec<_>, _>>()?;
        let mut ids = std::collections::BTreeSet::new();
        if let Some(dup) = queue.iter().find(|i| !ids.insert(i.id.as_str())) {
            return Err(format!("item {} is in the queue twice", dup.id));
        }
        let index = match v.get("index") {
            None | Some(Value::Null) => None,
            Some(n) => Some(
                n.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("index must be a whole number")?,
            ),
        };
        let time = seconds(v, "time", "the source")?;
        let play = v.get("play").and_then(Value::as_bool).unwrap_or(false);
        let slots: Vec<Slot> = match v.get("slots") {
            None | Some(Value::Null) => Vec::new(),
            Some(s) => serde_json::from_value(s.clone())
                .map_err(|_| "slots: a list of \"click\" and \"open\"".to_owned())?,
        };
        if slots.len() > MAX_SLOTS || matches!(slots.as_slice(), [a, b] if a == b) {
            return Err(format!("slots: at most {MAX_SLOTS}, each once"));
        }
        Ok(Self {
            slots,
            name,
            index: index.map(|i| i.min(queue.len().saturating_sub(1))),
            queue,
            time,
            play,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn media_and_art_are_vault_paths_never_urls() {
        for ok in [
            "Recordings/Amazing Grace.m4a",
            "a.mp3",
            "a",
            "Songs/Merry & Bright/01 – Joy.flac",
            "Songs/Verse one/take 2.wav",
            "Art/a b.jpg",
        ] {
            assert!(vault_path(ok), "{ok}");
        }
        for bad in [
            "",
            "https://evil.test/a.mp3",
            "data:audio/wav;base64,AAAA",
            "blob:https://x/1",
            "javascript:alert(1)",
            "C:/a.ogg",
            "Songs/Verse: one/take 2.wav",
            "/etc/passwd",
            "../outside.mp3",
            "a/../../b.mp3",
            "a//b.mp3",
            "a/",
            "a\\b.mp3",
            "./a.mp3",
            "a/./b.mp3",
            "a/\n.mp3",
            "a\u{0}.mp3",
            "a\u{7f}.mp3",
            // A query, a fragment, percent-encoding (#660 M2).
            "Recordings/%2e%2e/%2e%2e/secret.m4a",
            "Recordings%2F..%2Fsecret.m4a",
            "Recordings/a.mp3?download=1",
            "Recordings/a.mp3#t=10",
            "Media/a%20b.ogg",
            // Look-alike slashes and dots, a segment ending in a dot or a
            // space, invisible characters (#659 re-check N3).
            "Recordings\u{FF0F}..\u{FF0F}secret.m4a",
            "Recordings/\u{FF0E}\u{FF0E}/secret.m4a",
            "Recordings\u{2215}a.m4a",
            "Recordings\u{2044}a.m4a",
            "Recordings/a\u{2024}m4a",
            "Recordings./a.m4a",
            "Recordings/a.m4a.",
            "Recordings /a.m4a",
            "Recordings/a.m4a ",
            "Recordings/\u{200B}a.m4a",
            "Recordings/a\u{202E}a4m.mp3",
            "\u{FEFF}Recordings/a.m4a",
            "Recordings/a\u{00AD}b.m4a",
        ] {
            assert!(!vault_path(bad), "{bad:?}");
        }
        assert!(vault_path(&"a".repeat(1024)));
        assert!(!vault_path(&"a".repeat(1025)));
    }

    #[test]
    fn a_set_is_checked_whole() {
        let ok = Spec::parse(&json!({
            "name": "Studio",
            "queue": [
                {"id": "a", "title": "A", "media": "R/a.m4a",
                 "sections": [{"name": "Verse", "start": 0, "end": 10, "color": "#2563eb"}],
                 "marks": [{"at": 1.5, "text": "Amazing grace"}]},
                {"id": "b", "title": "B", "subtitle": "Them", "duration": 90}
            ],
            "index": 9,
            "play": true
        }))
        .unwrap();
        assert_eq!(ok.queue.len(), 2);
        assert_eq!(ok.index, Some(1), "clamped to the queue");
        assert!(ok.play);
        assert!(Spec::parse(&json!({"queue": []})).unwrap().queue.is_empty());
        let tempo = Spec::parse(&json!({"queue": [{"id": "a", "title": "A", "duration": 9,
            "tempo": {"bpm": 120, "at": 0.5, "meter": 3}}], "slots": ["click", "open"]}))
        .unwrap();
        assert_eq!(tempo.slots, [Slot::Click, Slot::Open]);
        assert_eq!(
            tempo.queue[0].tempo,
            Some(Tempo {
                bpm: 120.0,
                at: 0.5,
                meter: 3
            })
        );
        let refused = |v: Value| Spec::parse(&v).unwrap_err();
        assert!(refused(json!({})).contains("queue"));
        assert!(
            refused(json!({"queue": [{"id": "a", "title": "A", "media": "https://x/a.mp3"}]}))
                .contains("vault path")
        );
        assert!(refused(json!({"queue": [{"id": "a", "title": "A"}]})).contains("duration"));
        assert!(
            refused(json!({"queue": [
                {"id": "a", "title": "A", "duration": 1},
                {"id": "a", "title": "B", "duration": 1}
            ]}))
            .contains("twice")
        );
        assert!(
            refused(json!({"queue": [{"id": "a", "title": "", "duration": 1}]})).contains("title")
        );
        assert!(
            refused(json!({"queue": [{"id": "a", "title": "A", "duration": 1,
                "sections": [{"name": "x", "start": 5, "end": 1}]}]}))
            .contains("section")
        );
        assert!(
            refused(json!({"queue": [{"id": "a", "title": "A", "duration": 1,
                "sections": [{"name": "x", "start": 0, "end": 1, "color": "red;}"}]}]}))
            .contains("section")
        );
        assert!(
            refused(
                json!({"queue": [{"id": "a", "title": "A", "duration": 1, "tempo": {"bpm": 9000}}]})
            )
            .contains("tempo")
        );
        assert!(refused(json!({"queue": [], "slots": ["loop"]})).contains("slots"));
        assert!(refused(json!({"queue": [], "slots": ["click", "click"]})).contains("slots"));
        assert!(
            refused(json!({"queue": [], "slots": ["click", "open", "open"]})).contains("slots")
        );
        let many: Vec<Value> = (0..=MAX_QUEUE)
            .map(|n| json!({"id": n.to_string(), "title": "t", "duration": 1}))
            .collect();
        assert!(refused(json!({ "queue": many })).contains("at most"));
    }

    #[test]
    fn a_tempo_gives_its_beats_and_accents_each_bar_s_first() {
        let t = Tempo {
            bpm: 120.0,
            at: 0.5,
            meter: 3,
        };
        let beats: Vec<(f64, bool)> = t.beats(0.0, 2.6).collect();
        assert_eq!(
            beats,
            [
                (0.5, true),
                (1.0, false),
                (1.5, false),
                (2.0, true),
                (2.5, false)
            ]
        );
        let later: Vec<(f64, bool)> = t.beats(1.2, 2.1).collect();
        assert_eq!(later, [(1.5, false), (2.0, true)]);
        assert_eq!(t.beats(f64::NAN, 2.0).count(), 0);
        assert_eq!(t.beats(0.0, f64::INFINITY).count(), 0);
    }

    /// #667 H1: an extension's item 2e15 s long, playing at 1.36e15 s at
    /// 400 bpm. Past 2^53 beats a float count stood still and Click's
    /// scheduler never left its loop; the `SET` is refused now, and the
    /// beats end whatever they are asked.
    #[test]
    fn a_far_time_is_refused_and_its_beats_still_end() {
        let probe = json!({"queue": [{"id": "x", "title": "X", "duration": 2e15,
            "tempo": {"bpm": 400}}], "time": 1.36e15, "play": true});
        assert!(Spec::parse(&probe).unwrap_err().contains("seconds"));
        let at = json!({"queue": [{"id": "x", "title": "X", "duration": 60}], "time": 1.36e15});
        assert!(Spec::parse(&at).unwrap_err().contains("time"));
        for (key, item) in [
            (
                "tempo",
                json!({"id": "x", "title": "X", "duration": 9, "tempo": {"bpm": 120, "at": 1e9}}),
            ),
            (
                "mark",
                json!({"id": "x", "title": "X", "duration": 9, "marks": [{"at": 1e9, "text": "t"}]}),
            ),
            (
                "section",
                json!({"id": "x", "title": "X", "duration": 9,
                "sections": [{"name": "s", "start": 0, "end": 1e9}]}),
            ),
        ] {
            assert!(
                Spec::parse(&json!({"queue": [item]}))
                    .unwrap_err()
                    .contains(key),
                "{key}"
            );
        }
        let day = json!({"queue": [{"id": "x", "title": "X", "duration": MAX_SECONDS}], "time": MAX_SECONDS});
        assert!(Spec::parse(&day).is_ok(), "24 h is still taken");
        let t = Tempo {
            bpm: 400.0,
            at: 0.0,
            meter: 4,
        };
        let from = 1.36e15;
        let beats: Vec<(f64, bool)> = t.beats(from - 0.03, from + 0.15).collect();
        assert!(beats.len() <= 4, "{}", beats.len());
        assert!(beats.iter().all(|(b, _)| *b < from + 0.15));
        // Far past 2^53 beats, where `n + 1.0 == n`.
        assert!(t.beats(1e17, 1e17 + 1.0).count() < 10);
    }
}
