//! The vault index (`vault_query::index`) synced to a client that does not
//! hold the vault (a browser), from where the vault lives. The node
//! serves it on its org lane, each answer filtered to what the caller may
//! read.

/// Why the index was not given.
#[architect::error]
#[derive(Eq)]
pub enum IndexError {
    #[error("no such vault: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

/// A vault's index on the wire, and where its reader is.
#[architect::wire]
#[derive(Eq, Default)]
pub struct IndexBytes {
    /// The builder's history: a node started again is another one, and a
    /// `since` of another history answers a snapshot.
    pub epoch: u64,
    /// A fingerprint of what this reader may read: when it moves (a role
    /// changed, a note shared or unshared), `since` answers a snapshot.
    pub access: u64,
    /// The revision it is at.
    pub revision: u64,
    /// `VaultIndex::to_bytes` (a snapshot) or `IndexDelta::to_bytes`.
    pub bytes: Vec<u8>,
}

/// A vault's index, synced to a client that does not hold the vault.
#[architect::service]
#[allow(async_fn_in_trait)]
pub trait VaultIndexSync {
    /// The index of `vault`, as the caller may read it.
    async fn snapshot(&self, vault: String) -> Result<IndexBytes, IndexError>;
    /// What changed in `vault`'s index, as the caller may read it, since
    /// revision `from` of history `epoch` read with access `access` (all
    /// three from the last answer). `None`: take a snapshot (another
    /// history, other access, or further back than the deltas kept). When
    /// nothing the caller may read changed, the answer stays at `from`, so
    /// a caller cannot count edits to notes it cannot read.
    async fn since(
        &self,
        vault: String,
        epoch: u64,
        access: u64,
        from: u64,
    ) -> Result<Option<IndexBytes>, IndexError>;
}

pub use prelude::*;
