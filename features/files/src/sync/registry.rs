//! The registry: every live file of every root this device may see, in
//! one local index that is searched offline.
//!
//! Built from the manifests (each root re-read only when its document
//! moved: the digest of its version vector), persisted one file per root
//! under `<data>/registry/`, and loaded before the first pass, so search
//! answers at start. A root's rows are kept as one lowercase haystack
//! (`\n` between rows): a query finds its longest word with `memmem`
//! over it and checks the other words in the rows it lands in.
//!
//! Every hash any version names maps to the orgs that name it: that is
//! what scopes the ledger (an org's ledger attests only its own bytes).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;
use std::path::{Path, PathBuf};

use facet::Facet;
use uuid::Uuid;

use crate::roots::Section;

/// One live file (or a library's asset, or a project itself).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Row {
    /// Root-relative; empty for the root itself.
    pub path: String,
    pub size: u64,
    pub at_us: i64,
    /// sha-256 hex; empty for an asset or the root.
    pub content: String,
    pub versions: u32,
    /// `file`, `project`, or an asset's `type`.
    pub what: String,
    /// An asset's property values, space-separated; searched.
    pub props: String,
    /// An asset's name, on its note's row (else the file name is shown).
    #[facet(default)]
    pub name: String,
}

/// A root's part of the registry, as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RootIndex {
    pub doc_id: String,
    pub org: String,
    pub place: String,
    pub name: String,
    pub section: Section,
    /// The manifest's version-vector digest when this was built.
    pub digest: String,
    pub rows: Vec<Row>,
    /// Every hash any version of the root names, live or not.
    pub hashes: Vec<String>,
}

struct Loaded {
    index: RootIndex,
    /// The rows' lowercase text, `\n`-separated.
    hay: String,
    /// Where each row's text ends in `hay`.
    ends: Vec<usize>,
    /// The root's own text: org, name, place.
    root_hay: String,
}

impl Loaded {
    fn new(index: RootIndex) -> Self {
        let mut hay = String::new();
        let mut ends = Vec::with_capacity(index.rows.len());
        for r in &index.rows {
            hay.push_str(&r.path.to_lowercase());
            if !r.props.is_empty() {
                hay.push(' ');
                hay.push_str(&r.props.to_lowercase());
            }
            ends.push(hay.len());
            hay.push('\n');
        }
        let root_hay = format!("{} {} {}", index.org, index.name, index.place).to_lowercase();
        Self {
            index,
            hay,
            ends,
            root_hay,
        }
    }

    fn row_text(&self, i: usize) -> &str {
        let start = if i == 0 {
            0
        } else {
            self.ends
                .get(i.saturating_sub(1))
                .map_or(0, |e| e.saturating_add(1))
        };
        let end = self.ends.get(i).copied().unwrap_or(0);
        self.hay.get(start..end).unwrap_or_default()
    }
}

/// One thing a search found: a row of a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub doc_id: Uuid,
    pub org: String,
    pub place: String,
    pub root_name: String,
    pub row: Row,
}

/// What a query asks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    /// `sha:<prefix>` (8+ hex) or a bare 64-hex.
    pub hash: Option<String>,
    pub org: Option<String>,
    pub root: Option<String>,
    pub kind: Option<String>,
}

impl Query {
    /// Words, `sha:`, `org:`, `root:`, `kind:`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut q = Self::default();
        for token in text.split_whitespace() {
            let lower = token.to_lowercase();
            let is_hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
            if let Some(h) = lower
                .strip_prefix("sha:")
                .filter(|h| h.len() >= 8 && is_hex(h))
            {
                q.hash = Some(h.to_owned());
            } else if lower.len() == 64 && is_hex(&lower) {
                q.hash = Some(lower);
            } else if let Some(o) = lower.strip_prefix("org:") {
                q.org = Some(o.to_owned());
            } else if let Some(r) = lower.strip_prefix("root:") {
                q.root = Some(r.to_owned());
            } else if let Some(k) = lower.strip_prefix("kind:") {
                q.kind = Some(k.to_owned());
            } else {
                q.words.push(lower);
            }
        }
        q
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.words.is_empty() && self.hash.is_none() && self.root.is_none() && self.kind.is_none()
    }
}

/// The registry.
#[derive(Default)]
pub struct Registry {
    dir: PathBuf,
    roots: BTreeMap<Uuid, Loaded>,
    /// hash → `(root, row)` of the live files holding it (ordered, so a
    /// prefix is a range).
    by_hash: BTreeMap<[u8; 32], Vec<(Uuid, usize)>>,
    /// hash → the roots that name it (any version).
    named_by: HashMap<[u8; 32], Vec<Uuid>>,
    loaded: bool,
}

fn key(hex: &str) -> Option<[u8; 32]> {
    super::verify::node_of(hex)
}

