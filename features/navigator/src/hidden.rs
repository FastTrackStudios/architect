//! What a vault profile hides: folders, tags, notes and files, by
//! pattern, by frontmatter or by tag; and which files show at all.
//!
//! Notebook Navigator's hiding (`src/utils/fileFilters.ts`
//! `matchesFolderPattern`, `isFolderInExcludedFolder`,
//! `shouldExcludeFolderFromDescendants`, `createHiddenFileNameMatcher`,
//! `createFrontmatterPropertyExclusionMatcher`;
//! `src/utils/pathPatternMatcher.ts`; `src/utils/tagPrefixMatcher.ts`
//! `createHiddenTagMatcher`, `matchesHiddenTagPattern`;
//! `src/utils/exclusionUtils.ts` `createFileHiddenMatcher`;
//! `src/utils/fileTypeUtils.ts` `FILE_VISIBILITY`):
//!
//! - **Folders** (`hiddenFolders`): a name (`archive`), a name that starts
//!   or ends so (`temp*`, `*_old`), or a path from the root (`/work/old`,
//!   `/projects/*/drafts`, `/clients/acme*`). A folder hidden hides every
//!   folder and file below it.
//! - **Folders excluded from descendants** (`descendantExcludedFolders`):
//!   the same patterns, but the folder still shows and can be browsed;
//!   only its notes are left out of the lists of the folders above it
//!   (with "notes from subfolders" on). A path matches the folder itself,
//!   not the ones below it.
//! - **Tags** (`hiddenTags`): `archive` (and the tags below it),
//!   `temp*`, `*draft`, or a path `projects/*/old`.
//! - **Notes by tag** (`hiddenFileTags`): a note with a tag the same
//!   patterns match.
//! - **Notes by name** (`hiddenFileNames`): a file name (`Untitled.md`),
//!   a name without its extension, an extension (`.excalidraw`), a path
//!   (`/Templates/Daily.md`), each with `*` anywhere.
//! - **Notes by frontmatter** (`hiddenFileProperties`): `key` (a note with
//!   the key) or `key=value`.
//! - **File visibility**: documents (Markdown, canvases, bases), the types
//!   architect opens, or every file.
//!
//! Everything is matched in any case. With "show hidden items" on, nothing
//! is hidden.

use facet::Facet;

use crate::list::value_parts;
use crate::properties::value_path;
use crate::source::NoteFile;

/// Which files the navigator shows (`fileVisibility`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum FileVisibility {
    /// Markdown, canvases and bases.
    Documents,
    /// The types architect opens: documents, PDFs, images, audio, video.
    Supported,
    /// Every file: the default, as architect's extensions open types of
    /// their own (Notebook Navigator's default, `supported`, counts the
    /// types Obsidian's plugins registered).
    #[default]
    All,
}

/// The documents (`isPrimaryDocumentFile`).
pub const DOCUMENTS: [&str; 3] = ["md", "canvas", "base"];

/// What architect opens beyond the documents (Obsidian's own viewers).
pub const SUPPORTED: [&str; 21] = [
    "pdf", "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "avif", "mp3", "wav", "m4a", "ogg",
    "3gp", "flac", "webm", "mp4", "ogv", "mov", "mkv", "txt",
];

impl FileVisibility {
    /// Whether a file with extension `ext` shows.
    #[must_use]
    pub fn shows(self, ext: &str) -> bool {
        let ext = ext.to_ascii_lowercase();
        match self {
            Self::All => true,
            Self::Documents => DOCUMENTS.contains(&ext.as_str()),
            Self::Supported => {
                DOCUMENTS.contains(&ext.as_str()) || SUPPORTED.contains(&ext.as_str())
            }
        }
    }

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Documents => "documents",
            Self::Supported => "supported",
            Self::All => "all",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Documents, Self::Supported, Self::All]
            .into_iter()
            .find(|v| v.id() == id)
    }
}

/// One segment of a path pattern (`PathPatternSegment`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Prefix(String),
    Any,
}

/// A path pattern (`ParsedPathPattern`): segments, and how many lead as
/// literals.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathPattern {
    segments: Vec<Segment>,
    literal_prefix: usize,
}

