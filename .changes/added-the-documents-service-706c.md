---
"architect-documents" = "minor"
"architect-extensions" = "minor"
---

Added `architect-documents`, the documents service extensions reach the app's documents through: the `#[architect::rpc] trait Documents` (list, read, create, edit, rename, delete, the vault's `metadata`, an extension's own `data`/`set_data`), its one implementation `DocumentsServer` with the `reads`/`writes = ["documents"]` gate in it (`Gate`, `Access`), the stores under it (`DocumentStore`, `DataStore`) and `MemoryStore`. Consumed through the generated `DocumentsClient` on every transport (`architect::serve_local` on the desk, iOS and the wasm demo; vox on the org lane in a browser). Wasm-clean.

Changed: the extension host reaches the app's documents and each extension's data only through the service's client (`architect_extensions::docs_service`: the app's `Documents` served by `DocumentsServer` with `AsStore`, `FileData` and the host's gate, in-process on a runtime of its own). `Documents` gains `metadata`, which `VaultDocuments` answers from the vault, and `DocError::Unsupported`.
