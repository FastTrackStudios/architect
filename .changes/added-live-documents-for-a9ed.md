---
"architect-extensions" = "minor"
"vault-live" = "minor"
---

Added live documents for extensions (feature `doc-sync`): `HostRequest::LiveDocument` (`live_document {path}`) names a vault file of a kind the extension declares (`[[doc_kinds]]`, `reads = ["documents"]`), and `linked::Handle::document` / `guest::document` return a `live::DocLane` whose `replica()` or `sync(&mut SyncedDoc)` edits and follows the node's copy over the extension's existing host connection (a `DocSync` lane the host relays as `Principal::Extension`, `live::ExtensionDocSync`; edits reach the node only with `writes = ["documents"]`). Apps give it with `Host::set_kind_documents` (`live::KindDocuments`, `live::RegistryDocuments`). `vault_live::write_guard::account_of` now holds an extension to the person it acts for
