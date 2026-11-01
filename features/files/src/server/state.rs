//! One org's files as data: its roots and their trees, labels, grants,
//! uploads in flight and quota — and the tree operations over it, pure
//! (the clock and the actor come in as arguments).

use std::collections::{BTreeMap, BTreeSet};

use facet::Facet;

use crate::access::Grant;
use crate::error::FilesFault;
use crate::id::{ContentId, RootId, UploadId, VersionId};
use crate::model::{
    ByteRange, Entry, EntryKind, FilesEvent, OnConflict, Outcome, Root, UploadSpec, Version,
    VersionLabel,
};
use crate::path::RootPath;

/// Everything an org keeps, persisted as one JSON document.
#[derive(Debug, Clone, Default, Facet)]
pub struct State {
    pub roots: Vec<RootState>,
    pub labels: Vec<VersionLabel>,
    pub grants: Vec<Grant>,
    pub uploads: Vec<Upload>,
    /// Bytes the org's live files may hold; `None` is unbounded.
    pub quota: Option<u64>,
}

/// A root and its tree, by path (the root itself is not a node).
#[derive(Debug, Clone, Facet)]
pub struct RootState {
    pub root: Root,
    pub nodes: BTreeMap<String, Node>,
}

/// A file (its versions, newest last) or a folder.
#[derive(Debug, Clone, Facet)]
pub struct Node {
    pub kind: EntryKind,
    pub versions: Vec<Version>,
    pub modified_us: i64,
}

/// An upload in flight.
#[derive(Debug, Clone, Facet)]
pub struct Upload {
    pub id: UploadId,
    pub spec: UploadSpec,
    /// Ranges received, merged and in order.
    pub received: Vec<ByteRange>,
    /// Who began it: only they may send, land or abort it.
    pub by: String,
    /// Whether bytes are staged on disk; `false` when the org already
    /// held the content and nothing was sent.
    pub staged: bool,
}

impl Upload {
    /// What is still to come.
    pub fn needed(&self) -> Vec<ByteRange> {
        if !self.staged {
            return Vec::new();
        }
        gaps(self.spec.size, &self.received)
    }

    pub fn received_bytes(&self) -> u64 {
        self.received.iter().map(ByteRange::len).sum()
    }
}

impl Node {
    pub const fn dir(now: i64) -> Self {
        Self {
            kind: EntryKind::Directory,
            versions: Vec::new(),
            modified_us: now,
        }
    }

    pub fn current(&self) -> Option<&Version> {
        self.versions.last()
    }

    /// A folder, or a file whose last version is not a delete.
    pub fn is_live(&self) -> bool {
        match self.kind {
            EntryKind::Directory => true,
            EntryKind::File => self.current().is_some_and(|v| v.content.is_some()),
        }
    }

    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    /// Bytes this node holds now (0 for a folder or a deleted file).
    pub fn live_size(&self) -> u64 {
        if self.is_live() {
            self.current().map_or(0, |v| v.size)
        } else {
            0
        }
    }

    pub fn entry(&self, root_id: RootId, path: RootPath) -> Entry {
        let current = self.current().filter(|_| self.is_file());
        Entry {
            root_id,
            path,
            kind: self.kind,
            size: current.map_or(0, |v| v.size),
            content: current.and_then(|v| v.content.clone()),
            version: current.map(|v| v.id),
            modified_us: self.modified_us,
        }
    }
}

/// The root itself, as an entry.
pub const fn root_entry(root: &Root) -> Entry {
    Entry {
        root_id: root.id,
        path: RootPath::root(),
        kind: EntryKind::Directory,
        size: 0,
        content: None,
        version: None,
        modified_us: root.created_us,
    }
}

/// `ranges` with `range` added, merged and in order.
pub fn add_range(ranges: &mut Vec<ByteRange>, range: ByteRange) {
    if range.is_empty() {
        return;
    }
    ranges.push(range);
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<ByteRange> = Vec::with_capacity(ranges.len());
    for r in ranges.drain(..) {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    *ranges = merged;
}

/// What of `[0, size)` `received` (merged, in order) does not cover.
pub fn gaps(size: u64, received: &[ByteRange]) -> Vec<ByteRange> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in received {
        if r.start > at {
            out.push(ByteRange {
                start: at,
                end: r.start.min(size),
            });
        }
        at = at.max(r.end);
    }
    if at < size {
        out.push(ByteRange {
            start: at,
            end: size,
        });
    }
    out.retain(|r| !r.is_empty());
    out
}

