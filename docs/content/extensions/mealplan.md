+++
title = "Mealplan"
description = "How the Mealplan extension is built: the week's meals, the pantry, substitutions and grocery lists as vault notes, the automatic grocery list from the Cookbook's ingredients, and the architect.mealplan service; it reaches the Cookbook, Day Plans and the Calendar only as services."
weight = 25
+++

Mealplan (`extensions/mealplan`, crate `architect-mealplan`, id
`mealplan`) plans the week's meals from the Cookbook and makes the
grocery list. It brings Task's meal planning
(`task:features/mealplan/mealplan`, `task:features/mealplan/pantry`) to
architect, with its data as notes instead of rows: a plan, a pantry item
and a grocery list sync, open as notes, show in Bases and travel by
share link like any other.

The recipes are the Cookbook's (`extensions/cookbook`): Mealplan
`[requires] "architect.cookbook"` and calls its `list` and `ingredients`
through the host (`Handle::service`), never its crate. Day Plans and the
Calendar are `[uses]`: when they are installed, a planned meal can go on
its day.

## 1. A week's plan is a note

`Meal plans/<YYYY-Www>.md` (ISO weeks; the folder is the setting
`mealplan.plans_folder`) lists the week's meals in its frontmatter:

```markdown
---
type: meal-plan
week: 2026-W47
meals:
  - date: 2026-11-16
    meal: breakfast
    recipe: Cookbook/Pancakes.cook
    servings: 4
  - date: 2026-11-18
    meal: dinner
    recipe: Cookbook/Chicken and Waffles.cook
    servings: 4
    note: Sam's birthday
    event: Calendar/Dinner Chicken and Waffles.md
---
# Meals, 2026-W47
```

`meal` is `breakfast`, `lunch`, `dinner` or `snack` (`supper` and
`brunch` read as dinner and breakfast); `recipe` is a `.cook` file's
vault path; `servings` is for how many (`mealplan.servings`, 2, when it
is not given); `event` is the Calendar event the meal became. One note a
week, not one a day: a week is what a person plans and shops for, and
moving a meal within it is one edit to one note. A write keeps the
frontmatter keys it does not own and the body
(`extensions/mealplan/src/model.rs` `write_plan`).

## 2. The pantry and the substitutions

Each item is a note, `Pantry/<Item>.md` (`mealplan.pantry_folder`):

```markdown
---
type: pantry
qty: 1
unit: kg
aisle: Baking
staple: true
min: 0.5
expires: 2026-12-01
aliases:
  - plain flour
---
# Flour
```

A recipe's ingredient is the item of that name or alias, case and a
plural's `s` ignored (`model::fold`, `model::is_item`). A `staple` with
no `qty` is always there; under its `min` it goes on the grocery list.
Stock past `expires` counts as none.

The substitutions are one note, `Pantry/Substitutions.md`: `ratio` of
`to` for one of `from`. One that is `always` is bought instead (oat milk
for milk); otherwise it is used when the pantry has enough of `to`.

```markdown
---
type: substitutions
substitutions:
  - from: butter
    to: olive oil
    ratio: 0.75
  - from: milk
    to: oat milk
    ratio: 1
    always: true
---
```

## 3. The grocery list builds itself

`MEALPLAN_GROCERIES` (`extensions/mealplan/src/groceries.rs` `build`, Task's
`fulfillment.rs` and `shopping.rs` over notes):

1. the range's planned meals (a week by default);
2. the Cookbook's `ingredients` for them, each recipe scaled to its
   meal's servings and the recipes it calls for (`[[Hot Honey]]{3}`)
   cooked out into their own ingredients; optional ones left off;
3. added up by ingredient with units converted by Cooklang's own units
   (`extensions/mealplan/src/units.rs` `convert`: 500 g and 0.5 kg are one row;
   grams and cloves stay two);
4. less the pantry, each item in its own unit, used up across rows;
5. substitutions applied;
6. staples under their `min` added (`restock`);
7. grouped by aisle: the item's `aisle`, else a guess from its name
   (`guess_aisle`), else Other; in the order a shop is walked.

It is written as a checklist note, `Groceries/<week>.md`
(`mealplan.groceries_folder`):

```markdown
---
type: groceries
week: 2026-W47
from: 2026-11-16
to: 2026-11-23
items:
  - name: eggs
    qty: 4
    line: 4 eggs
substituted:
  - honey → golden syrup
---
# Groceries, 2026-W47

## Dairy & eggs
- [ ] 4 eggs

## Baking
- [x] 650 g flour
- [ ] baking paper
```

Building it again keeps what a person did with it (`render`): an item
ticked stays ticked, an item ticked that is no longer needed stays (it
was bought), and a line written by hand stays under its heading. The
frontmatter's `items` is how a built line is told from a hand-written
one. `MEALPLAN_CHECK` ticks a line; with `bought` it also adds the
amount to the pantry (Task's `mark-purchased`; without it, Task's
`mark-have`, which never touches stock).

