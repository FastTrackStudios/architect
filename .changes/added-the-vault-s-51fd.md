---
"architect-app" = "minor"
"architect-extensions" = "minor"
"vault" = "minor"
"vault-proto" = "minor"
"vault-query" = "minor"
---

Added the vault's query engine (#710, #272): vault_query::QueryEngine runs a base's view (filters, formulas, sort, group-by) over the vault index, never over note bodies; it is the one implementation of vault_proto::query::VaultQuery, served in process with architect::serve_local over vault_query::Built (a vault you hold) or engine::Replica (a synced copy). Answers carry the index's revision and a hash; asked again with if_changed and that hash, an unchanged answer costs nothing. Extensions query with the host call query_vault (HostRequest::QueryVault, needing reads = ["documents"]; host.query_vault in Luau) through the client the app gives Host::with_vault_query; the Architect app serves it over its vault. BaseRow::frontmatter is now an Arc shared with the index entry (BaseRow::from_shared); bases::view_table_at_most stops at a row limit; a view's sort evaluates each row's keys once.
