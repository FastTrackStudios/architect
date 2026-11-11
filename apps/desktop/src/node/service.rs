//! What the node answers, declared once each (`#[architect::service]`).
//!
//! - [`Node`]: what this node is (`status`). Its whole face is public, so
//!   it is served on every wire: same-origin vox (`/vox`), the HTTP face
//!   (`POST /node/status`) and the Action List (`NODE_STATUS`, over MCP).
//! - [`Notes`]: the notes of the vaults it holds, read and written. Only
//!   on the Action List, so only over `/mcp`, which needs its bearer: an
//!   agent writes a note on the always-on node and every device that
//!   syncs with it gets it.

use std::sync::{Arc, Mutex};

use architect_devices::Admission;
use vault_live::Backend;
use vault_proto::{IfMatch, VaultSync as _, VaultSyncError};

use super::catalog::VaultCatalog;

/// Why the node refused.
#[architect::error]
#[derive(Eq)]
pub enum NodeError {
    #[error("no such vault or note: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

impl From<VaultSyncError> for NodeError {
    fn from(e: VaultSyncError) -> Self {
        match e {
            VaultSyncError::NotFound => Self::NotFound("the vault or the note".to_owned()),
            other => Self::Refused(other.to_string()),
        }
    }
}

/// What a node is: its name and endpoint, what it holds and who it knows.
#[architect::wire]
#[derive(Eq)]
pub struct NodeStatus {
    /// Its name on the account.
    pub name: String,
    /// `CARGO_PKG_VERSION` of the build.
    pub version: String,
    /// Its iroh endpoint id: how the others dial it.
    pub endpoint: String,
    /// The org the account's devices share, once the pass found one.
    pub org: Option<String>,
    /// Whether its last account pass went through.
    pub signed_in: bool,
    /// How many vaults it holds, and their notes.
    pub vaults: u32,
    pub notes: u32,
    /// Where its Sites are mounted.
    pub sites: Vec<String>,
    /// The devices it lets in.
    pub devices: u32,
    /// Its blob store against its budget, when it runs the files engine.
    pub files_store: Option<FilesStore>,
}

/// The files engine's blob store against its budget (#525).
#[architect::wire]
#[derive(Eq)]
pub struct FilesStore {
    /// Bytes in the store, and promised to writes in flight.
    pub used: u64,
    pub reserved: u64,
    /// Its limit (`files_store_max`).
    pub max: u64,
    /// The floor kept free on its filesystem (`files_keep_free`), and
    /// that filesystem's free bytes and size, when it can be asked.
    pub keep_free: u64,
    pub fs_free: Option<u64>,
    pub fs_total: Option<u64>,
    /// Fetches waiting for room now, and writes refused for space since start.
    pub deferred: u32,
    pub refused: u64,
}

/// A note in a vault.
#[architect::wire]
#[derive(Eq)]
pub struct NoteRef {
    pub vault: String,
    pub path: String,
}

/// [`Node`]'s declaration: one service per module, as the generated
/// names (`http`, `prelude`) are per module.
pub mod node {
    use super::{NodeError, NodeStatus};

    /// What this node is.
    #[architect::service(actions)]
    pub trait Node {
        /// This node: its name, endpoint, org, vaults, Sites, devices, and
        /// its blob store against its budget.
        #[action(effect = read_only)]
        async fn status(&self) -> Result<NodeStatus, NodeError>;
    }

    pub use prelude::*;
}

/// [`Notes`]'s declaration.
pub mod notes {
    use super::{NodeError, NoteRef};

    /// The notes of the vaults this node holds.
    #[architect::service(actions)]
    pub trait Notes {
        /// Every note this node holds.
        #[action(effect = read_only)]
        async fn list(&self) -> Result<Vec<NoteRef>, NodeError>;
        /// A note's markdown.
        #[action(effect = read_only)]
        async fn read(&self, vault: String, path: String) -> Result<String, NodeError>;
        /// Write a note's markdown; it reaches every peer on the next sweep.
        async fn write(&self, vault: String, path: String, text: String) -> Result<(), NodeError>;
    }

    pub use prelude::*;
}

pub use node::Node;
pub use notes::Notes;

/// What the account pass last said.
#[derive(Debug, Clone, Default)]
pub struct AccountView {
    pub org: Option<String>,
    pub signed_in: bool,
}

/// The node's state, as its services see it. Cheap to clone.
#[derive(Clone)]
pub struct NodeState {
    pub(crate) name: String,
    pub(crate) endpoint: String,
    pub(crate) backend: Backend,
    pub(crate) sites: Vec<String>,
    pub(crate) admission: Admission,
    pub(crate) account: Arc<Mutex<AccountView>>,
    pub(crate) files: Option<architect_files::sync::Engine>,
}

impl NodeState {
    fn catalog(&self) -> VaultCatalog {
        VaultCatalog::new(self.backend.clone())
    }

    /// Whether the node lets `endpoint` in.
    #[must_use]
    pub fn admits(&self, endpoint: &str) -> bool {
        self.admission.admits(endpoint)
    }

    /// Where vault `id` lives on this node.
    #[must_use]
    pub fn vault_root(&self, id: &str) -> Option<std::path::PathBuf> {
        self.backend.root(id).ok()
    }

    /// The status, without the wire.
    #[must_use]
    pub fn snapshot(&self) -> NodeStatus {
        let account = architect::lock(&self.account).clone();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        NodeStatus {
            name: self.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            endpoint: self.endpoint.clone(),
            org: account.org,
            signed_in: account.signed_in,
            vaults: count(self.backend.vault_ids().len()),
            notes: count(self.catalog().notes().len()),
            sites: self.sites.clone(),
            devices: count(self.admission.list().len()),
            files_store: self.files.as_ref().and_then(|engine| {
                let usage = engine.store_usage()?;
                Some(FilesStore {
                    used: usage.used,
                    reserved: usage.reserved,
                    max: usage.max,
                    keep_free: usage.keep_free,
                    fs_free: usage.space.map(|s| s.free),
                    fs_total: usage.space.map(|s| s.total),
                    deferred: engine.deferred(),
                    refused: usage.refused,
                })
            }),
        }
    }
}

impl Node for NodeState {
    async fn status(&self) -> Result<NodeStatus, NodeError> {
        Ok(self.snapshot())
    }
}

impl Notes for NodeState {
    async fn list(&self) -> Result<Vec<NoteRef>, NodeError> {
        Ok(self
            .catalog()
            .notes()
            .into_iter()
            .filter_map(|d| {
                d.name.split_once('/').map(|(v, p)| NoteRef {
                    vault: v.to_owned(),
                    path: p.to_owned(),
                })
            })
            .collect())
    }

    async fn read(&self, vault: String, path: String) -> Result<String, NodeError> {
        let bytes = self.backend.get_file(&vault, &path)?;
        Ok(String::from_utf8_lossy(&bytes.0).into_owned())
    }

    async fn write(&self, vault: String, path: String, text: String) -> Result<(), NodeError> {
        // Registered first, so an open document merges the write as a
        // change rather than the next open finding it.
        self.backend.register_collab(&vault, &path)?;
        // `put_file` announces the write under a blocking lock: off the
        // runtime's worker threads.
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.put_file(&vault, &path, text.into_bytes(), IfMatch::Force)
        })
        .await
        .map_err(|e| NodeError::Refused(e.to_string()))??;
        Ok(())
    }
}

/// The public wires: [`Node`] alone.
impl architect::Services for NodeState {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![node::NodeService]
    }
}
