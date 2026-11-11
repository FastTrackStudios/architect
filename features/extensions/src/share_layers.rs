//! `[[share_layers]]`: which layer of a share link each file of a note
//! type is in (#528).
//!
//! A share link plays to a layer its sharer sets (`architect_spaces::Listen`:
//! page, reference, full), and the node that serves it refuses a file
//! above it. Architect does not know a song's folder layout; the
//! extension that owns the note type says it, as data:
//!
//! ```toml
//! [[share_layers]]
//! note_type = "song"
//! reference = ["Media/Proxies/Reference*", "Media/Peaks/**", "*.session"]
//! full = ["Media/Proxies/**"]
//! ```
//!
//! Each pattern is matched against an attached file's path relative to
//! the folder of the note that attaches it (a file outside that folder:
//! its vault path), whole: `*` and `?` stay within one segment, `**` is
//! any number of segments. The lists are tried `page`, `reference`,
//! `full`, and the first that matches is the file's layer, so a narrow
//! `reference` pattern can carve a file out of a broad `full` one. A file
//! no rule matches is a page file, served as before.

use serde::{Deserialize, Serialize};

/// One `[[share_layers]]` rule: the files of notes of `note_type`, by
/// layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShareLayersDecl {
    /// The notes it is about, by their frontmatter `type:` (`song`).
    pub note_type: String,
    /// Files served on every link, as a file no rule names is.
    pub page: Vec<String>,
    /// Files a Reference link serves (and a Full one).
    pub reference: Vec<String>,
    /// Files only a Full link serves.
    pub full: Vec<String>,
}

impl ShareLayersDecl {
    /// The layer `file` (attached by `note`, both vault paths) is in, by
    /// its label (`page`, `reference`, `full`); `None` when no pattern
    /// matches it.
    #[must_use]
    pub fn layer_of(&self, note: &str, file: &str) -> Option<&'static str> {
        let folder = note.rsplit_once('/').map_or("", |(dir, _)| dir);
        let rel = if folder.is_empty() {
            file
        } else {
            file.strip_prefix(folder)
                .and_then(|r| r.strip_prefix('/'))
                .unwrap_or(file)
        };
        [
            ("page", &self.page),
            ("reference", &self.reference),
            ("full", &self.full),
        ]
        .into_iter()
        .find(|(_, patterns)| patterns.iter().any(|p| glob(p, rel)))
        .map(|(layer, _)| layer)
    }

    /// Whether it is about notes of `note_type` (any case).
    #[must_use]
    pub fn is_for(&self, note_type: &str) -> bool {
        self.note_type.eq_ignore_ascii_case(note_type.trim())
    }

    /// Its note type named, and each pattern a relative path.
    ///
    /// # Errors
    ///
    /// No note type, or a pattern that is empty, absolute, has a `..` or
    /// `.` segment or a backslash.
    pub fn check(&self) -> Result<(), String> {
        if self.note_type.trim().is_empty() {
            return Err("name its note_type".to_owned());
        }
        for pattern in self.page.iter().chain(&self.reference).chain(&self.full) {
            let bad = pattern.is_empty()
                || pattern.starts_with('/')
                || pattern.contains('\\')
                || pattern
                    .split('/')
                    .any(|s| s.is_empty() || s == ".." || s == ".");
            if bad {
                return Err(format!(
                    "{:?}: {pattern:?} is not a path below the note's folder",
                    self.note_type
                ));
            }
        }
        Ok(())
    }
}

/// Whether `path` matches `pattern`, whole, segment by segment.
#[must_use]
pub fn glob(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            (0..=path.len()).any(|skip| path.get(skip..).is_some_and(|tail| segments(rest, tail)))
        }
        Some((first, rest)) => path.split_first().is_some_and(|(segment, tail)| {
            one(first.as_bytes(), segment.as_bytes()) && segments(rest, tail)
        }),
    }
}

