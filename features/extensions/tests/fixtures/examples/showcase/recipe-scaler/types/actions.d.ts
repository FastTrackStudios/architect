// Each action's arguments, from its `params` schema in extension.toml, as
// the "architect" module's `ActionArgs`: what `defineAction("ID", …)` is
// typed by. Generated from the manifest (`architect-ext types`, #536).
declare module "architect" {
  interface ActionArgs {
    "SHOWCASE_RECIPES_LIST": {
      folder?: string;
    } | null | undefined;
    "SHOWCASE_RECIPES_SCALE": {
      path?: string;
      servings?: number;
      folder?: string;
    } | null | undefined;
    "SHOWCASE_RECIPES_OPEN": null | undefined;
  }
}
