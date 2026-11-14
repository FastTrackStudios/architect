//! The vault index: what queries read, never the notes' bodies.
//!
//! One [`Entry`] per note (its title, properties, tags, links, headings
//! and task lines) and per `.base` file (its YAML). [`Builder`] keeps it
//! from a vault that changes ([`NoteSource`]), a note at a time: the
//! notes that did not change keep their entries, and each sync is an
//! [`IndexDelta`] (the entries put, the paths gone). It runs wherever the
//! vault lives: on a desk and a phone over their own copy, on the node for
//! the web. A browser keeps a [`VaultIndex`] of its own from a snapshot
//! and the deltas after it ([`VaultIndex::apply`]). What a reader may read
//! is the server's to filter ([`VaultIndex::filtered`],
//! [`IndexDelta::filtered`]); the node does it per caller.
//!
//! **Histories.** Each builder has a random `epoch`: its revisions count
//! from its start, so a delta or a `since` from another builder's history
//! (a node restarted) is refused, and the reader takes a snapshot.
//!
//! **Memory.** An entry's property keys, tags and link targets are shared
//! strings, one per distinct text in the index ([`Interner`]); deltas hold
//! the same entries as the index; the full first build is no delta. An
//! entry is capped ([`MAX_PROPS`], [`MAX_LIST`], [`MAX_TEXT`],
//! [`MAX_VALUE`], [`MAX_BASE`]) and says so (`truncated`).
//!
//! On the wire the index is JSON ([`VaultIndex::to_bytes`],
//! [`IndexDelta::to_bytes`]): a version (`v`, [`WIRE`]), the epoch, short
//! keys, empty fields left out.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::obsidian_parse::{ParsedPage, parse_page};

/// The wire form's version.
pub const WIRE: u32 = 1;
/// The most bytes a snapshot or a delta may be on the wire.
pub const MAX_WIRE: usize = 256 << 20;
/// The most properties an entry keeps.
pub const MAX_PROPS: usize = 128;
/// The most tags, links, headings or tasks an entry keeps, each.
pub const MAX_LIST: usize = 512;
/// The most characters a heading's, a task's or a title's text keeps.
pub const MAX_TEXT: usize = 256;
/// The most bytes a property's value may take (as JSON): past it, the
/// property is left out.
pub const MAX_VALUE: usize = 4096;
/// The most bytes a `.base` file's YAML may take: past it, it is left out.
pub const MAX_BASE: usize = 64 << 10;

/// A note's properties: shared keys, any JSON values.
pub type Props = IndexMap<Arc<str>, Value>;

/// One note (or `.base` file) in the index.
// Not `Eq`: a property is any JSON value, numbers included.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its vault path: `Songs/Song A.md`.
    #[serde(rename = "p")]
    pub path: Arc<str>,
    /// Its title when it is not its file's name: the first `# heading`
    /// ([`Self::title`] reads either).
    #[serde(rename = "t", default, skip_serializing_if = "str::is_empty")]
    pub heading_title: Box<str>,
    /// When it was last written, in milliseconds since 1970.
    #[serde(rename = "m", default, skip_serializing_if = "is_zero")]
    pub mtime: u64,
    /// Its frontmatter, by key (sorted; the last of a repeated key).
    #[serde(rename = "f", default, skip_serializing_if = "is_empty")]
    pub props: Arc<Props>,
    /// Its tags: inline `#tag`s (the frontmatter's are in `props`).
    #[serde(rename = "g", default, skip_serializing_if = "<[_]>::is_empty")]
    pub tags: Box<[Arc<str>]>,
    /// The `[[links]]` in its body, as written (the frontmatter's are in
    /// `props`).
    #[serde(rename = "l", default, skip_serializing_if = "<[_]>::is_empty")]
    pub links: Box<[Arc<str>]>,
    /// Its headings: level and text.
    #[serde(rename = "h", default, skip_serializing_if = "<[_]>::is_empty")]
    pub headings: Box<[(u8, Box<str>)]>,
    /// Its task lines: the marker (` `, `x`, `/`, …) and the text.
    #[serde(rename = "k", default, skip_serializing_if = "<[_]>::is_empty")]
    pub tasks: Box<[(char, Box<str>)]>,
    /// A `.base` file's YAML (a base, not a note).
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Box<str>>,
    /// Something was left out to keep it under the caps.
    #[serde(rename = "x", default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(b: &bool) -> bool {
    !*b
}