impl Registry {
    /// The registry kept under `dir`; nothing is read until [`Self::load`].
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            ..Self::default()
        }
    }

    /// Read every root's file (once).
    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        let threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .min(12);
        let per = files.len().div_ceil(threads).max(1);
        let read: Vec<RootIndex> = std::thread::scope(|s| {
            // Collected, so every thread is running before the first is joined.
            #[allow(clippy::needless_collect)]
            let handles: Vec<_> = files
                .chunks(per)
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .filter_map(|f| std::fs::read_to_string(f).ok())
                            .filter_map(|t| facet_json::from_str::<RootIndex>(&t).ok())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .flatten()
                .collect()
        });
        for index in read {
            if let Ok(id) = index.doc_id.parse::<Uuid>() {
                self.add(id, index);
            }
        }
    }

    /// Index a root's rows and hashes (after [`Self::drop_root`] of the old).
    fn add(&mut self, id: Uuid, index: RootIndex) {
        for (i, r) in index.rows.iter().enumerate() {
            if let Some(k) = key(&r.content) {
                self.by_hash.entry(k).or_default().push((id, i));
            }
        }
        for h in &index.hashes {
            if let Some(k) = key(h) {
                self.named_by.entry(k).or_default().push(id);
            }
        }
        self.roots.insert(id, Loaded::new(index));
    }

    /// Take a root's rows and hashes out of the indexes.
    fn drop_root(&mut self, id: Uuid) -> bool {
        let Some(old) = self.roots.remove(&id) else {
            return false;
        };
        for r in &old.index.rows {
            if let Some(k) = key(&r.content)
                && let Some(v) = self.by_hash.get_mut(&k)
            {
                v.retain(|(r, _)| *r != id);
                if v.is_empty() {
                    self.by_hash.remove(&k);
                }
            }
        }
        for h in &old.index.hashes {
            if let Some(k) = key(h)
                && let Some(v) = self.named_by.get_mut(&k)
            {
                v.retain(|r| *r != id);
                if v.is_empty() {
                    self.named_by.remove(&k);
                }
            }
        }
        true
    }

    /// The digest a root was last built at.
    #[must_use]
    pub fn digest(&self, doc_id: Uuid) -> Option<&str> {
        self.roots.get(&doc_id).map(|l| l.index.digest.as_str())
    }

    /// Every root in the registry.
    pub fn roots(&self) -> impl Iterator<Item = (Uuid, &RootIndex)> {
        self.roots.iter().map(|(id, l)| (*id, &l.index))
    }

    /// A root's index.
    #[must_use]
    pub fn root(&self, doc_id: Uuid) -> Option<&RootIndex> {
        self.roots.get(&doc_id).map(|l| &l.index)
    }

    /// Put a root's rebuilt index, and persist it.
    pub fn put(&mut self, index: RootIndex) {
        let Ok(id) = index.doc_id.parse::<Uuid>() else {
            return;
        };
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(
            self.file(id),
            facet_json::to_string(&index).unwrap_or_default(),
        );
        self.drop_root(id);
        self.add(id, index);
    }

    /// Forget a root (its access was revoked): its rows and its file.
    pub fn remove(&mut self, doc_id: Uuid) {
        self.drop_root(doc_id);
        let _ = std::fs::remove_file(self.file(doc_id));
    }

    fn file(&self, doc_id: Uuid) -> PathBuf {
        self.dir.join(format!("{doc_id}.json"))
    }

    /// The orgs whose roots name `hash`.
    pub fn orgs_of(&mut self, hash: &str) -> BTreeSet<String> {
        key(hash)
            .and_then(|k| self.named_by.get(&k))
            .into_iter()
            .flatten()
            .filter_map(|id| self.roots.get(id).map(|l| l.index.org.clone()))
            .collect()
    }

    /// Every hash any root names, with its orgs.
    pub fn all_hashes(&mut self) -> Vec<(String, BTreeSet<String>)> {
        self.named_by
            .iter()
            .map(|(k, ids)| {
                let orgs = ids
                    .iter()
                    .filter_map(|id| self.roots.get(id).map(|l| l.index.org.clone()))
                    .collect();
                (super::scan::hex(k), orgs)
            })
            .collect()
    }

    /// Every live file holding `hash`.
    pub fn places(&mut self, hash: &str) -> Vec<Found> {
        let Some(k) = key(hash) else {
            return Vec::new();
        };
        self.by_hash
            .get(&k)
            .into_iter()
            .flatten()
            .filter_map(|(id, i)| self.found(*id, *i))
            .collect()
    }

    fn found(&self, id: Uuid, i: usize) -> Option<Found> {
        let l = self.roots.get(&id)?;
        Some(Found {
            doc_id: id,
            org: l.index.org.clone(),
            place: l.index.place.clone(),
            root_name: l.index.name.clone(),
            row: l.index.rows.get(i)?.clone(),
        })
    }

    /// Up to `limit` rows matching `q`.
    pub fn search(&mut self, q: &Query, limit: usize) -> Vec<Found> {
        if q.is_empty() {
            return Vec::new();
        }
        if let Some(h) = &q.hash {
            return self.by_hash_prefix(h, q, limit);
        }
        let mut out = Vec::new();
        for (id, l) in &self.roots {
            if out.len() >= limit {
                break;
            }
            if !in_scope(&l.index, q) {
                continue;
            }
            // Words the root's own name holds need not be in the row.
            let rest: Vec<&str> = q
                .words
                .iter()
                .map(String::as_str)
                .filter(|w| !l.root_hay.contains(w))
                .collect();
            if rest.is_empty() && q.kind.is_none() {
                // The root itself, once.
                if let Some(i) = l.index.rows.iter().position(|r| r.path.is_empty()) {
                    out.extend(self.found(*id, i));
                }
                continue;
            }
            let longest = rest.iter().copied().max_by_key(|w| w.len());
            let rows: Vec<usize> = longest.map_or_else(
                || (0..l.index.rows.len()).collect(),
                |w| {
                    let finder = memchr::memmem::Finder::new(w.as_bytes());
                    let mut rows = Vec::new();
                    for at in finder.find_iter(l.hay.as_bytes()) {
                        let i = l.ends.partition_point(|e| *e < at);
                        if rows.last() != Some(&i) {
                            rows.push(i);
                        }
                    }
                    rows
                },
            );
            for i in rows {
                if out.len() >= limit {
                    break;
                }
                let text = l.row_text(i);
                let Some(row) = l.index.rows.get(i) else {
                    continue;
                };
                if row.path.is_empty() {
                    continue;
                }
                if rest.iter().all(|w| text.contains(w))
                    && q.kind
                        .as_ref()
                        .is_none_or(|k| row.what.eq_ignore_ascii_case(k))
                {
                    out.extend(self.found(*id, i));
                }
            }
        }
        out
    }

    fn by_hash_prefix(&self, prefix: &str, q: &Query, limit: usize) -> Vec<Found> {
        let pad = |c: char| {
            let mut full = prefix.to_owned();
            while full.len() < 64 {
                full.push(c);
            }
            key(&full)
        };
        let (Some(lo), Some(hi)) = (pad('0'), pad('f')) else {
            return Vec::new();
        };
        self.by_hash
            .range((Bound::Included(lo), Bound::Included(hi)))
            .flat_map(|(_, v)| v.iter().copied())
            .filter(|(id, _)| self.roots.get(id).is_some_and(|l| in_scope(&l.index, q)))
            .filter_map(|(id, i)| self.found(id, i))
            .take(limit)
            .collect()
    }

    /// How many rows in all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.roots.values().map(|l| l.index.rows.len()).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The registry's directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn in_scope(index: &RootIndex, q: &Query) -> bool {
    q.org
        .as_ref()
        .is_none_or(|o| index.org.eq_ignore_ascii_case(o))
        && q.root
            .as_ref()
            .is_none_or(|r| index.place.to_lowercase().contains(r))
}

