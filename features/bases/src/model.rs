//! A base as data: its views as JSON, the edits a toolbar (or an agent)
//! makes to it, and the view types there are.
//!
//! Everything here is over `vault::bases` (`ParsedBase`, `ViewSpec`): the
//! parser and evaluator are the vault's. A base with no views reads as one
//! table of `file.name` ([`default_view`]), as Obsidian shows one.

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vault::bases::{
    BaseRow, Conjunction, Formula, ParsedBase, SortDir, SortKey, ViewKind, ViewSpec,
    filter_clauses, filter_of_clauses,
};

/// A view type a base's view may be: Bases' own (table, cards, list), or
/// one an extension adds (`[[base_views]]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewType {
    /// The base's `type:` (`table`, `board`).
    pub id: String,
    pub name: String,
    /// A lucide icon's name (`table`, `kanban`).
    pub icon: String,
    /// The extension that adds it; `None` for Bases' own.
    pub owner: Option<String>,
    /// The options its views take (the extension's `[[base_views]]
    /// options`), as JSON.
    pub options: Vec<Value>,
}

/// Bases' own view types: table, cards and list.
#[must_use]
pub fn builtin_types() -> Vec<ViewType> {
    [
        ("table", "Table", "table"),
        ("cards", "Cards", "layout-grid"),
        ("list", "List", "list"),
    ]
    .into_iter()
    .map(|(id, name, icon)| ViewType {
        id: id.to_owned(),
        name: name.to_owned(),
        icon: icon.to_owned(),
        owner: None,
        options: Vec::new(),
    })
    .collect()
}

/// Whether Bases draws `kind` itself (`gallery` is cards' old name).
#[must_use]
pub fn is_builtin(kind: &str) -> bool {
    matches!(kind, "table" | "cards" | "gallery" | "list")
}

/// The contributed view types, as they are now: what an app hands the
/// `BASES_*` actions (`actions::register`), filled from its extension
/// host (`ui::contributed_types`). Cheap to clone; clones share it.
#[derive(Clone, Default)]
pub struct ViewTypes(Arc<RwLock<Option<Source>>>);

type Source = Arc<dyn Fn() -> Vec<ViewType> + Send + Sync>;

impl std::fmt::Debug for ViewTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ViewTypes")
            .field(&self.contributed().len())
            .finish()
    }
}

impl ViewTypes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the contributed types come from from now on.
    pub fn set_source(&self, source: impl Fn() -> Vec<ViewType> + Send + Sync + 'static) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(source));
    }

    /// The types extensions add now.
    #[must_use]
    pub fn contributed(&self) -> Vec<ViewType> {
        let source = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        source.map(|s| s()).unwrap_or_default()
    }

    /// Bases' own types, then the contributed ones (one per id, the first
    /// that came).
    #[must_use]
    pub fn all(&self) -> Vec<ViewType> {
        let mut out = builtin_types();
        for t in self.contributed() {
            if !out.iter().any(|o| o.id.eq_ignore_ascii_case(&t.id)) {
                out.push(t);
            }
        }
        out
    }
}

/// A view's kind from its `type:` text.
#[must_use]
pub fn kind_of(kind: &str) -> ViewKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "table" => ViewKind::Table,
        "cards" => ViewKind::Cards,
        "board" => ViewKind::Board,
        "gallery" => ViewKind::Gallery,
        "calendar" => ViewKind::Calendar,
        "list" => ViewKind::List,
        _ => ViewKind::Other(kind.trim().to_owned()),
    }
}

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

/// A JSON value's field, `null` when it has none (never panics, unlike
/// `value["key"]`).
pub trait At {
    fn at(&self, key: &str) -> &Value;
}

impl At for Value {
    fn at(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(key).unwrap_or(&NULL)
    }
}

/// Sets field `key` of `value`, when it is an object.
pub fn put(value: &mut Value, key: &str, field: Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_owned(), field);
    }
}

/// View `name` (any case), or the first, to change.
fn view_mut<'a>(base: &'a mut ParsedBase, name: Option<&str>) -> Result<&'a mut ViewSpec, String> {
    let i = view_index(base, name)?;
    base.views
        .get_mut(i)
        .ok_or_else(|| "no such view".to_owned())
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

