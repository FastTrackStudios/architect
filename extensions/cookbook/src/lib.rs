//! Cookbook, a first-party architect extension: Task's cookbook
//! (`task:features/mealplan/cookbook`), the architect way.
//!
//! - **Recipes are Cooklang files** in the vault (`Cookbook/Pancakes.cook`):
//!   YAML frontmatter (title, servings, times, tags, source), then
//!   Cooklang (`@flour{200%g}`, `#pan{}`, `~{3%minutes}`). The manifest
//!   declares the `.cook` file type, so they are notes: they sync, open,
//!   show in Bases and travel by share link like any other.
//! - **One service, `Recipes`** ([`service`], `#[architect::rpc]`): its
//!   one implementation [`backend::Backend`] over the vault's documents
//!   and index ([`files`]), served in process on every platform
//!   ([`client`]). The actions ([`actions`]) and the views hold only the
//!   client; other extensions call `architect.cookbook` (`[provides]`).
//! - [`parse`] is Task's Cooklang projection; [`scale`] makes a recipe
//!   for more or fewer people; [`units`] converts with Cooklang's units.
//!
//! The model and the parser are pure and MIT OR Apache-2.0 like the rest
//! of architect. [`Cookbook`] is the extension
//! (`extensions/cookbook/extension.toml`).

// The wire types derive facet (the service's wire) and serde (the JSON
// an extension's actions speak); facet's derive writes `unsafe`, which
// this lint reads as a hand-written invariant serde could skip.
#![allow(clippy::unsafe_derive_deserialize)]

pub mod actions;
pub mod backend;
pub mod client;
pub mod files;
pub mod scale;
pub mod service;

// Task's cookbook (`task:features/mealplan/cookbook`), moved as it was
// under the lints that repository relaxes: byte offsets into Cooklang
// text, counts as `u32`. New code here meets the house's.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::string_slice,
    clippy::indexing_slicing,
    clippy::missing_const_for_fn,
    clippy::derive_partial_eq_without_eq,
    clippy::too_long_first_doc_paragraph,
    clippy::option_if_let_else,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::match_bool,
    clippy::explicit_iter_loop,
    clippy::needless_collect,
    clippy::assigning_clones
)]
mod moved {
    pub mod model;
    pub mod parse;
    pub mod units;
    pub mod wiki;
}
pub use moved::{model, parse, units, wiki};

pub use model::{CookStep, Ingredient, Recipe, RecipeTimer};
pub use service::{
    CookbookError, Needed, Portion, RecipeSummary, Recipes, RecipesClient, Save, Search,
};

use architect_extensions::linked::{Builder, Extension};
use serde_json::json;

/// Its id: its actions are `COOKBOOK_*`.
pub const ID: &str = "cookbook";

/// The service it provides (`[provides]`): method `m` is `COOKBOOK_<M>`.
pub const SERVICE: &str = "architect.cookbook";

/// The base it writes beside the recipes.
pub const BASE_FILE: &str = "Cookbook.base";

/// That base: every Cooklang file, as a table, cards and by course.
pub const BASE: &str = include_str!("../templates/Cookbook.base");

/// The extension: link it in (`Extensions::with_linked`) or run its
/// program.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cookbook;

impl Extension for Cookbook {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        client::connect(ext.handle());
        ext.action("COOKBOOK_LIST", actions::list)
            .action("COOKBOOK_GET", actions::get)
            .action("COOKBOOK_SAVE", actions::save)
            .action("COOKBOOK_RENAME", actions::rename)
            .action("COOKBOOK_DELETE", actions::delete)
            .action("COOKBOOK_INGREDIENTS", actions::ingredients)
            .action("COOKBOOK_BASE", actions::base)
            .action("COOKBOOK_STARTED", |_| async {
                Ok(json!({ "ready": true }))
            });
    }
}