fn is_empty(props: &Arc<Props>) -> bool {
    props.is_empty()
}

/// One shared string per distinct text: property keys, tags, links.
#[derive(Clone, Debug, Default)]
pub struct Interner(HashSet<Arc<str>>);

impl Interner {
    /// The shared string for `text`.
    pub fn get(&mut self, text: &str) -> Arc<str> {
        if let Some(have) = self.0.get(text) {
            return Arc::clone(have);
        }
        let new: Arc<str> = Arc::from(text);
        self.0.insert(Arc::clone(&new));
        new
    }

    /// Lets go of the strings nothing else holds.
    pub fn prune(&mut self) {
        self.0.retain(|s| Arc::strong_count(s) > 1);
    }
}

/// `text` cut to [`MAX_TEXT`] characters, and whether it was.
fn capped(text: &str) -> (Box<str>, bool) {
    match text.char_indices().nth(MAX_TEXT) {
        Some((at, _)) => (text.get(..at).unwrap_or(text).into(), true),
        None => (text.into(), false),
    }
}

impl Entry {
    /// A note's entry from its parse, its strings shared through `names`.
    #[must_use]
    pub fn of_note(path: &str, mtime: u64, parsed: &ParsedPage, names: &mut Interner) -> Self {
        let mut truncated = false;
        let mut tags: IndexSet<&str> = IndexSet::new();
        let mut links: IndexSet<&str> = IndexSet::new();
        let mut headings = Vec::new();
        let mut tasks = Vec::new();
        let mut joined: Vec<String> = Vec::new();
        for block in &parsed.blocks {
            for r in &block.refs {
                match r {
                    crate::refs::Ref::Tag(t) => joined.push(t.path.join("/")),
                    crate::refs::Ref::Link(l) => {
                        links.insert(l.target_linkpath.as_str());
                    }
                    _ => {}
                }
            }
            if let Some(level) = block.heading_level {
                let (text, cut) = capped(block.content.trim());
                truncated |= cut;
                headings.push((u8::try_from(level).unwrap_or(6), text));
            }
            if let Some(mark) = &block.list_task {
                let (text, cut) = capped(block.content.trim());
                truncated |= cut;
                tasks.push((mark.chars().next().unwrap_or(' '), text));
            }
        }
        for t in &joined {
            if !t.is_empty() {
                tags.insert(t.as_str());
            }
        }
        let mut list = |set: IndexSet<&str>, names: &mut Interner| -> Box<[Arc<str>]> {
            truncated |= set.len() > MAX_LIST;
            set.into_iter().take(MAX_LIST).map(|s| names.get(s)).collect()
        };
        let tags = list(tags, names);
        let links = list(links, names);
        truncated |= headings.len() > MAX_LIST || tasks.len() > MAX_LIST;
        headings.truncate(MAX_LIST);
        tasks.truncate(MAX_LIST);
        // Through a `serde_json::Map`: last write wins on a repeated key, as
        // Obsidian reads it, and the keys sorted, as a base's row has them.
        let map: serde_json::Map<String, Value> = parsed
            .frontmatter
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        truncated |= map.len() > MAX_PROPS;
        let mut props = Props::with_capacity(map.len().min(MAX_PROPS));
        for (k, v) in map.into_iter().take(MAX_PROPS) {
            if serde_json::to_vec(&v).map_or(true, |b| b.len() > MAX_VALUE) {
                truncated = true;
                continue;
            }
            props.insert(names.get(&k), v);
        }
        let name = crate::rows::split_rel(path).1;
        let heading_title = headings
            .iter()
            .find(|(level, _)| *level == 1)
            .map_or_else(Box::default, |(_, t)| t.clone());
        let heading_title = if *heading_title == *name { Box::default() } else { heading_title };
        Self {
            path: Arc::from(path),
            heading_title,
            mtime,
            props: Arc::new(props),
            tags,
            links,
            headings: headings.into_boxed_slice(),
            tasks: tasks.into_boxed_slice(),
            base: None,
            truncated,
        }
    }

    /// A note's entry from its markdown.
    #[must_use]
    pub fn of_markdown(path: &str, mtime: u64, markdown: &str, names: &mut Interner) -> Self {
        Self::of_note(path, mtime, &parse_page(markdown), names)
    }

