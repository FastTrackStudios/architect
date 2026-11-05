---
"vault-collab" = "minor"
"vault-sync-client" = "minor"
---

The vault's collaboration and sync moves into architect from Task (#333): `vault-collab` serves every vault file as a co-edited Loro document on `crdt::DocRegistry`, writing docs back to their files after a debounce, and `vault-sync-client` pulls and pushes a device's vault folder over `VaultSync`.
