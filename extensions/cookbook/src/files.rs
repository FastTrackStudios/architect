//! The recipes' files, through the host.
//!
//! The vault's documents (read, create, edit as one undo step, rename,
//! delete) and its index (what a browser has of the notes it has not
//! opened). The only way the
//! cookbook reaches the vault, so its process copy and a browser's work
//! as the linked one on a desk does.
//!
//! [`Files::Memory`] is a vault in memory, for the model's tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::linked::Handle;
use serde_json::{Map, Value};

use crate::service::CookbookError;

/// Where the recipes are.
#[derive(Clone)]
pub enum Files {
    /// The app's vault, through the host.
    Host(Handle),
    /// Files in memory (no index: a list reads them).
    Memory(Arc<Mutex<BTreeMap<String, String>>>),
}

/// One recipe as the vault's index has it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    pub path: String,
    pub name: String,
    /// Its properties, by key.
    pub values: Map<String, Value>,
}

/// The base the index is asked: every Cooklang file, with the
/// properties a summary shows.
pub const INDEX_BASE: &str = "filters:\n  and:\n    - file.ext == \"cook\"\nviews:\n  - type: table\n    name: Recipes\n    order:\n      - file.name\n      - title\n      - description\n      - course\n      - cuisine\n      - servings\n      - prep time\n      - cook time\n      - time\n      - tags\n      - source\n";

fn failed(e: HostError, path: &str) -> CookbookError {
    match e {
        HostError::NotFound { .. } => CookbookError::NotFound(path.to_owned()),
        other => CookbookError::Failed(other.to_string()),
    }
}

fn lock(
    m: &Mutex<BTreeMap<String, String>>,
) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// The in-memory vault holds its lock for one arm's few lines.
#[allow(clippy::significant_drop_tightening)]
impl Files {
    /// A vault in memory holding `files` (path, text).
    pub fn memory<P: Into<String>, T: Into<String>>(
        files: impl IntoIterator<Item = (P, T)>,
    ) -> Self {
        Self::Memory(Arc::new(Mutex::new(
            files
                .into_iter()
                .map(|(p, t)| (p.into(), t.into()))
                .collect(),
        )))
    }

    async fn call(h: &Handle, request: HostRequest, path: &str) -> Result<Value, CookbookError> {
        h.host(request).await.map_err(|e| failed(e, path))
    }

