//! The store kept within its budget (#525) without ever losing a byte
//! (#527).
//!
//! Never losing bytes comes first, a bounded store second: where the two
//! conflict the engine defers or refuses and says so, it never deletes.
//!
//! The store ([`architect::blob::LocalFs`] under a
//! [`architect::blob::StoreBudget`]) refuses a write that would take it
//! past its limit or its filesystem under the keep-free floor; the engine
//! defers what it could not fetch or keep and tries again each pass
//! (`RootStatus::deferred`), and a file larger than the whole limit is
//! reported as such (`RootStatus::too_large`). After each pass
//! [`Engine::keep_store`]:
//!
//! 1. reconciles each area's running total with a walk of it;
//! 2. removes orphans older than [`ORPHAN_GRACE`]: partial fetches no
//!    transfer is resuming, temporary puts, staged copies;
//! 3. over [`SOFT_PERCENT`] of the store's limit, drops store blobs whose
//!    bytes a working file holds in place, re-hashed just before
//!    ([`Engine::evict_for_space`]), until under [`TARGET_PERCENT`].
//!
//! Nothing else is dropped for space: not an old version, not a blob only
//! another holder also has (#530). Old versions are kept until the owner
//! says otherwise; a full store waits.

use std::time::{Duration, SystemTime};

use architect::blob::{Sha256, StoreUsage};

use super::engine::Engine;

/// Over this share of its limit, the store drops what is safe to drop.
pub const SOFT_PERCENT: u64 = 90;
/// What eviction brings it down to.
pub const TARGET_PERCENT: u64 = 80;
/// How old a partial fetch or a staged copy must be before it counts as
/// an orphan (a transfer resuming within it keeps its bytes).
pub const ORPHAN_GRACE: Duration = Duration::from_hours(1);

impl Engine {
    /// Where the store stands against its budget, if it has one.
    #[must_use]
    pub fn store_usage(&self) -> Option<StoreUsage> {
        self.store.local.budget().map(|b| b.usage())
    }

    /// Where the kept versions stand (a holding node's area on its
    /// tree's filesystem), when they are not in the store.
    #[must_use]
    pub fn history_usage(&self) -> Option<StoreUsage> {
        (self.store.history.root() != self.store.local.root())
            .then(|| self.store.history.budget().map(|b| b.usage()))
            .flatten()
    }

    /// Where incoming bytes stand (a holding node's area on its tree's
    /// filesystem), when they do not land in the store.
    #[must_use]
    pub fn inbox_usage(&self) -> Option<StoreUsage> {
        (self.store.inbox.root() != self.store.local.root())
            .then(|| self.store.inbox.budget().map(|b| b.usage()))
            .flatten()
    }

    /// One look at the store (see the module docs).
    pub async fn keep_store(&self) {
        let running = self.fetcher.running();
        for area in self.store.areas() {
            area.clean_orphans(ORPHAN_GRACE, &running).await;
            area.reconcile().await;
        }
        let local = &self.store.local;
        let over = |percent| local.budget().is_some_and(|b| b.over(percent));
        if !over(SOFT_PERCENT) {
            return;
        }
        let (Ok(all), Ok(pinned)) = (local.list().await, local.pinned().await) else {
            return;
        };
        let mut duplicates: Vec<(SystemTime, Sha256)> = all
            .into_iter()
            .filter(|h| !pinned.contains(h) && self.store.in_place.find(h).is_some())
            .map(|h| {
                let at = std::fs::metadata(local.path_for(&h))
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (at, h)
            })
            .collect();
        duplicates.sort();
        for (_, hash) in duplicates {
            if !over(TARGET_PERCENT) {
                break;
            }
            // Refused (changed under an equal stat, pinned meanwhile): it stays.
            let _ = self.evict_for_space(hash).await;
        }
    }
}
