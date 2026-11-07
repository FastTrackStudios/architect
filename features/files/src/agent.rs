//! A device's files agent, as a service.
//!
//! What the window, the CLI and an agent over MCP ask the node that keeps
//! this device's roots (`architect-desktop serve` with a `files_dir`). Declared once
//! (`#[architect::service(actions)]`): its HTTP face on the node's
//! loopback port (`FilesHttpClient`), and `FILES_*` on the Action List.
//!
//! A root is named by its place (`codywright/Projects/Right Here`) or its
//! manifest's id. The engine answers it (`crate::sync`).

use crate::rendition::Peaks;
use crate::roots::{Asset, Checkpoint, DriveStatus, FileEntry, FileVersion, Hit, RootStatus};

/// Why the agent refused.
#[architect::error]
#[derive(Eq)]
pub enum AgentError {
    #[error("no such root, path or version: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    #[error("not synced yet: {0}")]
    #[architect(http_status = 409)]
    NotSynced(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

pub mod files {
    use super::{
        AgentError, Asset, Checkpoint, DriveStatus, FileEntry, FileVersion, Hit, Peaks, RootStatus,
    };

    /// The agent: roots, pins, versions.
    #[architect::service(actions)]
    pub trait Files {
        /// Every root this device knows: its projects and libraries, pinned or not.
        #[action(effect = read_only)]
        async fn roots(&self) -> Result<Vec<RootStatus>, AgentError>;
        /// One folder of a root (`""` is its top).
        #[action(effect = read_only)]
        async fn browse(&self, root: String, path: String) -> Result<Vec<FileEntry>, AgentError>;
        /// Keep a full working copy of a root here, at `path` or the default.
        async fn pin(&self, root: String, path: Option<String>) -> Result<String, AgentError>;
        /// Remove the working copy, once every file in it is synced.
        async fn unpin(&self, root: String) -> Result<(), AgentError>;
        /// A file's versions, oldest first.
        #[action(effect = read_only)]
        async fn history(&self, root: String, path: String)
        -> Result<Vec<FileVersion>, AgentError>;
        /// Put a file back at a version (a new version; nothing is lost).
        async fn restore(
            &self,
            root: String,
            path: String,
            version: String,
        ) -> Result<FileVersion, AgentError>;
        /// Name the root's current state.
        async fn checkpoint(&self, root: String, name: String) -> Result<Checkpoint, AgentError>;
        /// The root's checkpoints: save points, imports, named ones.
        #[action(effect = read_only)]
        async fn checkpoints(&self, root: String) -> Result<Vec<Checkpoint>, AgentError>;
        /// A library's assets: their properties, media and links.
        #[action(effect = read_only)]
        async fn assets(&self, root: String) -> Result<Vec<Asset>, AgentError>;
        /// Projects, assets and files whose name or properties hold every word.
        #[action(effect = read_only)]
        async fn search(&self, query: String) -> Result<Vec<Hit>, AgentError>;
        /// A sound file's waveform (`audiowaveform` JSON v2).
        #[action(effect = read_only)]
        async fn peaks(&self, root: String, path: String) -> Result<Peaks, AgentError>;
        /// The drive: where it is mounted, what is syncing, offline, the cache's size.
        #[action(effect = read_only)]
        async fn drive(&self) -> Result<DriveStatus, AgentError>;
        /// Put the whole root back at a checkpoint; how many files changed.
        async fn restore_checkpoint(
            &self,
            root: String,
            checkpoint: String,
        ) -> Result<u32, AgentError>;
    }

    pub use prelude::*;
}

pub use files::Files;
