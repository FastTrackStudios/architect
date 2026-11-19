---
"architect" = "minor"
"architect-desktop" = "minor"
"architect-sites" = "minor"
"vault-obsidian" = "minor"
"crdt" = "minor"
---

Added a share link's files from wherever they are held (#490): the node that answers a link streams a file it does not hold from a holder that has it (`node::holders::Holders::respond`: its own disk, a `302` to a presigned bucket read of that one object for 10 minutes, or a range relayed from a peer with nothing kept; `x-architect-from` says which), and names a vault's attachments held elsewhere in its listings. In `architect`: `blob::Fetcher::relay(hash, offset)` (the first remote's own stream, nothing written to the store) and `blob::S3Store::signed_get(hash, expires)`. A link's attachments are no longer copied into each build: `architect_sites::Options::attachments_in_place` and `Options::elsewhere` (`Site::attachments_in_place`), `vault::obsidian::Vault::with_attachments_elsewhere`. `crdt::file::read_main(bytes)` reads a document file's main branch whatever its kind.