impl State {
    pub fn root(&self, id: RootId) -> Result<&RootState, FilesFault> {
        self.roots
            .iter()
            .find(|r| r.root.id == id)
            .ok_or(FilesFault::RootNotFound(id))
    }

    pub fn root_mut(&mut self, id: RootId) -> Result<&mut RootState, FilesFault> {
        self.roots
            .iter_mut()
            .find(|r| r.root.id == id)
            .ok_or(FilesFault::RootNotFound(id))
    }

    /// Bytes the org's live files hold.
    pub fn used(&self) -> u64 {
        self.roots
            .iter()
            .flat_map(|r| r.nodes.values())
            .map(Node::live_size)
            .sum()
    }

    /// Whether any version of any file in the org is this content: the
    /// org has these bytes, so an upload of them need send nothing. Only
    /// the org's own: content another org holds is not proof of having
    /// the bytes.
    pub fn references(&self, content: &ContentId) -> bool {
        self.roots
            .iter()
            .flat_map(|r| r.nodes.values())
            .flat_map(|n| n.versions.iter())
            .any(|v| v.content.as_ref() == Some(content))
    }
}

/// Who and when, for what a batch writes.
pub struct Stamp<'a> {
    pub now: i64,
    pub by: &'a str,
    pub mint: &'a mut dyn FnMut() -> VersionId,
}

impl Stamp<'_> {
    fn version(&mut self, content: Option<ContentId>, size: u64) -> Version {
        Version {
            id: (self.mint)(),
            content,
            size,
            by: self.by.to_owned(),
            at_us: self.now,
        }
    }
}

/// A batch of tree changes to one root: applied to the nodes in place
/// (the caller works on a copy and keeps it only on success), recording
/// what changed for the events.
pub struct Batch<'a, 's> {
    pub root_id: RootId,
    pub nodes: &'a mut BTreeMap<String, Node>,
    pub stamp: Stamp<'s>,
    changed: BTreeSet<String>,
    removed: BTreeSet<String>,
}

impl<'a, 's> Batch<'a, 's> {
    pub const fn new(
        root_id: RootId,
        nodes: &'a mut BTreeMap<String, Node>,
        stamp: Stamp<'s>,
    ) -> Self {
        Self {
            root_id,
            nodes,
            stamp,
            changed: BTreeSet::new(),
            removed: BTreeSet::new(),
        }
    }

    pub fn live(&self, path: &RootPath) -> Option<&Node> {
        self.nodes.get(path.as_str()).filter(|n| n.is_live())
    }

    /// The live nodes at and under `path`, parents first.
    fn subtree(&self, path: &RootPath) -> Vec<(RootPath, Node)> {
        self.nodes
            .iter()
            .filter(|(k, n)| n.is_live() && within(k, path))
            .filter_map(|(k, n)| RootPath::parse(k).ok().map(|p| (p, n.clone())))
            .collect()
    }

    /// `path` and every folder above it are folders, made if missing.
    pub fn ensure_dir(&mut self, path: &RootPath) -> Result<(), FilesFault> {
        let mut chain = path.ancestors();
        if !path.is_root() {
            chain.push(path.clone());
        }
        for dir in chain {
            match self.nodes.get(dir.as_str()) {
                Some(n) if n.kind == EntryKind::Directory => {}
                Some(n) if n.is_live() => return Err(FilesFault::NotADirectory(dir)),
                _ => {
                    self.nodes
                        .insert(dir.as_str().to_owned(), Node::dir(self.stamp.now));
                    self.changed.insert(dir.as_str().to_owned());
                }
            }
        }
        Ok(())
    }

    /// A new version of the file at `path` (made, with its folders, if
    /// there is none). The caller has checked no live folder is there.
    pub fn write_file(
        &mut self,
        path: &RootPath,
        content: ContentId,
        size: u64,
    ) -> Result<(), FilesFault> {
        if let Some(parent) = path.parent() {
            self.ensure_dir(&parent)?;
        }
        let version = self.stamp.version(Some(content), size);
        match self.nodes.get_mut(path.as_str()) {
            Some(node) if node.is_file() => {
                node.versions.push(version);
                node.modified_us = self.stamp.now;
            }
            Some(_) => return Err(FilesFault::NotAFile(path.clone())),
            None => {
                self.nodes.insert(
                    path.as_str().to_owned(),
                    Node {
                        kind: EntryKind::File,
                        versions: vec![version],
                        modified_us: self.stamp.now,
                    },
                );
            }
        }
        self.changed.insert(path.as_str().to_owned());
        Ok(())
    }

