---
"vault" = "minor"
"vault-proto" = "minor"
"vault-live" = "minor"
"vault-obsidian" = "minor"
"vault-entity" = "minor"
"vault-doc" = "minor"
"architect-lab-ui" = "none"
---

The vault moves into architect from Task (#333): a folder of markdown as notes with properties (frontmatter), links and tags, its page index (constant-time lookup, one parse per change, incremental, a parse failure costs one page, writes lock what they touch), `.base` views, typed property schemas and notes as Loro documents (`vault-proto`, `vault-live`, `vault-obsidian`, `vault-entity`, `vault-doc`, the `vault` facade), with its spec (`features/vault/spec/vault.md`, tracked by tracey). The `vault` crate's `actions` feature adds `VAULT_*` actions over an open folder (`OpenVault`), and the lab opens a sample vault (`apps/lab/vault`) with them. What was Task's alone became hooks: `PropertySchemaRegistry::with_builtins` has `node`, `person` and `daily` only (an app inserts its kinds), `bases::register_formula_fn` takes the place of the built-in `transpose_key`, `shelf_boundary::set_shelf_markers` takes the place of `project.md`, and the private-directory names are `vault_proto::consts`.
