//! The notes of the vaults a node holds, as a service: what a signed-in
//! client (the web app, `apps/web`) lists, reads and writes over the node's
//! org lane (`/org/{slug}/vox`, the session as the bearer). The node
//! (`apps/desktop/src/node/service.rs`) implements it; declared here, beside
//! the vault's other contracts, so a browser has the client.

use crate::VaultSyncError;

/// A note in a vault.
#[architect::wire]
#[derive(Eq)]
pub struct NoteRef {
    pub vault: String,
    pub path: String,
}

/// Why the node refused.
#[architect::error]
#[derive(Eq)]
pub enum NotesError {
    #[error("no such vault or note: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

impl From<VaultSyncError> for NotesError {
    fn from(e: VaultSyncError) -> Self {
        match e {
            VaultSyncError::NotFound => Self::NotFound("the vault or the note".to_owned()),
            other => Self::Refused(other.to_string()),
        }
    }
}

/// The notes of the vaults this node holds. Its actions (`NOTES_*`, on the
/// node's Action List) natively; a browser has the client alone.
#[cfg_attr(not(target_arch = "wasm32"), architect::service(actions))]
#[cfg_attr(target_arch = "wasm32", architect::service)]
pub trait Notes {
    /// Every note this node holds.
    #[cfg_attr(not(target_arch = "wasm32"), action(effect = read_only))]
    async fn list(&self) -> Result<Vec<NoteRef>, NotesError>;
    /// A note's markdown.
    #[cfg_attr(not(target_arch = "wasm32"), action(effect = read_only))]
    async fn read(&self, vault: String, path: String) -> Result<String, NotesError>;
    /// Write a note's markdown; it reaches every peer on the next sweep.
    async fn write(&self, vault: String, path: String, text: String) -> Result<(), NotesError>;
}

pub use prelude::*;