    /// The text of the file at `path`.
    pub async fn read(&self, path: &str) -> Result<String, CookbookError> {
        match self {
            Self::Host(h) => {
                let v = Self::call(
                    h,
                    HostRequest::ReadDocument {
                        path: path.to_owned(),
                    },
                    path,
                )
                .await?;
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| CookbookError::NotFound(path.to_owned()))
            }
            Self::Memory(m) => lock(m)
                .get(path)
                .cloned()
                .ok_or_else(|| CookbookError::NotFound(path.to_owned())),
        }
    }

    /// A new file at `path`: refused when one is there.
    pub async fn create(&self, path: &str, text: &str) -> Result<(), CookbookError> {
        match self {
            Self::Host(h) => {
                if Self::call(
                    h,
                    HostRequest::ReadDocument {
                        path: path.to_owned(),
                    },
                    path,
                )
                .await
                .is_ok()
                {
                    return Err(CookbookError::Exists(path.to_owned()));
                }
                Self::call(
                    h,
                    HostRequest::CreateDocument {
                        path: path.to_owned(),
                        text: text.to_owned(),
                    },
                    path,
                )
                .await
                .map(|_| ())
            }
            Self::Memory(m) => {
                let mut m = lock(m);
                if m.contains_key(path) {
                    return Err(CookbookError::Exists(path.to_owned()));
                }
                m.insert(path.to_owned(), text.to_owned());
                Ok(())
            }
        }
    }

    /// The file at `path` written as `text`: one undo step named `step`
    /// (diffed, so the person's typing elsewhere in it survives).
    pub async fn replace(&self, path: &str, text: &str, step: &str) -> Result<(), CookbookError> {
        match self {
            Self::Host(h) => Self::call(
                h,
                HostRequest::EditDocument {
                    path: path.to_owned(),
                    edits: vec![architect_extensions::documents::DocEdit::Replace {
                        text: text.to_owned(),
                    }],
                    name: Some(step.to_owned()),
                },
                path,
            )
            .await
            .map(|_| ()),
            Self::Memory(m) => {
                let mut m = lock(m);
                let Some(t) = m.get_mut(path) else {
                    return Err(CookbookError::NotFound(path.to_owned()));
                };
                text.clone_into(t);
                Ok(())
            }
        }
    }

    /// The file at `path` moved to `to` (its links follow).
    pub async fn rename(&self, path: &str, to: &str) -> Result<(), CookbookError> {
        match self {
            Self::Host(h) => Self::call(
                h,
                HostRequest::RenameDocument {
                    path: path.to_owned(),
                    to: to.to_owned(),
                },
                path,
            )
            .await
            .map(|_| ()),
            Self::Memory(m) => {
                let mut m = lock(m);
                if m.contains_key(to) {
                    return Err(CookbookError::Exists(to.to_owned()));
                }
                let t = m
                    .remove(path)
                    .ok_or_else(|| CookbookError::NotFound(path.to_owned()))?;
                m.insert(to.to_owned(), t);
                Ok(())
            }
        }
    }

    /// The file at `path` deleted (to the vault's trash).
    pub async fn delete(&self, path: &str) -> Result<(), CookbookError> {
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
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| CookbookError::NotFound(path.to_owned())),
        }
    }

    /// The cookbook's setting `key` (`cookbook.folder`), or `null`.
    pub async fn setting(&self, key: &str) -> Value {
        match self {
            Self::Host(h) => h
                .host(HostRequest::Setting {
                    key: key.to_owned(),
                })
                .await
                .unwrap_or(Value::Null),
            Self::Memory(_) => Value::Null,
        }
    }

    /// Every Cooklang file's path (`.cook`), read from the documents.
    pub async fn paths(&self) -> Result<Vec<String>, CookbookError> {
        let all: Vec<String> = match self {
            Self::Host(h) => {
                let v = Self::call(h, HostRequest::ListDocuments { under: None }, "").await?;
                v.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d.get("path").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            }
            Self::Memory(m) => lock(m).keys().cloned().collect(),
        };
        Ok(all.into_iter().filter(|p| is_recipe_path(p)).collect())
    }

    /// The recipes as the vault's index has them, or `None` when the app
    /// keeps no index (then [`Self::paths`] and reads).
    pub async fn indexed(&self) -> Result<Option<Vec<Listed>>, CookbookError> {
        let Self::Host(h) = self else {
            return Ok(None);
        };
        let answer = h
            .host(HostRequest::QueryVault {
                path: None,
                source: Some(INDEX_BASE.to_owned()),
                view: None,
                if_changed: None,
                limit: Some(5000),
            })
            .await;
        match answer {
            Ok(v) => Ok(Some(rows_of(&v))),
            Err(HostError::Unsupported { .. }) => Ok(None),
            Err(e) => Err(CookbookError::Failed(e.to_string())),
        }
    }
}

/// Whether `path` is a Cooklang file.
#[must_use]
pub fn is_recipe_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cook"))
}

/// The rows of a query's answer (`{rows: {groups: [{rows}]}}`), a value
/// named `note.<key>` read as `<key>`.
#[must_use]
pub fn rows_of(out: &Value) -> Vec<Listed> {
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
        .map(|r| {
            let text = |k: &str| {
                r.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let mut values = Map::new();
            if let Some(m) = r.get("values").and_then(Value::as_object) {
                for (k, v) in m {
                    let k = k.strip_prefix("note.").unwrap_or(k);
                    if !v.is_null() {
                        values.insert(k.to_owned(), v.clone());
                    }
                }
            }
            Listed {
                path: text("path"),
                name: text("name"),
                values,
            }
        })
        .collect()
}

/// A JSON value as the text it reads as (`4`, `"1 loaf"`), if any.
#[must_use]
pub fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => a.first().and_then(text_of),
        _ => None,
    }
}
