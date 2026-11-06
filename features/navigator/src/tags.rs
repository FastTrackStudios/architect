//! The tag tree: the vault's tags nested by `/`, each with the notes that
//! have it, under a "Tags" row that lists every tagged note, and an
//! "Untagged" row for the notes with none.
//!
//! Notebook Navigator's `buildTagTreeFromDatabase` and
//! `buildTreeFromTagList` (`src/utils/tagTree.ts`), its counts
//! (`getTotalNoteCount`, `createTagNoteCountInfo`), its root order
//! (`resolveRootTagOrdering`,
//! `src/hooks/navigationPane/data/useNavigationPaneTreeSections.ts`) and
//! its comparators (`createTagComparator`, `createFrequencyComparator`,
//! `src/hooks/navigationPane/data/navigationComparators.ts`):
//!
//! - A tag's key is its path without `#` or `/` at either end, lower case;
//!   it shows as it was first written (the notes read in path order).
//! - A node for each tag and each tag above one (`a/b/c` makes `a`, `a/b`
//!   and `a/b/c`); a note is counted on the tag it has, and with the
//!   notes below it once each.
//! - Only Markdown notes have tags. Untagged counts the Markdown notes
//!   with none.
//! - The roots in the order a person gave them (`rootTagOrder`, Untagged
//!   among them), then the rest by the sort, Untagged last. Children by the
//!   sort. A to Z is natural order, case folded, the key breaking ties;
//!   frequency is the notes with the tag (and below it when notes from
//!   below are on), A to Z breaking ties; descending is the whole order
//!   reversed.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::index::NoteCount;
use crate::settings::TreeSort;
use crate::source::NoteFile;
use crate::tree::natural_cmp;

/// The "Tags" row's id: every tagged note (`__tagged__`).
pub const TAGGED: &str = "__tagged__";
/// The "Untagged" row's id: the notes with no tag (`__untagged__`).
pub const UNTAGGED: &str = "__untagged__";

/// A tag's key: no `#`, no `/` at either end, lower case
/// (`normalizeTagPathValue`).
#[must_use]
pub fn tag_key(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase()
}

/// Whether the tag keyed `key` is below `want`.
#[must_use]
pub fn is_below(key: &str, want: &str) -> bool {
    key.len() > want.len() && key.starts_with(want) && key.as_bytes().get(want.len()) == Some(&b'/')
}

/// Whether the tag keyed `key` is `want` or below it.
#[must_use]
pub fn is_or_below(key: &str, want: &str) -> bool {
    key == want || is_below(key, want)
}

/// Whether `id` is one of the rows that are not a tag ("Tags",
/// "Untagged").
#[must_use]
pub fn is_virtual(id: &str) -> bool {
    id == TAGGED || id == UNTAGGED
}

/// One tag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagNode {
    /// Its key ([`tag_key`]).
    pub key: String,
    /// Its last segment, as first written.
    pub name: String,
    /// Its path, as first written.
    pub display: String,
    /// The keys of the tags directly below it.
    pub children: BTreeSet<String>,
    /// The notes that have it, not counting those below.
    pub notes: BTreeSet<String>,
    /// The notes that have it or a tag below it.
    pub all: BTreeSet<String>,
}

impl TagNode {
    /// Its count: its own notes, and those below it once each.
    #[must_use]
    pub fn count(&self) -> NoteCount {
        NoteCount {
            current: self.notes.len(),
            descendants: self.all.len().saturating_sub(self.notes.len()),
        }
    }
}

/// The tags of a set of notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagTree {
    nodes: BTreeMap<String, TagNode>,
    roots: BTreeSet<String>,
    /// How many Markdown notes have a tag, and how many have none.
    pub tagged: usize,
    pub untagged: usize,
}

