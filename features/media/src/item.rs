//! A media item: a note that names something to play and how to show it.

use serde_json::{Map, Value, json};

use crate::link;

/// The fields a media item reads, each with its alias: what a property
/// table leaves to the item's own row.
pub const TITLE: &str = "title";
pub const SUBTITLE: [&str; 2] = ["subtitle", "artist"];
pub const DURATION: &str = "duration";
pub const MEDIA: [&str; 2] = ["media", "recording"];
pub const COVER: [&str; 2] = ["cover", "art"];

/// One media item, as its note's frontmatter gives it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    /// The note's vault path.
    pub path: String,
    /// Its `title`, else its file's name.
    pub title: String,
    /// Its `subtitle`, else its `artist` (a list joined with commas).
    pub subtitle: Option<String>,
    /// Its `duration` in seconds: a number, or `m:ss` / `h:mm:ss`.
    pub duration: Option<f64>,
    /// What it plays: its `media`, else its `recording`, as written (a
    /// vault path or a link to one); see [`Item::media_in`].
    pub media: Option<String>,
    /// Its picture: its `cover`, else its `art`, as written.
    pub cover: Option<String>,
}

impl Item {
    /// The item note `path` is, from its frontmatter `fields`.
    #[must_use]
    pub fn read(path: &str, fields: &Map<String, Value>) -> Self {
        Self {
            path: path.to_owned(),
            title: text(fields.get(TITLE)).unwrap_or_else(|| stem(path).to_owned()),
            subtitle: SUBTITLE.iter().find_map(|k| text(fields.get(*k))),
            duration: fields.get(DURATION).and_then(seconds),
            media: MEDIA
                .iter()
                .find_map(|k| text(fields.get(*k)))
                .and_then(|m| link::target(&m)),
            cover: COVER
                .iter()
                .find_map(|k| text(fields.get(*k)))
                .and_then(|c| link::target(&c)),
        }
    }

    /// Its media among `files` (the vault's, or what a link serves),
    /// resolved from its note's folder.
    pub fn media_in<'a, I>(&self, files: I) -> Option<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
        I::IntoIter: Clone,
    {
        let m = self.media.as_deref()?;
        link::resolve(m, link::folder_of(&self.path), files)
    }

    /// Its cover among `files`, resolved from its note's folder.
    pub fn cover_in<'a, I>(&self, files: I) -> Option<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
        I::IntoIter: Clone,
    {
        let c = self.cover.as_deref()?;
        link::resolve(c, link::folder_of(&self.path), files)
    }

    /// The item as the playhead takes one (`architect.playhead` v0.1:
    /// `id`, `title`, `subtitle`, `duration`, `media`, `art`), with its
    /// `media` and `art` already resolved to vault paths.
    #[must_use]
    pub fn playhead_item(&self, id: &str, media: Option<&str>, art: Option<&str>) -> Value {
        let mut v = json!({ "id": id, "title": self.title });
        if let Some(o) = v.as_object_mut() {
            if let Some(s) = &self.subtitle {
                o.insert("subtitle".to_owned(), Value::from(s.as_str()));
            }
            if let Some(d) = self.duration {
                o.insert("duration".to_owned(), Value::from(d));
            }
            if let Some(m) = media {
                o.insert("media".to_owned(), Value::from(m));
            }
            if let Some(a) = art {
                o.insert("art".to_owned(), Value::from(a));
            }
        }
        v
    }
}

/// A file's name without its folder or extension.
#[must_use]
pub fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

