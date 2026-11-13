---
"vault-proto" = "minor"
---

Added `Notes::create` (`NOTES_CREATE` with `actions`): a new note only where none is, else `NotesError::Exists`. `vault_proto::notes` is behind the `vox` feature; the Action List side behind `actions`, so crates reading the vault's types do not pull in the native dispatcher.