    /// Delete `path` and everything under it: folders go, files get a
    /// delete as their newest version (their history stays).
    pub fn remove(&mut self, path: &RootPath) -> Result<(), FilesFault> {
        if path.is_root() {
            return Err(FilesFault::invalid("the root itself cannot be deleted"));
        }
        if self.live(path).is_none() {
            return Err(FilesFault::PathNotFound(path.clone()));
        }
        for (p, node) in self.subtree(path) {
            let key = p.as_str().to_owned();
            if node.is_file() {
                let tombstone = self.stamp.version(None, 0);
                if let Some(n) = self.nodes.get_mut(&key) {
                    n.versions.push(tombstone);
                    n.modified_us = self.stamp.now;
                }
            } else {
                self.nodes.remove(&key);
            }
            self.changed.remove(&key);
            self.removed.insert(key);
        }
        Ok(())
    }

    /// Move (`keep_source: false`) or copy `from` to `to`.
    pub fn relocate(
        &mut self,
        from: &RootPath,
        to: &RootPath,
        policy: OnConflict,
        keep_source: bool,
    ) -> Result<Outcome, FilesFault> {
        if from.is_root() || to.is_root() {
            return Err(FilesFault::invalid(
                "the root itself cannot be moved or copied",
            ));
        }
        let source = self
            .live(from)
            .ok_or_else(|| FilesFault::PathNotFound(from.clone()))?
            .clone();
        if to.is_within(from) {
            return Err(FilesFault::IntoSelf {
                from: from.clone(),
                to: to.clone(),
            });
        }
        let mut dest = to.clone();
        if let Some(occupant) = self.live(to) {
            match policy {
                OnConflict::Fail => return Err(FilesFault::Exists { path: to.clone() }),
                OnConflict::KeepExisting => {
                    return Ok(Outcome {
                        path: from.clone(),
                        landed_at: None,
                        skipped: true,
                    });
                }
                OnConflict::KeepBoth => dest = self.free_name(to),
                OnConflict::Replace => {
                    if from.is_within(to) {
                        return Err(FilesFault::invalid(format!(
                            "{to} holds {from}: replacing it would delete the source"
                        )));
                    }
                    // File onto file: a new version of the destination.
                    // Anything else: the destination goes first.
                    if !(occupant.is_file() && source.is_file()) {
                        self.remove(to)?;
                    }
                }
            }
        }
        if let Some(parent) = dest.parent() {
            self.ensure_dir(&parent)?;
        }
        for (path, node) in self.subtree(from) {
            let Some(target) = path.rebase(from, &dest) else {
                continue;
            };
            let key = target.as_str().to_owned();
            let merges = match (self.nodes.get(&key), node.kind) {
                (Some(existing), EntryKind::File) => existing.is_file(),
                (Some(existing), EntryKind::Directory) => existing.kind == EntryKind::Directory,
                (None, _) => false,
            };
            if merges {
                if let Some(current) = node.current().filter(|_| node.is_file()) {
                    let version = self.stamp.version(current.content.clone(), current.size);
                    if let Some(existing) = self.nodes.get_mut(&key) {
                        existing.versions.push(version);
                        existing.modified_us = self.stamp.now;
                    }
                }
            } else {
                let placed = if keep_source {
                    self.fresh(&node)
                } else {
                    node.clone()
                };
                self.nodes.insert(key.clone(), placed);
            }
            self.removed.remove(&key);
            self.changed.insert(key);
            if !keep_source {
                self.drop_source(&path, &node, merges);
            }
        }
        Ok(Outcome {
            path: from.clone(),
            landed_at: Some(dest),
            skipped: false,
        })
    }

