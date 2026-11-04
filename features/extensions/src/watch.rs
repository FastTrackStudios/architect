//! Knowing when an extension's files changed.
//!
//! Each folder's fingerprint
//! (every file's path and contents, `.git` left out), compared on each
//! poll. No file-system events, so it is the same on every platform; an
//! app polls it on a timer (the lab: every half second), and a package
//! is a few small files.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// The folders' fingerprints when last polled.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    seen: BTreeMap<PathBuf, u64>,
}

impl Watch {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Which of `dirs` changed since the last poll. A folder seen for the
    /// first time is remembered, not reported; one no longer given is
    /// forgotten.
    pub fn changed(&mut self, dirs: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
        let mut next = BTreeMap::new();
        let mut changed = Vec::new();
        for dir in dirs {
            let print = fingerprint(&dir);
            if self.seen.get(&dir).is_some_and(|was| *was != print) {
                changed.push(dir.clone());
            }
            next.insert(dir, print);
        }
        self.seen = next;
        changed
    }
}

/// A folder's fingerprint: its files' paths and contents.
#[must_use]
pub fn fingerprint(dir: &Path) -> u64 {
    fingerprint_except(dir, &|_| false)
}

/// [`fingerprint`] of what `skip` (given each path relative to `dir`)
/// does not leave out: a crate's sources, not its build output.
#[must_use]
pub fn fingerprint_except(dir: &Path, skip: &dyn Fn(&Path) -> bool) -> u64 {
    let mut files = Vec::new();
    walk(dir, dir, skip, &mut files);
    files.sort();
    let mut hasher = DefaultHasher::new();
    for (path, bytes) in files {
        path.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    hasher.finish()
}

fn walk(root: &Path, dir: &Path, skip: &dyn Fn(&Path) -> bool, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        if skip(&rel) {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(root, &path, skip, out),
            Ok(_) => {
                let bytes = std::fs::read(&path).unwrap_or_default();
                out.push((rel, bytes));
            }
            Err(_) => {}
        }
    }
}
