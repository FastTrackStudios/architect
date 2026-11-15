+++
title = "Commands"
description = "The actions of architect's first-party extensions that this page covers: each one's id, effect and arguments, as ⌘K, the CLI and MCP run them."
weight = 40
+++

Every action here is on the app's Action List: ⌘K finds it by name,
`architect-desktop actions run <ID> '<json>'` runs it, and the app's MCP
server offers it as a tool. `actions describe <ID>` prints its schema from
the running app. Each extension's page says how it is built.

## Cookbook

[Cookbook](@/extensions/cookbook.md), `extensions/cookbook/extension.toml`.

| id | effect | arguments | answer |
|---|---|---|---|
| `COOKBOOK_LIST` | read-only | `text?`, `tag?`, `course?`, `ingredient?`, `limit?` | `[{path, name, description?, course?, cuisine?, servings?, prepMinutes?, cookMinutes?, tags, source?}]` by name |
| `COOKBOOK_GET` | read-only | `path`, `servings?` | the recipe: `name`, `servings`, `ingredients` (`name, qty, unit, qtyDisplay, note, optional, isRecipeRef`), `cookSteps` (`text, section, timers [{name, seconds, display}], notes`), `cookware`, `source` |
| `COOKBOOK_SAVE` | mutating | `source`, `path?`, `name?`, `create?` | the recipe parsed back |
| `COOKBOOK_RENAME` | mutating | `path`, `to` | the recipe at its new path |
| `COOKBOOK_DELETE` | destructive | `path` | `{deleted}` |
| `COOKBOOK_INGREDIENTS` | read-only | `portions: [{path, servings?}]` | `[{name, qty?, unit, display?, recipe, optional}]` |
| `COOKBOOK_IMPORT` | mutating | `url?` or `text?`, `path?`, `save?` | `{path, source, recipe, saved, review, unplaced?}` |
| `COOKBOOK_TIDY` | mutating | `folder?` | `{changed: [path]}` |
| `COOKBOOK_BASE` | mutating | none | `{path, written, opened}` |
| `COOKBOOK_OPEN` | read-only | `path?`, `view?` (`cookbook`, `cook`, `edit`), `servings?` | `{opened}` |
| `COOKBOOK_TIMER` | read-only | `seconds` (1–86400), `label?` | `{id, seconds, notify: "os" \| "app"}` |
