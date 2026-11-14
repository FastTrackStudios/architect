//! The vault index: what queries read, never the notes' bodies.
//!
//! One [`Entry`] per note (its title, properties, tags, links, headings
//! and task lines) and per `.base` file (its YAML). [`Builder`] keeps it
//! from a vault that changes ([`NoteSource`]), a note at a time: the
//! notes that did not change keep their entries, and each sync is an
//! [`IndexDelta`] (the entries put, the paths gone). It runs wherever the
//! vault lives: on a desk and a phone over their own copy, on the node for
//! the web. A browser keeps a [`VaultIndex`] of its own from a snapshot
//! and the deltas after it ([`VaultIndex::apply`]), filtered to what its
//! person may read ([`VaultIndex::filtered`], [`IndexDelta::filtered`]).
//!
//! On the wire the index is JSON ([`VaultIndex::to_bytes`],
//! [`IndexDelta::to_bytes`]): short keys, empty fields left out.

use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::obsidian_parse::{ParsedPage, parse_page};

/// One note (or `.base` file) in the index.
// Not `Eq`: a property is any JSON value, numbers included.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its vault path: `Songs/Song A.md`.
    #[serde(rename = "p")]
    pub path: String,
    /// Its title: the first `# heading`, else the file's name.
    #[serde(rename = "t", default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// When it was last written, in milliseconds since 1970.
    #[serde(rename = "m", default, skip_serializing_if = "is_zero")]
    pub mtime: u64,
    /// Its frontmatter, by key (sorted; the last of a repeated key), shared
    /// with the row the query engine makes of it.
    #[serde(rename = "f", default, skip_serializing_if = "is_empty")]
    pub props: Arc<IndexMap<String, Value>>,
    /// Its tags: inline `#tag`s (the frontmatter's are in `props`).
    #[serde(rename = "g", default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The `[[links]]` in its body, as written (the frontmatter's are in
    /// `props`).
    #[serde(rename = "l", default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Its headings: level and text.
    #[serde(rename = "h", default, skip_serializing_if = "Vec::is_empty")]
    pub headings: Vec<(u8, String)>,
    /// Its task lines: the marker (` `, `x`, `/`, …) and the text.
    #[serde(rename = "k", default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<(String, String)>,
    /// A `.base` file's YAML (a base, not a note).
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[allow(clippy::ref_option_ref)]
fn is_empty(props: &Arc<IndexMap<String, Value>>) -> bool {
    props.is_empty()
}

impl Entry {
    /// A note's entry from its parse.
    #[must_use]
    pub fn of_note(path: &str, mtime: u64, parsed: &ParsedPage) -> Self {
        let mut tags: Vec<String> = Vec::new();
        let mut links: Vec<String> = Vec::new();
        let mut headings = Vec::new();
        let mut tasks = Vec::new();
        for block in &parsed.blocks {
            for r in &block.refs {
                match r {
                    crate::refs::Ref::Tag(t) => {
                        let joined = t.path.join("/");
                        if !joined.is_empty() && !tags.contains(&joined) {
                            tags.push(joined);
                        }
                    }
                    crate::refs::Ref::Link(l) => {
                        if !links.contains(&l.target_linkpath) {
                            links.push(l.target_linkpath.clone());
                        }
                    }
                    _ => {}
                }
            }
            if let Some(level) = block.heading_level {
                headings.push((u8::try_from(level).unwrap_or(6), block.content.trim().to_owned()));
            }
            if let Some(mark) = &block.list_task {
                tasks.push((mark.clone(), block.content.trim().to_owned()));
            }
        }
        // Through a `serde_json::Map`: last write wins on a repeated key, as
        // Obsidian reads it, and the keys sorted, as a base's row has them.
        let props: serde_json::Map<String, Value> = parsed
            .frontmatter
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        let props = Arc::new(props.into_iter().collect::<IndexMap<_, _>>());
        let name = crate::rows::split_rel(path).1;
        let title = headings
            .iter()
            .find(|(level, _)| *level == 1)
            .map_or(name, |(_, t)| t.clone());
        Self {
            path: path.to_owned(),
            title,
            mtime,
            props,
            tags,
            links,
            headings,
            tasks,
            base: None,
        }
    }

