//! The vault and the Cookbook, through the host.
//!
//! The vault's documents (read, create, edit as one undo step, delete,
//! list) and its index (what a browser has of the notes it has not
//! opened), and the services Mealplan calls (`architect.cookbook`, and
//! `architect.dayplan` and `architect.calendar` when they are there):
//! the only way it reaches either, so its process copy and a browser's
//! work as the linked one on a desk does.
//!
//! [`Files::Memory`] is a vault and a cookbook in memory, for the
//! model's tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::linked::Handle;
use serde_json::{Map, Value, json};

use crate::groceries::Need;
use crate::model::{fold, stem};
use crate::service::MealplanError;

/// The service of the recipes (`[requires]`).
pub const COOKBOOK: &str = "architect.cookbook";
/// Day Plans and the Calendar (`[uses]`).
pub const DAYPLAN: &str = "architect.dayplan";
pub const CALENDAR: &str = "architect.calendar";

/// A recipe in the in-memory cookbook.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryRecipe {
    pub servings: f64,
    /// For `servings`.
    pub ingredients: Vec<Need>,
}

/// A vault and a cookbook in memory.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    pub files: BTreeMap<String, String>,
    /// By path.
    pub recipes: BTreeMap<String, MemoryRecipe>,
    pub settings: Map<String, Value>,
}

/// Where the notes and the recipes are.
#[derive(Clone)]
pub enum Files {
    /// The app's vault and services, through the host.
    Host(Handle),
    /// In memory.
    Memory(Arc<Mutex<Memory>>),
}

fn failed(e: HostError, path: &str) -> MealplanError {
    match e {
        HostError::NotFound { .. } => MealplanError::NotFound(format!("no note {path}")),
        other => MealplanError::Failed(other.to_string()),
    }
}

fn lock(m: &Mutex<Memory>) -> std::sync::MutexGuard<'_, Memory> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The rows of a query's answer (`{rows: {groups: [{rows}]}}`): each
/// one's path and its values (`note.<key>` read as `<key>`).
#[must_use]
pub fn rows_of(out: &Value) -> Vec<(String, Map<String, Value>)> {
    let table = out.get("rows").unwrap_or(out);
    table
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|g| {
            g.get("rows")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|r| {
            let path = r.get("path").and_then(Value::as_str)?.to_owned();
            let mut values = Map::new();
            if let Some(m) = r.get("values").and_then(Value::as_object) {
                for (k, v) in m {
                    let k = k.strip_prefix("note.").unwrap_or(k);
                    if !v.is_null() {
                        values.insert(k.to_owned(), v.clone());
                    }
                }
            }
            Some((path, values))
        })
        .collect()
}

// The in-memory vault holds its lock for one arm's few lines.
#[allow(clippy::significant_drop_tightening)]
impl Files {
    /// An empty vault and cookbook in memory.
    #[must_use]
    pub fn memory() -> Self {
        Self::Memory(Arc::new(Mutex::new(Memory::default())))
    }

    async fn call(h: &Handle, request: HostRequest, path: &str) -> Result<Value, MealplanError> {
        h.host(request).await.map_err(|e| failed(e, path))
    }

    /// The text of the note at `path`; `None` when there is none.
    pub async fn read(&self, path: &str) -> Result<Option<String>, MealplanError> {
        match self {
            Self::Host(h) => {
                let answer = h
                    .host(HostRequest::ReadDocument {
                        path: path.to_owned(),
                    })
                    .await;
                match answer {
                    Ok(v) => Ok(v.as_str().map(str::to_owned)),
                    Err(HostError::NotFound { .. }) => Ok(None),
                    Err(e) => Err(MealplanError::Failed(e.to_string())),
                }
            }
            Self::Memory(m) => Ok(lock(m).files.get(path).cloned()),
        }
    }

    /// The note at `path` written as `text`: made when it is not there,
    /// else one undo step named `step` (diffed, so the person's typing
    /// elsewhere in it survives).
    pub async fn write(&self, path: &str, text: &str, step: &str) -> Result<(), MealplanError> {
        match self {
            Self::Host(h) => {
                let exists = self.read(path).await?.is_some();
                let request = if exists {
                    HostRequest::EditDocument {
                        path: path.to_owned(),
                        edits: vec![architect_extensions::documents::DocEdit::Replace {
                            text: text.to_owned(),
                        }],
                        name: Some(step.to_owned()),
                    }
                } else {
                    HostRequest::CreateDocument {
                        path: path.to_owned(),
                        text: text.to_owned(),
                    }
                };
                Self::call(h, request, path).await.map(|_| ())
            }
            Self::Memory(m) => {
                lock(m).files.insert(path.to_owned(), text.to_owned());
                Ok(())
            }
        }
    }

