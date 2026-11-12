//! The notes a node holds, as a service (feature `notes`): what the node
//! answers on its Action List and on an org's lane (`/org/{slug}/vox`,
//! to the org's members), and what an app pulls an org's vault with.
//!
//! Declared here, not in the node, so the app can call it too: the node
//! implements [`service::Notes`] (`apps/desktop/src/node/service.rs`), the
//! app reads it as a client (`service::NotesClient`).

use crate::VaultSyncError;

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

/// A note in a vault.
#[architect::wire]
#[derive(Eq)]
pub struct NoteRef {
    pub vault: String,
    pub path: String,
}

/// [`service::Notes`]'s declaration: one service per module, as the
/// generated names (`http`, `prelude`) are per module.
pub mod service {
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
