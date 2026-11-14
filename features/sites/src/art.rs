//! A note's art: the image its `art:` property names (`art:
//! "[[cover.jpg]]"`), shown beside it where a list shows the note (a
//! setlist's row, the player's now-playing card).
//!
//! The image is one of the note's files, as an attachment it links in its
//! body is: published when the note is, under the same rules (a scope
//! that holds the file, a layer the link plays to), and never otherwise.
//! A note outside a share's scope has no art there.

use serde_json::{Map, Value};

/// The property that names a note's art.
pub const ART_PROPERTY: &str = "art";

/// The image files art may be, by extension (what a page shows in an
/// `<img>`).
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "avif"];

/// Whether `path` is an image art may be, by its extension.
#[must_use]
pub fn is_image(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && IMAGES.iter().any(|i| i.eq_ignore_ascii_case(ext))
    })
}

/// The property a media item names its art by when it has no `art:`
/// (`architect_media::Item`'s `cover`).
pub const COVER_PROPERTY: &str = "cover";

/// The link `fields` (a note's frontmatter) give as its art, as written.
///
/// The property's text, or its list's first; `None` without one. `art:`,
/// else `cover:` (`architect_media::item::COVER`, the one precedence a
/// media item's art reads by): the first that names a vault path.
#[must_use]
pub fn art_link(fields: &Map<String, Value>) -> Option<String> {
    art_of(fields).map(|(_, link)| link)
}

/// [`art_link`], with the property it came from (`art` or `cover`).
#[must_use]
pub fn art_of(fields: &Map<String, Value>) -> Option<(&'static str, String)> {
    architect_media::item::COVER.iter().find_map(|key| {
        let text = match fields.get(*key)? {
            Value::String(s) => s.as_str(),
            Value::Array(list) => list.iter().find_map(Value::as_str)?,
            _ => return None,
        };
        let text = text.trim();
        (!text.is_empty() && architect_media::link::target(text).is_some())
            .then(|| (*key, text.to_owned()))
    })
}

/// What `link` (`[[cover.jpg]]`, `![[Art/cover.jpg|Cover]]`, a plain
/// path) points at: its target without brackets, alias or anchor. `None`
/// for a URL (art is a file of the vault, never another site's).
fn target(link: &str) -> Option<&str> {
    let inner = link
        .trim()
        .trim_start_matches('!')
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .unwrap_or_else(|| link.trim());
    let inner = inner.split(['|', '#']).next().unwrap_or_default().trim();
    let inner = inner.strip_prefix("./").unwrap_or(inner);
    if inner.is_empty() || inner.contains("://") || inner.starts_with("//") {
        return None;
    }
    Some(inner.trim_start_matches('/'))
}

/// The attachment `link` names from a note in folder `folder`.
///
/// The vault's way (`folder` is `A/B` for `A/B/C.md`): its vault path,
/// else that path under the note's folder, else a file name only one
/// attachment has. `files` are the vault's attachments. Only an image is
/// art.
pub fn resolve<'a, I>(link: &str, folder: &str, files: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: Clone,
{
    let want = target(link)?;
    if want.split('/').any(|s| s == "..") || !is_image(want) {
        return None;
    }
    let files = files.into_iter();
    let joined = (!folder.is_empty()).then(|| format!("{folder}/{want}"));
    let exact = || files.clone().find(|f| *f == want);
    let beside = || {
        joined
            .as_deref()
            .and_then(|j| files.clone().find(|f| *f == j))
    };
    // A bare name is the note's own file first (`cover.jpg` beside the
    // song, not one at the vault's root); a path is the vault's first.
    let found = if want.contains('/') {
        exact().or_else(beside)
    } else {
        beside().or_else(exact)
    };
    if found.is_some() {
        return found;
    }
    if want.contains('/') {
        return None;
    }
    let mut named = files.filter(|f| f.rsplit('/').next() == Some(want));
    match (named.next(), named.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// The folder of vault path `path` (`A/B` for `A/B/C.md`).
#[must_use]
pub fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: &[&str] = &[
        "Songs/Joy/Joy.mp3",
        "Songs/Joy/cover.jpg",
        "Songs/Noel/cover.jpg",
        "Art/sleigh.png",
        "Art/notes.pdf",
    ];

    fn art(link: &str, note: &str) -> Option<&'static str> {
        resolve(link, folder_of(note), FILES.iter().copied())
    }

    #[test]
    fn art_is_an_image_the_note_names() {
        assert_eq!(
            art("[[cover.jpg]]", "Songs/Joy/Joy.md"),
            Some("Songs/Joy/cover.jpg")
        );
        assert_eq!(
            art("![[Art/sleigh.png|Sleigh]]", "Songs/Joy/Joy.md"),
            Some("Art/sleigh.png")
        );
        assert_eq!(
            art("sleigh.png", "Songs/Joy/Joy.md"),
            Some("Art/sleigh.png")
        );
        assert_eq!(
            art("[[Songs/Noel/cover.jpg]]", "Songs/Joy/Joy.md"),
            Some("Songs/Noel/cover.jpg")
        );
        // Not an image, nowhere, ambiguous, another site, out of the vault.
        assert_eq!(art("[[Art/notes.pdf]]", "Songs/Joy/Joy.md"), None);
        assert_eq!(art("[[missing.png]]", "Songs/Joy/Joy.md"), None);
        assert_eq!(art("[[cover.jpg]]", "Setlists/Eve.md"), None);
        assert_eq!(art("https://example.com/a.png", "Songs/Joy/Joy.md"), None);
        assert_eq!(art("[[../Art/sleigh.png]]", "Songs/Joy/Joy.md"), None);
    }

    /// A bare name is the song's own cover before one at the vault's root.
    #[test]
    fn a_bare_name_is_the_notes_own_file_first() {
        let files = ["cover.jpg", "Songs/Joy/cover.jpg"];
        assert_eq!(
            resolve("[[cover.jpg]]", "Songs/Joy", files.iter().copied()),
            Some("Songs/Joy/cover.jpg")
        );
        assert_eq!(
            resolve("[[cover.jpg]]", "Songs/Noel", files.iter().copied()),
            Some("cover.jpg")
        );
    }

    #[test]
    fn the_property_as_written() {
        let mut fields = Map::new();
        assert_eq!(art_link(&fields), None);
        fields.insert("art".into(), Value::String(" [[cover.jpg]] ".into()));
        assert_eq!(art_link(&fields).as_deref(), Some("[[cover.jpg]]"));
        fields.insert("art".into(), serde_json::json!(["[[a.png]]", "[[b.png]]"]));
        assert_eq!(art_link(&fields).as_deref(), Some("[[a.png]]"));
        fields.insert("art".into(), Value::Bool(true));
        assert_eq!(art_link(&fields), None);
    }

    #[test]
    fn a_media_items_cover_is_its_art_when_it_has_no_art() {
        let mut fields = Map::new();
        fields.insert("cover".into(), Value::String("Songs/Joy/cover.png".into()));
        assert_eq!(art_link(&fields).as_deref(), Some("Songs/Joy/cover.png"));
        fields.insert("art".into(), Value::String("[[art.png]]".into()));
        assert_eq!(art_link(&fields).as_deref(), Some("[[art.png]]"));
        assert_eq!(art_of(&fields).map(|(k, _)| k), Some("art"));
        // An `art:` that names no vault file leaves `cover:` its turn.
        fields.insert(
            "art".into(),
            Value::String("https://example.com/a.png".into()),
        );
        assert_eq!(
            art_of(&fields),
            Some(("cover", "Songs/Joy/cover.png".to_owned()))
        );
    }
}
