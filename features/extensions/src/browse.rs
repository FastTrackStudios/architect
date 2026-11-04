//! What the extensions page lists (#181): every extension there is —
//! built in, installed, in development, and what every registry offers —
//! one row each, searched by name, kind and what it adds.
//!
//! A query is words, each of which a row must match somewhere: its name,
//! id, description, kind, origin or a line of what it adds (`tab`,
//! `status item`, `opener`). `kind:luau` and `adds:tab` narrow to that
//! field alone. What a registry offers but is not here has no manifest
//! yet: it matches on its listing until its preview is read
//! ([`Browse::remember`]).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::catalog::Origin;
use crate::extensions::Extensions;
use crate::manage::Preview;

/// Where a row's extension is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Where {
    Builtin,
    Installed,
    Dev,
    /// Offered by a registry, not here.
    Available,
}

impl Where {
    /// `Built in`, `Installed`, `In development`, `Available`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Builtin => "Built in",
            Self::Installed => "Installed",
            Self::Dev => "In development",
            Self::Available => "Available",
        }
    }
}

/// One extension, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub description: String,
    pub at: Where,
    /// Which copy runs (`built in (0.1.0), overridden by 1.4.0-dev (in
    /// development)`): `None` for one not here.
    pub runs: Option<String>,
    /// Whether a copy here overrides the built-in one.
    pub overrides_builtin: bool,
    pub loaded: bool,
    pub disabled: bool,
    /// A newer version a registry lists.
    pub newer: Option<String>,
    /// What it adds, a line a kind.
    pub adds: Vec<String>,
    /// The registry that lists it, when one does.
    pub registry: Option<String>,
}

/// The page's catalog: the extensions, and the previews read so far.
#[derive(Clone)]
pub struct Browse {
    extensions: Extensions,
    previews: Arc<Mutex<BTreeMap<String, Preview>>>,
}

impl PartialEq for Browse {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.previews, &other.previews)
    }
}

impl std::fmt::Debug for Browse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browse").finish_non_exhaustive()
    }
}

/// Whether `row` has every word of `query`.
fn matches(row: &Row, query: &str) -> bool {
    let hay = format!(
        "{} {} {} {} {} {}",
        row.name,
        row.id,
        row.description,
        row.kind,
        row.at.label(),
        row.adds.join(" ")
    )
    .to_lowercase();
    query.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        if let Some(k) = word.strip_prefix("kind:") {
            row.kind == k
        } else if let Some(a) = word.strip_prefix("adds:") {
            row.adds.iter().any(|l| l.to_lowercase().contains(a))
        } else {
            hay.contains(&word)
        }
    })
}

impl Browse {
    #[must_use]
    pub fn new(extensions: Extensions) -> Self {
        Self {
            extensions,
            previews: Arc::default(),
        }
    }

    #[must_use]
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Keeps `preview` (what a registry's version adds) for search.
    pub fn remember(&self, preview: Preview) {
        self.previews
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(preview.id.clone(), preview);
    }

    /// The preview read for `id`, when one was.
    #[must_use]
    pub fn preview(&self, id: &str) -> Option<Preview> {
        self.previews
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Every row matching `query`: those here first (by name), then those
    /// a registry offers.
    #[must_use]
    pub fn rows(&self, query: &str) -> Vec<Row> {
        let ext = &self.extensions;
        let offers = ext.offers();
        let newest = |id: &str| {
            offers
                .iter()
                .filter(|l| l.id == id)
                .max_by(|a, b| a.version.cmp(&b.version))
                .cloned()
        };
        let sources: BTreeMap<String, String> = ext
            .search("")
            .into_iter()
            .map(|f| (f.hit.listing.id, f.source))
            .collect();
        let mut here: Vec<Row> = ext
            .chosen()
            .into_iter()
            .map(|c| {
                let m = &c.runs.package.manifest;
                let listing = crate::index::Listing {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.clone(),
                    kind: m.kind.name().to_owned(),
                    description: m.description.clone(),
                    category: String::new(),
                    location: crate::index::Location::Path(String::new()),
                };
                let preview = ext.preview_of(m, &listing, "");
                let newer = newest(&m.id)
                    .filter(|l| l.version > m.version)
                    .map(|l| l.version.to_string());
                Row {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.to_string(),
                    kind: m.kind.name().to_owned(),
                    description: m.description.clone(),
                    at: match c.runs.origin {
                        Origin::Builtin => Where::Builtin,
                        Origin::Installed => Where::Installed,
                        Origin::Dev => Where::Dev,
                    },
                    runs: Some(c.describe()),
                    overrides_builtin: c.runs.origin != Origin::Builtin
                        && c.shadows.iter().any(|(o, _)| *o == Origin::Builtin),
                    loaded: ext.host().get(&m.id).is_some(),
                    disabled: ext.is_disabled(&m.id),
                    newer,
                    adds: preview.adds,
                    registry: sources.get(&m.id).cloned(),
                }
            })
            .collect();
        here.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let mut ids: Vec<String> = here.iter().map(|r| r.id.clone()).collect();
        let mut offered = Vec::new();
        for l in &offers {
            if ids.contains(&l.id) {
                continue;
            }
            ids.push(l.id.clone());
            let Some(l) = newest(&l.id) else { continue };
            let adds = self.preview(&l.id).map(|p| p.adds).unwrap_or_default();
            offered.push(Row {
                id: l.id.clone(),
                name: l.name.clone(),
                version: l.version.to_string(),
                kind: l.kind.clone(),
                description: l.description.clone(),
                at: Where::Available,
                runs: None,
                overrides_builtin: false,
                loaded: false,
                disabled: false,
                newer: None,
                adds,
                registry: sources.get(&l.id).cloned(),
            });
        }
        offered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        here.into_iter()
            .chain(offered)
            .filter(|r| matches(r, query))
            .collect()
    }
}
