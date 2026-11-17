+++
title = "Cookbook"
description = "How the Cookbook extension is built: recipes as Cooklang files in the vault, one Recipes service served in process on every platform, scaling and nested recipes, the recipe card a recipe note draws, cook mode with timers, and the architect.cookbook service Mealplan reads."
weight = 24
+++

Cookbook (`extensions/cookbook`, crate `architect-cookbook`, id
`cookbook`) is where the recipes live. It brings Task's cookbook
(`task:features/mealplan/cookbook`) to architect: recipes written in
[Cooklang](https://cooklang.org), read scaled to the people eating,
edited, and cooked step by step. Mealplan plans them onto days and
shops for them (the Mealplan extension).

## 1. A recipe is a Cooklang file

A recipe is a `.cook` file in the vault, `Cookbook/<Name>.cook` (the
folder is the setting `cookbook.folder`): YAML frontmatter, then
Cooklang.

```text
---
title: Chicken and Waffles
servings: 4
course: dinner
prep time: 20 min
cook time: 30 min
tags: [comfort, weekend]
---
= Chicken

Fry @chicken thighs{8} in @oil{500%ml} in a #deep pan{} for ~{12%minutes}.

= Serve

Drizzle [[Hot Honey]]{3} over, with @waffles{4}.
```

- `@name{qty%unit}` is an ingredient, `#name{}` cookware, `~{n%unit}` a
  timer, `= Section` a section, `> …` a note on the step before.
- `[[Hot Honey]]{3}` calls for another recipe, 3 servings of it, in the
  vault's own link syntax, so the recipe is a link to it like any note
  (Cooklang's `@@./sauce{}` works too). `@salt{=1%tsp}` is pinned: it
  never scales.
- The manifest declares the file type (`[[file_types]] extension =
  "cook"`), so a `.cook` file is a note: it syncs, opens, shows in Bases
  (as `type: recipe`, its frontmatter its properties) and travels by
  share link like a markdown note. Its row in the navigator shows its
  `description` and its `image` (`[[file_types]] preview`).
- Open, it is its text, rendered as it is edited (`[[renderers]]`,
  `COOKBOOK_RENDER`, `extensions/cookbook/src/render.rs`): `@flour{250%g}`
  reads as a chip "250 g flour", `#bowl{}` as "bowl", `~{10%minutes}` as
  "10 minutes", `= Batter` as a heading, and the line the caret is on as
  it is written. Over it, its card: servings, Cook and its ingredients.

The model is Task's, moved as it was: `extensions/cookbook/src/moved/model.rs`
`Recipe`, `Ingredient`, `CookStep`, `RecipeTimer`; the projection of the
`cooklang` crate's parse is `extensions/cookbook/src/moved/parse.rs`
`parse_cook`, with Task's tests. A recipe is written only when it reads:
`extensions/cookbook/src/moved/parse.rs` `check` refuses frontmatter that
is not a YAML mapping, since the vault reads it as properties.

## 2. One service, served where the person is

`Recipes` (`extensions/cookbook/src/service.rs`, `#[architect::rpc]`) is
the cookbook's one service:

| method | what it does |
|---|---|
| `list(search)` | recipes by name: `text`, `tag`, `course`, `ingredient`, `limit` |
| `get(path, servings)` | a recipe parsed, scaled when `servings` is given |
| `save(recipe)` | writes Cooklang as given (one undo step), parsed back |
| `rename(path, to)` | moves it; links follow |
| `delete(path)` | to the vault's trash |
| `ingredients(portions)` | what a set of `{path, servings}` needs, nested recipes cooked out, scaled |

Its one implementation is `extensions/cookbook/src/backend.rs` `Backend`,
over the vault as the host gives it (`extensions/cookbook/src/files.rs`
`Files`): the documents (read, create, edit, rename, delete) and the
vault's index. `list` reads the index (`QueryVault` over every `.cook`
file), so it needs no recipe's body: in a browser, which holds only the
index and the notes it opens, the cookbook lists and searches the same
way. Only `ingredient` and `get` open a file.

The extension serves it in process with `architect::serve_local`
(`extensions/cookbook/src/client.rs` `client`), on a runtime of its own
on a desk and a phone and on the page's thread in a browser; its
actions and views hold only the `RecipesClient`. Other extensions reach
it as `architect.cookbook` (`[provides]`), method `m` its action
`COOKBOOK_<M>`, by JSON through the host, never by importing this crate:
Mealplan `[requires]` it.

**Scaling** (`extensions/cookbook/src/scale.rs` `scaled`) multiplies
every quantity Cooklang does not pin by the servings wanted over the
recipe's; a recipe whose servings are not a number (`1 loaf`) scales one
batch. **Nested recipes** are cooked out by `ingredients`: `[[Hot
Honey]]{3}` is 3 servings of Hot Honey, its own ingredients scaled to
that, to any depth (a loop is refused); one the vault does not have is
bought as itself.

## 3. Its views

- **The cookbook** (`extensions/cookbook/src/ui.rs` `CookbookView`,
  `COOKBOOK_OPEN`): search on top, the recipes on the left, the one
  picked on the right. New, and the recipes as a base.
- **The recipe card** (`RecipeCard`): serves with − and +, the
  ingredients scaled, the steps by section with their timers, the
  cookware; Cook and Edit.
- **A recipe note's head** (`RecipeEmbed`): the cookbook claims
  `note_type = ["recipe"]` (`[[embeds]]`), so a recipe opened as a note
  draws its card above the Cooklang, which stays the note's text to
  edit.
- **Cook mode** (`CookView`, `COOKBOOK_OPEN {path, view: "cook"}`):
  Gather (each ingredient ticked off), then one step at a time in large
  type, each timer a countdown with Start. Start also runs
  `COOKBOOK_TIMER`: the device's notification when it is done (on iOS,
  with the app in the background too), else a toast.