impl PathPattern {
    /// `parsePathPattern`: `*` alone is any one segment, `name*` a
    /// segment starting so; anything else with a `*` is not a pattern.
    fn parse(pattern: &str) -> Option<Self> {
        let mut segments = Vec::new();
        for part in pattern.split('/').filter(|p| !p.is_empty()) {
            let stars = part.matches('*').count();
            segments.push(match (stars, part) {
                (0, p) => Segment::Literal(p.to_owned()),
                (1, "*") => Segment::Any,
                (1, p) if p.len() > 1 && p.ends_with('*') => {
                    Segment::Prefix(p.trim_end_matches('*').to_owned())
                }
                _ => return None,
            });
        }
        if segments.is_empty() {
            return None;
        }
        let literal_prefix = segments
            .iter()
            .take_while(|s| matches!(s, Segment::Literal(_)))
            .count();
        Some(Self {
            segments,
            literal_prefix,
        })
    }

    /// `matchesParsedPatternSegments`: the path, or one below it.
    fn matches(&self, path: &[&str]) -> bool {
        if path.is_empty() {
            return false;
        }
        for (seg, part) in self.segments.iter().zip(path) {
            let ok = match seg {
                Segment::Any => true,
                Segment::Prefix(p) => part.starts_with(p.as_str()),
                Segment::Literal(l) => part == l,
            };
            if !ok {
                return false;
            }
        }
        if path.len() >= self.segments.len() {
            return true;
        }
        // Shorter: only trailing wildcards left, past the literals.
        path.len() > self.literal_prefix
            && self
                .segments
                .get(path.len()..)
                .is_some_and(|rest| rest.iter().all(|s| *s == Segment::Any))
    }

    /// The path itself, not one below it (the boundary matcher).
    fn matches_exactly(&self, path: &[&str]) -> bool {
        path.len() == self.segments.len() && self.matches(path)
    }
}

/// `matchesFolderPattern`: a name, `prefix*`, `*suffix`.
fn name_matches(name: &str, pattern: &str) -> bool {
    if pattern.is_empty() || name.is_empty() {
        return false;
    }
    if !pattern.contains('*') {
        return name == pattern;
    }
    match (pattern.strip_suffix('*'), pattern.strip_prefix('*')) {
        (Some(prefix), None) => name.starts_with(prefix),
        (None, Some(suffix)) => name.ends_with(suffix),
        _ => name == pattern,
    }
}

/// Folder patterns, compiled: names and paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FolderRules {
    names: Vec<String>,
    paths: Vec<PathPattern>,
}

impl FolderRules {
    fn new(patterns: &[String]) -> Self {
        let mut out = Self::default();
        for p in patterns {
            let p = p.trim().to_lowercase().replace('\\', "/");
            if p.is_empty() {
                continue;
            }
            if p.starts_with('/') {
                out.paths.extend(PathPattern::parse(&p));
            } else {
                out.names.push(p);
            }
        }
        out
    }

    const fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty()
    }

    /// `isFolderInExcludedFolder`: the folder or one it is in.
    fn hides(&self, folder: &str) -> bool {
        if self.is_empty() || folder.is_empty() {
            return false;
        }
        let lower = folder.to_lowercase();
        let parts: Vec<&str> = lower.split('/').filter(|p| !p.is_empty()).collect();
        parts
            .iter()
            .any(|n| self.names.iter().any(|p| name_matches(n, p)))
            || self.paths.iter().any(|p| p.matches(&parts))
    }

    /// `shouldExcludeFolderFromDescendants`: the folder itself.
    fn bounds(&self, folder: &str) -> bool {
        if self.is_empty() || folder.is_empty() {
            return false;
        }
        let lower = folder.to_lowercase();
        let parts: Vec<&str> = lower.split('/').filter(|p| !p.is_empty()).collect();
        parts
            .last()
            .is_some_and(|n| self.names.iter().any(|p| name_matches(n, p)))
            || self.paths.iter().any(|p| p.matches_exactly(&parts))
    }
}

/// Tag patterns, compiled (`HiddenTagMatcher`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TagRules {
    prefixes: Vec<String>,
    starts: Vec<String>,
    ends: Vec<String>,
    paths: Vec<PathPattern>,
}

