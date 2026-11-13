---
"vault-collab" = "patch"
---

Fixed: a note whose file is deleted on disk while its document is closed (registered, evicted, or made through `Notes::create` and never opened) no longer deadlocks the document registry. The delete listener opened the document under its own reconcile lock while the registry held its creation lock, so no note on that node opened live again (and its vault took no more outside edits) until a restart. `apply_delete` and `apply_inbound` now fetch the document before taking its lock.
