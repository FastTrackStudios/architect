//! The files a member's view on a note may read (#669): the images and
//! synced lyrics the note references in their own vault, and those of the
//! notes it links (a setlist's songs' art), one step away.
//!
//! A guest's come from the link's node instead (`GuestNote::files`), never
//! from here. Both reach a view only through `ext_host::note_props`, and
//! only for an extension that declares `reads = ["files"]`.
//!
//! What a note references: the targets of its `[[wikilinks]]` and
//! `![[embeds]]`, of its markdown links `[x](path)`, and its frontmatter's
//! values (`cover: Art/joy.png`, a list's items), each a file a view
//! reads (`architect_spaces::view::ViewFileKind`) at a vault path
//! (`ext_host::vault_path`). A name is found as the vault would: the path
//! itself, beside the note, within the note's vault, else the one file of
//! that name. Nothing outside `paths` (the vault's own files) is ever
//! named.

use architect_spaces::view::{ViewFileKind, within_caps};

use crate::ext_host::{ViewFile, vault_path};

/// What the vault holds, as the app reads it.
pub struct Vault<'a> {
    /// Every file's path.
    pub paths: &'a [String],
    /// A file's text.
    pub text: &'a dyn Fn(&str) -> Option<String>,
    /// A file's bytes, where the app holds them (an image's `data:` URL
    /// decoded).
    pub bytes: &'a dyn Fn(&str) -> Option<Vec<u8>>,
}

/// The files a view on the note at `note` (its text `text`) may read in
/// `vault`: its own references first, then those of the notes it links,
/// within the caps.
#[must_use]
pub fn member_files(vault: &Vault<'_>, note: &str, text: &str) -> Vec<ViewFile> {
    let mut wanted: Vec<String> = Vec::new();
    let mut linked: Vec<String> = Vec::new();
    for r in references(text) {
        let Some(found) = resolve(&r, note, vault.paths) else {
            continue;
        };
        if ViewFileKind::of(&found).is_some() {
            push_once(&mut wanted, found);
        } else if is_note(&found) && found != note {
            push_once(&mut linked, found);
        }
    }
    for other in &linked {
        let Some(t) = (vault.text)(other) else {
            continue;
        };
        for r in references(&t) {
            if let Some(found) = resolve(&r, other, vault.paths)
                && ViewFileKind::of(&found).is_some()
            {
                push_once(&mut wanted, found);
            }
        }
    }
    let read: Vec<ViewFile> = wanted
        .into_iter()
        .filter_map(|p| ViewFile::of(&p, &(vault.bytes)(&p)?).ok())
        .collect();
    within_caps(read, |f| f.path.as_str(), |f| f.size)
}

/// The bytes of a `data:` URL in base64 (`data:image/png;base64,…`), as
/// the app holds a vault's images; `None` for anything else.
#[must_use]
pub fn data_bytes(url: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
    if !head.ends_with(";base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

fn push_once(list: &mut Vec<String>, path: String) {
    if !list.contains(&path) {
        list.push(path);
    }
}

fn is_note(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".md")
}

/// What `text` (a note, frontmatter and all) names, as written: link and
/// embed targets without their `|alias` or `#heading`, and frontmatter
/// values.
#[must_use]
pub fn references(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    if text.starts_with("---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim_end() == "---" {
                break;
            }
            let value = line
                .split_once(':')
                .map_or(line, |(_, v)| v)
                .trim()
                .trim_start_matches("- ")
                .trim();
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            let value = value
                .strip_prefix("[[")
                .and_then(|v| v.strip_suffix("]]"))
                .unwrap_or(value);
            if !value.is_empty() {
                out.push(target_of(value));
            }
        }
    }
    let body: String = lines.collect::<Vec<_>>().join("\n");
    let mut rest = body.as_str();
    while let Some(at) = rest.find("[[") {
        let after = rest.get(at.saturating_add(2)..).unwrap_or_default();
        let Some(end) = after.find("]]") else {
            break;
        };
        out.push(target_of(after.get(..end).unwrap_or_default()));
        rest = after.get(end.saturating_add(2)..).unwrap_or_default();
    }
    let mut rest = body.as_str();
    while let Some(at) = rest.find("](") {
        let after = rest.get(at.saturating_add(2)..).unwrap_or_default();
        let Some(end) = after.find(')') else {
            break;
        };
        let dest = after.get(..end).unwrap_or_default().trim();
        let dest = dest.trim_start_matches('<').trim_end_matches('>');
        if !dest.contains("://") {
            out.push(target_of(dest));
        }
        rest = after.get(end.saturating_add(1)..).unwrap_or_default();
    }
    out.retain(|r| !r.is_empty());
    out
}

