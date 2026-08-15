//! `Persistence` — the seam between a `CrdtDoc` and wherever its
//! bytes actually live. Object-safe (via `async-trait`) so a doc can
//! hold `Arc<dyn Persistence>` and the call sites stay uniform across
//! deployments.
//!
//! Two storage shapes, both supported by one trait:
//!
//! - **Snapshot** — `load_snapshot` / `write_snapshot`. The full state
//!   in one blob. Simple, larger writes.
//! - **Update log** — `append_update` per commit, `load_updates` on
//!   cold start. Append-mostly. Eventually `compact` to fold the log
//!   into a fresh snapshot and prune.
//!
//! Backends are free to support both or just one — `CrdtDoc::open`
//! loads the snapshot first, then replays everything in `load_updates`
//! on top. A "snapshot-only" backend can return an empty Vec for
//! `load_updates`; an "updates-only" backend can return None for
//! `load_snapshot`.

use async_trait::async_trait;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("backend: {0}")]
    Backend(String),
}

/// Marker supertrait: `Send + Sync` on native, nothing on wasm. Browser
/// storage handles (IndexedDB) are single-threaded and `!Send`; wasm is
/// single-threaded anyway, so the bound only exists where threads do.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSendSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSendSync for T {}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Persistence: MaybeSendSync + 'static {
    /// Load the most recent compacted snapshot for `doc_id`, or
    /// `None` if there isn't one yet (first-ever open).
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError>;

    /// Replace the stored snapshot. Called by `CrdtDoc::compact`.
    /// Backends supporting an update log should atomically prune
    /// updates older than this snapshot's frontiers.
    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError>;

    /// Append a single local-update blob. Called from inside the
    /// LoroDoc's `subscribe_local_update` callback on every commit.
    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError>;

    /// Load every stored update for `doc_id` in commit order. Used
    /// during cold open to replay history on top of the snapshot.
    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError>;

    /// Atomic snapshot-and-prune. The default impl does the two
    /// writes non-atomically — backends with real transactions
    /// should override to make this a single round-trip.
    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        self.write_snapshot(doc_id, snapshot).await
    }
}
