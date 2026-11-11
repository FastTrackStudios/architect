//! The store kept within its budget (#525).
//!
//! The store ([`architect::blob::LocalFs`] under a
//! [`architect::blob::StoreBudget`]) refuses a write that would take it
//! past its limit or its filesystem under the keep-free floor; the engine
//! defers what it could not fetch and tries again each pass. After each
//! pass [`Engine::keep_store`]:
//!
//! 1. reconciles the budget's running total with a walk of the store;
//! 2. drops every blob a working file also holds (nothing stored twice):
//!    always on a holding node, on other devices once over the soft
//!    threshold (their store keeps the history of small files);
//! 3. over [`SOFT_PERCENT`] of the limit, drops the oldest blobs through
//!    [`Engine::evict_for_space`] (the copy policy; never a last copy,
//!    never a pinned blob) until under [`TARGET_PERCENT`].
//!
//! When nothing is safe to drop the store stays full and fetches stay
//! deferred: they go ahead once space appears (another holder verifies a
//! copy, a pin is lifted, the budget is raised).

use std::time::SystemTime;

use architect::blob::{Sha256, StoreUsage};

use super::engine::Engine;

/// Over this share of its limit, the store drops what is safe to drop.
pub const SOFT_PERCENT: u64 = 90;
/// What eviction brings it down to.
pub const TARGET_PERCENT: u64 = 80;

impl Engine {
    /// Where the store stands against its budget, if it has one.
    #[must_use]
    pub fn store_usage(&self) -> Option<StoreUsage> {
        self.store.local.budget().map(|b| b.usage())
    }

    /// One look at the store (see the module docs).
    pub async fn keep_store(&self) {
        let local = &self.store.local;
        local.reconcile().await;
        let over = |percent| local.budget().is_some_and(|b| b.over(percent));
        if !(self.config().hold_all || over(SOFT_PERCENT)) {
            return;
        }
        let (Ok(all), Ok(pinned)) = (local.list().await, local.pinned().await) else {
            return;
        };
        let mut rest: Vec<(SystemTime, Sha256)> = Vec::new();
        for hash in all {
            if pinned.contains(&hash) {
                continue;
            }
            if self.store.in_place.find(&hash).is_some() {
                // A working file holds these bytes: the store's are a
                // second copy on the same device.
                let _ = local.remove(&hash);
                continue;
            }
            let at = std::fs::metadata(local.path_for(&hash))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            rest.push((at, hash));
        }
        if !over(SOFT_PERCENT) {
            return;
        }
        rest.sort();
        for (_, hash) in rest {
            if !over(TARGET_PERCENT) {
                break;
            }
            // Refused under policy or as a last copy: it stays.
            let _ = self.evict_for_space(hash).await;
        }
    }
}