    /// A `.base` file's entry.
    #[must_use]
    pub fn of_base(path: &str, mtime: u64, yaml: &str) -> Self {
        let fits = yaml.len() <= MAX_BASE;
        Self {
            path: Arc::from(path),
            mtime,
            base: Some(if fits { yaml.into() } else { Box::default() }),
            truncated: !fits,
            ..Self::default()
        }
    }

    /// Its title: its first `# heading`, else its file's name.
    #[must_use]
    pub fn title(&self) -> String {
        if self.heading_title.is_empty() {
            crate::rows::split_rel(&self.path).1
        } else {
            self.heading_title.to_string()
        }
    }

    /// Whether it is a `.base` file's.
    #[must_use]
    pub const fn is_base(&self) -> bool {
        self.base.is_some()
    }

    /// The same entry, its keys, tags and links shared through `names`
    /// (one read off the wire holds its own).
    #[must_use]
    pub fn interned(mut self, names: &mut Interner) -> Self {
        let props: Props = self
            .props
            .iter()
            .map(|(k, v)| (names.get(k), v.clone()))
            .collect();
        self.props = Arc::new(props);
        self.tags = self.tags.iter().map(|t| names.get(t)).collect();
        self.links = self.links.iter().map(|l| names.get(l)).collect();
        self
    }
}

/// The index at one revision of one builder's history: every entry, by
/// path. Cheap to clone (the entries are shared).
#[derive(Clone, Debug, Default)]
pub struct VaultIndex {
    /// Its builder's history ([`Builder`]'s random epoch).
    pub epoch: u64,
    /// Where its builder was: moves on with every delta.
    pub revision: u64,
    pub entries: BTreeMap<Arc<str>, Arc<Entry>>,
    /// The strings its entries share.
    names: Interner,
}

impl PartialEq for VaultIndex {
    fn eq(&self, other: &Self) -> bool {
        self.epoch == other.epoch && self.revision == other.revision && self.entries == other.entries
    }
}

/// What changed from one revision to the next.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexDelta {
    /// The wire form's version ([`WIRE`]).
    #[serde(default)]
    pub v: u32,
    /// The builder's history it belongs to.
    #[serde(default)]
    pub epoch: u64,
    /// The revision it applies to.
    pub from: u64,
    /// The revision after it.
    pub to: u64,
    /// The entries made or changed (shared with the index).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub put: Vec<Arc<Entry>>,
    /// The paths no longer in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<Arc<str>>,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    v: u32,
    epoch: u64,
    revision: u64,
    entries: Vec<Arc<Entry>>,
}

fn wire_ok(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_WIRE {
        return Err(format!("{} bytes: more than the {MAX_WIRE} an index may be", bytes.len()));
    }
    Ok(())
}

impl VaultIndex {
    /// Applies `delta`, when it is of this history and starts where this is.
    ///
    /// # Errors
    ///
    /// Another history, or another revision: take a snapshot, or ask for
    /// the deltas since this revision.
    pub fn apply(&mut self, delta: &IndexDelta) -> Result<(), String> {
        if delta.epoch != self.epoch {
            return Err("the delta is of another history: take a snapshot".to_owned());
        }
        if delta.from != self.revision {
            return Err(format!(
                "the delta is from revision {}, the index is at {}",
                delta.from, self.revision
            ));
        }
        for gone in &delta.gone {
            self.entries.remove(gone);
        }
        for e in &delta.put {
            self.entries.insert(Arc::clone(&e.path), Arc::clone(e));
        }
        self.revision = delta.to;
        Ok(())
    }

    /// [`Self::apply`] for a delta read off the wire: its strings shared
    /// with the index's.
    ///
    /// # Errors
    ///
    /// As [`Self::apply`].
    pub fn apply_interned(&mut self, mut delta: IndexDelta) -> Result<(), String> {
        let names = &mut self.names;
        delta.put = delta
            .put
            .into_iter()
            .map(|e| Arc::new(Arc::unwrap_or_clone(e).interned(names)))
            .collect();
        self.apply(&delta)?;
        self.names.prune();
        Ok(())
    }

    /// Only the entries `may_read` lets through (a person's view of it).
    #[must_use]
    pub fn filtered(&self, may_read: impl Fn(&str) -> bool) -> Self {
        Self {
            epoch: self.epoch,
            revision: self.revision,
            entries: self
                .entries
                .iter()
                .filter(|(p, _)| may_read(p))
                .map(|(p, e)| (Arc::clone(p), Arc::clone(e)))
                .collect(),
            names: Interner::default(),
        }
    }

