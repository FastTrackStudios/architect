---
"architect-app" = "minor"
"architect-desktop" = "minor"
---

Added live documents in the app: the node gives the window's extension host its vaults' documents of a kind (`Running::kind_documents`, `node::vault_path`; `architect_app::on_extension_host`), and architect-desktop turns on `architect-extensions/doc-sync`, so a linked or process extension opens a vault `.session` of its kind with `Handle::document`
