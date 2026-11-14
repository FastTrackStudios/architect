---
"architect-navigator" = "minor"
"architect-extensions" = "minor"
"architect-app" = "minor"
---

Added `architect_navigator::documents::VaultDocuments` (feature `documents`): the person's vault as extensions' `Documents`, written through the `VaultSource` the navigator and editors share, so an extension's edit (`documents::apply_edits`, one Loro commit) merges into an open note's typing; `VaultDocuments::raise_on` raises the person's own saves, new notes, renames and deletes as `doc.*`. The Architect app's extension host now reads and writes the opened vault on desktop and iOS (#706).
