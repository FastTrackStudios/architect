---
"architect-files" = "major"
---

Added a vault's attachments as a synced root: with `vault_files` (`ARCHITECT_VAULT_FILES=true`) a node holds each vault's files that are not notes (recordings, PDFs, covers) as the root `<org>/Vaults/<vault>` where the vault is, so they sync between the nodes that hold the vault, go to the bucket and follow the holding rule (`node::files::not_an_attachment`, `Config::vault_files_org`, `Config::files_root`). In `architect-files`: `Section::Vaults`, `Engine::hold_at(meta, dir, skip)` and `sync::Skip` (a root held in place anywhere, the paths that are not its own never scanned, written, saved or deleted, whatever a manifest names), `Scanner::scan_but`, `layout::id_for_place`. Fixed two devices that each start one root's manifest (a vault held on two nodes, one project tree on two machines) losing one side's versions when they met: every device starts a manifest with the same first change (`Manifest::seed`), so the two are one history and merge.

Migration: `architect_files::sync::Section` has a new variant, `Vaults`, so an exhaustive `match` on it adds an arm.
