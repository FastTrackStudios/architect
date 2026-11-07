---
"crdt" = "patch"
---

Changed DocPeerHost::list to list only the documents the caller may read (DocRegistry::may_read), so a peer never learns the names of documents it cannot open.
