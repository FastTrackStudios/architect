//! A package: a manifest and its files, in a folder (installed, or in
//! development) or linked into the build.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::disk::Disk;
use crate::manifest::{MANIFEST, Manifest, ManifestError};

/// Where a package's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Files {
    /// A folder on disk: the manifest beside its files.
    Dir(PathBuf),
    /// Linked into the build (`include_str!`): each file's path and text.
    Embedded(Vec<(&'static str, &'static str)>),
    /// A folder of a [`Disk`] that is not the operating system's (a
    /// browser's installs, `crate::disk::MemDisk`): read through it.
    Disk(DiskDir),
}

/// A folder of a [`Disk`].
#[derive(Clone)]
pub struct DiskDir {
    pub disk: Arc<dyn Disk>,
    pub dir: PathBuf,
}

impl std::fmt::Debug for DiskDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskDir")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl PartialEq for DiskDir {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.disk, &other.disk) && self.dir == other.dir
    }
}

impl Eq for DiskDir {}

/// A manifest and its files.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub manifest: Manifest,
    pub files: Files,
}

/// Why a package could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// A file of it could not be read.
    Read {
        path: String,
        why: String,
    },
    Manifest(ManifestError),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, why } => write!(f, "{path} cannot be read: {why}"),
            Self::Manifest(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<ManifestError> for PackageError {
    fn from(e: ManifestError) -> Self {
        Self::Manifest(e)
    }
}

impl Package {
    /// The package in `dir`: its `extension.toml`, read and checked.
    ///
    /// # Errors
    ///
    /// The manifest is missing, unreadable or refused.
    pub fn read(dir: impl Into<PathBuf>) -> Result<Self, PackageError> {
        let dir = dir.into();
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path).map_err(|e| PackageError::Read {
            path: path.display().to_string(),
            why: e.to_string(),
        })?;
        Ok(Self {
            manifest: Manifest::parse(&text)?,
            files: Files::Dir(dir),
        })
    }

    /// The package in folder `dir` of `disk`: through `std::fs` when it is
    /// the operating system's ([`Self::read`]), else read through it.
    ///
    /// # Errors
    ///
    /// The manifest is missing, unreadable or refused.
    pub fn read_on(disk: &Arc<dyn Disk>, dir: impl Into<PathBuf>) -> Result<Self, PackageError> {
        let dir = dir.into();
        if disk.is_os() {
            return Self::read(dir);
        }
        let path = dir.join(MANIFEST);
        let text = disk.read_to_string(&path).map_err(|e| PackageError::Read {
            path: path.display().to_string(),
            why: e.to_string(),
        })?;
        Ok(Self {
            manifest: Manifest::parse(&text)?,
            files: Files::Disk(DiskDir {
                disk: Arc::clone(disk),
                dir,
            }),
        })
    }

    /// A package linked into the build: its manifest's text and its
    /// files, each `(path, text)`.
    ///
    /// # Errors
    ///
    /// The manifest is refused.
    pub fn embedded(
        manifest: &str,
        files: &[(&'static str, &'static str)],
    ) -> Result<Self, ManifestError> {
        let parsed = Manifest::parse(manifest)?;
        let mut files = files.to_vec();
        // Its own text among its files, so a copy can be written out
        // (`EXT_CLONE`); an embedded package is made once, so it is kept
        // for the process's life.
        if !files.iter().any(|(p, _)| *p == MANIFEST) {
            files.push((MANIFEST, Box::leak(manifest.to_owned().into_boxed_str())));
        }
        Ok(Self {
            manifest: parsed,
            files: Files::Embedded(files),
        })
    }

    /// Its files, each `(path, text)`, when it is one linked in
    /// ([`Package::embedded`]); none for any other.
    #[must_use]
    pub fn embedded_files(&self) -> &[(&'static str, &'static str)] {
        match &self.files {
            Files::Embedded(files) => files,
            Files::Dir(_) | Files::Disk(_) => &[],
        }
    }

    /// Its folder on the operating system's disk, when it has one (not
    /// one linked in, nor one kept in a browser's storage).
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        match &self.files {
            Files::Dir(d) => Some(d),
            Files::Embedded(_) | Files::Disk(_) => None,
        }
    }

    /// The text of its file `path` (relative to the package).
    ///
    /// # Errors
    ///
    /// It has no such file, or it cannot be read.
    pub fn text(&self, path: &str) -> Result<String, PackageError> {
        let missing = |why: String| PackageError::Read {
            path: path.to_owned(),
            why,
        };
        match &self.files {
            Files::Dir(dir) => {
                std::fs::read_to_string(dir.join(path)).map_err(|e| missing(e.to_string()))
            }
            Files::Embedded(files) => files
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, text)| (*text).to_owned())
                .ok_or_else(|| missing("it is not in the package".to_owned())),
            Files::Disk(d) => {
                // Inside the package only.
                if !Path::new(path)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    return Err(missing("not inside the package".to_owned()));
                }
                d.disk
                    .read_to_string(&d.dir.join(path))
                    .map_err(|e| missing(e.to_string()))
            }
        }
    }

    /// Every file of it with its text, by its `/`-separated path from the
    /// package (what a browser's runner hands a Web Worker): `None` for a
    /// folder on the operating system's disk, read by its path.
    #[must_use]
    pub fn texts(&self) -> Option<Vec<(String, String)>> {
        match &self.files {
            Files::Dir(_) => None,
            Files::Embedded(files) => Some(
                files
                    .iter()
                    .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
                    .collect(),
            ),
            Files::Disk(d) => {
                let mut out = Vec::new();
                let mut stack = vec![String::new()];
                while let Some(rel) = stack.pop() {
                    let at = if rel.is_empty() {
                        d.dir.clone()
                    } else {
                        d.dir.join(&rel)
                    };
                    for name in d.disk.list(&at).unwrap_or_default() {
                        let path = if rel.is_empty() {
                            name
                        } else {
                            format!("{rel}/{name}")
                        };
                        match d.disk.read_to_string(&d.dir.join(&path)) {
                            Ok(text) => out.push((path, text)),
                            // A folder (or a file that is not text: left out).
                            Err(_) if d.disk.list(&d.dir.join(&path)).is_ok() => stack.push(path),
                            Err(_) => {}
                        }
                    }
                }
                out.sort();
                Some(out)
            }
        }
    }
}
