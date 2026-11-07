---
"crdt" = "patch"
---

Turso stores wait up to 5 s for the file's lock instead of failing with "database is locked" when the index, the persistence or a restarted process hold it.
