//! A base as JSON (feature `live`): the shape `VAULT_QUERY` answers in
//! and the Bases extension reads and edits (`architect_bases::model`).
//!
//! Over `crate::bases` (`ParsedBase`, `ViewSpec`). A base with no
//! views reads as one table of `file.name` ([`default_view`]), as
//! Obsidian shows one.

use serde_json::{Value, json};
use crate::bases::{BaseRow, Conjunction, ParsedBase, SortDir, ViewKind, ViewSpec, filter_clauses};

/// One table of `file.name`: the view a base with none reads as.
#[must_use]
pub fn default_view() -> ViewSpec {
    ViewSpec {
        kind: ViewKind::Table,
        name: "Table".to_owned(),
        filter: None,
        order: vec!["file.name".to_owned()],
        sort: Vec::new(),
        limit: None,
        group_by: None,
        extras: Value::Null,
    }
}

/// `base` with a view to show: as it is, or with [`default_view`] when it
/// has none.
#[must_use]
pub fn with_a_view(mut base: ParsedBase) -> ParsedBase {
    if base.views.is_empty() {
        base.views.push(default_view());
    }
    base
}

/// The index of view `name` (any case), or the first when `None`.
///
/// # Errors
///
/// No view has that name.
pub fn view_index(base: &ParsedBase, name: Option<&str>) -> Result<usize, String> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None if base.views.is_empty() => Err("the base has no views".to_owned()),
        None => Ok(0),
        Some(n) => base
            .views
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(n))
            .ok_or_else(|| format!("no view {n:?}")),
    }
}

/// A view as JSON, in the base's own words: `type`, `name`, `order`,
/// `sort` (`[{property, direction}]`), `groupBy`, `filters` (`{join,
/// clauses}`), `limit`, and its type's options (`options`).
#[must_use]
pub fn view_json(view: &ViewSpec) -> Value {
    let (join, clauses) = view
        .filter
        .as_ref()
        .map_or((Conjunction::And, Vec::new()), filter_clauses);
    json!({
        "type": view.kind.as_str(),
        "name": view.name,
        "order": view.order,
        "sort": view.sort.iter().map(|k| json!({
            "property": k.property,
            "direction": if k.direction == SortDir::Desc { "DESC" } else { "ASC" },
        })).collect::<Vec<_>>(),
        "groupBy": view.group_by,
        "filters": { "join": join, "clauses": clauses },
        "limit": view.limit,
        "options": if view.extras.is_object() { view.extras.clone() } else { json!({}) },
    })
}

/// A base as JSON: its filters, formulas, properties' display names and
/// views.
#[must_use]
pub fn base_json(path: &str, base: &ParsedBase) -> Value {
    let (join, clauses) = filter_clauses(&base.global_filter);
    json!({
        "path": path,
        "filters": { "join": join, "clauses": clauses },
        "formulas": base.formulas.iter().map(|f| json!({ "name": f.name, "expression": f.expression })).collect::<Vec<_>>(),
        "properties": base.properties.iter().map(|p| json!({ "key": p.key, "displayName": p.display_name })).collect::<Vec<_>>(),
        "views": base.views.iter().map(view_json).collect::<Vec<_>>(),
    })
}

/// The file's own columns, which every row has.
pub const FILE_COLUMNS: &[&str] = &[
    "file.name",
    "file.path",
    "file.folder",
    "file.ext",
    "file.tags",
    "file.mtime",
];

/// The property keys `rows` have, once each, sorted without case.
#[must_use]
pub fn property_keys(rows: &[BaseRow]) -> Vec<String> {
    let mut keys: Vec<String> = rows
        .iter()
        .flat_map(|r| r.frontmatter.keys().cloned())
        .collect();
    keys.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    keys.dedup();
    keys
}

/// Every column a view may show: the file's own ([`FILE_COLUMNS`]), each
/// property in `keys` ([`property_keys`]), each of the base's formulas
/// (`formula.<name>`).
#[must_use]
pub fn columns(base: &ParsedBase, keys: &[String]) -> Vec<String> {
    FILE_COLUMNS
        .iter()
        .map(|c| (*c).to_owned())
        .chain(keys.iter().cloned())
        .chain(base.formulas.iter().map(|f| format!("formula.{}", f.name)))
        .collect()
}
