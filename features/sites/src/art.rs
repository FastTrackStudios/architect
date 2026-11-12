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

/// The link `fields` (a note's frontmatter) give as its art, as written:
/// the property's text, or its list's first; `None` without one.
#[must_use]
pub fn art_link(fields: &Map<String, Value>) -> Option<String> {
    let value = fields.get(ART_PROPERTY)?;
    let text = match value {
        Value::String(s) => s.as_str(),
        Value::Array(list) => list.iter().find_map(Value::as_str)?,
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
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
    if let Some(f) = files.clone().find(|f| *f == want) {
        return Some(f);
    }
    if let Some(f) = joined.and_then(|j| files.clone().find(|f| *f == j)) {
        return Some(f);
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
}
