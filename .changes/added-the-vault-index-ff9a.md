---
"vault" = "minor"
"vault-live" = "minor"
"vault-obsidian" = "minor"
"vault-proto" = "minor"
"vault-query" = "minor"
---

Added the vault index (#710, #272): vault-query, a wasm-clean crate. vault_query::index has an Entry per note (title, properties, tags, links, headings, task lines; no body) and per .base file, capped (truncated) with interned keys, tags and links; Builder keeps it a note at a time from any NoteSource (Note::text is Text::Here or Text::Load, read only when the stamp moved) and answers IndexDelta deltas of one history (a random epoch; Builder::since(epoch, from)); Built keeps one over a source. OpenVault is a NoteSource (its stamps hash each note's text, OpenVault::revision), and vault::actions::query::DiskSource is a vault folder that holds no text. The node serves the index to browsers with vault_proto::index_sync::VaultIndexSync (snapshot, since(vault, epoch, access, from)) on its org lane, filtered per caller (OrgLane::reads, OrgLane::read_key). vault-live's bases and refs and vault-obsidian's obsidian_parse moved to vault-query and are re-exported where they were.
