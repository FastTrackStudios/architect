---
"architect" = "minor"
"architect-app" = "patch"
"architect-desktop" = "minor"
"architect-files" = "minor"
---

Fixed the files engine's unbounded blob store (#525) without letting it lose a byte. Never losing bytes comes first, a bounded store second: where the two conflict, the engine defers or refuses and says so; it never deletes.

- **The cause.** A node that holds its tree in place (`hold_all`) no longer copies the tree into `<data dir>/files/blobs` when it scans it (`EngineConfig::keep_inline` is 0 there). Its fetches land, and the versions it replaces are linked aside, on the tree's own filesystem: `TreeStore::with_areas`, `layout::areas`, `<files_dir>/.architect/{incoming,versions}`.
- **The budget.** Added `architect::blob::StoreBudget` (`LocalFs::with_budget`, `reserve`, `check_room`, `reconcile`, `remove`, `adopt`, `clean_orphans`, `fs_space`, `SpaceProbe`): a size limit and a keep-free floor, checked before any byte is written and shared by concurrent writers. An unreadable filesystem refuses what needs the floor.
  - A write with no room is `BlobError::NoRoom` (`NoRoom::too_large` when it exceeds the whole limit), `Progress::NoRoom`, `EngineError::NoRoom` / `TooLarge`, and `AgentError::NoRoom` (HTTP 507).
  - The engine defers such writes (`RootStatus::deferred`, `RootStatus::too_large`, `Engine::deferred`, `Engine::too_large`) and retries each pass.
- **Placing bytes safely.** Added `architect::blob::{stage, commit, move_file, copy_verified_with, hash_file, STAGING, staging_path}`. Every move stages under one temp suffix, syncs, verifies length and sha-256, renames and syncs the directory before removing a source.
  - `Engine::write_file` stages the new version first, keeps the old one, re-checks the target and swaps with no clobber.
  - A target that changed meanwhile is `EngineError::Changed`; it is left for the scan.
  - `layout::ignored` covers every temp name; leftovers are removed at start.
- **Eviction.** `Engine::evict_for_space` drops only a store blob a working file on the same device holds, re-hashed just before (cross-holder eviction is #530). `Engine::keep_store` also cleans orphaned partial fetches (`Fetcher::running`).
- **History.** Added `Files::held` / `FILES_HELD` (`roots::VersionBytes`, `Engine::version_bytes`): who holds each version's bytes, so a version no one holds says so. `scan::Stat` now carries the ctime and inode (`Stat::same_file`); an index from before is upgraded in place.
- **The node.** It takes `ARCHITECT_FILES_STORE_MAX`, `ARCHITECT_FILES_KEEP_FREE` and `ARCHITECT_FILES_HISTORY_MAX` (bytes, `16Gi`, `1.5Gi`, `40%`, units in any case; 0 and `100%` are refused). Defaults: a quarter of the filesystem up to 100 GiB, 15% up to 64 GiB, and every version kept. It logs each area's budget and where each number came from, and reports `files_store` in `NODE_STATUS`.
- **The chart.** It sets the limit to 75% of the data volume, validates `node.store.max`, sets the floor to 15%, adds `node.store.historyMax`, an `ephemeral-storage` limit that counts an `emptyDir` `/data`, and an `emptyDir` `sizeLimit`.

Migration: a `match` over `BlobError`, `Progress`, `EngineError` or `AgentError` needs arms for the new variants. A `Files` implementation needs `held`.
