---
"architect-documents" = "minor"
"architect-extensions" = "minor"
---

Added a document's undo for views that are not text editors: the host calls `undo_document` / `redo_document` and the actions `DOC_UNDO` / `DOC_REDO {path?}` (without `path`, the active pane's document; ⌘Z / ⇧⌘Z in every view an opener shows a file in). New `Documents::undo`/`redo` (host trait and the `architect_documents` rpc service), `DocumentStore::undo`/`redo`, `Undone` and `architect_documents::Journal`; `CrdtDocuments` walks its `History`, `VaultDocuments` its journal of edits made through it, merged three ways so later edits stay
