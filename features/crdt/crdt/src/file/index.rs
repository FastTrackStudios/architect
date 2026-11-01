//! What a file's records add up to: the document's meta, blobs,
//! branches, steps, snapshots and checkpoints, kept in memory and
//! rebuilt by reading the records in order.

use std::collections::BTreeMap;

use loro::{Frontiers, LoroDoc};

use super::format::Record;
use super::{BlobInfo, BranchInfo, FileError, MAIN, Version, VersionKind};

#[derive(Debug, Clone)]
struct Blob {
    kind: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Step {
    seq: u64,
    branch: String,
    label: String,
    at: i64,
    frontiers: Vec<u8>,
    /// Empty once a snapshot folded it in.
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    upto: u64,
    at: i64,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Checkpoint {
    seq: u64,
    branch: String,
    name: String,
    at: i64,
    frontiers: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Branch {
    name: String,
    parent: Option<String>,
    base: Option<Vec<u8>>,
    at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    meta: BTreeMap<String, String>,
    blobs: BTreeMap<String, Blob>,
    /// In the order they were made; `main` first.
    branches: Vec<Branch>,
    steps: Vec<Step>,
    snapshots: BTreeMap<String, Snapshot>,
    checkpoints: Vec<Checkpoint>,
    next_seq: u64,
}

impl Index {
    pub fn from_records(records: impl IntoIterator<Item = Record>) -> Self {
        let mut index = Self::default();
        for r in records {
            index.apply(r);
        }
        index
    }

    /// Adds what `record` says.
    pub fn apply(&mut self, record: Record) {
        match record {
            Record::Meta { key, value } => {
                self.meta.insert(key, value);
            }
            Record::Blob { hash, kind, data } => {
                self.blobs.entry(hash).or_insert(Blob { kind, data });
            }
            Record::Branch {
                name,
                parent,
                base,
                at,
            } => {
                let made = Branch {
                    name,
                    parent,
                    base,
                    at,
                };
                match self.branches.iter_mut().find(|b| b.name == made.name) {
                    Some(b) => *b = made,
                    None => self.branches.push(made),
                }
            }
            Record::DropBranch { name } => {
                self.branches.retain(|b| b.name != name);
                self.steps.retain(|s| s.branch != name);
                self.snapshots.remove(&name);
                self.checkpoints.retain(|c| c.branch != name);
            }
            Record::Step {
                seq,
                branch,
                label,
                at,
                frontiers,
                data,
            } => {
                self.bump(seq);
                // A step already folded keeps only its name and place.
                let folded = self
                    .snapshots
                    .get(&branch)
                    .is_some_and(|snap| seq <= snap.upto);
                self.steps.push(Step {
                    seq,
                    branch,
                    label,
                    at,
                    frontiers,
                    data: if folded { Vec::new() } else { data },
                });
            }
            Record::Snapshot {
                branch,
                upto,
                at,
                data,
            } => {
                for s in &mut self.steps {
                    if s.branch == branch && s.seq <= upto {
                        s.data = Vec::new();
                    }
                }
                self.snapshots.insert(branch, Snapshot { upto, at, data });
            }
            Record::Checkpoint {
                seq,
                branch,
                name,
                at,
                frontiers,
            } => {
                self.bump(seq);
                self.checkpoints
                    .retain(|c| !(c.branch == branch && c.name == name));
                self.checkpoints.push(Checkpoint {
                    seq,
                    branch,
                    name,
                    at,
                    frontiers,
                });
            }
            Record::DropCheckpoint { branch, name } => {
                self.checkpoints
                    .retain(|c| !(c.branch == branch && c.name == name));
            }
        }
    }

    fn bump(&mut self, seq: u64) {
        self.next_seq = self.next_seq.max(seq.saturating_add(1));
    }

    /// The sequence number the next step or checkpoint takes.
    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The last step `branch` has, or 0.
    pub fn last_step(&self, branch: &str) -> u64 {
        self.steps
            .iter()
            .filter(|s| s.branch == branch)
            .map(|s| s.seq)
            .max()
            .unwrap_or(0)
    }

    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta.get(key).map(String::as_str)
    }

    pub fn has_blob(&self, hash: &str) -> bool {
        self.blobs.contains_key(hash)
    }

    pub fn blob(&self, hash: &str) -> Option<&[u8]> {
        self.blobs.get(hash).map(|b| b.data.as_slice())
    }

    pub fn blobs(&self) -> Vec<BlobInfo> {
        self.blobs
            .iter()
            .map(|(hash, b)| BlobInfo {
                hash: hash.clone(),
                kind: b.kind.clone(),
                size: u64::try_from(b.data.len()).unwrap_or(u64::MAX),
            })
            .collect()
    }

    pub fn has_branch(&self, name: &str) -> bool {
        self.branches.iter().any(|b| b.name == name)
    }

    pub fn branches(&self) -> Vec<BranchInfo> {
        let mut out: Vec<BranchInfo> = self
            .branches
            .iter()
            .map(|b| BranchInfo {
                name: b.name.clone(),
                parent: b.parent.clone(),
                at: b.at,
            })
            .collect();
        // `main` first, then oldest first (a stable sort keeps the order
        // they were made in among equals).
        out.sort_by_key(|b| (b.name != MAIN, b.at));
        out
    }

    /// The branches forked from `name`.
    pub fn children(&self, name: &str) -> Vec<String> {
        self.branches
            .iter()
            .filter(|b| b.parent.as_deref() == Some(name))
            .map(|b| b.name.clone())
            .collect()
    }

    pub fn parent(&self, name: &str) -> Option<String> {
        self.branches
            .iter()
            .find(|b| b.name == name)
            .and_then(|b| b.parent.clone())
    }

    /// `branch`'s steps and checkpoints, in the order they were made.
    pub fn versions(&self, branch: &str) -> Result<Vec<Version>, FileError> {
        let mut out: Vec<(u64, Version)> = Vec::new();
        // A step written without frontiers (a raw sync update) is replayed
        // but is not a point to open.
        for s in self
            .steps
            .iter()
            .filter(|s| s.branch == branch && !s.frontiers.is_empty())
        {
            out.push((
                s.seq,
                Version {
                    branch: s.branch.clone(),
                    name: s.label.clone(),
                    at: s.at,
                    kind: VersionKind::Step,
                    frontiers: decode_frontiers(&s.frontiers)?,
                },
            ));
        }
        for c in self.checkpoints.iter().filter(|c| c.branch == branch) {
            out.push((
                c.seq,
                Version {
                    branch: c.branch.clone(),
                    name: c.name.clone(),
                    at: c.at,
                    kind: VersionKind::Checkpoint,
                    frontiers: decode_frontiers(&c.frontiers)?,
                },
            ));
        }
        out.sort_by_key(|(seq, _)| *seq);
        Ok(out.into_iter().map(|(_, v)| v).collect())
    }

    /// `name` as its records leave it: its snapshot, or its parent forked
    /// where it forked; then each of its own steps since.
    pub fn materialize(&self, name: &str) -> Result<LoroDoc, FileError> {
        // From `name` up to the first branch that stands on its own (a
        // snapshot, or no parent).
        let mut chain: Vec<&Branch> = Vec::new();
        let mut at = name;
        loop {
            let b = self
                .branches
                .iter()
                .find(|b| b.name == at)
                .ok_or_else(|| FileError::NoBranch(at.to_owned()))?;
            if chain.iter().any(|c| c.name == b.name) {
                return Err(FileError::NotADocument(format!(
                    "branch {name} is its own ancestor"
                )));
            }
            chain.push(b);
            if self.snapshots.contains_key(at) {
                break;
            }
            match (&b.parent, &b.base) {
                (Some(parent), Some(_)) => at = parent,
                _ => break,
            }
        }
        let mut doc: Option<LoroDoc> = None;
        for b in chain.into_iter().rev() {
            let here = match (doc.take(), self.snapshots.get(&b.name), &b.base) {
                (_, Some(snap), _) => {
                    let d = LoroDoc::new();
                    d.import(&snap.data).map_err(loro)?;
                    d
                }
                (Some(parent), None, Some(base)) => {
                    parent.fork_at(&decode_frontiers(base)?).map_err(loro)?
                }
                _ => LoroDoc::new(),
            };
            let updates: Vec<Vec<u8>> = self
                .steps
                .iter()
                .filter(|s| s.branch == b.name && !s.data.is_empty())
                .map(|s| s.data.clone())
                .collect();
            // In one batch: imported one at a time, each update has the
            // whole state worked out again (Studio measured 17 s for a
            // song of 377 steps).
            if !updates.is_empty() {
                here.import_batch(&updates).map_err(loro)?;
            }
            doc = Some(here);
        }
        doc.ok_or_else(|| FileError::NoBranch(name.to_owned()))
    }

    /// The records that rebuild this index, nothing more: what the file
    /// is rewritten to when it is folded.
    pub fn records(&self) -> Vec<Record> {
        let mut out: Vec<Record> = Vec::new();
        out.extend(self.meta.iter().map(|(k, v)| Record::Meta {
            key: k.clone(),
            value: v.clone(),
        }));
        out.extend(self.blobs.iter().map(|(hash, b)| Record::Blob {
            hash: hash.clone(),
            kind: b.kind.clone(),
            data: b.data.clone(),
        }));
        out.extend(self.branches.iter().map(|b| Record::Branch {
            name: b.name.clone(),
            parent: b.parent.clone(),
            base: b.base.clone(),
            at: b.at,
        }));
        // Snapshots before steps: a step after a snapshot's `upto` is
        // kept whole, one before it is already empty.
        out.extend(self.snapshots.iter().map(|(branch, s)| Record::Snapshot {
            branch: branch.clone(),
            upto: s.upto,
            at: s.at,
            data: s.data.clone(),
        }));
        out.extend(self.steps.iter().map(|s| Record::Step {
            seq: s.seq,
            branch: s.branch.clone(),
            label: s.label.clone(),
            at: s.at,
            frontiers: s.frontiers.clone(),
            data: s.data.clone(),
        }));
        out.extend(self.checkpoints.iter().map(|c| Record::Checkpoint {
            seq: c.seq,
            branch: c.branch.clone(),
            name: c.name.clone(),
            at: c.at,
            frontiers: c.frontiers.clone(),
        }));
        out
    }
}

fn decode_frontiers(bytes: &[u8]) -> Result<Frontiers, FileError> {
    if bytes.is_empty() {
        return Ok(Frontiers::default());
    }
    Frontiers::decode(bytes).map_err(loro)
}

// By value: an error-mapping adapter for `map_err`.
#[allow(clippy::needless_pass_by_value)]
pub fn loro(e: impl std::fmt::Display) -> FileError {
    FileError::Loro(e.to_string())
}
