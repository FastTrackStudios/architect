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
- **Placing bytes safely.** Added `architect::blob::{stage, commit, copy_verified_with, hash_file, STAGING, staging_path}`. Every copy stages under one temp suffix, syncs, verifies length and sha-256, renames and syncs the directory before removing a source.
  - `Engine::write_file` stages the new version first, keeps the old one, re-checks the target and swaps with no clobber.
  - A target that changed meanwhile is `EngineError::Changed`; it is left for the scan.
  - `layout::ignored` covers every temp name; leftovers are removed at start.
- **Keeping versions.** A replaced version is kept unless the versions area or the store already holds its bytes; another working file with the same bytes never counts.
  - A version that cannot be kept is `EngineError::NoHistory` (`RootStatus::no_history`, `history_error`).
  - Temp files are journalled (`sync::pending::Pending`); only the journal's are removed at start, and others are listed in `RootStatus::strays`.
  - At start, store files that share an inode with another name are re-hashed: changed ones are quarantined, unchanged ones get their own inode.
- **Deletes keep their bytes.** `Engine::apply` writes first and deletes last, and every delete (a move's old name, a peer's delete, a WebDAV or S3 DELETE, a checkpoint restore) keeps the bytes before removing the file. History at its bound is `EngineError::HistoryFull` (`RootStatus::history_full`, `files_store.history_full`).
- **Links.** `architect::blob::stage` links only a source with one name, else clones it (`FICLONE`) or copies and verifies it. The start-up check of shared store files runs before the first write and re-reads the working file's stat after a split. `NoRoom` names the area that refused.
- **A share the node cannot write.** `layout::claim_area` returns `Claimed::Unwritable`: the node starts with history off (`Engine::set_history_off`). An empty `.architect` is claimed, and the mark is written atomically.
- **Eviction.** `Engine::evict_for_space` drops only a store blob that a working file on the same device holds and that is no old version anywhere. It re-hashes the file just before, under the pass's lock, re-reading at most 2 GiB per pass (cross-holder eviction is #530). `Engine::keep_store` also cleans orphaned partial fetches (`Fetcher::running`).
- **History.** Added `Files::held` / `FILES_HELD` (`roots::VersionBytes`, `Engine::version_bytes`): who holds each version's bytes, so a version no one holds says so. `scan::Stat` now carries the ctime and inode (`Stat::same_file`); an index from before is upgraded in place.
- **The node.** It takes `ARCHITECT_FILES_STORE_MAX`, `ARCHITECT_FILES_KEEP_FREE` and `ARCHITECT_FILES_HISTORY_MAX` (bytes, `16Gi`, `1.5Gi`, `40%`, units in any case; 0 and `100%` are refused). Defaults: a quarter of the filesystem up to 100 GiB, 15% up to 64 GiB, and a tenth of the tree's filesystem up to 500 GiB for kept versions. An unreadable filesystem refuses writes that need the floor. A holding node refuses to start when `<files_dir>/.architect` is not its own folder (`layout::claim_area`). It logs each area's budget and where each number came from, and reports `files_store` in `NODE_STATUS`.
- **The chart.** It sets the limit to 75% of the data volume, validates `node.store.max`, sets the floor to 15%, adds `node.store.historyMax`, an `ephemeral-storage` limit that counts an `emptyDir` `/data`, and an `emptyDir` `sizeLimit`.

Migration: a `match` over `BlobError`, `Progress`, `EngineError` or `AgentError` needs arms for the new variants. A `Files` implementation needs `held`.