impl TagRules {
    fn new(patterns: &[String]) -> Self {
        let mut out = Self::default();
        for raw in patterns {
            let p = clean_tag(raw);
            if p.is_empty() {
                continue;
            }
            if let Some(path) = PathPattern::parse(&p).filter(|_| p.contains('/') || p == "*") {
                out.paths.push(path);
            }
            let stars = p.matches('*').count();
            let slash = p.contains('/');
            if !slash && stars == 1 && p.starts_with('*') && !p.ends_with('*') {
                out.ends.push(p.trim_start_matches('*').to_owned());
            } else if !slash && stars == 1 && p.ends_with('*') && !p.starts_with('*') {
                let prefix = p.trim_end_matches('*').to_owned();
                out.starts.push(prefix.clone());
                out.prefixes.push(prefix);
            } else if stars == 0 {
                out.prefixes.push(p);
            }
        }
        out
    }

    const fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
            && self.starts.is_empty()
            && self.ends.is_empty()
            && self.paths.is_empty()
    }

    /// `matchesHiddenTagPattern`.
    fn hides(&self, tag: &str) -> bool {
        if self.is_empty() {
            return false;
        }
        let path = clean_tag(tag);
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let name = parts.last().copied().unwrap_or_default();
        self.paths.iter().any(|p| p.matches(&parts))
            || self.prefixes.iter().any(|p| {
                path == *p
                    || path
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            })
            || self.starts.iter().any(|p| name.starts_with(p.as_str()))
            || self.ends.iter().any(|p| name.ends_with(p.as_str()))
    }
}

fn clean_tag(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase()
}

/// A `*` glob (`compileGlobPattern`, `matchesCompiledGlobPattern`).
fn glob_matches(value: &str, glob: &str) -> bool {
    if glob == "*" {
        return true;
    }
    let parts: Vec<&str> = glob.split('*').filter(|p| !p.is_empty()).collect();
    let Some((first, rest)) = parts.split_first() else {
        return true;
    };
    let anchored_start = !glob.starts_with('*');
    let anchored_end = !glob.ends_with('*');
    if anchored_start && !value.starts_with(first) {
        return false;
    }
    let (mut at, middle): (usize, &[&str]) = if anchored_start {
        (first.len(), rest)
    } else {
        (0, &parts)
    };
    let (middle, last) = if anchored_end {
        match middle.split_last() {
            Some((last, m)) => (m, Some(*last)),
            // One part, anchored both ends: it is the whole value.
            None => return value.len() == at,
        }
    } else {
        (middle, None)
    };
    for part in middle {
        match value.get(at..).and_then(|v| v.find(part)) {
            Some(i) => at = at.saturating_add(i).saturating_add(part.len()),
            None => return false,
        }
    }
    last.is_none_or(|suffix| {
        value
            .rfind(suffix)
            .is_some_and(|i| i >= at && i.saturating_add(suffix.len()) == value.len())
    })
}

/// File name patterns, compiled (`createHiddenFileNameMatcher`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NameRules {
    names: Vec<String>,
    paths: Vec<String>,
    extensions: Vec<String>,
    name_globs: Vec<String>,
    path_globs: Vec<String>,
}

impl NameRules {
    fn new(patterns: &[String]) -> Self {
        let mut out = Self::default();
        let rooted = |p: &str| {
            if p.starts_with('/') {
                p.to_owned()
            } else {
                format!("/{p}")
            }
        };
        for raw in patterns {
            let p = raw.trim().to_lowercase();
            if p.is_empty() {
                continue;
            }
            let path = p.contains('/');
            match (p.contains('*'), path) {
                (false, true) => out.paths.push(rooted(&p)),
                (false, false) if p.starts_with('.') && p.len() > 1 => out.extensions.push(p),
                (false, false) => out.names.push(p),
                (true, true) => out.path_globs.push(rooted(&p)),
                (true, false) => out.name_globs.push(p),
            }
        }
        out
    }

    fn hides(&self, f: &NoteFile) -> bool {
        let name = f.file_name().to_lowercase();
        let base = f.name().to_lowercase();
        let path = format!("/{}", f.path.to_lowercase());
        let ext = match f.extension() {
            "" => String::new(),
            e => format!(".{}", e.to_lowercase()),
        };
        self.names.contains(&name)
            || self.paths.contains(&path)
            || (!ext.is_empty() && self.extensions.contains(&ext))
            || self
                .name_globs
                .iter()
                .any(|g| glob_matches(&name, g) || glob_matches(&base, g))
            || self.path_globs.iter().any(|g| glob_matches(&path, g))
            || (!ext.is_empty() && self.name_globs.iter().any(|g| glob_matches(&ext, g)))
    }
}