    /// The source of a move, once its node has landed: gone — or, for a
    /// file that became a new version of another, a delete in its own
    /// history, so that history is not lost.
    fn drop_source(&mut self, path: &RootPath, node: &Node, merged: bool) {
        let key = path.as_str().to_owned();
        if merged && node.is_file() {
            let tombstone = self.stamp.version(None, 0);
            if let Some(n) = self.nodes.get_mut(&key) {
                n.versions.push(tombstone);
                n.modified_us = self.stamp.now;
            }
        } else {
            self.nodes.remove(&key);
        }
        self.changed.remove(&key);
        self.removed.insert(key);
    }

    /// A copy of `node` with a history of its own: one version, the same
    /// content.
    fn fresh(&mut self, node: &Node) -> Node {
        let versions = node
            .current()
            .filter(|_| node.is_file())
            .map(|v| vec![self.stamp.version(v.content.clone(), v.size)])
            .unwrap_or_default();
        Node {
            kind: node.kind,
            versions,
            modified_us: self.stamp.now,
        }
    }

    /// `to`, or `name (2).ext`, `name (3).ext`, … — the first that is
    /// free.
    pub fn free_name(&self, to: &RootPath) -> RootPath {
        let name = to.name().unwrap_or_default();
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
            _ => (name, None),
        };
        // Bounded: a folder cannot hold four billion files of one name.
        (2..u32::MAX)
            .map(|n| {
                ext.map_or_else(
                    || format!("{stem} ({n})"),
                    |ext| format!("{stem} ({n}).{ext}"),
                )
            })
            .filter_map(|candidate| to.with_name(&candidate).ok())
            .find(|candidate| self.live(candidate).is_none())
            .unwrap_or_else(|| to.clone())
    }

    /// The events this batch makes.
    pub fn events(&self) -> Vec<FilesEvent> {
        let mut out = Vec::new();
        let entries: Vec<Entry> = self
            .changed
            .iter()
            .filter_map(|k| {
                let node = self.nodes.get(k).filter(|n| n.is_live())?;
                let path = RootPath::parse(k).ok()?;
                Some(node.entry(self.root_id, path))
            })
            .collect();
        if !entries.is_empty() {
            out.push(FilesEvent::Changed {
                root_id: self.root_id,
                entries,
            });
        }
        let paths: Vec<RootPath> = self
            .removed
            .iter()
            .filter_map(|k| RootPath::parse(k).ok())
            .collect();
        if !paths.is_empty() {
            out.push(FilesEvent::Removed {
                root_id: self.root_id,
                paths,
            });
        }
        out
    }
}