#[cfg(test)]
#[allow(clippy::as_conversions, clippy::string_slice)]
mod tests {
    use super::*;

    fn row(path: &str, content: &str) -> Row {
        Row {
            path: path.into(),
            size: 1,
            at_us: 1,
            content: content.into(),
            versions: 1,
            what: if path.is_empty() { "project" } else { "file" }.into(),
            props: String::new(),
            name: String::new(),
        }
    }

    fn index(org: &str, name: &str, rows: Vec<Row>) -> RootIndex {
        let place = format!("{org}/Projects/{name}");
        RootIndex {
            doc_id: super::super::manifest::doc_id_for(&place).to_string(),
            org: org.into(),
            name: name.into(),
            place,
            section: Section::Projects,
            digest: "d".into(),
            hashes: rows
                .iter()
                .map(|r| r.content.clone())
                .filter(|c| !c.is_empty())
                .collect(),
            rows,
        }
    }

    #[test]
    fn words_split_between_the_root_and_the_row_and_persist() {
        let dir = tempfile::tempdir().unwrap_or_else(|_| panic!("tmp"));
        let mut reg = Registry::new(dir.path().to_owned());
        let h = "ab".repeat(32);
        reg.put(index(
            "tombrooksmusic",
            "Crescendum Album",
            vec![
                row("", ""),
                row("Crescendum/Audio Files/148-V LEAD-241011_2006.wav", &h),
                row("Crescendum/Crescendum.rpp", &"cd".repeat(32)),
            ],
        ));
        let found = reg.search(&Query::parse("crescendum lead"), 10);
        assert_eq!(found.len(), 1);
        assert!(found[0].row.path.ends_with("2006.wav"));
        assert_eq!(
            reg.search(&Query::parse("crescendum"), 10)[0].row.what,
            "project"
        );
        assert_eq!(
            reg.search(&Query::parse(&format!("sha:{}", &h[..10])), 10)
                .len(),
            1
        );
        assert_eq!(reg.search(&Query::parse("lead org:cbu"), 10).len(), 0);
        assert_eq!(
            reg.orgs_of(&h).into_iter().collect::<Vec<_>>(),
            vec!["tombrooksmusic".to_owned()]
        );

        let mut again = Registry::new(dir.path().to_owned());
        again.load();
        assert_eq!(again.len(), 3);
        assert_eq!(again.places(&h).len(), 1);
    }

    #[test]
    fn a_date_in_a_name_is_a_word_not_a_hash() {
        let q = Query::parse("241011_2006 20241011");
        assert!(q.hash.is_none());
        assert_eq!(q.words.len(), 2);
    }
}
