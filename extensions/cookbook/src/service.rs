//! `Recipes`, the cookbook's one service (`#[architect::rpc]`).
//!
//! Its one implementation is [`crate::backend::Backend`], over the
//! vault's documents and its index as the host gives them. The
//! extension serves it in process (`architect::serve_local`, on every
//! platform: a desk, a phone, a browser) and its actions, views and
//! `[provides] "architect.cookbook"` hold only the client
//! ([`crate::client`]). Other extensions (Mealplan) reach it as that
//! service, by JSON through the host, never by importing this crate.

use facet::Facet;
use serde::{Deserialize, Serialize};

use crate::model::Recipe;

/// What a recipe's index entry says of it: enough to list, search and
/// pick one without reading its file (all a browser has of a note it has
/// not opened).
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct RecipeSummary {
    /// Its vault path: `Cookbook/Pancakes.cook`.
    pub path: String,
    /// `title`, else the file's name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub course: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuisine: Option<String>,
    /// As written (`4`, `1 loaf`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servings: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "prepMinutes"
    )]
    pub prep_minutes: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cookMinutes"
    )]
    pub cook_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A search of the cookbook: every field narrows it; none lists all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Search {
    /// Words each found in its name, description, tags, course or
    /// cuisine (case and accents ignored).
    #[serde(default)]
    pub text: Option<String>,
    /// A tag it has.
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub course: Option<String>,
    /// An ingredient it uses (its file is read: on a desk, every recipe;
    /// in a browser, the ones it can open).
    #[serde(default)]
    pub ingredient: Option<String>,
    /// At most this many (all when unset).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// A recipe made for some number of people.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Portion {
    pub path: String,
    /// The servings wanted; the recipe's own when unset.
    #[serde(default)]
    pub servings: Option<f64>,
}

/// One ingredient a set of portions needs, nested recipes cooked out
/// into theirs, scaled.
#[derive(Debug, Clone, Default, PartialEq, Facet, Serialize, Deserialize)]
pub struct Needed {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<f64>,
    /// As Cooklang wrote it (`g`, `cups`); empty for a count.
    #[serde(default)]
    pub unit: String,
    /// The quantity as text when it is not a number (`a pinch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// The recipe it is for (a nested one's own path).
    pub recipe: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// A recipe to write: its Cooklang source, kept exactly as given.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Save {
    /// Where; `<folder>/<name>.cook` when unset (a new recipe).
    #[serde(default)]
    pub path: Option<String>,
    /// The name of a new one, when there is no path and no `title`.
    #[serde(default)]
    pub name: Option<String>,
    pub source: String,
    /// `true`: a new recipe, refused when the path is taken.
    #[serde(default)]
    pub create: bool,
}

/// A recipe from somewhere else, made Cooklang.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Import {
    /// A page: its schema.org Recipe, fetched through the host.
    #[serde(default)]
    pub url: Option<String>,
    /// Or a recipe's text: a `.cook` file (`>>` metadata or YAML
    /// frontmatter), or a page's HTML.
    #[serde(default)]
    pub text: Option<String>,
    /// Where to write it; `<folder>/<its name>.cook` when unset.
    #[serde(default)]
    pub path: Option<String>,
    /// Write it (refused when the path is taken); else only the draft.
    #[serde(default)]
    pub save: bool,
}

/// What an import made.
#[derive(Debug, Clone, PartialEq, Facet, Serialize, Deserialize)]
pub struct Imported {
    /// Where it is, or would be, written.
    pub path: String,
    /// Its Cooklang.
    pub source: String,
    /// It parsed back.
    pub recipe: Recipe,
    /// Whether it was written.
    pub saved: bool,
    /// Made by the synthesizer from a page: worth a read
    /// (`import: needs-review` in its frontmatter).
    pub review: bool,
    /// Ingredient lines the synthesizer could not place in a step (kept
    /// in a last step to place by hand).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unplaced: Vec<String>,
}

/// Why the cookbook could not.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Serialize, Deserialize, thiserror::Error)]
#[repr(u8)]
pub enum CookbookError {
    #[error("no recipe {0}")]
    NotFound(String),
    #[error("{0} is there already")]
    Exists(String),
    /// Cooklang that does not parse, a path that is not a recipe's.
    #[error("{0}")]
    Invalid(String),
    /// The vault refused or failed.
    #[error("{0}")]
    Failed(String),
}

impl From<architect::TransportError> for CookbookError {
    fn from(e: architect::TransportError) -> Self {
        Self::Failed(e.to_string())
    }
}

/// The cookbook.
#[architect::rpc]
pub trait Recipes {
    /// The recipes a search finds, by name.
    async fn list(&self, search: Search) -> Result<Vec<RecipeSummary>, CookbookError>;
    /// One recipe, parsed; scaled to `servings` when given.
    async fn get(&self, path: String, servings: Option<f64>) -> Result<Recipe, CookbookError>;
    /// Writes a recipe (one undo step), parsed back.
    async fn save(&self, recipe: Save) -> Result<Recipe, CookbookError>;
    /// Moves a recipe (links to it follow).
    async fn rename(&self, path: String, to: String) -> Result<Recipe, CookbookError>;
    /// Deletes a recipe (to the vault's trash).
    async fn delete(&self, path: String) -> Result<(), CookbookError>;
    /// What a set of portions needs, nested recipes cooked out, scaled.
    async fn ingredients(&self, portions: Vec<Portion>) -> Result<Vec<Needed>, CookbookError>;
    /// A recipe from a page or a text, as Cooklang; written when asked.
    async fn import(&self, import: Import) -> Result<Imported, CookbookError>;
    /// Each `.cook` file under `folder` (every one when unset) whose
    /// metadata is Cooklang's `>>` lines, rewritten with it as
    /// frontmatter (the properties the vault reads): their paths.
    async fn tidy(&self, folder: Option<String>) -> Result<Vec<String>, CookbookError>;
}

pub use prelude::*;
