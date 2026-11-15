//! Mealplan, a first-party architect extension: Task's meal planning
//! (`task:features/mealplan/mealplan`, `task:features/mealplan/pantry`),
//! the architect way.
//!
//! - **Everything is a note** ([`model`]): a week's plan
//!   (`Meal plans/2026-W47.md`, its meals in the frontmatter), each
//!   pantry item (`Pantry/Flour.md`), the substitutions
//!   (`Pantry/Substitutions.md`) and each grocery list
//!   (`Groceries/2026-W47.md`, a checklist under `## <aisle>`
//!   headings). They sync, open, show in Bases and travel by share link.
//! - **The grocery list builds itself** ([`groceries`]): the planned
//!   meals' recipes' ingredients from `architect.cookbook` (scaled,
//!   nested recipes cooked out), added up with Cooklang's units
//!   ([`units`]), less the pantry, substitutions applied, grouped by
//!   aisle; built again with the ticks and the hand-written lines kept.
//! - **One service, `Meals`** ([`service`], `#[architect::rpc]`): its
//!   one implementation [`backend::Backend`] over the vault and the
//!   Cookbook through the host ([`files`]), served in process on every
//!   platform ([`client`]). The actions ([`actions`]) hold only the
//!   client; other extensions call `architect.mealplan` (`[provides]`).
//! - The Cookbook, Day Plans and the Calendar are services it calls
//!   (`[requires]`, `[uses]`), never crates it links.
//!
//! [`Mealplan`] is the extension (`extensions/mealplan/extension.toml`).

// The wire types derive facet (the service's wire) and serde (the JSON
// an extension's actions speak); facet's derive writes `unsafe`, which
// this lint reads as a hand-written invariant serde could skip.
#![allow(clippy::unsafe_derive_deserialize)]
// Amounts are f64 (no overflow, only infinities a person cannot type)
// and dates stay far inside chrono's range; indexes are positions just
// found in the same vector.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

pub mod actions;
pub mod backend;
pub mod client;
pub mod files;
pub mod groceries;
pub mod model;
pub mod service;
pub mod ui;
pub mod units;

pub use service::{
    Aisle, Check, Day, Fulfilment, Groceries, GroceryItem, MealRef, MealplanError, Meals,
    MealsClient, MoveTo, PantryItem, PantrySearch, PlannedMeal, Range, ScheduleHow, Scheduled,
    Stock, Substitution, Week,
};

use architect_extensions::linked::{Builder, Extension};
use serde_json::json;

/// Its id: its actions are `MEALPLAN_*`.
pub const ID: &str = "mealplan";

/// The service it provides (`[provides]`): method `m` is `MEALPLAN_<M>`.
pub const SERVICE: &str = "architect.mealplan";

/// The extension: link it in (`Extensions::with_linked`) or run its
/// program.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mealplan;

impl Extension for Mealplan {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        client::connect(ext.handle());
        ext.view("week", ui::WeekView)
            .view("pantry", ui::PantryView)
            .view("groceries", ui::GroceriesView)
            .action("MEALPLAN_WEEK", actions::week)
            .action("MEALPLAN_MEALS", actions::meals)
            .action("MEALPLAN_PLAN", actions::plan)
            .action("MEALPLAN_UNPLAN", actions::unplan)
            .action("MEALPLAN_MOVE_MEAL", actions::move_meal)
            .action("MEALPLAN_SCHEDULE", actions::schedule)
            .action("MEALPLAN_PANTRY", actions::pantry)
            .action("MEALPLAN_STOCK", actions::stock)
            .action("MEALPLAN_REMOVE_ITEM", actions::remove_item)
            .action("MEALPLAN_SUBSTITUTIONS", actions::substitutions)
            .action("MEALPLAN_SUBSTITUTE", actions::substitute)
            .action("MEALPLAN_UNSUBSTITUTE", actions::unsubstitute)
            .action("MEALPLAN_GROCERIES", actions::groceries)
            .action("MEALPLAN_GROCERY_LIST", actions::grocery_list)
            .action("MEALPLAN_CHECK", actions::check)
            .action("MEALPLAN_OPEN", actions::open)
            .action("MEALPLAN_CAN_COOK", actions::can_cook)
            .action("MEALPLAN_STARTED", |_| async {
                Ok(json!({ "ready": true }))
            });
    }
}
