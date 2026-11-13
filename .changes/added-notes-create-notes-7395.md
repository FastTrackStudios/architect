---
"vault-proto" = "minor"
---

Added `Notes::create` (`NOTES_CREATE`): a new note only where none is, else `NotesError::Exists`. `vault_proto::notes` is behind the `vox` feature, and builds natively on its own (`architect/action-list`).
