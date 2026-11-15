// The Recipe scaler's tab: the recipes in Recipes/, servings to scale to,
// the scaled ingredients, and a button that writes the shopping list. A
// TSX page on QuickJS in the app (a Web Worker in a browser); the work is
// its TypeScript actions' (`../actions/*.ts`), the scaling shared with
// them (`./lib/recipe.ts`).
import { h, useData, useState, actions } from "architect";
import { formatIngredient, scale, type Recipe } from "./lib/recipe";

type Listed = { recipes: Recipe[]; skipped: string[] };
type Made = { path: string; servings: number; items: string[] } | null;

const button = {
  padding: "4px 10px",
  borderRadius: 6,
  border: "1px solid $border",
  background: "$secondary",
  color: "$secondary-foreground",
};
const primary = { ...button, background: "$primary", color: "$primary-foreground", border: "1px solid $primary" };

export default function Scaler() {
  const list = useData<Listed>("SHOWCASE_RECIPES_LIST", {});
  const [picked, setPicked] = useState<string | null>(null);
  const [servings, setServings] = useState<number | null>(null);
  const [made, setMade] = useState<string>("");
  const [error, setError] = useState<string>("");
  const recipes = list.value?.recipes ?? [];
  const recipe = recipes.find((r) => r.path === picked) ?? recipes[0];
  const want = servings ?? (recipe ? recipe.servings * 2 : 0);
  const write = async () => {
    if (!recipe) return;
    setError("");
    try {
      const out = await actions.run<Made>("SHOWCASE_RECIPES_SCALE", { path: recipe.path, servings: want });
      if (out) setMade(out.path);
    } catch (e) {
      setError(String((e as Error).message));
    }
  };
  return (
    <div id="scaler" style={{ display: "flex", flexDirection: "column", gap: 12, padding: 20, maxWidth: 640 }}>
      <div style={{ fontSize: 18, fontWeight: 700 }}>Recipe scaler</div>
      <div style={{ color: "$muted-foreground" }}>
        Recipes in Recipes/, scaled from their frontmatter. Written in TypeScript: no Rust.
      </div>
      {list.error ? <div style={{ color: "$destructive" }}>{list.error}</div> : null}
      {recipes.length === 0 && !list.loading ? <div style={{ color: "$muted-foreground" }}>No recipes yet.</div> : null}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        {recipes.map((r) => (
          <button
            key={r.path}
            class="recipe"
            style={r === recipe ? primary : button}
            onClick={() => {
              setPicked(r.path);
              setServings(null);
              setMade("");
            }}
          >
            {r.title}
          </button>
        ))}
      </div>
      {recipe ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: "12px 14px", borderRadius: 8, border: "1px solid $border", background: "$card" }}>
          <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
            <span style={{ fontWeight: 600 }}>{recipe.title}</span>
            <span style={{ color: "$muted-foreground" }}>makes {recipe.servings}; for</span>
            <button style={button} onClick={() => setServings(Math.max(1, want - 1))}>−</button>
            <span id="servings" style={{ minWidth: 24, textAlign: "center", fontWeight: 600 }}>{want}</span>
            <button style={button} onClick={() => setServings(want + 1)}>+</button>
          </div>
          <ul id="items" style={{ margin: 0, paddingLeft: 20 }}>
            {scale(recipe, want).map((i, n) => (
              <li key={String(n)}>{formatIngredient(i)}</li>
            ))}
          </ul>
          <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
            <button id="write" style={primary} onClick={write}>Write shopping list</button>
            {made ? <span id="made" style={{ color: "$muted-foreground" }}>Wrote {made}</span> : null}
            {error ? <span style={{ color: "$destructive" }}>{error}</span> : null}
          </div>
        </div>
      ) : null}
      {(list.value?.skipped ?? []).length > 0 ? (
        <div style={{ color: "$muted-foreground", fontSize: 12 }}>Skipped: {list.value?.skipped.join("; ")}</div>
      ) : null}
    </div>
  );
}
