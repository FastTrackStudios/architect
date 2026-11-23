// SHOWCASE_RECIPES_LIST: the recipes in `folder` (Recipes when left out),
// each with its servings and ingredients, read from its frontmatter. A
// note that is not a recipe (no servings or no ingredients) is skipped,
// and said why.
import { actions, defineAction } from "architect";
import { recipeOf, type Recipe } from "../pages/lib/recipe";

type Args = { folder?: string } | null;
type Note = { path: string; title?: string; properties?: Record<string, unknown> };

export async function recipes(folder: string): Promise<{ recipes: Recipe[]; skipped: string[] }> {
  const notes = await actions.run<Array<{ path: string }>>("VAULT_LIST", { folder });
  const out: Recipe[] = [];
  const skipped: string[] = [];
  for (const n of notes.filter((n) => n.path.endsWith(".md"))) {
    const note = await actions.run<Note>("VAULT_READ", { path: n.path });
    try {
      out.push(recipeOf(note));
    } catch (e) {
      skipped.push(String((e as Error).message));
    }
  }
  out.sort((a, b) => a.title.localeCompare(b.title));
  return { recipes: out, skipped };
}

export default defineAction<Args>(async (args) => recipes(args?.folder ?? "Recipes"));
