//! Which of a root's files a device holds (#904).
//!
//! A root is all or nothing on a device unless it has a [`HoldRule`]: by
//! path and by size, the files a device fetches, keeps and copies to its
//! bucket. A file the rule leaves out stays in the manifest (listed, its
//! versions too); its bytes live on the holders that keep it. So the
//! always-on public node holds a project's charts, covers and reference
//! mixes and never its multitracks, which stay on a storage server.
//! Wasm-clean: the model alone; the engine (`crate::sync`) applies it.

use facet::Facet;

/// What a device holds of the roots it has.
///
/// The default holds everything, as a device without a rule does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct HoldRule {
    /// Root-relative globs ([`glob`]). When any is set, only a path one of
    /// them matches is held.
    #[facet(default)]
    pub paths: Vec<String>,
    /// Root-relative globs never held, whatever [`Self::paths`] says.
    #[facet(default)]
    pub skip: Vec<String>,
    /// A single file over this many bytes is never held. `None`: any size.
    #[facet(default)]
    pub max: Option<u64>,
}

impl HoldRule {
    /// Whether it holds everything (no rule).
    #[must_use]
    pub const fn is_all(&self) -> bool {
        self.paths.is_empty() && self.skip.is_empty() && self.max.is_none()
    }

    /// Whether a file at the root-relative `path`, `size` bytes long, is
    /// held: under [`Self::max`], matched by no [`Self::skip`] glob, and
    /// by a [`Self::paths`] glob when there is any.
    #[must_use]
    pub fn keeps(&self, path: &str, size: u64) -> bool {
        self.max.is_none_or(|max| size <= max)
            && !self.skip.iter().any(|g| glob(g, path))
            && (self.paths.is_empty() || self.paths.iter().any(|g| glob(g, path)))
    }

    /// Both rules at once: what `self` and `other` each keep.
    #[must_use]
    pub fn keeps_both(&self, other: &Self, path: &str, size: u64) -> bool {
        self.keeps(path, size) && other.keeps(path, size)
    }
}

/// Whether `pattern` matches the root-relative `path`.
///
/// Segments are split at `/`. `**` is any number of folders, none
/// included (`**/*.md` matches `a.md` and `x/y/a.md`; `**/media/**`
/// everything under any `media` folder). In a segment `*` is any run of
/// characters and `?` any one. Matching is case-sensitive, as paths are.
#[must_use]
pub fn glob(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            // A trailing `**` takes whatever is left, files included.
            rest.is_empty()
                || (0..=path.len()).any(|skip| segments(rest, path.get(skip..).unwrap_or_default()))
        }
        Some((first, rest)) => path.split_first().is_some_and(|(name, below)| {
            segment(first.as_bytes(), name.as_bytes()) && segments(rest, below)
        }),
    }
}

fn segment(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => {
            (0..=name.len()).any(|skip| segment(rest, name.get(skip..).unwrap_or_default()))
        }
        Some((b'?', rest)) => name
            .split_first()
            .is_some_and(|(_, below)| segment(rest, below)),
        Some((c, rest)) => name
            .split_first()
            .is_some_and(|(n, below)| n == c && segment(rest, below)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_folders_at_any_depth_and_names_by_wildcard() {
        assert!(glob("**/*.md", "notes.md"));
        assert!(glob("**/*.md", "songs/joy/notes.md"));
        assert!(!glob("**/*.md", "songs/joy/notes.mdx"));
        assert!(glob("**/media/**", "songs/joy/media/kick.wav"));
        assert!(glob("**/media/**", "media/a/b/kick.wav"));
        assert!(
            !glob("**/media/**", "songs/Media/kick.wav"),
            "case-sensitive"
        );
        assert!(!glob("**/media/**", "songs/joy/multimedia/kick.wav"));
        assert!(glob("renders/*.wav", "renders/mix.wav"));
        assert!(!glob("renders/*.wav", "renders/old/mix.wav"));
        assert!(glob("take-??.wav", "take-01.wav"));
        assert!(!glob("take-??.wav", "take-1.wav"));
        assert!(glob("**", "anything/at/all.bin"));
        assert!(!glob("*.md", "a/b.md"));
    }

    /// The hosted node's rule (#904): the light part of every root,
    /// never the multitracks.
    #[test]
    fn a_rule_holds_the_light_part_and_leaves_the_multitracks_out() {
        let rule = HoldRule {
            paths: vec![
                "**/reference/**".to_owned(),
                "**/*.md".to_owned(),
                "**/*.kf".to_owned(),
                "**/*.pdf".to_owned(),
            ],
            skip: vec!["**/Media/**".to_owned()],
            max: Some(256 << 20),
        };
        assert!(rule.keeps("songs/joy/Joy.kf", 4_000));
        assert!(rule.keeps("songs/joy/reference/mix.mp3", 9 << 20));
        assert!(!rule.keeps("songs/joy/Media/kick.wav", 9 << 20), "skipped");
        assert!(!rule.keeps("songs/joy/Media/notes.md", 10), "skip wins");
        assert!(!rule.keeps("songs/joy/Joy.RPP", 4_000), "not listed");
        assert!(
            !rule.keeps("songs/joy/reference/film.mov", 1 << 30),
            "too large"
        );

        assert!(HoldRule::default().is_all());
        assert!(HoldRule::default().keeps("anything", u64::MAX));
        let small = HoldRule {
            max: Some(1 << 20),
            ..HoldRule::default()
        };
        assert!(!small.is_all());
        assert!(small.keeps("cover.jpg", 1 << 20));
        assert!(!small.keeps("stem.wav", (1 << 20) + 1));
        assert!(!rule.keeps_both(&small, "songs/joy/reference/mix.mp3", 9 << 20));
    }
}