/// Frontmatter rules, compiled: keys, and keys with values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PropertyRules {
    keys: Vec<String>,
    values: Vec<(String, String)>,
}

impl PropertyRules {
    fn new(rules: &[String]) -> Self {
        let mut out = Self::default();
        for raw in rules {
            match raw.split_once('=') {
                None => {
                    let k = raw.trim().to_lowercase();
                    if !k.is_empty() {
                        out.keys.push(k);
                    }
                }
                Some((k, v)) => {
                    let (k, v) = (k.trim().to_lowercase(), value_path(v));
                    if !k.is_empty() && !v.is_empty() {
                        out.values.push((k, v));
                    }
                }
            }
        }
        out
    }

    fn hides(&self, f: &NoteFile) -> bool {
        f.properties.iter().any(|(k, v)| {
            let k = k.trim().to_lowercase();
            self.keys.contains(&k)
                || self
                    .values
                    .iter()
                    .any(|(rk, rv)| *rk == k && value_parts(v).iter().any(|p| value_path(p) == *rv))
        })
    }
}

/// A profile's hiding rules, compiled (`createFileHiddenMatcher`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hidden {
    folders: FolderRules,
    boundaries: FolderRules,
    tags: TagRules,
    file_tags: TagRules,
    names: NameRules,
    properties: PropertyRules,
    visibility: FileVisibility,
}

/// The rules a [`Hidden`] is made from: a profile's.
#[derive(Debug, Clone, Copy)]
pub struct HiddenRules<'a> {
    pub folders: &'a [String],
    pub descendant_excluded: &'a [String],
    pub tags: &'a [String],
    pub file_tags: &'a [String],
    pub file_names: &'a [String],
    pub file_properties: &'a [String],
    pub visibility: FileVisibility,
}