    /// Its wire form: JSON.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let snap = Snapshot {
            v: WIRE,
            epoch: self.epoch,
            revision: self.revision,
            entries: self.entries.values().cloned().collect(),
        };
        serde_json::to_vec(&snap).unwrap_or_default()
    }

    /// An index from its wire form, its strings shared.
    ///
    /// # Errors
    ///
    /// The bytes are not one, too many, or of another version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        wire_ok(bytes)?;
        let snap: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if snap.v != WIRE {
            return Err(format!("an index of version {}, not {WIRE}", snap.v));
        }
        let mut names = Interner::default();
        let entries = snap
            .entries
            .into_iter()
            .map(|e| {
                let e = Arc::unwrap_or_clone(e).interned(&mut names);
                (Arc::clone(&e.path), Arc::new(e))
            })
            .collect();
        Ok(Self {
            epoch: snap.epoch,
            revision: snap.revision,
            entries,
            names,
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

    /// No change at `at`, of history `epoch`.
    #[must_use]
    pub const fn none(epoch: u64, at: u64) -> Self {
        Self {
            v: WIRE,
            epoch,
            from: at,
            to: at,
            put: Vec::new(),
            gone: Vec::new(),
        }
    }

    /// Only what `may_read` lets through: a path it hides is never named,
    /// put or gone (a note renamed out of view is its old path gone).
    #[must_use]
    pub fn filtered(&self, may_read: impl Fn(&str) -> bool) -> Self {
        Self {
            v: self.v,
            epoch: self.epoch,
            from: self.from,
            to: self.to,
            put: self.put.iter().filter(|e| may_read(&e.path)).cloned().collect(),
            gone: self.gone.iter().filter(|p| may_read(p)).cloned().collect(),
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
    /// The bytes are not one, too many, or of another version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        wire_ok(bytes)?;
        let d: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if d.v != WIRE {
            return Err(format!("a delta of version {}, not {WIRE}", d.v));
        }
        Ok(d)
    }

    /// The deltas one after another, as one.
    #[must_use]
    pub fn merged(deltas: &[Self]) -> Option<Self> {
        let first = deltas.first()?;
        let mut put: BTreeMap<Arc<str>, Arc<Entry>> = BTreeMap::new();
        let mut gone: BTreeSet<Arc<str>> = BTreeSet::new();
        for d in deltas {
            for p in &d.gone {
                put.remove(p);
                gone.insert(Arc::clone(p));
            }
            for e in &d.put {
                gone.remove(&e.path);
                put.insert(Arc::clone(&e.path), Arc::clone(e));
            }
        }
        Some(Self {
            v: WIRE,
            epoch: first.epoch,
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
    /// Changes whenever the note's text does (a hash of it, a generation
    /// moved by every write): equal means unchanged, and its text is not
    /// looked at again. A source that cannot promise that gives one that
    /// changes on every read, and the builder compares the text's hash.
    pub stamp: u64,
    /// Its text, or how to read it: read only when the stamp moved.
    pub text: Text<'a>,
    /// Its parse, when the source keeps one (else the text is parsed).
    pub parsed: Option<&'a ParsedPage>,
    /// Milliseconds since 1970.
    pub mtime: u64,
    /// A `.base` file, not a note.
    pub is_base: bool,
}

/// A note's text as a [`NoteSource`] gives it.
#[derive(Clone, Copy)]
pub enum Text<'a> {
    /// Held already.
    Here(&'a str),
    /// Read when needed (from disk): `None` when it is gone.
    Load(&'a dyn Fn() -> Option<String>),
}

impl Text<'_> {
    /// The text: `None` when it could not be read.
    #[must_use]
    pub fn get(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Self::Here(t) => Some(std::borrow::Cow::Borrowed(t)),
            Self::Load(load) => load().map(std::borrow::Cow::Owned),
        }
    }
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

/// How many times a sync reads again when a write comes between two
/// chunks, waiting longer each time, before it takes what it read (and
/// reads again on the next sync).
pub const TRIES: u32 = 8;

/// How many deltas a builder keeps for [`Builder::since`].
pub const KEPT: usize = 256;

/// A hash of a note's text (what [`Note::stamp`] may be).
#[must_use]
pub fn hash_of(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn new_epoch() -> u64 {
    uuid::Uuid::new_v4().as_u64_pair().0
}

/// What one read of a source saw: each note's path, stamp, text hash and
/// new entry (`None`: unchanged).
type Seen = Vec<(Arc<str>, u64, u64, Option<Entry>)>;

/// Keeps an index from a [`NoteSource`], a note at a time.
pub struct Builder {
    index: Arc<VaultIndex>,
    /// Each note's stamp and its text's hash when its entry was made.
    stamps: BTreeMap<Arc<str>, (u64, u64)>,
    /// The source's revision last synced from, when that read was whole.
    synced: Option<u64>,
    /// The latest deltas, oldest first (the first full build is not one).
    deltas: VecDeque<IndexDelta>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// A builder of a new history (a random epoch).
    #[must_use]
    pub fn new() -> Self {
        Self {
            index: Arc::new(VaultIndex {
                epoch: new_epoch(),
                ..VaultIndex::default()
            }),
            stamps: BTreeMap::new(),
            synced: None,
            deltas: VecDeque::new(),
        }
    }

    /// The index now.
    #[must_use]
    pub const fn index(&self) -> &Arc<VaultIndex> {
        &self.index
    }

    /// Its history's epoch.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.index.epoch
    }

    /// Brings the index up to `source`: each note whose stamp moved and
    /// whose text changed gets a new entry, the notes gone go. The delta
    /// (empty when nothing changed, or the source has not moved).
    pub fn sync(&mut self, source: &dyn NoteSource) -> IndexDelta {
        if self.synced.is_some() && self.synced == Some(source.revision()) {
            return IndexDelta::none(self.index.epoch, self.index.revision);
        }
        let mut names = std::mem::take(&mut Arc::make_mut(&mut self.index).names);
        // A write between two chunks starts it again, waiting a little
        // longer each time; past `TRIES` it takes the last read, never
        // holding the vault for more than a chunk, and reads again on the
        // next sync.
        let mut whole = None;
        let mut last = None;
        for attempt in 0..TRIES {
            match self.read(source, &mut names) {
                (seen, Some(revision)) => {
                    whole = Some(revision);
                    last = Some(seen);
                    break;
                }
                (seen, None) => {
                    last = Some(seen);
                    pause(attempt);
                }
            }
        }
        let seen = last.unwrap_or_default();
        let mut put = Vec::new();
        let mut present = BTreeSet::new();
        for (path, stamp, hash, entry) in seen {
            present.insert(Arc::clone(&path));
            self.stamps.insert(Arc::clone(&path), (stamp, hash));
            if let Some(entry) = entry {
                put.push(Arc::new(entry));
            }
        }
        let gone: Vec<Arc<str>> = self
            .index
            .entries
            .keys()
            .filter(|p| !present.contains(*p))
            .cloned()
            .collect();
        for p in &gone {
            self.stamps.remove(p);
        }
        self.synced = whole;
        let delta = self.delta(put, gone);
        let index = Arc::make_mut(&mut self.index);
        index.names = names;
        index.names.prune();
        delta
    }

    /// One read of `source`, [`CHUNK`] notes at a time, and the revision
    /// it read at; no revision when the source moved between two chunks.
    fn read(&self, source: &dyn NoteSource, names: &mut Interner) -> (Seen, Option<u64>) {
        let mut seen: Seen = Vec::new();
        let mut at = None;
        let mut whole = true;
        let mut from: usize = 0;
        loop {
            let mut got: usize = 0;
            let (revision, total) = source.visit(from, CHUNK, &mut |note: Note<'_>| {
                got = got.saturating_add(1);
                let was = self.stamps.get_key_value(note.path);
                let path = was.map_or_else(|| Arc::from(note.path), |(p, _)| Arc::clone(p));
                let old = was.map(|(_, s)| *s);
                if let Some((stamp, hash)) = old.filter(|(s, _)| *s == note.stamp) {
                    seen.push((path, stamp, hash, None));
                    return;
                }
                // Gone between the listing and the read: not present.
                let Some(text) = note.text.get() else { return };
                let hash = hash_of(&text);
                if old.is_some_and(|(_, h)| h == hash) {
                    seen.push((path, note.stamp, hash, None));
                    return;
                }
                let entry = if note.is_base {
                    Entry::of_base(note.path, note.mtime, &text)
                } else if let Some(parsed) = note.parsed {
                    Entry::of_note(note.path, note.mtime, parsed, names)
                } else {
                    Entry::of_markdown(note.path, note.mtime, &text, names)
                };
                seen.push((path, note.stamp, hash, Some(entry)));
            });
            whole &= *at.get_or_insert(revision) == revision;
            from = from.saturating_add(got);
            if got == 0 || from >= total {
                return (seen, at.filter(|_| whole));
            }
        }
    }

    /// Records a delta of `put` and `gone` and applies it. The first full
    /// build is no delta: a reader that has nothing takes a snapshot.
    fn delta(&mut self, put: Vec<Arc<Entry>>, gone: Vec<Arc<str>>) -> IndexDelta {
        let (epoch, from) = (self.index.epoch, self.index.revision);
        if put.is_empty() && gone.is_empty() {
            return IndexDelta::none(epoch, from);
        }
        let delta = IndexDelta {
            v: WIRE,
            epoch,
            from,
            to: from.saturating_add(1),
            put,
            gone,
        };
        // It starts where the index is: it applies.
        let _ = Arc::make_mut(&mut self.index).apply(&delta);
        if from > 0 {
            self.deltas.push_back(delta.clone());
            while self.deltas.len() > KEPT {
                self.deltas.pop_front();
            }
        }
        delta
    }

    /// Everything since revision `from` of history `epoch`, as one delta;
    /// `None` when that is another history or further back than the
    /// deltas kept (take a snapshot instead).
    #[must_use]
    pub fn since(&self, epoch: u64, from: u64) -> Option<IndexDelta> {
        if epoch != self.index.epoch {
            return None;
        }
        if from == self.index.revision {
            return Some(IndexDelta::none(epoch, from));
        }
        let start = self.deltas.iter().position(|d| d.from == from)?;
        let tail: Vec<IndexDelta> = self.deltas.iter().skip(start).cloned().collect();
        IndexDelta::merged(&tail)
    }
}

/// Waits a little before reading again: longer each `attempt`. A browser
/// has no thread to block, so there it does not.
#[allow(clippy::missing_const_for_fn)]
fn pause(attempt: u32) {
    #[cfg(not(target_family = "wasm"))]
    std::thread::sleep(std::time::Duration::from_micros(250_u64 << attempt.min(8)));
    #[cfg(target_family = "wasm")]
    let _ = attempt;
}

/// A [`Builder`] over a [`NoteSource`], kept up to it on every read: a
/// desk's, a phone's or the node's index.
pub struct Built<S> {
    source: S,
    builder: Mutex<Builder>,
}

impl<S: NoteSource> Built<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self {
            source,
            builder: Mutex::new(Builder::new()),
        }
    }

    /// Brings the index up to the source; the delta.
    pub fn sync(&self) -> IndexDelta {
        self.builder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sync(&self.source)
    }

    /// Everything since revision `from` of history `epoch`
    /// ([`Builder::since`]), brought up to the source first.
    pub fn since(&self, epoch: u64, from: u64) -> Option<IndexDelta> {
        let mut b = self.builder.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = b.sync(&self.source);
        b.since(epoch, from)
    }

    /// The source.
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// The index, brought up to the source first.
    pub fn index(&self) -> Arc<VaultIndex> {
        let mut b = self.builder.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = b.sync(&self.source);
        Arc::clone(b.index())
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::as_conversions,
        clippy::arithmetic_side_effects,
        clippy::format_collect
    )]

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
                    text: Text::Here(text),
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

    fn paths(d: &IndexDelta) -> Vec<&str> {
        d.put.iter().map(|e| &*e.path).collect()
    }

    /// An entry holds what queries read and nothing of the body's prose,
    /// on the wire too.
    #[test]
    fn an_entry_is_the_notes_facts() {
        let e = Entry::of_markdown(
            "Songs/A.md",
            7,
            "---\nstatus: todo\ntags: [song]\n---\n# Amazing\n\nSome prose #live with [[B]] and [[B]] again #live.\n\n## Parts\n- [ ] learn it\n- [x] chart it\n",
            &mut Interner::default(),
        );
        assert_eq!(e.title(), "Amazing");
        assert_eq!(e.props["status"], "todo");
        assert_eq!(&*e.tags, [Arc::from("live")]);
        assert_eq!(&*e.links, [Arc::from("B")]);
        assert_eq!(e.headings.len(), 2);
        assert_eq!(e.tasks.iter().map(|(m, _)| *m).collect::<String>(), " x");
        let wire = serde_json::to_string(&e).unwrap();
        assert!(wire.contains("status") && wire.contains("learn it"), "{wire}");
        assert!(!wire.contains("prose"), "{wire}");
        // A title that is the file's name is not sent twice.
        let plain = Entry::of_markdown("N.md", 0, "# N\n", &mut Interner::default());
        assert!(plain.heading_title.is_empty() && plain.title() == "N");
    }

    /// Past the caps an entry keeps what fits and says it is truncated.
    #[test]
    fn an_entry_is_capped() {
        let links: String = (0..MAX_LIST + 10).map(|i| format!("[[L{i}]] ")).collect();
        let big = "x".repeat(MAX_VALUE + 1);
        let e = Entry::of_markdown(
            "A.md",
            0,
            &format!("---\nbig: {big}\nok: 1\n---\n{links}\n# {}\n", "h".repeat(MAX_TEXT * 2)),
            &mut Interner::default(),
        );
        assert!(e.truncated);
        assert_eq!(e.links.len(), MAX_LIST);
        assert!(!e.props.contains_key("big") && e.props.contains_key("ok"));
        assert_eq!(e.headings[0].1.chars().count(), MAX_TEXT);
        assert!(!Entry::of_markdown("B.md", 0, "# B\n", &mut Interner::default()).truncated);
        assert!(Entry::of_base("big.base", 0, &"#".repeat(MAX_BASE + 1)).truncated);
    }

    /// Notes share one string per property key, tag and link, in a builder
    /// and in a replica read off the wire.
    #[test]
    fn strings_are_shared() {
        let mut b = Builder::new();
        let _ = b.sync(&notes(2, &[("A.md", "---\nstatus: a\n---\n#t [[X]]", 1), ("B.md", "---\nstatus: b\n---\n#t [[X]]", 1)]));
        let key = |i: &VaultIndex, p: &str| Arc::clone(i.entries[p].props.keys().next().unwrap());
        assert!(Arc::ptr_eq(&key(b.index(), "A.md"), &key(b.index(), "B.md")));
        let replica = VaultIndex::from_bytes(&b.index().to_bytes()).unwrap();
        assert!(Arc::ptr_eq(&key(&replica, "A.md"), &key(&replica, "B.md")));
        assert!(Arc::ptr_eq(&replica.entries["A.md"].links[0], &replica.entries["B.md"].links[0]));
    }

    /// A sync makes entries for the notes that changed only, deltas carry
    /// them, and a replica that applies the deltas is the index.
    #[test]
    fn deltas_carry_only_what_changed() {
        let mut b = Builder::new();
        let first = b.sync(&notes(2, &[("A.md", "# A", 1), ("B.md", "# B", 1)]));
        assert_eq!((first.put.len(), first.to), (2, 1));
        let mut replica = VaultIndex::from_bytes(&b.index().to_bytes()).unwrap();
        assert_eq!(&replica, &**b.index());

        // Nothing moved: no delta.
        assert!(b.sync(&notes(2, &[("A.md", "# A", 1), ("B.md", "# B", 1)])).is_empty());
        // A's stamp moved but its text did not: no delta.
        assert!(b.sync(&notes(4, &[("A.md", "# A", 2), ("B.md", "# B", 1)])).is_empty());
        // B changed, C came, A went.
        let d = b.sync(&notes(6, &[("B.md", "# Bee", 3), ("C.base", "views: []", 1)]));
        assert_eq!(paths(&d), ["B.md", "C.base"]);
        assert_eq!(d.gone, [Arc::from("A.md")]);
        replica.apply_interned(IndexDelta::from_bytes(&d.to_bytes()).unwrap()).unwrap();
        assert_eq!(&replica, &**b.index());
        assert_eq!(replica.base("C.base"), Some("views: []"));
        // A delta from elsewhere is refused.
        assert!(replica.apply(&first).is_err());
        // Since revision 1, as one delta; the first full build is none.
        assert_eq!(b.since(b.epoch(), 1).unwrap().gone, [Arc::from("A.md")]);
        assert!(b.since(b.epoch(), 0).is_none(), "no delta of the first build");
        assert!(b.since(b.epoch(), 99).is_none());
    }

    /// A builder made again (a node restarted) is another history: an old
    /// reader's `since` is refused, and its deltas do not apply.
    #[test]
    fn a_new_history_asks_for_a_snapshot() {
        let v = &[("A.md", "# A", 1), ("B.md", "# B", 1)];
        let mut old = Builder::new();
        let _ = old.sync(&notes(2, v));
        let _ = old.sync(&notes(4, &[("A.md", "# Aa", 2), ("B.md", "# B", 1)]));
        let mut replica = VaultIndex::from_bytes(&old.index().to_bytes()).unwrap();
        let mut new = Builder::new();
        let _ = new.sync(&notes(2, v));
        let _ = new.sync(&notes(4, &[("A.md", "# A", 2), ("B.md", "# Bee", 2)]));
        assert_eq!(new.index().revision, replica.revision, "the same revision, another history");
        assert!(new.since(replica.epoch, replica.revision).is_none());
        let d = new.sync(&notes(6, &[("A.md", "# A", 2), ("B.md", "# B3", 3)]));
        assert!(replica.apply(&d).is_err());
    }

    /// A reader that missed a delta cannot apply the next one; it asks
    /// `since` its revision, or takes a snapshot past the kept deltas.
    #[test]
    fn a_missed_delta_is_caught_up_or_snapshotted() {
        let mut b = Builder::new();
        let _ = b.sync(&notes(2, &[("A.md", "# A", 1)]));
        let mut replica = VaultIndex::from_bytes(&b.index().to_bytes()).unwrap();
        let _missed = b.sync(&notes(4, &[("A.md", "# A2", 2)]));
        let next = b.sync(&notes(6, &[("A.md", "# A3", 3)]));
        assert!(replica.apply(&next).is_err());
        replica.apply_interned(b.since(replica.epoch, replica.revision).unwrap()).unwrap();
        assert_eq!(&replica, &**b.index());
        for i in 0..=KEPT {
            let _ = b.sync(&notes(8 + 2 * i as u64, &[("A.md", &format!("# A{i}x"), 10 + i as u64)]));
        }
        assert!(b.since(replica.epoch, replica.revision).is_none(), "past the kept deltas");
        let fresh = VaultIndex::from_bytes(&b.index().to_bytes()).unwrap();
        assert_eq!(&fresh, &**b.index());
    }

    /// The wire carries its version, and a wrong one is refused.
    #[test]
    fn the_wire_has_a_version() {
        let mut b = Builder::new();
        let _ = b.sync(&notes(2, &[("A.md", "# A", 1)]));
        let text = String::from_utf8(b.index().to_bytes()).unwrap();
        assert!(text.contains("\"v\":1") && text.contains("\"epoch\""), "{text}");
        assert!(VaultIndex::from_bytes(text.replace("\"v\":1", "\"v\":9").as_bytes()).is_err());
    }

    /// A write between two chunks reads again; one on every read leaves a
    /// read taken but not trusted, read whole on the next sync.
    #[test]
    fn a_torn_read_is_read_again() {
        struct Moving(std::cell::Cell<u64>, Vec<(String, String)>);
        impl NoteSource for Moving {
            fn revision(&self) -> u64 {
                self.0.get()
            }
            fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
                self.0.set(self.0.get() + 2);
                for (path, text) in self.1.iter().skip(from).take(n) {
                    each(Note { path, stamp: 1, text: Text::Here(text), parsed: None, mtime: 0, is_base: false });
                }
                (self.0.get(), self.1.len())
            }
        }
        let many: Vec<(String, String)> = (0..CHUNK * 2).map(|i| (format!("N{i}.md"), format!("# {i}"))).collect();
        let mut b = Builder::new();
        let d = b.sync(&Moving(std::cell::Cell::new(0), many));
        assert_eq!(d.put.len(), CHUNK * 2, "everything read");
        assert!(b.synced.is_none(), "but not trusted: read again next time");
    }

    /// A reader sees only what it may: hidden entries are never named.
    #[test]
    fn a_reader_sees_what_it_may() {
        let mut b = Builder::new();
        let _ = b.sync(&notes(2, &[("Public/A.md", "# A", 1), ("Private/B.md", "# B", 1)]));
        let public = |p: &str| p.starts_with("Public/");
        assert_eq!(b.index().filtered(public).entries.len(), 1);
        let d = b.sync(&notes(4, &[("Public/A.md", "# A", 1), ("Private/B.md", "# Bee", 2)]));
        assert!(d.filtered(public).is_empty(), "a hidden note is never named");
        let d = b.sync(&notes(6, &[("Private/A.md", "# A", 3), ("Private/B.md", "# Bee", 2)]));
        assert_eq!(d.filtered(public).gone, [Arc::from("Public/A.md")], "moved out of view: gone");
    }
}
