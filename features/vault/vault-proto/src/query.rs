//! The vault's query service and its index's sync.
//!
//! - [`VaultQuery`]: a base's view run over the vault index. Every client
//!   runs it over its own index, in process (`architect::serve_local`): a
//!   desk's and a phone's kept from their vault, a browser's synced from
//!   the node. Its one implementation is `vault_query::QueryEngine`;
//!   Bases, the Calendar and the extension host hold only the client.
//! - `VaultIndexSync` (`crate::index_sync`): the index, from where the vault lives to a
//!   client that has only the notes it opens (a browser): a snapshot, then
//!   the deltas since a revision, filtered to what the caller may read.
//!   The node serves it on its org lane (`crate::index_sync`). Both carry `vault_query::index`'s
//!   wire form (JSON: `VaultIndex::to_bytes`, `IndexDelta::to_bytes`).

/// One query: a base's view, given as a `.base` file's path or its YAML.
#[architect::wire]
#[derive(Eq, Default)]
pub struct QueryRequest {
    /// A `.base` file's vault path.
    pub path: Option<String>,
    /// A base written as YAML, run in place of a file's.
    pub source: Option<String>,
    /// The view by name; the first when left out.
    pub view: Option<String>,
    /// The `hash` of an answer had before: the same answer says
    /// `{unchanged: true}`.
    pub if_changed: Option<String>,
    /// At most this many rows over every group.
    pub limit: Option<u32>,
}

/// Why a query was refused.
#[architect::error]
#[derive(Eq)]
pub enum QueryError {
    #[error("no such vault: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    /// No base at the path, a view that is not there, YAML that does not
    /// parse.
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

/// The vault's query service.
#[architect::service]
// Mostly a client's: `vault_query::QueryEngine` is its one implementation.
#[allow(async_fn_in_trait)]
pub trait VaultQuery {
    /// Runs one view of a base over the vault index: JSON text of its
    /// views, the view, its rows in groups, `total`, every column,
    /// formulas, filters, the index's `revision` and the answer's `hash`.
    async fn query(&self, request: QueryRequest) -> Result<String, QueryError>;
}

pub use prelude::*;
