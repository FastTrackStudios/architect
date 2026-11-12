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
//! its vault path), whole, in any case (Unicode lowercase, character by
//! character; `note_type` the same, its spaces trimmed): `*` and
//! `?` stay within one segment, `**` is any number of segments. Matching
//! takes time linear in the pattern times the path, never exponential,
//! and [`ShareLayersDecl::check`] bounds what a manifest may ask
//! ([`MAX_PATTERNS`], [`MAX_PATTERN_LEN`], [`MAX_STARS`]). Within one
//! rule the lists are tried `page`, `reference`, `full`, and the first
//! that matches is the file's layer, so a narrow `reference` pattern can
//! carve a file out of a broad `full` one. Across rules (several
//! extensions for one note type) the node takes the highest: no rule
//! lowers another's. A file no rule matches is a page file.

use serde::{Deserialize, Serialize};

/// The most patterns one rule may give, its three lists together.
pub const MAX_PATTERNS: usize = 64;
/// The longest pattern, in bytes.
pub const MAX_PATTERN_LEN: usize = 256;
/// The most `*` in one pattern (a `**` is two).
pub const MAX_STARS: usize = 8;

/// One `[[share_layers]]` rule: the files of notes of `note_type`, by
/// layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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
        let (mine, theirs) = (self.note_type.trim(), note_type.trim());
        mine.chars().count() == theirs.chars().count()
            && mine
                .chars()
                .zip(theirs.chars())
                .all(|(a, b)| same_char(a, b))
    }

    /// Its note type named, and each pattern a relative path within the
    /// limits.
    ///
    /// # Errors
    ///
    /// No note type; more than [`MAX_PATTERNS`] patterns; a pattern that
    /// is empty, absolute, longer than [`MAX_PATTERN_LEN`], has more than
    /// [`MAX_STARS`] `*`, a `..` or `.` segment or a backslash.
    pub fn check(&self) -> Result<(), String> {
        if self.note_type.trim().is_empty() {
            return Err("name its note_type".to_owned());
        }
        let count = self
            .page
            .len()
            .saturating_add(self.reference.len())
            .saturating_add(self.full.len());
        if count > MAX_PATTERNS {
            return Err(format!(
                "{:?}: {count} patterns; at most {MAX_PATTERNS}",
                self.note_type
            ));
        }
        for pattern in self.page.iter().chain(&self.reference).chain(&self.full) {
            let stars = pattern.matches('*').count();
            if pattern.len() > MAX_PATTERN_LEN || stars > MAX_STARS {
                return Err(format!(
                    "{:?}: {pattern:?} is longer than {MAX_PATTERN_LEN} bytes or has more than {MAX_STARS} `*`",
                    self.note_type
                ));
            }
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
///
/// In any case (Unicode lowercase, character by character: `Gänse*` is
/// `GÄNSE.ogg`; the Unicode form is not folded: spell it as the vault
/// does). Linear in the pattern times the path: one wildcard is retried
/// at a time, never a tree of them.
#[must_use]
pub fn glob(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    wildcard(&pattern, &path, |p| *p == "**", |p, n| one(p, n))
}

/// One segment against one pattern segment: `*` any run, `?` one
/// character.
fn one(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    wildcard(
        &pattern,
        &name,
        |c| *c == '*',
        |p, n| *p == '?' || same_char(*p, *n),
    )
}

/// `a` and `b` alike in any case: equal, or equal lower cased (Unicode).
fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// `items` matched whole by `pattern`, where `star(p)` matches any run
/// and `fits(p, i)` matches one item: the two-pointer match that, on a
/// mismatch, retries from the last star only.
fn wildcard<P, I>(
    pattern: &[P],
    items: &[I],
    star: impl Fn(&P) -> bool,
    fits: impl Fn(&P, &I) -> bool,
) -> bool {
    let (mut p, mut i) = (0, 0);
    let mut retry: Option<(usize, usize)> = None;
    while i < items.len() {
        match (pattern.get(p), items.get(i)) {
            (Some(at), _) if star(at) => {
                retry = Some((p, i));
                p = p.saturating_add(1);
            }
            (Some(at), Some(item)) if fits(at, item) => {
                p = p.saturating_add(1);
                i = i.saturating_add(1);
            }
            _ => match retry {
                Some((sp, si)) => {
                    p = sp.saturating_add(1);
                    i = si.saturating_add(1);
                    retry = Some((sp, i));
                }
                None => return false,
            },
        }
    }
    pattern.get(p..).is_some_and(|rest| rest.iter().all(&star))
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

    /// A hostile pattern is refused when the manifest is read, and
    /// matching stays linear for the patterns that are let in.
    #[test]
    fn a_pathological_pattern_is_refused_and_matching_is_linear() {
        let twelve = format!("{}b", "*a".repeat(12));
        let manifest = format!(
            "id = \"x.y\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n[[share_layers]]\nnote_type = \"song\"\nfull = [\"{twelve}\"]\n"
        );
        assert!(crate::manifest::Manifest::parse(&manifest).is_err());
        let long = ShareLayersDecl {
            full: vec!["a".repeat(MAX_PATTERN_LEN + 1)],
            ..song()
        };
        assert!(long.check().is_err());
        let many = ShareLayersDecl {
            full: vec!["x".to_owned(); MAX_PATTERNS + 1],
            ..song()
        };
        assert!(many.check().is_err());
        let deep = ShareLayersDecl {
            full: vec!["**/".repeat(MAX_STARS) + "x"],
            ..song()
        };
        assert!(deep.check().is_err());
        // The worst the limits let in, against long names: fast.
        let worst = format!("{}b", "*a".repeat(MAX_STARS));
        let name = "a".repeat(200);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert!(!glob(&worst, &name));
            assert!(!glob(
                &format!("{}x", "**/".repeat(MAX_STARS / 2)),
                &"a/".repeat(100)
            ));
        }
        assert!(
            start.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            start.elapsed()
        );
    }

    /// A note type and a pattern both match in any case.
    #[test]
    fn note_types_and_patterns_ignore_ascii_case() {
        let rule = song();
        assert!(rule.is_for("SONG"));
        assert_eq!(
            rule.layer_of("Joy/Joy.md", "Joy/media/proxies/bass.ogg"),
            Some("full")
        );
        assert_eq!(
            rule.layer_of("Joy/Joy.md", "Joy/JOY.SESSION"),
            Some("reference")
        );
        assert!(glob("Media/*.OGG", "media/bass.ogg"));
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

    /// A misspelled list is not ignored: the manifest does not read.
    #[test]
    fn an_unknown_key_is_refused() {
        let text = "id = \"x.y\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n[[share_layers]]\nnote_type = \"song\"\nfulll = [\"Media/**\"]\n";
        assert!(crate::manifest::Manifest::parse(text).is_err());
    }

    /// Spaces around a note type are not part of it.
    #[test]
    fn a_note_type_with_spaces_matches() {
        let rule = ShareLayersDecl {
            note_type: " Song ".into(),
            ..song()
        };
        assert!(rule.is_for("song"));
        assert!(rule.is_for(" SONG"));
    }

    /// Case is folded beyond ASCII: `Gänse*` is `GÄNSE.ogg`.
    #[test]
    fn case_folding_is_unicode() {
        assert!(glob("Media/Gänse*", "MEDIA/GÄNSE.ogg"));
        assert!(glob("Ä?", "äb"));
        let rule = ShareLayersDecl {
            note_type: "Lied".into(),
            ..song()
        };
        assert!(rule.is_for("LIED"));
        let umlaut = ShareLayersDecl {
            note_type: "Stück".into(),
            ..song()
        };
        assert!(umlaut.is_for("STÜCK"));
    }
}
