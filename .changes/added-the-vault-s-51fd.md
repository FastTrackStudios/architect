---
"architect-extensions" = "minor"
"vault" = "minor"
"vault-proto" = "minor"
"vault-query" = "minor"
---

Added the vault's query engine (#710, #272): vault_query::QueryEngine runs a base's view (filters, formulas, sort, group-by) over the vault index, never over note bodies, keeping no rows between queries; it is the one implementation of vault_proto::query::VaultQuery, served in process with architect::serve_local over vault_query::Built (a vault you hold) or engine::Replica (a synced copy). QueryEngine::run_text writes an answer once, with the index's revision and its hash; asked again with if_changed on an index that has not moved, it costs nothing. Extensions query with the host call query_vault (HostRequest::QueryVault, needing reads = ["documents"]; QUERY_LIMIT rows by default, QUERY_LIMIT_MAX at most) through the client the app gives with its documents (Host::set_vault, VaultScope: a guest's host gets none; set_documents alone drops it). Changed: BASES_ROWS answers through the same client (architect_bases::actions::register takes a VaultQueryClient; local_queries serves one), and every BASES_* action needs reads or writes = ["documents"] from an extension, as VAULT_* does. BaseRow::frontmatter is an Arc of shared keys (BaseRow::from_shared) and a row has its note's mtime (file.mtime); bases::view_table_at_most stops at a row limit; a view's sort evaluates each row's keys once.
