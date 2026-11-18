---
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
"crdt" = "minor"
"vault-collab" = "minor"
---

A vault file of a kind an extension declares ([[doc_kinds]], now with an optional schema) is a document the node keeps in the file itself and syncs between devices and through the org's node like a note: crdt::SharedDocKinds, DocFile::peek, VaultCollab::document_of/register_document, the node's ExtensionKinds and Running::document.
