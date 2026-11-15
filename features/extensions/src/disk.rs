//! Where installs, grants and installed packages are kept (`Disk`).
//!
//! On a desk and a phone that is the app's folder ([`OsDisk`], `std::fs`).
//! A browser has no folder (`std::fs` fails on `wasm32-unknown-unknown`),
//! so the web app gives [`MemDisk`]: the same files by path in memory,
//! each change handed to a callback that keeps them in the browser's
//! storage, and read back from there when the page loads (#495). The
//! install path ([`crate::install::Installs`]) and consent
//! ([`crate::consent::Grants`]) are the same code on either: only where
//! their bytes go differs.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path};
use std::sync::{Arc, Mutex, PoisonError};

/// Files by path: what an install, a grant record and an installed package
/// are kept in. Paths are the same an app's folder would have.
pub trait Disk: Send + Sync {
    /// The bytes of file `path`.
    ///
    /// # Errors
    ///
    /// There is no such file, or it cannot be read.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Writes file `path` (its folders made as needed), in place of what
    /// was there.
    ///
    /// # Errors
    ///
    /// It cannot be written.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// The names in folder `dir`: files and folders, one level.
    ///
    /// # Errors
    ///
    /// There is no such folder, or it cannot be read.
    fn list(&self, dir: &Path) -> io::Result<Vec<String>>;

    /// Whether `path` is a file or a folder here.
    fn exists(&self, path: &Path) -> bool;

    /// Makes folder `dir` and the folders above it.
    ///
    /// # Errors
    ///
    /// It cannot be made.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;

    /// Removes folder `dir` and all in it.
    ///
    /// # Errors
    ///
    /// It cannot be removed (there being none is an error too).
    fn remove_dir_all(&self, dir: &Path) -> io::Result<()>;

    /// Moves folder (or file) `from` to `to`, which must not be there.
    ///
    /// # Errors
    ///
    /// It cannot be moved.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Whether its paths are the operating system's: what a runner that
    /// reads a package by its folder (a process, a WASI component, a page
    /// on `QuickJS`) needs. False for one in memory, whose packages are
    /// read through it ([`crate::package::Files::Disk`]).
    fn is_os(&self) -> bool {
        false
    }

    /// The text of file `path`.
    ///
    /// # Errors
    ///
    /// As [`Disk::read`], or it is not UTF-8.
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}

/// The operating system's files (`std::fs`): an app's folder.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsDisk;

impl Disk for OsDisk {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, bytes)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        std::fs::read_dir(dir)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(dir)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn is_os(&self) -> bool {
        true
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// What is told of each change to a [`MemDisk`]: all its files, by path.
pub type OnChange = Arc<dyn Fn(&BTreeMap<String, Vec<u8>>) + Send + Sync>;

/// Files in memory, by path: a browser's [`Disk`]. Each change is handed
/// to [`MemDisk::on_change`]'s callback (the web app keeps them in the
/// browser's storage); [`MemDisk::with_files`] starts it from what was
/// kept.
#[derive(Clone, Default)]
pub struct MemDisk {
    inner: Arc<Mutex<Mem>>,
    on_change: Option<OnChange>,
}

#[derive(Default)]
struct Mem {
    files: BTreeMap<String, Vec<u8>>,
    /// Folders made empty (`create_dir_all`); a folder with files is
    /// there anyway.
    dirs: BTreeSet<String>,
}

impl std::fmt::Debug for MemDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemDisk")
            .field("files", &self.mem().files.len())
            .finish_non_exhaustive()
    }
}

/// `path` as a key: its normal parts, `/`-joined, with no `.`, `..` or
/// root (refused: a path here never leaves its tree).
fn key(path: &Path) -> io::Result<String> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{}: not a path inside the store", path.display()),
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

fn under(dir: &str, path: &str) -> bool {
    dir.is_empty() || path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: no such file", path.display()),
    )
}

impl MemDisk {
    /// Empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starting from `files` (each path and its bytes): what was kept.
    #[must_use]
    pub fn with_files(files: BTreeMap<String, Vec<u8>>) -> Self {
        let disk = Self::new();
        disk.mem().files = files;
        disk
    }

    /// `f` hears every change, with all the files there are after it.
    #[must_use]
    pub fn on_change(
        mut self,
        f: impl Fn(&BTreeMap<String, Vec<u8>>) + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(f));
        self
    }

    /// All its files, by path.
    #[must_use]
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        self.mem().files.clone()
    }

    fn mem(&self) -> std::sync::MutexGuard<'_, Mem> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changed(&self) {
        if let Some(f) = &self.on_change {
            let files = self.files();
            f(&files);
        }
    }

    fn is_dir(mem: &Mem, k: &str) -> bool {
        k.is_empty()
            || mem.dirs.contains(k)
            || mem.dirs.iter().any(|d| under(k, d))
            || mem.files.keys().any(|f| under(k, f))
    }
}