    /// A note's entry from its markdown.
    #[must_use]
    pub fn of_markdown(path: &str, mtime: u64, markdown: &str) -> Self {
        Self::of_note(path, mtime, &parse_page(markdown))
    }

    /// A `.base` file's entry.
    #[must_use]
    pub fn of_base(path: &str, mtime: u64, yaml: &str) -> Self {
        Self {
            path: path.to_owned(),
            title: crate::rows::split_rel(path).1,
            mtime,
            base: Some(yaml.to_owned()),
            ..Self::default()
        }
    }

    /// Whether it is a `.base` file's.
    #[must_use]
    pub const fn is_base(&self) -> bool {
        self.base.is_some()
    }
}

/// The index at one revision: every entry, by path. Cheap to clone (the
/// entries are shared).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VaultIndex {
    /// Where its builder was: moves on with every delta.
    pub revision: u64,
    pub entries: BTreeMap<Arc<str>, Arc<Entry>>,
}

/// What changed from one revision to the next.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexDelta {
    /// The revision it applies to.
    pub from: u64,
    /// The revision after it.
    pub to: u64,
    /// The entries made or changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub put: Vec<Entry>,
    /// The paths no longer in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    revision: u64,
    entries: Vec<Entry>,
}

impl VaultIndex {
    /// Applies `delta`, when it starts where this is.
    ///
    /// # Errors
    ///
    /// It starts elsewhere: ask for a snapshot, or the deltas since this
    /// revision.
    pub fn apply(&mut self, delta: &IndexDelta) -> Result<(), String> {
        if delta.from != self.revision {
            return Err(format!(
                "the delta is from revision {}, the index is at {}",
                delta.from, self.revision
            ));
        }
        for gone in &delta.gone {
            self.entries.remove(gone.as_str());
        }
        for e in &delta.put {
            self.entries.insert(Arc::from(e.path.as_str()), Arc::new(e.clone()));
        }
        self.revision = delta.to;
        Ok(())
    }

    /// Only the entries `may_read` lets through (a person's view of it).
    #[must_use]
    pub fn filtered(&self, may_read: impl Fn(&str) -> bool) -> Self {
        Self {
            revision: self.revision,
            entries: self
                .entries
                .iter()
                .filter(|(p, _)| may_read(p))
                .map(|(p, e)| (Arc::clone(p), Arc::clone(e)))
                .collect(),
        }
    }

    /// Its wire form: JSON.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let snap = Snapshot {
            revision: self.revision,
            entries: self.entries.values().map(|e| (**e).clone()).collect(),
        };
        serde_json::to_vec(&snap).unwrap_or_default()
    }

    /// An index from its wire form.
    ///
    /// # Errors
    ///
    /// The bytes are not one.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let snap: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            revision: snap.revision,
            entries: snap
                .entries
                .into_iter()
                .map(|e| (Arc::from(e.path.as_str()), Arc::new(e)))
                .collect(),
        })
    }

    /// The `.base` file at `path`'s YAML.
    #[must_use]
    pub fn base(&self, path: &str) -> Option<&str> {
        self.entries.get(path).and_then(|e| e.base.as_deref())
    }
}

impl IndexDelta {
    /// Whether nothing changed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.put.is_empty() && self.gone.is_empty()
    }

    /// Only what `may_read` lets through: a path it hides is never named,
    /// put or gone (a note renamed out of view is its old path gone). When
    /// what a reader may read changes, it takes a snapshot again.
    #[must_use]
    pub fn filtered(&self, may_read: impl Fn(&str) -> bool) -> Self {
        let put = self.put.iter().filter(|e| may_read(&e.path)).cloned().collect();
        let gone = self.gone.iter().filter(|p| may_read(p)).cloned().collect();
        Self {
            from: self.from,
            to: self.to,
            put,
            gone,
        }
    }

    /// Its wire form: JSON.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// A delta from its wire form.
    ///
    /// # Errors
    ///
    /// The bytes are not one.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// The deltas one after another, as one.
    #[must_use]
    pub fn merged(deltas: &[Self]) -> Option<Self> {
        let first = deltas.first()?;
        let mut put: BTreeMap<String, Entry> = BTreeMap::new();
        let mut gone: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for d in deltas {
            for p in &d.gone {
                put.remove(p);
                gone.insert(p.clone());
            }
            for e in &d.put {
                gone.remove(&e.path);
                put.insert(e.path.clone(), e.clone());
            }
        }
        Some(Self {
            from: first.from,
            to: deltas.last().map_or(first.to, |d| d.to),
            put: put.into_values().collect(),
            gone: gone.into_iter().collect(),
        })
    }
}

