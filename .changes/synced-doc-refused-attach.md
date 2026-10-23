---
"crdt" = "patch"
---

`SyncedDoc::run` reports a refused attach as the attach error every time. The refusal arrives as the call's response, which also closes the down channel, and `run` raced the two: about a third of the time it read the closed channel as a clean end and returned `Ok(())`. It now polls the call first (`biased`), so a replica with edits already waiting no longer takes the outbox branch before the request that binds the up channel goes out. When either channel closes, it returns the call's result.
