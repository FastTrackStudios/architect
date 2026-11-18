---
"crdt" = "minor"
---

Read-only document follow: a caller admitted to read but not write follows a doc live (an update up ends its session with SyncError::Forbidden; its presence is dropped, now stays open), DocRegistry::with_write_authorizer, SyncedDoc::read_only, read links with Shares::share_with(.., Access::Read) (#847)