/// Whether the node key `key` is `path` or under it.
fn within(key: &str, path: &RootPath) -> bool {
    path.is_root()
        || key == path.as_str()
        || key
            .strip_prefix(path.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn p(s: &str) -> RootPath {
        RootPath::parse(s).unwrap()
    }

    fn c(s: &str) -> ContentId {
        ContentId::new(s)
    }

    struct Tree {
        nodes: BTreeMap<String, Node>,
        next: u128,
    }

    impl Tree {
        fn new() -> Self {
            Self {
                nodes: BTreeMap::new(),
                next: 0,
            }
        }

        fn batch<T>(&mut self, f: impl FnOnce(&mut Batch<'_, '_>) -> T) -> (T, Vec<FilesEvent>) {
            let next = &mut self.next;
            let mut mint = move || {
                *next = next.wrapping_add(1);
                VersionId::new(Uuid::from_u128(*next))
            };
            let mut batch = Batch::new(
                RootId::new(Uuid::nil()),
                &mut self.nodes,
                Stamp {
                    now: 1,
                    by: "u",
                    mint: &mut mint,
                },
            );
            let out = f(&mut batch);
            let events = batch.events();
            (out, events)
        }

        fn live(&self) -> Vec<&str> {
            self.nodes
                .iter()
                .filter(|(_, n)| n.is_live())
                .map(|(k, _)| k.as_str())
                .collect()
        }
    }

    #[test]
    fn a_save_makes_its_folders_and_a_second_is_a_version() {
        let mut t = Tree::new();
        t.batch(|b| b.write_file(&p("a/b/x.wav"), c("1"), 3))
            .0
            .unwrap();
        t.batch(|b| b.write_file(&p("a/b/x.wav"), c("2"), 4))
            .0
            .unwrap();
        assert_eq!(t.live(), vec!["a", "a/b", "a/b/x.wav"]);
        assert_eq!(t.nodes["a/b/x.wav"].versions.len(), 2);
        assert!(t.batch(|b| b.write_file(&p("a"), c("3"), 1)).0.is_err());
    }

    #[test]
    fn a_move_takes_the_history_and_a_copy_starts_its_own() {
        let mut t = Tree::new();
        t.batch(|b| b.write_file(&p("a/x"), c("1"), 1)).0.unwrap();
        t.batch(|b| b.write_file(&p("a/x"), c("2"), 2)).0.unwrap();
        let (copied, _) = t.batch(|b| b.relocate(&p("a"), &p("b"), OnConflict::Fail, true));
        assert_eq!(copied.unwrap().landed_at, Some(p("b")));
        assert_eq!(t.nodes["b/x"].versions.len(), 1);
        let (moved, events) = t.batch(|b| b.relocate(&p("a"), &p("c/d"), OnConflict::Fail, false));
        moved.unwrap();
        assert_eq!(t.live(), vec!["b", "b/x", "c", "c/d", "c/d/x"]);
        assert_eq!(t.nodes["c/d/x"].versions.len(), 2);
        assert!(
            matches!(&events[1], FilesEvent::Removed { paths, .. } if paths == &vec![p("a"), p("a/x")])
        );
    }

    #[test]
    fn a_taken_destination_is_the_callers_choice() {
        let mut t = Tree::new();
        t.batch(|b| b.write_file(&p("x.wav"), c("1"), 1)).0.unwrap();
        t.batch(|b| b.write_file(&p("y.wav"), c("2"), 2)).0.unwrap();
        let fail = t
            .batch(|b| b.relocate(&p("x.wav"), &p("y.wav"), OnConflict::Fail, false))
            .0;
        assert_eq!(fail, Err(FilesFault::Exists { path: p("y.wav") }));
        let skip = t
            .batch(|b| b.relocate(&p("x.wav"), &p("y.wav"), OnConflict::KeepExisting, false))
            .0
            .unwrap();
        assert!(skip.skipped);
        let both = t
            .batch(|b| b.relocate(&p("x.wav"), &p("y.wav"), OnConflict::KeepBoth, true))
            .0
            .unwrap();
        assert_eq!(both.landed_at, Some(p("y (2).wav")));
        t.batch(|b| b.relocate(&p("x.wav"), &p("y.wav"), OnConflict::Replace, false))
            .0
            .unwrap();
        // y.wav: its own version, then x's; x.wav: its own, then a delete.
        assert_eq!(t.nodes["y.wav"].versions.len(), 2);
        assert_eq!(t.nodes["y.wav"].current().unwrap().content, Some(c("1")));
        assert!(!t.nodes["x.wav"].is_live());
        assert_eq!(t.live(), vec!["y (2).wav", "y.wav"]);
    }

    #[test]
    fn into_itself_is_refused_and_a_delete_keeps_history() {
        let mut t = Tree::new();
        t.batch(|b| b.write_file(&p("a/x"), c("1"), 5)).0.unwrap();
        let into = t
            .batch(|b| b.relocate(&p("a"), &p("a/b"), OnConflict::Fail, false))
            .0;
        assert!(matches!(into, Err(FilesFault::IntoSelf { .. })));
        t.batch(|b| b.remove(&p("a"))).0.unwrap();
        assert!(t.live().is_empty());
        assert_eq!(t.nodes["a/x"].versions.len(), 2, "the delete is a version");
        assert_eq!(
            t.batch(|b| b.remove(&p("a"))).0,
            Err(FilesFault::PathNotFound(p("a")))
        );
    }

    #[test]
    fn upload_ranges_merge_and_leave_gaps() {
        let mut got = Vec::new();
        add_range(&mut got, ByteRange { start: 10, end: 20 });
        add_range(&mut got, ByteRange { start: 0, end: 5 });
        add_range(&mut got, ByteRange { start: 5, end: 10 });
        assert_eq!(got, vec![ByteRange { start: 0, end: 20 }]);
        add_range(&mut got, ByteRange { start: 30, end: 40 });
        assert_eq!(
            gaps(50, &got),
            vec![
                ByteRange { start: 20, end: 30 },
                ByteRange { start: 40, end: 50 }
            ]
        );
        assert!(gaps(20, &[ByteRange { start: 0, end: 20 }]).is_empty());
    }
}
