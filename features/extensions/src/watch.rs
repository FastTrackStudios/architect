//! Knowing when an extension's files changed.
//!
//! Each folder's fingerprint
//! (every file's path and contents, `.git` left out), compared on each
//! poll. No file-system events, so it is the same on every platform; an
//! app polls it on a timer (the lab: every half second), and a package
//! is a few small files.
//!
//! A large file (at least [`READ_UNDER`]: a process extension's program,
//! media) is known by its size and modification time instead of read:
//! the poll runs on the window's thread, and reading a development
//! build's program (hundreds of megabytes) on every poll stalled the
//! window for a second at a time. A rebuilt program has a new time.

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

/// Files smaller than this are known by their contents; larger ones by
/// their size and modification time.
pub const READ_UNDER: u64 = 1024 * 1024;

/// What a file is known by: its bytes, or (large) its size and time.
fn known_by(path: &Path) -> Vec<u8> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() < READ_UNDER {
        return std::fs::read(path).unwrap_or_default();
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let mut out = meta.len().to_le_bytes().to_vec();
    out.extend_from_slice(&modified.to_le_bytes());
    out
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
            Ok(_) => out.push((rel, known_by(&path))),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod large_files {
    use super::*;

    #[test]
    fn a_large_file_is_known_by_its_size_and_time_and_a_rebuild_is_seen() {
        let dir = std::env::temp_dir().join(format!("watch-large-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("program");
        std::fs::write(&program, vec![1_u8; 2 * 1024 * 1024]).unwrap();
        std::fs::write(dir.join("extension.toml"), "id = \"x\"").unwrap();
        let mut watch = Watch::new();
        assert!(watch.changed([dir.clone()]).is_empty(), "first seen");
        assert!(watch.changed([dir.clone()]).is_empty(), "nothing moved");
        // A rebuilt program: the same size, written again later.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&program, vec![2_u8; 2 * 1024 * 1024]).unwrap();
        assert_eq!(watch.changed([dir.clone()]), vec![dir.clone()]);
        // A small file is still known by its contents.
        std::fs::write(dir.join("extension.toml"), "id = \"y\"").unwrap();
        assert_eq!(watch.changed([dir.clone()]), vec![dir.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
