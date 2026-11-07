---
"architect-desktop" = "minor"
"architect-files" = "minor"
"crdt" = "minor"
"vault-collab" = "minor"
---

Synced roots (architect-files feature sync): projects and asset libraries held in place by the headless node, a manifest per root on the peer lanes, bytes over BlobTransfer, pins per machine, versions, forks kept as conflict copies, checkpoints at save points. The node takes files_dir/files_hold/files_secs; crdt::p2p::node_router_with adds services to the peer lane; VaultCollab::with_plain_docs opens non-vault documents.