impl TagTree {
    /// The tree of `files`' tags. Each note's tags once each, case folded
    /// (`extractFileTagsFromRawTags`).
    #[must_use]
    pub fn build<'a>(files: impl IntoIterator<Item = &'a NoteFile>) -> Self {
        let mut files: Vec<&NoteFile> = files.into_iter().filter(|f| f.is_markdown()).collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut tree = Self::default();
        for f in files {
            let mut seen = BTreeSet::new();
            let tags: Vec<&str> = f
                .tags
                .iter()
                .map(|t| t.trim().trim_start_matches('#').trim_matches('/'))
                .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
                .collect();
            if tags.is_empty() {
                tree.untagged = tree.untagged.saturating_add(1);
                continue;
            }
            tree.tagged = tree.tagged.saturating_add(1);
            for t in tags {
                tree.add(t, &f.path);
            }
        }
        tree.fill_all();
        tree
    }

    fn add(&mut self, tag: &str, path: &str) {
        let mut parent: Option<String> = None;
        let mut display = String::new();
        let segments: Vec<&str> = tag.split('/').filter(|s| !s.is_empty()).collect();
        let last = segments.len().saturating_sub(1);
        for (i, seg) in segments.into_iter().enumerate() {
            if !display.is_empty() {
                display.push('/');
            }
            display.push_str(seg);
            let key = display.to_lowercase();
            let node = self.nodes.entry(key.clone()).or_insert_with(|| TagNode {
                key: key.clone(),
                name: seg.to_owned(),
                display: display.clone(),
                ..TagNode::default()
            });
            if i == last {
                node.notes.insert(path.to_owned());
            }
            match &parent {
                Some(p) if *p != key => {
                    if let Some(up) = self.nodes.get_mut(p) {
                        up.children.insert(key.clone());
                    }
                }
                Some(_) => {}
                None => {
                    self.roots.insert(key.clone());
                }
            }
            parent = Some(key);
        }
    }

    /// Each node's notes with those below it: deepest first.
    fn fill_all(&mut self) {
        let mut keys: Vec<String> = self.nodes.keys().cloned().collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.matches('/').count()));
        for k in keys {
            let Some(node) = self.nodes.get(&k) else {
                continue;
            };
            let mut all = node.notes.clone();
            for c in &node.children {
                if let Some(child) = self.nodes.get(c) {
                    all.extend(child.all.iter().cloned());
                }
            }
            if let Some(node) = self.nodes.get_mut(&k) {
                node.all = all;
            }
        }
    }

    /// The tag keyed `key` (any case, `#` or not).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&TagNode> {
        self.nodes.get(&tag_key(key))
    }

    /// Every tag, by key.
    pub fn nodes(&self) -> impl Iterator<Item = &TagNode> {
        self.nodes.values()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The root tags' keys, in `order` first (keys, [`UNTAGGED`] among
    /// them), then the rest by `sort`, then Untagged when `untagged` and
    /// not placed.
    #[must_use]
    pub fn roots(
        &self,
        order: &[String],
        sort: TreeSort,
        below: bool,
        untagged: bool,
    ) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for k in order {
            let k = if k == UNTAGGED { k.clone() } else { tag_key(k) };
            let fits = if k == UNTAGGED {
                untagged
            } else {
                self.roots.contains(&k)
            };
            if fits && !out.contains(&k) {
                out.push(k);
            }
        }
        let mut rest: Vec<&String> = self.roots.iter().filter(|k| !out.contains(k)).collect();
        rest.sort_by(|a, b| self.compare(a, b, sort, below));
        out.extend(rest.into_iter().cloned());
        if untagged && !out.iter().any(|k| k == UNTAGGED) {
            out.push(UNTAGGED.to_owned());
        }
        out
    }

    /// The keys of the tags directly below `key`, by `sort`.
    #[must_use]
    pub fn children(&self, key: &str, sort: TreeSort, below: bool) -> Vec<String> {
        let Some(node) = self.nodes.get(key) else {
            return Vec::new();
        };
        let mut out: Vec<&String> = node.children.iter().collect();
        out.sort_by(|a, b| self.compare(a, b, sort, below));
        out.into_iter().cloned().collect()
    }

    /// Two tags in `sort`'s order.
    fn compare(&self, left: &str, right: &str, sort: TreeSort, below: bool) -> Ordering {
        let (Some(l), Some(r)) = (self.nodes.get(left), self.nodes.get(right)) else {
            return left.cmp(right);
        };
        let alpha = || natural_cmp(&l.name, &r.name).then_with(|| l.key.cmp(&r.key));
        let count = |t: &TagNode| if below { t.all.len() } else { t.notes.len() };
        match sort {
            TreeSort::AlphaAsc => alpha(),
            TreeSort::AlphaDesc => alpha().reverse(),
            TreeSort::FrequencyAsc => count(l).cmp(&count(r)).then_with(alpha),
            TreeSort::FrequencyDesc => count(l).cmp(&count(r)).then_with(alpha).reverse(),
        }
    }

    /// The root keys with `key` moved `by` places, as the new manual order
    /// (every root listed, so it holds as tags come and go).
    #[must_use]
    pub fn reorder(
        &self,
        order: &[String],
        sort: TreeSort,
        below: bool,
        untagged: bool,
        key: &str,
        by: isize,
    ) -> Vec<String> {
        let mut keys = self.roots(order, sort, below, untagged);
        if let Some(at) = keys.iter().position(|k| k == key) {
            let to = at
                .saturating_add_signed(by)
                .min(keys.len().saturating_sub(1));
            let moved = keys.remove(at);
            keys.insert(to, moved);
        }
        keys
    }
}

