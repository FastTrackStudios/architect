---
"architect-desktop" = "minor"
"vault" = "minor"
"vault-live" = "minor"
"vault-obsidian" = "minor"
"vault-proto" = "minor"
"vault-query" = "minor"
---

Added the vault index (#710, #272): vault-query, a wasm-clean crate. vault_query::index has an Entry per note (title, properties, tags, links, headings, task lines; no body) and per .base file; Builder keeps it a note at a time from any NoteSource and answers IndexDelta deltas (Builder::since); Built keeps one over a source; VaultIndex::filtered and IndexDelta::filtered give a reader only what it may read. OpenVault is a NoteSource, and OpenVault::revision says where it is in its writes. The node serves the index to browsers with vault_proto::index_sync::VaultIndexSync (snapshot, since) on its org lane. vault-live's bases and refs and vault-obsidian's obsidian_parse moved to vault-query and are re-exported where they were.
