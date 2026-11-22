// A recipe's ingredients, read from its frontmatter and scaled. Shared by
// the actions (`../../actions/*.ts`) and the page (`../scaler.tsx`): plain
// TypeScript, no host calls.

/** One ingredient line: `1 1/2 cup flour`, `2 eggs`, `a pinch of salt`. */
export interface Ingredient {
  /** How much, when the line starts with a number. */
  qty?: number;
  /** Its unit, when it names one (`cup`, `g`, `tbsp`…). */
  unit?: string;
  /** What it is. */
  name: string;
}

/** A recipe note, as its frontmatter says it. */
export interface Recipe {
  path: string;
  title: string;
  servings: number;
  ingredients: Ingredient[];
}

const UNITS = new Set([
  "cup", "cups", "tbsp", "tablespoon", "tablespoons", "tsp", "teaspoon", "teaspoons",
  "g", "kg", "mg", "ml", "l", "oz", "lb", "lbs", "pinch", "pinches", "clove", "cloves",
  "can", "cans", "slice", "slices", "stick", "sticks", "handful", "handfuls",
]);

const UNICODE: Record<string, number> = { "¼": 0.25, "½": 0.5, "¾": 0.75, "⅓": 1 / 3, "⅔": 2 / 3, "⅛": 0.125 };

/** `2`, `1.5`, `1/2`, `1 1/2`, `½`, `1½`: the number, or undefined. */
export function parseQty(text: string): number | undefined {
  const t = text.trim();
  const uni = t.match(/^(\d+)?\s*([¼½¾⅓⅔⅛])$/);
  if (uni) return (uni[1] ? Number(uni[1]) : 0) + UNICODE[uni[2]];
  const mixed = t.match(/^(\d+)\s+(\d+)\/(\d+)$/);
  if (mixed) return Number(mixed[1]) + Number(mixed[2]) / Number(mixed[3]);
  const frac = t.match(/^(\d+)\/(\d+)$/);
  if (frac) return Number(frac[2]) === 0 ? undefined : Number(frac[1]) / Number(frac[2]);
  return /^\d+(\.\d+)?$/.test(t) ? Number(t) : undefined;
}

/** One line of a recipe's `ingredients`. */
export function parseIngredient(line: string): Ingredient {
  const text = line.trim();
  const m = text.match(/^(\d+\s+\d+\/\d+|\d+\/\d+|\d+(?:\.\d+)?\s*[¼½¾⅓⅔⅛]?|[¼½¾⅓⅔⅛])\s+(.*)$/);
  if (!m) return { name: text };
  const qty = parseQty(m[1]);
  if (qty === undefined) return { name: text };
  const [first, ...rest] = m[2].split(/\s+/);
  if (rest.length > 0 && UNITS.has(first.toLowerCase())) return { qty, unit: first, name: rest.join(" ") };
  return { qty, name: m[2] };
}

/** An ingredient from frontmatter: a line of text, or `{name, qty, unit}`. */
function ingredientOf(v: unknown): Ingredient | undefined {
  if (typeof v === "string") return v.trim() ? parseIngredient(v) : undefined;
  if (v && typeof v === "object" && typeof (v as { name?: unknown }).name === "string") {
    const o = v as { name: string; qty?: unknown; unit?: unknown };
    const qty = typeof o.qty === "number" ? o.qty : typeof o.qty === "string" ? parseQty(o.qty) : undefined;
    return { name: o.name, qty, unit: typeof o.unit === "string" ? o.unit : undefined };
  }
  return undefined;
}

/** A recipe from a note's `VAULT_READ` answer: its `servings` and `ingredients`. */
export function recipeOf(note: { path: string; title?: string; properties?: Record<string, unknown> }): Recipe {
  const p = note.properties ?? {};
  const servings = Number(p.servings);
  const list = Array.isArray(p.ingredients) ? p.ingredients : [];
  if (!(servings > 0)) throw new Error(`${note.path}: its frontmatter has no servings (servings: 4)`);
  if (list.length === 0) throw new Error(`${note.path}: its frontmatter has no ingredients (a list)`);
  const title = note.title ?? note.path.replace(/^.*\//, "").replace(/\.md$/, "");
  return {
    path: note.path,
    title,
    servings,
    ingredients: list.map(ingredientOf).filter((i): i is Ingredient => i !== undefined),
  };
}

/** Each ingredient for `servings` instead of the recipe's own. */
export function scale(recipe: Recipe, servings: number): Ingredient[] {
  if (!(servings > 0)) throw new Error(`servings must be more than 0, not ${servings}`);
  const factor = servings / recipe.servings;
  return recipe.ingredients.map((i) => (i.qty === undefined ? i : { ...i, qty: i.qty * factor }));
}

const NICE: Array<[number, string]> = [[0.25, "1/4"], [1 / 3, "1/3"], [0.5, "1/2"], [2 / 3, "2/3"], [0.75, "3/4"]];

/** `1.5` as `1 1/2`, `0.3333` as `1/3`, `2.4` as `2.4`. */
export function formatQty(n: number): string {
  const whole = Math.floor(n + 1e-9);
  const part = n - whole;
  if (part < 0.02) return String(whole);
  for (const [v, s] of NICE) if (Math.abs(part - v) < 0.02) return whole > 0 ? `${whole} ${s}` : s;
  return String(Math.round(n * 100) / 100);
}

/** An ingredient as a line: `1 1/2 cup flour`. */
export function formatIngredient(i: Ingredient): string {
  if (i.qty === undefined) return i.name;
  return [formatQty(i.qty), i.unit, i.name].filter(Boolean).join(" ");
}

/** The shopping list's note body (its title is the note's name). */
export function shoppingList(recipe: Recipe, servings: number): string {
  const lines = scale(recipe, servings).map((i) => `- [ ] ${formatIngredient(i)}`);
  return `Scaled from ${recipe.servings} servings of [[${recipe.title}]].\n\n${lines.join("\n")}\n`;
}