/// A note (or `.base` file) as a [`NoteSource`] shows it to the builder.
pub struct Note<'a> {
    pub path: &'a str,
    /// Changes whenever the note may have (its modified time, its length,
    /// where its text is held): equal means unchanged.
    pub stamp: u64,
    /// Its text.
    pub text: &'a str,
    /// Its parse, when the source keeps one (else the text is parsed).
    pub parsed: Option<&'a ParsedPage>,
    /// Milliseconds since 1970.
    pub mtime: u64,
    /// A `.base` file, not a note.
    pub is_base: bool,
}

/// Where a vault's notes are, for [`Builder::sync`]: a vault on disk (the
/// `vault` crate's `OpenVault`), a node's copy.
pub trait NoteSource {
    /// Moves on with every change: the same revision before and after a
    /// read means the read saw one state.
    fn revision(&self) -> u64;

    /// Shows `each` the notes from the `from`th, at most `n`, in one
    /// state; answers the revision of that state and how many notes there
    /// are in all.
    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize);
}

/// How many notes [`Builder::sync`] reads in one [`NoteSource::visit`]:
/// the vault is held for no longer than that many notes take.
pub const CHUNK: usize = 256;

/// How many deltas a builder keeps for [`Builder::since`].
pub const KEPT: usize = 256;

