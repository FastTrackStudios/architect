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

    /// One look at the store (see the module docs). Holds the pass's
    /// lock throughout, so no write decides "the store keeps these
    /// bytes" while they are being dropped.
    pub async fn keep_store(&self) {
        let _busy = self.busy().await;
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
        let Some(old) = self.old_contents().await else {
            // A root's history cannot be read: none of its old versions is
            // known, so nothing is dropped this pass.
            tracing::warn!("files: a root's history cannot be read: no eviction this pass");
            return;
        };
        let mut duplicates: Vec<(SystemTime, u64, Sha256)> = all
            .into_iter()
            .filter(|h| {
                !pinned.contains(h)
                    && !old.contains(&h.to_hex())
                    && self.store.in_place.find(h).is_some()
            })
            .filter_map(|h| {
                let meta = std::fs::metadata(local.path_for(&h)).ok()?;
                let at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                Some((at, meta.len(), h))
            })
            .collect();
        duplicates.sort();
        let candidates = duplicates.len();
        // A bounded amount per pass: each drop re-reads its working file
        // (from NFS on a holding node), and sync waits while this runs.
        let (mut read, mut dropped, mut freed) = (0u64, 0u64, 0u64);
        for (_, len, hash) in duplicates {
            if !over(TARGET_PERCENT) || read >= EVICT_READ_PER_PASS {
                break;
            }
            read = read.saturating_add(len);
            // Refused (changed under an equal stat, pinned meanwhile): it stays.
            if self.evict_for_space_locked(hash, &old).await.is_ok() {
                dropped = dropped.saturating_add(1);
                freed = freed.saturating_add(len);
            }
        }
        let usage = local.budget().map(|b| b.usage());
        tracing::info!(
            store.dropped = dropped,
            store.freed = freed,
            store.read = read,
            store.candidates = candidates,
            store.used = usage.map(|u| u.used),
            store.max = usage.map(|u| u.max),
            store.done = !over(TARGET_PERCENT),
            "files: store over its soft limit: dropped copies of working files"
        );
    }

    /// The working files holding `hash`: their index entries and the
    /// in-place record take the file's stat as it is now, if it is the
    /// same file (only its ctime moved: a link to it came or went).
    pub(super) fn restat(&self, hash: Sha256) {
        let hex = hash.to_hex();
        for doc_id in self.known() {
            let Some(dir) = self.working_dir(doc_id) else {
                continue;
            };
            self.with_index(doc_id, |index| {
                let mut changed = false;
                for (path, known) in &mut index.files {
                    if known.content != hex {
                        continue;
                    }
                    let file = dir.join(path);
                    let Ok(meta) = std::fs::metadata(&file) else {
                        continue;
                    };
                    let now = super::scan::Stat::of(&meta);
                    if now.same_file(&known.stat) {
                        known.stat = now;
                        self.store.in_place.insert(hash, file, now);
                        changed = true;
                    }
                }
                changed
            });
        }
    }

    /// Once per start: a file in a store area that shares its inode with
    /// another name (a hard link left by a run that died between linking
    /// a version aside and the swap, or between a swap and removing the
    /// incoming copy) may be written in place through that other name.
    /// Each is re-hashed: bad bytes are quarantined and the claim
    /// withdrawn; good ones get an inode of their own (a verified copy
    /// renamed over the link). The count of each.
    pub(super) async fn check_links(&self) -> (u32, u32) {
        let mut shared = Vec::new();
        for area in self.store.areas() {
            shared.extend(linked_blobs(area.root()));
        }
        let (mut bad, mut split) = (0u32, 0u32);
        for (hash, path) in shared {
            match architect::blob::hash_file(&path).await {
                Ok((_, actual)) if actual == hash => {
                    let copied = async {
                        let staged = architect::blob::stage(&path, &path, hash, false).await?;
                        architect::blob::commit(&staged.path, &path, false).await
                    };
                    if copied.await.is_ok() {
                        split = split.saturating_add(1);
                        // The split moved the ctime of the working file
                        // that shared the inode: re-read its stat.
                        self.restat(hash);
                    }
                }
                _ => {
                    self.quarantine(hash, &path).await;
                    bad = bad.saturating_add(1);
                }
            }
        }
        if bad > 0 || split > 0 {
            tracing::warn!(
                store.linked_bad = bad,
                store.linked_split = split,
                "files: store files shared with another name were checked"
            );
        }
        (bad, split)
    }
}

/// How many bytes of working files one pass re-reads to drop store
/// copies: 2 GiB (about 20 s at 1 Gb/s), so sync never waits long.
pub const EVICT_READ_PER_PASS: u64 = 2 << 30;

/// The blobs under `root` with more than one name.
fn linked_blobs(root: &std::path::Path) -> Vec<(Sha256, std::path::PathBuf)> {
    let mut out = Vec::new();
    for prefix in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if prefix.file_name().len() != 2 {
            continue;
        }
        for entry in std::fs::read_dir(prefix.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(hash) = Sha256::from_hex(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            if entry.metadata().is_ok_and(|m| links(&m) > 1) {
                out.push((hash, entry.path()));
            }
        }
    }
    out
}

#[cfg(unix)]
fn links(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    meta.nlink()
}

#[cfg(not(unix))]
const fn links(_meta: &std::fs::Metadata) -> u64 {
    1
}
