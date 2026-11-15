//! `Meals`, Mealplan's one service (`#[architect::rpc]`).
//!
//! Its one implementation is [`crate::backend::Backend`], over the
//! vault's documents and index as the host gives them and the
//! Cookbook's `architect.cookbook`, reached through the host. The
//! extension serves it in process (`architect::serve_local`, on every
//! platform) and its actions, views and `[provides]
//! "architect.mealplan"` hold only the client ([`crate::client`]).

use facet::Facet;
use serde::{Deserialize, Serialize};

/// One meal on the plan: a recipe for a meal of a day.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct PlannedMeal {
    /// YYYY-MM-DD.
    #[serde(default)]
    pub date: String,
    /// `breakfast`, `lunch`, `dinner` or `snack`.
    #[serde(default)]
    pub meal: String,
    /// The recipe's vault path: `Cookbook/Pancakes.cook`.
    pub recipe: String,
    /// The recipe's name (its file's), for showing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// For how many; the recipe's own when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servings: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Its Calendar event's path, once it is one (`MEALPLAN_SCHEDULE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// A day of a week, its meals by meal.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Day {
    pub date: String,
    /// `monday`…`sunday`.
    pub weekday: String,
    pub meals: Vec<PlannedMeal>,
}

/// A week's plan.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Week {
    /// `2026-W47`.
    pub week: String,
    /// Its note: `Meal plans/2026-W47.md` (there or not).
    pub path: String,
    /// Monday, and the Monday after (exclusive).
    pub from: String,
    pub to: String,
    /// Its seven days.
    pub days: Vec<Day>,
}

/// Dates: a `week` (2026-W47), the week of a `date`, or `from`..`to`
/// (`to` exclusive). None: this week.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Range {
    #[serde(default)]
    pub week: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// A meal on the plan, by its day and meal (and recipe, when the meal
/// has more than one).
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct MealRef {
    pub date: String,
    pub meal: String,
    #[serde(default)]
    pub recipe: Option<String>,
}

/// Where a meal goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct MoveTo {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub meal: Option<String>,
}

/// How a meal is put on its day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct ScheduleHow {
    /// The day plan's block; the meal block named like the meal else.
    #[serde(default)]
    pub block: Option<String>,
    /// HH:MM, for an event.
    #[serde(default)]
    pub time: Option<String>,
    /// Make it a Calendar event even when a block took it.
    #[serde(default)]
    pub event: bool,
}

/// Where a meal went on its day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Scheduled {
    /// The Day Plans block it was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// Its Calendar event's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Why one was not (no Day Plans, no meal block, no Calendar).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// One thing in the pantry: `Pantry/<name>.md`.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct PantryItem {
    #[serde(default)]
    pub path: String,
    pub name: String,
    /// How much there is; unknown when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<f64>,
    /// `g`, `ml`, `cups`; empty for a count.
    #[serde(default)]
    pub unit: String,
    /// The aisle or category it is bought from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aisle: Option<String>,
    /// Always kept in the house: with no `qty` it counts as there.
    #[serde(default)]
    pub staple: bool,
    /// Under this, a staple goes on the grocery list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// Other names recipes use for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// A staple under its min.
    #[serde(default)]
    pub low: bool,
    /// Past its date (today).
    #[serde(default)]
    pub expired: bool,
}

/// A look in the pantry: every field narrows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct PantrySearch {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub aisle: Option<String>,
    #[serde(default)]
    pub low: bool,
    #[serde(default)]
    pub expired: bool,
}

/// A change to a pantry item's stock and fields.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Stock {
    pub name: String,
    /// Sets the stock.
    #[serde(default)]
    pub qty: Option<f64>,
    /// Adds to it (after `qty`).
    #[serde(default)]
    pub add: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub aisle: Option<String>,
    #[serde(default)]
    pub staple: Option<bool>,
    #[serde(default)]
    pub min: Option<f64>,
    /// YYYY-MM-DD; empty clears it.
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
}

/// One substitution: `ratio` of `to` for one of `from`.
#[derive(Debug, Clone, PartialEq, Facet, Serialize, Deserialize)]
pub struct Substitution {
    pub from: String,
    pub to: String,
    #[serde(default = "one")]
    pub ratio: f64,
    /// Always bought instead (oat milk for milk); otherwise used when
    /// the pantry has enough of `to`.
    #[serde(default)]
    pub always: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

const fn one() -> f64 {
    1.0
}

impl Default for Substitution {
    fn default() -> Self {
        Self {
            from: String::new(),
            to: String::new(),
            ratio: 1.0,
            always: false,
            note: None,
        }
    }
}

/// One line of a grocery list.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct GroceryItem {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<f64>,
    #[serde(default)]
    pub unit: String,
    /// Amounts that are not numbers (`a pinch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Why it is there, when not for a recipe (`restock`, `for butter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The recipes it is for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipes: Vec<String>,
    /// The line as written: `600 g flour`.
    pub text: String,
    pub checked: bool,
    /// Written by hand, not built.
    #[serde(default)]
    pub own: bool,
}

