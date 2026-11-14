// SHOWCASE_RECIPES_SCALE: a recipe scaled to `servings`, written as a
// shopping-list note in Shopping/. The recipe is `path`, or picked from
// Recipes/ when left out; `servings` is asked when left out. Answers the
// new note's path and its lines.
import { actions, defineAction, notify, ui } from "architect";
import { formatIngredient, recipeOf, scale, shoppingList } from "../pages/lib/recipe";
import { recipes } from "./list";

type Args = { path?: string; servings?: number; folder?: string } | null;
type Answer = { path: string; servings: number; items: string[] } | null;
type Note = { path: string; title?: string; properties?: Record<string, unknown> };

export default defineAction<Args, Answer>(async (args, ctx) => {
  let path = args?.path;
  if (!path) {
    const { recipes: found } = await recipes(args?.folder ?? "Recipes");
    if (found.length === 0) throw new Error("no recipes in Recipes/: a note with servings and ingredients in its frontmatter");
    const picked = await ui.pickOne(
      "Scale which recipe?",
      found.map((r) => ({ id: r.path, label: r.title, detail: `${r.servings} servings, ${r.ingredients.length} ingredients` })),
    );
    if (picked == null) return null;
    path = picked;
  }
  const recipe = recipeOf(await actions.run<Note>("VAULT_READ", { path }));
  let servings = args?.servings;
  if (servings == null) {
    const said = await ui.input(`Servings of ${recipe.title}`, {
      message: `The recipe makes ${recipe.servings}.`,
      default: String(recipe.servings * 2),
    });
    if (said == null) return null;
    servings = Number(said);
  }
  const items = scale(recipe, servings).map(formatIngredient);
  const properties = JSON.stringify({
    recipe: `[[${recipe.title}]]`,
    servings,
    scaled_from: recipe.servings,
    tags: ["shopping"],
  });
  const body = shoppingList(recipe, servings);
  // A new note each time: `Pancakes for 6.md`, then `Pancakes for 6 (2).md`.
  for (let n = 1; ; n++) {
    const made = `Shopping/${recipe.title} for ${servings}${n > 1 ? ` (${n})` : ""}.md`;
    try {
      await actions.run("VAULT_CREATE", { path: made, body, properties });
    } catch (e) {
      if (n < 20 && /exists|taken/i.test(String((e as Error).message))) continue;
      throw e;
    }
    console.log(`${ctx.source}: ${recipe.path} for ${servings} -> ${made}`);
    // A toast, as the extension (the app's; a page's notify in a browser).
    notify(`Shopping list: ${made}`, "success");
    return { path: made, servings, items };
  }
});
