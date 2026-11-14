//! An open vault as the vault index's source (`vault_query::NoteSource`):
//! what a desk, a phone and the node build their index from.
//!
//! ```ignore
//! let index = vault::query::Built::new(open.clone());
//! let delta = index.sync();          // what changed since the last sync
//! let now = index.index();           // the index, synced first
//! ```
//!
//! The builder reads it a chunk of notes at a time
//! (`vault_query::index::CHUNK`) under the vault's read lock, and only
//! the notes whose stamp moved are looked at again: an editor's save
//! never waits on a large vault's index.

use std::hash::{Hash, Hasher};
use std::time::UNIX_EPOCH;

use vault_query::index::{Note, NoteSource};

use super::OpenVault;

fn millis(t: std::time::SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What may have changed a note: its modified time, its length and
/// where its text is held (every write makes a new one).
fn stamp(mtime: std::time::SystemTime, text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mtime.hash(&mut h);
    text.len().hash(&mut h);
    text.as_ptr().addr().hash(&mut h);
    h.finish()
}

impl NoteSource for OpenVault {
    fn revision(&self) -> u64 {
        Self::revision(self)
    }

    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        self.with(|v| {
            // Read under the lock: no write is under way, so this is the
            // state the notes are in.
            let revision = Self::revision(self);
            let total = v.pages.len().saturating_add(v.bases.len());
            let pages = v.pages.iter().map(|p| Note {
                path: &p.rel_path,
                stamp: stamp(p.mtime, &p.raw),
                text: &p.raw,
                parsed: Some(&p.parsed),
                mtime: millis(p.mtime),
                is_base: false,
            });
            let bases = v.bases.iter().map(|b| Note {
                path: &b.rel_path,
                stamp: stamp(b.mtime, &b.raw),
                text: &b.raw,
                parsed: None,
                mtime: millis(b.mtime),
                is_base: true,
            });
            for note in pages.chain(bases).skip(from).take(n) {
                each(note);
            }
            (revision, total)
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use serde_json::json;
    use vault_query::Built;

    use super::*;

    fn vault(notes: &[(&str, &str)]) -> (tempfile::TempDir, OpenVault) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in notes {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, text).unwrap();
        }
        let open = OpenVault::open(dir.path()).unwrap();
        (dir, open)
    }

    /// An open vault's index: every note and base at first, then a delta
    /// of only the note a write changed, a note made, and no delta for a
    /// reload of unchanged notes.
    #[test]
    fn the_index_follows_the_vault() {
        let (_dir, open) = vault(&[
            ("Songs/A.md", "---\nstatus: todo\n---\n# A\n- [ ] learn it\n"),
            ("Notes/C.md", "C"),
            ("Songs.base", "views: []\n"),
        ]);
        let index = Built::new(open.clone());
        let first = index.sync();
        assert_eq!(first.put.len(), 3);
        let now = index.index();
        assert_eq!(now.entries["Songs/A.md"].props["status"], "todo");
        assert_eq!(now.entries["Songs/A.md"].tasks.len(), 1);
        assert_eq!(now.base("Songs.base"), Some("views: []\n"));

        open.set_property("Notes/C.md", "status", json!("todo")).unwrap();
        let delta = index.sync();
        assert_eq!(delta.put.len(), 1, "one note changed: {delta:?}");
        assert_eq!(delta.put[0].path, "Notes/C.md");
        assert!(index.sync().is_empty(), "nothing moved since");

        open.create("Songs/D.md", "---\nstatus: x\n---\n").unwrap();
        assert_eq!(index.sync().put[0].path, "Songs/D.md");
        open.reload().unwrap();
        assert!(index.sync().is_empty(), "a reload of unchanged notes");
        assert_eq!(index.since(first.to).unwrap().put.len(), 2);
    }
}
