---
"crdt" = "patch"
---

A save of a CrdtDoc over a document file holds every commit: BranchPersistence::compact writes the offered snapshot before folding, so a file reopened before the doc's writer task ran no longer loses the newest edits