/// Every column a view may show over `rows`: the file's own (`file.name`,
/// `file.folder`, …), each property a note has, each of the base's
/// formulas (`formula.<name>`).
#[must_use]
pub fn columns(base: &ParsedBase, rows: &[BaseRow]) -> Vec<String> {
    let mut out: Vec<String> = [
        "file.name",
        "file.path",
        "file.folder",
        "file.ext",
        "file.tags",
        "file.mtime",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut keys: Vec<String> = rows
        .iter()
        .flat_map(|r| r.frontmatter.keys().cloned())
        .collect();
    keys.sort_by_key(|k| k.to_lowercase());
    keys.dedup();
    out.extend(keys);
    out.extend(base.formulas.iter().map(|f| format!("formula.{}", f.name)));
    out
}

/// The property a column writes when its cell is edited: a note's own
/// (`status`, `note.status`); `None` for a file's or a formula's.
#[must_use]
pub fn editable(column: &str) -> Option<&str> {
    let key = column.strip_prefix("note.").unwrap_or(column);
    let derived = key.starts_with("file.") || key.starts_with("formula.") || key == "basename";
    (!derived && !key.is_empty() && !key.contains('.')).then_some(key)
}

/// Where a filter edit goes: one view's, or the whole base's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    View,
    Base,
}

/// Sets the filters of view `view` (or the base's, `Scope::Base`) to
/// `clauses`, joined by `join`.
///
/// # Errors
///
/// No such view, or a clause that does not parse.
pub fn set_filters(
    base: &mut ParsedBase,
    view: Option<&str>,
    scope: Scope,
    join: Conjunction,
    clauses: &[String],
) -> Result<(), String> {
    let node = filter_of_clauses(join, clauses).map_err(|e| e.to_string())?;
    match scope {
        Scope::Base => base.global_filter = node,
        Scope::View => {
            view_mut(base, view)?.filter =
                (!matches!(node, vault::bases::FilterNode::None)).then_some(node);
        }
    }
    Ok(())
}

/// Sorts view `view` by `keys`, each `property` or `property desc`.
///
/// # Errors
///
/// No such view.
pub fn set_sort(base: &mut ParsedBase, view: Option<&str>, keys: &[String]) -> Result<(), String> {
    view_mut(base, view)?.sort = keys
        .iter()
        .filter_map(|k| {
            let mut words = k.split_whitespace();
            let property = words.next()?.to_owned();
            let direction = match words.next().map(str::to_ascii_lowercase).as_deref() {
                Some("desc") => SortDir::Desc,
                _ => SortDir::Asc,
            };
            Some(SortKey {
                property,
                direction,
            })
        })
        .collect();
    Ok(())
}

/// Groups view `view` by `property`, or not at all.
///
/// # Errors
///
/// No such view.
pub fn set_group(
    base: &mut ParsedBase,
    view: Option<&str>,
    property: Option<&str>,
) -> Result<(), String> {
    view_mut(base, view)?.group_by = property
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    Ok(())
}

/// Shows `order` in view `view`, in that order (the rest hidden).
///
/// # Errors
///
/// No such view.
pub fn set_order(
    base: &mut ParsedBase,
    view: Option<&str>,
    order: &[String],
) -> Result<(), String> {
    let v = view_mut(base, view)?;
    let mut seen = Vec::new();
    for c in order.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        if !seen.iter().any(|s: &String| s == c) {
            seen.push(c.to_owned());
        }
    }
    v.order = seen;
    Ok(())
}

/// Sets formula `name` to `expression`, or removes it when `expression`
/// is empty.
///
/// # Errors
///
/// An empty name, or an expression that does not parse.
pub fn set_formula(base: &mut ParsedBase, name: &str, expression: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a formula needs a name".to_owned());
    }
    let expression = expression.trim();
    if expression.is_empty() {
        base.formulas.retain(|f| f.name != name);
        return Ok(());
    }
    vault::bases::expr_parser::parse_expression(expression).map_err(|e| e.to_string())?;
    match base.formulas.iter_mut().find(|f| f.name == name) {
        Some(f) => expression.clone_into(&mut f.expression),
        None => base.formulas.push(Formula {
            name: name.to_owned(),
            expression: expression.to_owned(),
        }),
    }
    Ok(())
}

