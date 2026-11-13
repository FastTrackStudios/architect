---
"vault-collab" = "patch"
---

Fixed a deadlock that stopped every live note on a node: deleting the file of a note whose document was closed (a new note never opened, or one closed for being idle) made the delete handler open the document under its own lock while the registry held its creation lock; no note attached again until a restart. The document is now fetched before its lock is taken (apply_delete, apply_inbound).

A note's file deleted while its document was closed can no longer come back: the delete now marks the document before opening it, so the open sets the tombstone itself (under the lock it holds) instead of scheduling a write-back of the old text that could run before the delete got its lock.