impl Disk for MemDisk {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let k = key(path)?;
        self.mem()
            .files
            .get(&k)
            .cloned()
            .ok_or_else(|| not_found(path))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let k = key(path)?;
        if k.is_empty() || Self::is_dir(&self.mem(), &k) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}: a folder", path.display()),
            ));
        }
        self.mem().files.insert(k, bytes.to_vec());
        self.changed();
        Ok(())
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        let k = key(dir)?;
        let mem = self.mem();
        if !Self::is_dir(&mem, &k) {
            return Err(not_found(dir));
        }
        let child = |p: &String| {
            let rest = if k.is_empty() {
                p.as_str()
            } else {
                p.strip_prefix(&k)?.strip_prefix('/')?
            };
            rest.split('/').next().map(str::to_owned)
        };
        let names: BTreeSet<String> = mem
            .files
            .keys()
            .chain(mem.dirs.iter())
            .filter_map(child)
            .filter(|n| !n.is_empty())
            .collect();
        drop(mem);
        Ok(names.into_iter().collect())
    }

    fn exists(&self, path: &Path) -> bool {
        key(path).is_ok_and(|k| {
            let mem = self.mem();
            mem.files.contains_key(&k) || Self::is_dir(&mem, &k)
        })
    }

    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        let k = key(dir)?;
        if self.mem().files.contains_key(&k) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{}: a file", dir.display()),
            ));
        }
        if !k.is_empty() {
            self.mem().dirs.insert(k);
        }
        Ok(())
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        let k = key(dir)?;
        {
            let mut mem = self.mem();
            if !Self::is_dir(&mem, &k) {
                return Err(not_found(dir));
            }
            mem.files.retain(|p, _| !under(&k, p));
            mem.dirs.retain(|d| d != &k && !under(&k, d));
        }
        self.changed();
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (f, t) = (key(from)?, key(to)?);
        {
            let mut mem = self.mem();
            if mem.files.contains_key(&t) || Self::is_dir(&mem, &t) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{}: already there", to.display()),
                ));
            }
            if let Some(bytes) = mem.files.remove(&f) {
                mem.files.insert(t, bytes);
            } else if Self::is_dir(&mem, &f) {
                let moved: Vec<(String, Vec<u8>)> = mem
                    .files
                    .iter()
                    .filter(|(p, _)| under(&f, p))
                    .map(|(p, b)| (format!("{t}{}", p.get(f.len()..).unwrap_or("")), b.clone()))
                    .collect();
                mem.files.retain(|p, _| !under(&f, p));
                mem.files.extend(moved);
                let dirs: Vec<String> = mem
                    .dirs
                    .iter()
                    .filter(|d| *d == &f || under(&f, d))
                    .map(|d| format!("{t}{}", d.get(f.len()..).unwrap_or("")))
                    .collect();
                mem.dirs.retain(|d| d != &f && !under(&f, d));
                mem.dirs.extend(dirs);
            } else {
                return Err(not_found(from));
            }
            drop(mem);
        }
        self.changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Disk, MemDisk};

    #[test]
    fn a_memory_disk_keeps_files_by_path_and_moves_folders_whole() {
        let heard = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&heard);
        let disk = MemDisk::new().on_change(move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        });
        let root = Path::new("/architect/extensions");
        disk.write(&root.join(".incoming-x/extension.toml"), b"id")
            .unwrap();
        disk.write(&root.join(".incoming-x/pages/a.tsx"), b"page")
            .unwrap();
        assert!(disk.exists(&root.join(".incoming-x")));
        disk.rename(&root.join(".incoming-x"), &root.join("x"))
            .unwrap();
        assert!(!disk.exists(&root.join(".incoming-x")));
        assert_eq!(disk.read(&root.join("x/pages/a.tsx")).unwrap(), b"page");
        assert_eq!(disk.list(root).unwrap(), vec!["x".to_owned()]);
        assert_eq!(
            disk.list(&root.join("x")).unwrap(),
            vec!["extension.toml".to_owned(), "pages".to_owned()]
        );
        disk.remove_dir_all(&root.join("x")).unwrap();
        assert!(!disk.exists(&root.join("x")));
        assert!(disk.list(root).is_err() || disk.list(root).unwrap().is_empty());
        assert_eq!(heard.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_memory_disk_starts_from_what_was_kept_and_refuses_leaving_its_tree() {
        let disk = MemDisk::with_files([("a/b.txt".to_owned(), b"hi".to_vec())].into());
        assert_eq!(disk.read_to_string(Path::new("/a/b.txt")).unwrap(), "hi");
        assert!(disk.read(Path::new("a/../../etc/passwd")).is_err());
        assert!(disk.write(Path::new("../x"), b"no").is_err());
    }
}
