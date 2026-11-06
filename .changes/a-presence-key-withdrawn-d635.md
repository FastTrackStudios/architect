---
"architect" = "patch"
"crdt" = "patch"
---

A presence key withdrawn right after it was written (a pointer leaving the window) no longer lingers on the other side until it times out; architect::platform re-exports the wall clock (SystemTime, UNIX_EPOCH)
