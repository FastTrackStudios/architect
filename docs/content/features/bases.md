+++
title = "Bases"
description = "architect-bases: a vault's .base files as table, cards and list views with filters, sort, group-by, properties and formulas; view types extensions add; the BASES_* actions; a note's ```base fence."
weight = 55
+++

## What it is and when to use it

A base is Obsidian's `.base` file: a query over a vault's notes (filters,
formulas) and one or more views of the result (a table, cards, a list),
each with its own columns, sort, grouping and filters. architect draws a
base as a workspace view, as Obsidian does, and lets extensions add view
types to it, as Obsidian plugins add Bases views (#346). Use it wherever
an app shows notes as rows: a project's tasks, a set list, a reading list.

The parser and the evaluator are the vault's (`features/vault/vault-live/src/bases.rs`,
moved from Task with the vault in #333; `skills/architect/references/vault.md`); this crate is
the model over them, the actions and the views. The design of the view-type
contribution point is in [Extensions](@/architecture/extensions.md), "Base
view types".

## A minimal example

```rust,ignore
// The actions, over an open vault; `types` says what extensions add.
let types = architect_bases::model::ViewTypes::new();
architect_bases::actions::register(&list, &open_vault, &types)?;
types.set_source(move || architect_bases::ui::contributed_types(&host));

// The view a base opens in, and Bases' fence embed, linked in.
let views = Views::new().view(architect_bases::ui::view());
let extensions = extensions.with_linked(&architect_bases::ui::Bases)?;
```

A `.base` then opens with `BASES_OPEN {"path": "Bases/Tasks.base"}`, or
by opening a pane of view `base` whose state is `{"path": …}`.

An extension adds a view type in its manifest and draws it in its code:

```toml
[[base_views]]
id = "board"
name = "Board"
icon = "kanban"
options = [{ key = "columnBy", name = "Columns", kind = "property" }]
```

```rust,ignore
fn build(&self, ext: &mut Builder) {
    ext.base_view("board", BoardView); // reads `linked::use_base_view()`
}
```

## Code tour

- `features/bases/src/model.rs`: `ViewType`, `builtin_types`, `ViewTypes`
  (`set_source`, `all`), `view_json`, `base_json`, `columns`, `editable`,
  and the edits `set_filters`, `set_sort`, `set_group`, `set_order`,
  `set_formula`, `add_view`, `set_option`. A base with no views reads as
  `default_view`.
- `features/bases/src/actions.rs`: `register` puts the `BASES_*` actions on
  a list (table below); `pane_id`, `VIEW_KIND`. Every edit saves the file
  through `features/vault/vault-obsidian/src/mutate.rs` `save_base`, under
  `features/vault/vault/src/actions.rs` `OpenVault::update`.
- `features/bases/src/ui.rs`: `view`, `PaneState`, `BaseView` (the view
  picker, the toolbar's Filter, Sort, Group, Properties and Formulas panels,
  the `+` that adds a view of any registered type), Bases' own table (a
  header press sorts; a note's property is edited in its cell, through
  `VAULT_SET_PROPERTY`), cards and list; `contributed_types`; `Bases`, the
  linked extension (`features/bases/extension.toml`) that claims ```` ```base ````
  fences — the fence's YAML, or one line naming a `.base` file.
- A contributed type is drawn by `features/extensions/src/embeds.rs`
  `BaseViewEmbed` (from `Embeds::base_view`), given a
  `features/extensions/src/contrib.rs` `BaseViewInput`.
- Grouping: `features/vault/vault-live/src/grouping.rs` `group_by` (with
  `label` and `Order`) is what a view's `groupBy` runs on, and what the
  navigator's list pane (`features/navigator`, #335) groups notes with.
- Filters as text: `features/vault/vault-live/src/bases.rs`
  `filter_clauses`, `filter_of_clauses`, `Conjunction`.

| action | does |
|---|---|
| `BASES_LIST` | the vault's bases, each view's name and type |
| `BASES_OPEN {path, view?}` | opens one as a workspace view (`WORKSPACE_OPEN`) |
| `BASES_READ {path}` | its filters, formulas, properties and views |
| `BASES_ROWS {path \| source, view?}` | a view's spec, columns, grouped rows and every column it could show |
| `BASES_SET_FILTERS {path, view?, clauses, join?, scope?}` | a view's (or the base's) filters |
| `BASES_SET_SORT {path, view?, sort}` | `["status desc", "file.name"]` |
| `BASES_SET_GROUP {path, view?, property?}` | group by a column, or not |
| `BASES_SET_PROPERTIES {path, view?, order}` | the columns shown, in order |
| `BASES_SET_FORMULA {path, name, expression}` | add, change or (empty) remove a formula |
| `BASES_SET_OPTION {path, view?, key, value}` | one of a view type's options |
| `BASES_ADD_VIEW {path, type, name?}` | a new view of any registered type |
| `BASES_VIEW_TYPES` | Bases' own types and those extensions add |

## Tests to read

- `features/bases/tests/actions.rs`: list, rows, a fence's YAML, every edit
  written to the file and read back, a cell's edit, the view types.
- `features/bases/tests/extension.rs`: `Bases` claims the fence, and lets it
  go when turned off.
- `apps/lab/board/tests/both_ways.rs`: the Board's `board` type, linked in
  and as a process, withdrawn when it is disabled.
- `features/extensions/src/contrib.rs` (unit tests): a type is drawn by a
  view, a page or its code, never takes a built-in's id.

## Status

Done in architect: the three built-in views, the toolbar, editable cells,
the fence embed, the contribution point, the actions; the lab opens
the base in `apps/lab/vault` ("Notes by status") from its Files pane and has a
note with a fence ("A base in a note"); the Board adds `board`.

Not yet: Obsidian's structured filter builder (filters are edited as
expressions), a property's display name and format in the table, a cards
view's image property, dragging a column to reorder (arrows in the
Properties panel do it), a view's own `limit` in the toolbar. Task's
calendar, kanban, gantt, heatmap and graph views become extensions that
add base view types (#350).
