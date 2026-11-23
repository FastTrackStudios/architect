---
"crdt" = "minor"
"vault-collab" = "minor"
---

A vault share link's guest follows a live document its link serves (a .session a listed note links to), read-only, on /s/<code>/_guest/live: every edit live, its writes and presence refused, ended when the link is revoked (#847 part 3). DocRegistry::sync_read_only and presence_read_only; VaultCollab::document_route.
