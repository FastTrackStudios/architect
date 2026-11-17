---
title: R7 TypeScript actions
tags: [showcase, showcase/R]
capability: R7
example: "Recipe scaler"
---
An extension written only in TypeScript (#536): its actions are `.ts` files beside its TSX page, on the same runtime as pages (QuickJS in the app, a Web Worker in a browser), with the whole host service, typed. No Rust, no WASM toolchain.

**Stands in for:** a light meal-planning extension (Health's meal plan), and Templater's and QuickAdd's JS user scripts.

## Try

1. Open ⌘K and run `Recipe scaler: open`.
2. Pick **Pancakes**, press **+** until it says 6, then **Write shopping list**.
3. Or run `Recipe scaler: scale a recipe` from ⌘K: it asks which recipe and how many servings.

## You should see

The scaled ingredients in the tab (Pancakes for 6: 2 1/4 cup flour, 1 1/2 egg…), then a toast and a new note, `Shopping/Pancakes for 6.md`, a checklist of them. The recipes are [[Pancakes]] and [[Chili]] in Recipes/.

**Example:** Recipe scaler (`examples/showcase/recipe-scaler`).

Its status on desktop, web and iOS is in [[Showcase]].
