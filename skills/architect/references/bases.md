# Bases: table, cards, list, and view types extensions add

A base is Obsidian's `.base` file: filters and formulas over a vault's notes, and views of the result. `architect-bases` (`features/bases`, MIT OR Apache-2.0) draws one as a workspace view with Obsidian's three built-in view types, and lets extensions add more (#346). The parser and evaluator are the vault's (`references/vault.md`, `features/vault/vault-live/src/bases.rs` `ParsedBase`, `ViewSpec`, `view_table`). Page: `docs/content/features/bases.md`.

## Wiring an app

| piece | where |
|---|---|
| the actions over an open vault | `features/bases/src/actions.rs` `register` (feature `actions`); `ids`, `VIEW_KIND` (`base`), `pane_id` |
| what extensions add, for `BASES_VIEW_TYPES` | `features/bases/src/model.rs` `ViewTypes` (`set_source`), filled with `features/bases/src/ui.rs` `contributed_types` over the extension host |
| the workspace view | `features/bases/src/ui.rs` `view` (feature `ui`): a pane's state is `PaneState` `{path, view?}` |
| one base anywhere | `features/bases/src/ui.rs` `BaseView` (`path` or `source`, `view`, `embedded`); it reads `Arc<ActionList>` from context and redraws on any `BASES_*`/`VAULT_*` write on it |
| a note's ```` ```base ```` fence | `features/bases/src/ui.rs` `Bases` (linked extension `architect.bases`, `features/bases/extension.toml`): `Extensions::with_linked(&Bases)`; the fence is a base's YAML, or one line naming a `.base` |

The lab does all of it: `apps/lab/ui/src/notes.rs` `register`, `view_types`; `apps/lab/ui/src/extensions.rs`; `apps/lab/ui/src/vault.rs` (the Files pane opens `Bases/Notes by status.base`; the note "A base in a note").

## The actions

`BASES_LIST`, `BASES_OPEN {path, view?}`, `BASES_READ {path}`, `BASES_ROWS {path | source, view?}`, `BASES_SET_FILTERS {path, view?, clauses, join?, scope?}` (clauses are expressions: `status != null`), `BASES_SET_SORT {path, view?, sort}` (`["rank desc", "file.name"]`), `BASES_SET_GROUP {path, view?, property?}`, `BASES_SET_PROPERTIES {path, view?, order}`, `BASES_SET_FORMULA {path, name, expression}` (empty removes), `BASES_SET_OPTION {path, view?, key, value}`, `BASES_ADD_VIEW {path, type, name?}`, `BASES_VIEW_TYPES`. Every edit is saved to the file (`features/vault/vault-obsidian/src/mutate.rs` `save_base`); a cell's edit is the note's `VAULT_SET_PROPERTY`. The edits as functions: `features/bases/src/model.rs` `set_filters`, `set_sort`, `set_group`, `set_order`, `set_formula`, `add_view`, `set_option`.

## Adding a view type (an extension)

```toml
[[base_views]]
id = "board"                 # the base's `type:`; table, cards and list are refused
name = "Board"
icon = "kanban"
options = [{ key = "columnBy", name = "Columns", kind = "property" }]
# view = "<one of its [[views]]>" | page = "pages/x.tsx" | neither: its code draws it
```

```rust
ext.base_view("board", BoardView); // in Extension::build; drawn as view `base.board`
// in BoardView: architect_extensions::linked::use_base_view() -> Option<BaseViewInput>
//   .base (path), .view (type, name, order, sort, groupBy, filters, limit, options), .rows ({columns, groups})
```

Symbols: `features/extensions/src/contrib.rs` `BaseViewDecl` (`view_name`), `BaseViewOption`, `BaseViewInput`, `BUILTIN_BASE_VIEWS`, `Contributions::base_views`, `Contributions::base_view`; `features/extensions/src/linked.rs` `Builder::base_view`, `use_base_view`, `BASE_VIEW_CLAIM`; `features/extensions/src/manifest.rs` `Manifest::draws`; `features/extensions/src/embeds.rs` `Embeds::base_view`, `BaseViewEmbed`. Turned off, its type leaves the picker and `BASES_VIEW_TYPES`, and its views say no extension draws them. Example: `apps/lab/board/src/lib.rs` `BaseBoard`; tests `apps/lab/board/tests/both_ways.rs`.

## Grouping, shared

`features/vault/vault-live/src/grouping.rs` `group_by` (`label`, `Order`: first seen, ascending, descending, the empty group last) is what a base's `groupBy` runs on and what the navigator's list pane uses to group notes by a property. Filters read and written as text: `features/vault/vault-live/src/bases.rs` `filter_clauses`, `filter_of_clauses`.

## Gotchas

- Blitz fires no `change` on an `<input>`: the toolbar's and cells' fields commit on Enter or blur (`features/bases/src/ui.rs` `Field`).
- `BASES_OPEN` opens only where `WORKSPACE_OPEN` is on the list (the lab: while the Vault tab shows); elsewhere it answers `opened: false` with the base.
