---
"architect-web" = "minor"
"vault-query" = "minor"
---

Added the vault's query service in the browser (#495): the web app runs `vault_query::QueryEngine` in the page (`architect::serve_local`) over the vault index, synced from the node's `VaultIndexSync` when signed in (a `Replica`, kept by its deltas, a snapshot again after a miss) or built in the page from the demo vault, and gives it to the extension host with the documents (`Host::set_vault`); `Replica::reset` takes a snapshot in place of what it had.
