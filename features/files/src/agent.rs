//! A device's files agent, as a service.
//!
//! What the window, the CLI and an agent over MCP ask the node that keeps
//! this device's roots (`architect-desktop serve` with a `files_dir`). Declared once
//! (`#[architect::service(actions)]`): its HTTP face on the node's
//! loopback port (`FilesHttpClient`), and `FILES_*` on the Action List.
//!
//! A root is named by its place (`codywright/Projects/Right Here`) or its
//! manifest's id. The engine answers it (`crate::sync`).

use crate::copies::{CopyReport, HolderStatus, Locations, Policy, ScrubReport, Violation};
use crate::rendition::Peaks;
use crate::roots::{
    Asset, Checkpoint, DriveStatus, FileEntry, FileVersion, Hit, RootStatus, VersionBytes,
};

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
    /// Dropping this device's copies would leave files under their copy policy.
    #[error("under the copy policy without this device: {0}")]
    #[architect(http_status = 409)]
    BelowPolicy(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
    /// The device's store has no room for it now (its budget); try later.
    #[error("{0}")]
    #[architect(http_status = 507)]
    NoRoom(String),
}

pub mod files {
    use super::{
        AgentError, Asset, Checkpoint, CopyReport, DriveStatus, FileEntry, FileVersion, Hit,
        HolderStatus, Locations, Peaks, Policy, RootStatus, ScrubReport, VersionBytes, Violation,
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
        /// Who holds each version's bytes, in [`Self::history`]'s order
        /// (deletes left out): a version no one holds cannot be restored.
        #[action(effect = read_only)]
        async fn held(&self, root: String, path: String) -> Result<Vec<VersionBytes>, AgentError>;
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
        /// The registry, offline: projects, assets and files whose path,
        /// name or properties hold every word (`sha:<hex>` by content,
        /// `org:`, `root:`, `kind:` to narrow), each with where its bytes are.
        #[action(effect = read_only)]
        async fn search(&self, query: String) -> Result<Vec<Hit>, AgentError>;
        /// Where a file's bytes are: every copy, its holder and site, how
        /// sure we are of it and since when. `root` empty: `path` is a hash.
        #[action(effect = read_only)]
        async fn locations(&self, root: String, path: String) -> Result<Locations, AgentError>;
        /// Copy counts of an org, a root (its place) or everything (`""`):
        /// bytes under policy, on one holder, unverified; each holder's share.
        #[action(effect = read_only)]
        async fn copies(&self, scope: String) -> Result<CopyReport, AgentError>;
        /// Set the minimum copies of an org or a root (`min_verified` 0 clears it).
        async fn policy_set(
            &self,
            scope: String,
            min_verified: u32,
            min_sites: u32,
        ) -> Result<Policy, AgentError>;
        /// The files under their copy policy in an org, a root or everything.
        #[action(effect = read_only)]
        async fn policy_check(&self, scope: String) -> Result<Vec<Violation>, AgentError>;
        /// Re-hash this device's copies now (an org, a root, a hash, or
        /// everything due with `""`) and challenge reachable peers' copies.
        async fn scrub(&self, scope: String) -> Result<ScrubReport, AgentError>;
        /// Drop a file's (or a folder's, `""` the root's) cached bytes here,
        /// unless that leaves a file under its policy; how many went.
        async fn evict(&self, root: String, path: String) -> Result<u32, AgentError>;
        /// Every holder of the orgs this device sees, and whether it is reachable now.
        #[action(effect = read_only)]
        async fn holders(&self) -> Result<Vec<HolderStatus>, AgentError>;
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
