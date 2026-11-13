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
}

/// What `PLAYHEAD_SET` gives: a source's whole queue.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spec {
    /// Its name in the bar (else the source's id).
    pub name: Option<String>,
    /// Its items; none removes the source.
    pub queue: Vec<Item>,
    /// The current item.
    pub index: usize,
    /// Where in it to start, seconds (else where it was, when it is the
    /// item that was current, or its start).
    pub time: Option<f64>,
    /// Play now (and take the bar).
    pub play: bool,
}

/// Whether `path` is a path in a vault: relative, `/`-separated, no
/// scheme, no `..`, nothing empty, no `\` and no control characters.
#[must_use]
pub fn vault_path(path: &str) -> bool {
    let scheme = path
        .split_once(':')
        .is_some_and(|(head, _)| !head.contains('/'));
    !path.is_empty()
        && path.len() <= 1024
        && !scheme
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
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
            .filter(|t| t.is_finite() && *t >= 0.0)
            .map(Some)
            .ok_or_else(|| format!("{what}: {key} must be seconds (0 or more)")),
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
            || m.at < 0.0
            || m.text.chars().count() > 300
            || m.kind.as_deref().is_some_and(|k| k.chars().count() > 32)
    }) {
        return Err(format!("{what}: a mark is not one"));
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
            None | Some(Value::Null) => 0,
            Some(n) => n
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("index must be a whole number")?,
        };
        let time = seconds(v, "time", "the source")?;
        let play = v.get("play").and_then(Value::as_bool).unwrap_or(false);
        Ok(Self {
            name,
            index: index.min(queue.len().saturating_sub(1)),
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
            "Songs/Verse: one/take 2.wav",
        ] {
            assert!(vault_path(ok), "{ok}");
        }
        for bad in [
            "",
            "https://evil.test/a.mp3",
            "data:audio/wav;base64,AAAA",
            "blob:https://x/1",
            "javascript:alert(1)",
            "/etc/passwd",
            "../outside.mp3",
            "a/../../b.mp3",
            "a//b.mp3",
            "a\\b.mp3",
            "./a.mp3",
            "a/\n.mp3",
        ] {
            assert!(!vault_path(bad), "{bad:?}");
        }
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
        assert_eq!(ok.index, 1, "clamped to the queue");
        assert!(ok.play);
        assert!(Spec::parse(&json!({"queue": []})).unwrap().queue.is_empty());
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
        let many: Vec<Value> = (0..=MAX_QUEUE)
            .map(|n| json!({"id": n.to_string(), "title": "t", "duration": 1}))
            .collect();
        assert!(refused(json!({ "queue": many })).contains("at most"));
    }
}
