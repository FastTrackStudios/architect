//! Synced roots as data.
//!
//! What a root is, its versions and checkpoints, and what a device's
//! agent reports (wasm-clean; the engine is
//! `crate::sync`, feature `sync`). The spec:
//! `docs/content/architecture/project-files.md`.

use std::collections::BTreeMap;

use facet::Facet;

use crate::id::{RootId, VersionId};

/// Where a root sits under an org.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum Section {
    /// `<org>/Projects/<name>`: a project, a DAW session's tree.
    Projects,
    /// `<org>/Assets/<kind>`: an asset library (`Tracks`, `Setlists`, …).
    Assets,
}

impl Section {
    /// The folder name under the org.
    #[must_use]
    pub const fn dir(&self) -> &'static str {
        match self {
            Self::Projects => "Projects",
            Self::Assets => "Assets",
        }
    }

    /// The section a folder name is.
    #[must_use]
    pub fn from_dir(dir: &str) -> Option<Self> {
        match dir {
            "Projects" => Some(Self::Projects),
            "Assets" => Some(Self::Assets),
            _ => None,
        }
    }
}

/// What a root is: the same on every device that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RootMeta {
    pub id: RootId,
    pub org: String,
    pub section: Section,
    /// The project's folder name, or the library's kind (`Tracks`).
    pub name: String,
}

impl RootMeta {
    /// `<org>/<Projects|Assets>/<name>`: its place, its name on the lane.
    #[must_use]
    pub fn place(&self) -> String {
        format!("{}/{}/{}", self.org, self.section.dir(), self.name)
    }
}

/// One save or delete of one path.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct FileVersion {
    pub id: VersionId,
    pub path: String,
    /// The bytes' sha-256 (hex); `None` for a delete.
    pub content: Option<String>,
    pub size: u64,
    /// The device that saved it.
    pub by: String,
    /// When its content was written (the file's mtime), µs since the epoch:
    /// the newest content wins a fork.
    pub at_us: i64,
    /// The versions this one replaced (several when it settles a fork).
    pub base: Vec<VersionId>,
}

impl FileVersion {
    #[must_use]
    pub const fn is_delete(&self) -> bool {
        self.content.is_none()
    }
}

/// A named set of heads: a save point, an import, or one asked for.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Checkpoint {
    pub id: VersionId,
    pub name: String,
    pub by: String,
    pub at_us: i64,
    /// Made by the engine (a session file saved, the first import).
    pub auto: bool,
    /// Every live path and the version it was at.
    pub heads: BTreeMap<String, VersionId>,
}

/// What a root looks like on this device.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RootStatus {
    pub doc_id: String,
    pub place: String,
    pub org: String,
    pub section: Section,
    pub name: String,
    /// The working copy, when the root is pinned (or held, on the node).
    pub path: Option<String>,
    pub pinned: bool,
    pub files: u32,
    pub bytes: u64,
    /// Files whose bytes are not here yet (pinned roots).
    pub missing: u32,
    /// Every file is at its head.
    pub synced: bool,
    /// Task stubs met in the working copy: never synced.
    pub stubs: Vec<String>,
    pub error: Option<String>,
}

/// One entry of a folder in a root, as the agent lists it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct FileEntry {
    /// Root-relative.
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub content: Option<String>,
    pub version: Option<VersionId>,
    pub at_us: i64,
    /// Its bytes are on this device (a pinned copy, or fetched once).
    pub here: bool,
}

/// A file an asset plays: a stem, or a sound file in its folder.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Media {
    pub name: String,
    /// Root-relative.
    pub path: String,
    pub size: u64,
}

/// An asset of a library: a song, a setlist, a preset.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Asset {
    /// The library's place (`days-to-praise/Assets/Tracks`).
    pub root: String,
    /// The library's kind (`Tracks`).
    pub kind: String,
    pub name: String,
    /// Its folder in the library, empty for a note on its own.
    pub folder: String,
    /// Its note, root-relative.
    pub note: Option<String>,
    /// The note's frontmatter (`type`, `artist`, `key`, …).
    pub properties: BTreeMap<String, String>,
    pub media: Vec<Media>,
    /// What its note links (`[[…]]`): a setlist's songs.
    pub links: Vec<String>,
    /// When its newest file was written, µs since the epoch.
    pub at_us: i64,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Hit {
    /// The root's place.
    pub root: String,
    /// Root-relative; empty for the root itself.
    pub path: String,
    pub name: String,
    /// `project`, `file`, or an asset's `type` (`song`, `setlist`, `asset`).
    pub what: String,
    pub detail: String,
    /// The root's org.
    #[facet(default)]
    pub org: String,
    #[facet(default)]
    pub size: u64,
    /// The bytes' sha-256 (files), hex.
    #[facet(default)]
    pub content: Option<String>,
    /// Where its bytes are: every holder with a copy (names), verified first.
    #[facet(default)]
    pub holders: Vec<String>,
    /// Copies counted (verified).
    #[facet(default)]
    pub verified: u32,
    /// A copy is on this device or on a holder connected now.
    #[facet(default)]
    pub reachable: bool,
}

/// A device's drive, for the app's status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct DriveStatus {
    /// Where it is mounted, when the agent has a drive.
    pub mount: Option<String>,
    pub mounted: bool,
    /// Pinned roots not at their heads yet.
    pub syncing: u32,
    /// No peer answers: an unpinned file that is not cached cannot open.
    pub offline: bool,
    /// Bytes in the cache (the blob store; pinned working copies aside).
    pub cache_bytes: u64,
}