/// Adds a view of type `kind` named `name` (a fresh name when it is
/// taken), showing what the first view shows; its name.
///
/// # Errors
///
/// An empty type.
pub fn add_view(base: &mut ParsedBase, name: &str, kind: &str) -> Result<String, String> {
    if kind.trim().is_empty() {
        return Err("a view needs a type".to_owned());
    }
    let wanted = if name.trim().is_empty() {
        kind.trim()
    } else {
        name.trim()
    };
    let mut name = wanted.to_owned();
    let mut n = 2_u32;
    while base
        .views
        .iter()
        .any(|v| v.name.eq_ignore_ascii_case(&name))
    {
        name = format!("{wanted} {n}");
        n = n.saturating_add(1);
    }
    let order = base
        .views
        .first()
        .map_or_else(|| vec!["file.name".to_owned()], |v| v.order.clone());
    base.views.push(ViewSpec {
        kind: kind_of(kind),
        name: name.clone(),
        order,
        ..default_view()
    });
    Ok(name)
}

/// Sets option `key` of view `view` (its type's: `columnBy`) to `value`,
/// or removes it when `value` is null.
///
/// # Errors
///
/// No such view, or a key the base itself uses.
pub fn set_option(
    base: &mut ParsedBase,
    view: Option<&str>,
    key: &str,
    value: Value,
) -> Result<(), String> {
    const OWN: &[&str] = &[
        "type", "name", "filters", "order", "sort", "limit", "groupBy",
    ];
    if key.trim().is_empty() || OWN.contains(&key) {
        return Err(format!("{key:?} is not a view option"));
    }
    let extras = &mut view_mut(base, view)?.extras;
    if !extras.is_object() {
        *extras = Value::Object(Map::new());
    }
    if let Value::Object(map) = extras {
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), value);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ParsedBase {
        vault::bases::parse(
            "views:\n  - type: table\n    name: All\n    order: [file.name, status]\n",
        )
        .unwrap()
    }

    #[test]
    fn edits_land_in_the_view_and_survive_a_round_trip() {
        let mut b = base();
        set_filters(
            &mut b,
            None,
            Scope::View,
            Conjunction::And,
            &["status != null".into()],
        )
        .unwrap();
        set_sort(
            &mut b,
            Some("all"),
            &["status desc".into(), "file.name".into()],
        )
        .unwrap();
        set_group(&mut b, None, Some("status")).unwrap();
        set_order(
            &mut b,
            None,
            &["status".into(), "file.name".into(), "status".into()],
        )
        .unwrap();
        set_formula(&mut b, "loud", "upper(status)").unwrap();
        let added = add_view(&mut b, "", "board").unwrap();
        assert_eq!(added, "board");
        assert_eq!(add_view(&mut b, "board", "board").unwrap(), "board 2");
        set_option(&mut b, Some("board"), "columnBy", json!("status")).unwrap();
        assert!(set_option(&mut b, None, "sort", json!(1)).is_err());
        let again = vault::bases::parse(&vault::bases::serialize(&b).unwrap()).unwrap();
        let v = view_json(&again.views[0]);
        assert_eq!(v["groupBy"], "status");
        assert_eq!(v["order"], json!(["status", "file.name"]));
        assert_eq!(
            v["sort"][0],
            json!({ "property": "status", "direction": "DESC" })
        );
        assert_eq!(v["filters"]["clauses"], json!(["status != null"]));
        assert_eq!(again.formulas[0].name, "loud");
        assert_eq!(again.views[1].kind.as_str(), "board");
        assert_eq!(view_json(&again.views[1])["options"]["columnBy"], "status");
        set_formula(&mut b, "loud", "").unwrap();
        assert!(b.formulas.is_empty());
        assert!(view_index(&b, Some("nope")).is_err());
    }

    #[test]
    fn cells_of_a_notes_own_properties_are_editable() {
        assert_eq!(editable("status"), Some("status"));
        assert_eq!(editable("note.status"), Some("status"));
        assert_eq!(editable("file.name"), None);
        assert_eq!(editable("formula.x"), None);
    }

    #[test]
    fn the_types_are_bases_own_then_the_contributed() {
        let types = ViewTypes::new();
        assert_eq!(types.all().len(), 3);
        types.set_source(|| {
            vec![
                ViewType {
                    id: "board".into(),
                    name: "Board".into(),
                    owner: Some("lab.board".into()),
                    ..ViewType::default()
                },
                ViewType {
                    id: "TABLE".into(),
                    ..ViewType::default()
                },
            ]
        });
        let ids: Vec<_> = types.all().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["table", "cards", "list", "board"]);
    }
}