fn hash_of(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Keeps an index from a [`NoteSource`], a note at a time.
#[derive(Default)]
pub struct Builder {
    index: VaultIndex,
    /// Each note's stamp and its text's hash when its entry was made.
    stamps: BTreeMap<Arc<str>, (u64, u64)>,
    /// The source's revision last synced from.
    synced: Option<u64>,
    /// The latest deltas, oldest first.
    deltas: VecDeque<IndexDelta>,
}

impl Builder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The index now.
    #[must_use]
    pub const fn index(&self) -> &VaultIndex {
        &self.index
    }

    /// Brings the index up to `source`: each note whose stamp moved and
    /// whose text changed gets a new entry, the notes gone go. The delta
    /// (empty when nothing changed, or the source has not moved).
    pub fn sync(&mut self, source: &dyn NoteSource) -> IndexDelta {
        if self.synced == Some(source.revision()) {
            return self.delta(Vec::new(), Vec::new());
        }
        // A change between two chunks starts it again; past a few, the
        // source is read in one go.
        let (seen, revision) = (0..3)
            .find_map(|_| self.read(source, CHUNK))
            .or_else(|| self.read(source, usize::MAX))
            .unwrap_or_default();
        let mut put = Vec::new();
        let mut present = std::collections::BTreeSet::new();
        for (path, stamp, hash, entry) in seen {
            present.insert(Arc::clone(&path));
            self.stamps.insert(Arc::clone(&path), (stamp, hash));
            if let Some(entry) = entry {
                put.push(entry);
            }
        }
        let gone: Vec<String> = self
            .index
            .entries
            .keys()
            .filter(|p| !present.contains(*p))
            .map(ToString::to_string)
            .collect();
        for p in &gone {
            self.stamps.remove(p.as_str());
        }
        self.synced = Some(revision);
        self.delta(put, gone)
    }

    /// One read of `source`, `chunk` notes at a time: each note's path,
    /// stamp, hash and new entry (`None`: unchanged). `None` when the
    /// source moved between chunks.
    #[allow(clippy::type_complexity)]
    fn read(&self, source: &dyn NoteSource, chunk: usize) -> Option<(Vec<(Arc<str>, u64, u64, Option<Entry>)>, u64)> {
        let mut seen = Vec::new();
        let mut at = None;
        let mut from = 0;
        loop {
            let mut got: usize = 0;
            let (revision, total) = source.visit(from, chunk, &mut |note: Note<'_>| {
                got = got.saturating_add(1);
                let was = self.stamps.get_key_value(note.path);
                let path = was.map_or_else(|| Arc::from(note.path), |(p, _)| Arc::clone(p));
                let old = was.map(|(_, s)| *s);
                if let Some((stamp, hash)) = old.filter(|(s, _)| *s == note.stamp) {
                    seen.push((path, stamp, hash, None));
                    return;
                }
                let hash = hash_of(note.text);
                if old.is_some_and(|(_, h)| h == hash) {
                    seen.push((path, note.stamp, hash, None));
                    return;
                }
                let entry = if note.is_base {
                    Entry::of_base(note.path, note.mtime, note.text)
                } else if let Some(parsed) = note.parsed {
                    Entry::of_note(note.path, note.mtime, parsed)
                } else {
                    Entry::of_markdown(note.path, note.mtime, note.text)
                };
                seen.push((path, note.stamp, hash, Some(entry)));
            });
            if *at.get_or_insert(revision) != revision {
                return None;
            }
            from = from.saturating_add(got);
            if got == 0 || from >= total {
                return Some((seen, revision));
            }
        }
    }

    /// Records a delta of `put` and `gone` and applies it.
    fn delta(&mut self, put: Vec<Entry>, gone: Vec<String>) -> IndexDelta {
        let from = self.index.revision;
        if put.is_empty() && gone.is_empty() {
            return IndexDelta { from, to: from, put, gone };
        }
        let delta = IndexDelta {
            from,
            to: from.saturating_add(1),
            put,
            gone,
        };
        // It starts where the index is: it applies.
        let _ = self.index.apply(&delta);
        self.deltas.push_back(delta.clone());
        while self.deltas.len() > KEPT {
            self.deltas.pop_front();
        }
        delta
    }

    /// Everything since revision `from`, as one delta; `None` when it is
    /// further back than the deltas kept (take a snapshot instead).
    #[must_use]
    pub fn since(&self, from: u64) -> Option<IndexDelta> {
        if from == self.index.revision {
            return Some(IndexDelta {
                from,
                to: from,
                ..IndexDelta::default()
            });
        }
        let start = self.deltas.iter().position(|d| d.from == from)?;
        let tail: Vec<IndexDelta> = self.deltas.iter().skip(start).cloned().collect();
        IndexDelta::merged(&tail)
    }
}

/// A [`Builder`] over a [`NoteSource`], kept
/// up to it on every read: a desk's, a phone's or the node's index.
pub struct Built<S> {
    source: S,
    builder: Mutex<(Builder, Arc<VaultIndex>)>,
}

impl<S: NoteSource> Built<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self {
            source,
            builder: Mutex::new((Builder::new(), Arc::default())),
        }
    }

    /// Brings the index up to the source; the delta.
    pub fn sync(&self) -> IndexDelta {
        let mut held = self.builder.lock().unwrap_or_else(PoisonError::into_inner);
        let delta = held.0.sync(&self.source);
        if !delta.is_empty() || held.1.revision != held.0.index().revision {
            held.1 = Arc::new(held.0.index().clone());
        }
        delta
    }

    /// Everything since revision `from` (`Builder::since`).
    pub fn since(&self, from: u64) -> Option<IndexDelta> {
        let _ = self.sync();
        self.builder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .since(from)
    }

    /// The source.
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// The index, brought up to the source first.
    pub fn index(&self) -> Arc<VaultIndex> {
        let _ = self.sync();
        Arc::clone(&self.builder.lock().unwrap_or_else(PoisonError::into_inner).1)
    }
}