- **The editor** (`EditView`, `COOKBOOK_OPEN {path, view: "edit"}`):
  Cooklang on the left, the card it reads as on the right, Save
  (`COOKBOOK_SAVE`).

The layouts wrap: a phone's narrow pane stacks what a desk shows side by
side. The views read in futures, never while drawing, and are drawn
again when the cookbook changes and every few seconds while they show.

## 4. Importing

`COOKBOOK_IMPORT` (`Recipes::import`) makes a recipe from elsewhere into
Cooklang, a draft unless `save` is given:

- **A page** (`url`): fetched through the host (`capabilities.hosts =
  ["*"]`: the consent screen says it may read any site), its schema.org
  Recipe read from the page's JSON-LD (`extensions/cookbook/src/moved/import/extract.rs`
  `extract`: `@graph`s, lists, `HowToSection`s, ISO durations), and
  written as Cooklang by Task's deterministic synthesizer
  (`extensions/cookbook/src/moved/import/heuristic.rs`
  `synthesize_heuristic`): each ingredient line parsed for its amount
  and unit and placed where a step first names it, every one kept (the
  ones it cannot place go in a last step, `unplaced` in the answer), the
  recipe marked `import: needs-review`. A site that refuses programs
  (403, 429, 503) is reported with what to do: save the page and import
  its text. In a browser the page must allow the app's origin (CORS).
- **A text** (`text`): a saved page's HTML (as above), or a `.cook` file
  as it is, Task's `>> key: value` metadata moved into YAML frontmatter
  (`extensions/cookbook/src/moved/import/formats.rs`
  `arrows_to_frontmatter`).
- `COOKBOOK_TIDY` does the same to `.cook` files already in the vault
  (Task's cookbook copied in): each rewritten once, one undo step each,
  so its title, servings and tags show in Bases and search.

Task's LLM synthesis and its collection import (a listing page's
recipes) do not come across: the first needs an API key the cookbook
would hold, the second Task's CLI loop.

## 5. Cookbook.base

`COOKBOOK_BASE` writes `Cookbook/Cookbook.base`
(`extensions/cookbook/templates/Cookbook.base`) when it is not there and
opens it: every `.cook` file (`file.ext == "cook"`) as a table, as cards
and grouped by course.

## 6. Actions

See [Commands](@/reference/commands.md) for each one's arguments.

| action | effect | what it does |
|---|---|---|
| `COOKBOOK_LIST` | read-only | find recipes |
| `COOKBOOK_GET` | read-only | read one, scaled |
| `COOKBOOK_SAVE` | mutating | write Cooklang, one undo step |
| `COOKBOOK_RENAME` | mutating | move a recipe |
| `COOKBOOK_DELETE` | destructive | to the trash |
| `COOKBOOK_INGREDIENTS` | read-only | what portions need |
| `COOKBOOK_IMPORT` | mutating | a page or a text as Cooklang |
| `COOKBOOK_TIDY` | mutating | `>>` metadata as frontmatter |
| `COOKBOOK_BASE` | mutating | open Cookbook.base, writing it first |
| `COOKBOOK_OPEN` | read-only | the cookbook, cook mode or the editor |
| `COOKBOOK_TIMER` | read-only | a kitchen timer's notification |

Each is in ⌘K, `architect-desktop actions run COOKBOOK_LIST '{"tag":"quick"}'`
and the app's MCP tools.

## Tests to read

- `extensions/cookbook/src/backend.rs` (its tests): search by name, tag,
  course and ingredient; nested recipes cooked out and scaled; a loop
  refused; saving, renaming, deleting; index rows as summaries.
- `extensions/cookbook/tests/host.rs`: linked into a host over the app's
  documents, every action through the host and the in-process service.
- `extensions/cookbook/src/moved/parse.rs`: Task's parser tests;
  `extensions/cookbook/src/moved/import/heuristic.rs`: Task's
  synthesizer tests; `extensions/cookbook/src/moved/import/extract.rs`:
  a page's JSON-LD.
