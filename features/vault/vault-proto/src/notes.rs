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
    /// [`Notes::create`] found a note at that path already: it is left
    /// as it is.
    #[error("{0} exists already")]
    #[architect(http_status = 409)]
    Exists(String),
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

/// The notes of the vaults this node holds. With the `actions` feature
/// (the node's) its actions too (`NOTES_*`, on the node's Action List);
/// otherwise the service and its client alone.
#[cfg_attr(feature = "actions", architect::service(actions))]
#[cfg_attr(not(feature = "actions"), architect::service)]
// Without `actions` the trait is mostly a client's: nothing implements it
// outside the node.
#[cfg_attr(not(feature = "actions"), allow(async_fn_in_trait))]
pub trait Notes {
    /// Every note this node holds.
    #[cfg_attr(feature = "actions", action(effect = read_only))]
    async fn list(&self) -> Result<Vec<NoteRef>, NotesError>;
    /// A note's markdown.
    #[cfg_attr(feature = "actions", action(effect = read_only))]
    async fn read(&self, vault: String, path: String) -> Result<String, NotesError>;
    /// Write a note's markdown; it reaches every peer on the next sweep.
    async fn write(&self, vault: String, path: String, text: String) -> Result<(), NotesError>;
    /// Make a new note with `text`, only if no note is at `path`:
    /// [`NotesError::Exists`] otherwise, the note there untouched. What a
    /// "New note" calls, so it never empties someone's note of that name.
    async fn create(&self, vault: String, path: String, text: String) -> Result<(), NotesError>;
}

pub use prelude::*;