#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// A source over `(path, text, stamp)`s.
    struct Notes(u64, Vec<(String, String, u64)>);

    impl NoteSource for Notes {
        fn revision(&self) -> u64 {
            self.0
        }
        fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
            for (path, text, stamp) in self.1.iter().skip(from).take(n) {
                each(Note {
                    path,
                    stamp: *stamp,
                    text,
                    parsed: None,
                    mtime: 0,
                    is_base: std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("base")),
                });
            }
            (self.0, self.1.len())
        }
    }

    fn notes(rev: u64, v: &[(&str, &str, u64)]) -> Notes {
        Notes(rev, v.iter().map(|(p, t, s)| ((*p).to_owned(), (*t).to_owned(), *s)).collect())
    }

    /// An entry holds what queries read, and nothing of the body's prose.
    #[test]
    fn an_entry_is_the_notes_facts() {
        let e = Entry::of_markdown(
            "Songs/A.md",
            7,
            "---\nstatus: todo\ntags: [song]\n---\n# Amazing\n\nSome prose #live with [[B]].\n\n## Parts\n- [ ] learn it\n- [x] chart it\n",
        );
        assert_eq!(e.title, "Amazing");
        assert_eq!(e.props["status"], "todo");
        assert_eq!(e.tags, ["live"]);
        assert_eq!(e.links, ["B"]);
        assert_eq!(e.headings, [(1, "Amazing".to_owned()), (2, "Parts".to_owned())]);
        assert_eq!(e.tasks.len(), 2);
        let wire = String::from_utf8(VaultIndex::default().to_bytes()).unwrap();
        assert!(!wire.contains("prose"));
    }

    /// A sync makes entries for the notes that changed only, deltas carry
    /// them, and a replica that applies the deltas is the index.
    #[test]
    fn deltas_carry_only_what_changed() {
        let mut b = Builder::new();
        let first = b.sync(&notes(2, &[("A.md", "# A", 1), ("B.md", "# B", 1)]));
        assert_eq!((first.put.len(), first.to), (2, 1));
        let mut replica = VaultIndex::from_bytes(&b.index().to_bytes()).unwrap();
        assert_eq!(&replica, b.index());

        // Nothing moved: no delta.
        assert!(b.sync(&notes(2, &[("A.md", "# A", 1), ("B.md", "# B", 1)])).is_empty());
        // A's stamp moved but its text did not: no delta.
        assert!(b.sync(&notes(4, &[("A.md", "# A", 2), ("B.md", "# B", 1)])).is_empty());
        // B changed, C came, A went.
        let d = b.sync(&notes(6, &[("B.md", "# Bee", 3), ("C.base", "views: []", 1)]));
        assert_eq!(d.put.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["B.md", "C.base"]);
        assert_eq!(d.gone, ["A.md"]);
        replica.apply(&IndexDelta::from_bytes(&d.to_bytes()).unwrap()).unwrap();
        assert_eq!(&replica, b.index());
        assert_eq!(replica.base("C.base"), Some("views: []"));
        // A delta from elsewhere is refused.
        assert!(replica.apply(&first).is_err());
        // Since revision 1, as one delta.
        assert_eq!(b.since(1).unwrap().gone, ["A.md"]);
        assert!(b.since(99).is_none());
    }

    /// A reader sees only what it may: hidden entries are gone to it.
    #[test]
    fn a_reader_sees_what_it_may() {
        let mut b = Builder::new();
        let _ = b.sync(&notes(2, &[("Public/A.md", "# A", 1), ("Private/B.md", "# B", 1)]));
        let public = |p: &str| p.starts_with("Public/");
        assert_eq!(b.index().filtered(public).entries.len(), 1);
        let d = b.sync(&notes(4, &[("Public/A.md", "# A", 1), ("Private/B.md", "# Bee", 2)]));
        let theirs = d.filtered(public);
        assert!(theirs.is_empty(), "a hidden note is never named");
        let d = b.sync(&notes(6, &[("Private/A.md", "# A", 3), ("Private/B.md", "# Bee", 2)]));
        assert_eq!(d.filtered(public).gone, ["Public/A.md"], "moved out of view: gone");
    }
}