/// A link's target: before its `|alias` and its `#heading`.
fn target_of(link: &str) -> String {
    let t = link.split('|').next().unwrap_or_default();
    t.split('#').next().unwrap_or_default().trim().to_owned()
}

/// The vault file `name` (as written in the note at `from`) is, if any:
/// only ever one of `paths`, and only a vault path.
#[must_use]
pub fn resolve(name: &str, from: &str, paths: &[String]) -> Option<String> {
    if !vault_path(name) {
        return None;
    }
    let held = |p: &str| paths.iter().any(|x| x == p);
    let with_md = |p: &str| {
        if held(p) {
            Some(p.to_owned())
        } else {
            let md = format!("{p}.md");
            held(&md).then_some(md)
        }
    };
    let folder = from.rsplit_once('/').map(|(f, _)| f);
    let vault = from.split_once('/').map(|(v, _)| v);
    folder
        .and_then(|f| with_md(&format!("{f}/{name}")))
        .or_else(|| with_md(name))
        .or_else(|| vault.and_then(|v| with_md(&format!("{v}/{name}"))))
        .or_else(|| {
            // The one file of that name, as a bare wikilink finds it.
            if name.contains('/') {
                return None;
            }
            let mut named = paths.iter().filter(|p| {
                let base = p.rsplit('/').next().unwrap_or(p);
                base == name || base.strip_suffix(".md") == Some(name)
            });
            let one = named.next()?;
            named.next().is_none().then(|| one.clone())
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        [
            "Sets/Merry.md",
            "Songs/Joy.md",
            "Songs/Silent.md",
            "Art/joy.png",
            "Art/silent.jpg",
            "Songs/Joy.lrc",
            "Songs/Joy.mp3",
            "Private/plans.png",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn a_setlist_reads_its_own_files_and_its_songs_art() {
        let paths = paths();
        let text = |p: &str| match p {
            "Songs/Joy.md" => Some(
                "---\ntype: song\ncover: \"[[Art/joy.png]]\"\n---\n![[Joy.lrc]] ![[Joy.mp3]]"
                    .to_owned(),
            ),
            "Songs/Silent.md" => Some("---\ncover: Art/silent.jpg\n---\n".to_owned()),
            _ => None,
        };
        let bytes = |p: &str| (p != "Art/silent.jpg").then(|| p.as_bytes().to_vec());
        let vault = Vault {
            paths: &paths,
            text: &text,
            bytes: &bytes,
        };
        let note = "---\ntype: setlist\n---\n1. [[Joy]]\n2. [Silent](../Songs/Silent.md) [[Songs/Silent]]\n![x](Art/joy.png)";
        let got: Vec<String> = member_files(&vault, "Sets/Merry.md", note)
            .into_iter()
            .map(|f| f.path)
            .collect();
        // Silent's art has no bytes here; Joy's mp3 is no file a view reads;
        // the `..` link is never followed.
        assert_eq!(got, ["Art/joy.png", "Songs/Joy.lrc"]);
    }

    #[test]
    fn a_name_resolves_only_to_a_file_of_the_vault() {
        let paths = paths();
        assert_eq!(
            resolve("joy.png", "Art/x.md", &paths).as_deref(),
            Some("Art/joy.png")
        );
        assert_eq!(
            resolve("Joy", "Sets/Merry.md", &paths).as_deref(),
            Some("Songs/Joy.md")
        );
        for bad in [
            "../Art/joy.png",
            "/Art/joy.png",
            "Art/%2e%2e/joy.png",
            "Art\\joy.png",
            "nope.png",
            "C:/Art/joy.png",
        ] {
            assert_eq!(resolve(bad, "Sets/Merry.md", &paths), None, "{bad}");
        }
    }

    #[test]
    fn a_data_url_reads_as_its_bytes() {
        assert_eq!(
            data_bytes("data:image/png;base64,iVBORw==").unwrap(),
            b"\x89PNG"
        );
        assert_eq!(data_bytes("data:text/plain,hi"), None);
        assert_eq!(data_bytes("/app/x.png"), None);
    }

    #[test]
    fn references_are_targets_and_frontmatter_values() {
        assert_eq!(
            references(
                "---\ncover: '[[a.png|Art]]'\ntags:\n  - b.lrc\n---\n[[c#h|C]] [d](e.png) [w](https://x/y.png)"
            ),
            ["a.png", "b.lrc", "c", "e.png"]
        );
    }
}
