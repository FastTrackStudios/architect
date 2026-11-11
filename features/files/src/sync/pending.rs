//! The engine's own temp files, written down before they exist.
//!
//! Every staged file the engine puts in a working tree
//! (`<file>.architect-part`) is recorded in `<data>/pending-writes.json`
//! before it is created and struck off once it is committed or removed.
//! At start, only a temp file this journal names is removed: the engine
//! can prove it wrote it. Any other file with that suffix is a user's,
//! left alone and reported (`RootStatus::strays`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The journal (see the module docs). Cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct Pending {
    file: PathBuf,
    paths: Mutex<BTreeSet<PathBuf>>,
}

impl Pending {
    /// The journal in `data`, with what an earlier run left in it.
    #[must_use]
    pub fn open(data: &Path) -> Self {
        let file = data.join("pending-writes.json");
        let paths = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| facet_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        Self {
            file,
            paths: Mutex::new(paths),
        }
    }

    fn save(&self, paths: &BTreeSet<PathBuf>) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, facet_json::to_string(&list).unwrap_or_default())?;
        std::fs::rename(tmp, &self.file)
    }

    /// `path` is about to be written.
    ///
    /// # Errors
    ///
    /// The journal cannot be written: then neither may the temp file be.
    pub fn add(&self, path: &Path) -> std::io::Result<()> {
        let mut paths = self.paths.lock().unwrap_or_else(PoisonError::into_inner);
        let saved = if paths.insert(path.to_owned()) {
            self.save(&paths)
        } else {
            Ok(())
        };
        drop(paths);
        saved
    }

    /// `path` is gone (committed or removed).
    pub fn remove(&self, path: &Path) {
        let mut paths = self.paths.lock().unwrap_or_else(PoisonError::into_inner);
        if paths.remove(path) {
            let _ = self.save(&paths);
        }
    }

    /// Whether the engine wrote `path` and has not finished with it.
    #[must_use]
    pub fn owns(&self, path: &Path) -> bool {
        self.paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(path)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn it_survives_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("tree/take.wav.architect-part");
        Pending::open(tmp.path()).add(&p).unwrap();
        let again = Pending::open(tmp.path());
        assert!(again.owns(&p));
        again.remove(&p);
        assert!(!Pending::open(tmp.path()).owns(&p));
    }
}