/// An aisle's items.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Aisle {
    pub aisle: String,
    pub items: Vec<GroceryItem>,
}

/// A grocery list.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Groceries {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week: Option<String>,
    pub from: String,
    pub to: String,
    pub aisles: Vec<Aisle>,
    /// `butter → olive oil`: what the pantry stood in for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub substituted: Vec<String>,
    /// The note's text.
    pub text: String,
}

/// A tick on a grocery list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Check {
    /// The list's week; this week's by default.
    #[serde(default)]
    pub week: Option<String>,
    /// Or its path.
    #[serde(default)]
    pub path: Option<String>,
    /// The item's name or its line's text.
    pub item: String,
    /// `false` unticks it.
    #[serde(default = "yes")]
    pub checked: bool,
    /// Bought: its quantity goes into the pantry.
    #[serde(default)]
    pub bought: bool,
}

const fn yes() -> bool {
    true
}

/// Whether the pantry has what a recipe needs.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Fulfilment {
    pub recipe: String,
    #[serde(rename = "canCook")]
    pub can_cook: bool,
    /// What it has.
    pub have: Vec<String>,
    /// What is missing, and how much.
    pub missing: Vec<GroceryItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub substituted: Vec<String>,
}

/// Why Mealplan could not.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Serialize, Deserialize, thiserror::Error)]
#[repr(u8)]
pub enum MealplanError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0} is there already")]
    Exists(String),
    /// A date, a meal, an amount it cannot read.
    #[error("{0}")]
    Invalid(String),
    /// The vault or the Cookbook refused or failed.
    #[error("{0}")]
    Failed(String),
}

impl From<architect::TransportError> for MealplanError {
    fn from(e: architect::TransportError) -> Self {
        Self::Failed(e.to_string())
    }
}

/// The meal plan, the pantry and the grocery list.
#[architect::rpc]
pub trait Meals {
    /// A week's plan.
    async fn week(&self, range: Range) -> Result<Week, MealplanError>;
    /// The meals planned in a range, by date and meal.
    async fn meals(&self, range: Range) -> Result<Vec<PlannedMeal>, MealplanError>;
    /// Plans a meal (one undo step): its week.
    async fn plan(&self, meal: PlannedMeal) -> Result<Week, MealplanError>;
    /// Takes a meal off the plan: its week.
    async fn unplan(&self, meal: MealRef) -> Result<Week, MealplanError>;
    /// Moves a meal: the week it went to.
    async fn move_meal(&self, meal: MealRef, to: MoveTo) -> Result<Week, MealplanError>;
    /// Puts a meal on its day: a Day Plans block, a Calendar event.
    async fn schedule(&self, meal: MealRef, how: ScheduleHow) -> Result<Scheduled, MealplanError>;
    /// The pantry, by name.
    async fn pantry(&self, search: PantrySearch) -> Result<Vec<PantryItem>, MealplanError>;
    /// Sets or adds stock (one undo step): the item.
    async fn stock(&self, change: Stock) -> Result<PantryItem, MealplanError>;
    /// Deletes a pantry item.
    async fn remove_item(&self, name: String) -> Result<(), MealplanError>;
    /// The substitutions.
    async fn substitutions(&self) -> Result<Vec<Substitution>, MealplanError>;
    /// Adds or changes a substitution: them all.
    async fn substitute(&self, rule: Substitution) -> Result<Vec<Substitution>, MealplanError>;
    /// Removes substitutions from `from` (to `to`): the rest.
    async fn unsubstitute(
        &self,
        from: String,
        to: Option<String>,
    ) -> Result<Vec<Substitution>, MealplanError>;
    /// Builds or refreshes a range's grocery list (one undo step).
    async fn groceries(&self, range: Range) -> Result<Groceries, MealplanError>;
    /// A range's grocery list as it is (not built again).
    async fn grocery_list(&self, range: Range) -> Result<Groceries, MealplanError>;
    /// Ticks or unticks an item on a grocery list.
    async fn check(&self, check: Check) -> Result<Groceries, MealplanError>;
    /// Whether the pantry has what a recipe needs.
    async fn can_cook(
        &self,
        recipe: String,
        servings: Option<f64>,
    ) -> Result<Fulfilment, MealplanError>;
}

pub use prelude::*;