impl Hidden {
    /// `rules`, compiled; with `show_hidden`, only the file visibility
    /// and the descendant boundaries stay.
    #[must_use]
    pub fn new(rules: &HiddenRules<'_>, show_hidden: bool) -> Self {
        let none: &[String] = &[];
        let pick = |r: &'_ [String]| {
            if show_hidden {
                none.to_vec()
            } else {
                r.to_vec()
            }
        };
        Self {
            folders: FolderRules::new(&pick(rules.folders)),
            boundaries: FolderRules::new(rules.descendant_excluded),
            tags: TagRules::new(&pick(rules.tags)),
            file_tags: TagRules::new(&pick(rules.file_tags)),
            names: NameRules::new(&pick(rules.file_names)),
            properties: PropertyRules::new(&pick(rules.file_properties)),
            visibility: rules.visibility,
        }
    }

    /// Whether nothing is hidden and every file shows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
            && self.tags.is_empty()
            && self.file_tags.is_empty()
            && self.names == NameRules::default()
            && self.properties == PropertyRules::default()
            && self.visibility == FileVisibility::All
    }

    /// Whether the folder (or one it is in) is hidden.
    #[must_use]
    pub fn hides_folder(&self, folder: &str) -> bool {
        self.folders.hides(folder)
    }

    /// Whether the folder's notes are left out of the lists above it.
    #[must_use]
    pub fn bounds(&self, folder: &str) -> bool {
        self.boundaries.bounds(folder)
    }

    /// Whether any folder from `below` (a folder in `above`) up to, not
    /// including, `above` keeps its notes out of `above`'s list.
    #[must_use]
    pub fn bounded_between(&self, below: &str, above: &str) -> bool {
        if self.boundaries.is_empty() {
            return false;
        }
        let mut at = below;
        while !at.is_empty() && at != above {
            if self.bounds(at) {
                return true;
            }
            at = at.rsplit_once('/').map_or("", |(p, _)| p);
        }
        false
    }

    /// Whether the tag (and so its row) is hidden.
    #[must_use]
    pub fn hides_tag(&self, tag: &str) -> bool {
        self.tags.hides(tag)
    }

    /// Whether the file is hidden: its type, its folder, its name, its
    /// frontmatter or one of its tags.
    #[must_use]
    pub fn hides_file(&self, f: &NoteFile) -> bool {
        !self.visibility.shows(f.extension())
            || self.hides_folder(f.folder())
            || (f.is_markdown() && self.properties.hides(f))
            || (f.is_markdown() && f.tags.iter().any(|t| self.file_tags.hides(t)))
            || self.names.hides(f)
    }

    /// The file as the navigator sees it: its hidden tags taken off.
    #[must_use]
    pub fn shown(&self, f: &NoteFile) -> NoteFile {
        let mut out = f.clone();
        if !self.tags.is_empty() {
            out.tags.retain(|t| !self.hides_tag(t));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn folder_names_paths_and_wildcards() {
        let r = FolderRules::new(&s(&[
            "archive",
            "temp*",
            "*_old",
            "/work/private",
            "/projects/*/drafts",
        ]));
        assert!(r.hides("Archive"));
        assert!(r.hides("a/archive/b"));
        assert!(r.hides("tempfiles"));
        assert!(r.hides("x/notes_old"));
        assert!(r.hides("work/private"));
        assert!(r.hides("work/private/deep"));
        assert!(!r.hides("work"));
        assert!(r.hides("projects/alpha/drafts"));
        assert!(!r.hides("projects/alpha"));
        assert!(!r.hides("other/drafts"));
    }

    #[test]
    fn a_boundary_is_the_folder_itself() {
        let r = FolderRules::new(&s(&["/journal", "assets"]));
        assert!(r.bounds("journal"));
        assert!(!r.bounds("journal/2026"));
        assert!(r.bounds("x/assets"));
    }

    #[test]
    fn tags_by_prefix_name_and_path() {
        let r = TagRules::new(&s(&["archive", "temp*", "*draft", "projects/*/old"]));
        assert!(r.hides("#archive"));
        assert!(r.hides("Archive/2024"));
        assert!(!r.hides("archived"));
        assert!(r.hides("temporary"));
        assert!(r.hides("x/firstdraft"));
        assert!(r.hides("projects/alpha/old"));
        assert!(!r.hides("projects/alpha"));
    }

    #[test]
    fn file_names_globs_extensions_and_paths() {
        let r = NameRules::new(&s(&[
            "untitled*",
            ".excalidraw",
            "/templates/daily.md",
            "*.bak",
        ]));
        let f = |p: &str| NoteFile::new(p, 0, 0);
        assert!(r.hides(&f("a/Untitled 2.md")));
        assert!(r.hides(&f("x.excalidraw")));
        assert!(r.hides(&f("Templates/Daily.md")));
        assert!(r.hides(&f("y/old.bak")));
        assert!(!r.hides(&f("Templates/Weekly.md")));
        assert!(glob_matches("abcxdef", "abc*def"));
        assert!(!glob_matches("abcdef", "abc*xdef"));
        assert!(glob_matches("anything", "*"));
    }

    #[test]
    fn frontmatter_keys_and_values() {
        let r = PropertyRules::new(&s(&["private", "status=archived"]));
        let mut f = NoteFile::new("a.md", 0, 0);
        assert!(!r.hides(&f));
        f.properties.push((
            "Status".into(),
            crate::PropertyValue::List(vec!["open".into(), "Archived".into()]),
        ));
        assert!(r.hides(&f));
        let mut g = NoteFile::new("b.md", 0, 0);
        g.properties
            .push(("private".into(), crate::PropertyValue::Bool(true)));
        assert!(r.hides(&g));
    }

    #[test]
    fn visibility_and_show_hidden() {
        let rules = HiddenRules {
            folders: &s(&["secret"]),
            descendant_excluded: &[],
            tags: &[],
            file_tags: &[],
            file_names: &[],
            file_properties: &[],
            visibility: FileVisibility::Documents,
        };
        let h = Hidden::new(&rules, false);
        assert!(h.hides_file(&NoteFile::new("a.png", 0, 0)));
        assert!(h.hides_file(&NoteFile::new("secret/a.md", 0, 0)));
        assert!(!h.hides_file(&NoteFile::new("a.md", 0, 0)));
        let shown = Hidden::new(&rules, true);
        assert!(!shown.hides_file(&NoteFile::new("secret/a.md", 0, 0)));
        assert!(shown.hides_file(&NoteFile::new("a.png", 0, 0)));
    }
}