/// A property's value as one line of text: a string, a number, or a
/// list's strings joined with commas; `None` when empty.
fn text(v: Option<&Value>) -> Option<String> {
    let s = match v? {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Array(list) => list
            .iter()
            .filter_map(|v| v.as_str().map(str::trim))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// A length in seconds: a number, or `m:ss` / `h:mm:ss` (a fraction on
/// the seconds allowed). Never negative, never past a day.
#[must_use]
pub fn seconds(v: &Value) -> Option<f64> {
    let s = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(t) => {
            let t = t.trim();
            let mut total = 0.0_f64;
            let parts: Vec<&str> = t.split(':').collect();
            if parts.len() > 3 || parts.iter().any(|p| p.trim().is_empty()) {
                return None;
            }
            for p in parts {
                total = total.mul_add(60.0, p.trim().parse::<f64>().ok()?);
            }
            total
        }
        _ => return None,
    };
    (s.is_finite() && (0.0..=86_400.0).contains(&s)).then_some(s)
}

/// `secs` as a list shows a length: `3:06`, `1:02:03`.
#[must_use]
pub fn clock(secs: f64) -> String {
    #[allow(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let whole = secs.max(0.0).round() as u64;
    let (h, m, s) = (whole / 3600, (whole / 60) % 60, whole % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front;

    #[test]
    fn an_item_reads_its_fields_and_their_aliases() {
        let f = front::fields(
            "---\ntitle: Sleigh Ride\nartist: Tori Kelly\nkey: B\ntempo: 86\nduration: \"3:06\"\nrecording: Recordings/Sleigh Ride.mp3\ncover: \"[[cover.svg]]\"\n---\n",
        );
        let i = Item::read("Songs/Sleigh Ride/Sleigh Ride.md", &f);
        assert_eq!(i.title, "Sleigh Ride");
        assert_eq!(i.subtitle.as_deref(), Some("Tori Kelly"));
        assert_eq!(i.duration, Some(186.0));
        assert_eq!(i.media.as_deref(), Some("Recordings/Sleigh Ride.mp3"));
        assert_eq!(i.cover.as_deref(), Some("cover.svg"));
        let files = ["Recordings/Sleigh Ride.mp3", "Songs/Sleigh Ride/cover.svg"];
        assert_eq!(i.media_in(files), Some("Recordings/Sleigh Ride.mp3"));
        assert_eq!(i.cover_in(files), Some("Songs/Sleigh Ride/cover.svg"));
        // Nothing of the key or tempo: they are an extension's.
        let v = i.playhead_item("x", i.media_in(files), i.cover_in(files));
        assert_eq!(
            v,
            json!({
                "id": "x", "title": "Sleigh Ride", "subtitle": "Tori Kelly",
                "duration": 186.0, "media": "Recordings/Sleigh Ride.mp3",
                "art": "Songs/Sleigh Ride/cover.svg",
            })
        );
    }

    #[test]
    fn the_generic_names_win_over_their_aliases() {
        let f = front::fields(
            "---\nsubtitle: Live\nartist: [A, B]\nmedia: a.mp3\nrecording: b.mp3\nart: x.png\ncover: y.png\n---\n",
        );
        let i = Item::read("Notes/Take.md", &f);
        assert_eq!(i.title, "Take");
        assert_eq!(i.subtitle.as_deref(), Some("Live"));
        assert_eq!(i.media.as_deref(), Some("a.mp3"));
        assert_eq!(i.cover.as_deref(), Some("y.png"));
        let only_artist = front::fields("---\nartist: [A, B]\n---\n");
        assert_eq!(
            Item::read("x.md", &only_artist).subtitle.as_deref(),
            Some("A, B")
        );
    }

    #[test]
    fn a_length_reads_as_seconds_and_shows_as_a_clock() {
        assert_eq!(seconds(&json!(186)), Some(186.0));
        assert_eq!(seconds(&json!("3:06")), Some(186.0));
        assert_eq!(seconds(&json!("1:02:03")), Some(3723.0));
        assert_eq!(seconds(&json!("230.8")), Some(230.8));
        assert_eq!(seconds(&json!("-3")), None);
        assert_eq!(seconds(&json!("3::06")), None);
        assert_eq!(seconds(&json!("long")), None);
        assert_eq!(clock(186.0), "3:06");
        assert_eq!(clock(230.8), "3:51");
        assert_eq!(clock(3723.0), "1:02:03");
    }

    #[test]
    fn a_media_link_outside_the_vault_is_none() {
        let f = front::fields("---\nmedia: https://example.com/a.mp3\ncover: ../x.png\n---\n");
        let i = Item::read("x.md", &f);
        assert_eq!(i.media, None);
        assert_eq!(i.cover, None);
    }
}