    /// The note at `path` deleted (to the vault's trash).
    pub async fn delete(&self, path: &str) -> Result<(), MealplanError> {
        match self {
            Self::Host(h) => Self::call(
                h,
                HostRequest::DeleteDocument {
                    path: path.to_owned(),
                },
                path,
            )
            .await
            .map(|_| ()),
            Self::Memory(m) => lock(m)
                .files
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| MealplanError::NotFound(format!("no note {path}"))),
        }
    }

    /// The Markdown notes under `folder`, by path: each with what the
    /// vault's index has of its properties (none when it keeps no index,
    /// or has not read it yet).
    pub async fn under(
        &self,
        folder: &str,
    ) -> Result<BTreeMap<String, Map<String, Value>>, MealplanError> {
        let prefix = format!("{}/", folder.trim_matches('/'));
        let mut out: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        match self {
            Self::Host(h) => {
                let source = format!(
                    "filters:\n  and:\n    - file.inFolder({})\n    - file.ext == \"md\"\nviews:\n  - type: table\n    name: Notes\n",
                    serde_json::to_string(folder.trim_matches('/')).unwrap_or_default()
                );
                let indexed = h
                    .host(HostRequest::QueryVault {
                        path: None,
                        source: Some(source),
                        view: None,
                        if_changed: None,
                        limit: Some(5000),
                    })
                    .await;
                match indexed {
                    Ok(v) => out.extend(rows_of(&v)),
                    Err(HostError::Unsupported { .. }) => {}
                    Err(e) => return Err(MealplanError::Failed(e.to_string())),
                }
                let listed = Self::call(
                    h,
                    HostRequest::ListDocuments {
                        under: Some(folder.trim_matches('/').to_owned()),
                    },
                    folder,
                )
                .await;
                // A host without the documents still has the index.
                if let Ok(v) = listed {
                    for d in v.as_array().into_iter().flatten() {
                        if let Some(p) = d.get("path").and_then(Value::as_str) {
                            out.entry(p.to_owned()).or_default();
                        }
                    }
                }
            }
            Self::Memory(m) => {
                for p in lock(m).files.keys() {
                    out.insert(p.clone(), Map::new());
                }
            }
        }
        out.retain(|p, _| {
            p.starts_with(&prefix)
                && std::path::Path::new(p)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        });
        Ok(out)
    }

    /// Mealplan's setting `key`, or `null`.
    pub async fn setting(&self, key: &str) -> Value {
        match self {
            Self::Host(h) => h
                .host(HostRequest::Setting {
                    key: key.to_owned(),
                })
                .await
                .unwrap_or(Value::Null),
            Self::Memory(m) => lock(m).settings.get(key).cloned().unwrap_or(Value::Null),
        }
    }

    /// Calls `method` of `service`: `None` when no loaded extension
    /// provides it (or in memory).
    pub async fn service(
        &self,
        service: &str,
        method: &str,
        args: Value,
    ) -> Option<Result<Value, String>> {
        let Self::Host(h) = self else {
            return None;
        };
        match h.service(service, method, args).await {
            Ok(v) => Some(Ok(v)),
            Err(HostError::NotFound { what }) if what.starts_with("provider of") => None,
            Err(e) => Some(Err(e.to_string())),
        }
    }

    async fn cookbook(&self, method: &str, args: Value) -> Result<Value, MealplanError> {
        self.service(COOKBOOK, method, args).await.map_or_else(
            || {
                Err(MealplanError::Failed(
                    "no Cookbook: Mealplan reads recipes from architect.cookbook".to_owned(),
                ))
            },
            |r| r.map_err(MealplanError::Failed),
        )
    }

    /// What `portions` (`[(path, servings)]`) need, by the Cookbook's
    /// `ingredients`: scaled, nested recipes cooked out.
    pub async fn ingredients(
        &self,
        portions: &[(String, Option<f64>)],
    ) -> Result<Vec<Need>, MealplanError> {
        if portions.is_empty() {
            return Ok(Vec::new());
        }
        if let Self::Memory(m) = self {
            let m = lock(m);
            let mut out = Vec::new();
            for (path, servings) in portions {
                let r = m
                    .recipes
                    .get(path)
                    .ok_or_else(|| MealplanError::NotFound(format!("no recipe {path}")))?;
                let k = servings.map_or(1.0, |s| s / r.servings.max(f64::EPSILON));
                out.extend(r.ingredients.iter().map(|n| Need {
                    qty: n.qty.map(|q| q * k),
                    recipe: path.clone(),
                    ..n.clone()
                }));
            }
            return Ok(out);
        }
        let portions: Vec<Value> = portions
            .iter()
            .map(|(path, servings)| json!({ "path": path, "servings": servings }))
            .collect();
        let v = self
            .cookbook("ingredients", json!({ "portions": portions }))
            .await?;
        serde_json::from_value(v)
            .map_err(|e| MealplanError::Failed(format!("the Cookbook's ingredients: {e}")))
    }

    /// The vault path of recipe `recipe`: a `.cook` path as it is, else
    /// the recipe of that name (the Cookbook's `list`).
    pub async fn recipe_path(&self, recipe: &str) -> Result<String, MealplanError> {
        let r = recipe.trim();
        if r.is_empty() {
            return Err(MealplanError::Invalid("give a `recipe`".to_owned()));
        }
        if std::path::Path::new(r)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cook"))
        {
            return Ok(r.to_owned());
        }
        let want = fold(r);
        // The Cookbook's `list {text}` finds names with those words.
        let found: Vec<(String, String)> = match self {
            Self::Memory(m) => lock(m)
                .recipes
                .keys()
                .map(|p| (p.clone(), stem(p)))
                .filter(|(_, n)| fold(n).contains(&want))
                .collect(),
            Self::Host(_) => self
                .cookbook("list", json!({ "text": r }))
                .await?
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    Some((
                        s.get("path")?.as_str()?.to_owned(),
                        s.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    ))
                })
                .collect(),
        };
        found
            .iter()
            .find(|(p, n)| fold(n) == want || fold(&stem(p)) == want)
            .or_else(|| found.first())
            .map(|(p, _)| p.clone())
            .ok_or_else(|| MealplanError::NotFound(format!("no recipe {r:?} in the Cookbook")))
    }
}
