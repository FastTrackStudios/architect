---
"architect-desktop" = "minor"
"vault-proto" = "minor"
---

Moved the node's `Notes` service (`notes::service::Notes`, `NoteRef`, `NodeError`) into vault-proto behind feature `notes`, so an app calls it as `notes::service::NotesClient`; architect-desktop re-exports it where it was.