/// `tags` (a note's, as written) with `want` added; `None` when unchanged.
///
/// As Notebook Navigator's add does to a note: left as they are when it
/// has the tag or one above it; else the tags below it dropped, the tag
/// added.
#[must_use]
pub fn added(tags: &[String], want: &str) -> Option<Vec<String>> {
    let key = tag_key(want);
    if tags.iter().any(|t| is_or_below(&key, &tag_key(t))) {
        return None;
    }
    let mut out: Vec<String> = tags
        .iter()
        .filter(|t| !is_below(&tag_key(t), &key))
        .cloned()
        .collect();
    out.push(
        want.trim()
            .trim_start_matches('#')
            .trim_matches('/')
            .to_owned(),
    );
    Some(out)
}

/// `tags` with the tag keyed `from` and those below it renamed to `to`,
/// each keeping the rest of its path as written.
#[must_use]
pub fn renamed(tags: &[String], from: &str, to: &str) -> Vec<String> {
    let from = tag_key(from);
    let to = to.trim().trim_start_matches('#').trim_matches('/');
    tags.iter()
        .map(|t| {
            let bare = t.trim().trim_start_matches('#');
            if is_or_below(&tag_key(bare), &from) {
                let tail: String = bare.chars().skip(from.chars().count()).collect();
                format!("{to}{tail}")
            } else {
                t.clone()
            }
        })
        .collect()
}

/// Whether `tag` can be a tag: not empty, no spaces, no `#`, no `/` at
/// either end or twice in a row (`hasValidTagCharacters`).
#[must_use]
pub fn valid(tag: &str) -> bool {
    let t = tag.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    !t.is_empty()
        && !t.starts_with('/')
        && !t.ends_with('/')
        && !t.contains("//")
        && t.chars().any(char::is_alphanumeric)
        && t.chars().all(|c| {
            c.is_alphanumeric()
                || matches!(c, '_' | '-' | '/')
                || (!c.is_ascii() && !c.is_whitespace())
        })
}

/// The leaf of a tag path: `a/b/c` is `c`.
#[must_use]
pub fn leaf(tag: &str) -> &str {
    tag.rsplit_once('/').map_or(tag, |(_, l)| l)
}