## 4. Actions and the service

One `#[architect::rpc]` service, `Meals`
(`extensions/mealplan/src/service.rs`), with one implementation,
`extensions/mealplan/src/backend.rs` `Backend`, over a port that reaches the
vault and the other extensions only through the host
(`extensions/mealplan/src/files.rs` `Files`; `Files::Memory` for the model's
tests). It is served in process with `architect::serve_local`
(`extensions/mealplan/src/client.rs`), as the Cookbook's is. Method `m` is
action `MEALPLAN_<M>` and `architect.mealplan`'s method `m`:

| action | what it does |
|---|---|
| `MEALPLAN_WEEK` | a week's plan: its note, each day and its meals |
| `MEALPLAN_MEALS` | the meals of `from`..`to` |
| `MEALPLAN_PLAN` | plans a recipe (a path, or a name the Cookbook finds) for a meal of a day |
| `MEALPLAN_UNPLAN` | takes a meal off, and its event |
| `MEALPLAN_MOVE_MEAL` | moves a meal to another day or meal, across weeks; its event follows |
| `MEALPLAN_SCHEDULE` | Task's `meal schedule`: gives the day's Day Plans meal block the recipe (`DAYPLAN_ASSIGN`, kind `recipe`), or makes it a Calendar event (`CALENDAR_CREATE`, an `external_ref` so it is made once) |
| `MEALPLAN_PANTRY` | the pantry, narrowed by text, aisle, low or expired |
| `MEALPLAN_STOCK` | sets or adds stock (converted to the item's unit), and an item's other fields |
| `MEALPLAN_REMOVE_ITEM` | deletes a pantry item |
| `MEALPLAN_SUBSTITUTIONS`, `MEALPLAN_SUBSTITUTE`, `MEALPLAN_UNSUBSTITUTE` | the substitutions |
| `MEALPLAN_GROCERIES` | builds or refreshes the grocery list |
| `MEALPLAN_GROCERY_LIST` | the grocery list as it is, not built again |
| `MEALPLAN_CHECK` | ticks a grocery item; `bought` stocks the pantry |
| `MEALPLAN_CAN_COOK` | Task's `recipe can-cook`: what the pantry has and lacks for a recipe |
| `MEALPLAN_OPEN` | opens a view: `week`, `pantry` or `groceries` |

Each change is one undo step named for what it did (a meal moved across
weeks is two notes, so two). Task's shopping-list templates and its
low-stock and expired verbs are folded into the list: staples under
their `min` are added to it, and expired stock does not count.

## 5. Its views

Three views (`extensions/mealplan/src/ui.rs`), one component with a tab for
each, so ⌘K's `MEALPLAN_OPEN` and the extension's view list land on any
of them:

- **Week** (`WeekView`): the days of a week (‹ › and the week's name,
  which goes back to this week), each with its breakfast, lunch, snack
  and dinner. A slot's `+` opens the recipe picker: the Cookbook's
  `list` through the host, a search, the servings. A meal opens its
  recipe; ⇄ picks it up and "Move here" on any slot puts it down (no
  drag, so a phone works the same); ✕ unplans it. A desk shows seven
  columns and the picker as a side panel; a phone (narrower than 760
  px, measured as the Calendar measures its views) a list of days with
  only the slots that have meals, and dinner, and the picker as a
  bottom sheet.
- **Pantry** (`PantryView`): the items with their stock, `−`/`+` (100 g
  or ml, else 1), ✕, low and expired flagged; a row to add one; the
  substitutions, added and removed.
- **Groceries** (`GroceriesView`): the week's list by aisle; a tap ticks
  a line (Task's mark-have), Bought ticks it and stocks the pantry;
  Build makes it again.

Each view holds only the `Meals` client. What it shows is read in a
future that peeks (`use_read`): when the week shown or a change it made
moves, and every five seconds, and a signal is written only when what
was read differs, so a view never reads for ever. No keyed row makes
signals, and there is no `<select>` (Blitz draws one blank).

## Tests to read

- `extensions/mealplan/src/groceries.rs`: Task's `units_and_scaling` (one
  row for 500 g and 0.5 kg, none for grams and cloves) and
  `shopping_run` (ticks and hand-written lines kept), the pantry,
  expiry, staples and substitutions.
- `extensions/mealplan/src/backend.rs`: Task's `end_to_end` over notes in
  memory: a week planned, the pantry stocked, the list built and bought.
- `extensions/mealplan/tests/host.rs`: linked into a host beside the
  Cookbook over the app's documents (Task's `nested_recipes`: a recipe
  that calls for another, cooked out and scaled), the note's text
  checked.
