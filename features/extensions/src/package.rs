//! A package: a manifest and its files, in a folder (installed, or in
//! development) or linked into the build.

use std::path::{Path, PathBuf};

use crate::manifest::{MANIFEST, Manifest, ManifestError};

/// Where a package's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Files {
    /// A folder on disk: the manifest beside its files.
    Dir(PathBuf),
    /// Linked into the build (`include_str!`): each file's path and text.
    Embedded(Vec<(&'static str, &'static str)>),
}

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
        Ok(Self {
            manifest: Manifest::parse(manifest)?,
            files: Files::Embedded(files.to_vec()),
        })
    }

    /// Its folder, when it has one.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        match &self.files {
            Files::Dir(d) => Some(d),
            Files::Embedded(_) => None,
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
        }
    }
}
