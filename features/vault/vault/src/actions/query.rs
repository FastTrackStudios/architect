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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use vault_obsidian::{VaultEntryKind, read_obsidian_config, walk_vault};
use vault_query::index::{Note, NoteSource, Text, hash_of};

use super::OpenVault;

fn millis(t: std::time::SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
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
            // A note's stamp is its generation, moved every time its text
            // is set (its time and length alone could miss an edit); a
            // base's, a hash of its YAML (bases are few and small).
            let pages = v.pages.iter().map(|p| Note {
                path: &p.rel_path,
                stamp: p.generation,
                text: Text::Here(&p.raw),
                parsed: Some(&p.parsed),
                mtime: millis(p.mtime),
                is_base: false,
            });
            let bases = v.bases.iter().map(|b| Note {
                path: &b.rel_path,
                stamp: hash_of(&b.raw),
                text: Text::Here(&b.raw),
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

/// A vault folder on disk as the index's source, holding no note's text.
///
/// What the node builds a vault's index from. Each sync lists the folder
/// (with `.obsidian/app.json`'s ignore filters) and reads only the notes
/// whose modified time, change time or size moved; a watcher moves its
/// revision on every change, so a sync of a quiet vault reads nothing.
pub struct DiskSource {
    root: PathBuf,
    revision: Arc<AtomicU64>,
    /// The listing at a revision: each note's path, stamp, time and kind.
    listing: Mutex<(u64, Arc<Vec<Listed>>)>,
    /// `None` when the folder could not be watched: then every sync lists.
    /// Held (behind a lock: the guard is not `Sync`) while the source is.
    watching: Option<Mutex<vault_live::watcher::WatcherGuard>>,
}

struct Listed {
    rel: String,
    stamp: u64,
    mtime: u64,
    is_base: bool,
}

impl std::fmt::Debug for DiskSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskSource")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl DiskSource {
    /// The folder at `root`, watched.
    #[must_use]
    pub fn open(root: &Path) -> Self {
        let revision = Arc::new(AtomicU64::new(1));
        let watching = match vault_live::watcher::watch(
            root.to_path_buf(),
            std::time::Duration::from_millis(250),
        ) {
            Ok((events, guard)) => {
                let moved = Arc::clone(&revision);
                let _ = std::thread::Builder::new()
                    .name("vault-index-watch".to_owned())
                    .spawn(move || {
                        for _ in events {
                            moved.fetch_add(1, Ordering::Release);
                        }
                    });
                Some(Mutex::new(guard))
            }
            Err(e) => {
                tracing::warn!(root = %root.display(), error = %e, "the index lists the folder on every sync");
                None
            }
        };
        Self {
            root: root.to_path_buf(),
            revision,
            listing: Mutex::new((0, Arc::new(Vec::new()))),
            watching,
        }
    }

    /// The listing at `revision`: kept, or the folder listed again.
    fn listing(&self, revision: u64) -> Arc<Vec<Listed>> {
        let mut held = self.listing.lock().unwrap_or_else(PoisonError::into_inner);
        if held.0 == revision && self.watching.is_some() {
            return Arc::clone(&held.1);
        }
        let config = read_obsidian_config(&self.root);
        let listed: Vec<Listed> = walk_vault(&self.root, &config)
            .into_iter()
            .filter(|e| matches!(e.kind, VaultEntryKind::Markdown | VaultEntryKind::Base))
            .filter_map(|e| {
                let meta = std::fs::symlink_metadata(&e.abs_path).ok()?;
                if !meta.is_file() {
                    return None;
                }
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&meta.modified().ok(), &mut h);
                std::hash::Hash::hash(&meta.len(), &mut h);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt as _;
                    std::hash::Hash::hash(&(meta.ctime(), meta.ctime_nsec(), meta.ino()), &mut h);
                }
                Some(Listed {
                    rel: e.rel_path,
                    stamp: std::hash::Hasher::finish(&h),
                    mtime: meta.modified().map_or(0, millis),
                    is_base: matches!(e.kind, VaultEntryKind::Base),
                })
            })
            .collect();
        *held = (revision, Arc::new(listed));
        Arc::clone(&held.1)
    }
}

impl NoteSource for DiskSource {
    fn revision(&self) -> u64 {
        if self.watching.is_some() {
            self.revision.load(Ordering::Acquire)
        } else {
            // Unwatched: every read is a new state.
            self.revision.fetch_add(1, Ordering::AcqRel)
        }
    }

    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        let revision = self.revision.load(Ordering::Acquire);
        let listing = self.listing(revision);
        for l in listing.iter().skip(from).take(n) {
            // Read only when the builder needs it; never through a symlink.
            let load = || {
                let mut file = vault_obsidian::open_own(&self.root, &l.rel).ok()?;
                let mut text = String::new();
                std::io::Read::read_to_string(&mut file, &mut text).ok()?;
                Some(text)
            };
            each(Note {
                path: &l.rel,
                stamp: l.stamp,
                text: Text::Load(&load),
                parsed: None,
                mtime: l.mtime,
                is_base: l.is_base,
            });
        }
        (revision, listing.len())
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
            (
                "Songs/A.md",
                "---\nstatus: todo\n---\n# A\n- [ ] learn it\n",
            ),
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

        open.set_property("Notes/C.md", "status", json!("todo"))
            .unwrap();
        let delta = index.sync();
        assert_eq!(delta.put.len(), 1, "one note changed: {delta:?}");
        assert_eq!(&*delta.put[0].path, "Notes/C.md");
        assert!(index.sync().is_empty(), "nothing moved since");

        open.create("Songs/D.md", "---\nstatus: x\n---\n").unwrap();
        assert_eq!(&*index.sync().put[0].path, "Songs/D.md");
        open.reload().unwrap();
        assert!(index.sync().is_empty(), "a reload of unchanged notes");
        let epoch = index.index().epoch;
        assert_eq!(index.since(epoch, first.to).unwrap().put.len(), 2);
    }

    /// A folder on disk as the source: read only what changed, a file
    /// edited with the same length within the same second seen too, a
    /// file removed gone.
    #[test]
    fn a_folder_on_disk_is_read_only_where_it_changed() {
        let (dir, _open) = vault(&[("A.md", "---\nstatus: a\n---\n"), ("B.md", "# B\n")]);
        let index = Built::new(DiskSource::open(dir.path()));
        assert_eq!(index.sync().put.len(), 2);
        assert!(index.sync().is_empty(), "nothing moved");
        std::fs::write(dir.path().join("A.md"), "---\nstatus: b\n---\n").unwrap();
        let mut seen = None;
        for _ in 0..100 {
            let d = index.sync();
            if !d.is_empty() {
                seen = Some(d);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let d = seen.expect("the edit was seen");
        assert_eq!(d.put.len(), 1);
        assert_eq!(d.put[0].props["status"], "b");
        std::fs::remove_file(dir.path().join("B.md")).unwrap();
        let mut gone = false;
        for _ in 0..100 {
            if index.sync().gone.iter().any(|p| &**p == "B.md") {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(gone, "the removed note is gone");
    }
}
