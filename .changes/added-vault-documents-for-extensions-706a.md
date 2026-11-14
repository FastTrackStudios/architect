---
"architect-navigator" = "minor"
"architect-extensions" = "minor"
"architect-app" = "minor"
"vault-obsidian" = "patch"
"vault" = "minor"
"architect-pages" = "none"
---

Added `architect_navigator::documents::VaultDocuments` (feature `documents`): the person's vault as extensions' `Documents`, written through the `VaultSource` the navigator and editors share, so an extension's edit (`documents::apply_edits`, one Loro commit) merges into an open note's typing; `VaultDocuments::raise_on` raises the person's own saves, new notes, renames and deletes as `doc.*`. The Architect app's extension host now reads and writes the opened vault on desktop and iOS (#706).

Changed: `Documents::edit` answers `documents::Edited { text, changed }`, and the host raises no `doc.saved` for an edit that changed nothing. Added `OpenVault::save_if` and `VaultSource::save_if` (compare-and-swap). Fixed: the vault's page writes (`create_page`, `move_page`, `delete_page`, `save_base`, `Vault::save_page`, and `OpenVault`'s moves, copies and trash) refuse a path through a symlink, and notes are written atomically; a hook on `changed:doc.*` needs `reads = ["documents"]`, and the host no longer raises `changed:doc.*`/`changed:app.*` copies; an extension's `data.json` is written atomically with mode 0600 under a lock, and one that does not parse is refused rather than wiped.

Migration: an app's own `Documents` returns `Edited { text, changed }` from `edit`.
