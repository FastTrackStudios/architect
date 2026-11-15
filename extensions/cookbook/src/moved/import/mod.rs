//! Importing recipes: Task's `recipe-import`
//! (`task:features/mealplan/recipe-import`), without its network and
//! its LLM: a page's schema.org Recipe ([`extract`], from JSON-LD) or a
//! recipe's text, made into Cooklang by Task's deterministic synthesizer
//! ([`heuristic`], every ingredient kept, marked `import: needs-review`),
//! and Task's own `.cook` files ([`formats`]).

pub mod coverage;
pub mod extract;
pub mod formats;
pub mod heuristic;
pub mod normalized;
pub mod validate;

pub use heuristic::synthesize_heuristic;
pub use normalized::NormalizedRecipe;