/// One segment against one pattern segment: `*` any run, `?` one byte.
fn one(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => {
            (0..=name.len()).any(|skip| name.get(skip..).is_some_and(|tail| one(rest, tail)))
        }
        Some((b'?', rest)) => name.split_first().is_some_and(|(_, tail)| one(rest, tail)),
        Some((c, rest)) => name
            .split_first()
            .is_some_and(|(n, tail)| n == c && one(rest, tail)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> ShareLayersDecl {
        ShareLayersDecl {
            note_type: "song".into(),
            page: vec!["Charts/*.pdf".into()],
            reference: vec![
                "Media/Proxies/Reference*".into(),
                "Media/Peaks/**".into(),
                "*.session".into(),
            ],
            full: vec!["Media/Proxies/**".into()],
        }
    }

    #[test]
    fn globs_match_whole_paths_segment_by_segment() {
        assert!(glob("*.session", "Joy.session"));
        assert!(!glob("*.session", "Old/Joy.session"), "one segment");
        assert!(glob("**/*.session", "Old/Joy.session"));
        assert!(glob("**/*.session", "Joy.session"), "** is zero or more");
        assert!(glob("Media/Proxies/**", "Media/Proxies/Bass/DI.ogg"));
        assert!(!glob("Media/Proxies/**", "Media/Proxies2/x.ogg"));
        assert!(glob("Take?.wav", "Take1.wav"));
        assert!(!glob("Take?.wav", "Take12.wav"));
        assert!(!glob("Media/*", "Media"));
    }

    #[test]
    fn the_first_layer_that_matches_is_the_files() {
        let rule = song();
        let note = "Songs/Joy/Joy.md";
        let at = |rel: &str| rule.layer_of(note, &format!("Songs/Joy/{rel}"));
        assert_eq!(at("Media/Proxies/Reference.ogg"), Some("reference"));
        assert_eq!(at("Media/Proxies/Reference Preview.ogg"), Some("reference"));
        assert_eq!(at("Media/Proxies/Bass.ogg"), Some("full"));
        assert_eq!(at("Media/Peaks/Bass.sessionpeaks"), Some("reference"));
        assert_eq!(at("Joy.session"), Some("reference"));
        assert_eq!(at("Charts/Joy.pdf"), Some("page"));
        assert_eq!(at("cover.png"), None, "no rule: a page file");
        // A file outside the note's folder: by its vault path.
        assert_eq!(rule.layer_of(note, "Media/Proxies/Bass.ogg"), Some("full"));
        // A note at the vault's root.
        assert_eq!(rule.layer_of("Joy.md", "Joy.session"), Some("reference"));
        assert!(rule.is_for("Song"));
        assert!(!rule.is_for("chart"));
    }

    #[test]
    fn a_manifest_reads_its_rules_and_refuses_a_bad_one() {
        let text = "id = \"fts.studio\"\nname = \"Studio\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
                    [[share_layers]]\nnote_type = \"song\"\nreference = [\"*.session\"]\nfull = [\"Media/**\"]\n";
        let manifest = crate::manifest::Manifest::parse(text).expect("it reads");
        assert_eq!(manifest.share_layers.len(), 1);
        assert_eq!(
            manifest.share_layers[0].layer_of("Joy.md", "Media/Bass.ogg"),
            Some("full")
        );
        let bad = text.replace("Media/**", "../Media/**");
        assert!(crate::manifest::Manifest::parse(&bad).is_err());
    }

    #[test]
    fn a_rule_is_checked() {
        assert!(song().check().is_ok());
        let with = |p: &str| ShareLayersDecl {
            full: vec![p.to_owned()],
            ..song()
        };
        for bad in ["", "/abs", "../up", "a/../b", "a\\b", "a//b", "./x"] {
            assert!(with(bad).check().is_err(), "{bad:?}");
        }
        assert!(
            ShareLayersDecl {
                note_type: " ".into(),
                ..song()
            }
            .check()
            .is_err()
        );
    }
}
