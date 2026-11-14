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

/// A vault's index on the wire.
#[architect::wire]
#[derive(Eq, Default)]
pub struct IndexBytes {
    /// The revision it is at.
    pub revision: u64,
    /// `VaultIndex::to_bytes` (a snapshot) or `IndexDelta::to_bytes`.
    pub bytes: Vec<u8>,
}

/// A vault's index, synced to a client that does not hold the vault.
#[architect::service]
#[allow(async_fn_in_trait)]
pub trait VaultIndexSync {
    /// The whole index of `vault`, as the caller may read it.
    async fn snapshot(&self, vault: String) -> Result<IndexBytes, IndexError>;
    /// What changed in `vault`'s index since revision `from`, as the
    /// caller may read it; `None` when that is further back than the
    /// deltas kept (take a snapshot).
    async fn since(&self, vault: String, from: u64) -> Result<Option<IndexBytes>, IndexError>;
}

pub use prelude::*;
