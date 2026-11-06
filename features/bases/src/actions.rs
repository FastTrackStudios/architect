//! The `BASES_*` actions (feature `actions`), over an open vault
//! (`vault::actions::OpenVault`):
//!
//! | action | does |
//! |---|---|
//! | `BASES_LIST` | the vault's bases: path, name, folder, each view's name and type, the parse error of one that failed |
//! | `BASES_OPEN {path, view?}` | opens a base as a workspace view (`WORKSPACE_OPEN`, view `base`), when the list has a workspace; the base either way |
//! | `BASES_READ {path}` | one base: its filters, formulas, properties and views |
//! | `BASES_ROWS {path? \| source?, view?}` | runs one view (the first by default) of a base, or of a base's YAML (`source`, a note's ```` ```base ```` fence): its spec, its columns and grouped rows, and every column it could show |
//! | `BASES_SET_FILTERS {path, view?, clauses, join?, scope?}` | sets a view's filters (or, `scope: "base"`, the base's): expressions, `and` or `or` |
//! | `BASES_SET_SORT {path, view?, sort}` | sorts a view: `["status desc", "file.name"]` |
//! | `BASES_SET_GROUP {path, view?, property?}` | groups a view by a property, or not at all |
//! | `BASES_SET_PROPERTIES {path, view?, order}` | the columns a view shows, in order |
//! | `BASES_SET_FORMULA {path, name, expression}` | adds or changes a formula; an empty expression removes it |
//! | `BASES_SET_OPTION {path, view?, key, value}` | one of a view type's options (`value` JSON, or text; `null` removes it) |
//! | `BASES_ADD_VIEW {path, type, name?}` | adds a view of a type (`BASES_VIEW_TYPES`) |
//! | `BASES_VIEW_TYPES` | every view type: Bases' own (table, cards, list) and those loaded extensions add |
//!
//! Every edit writes the `.base` file through the vault
//! (`vault_obsidian::save_base`); a cell's edit is the note's property,
//! `VAULT_SET_PROPERTY`.

use std::sync::{Arc, Weak};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};
use vault::actions::OpenVault;
use vault::bases::{Conjunction, ParsedBase};
use vault_obsidian::Vault;

use crate::model::{self, At as _, Scope, ViewTypes, put};

/// The action ids, to name them from code.
pub mod ids {
    pub const LIST: &str = "BASES_LIST";
    pub const OPEN: &str = "BASES_OPEN";
    pub const READ: &str = "BASES_READ";
    pub const ROWS: &str = "BASES_ROWS";
    pub const SET_FILTERS: &str = "BASES_SET_FILTERS";
    pub const SET_SORT: &str = "BASES_SET_SORT";
    pub const SET_GROUP: &str = "BASES_SET_GROUP";
    pub const SET_PROPERTIES: &str = "BASES_SET_PROPERTIES";
    pub const SET_FORMULA: &str = "BASES_SET_FORMULA";
    pub const SET_OPTION: &str = "BASES_SET_OPTION";
    pub const ADD_VIEW: &str = "BASES_ADD_VIEW";
    pub const VIEW_TYPES: &str = "BASES_VIEW_TYPES";
}

/// The workspace view kind a base opens as.
pub const VIEW_KIND: &str = "base";

/// The pane id a base opens in (`base:<path>`): one pane per base.
#[must_use]
pub fn pane_id(path: &str) -> String {
    format!("base:{path}")
}

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Bases",
        method_name: id,
        display_name: name,
        description,
        category: "Bases",
        group: "Bases",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta(
    ids::LIST,
    "List bases",
    "The vault's bases (`.base` files): path, name, folder, each view's name and type, and why one that failed to parse did.",
);
static OPEN: ActionMeta = meta(
    ids::OPEN,
    "Open a base",
    "Opens the base at `path` as a workspace view (its pane `base:<path>`), on view `view` when given; answers with the base.",
);
static READ: ActionMeta = meta(
    ids::READ,
    "Read a base",
    "One base: its filters (`{join, clauses}`), formulas, properties and views (type, name, order, sort, groupBy, filters, limit, options).",
);
static ROWS: ActionMeta = meta(
    ids::ROWS,
    "Read a base's rows",
    "Runs one view of a base (`path`), or of a base's YAML (`source`); the first view unless `view` names one. Answers with the view, its columns, its rows in groups, and every column it could show.",
);
static SET_FILTERS: ActionMeta = meta(
    ids::SET_FILTERS,
    "Set a base's filters",
    "Sets a view's filters to `clauses` (expressions: `status != null`, `file.hasTag(\"song\")`), joined by `join` (`and`, the default, or `or`); `scope: \"base\"` sets the whole base's. No clauses: no filter.",
);
static SET_SORT: ActionMeta = meta(
    ids::SET_SORT,
    "Sort a base's view",
    "Sorts a view by `sort`, each a column with an optional direction: `[\"status desc\", \"file.name\"]`.",
);
static SET_GROUP: ActionMeta = meta(
    ids::SET_GROUP,
    "Group a base's view",
    "Groups a view's rows by `property` (any column), or not at all when it is left out.",
);
static SET_PROPERTIES: ActionMeta = meta(
    ids::SET_PROPERTIES,
    "Set a view's properties",
    "The columns a view shows, in order (`order`); the rest are hidden.",
);
static SET_FORMULA: ActionMeta = meta(
    ids::SET_FORMULA,
    "Set a base's formula",
    "Adds or changes formula `name` (`expression`: `price * qty`); an empty expression removes it. A view shows it as `formula.<name>`.",
);
static SET_OPTION: ActionMeta = meta(
    ids::SET_OPTION,
    "Set a view's option",
    "Sets option `key` of a view (one its type takes) to `value`: JSON, or else text; `null` removes it.",
);
static ADD_VIEW: ActionMeta = meta(
    ids::ADD_VIEW,
    "Add a view to a base",
    "Adds a view of type `type` (one of BASES_VIEW_TYPES) named `name` (the type's when left out), showing what the first view shows.",
);
static VIEW_TYPES: ActionMeta = meta(
    ids::VIEW_TYPES,
    "Base view types",
    "Every type a base's view may be: Bases' own (table, cards, list) and those loaded extensions add (`[[base_views]]`), each with its options and owner.",
);

#[derive(Facet)]
struct PathArgs {
    /// The `.base` file's vault path.
    path: String,
}

#[derive(Facet)]
struct OpenArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
}

#[derive(Facet)]
struct RowsArgs {
    /// The `.base` file's vault path.
    #[facet(default)]
    path: Option<String>,
    /// A base's YAML, run in place of a file's (a note's fence).
    #[facet(default)]
    source: Option<String>,
    /// The view by name (any case); the first when left out.
    #[facet(default)]
    view: Option<String>,
}

#[derive(Facet)]
struct FiltersArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
    clauses: Vec<String>,
    /// `and` (the default) or `or`.
    #[facet(default)]
    join: Option<String>,
    /// `view` (the default) or `base`.
    #[facet(default)]
    scope: Option<String>,
}

#[derive(Facet)]
struct SortArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
    sort: Vec<String>,
}

#[derive(Facet)]
struct GroupArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
    #[facet(default)]
    property: Option<String>,
}

#[derive(Facet)]
struct PropertiesArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
    order: Vec<String>,
}

#[derive(Facet)]
struct FormulaArgs {
    path: String,
    name: String,
    #[facet(default)]
    expression: Option<String>,
}

#[derive(Facet)]
struct OptionArgs {
    path: String,
    #[facet(default)]
    view: Option<String>,
    key: String,
    value: String,
}

#[derive(Facet)]
struct AddViewArgs {
    path: String,
    r#type: String,
    #[facet(default)]
    name: Option<String>,
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

/// The base at `path`, with a view to show.
fn base_at(vault: &Vault, path: &str) -> Result<ParsedBase, String> {
    match vault_obsidian::find_base(vault, path) {
        None => Err(format!("{path}: no such base")),
        Some(Err(e)) => Err(format!("{path}: {e}")),
        Some(Ok(b)) => Ok(model::with_a_view(b.clone())),
    }
}

fn list(vault: &Vault) -> Value {
    let mut bases: Vec<_> = vault.bases.iter().collect();
    bases.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Value::Array(
        bases
            .into_iter()
            .map(|b| {
                let mut out = json!({ "path": b.rel_path, "name": b.basename, "folder": b.folder });
                match &b.parsed {
                    Ok(p) => {
                        let views = model::with_a_view(p.clone())
                            .views
                            .iter()
                            .map(|v| json!({ "name": v.name, "type": v.kind.as_str() }))
                            .collect();
                        put(&mut out, "views", views);
                    }
                    Err(e) => put(&mut out, "error", json!(e)),
                }
                out
            })
            .collect(),
    )
}

/// Runs view `view` of `base` over the vault's notes.
pub(crate) fn rows(
    vault: &Vault,
    path: &str,
    base: &ParsedBase,
    view: Option<&str>,
) -> Result<Value, String> {
    let i = model::view_index(base, view)?;
    let all = vault_obsidian::vault_to_rows(vault);
    let spec = base.views.get(i).ok_or_else(|| "no such view".to_owned())?;
    let table = vault::bases::view_table(base, spec, &all);
    Ok(json!({
        "base": path,
        "views": base.views.iter().map(|v| json!({ "name": v.name, "type": v.kind.as_str() })).collect::<Vec<_>>(),
        "view": model::view_json(spec),
        "rows": table,
        "columns": model::columns(base, &all),
        "formulas": base.formulas.iter().map(|f| json!({ "name": f.name, "expression": f.expression })).collect::<Vec<_>>(),
        "filters": model::base_json(path, base).at("filters"),
    }))
}

fn rows_of(vault: &Vault, a: &RowsArgs) -> Result<Value, String> {
    match (a.path.as_deref(), a.source.as_deref()) {
        (_, Some(source)) => {
            let base = vault::bases::parse(source).map_err(|e| e.to_string())?;
            rows(
                vault,
                a.path.as_deref().unwrap_or_default(),
                &model::with_a_view(base),
                a.view.as_deref(),
            )
        }
        (Some(path), None) => rows(vault, path, &base_at(vault, path)?, a.view.as_deref()),
        (None, None) => Err("give a base's `path` or its `source`".to_owned()),
    }
}

/// Changes the base at `path` with `edit` and saves it; the base after.
fn edit(
    open: &OpenVault,
    path: &str,
    edit: impl FnOnce(&mut ParsedBase) -> Result<(), String>,
) -> Result<Value, String> {
    open.update(|vault, guard| {
        let mut base = base_at(vault, path)?;
        edit(&mut base)?;
        vault_obsidian::save_base(vault, path, &base, guard).map_err(|e| e.to_string())?;
        Ok(model::base_json(path, &base))
    })
}

fn join_of(join: Option<&str>) -> Result<Conjunction, String> {
    match join.map(str::to_ascii_lowercase).as_deref() {
        None | Some("" | "and" | "all") => Ok(Conjunction::And),
        Some("or" | "any") => Ok(Conjunction::Or),
        Some(other) => Err(format!("join {other:?}: `and` or `or`")),
    }
}

fn types_json(types: &ViewTypes) -> RawJson {
    raw(&serde_json::to_value(types.all()).unwrap_or_default())
}

/// Puts the `BASES_*` actions on `list`, over `vault`; `types` says which
/// view types extensions add (`model::ViewTypes::set_source`).
/// `BASES_OPEN` opens through the same list's `WORKSPACE_OPEN`.
///
/// # Errors
///
/// An id is taken.
pub fn register(
    list: &Arc<ActionList>,
    vault: &OpenVault,
    types: &ViewTypes,
) -> Result<(), ActionError> {
    let v = vault.clone();
    list.register_with(&LIST, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
        Ok::<_, String>(raw(&v.with(self::list)))
    })?;
    let v = vault.clone();
    list.register_with(
        &READ,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: PathArgs| {
            v.with(|vault| base_at(vault, &a.path).map(|b| raw(&model::base_json(&a.path, &b))))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &ROWS,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: RowsArgs| v.with(|vault| rows_of(vault, &a)).map(|j| raw(&j)),
    )?;
    let (v, weak): (OpenVault, Weak<ActionList>) = (vault.clone(), Arc::downgrade(list));
    list.register_async_with(
        &OPEN,
        Effect::Mutating,
        move |_: &ActionCaller, a: OpenArgs| {
            let found =
                v.with(|vault| base_at(vault, &a.path).map(|b| model::base_json(&a.path, &b)));
            let weak = weak.clone();
            async move {
                let mut base = found?;
                let opened = match weak.upgrade() {
                    Some(list) if list.info("WORKSPACE_OPEN").is_some() => {
                        open_pane(&list, &a.path, a.view.as_deref()).await?
                    }
                    _ => false,
                };
                put(&mut base, "opened", json!(opened));
                Ok::<_, String>(raw(&base))
            }
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_FILTERS,
        Effect::Mutating,
        move |_: &ActionCaller, a: FiltersArgs| {
            let join = join_of(a.join.as_deref())?;
            let scope = match a.scope.as_deref() {
                None | Some("" | "view") => Scope::View,
                Some("base") => Scope::Base,
                Some(other) => return Err(format!("scope {other:?}: `view` or `base`")),
            };
            edit(&v, &a.path, |b| {
                model::set_filters(b, a.view.as_deref(), scope, join, &a.clauses)
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_SORT,
        Effect::Mutating,
        move |_: &ActionCaller, a: SortArgs| {
            edit(&v, &a.path, |b| {
                model::set_sort(b, a.view.as_deref(), &a.sort)
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_GROUP,
        Effect::Mutating,
        move |_: &ActionCaller, a: GroupArgs| {
            edit(&v, &a.path, |b| {
                model::set_group(b, a.view.as_deref(), a.property.as_deref())
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_PROPERTIES,
        Effect::Mutating,
        move |_: &ActionCaller, a: PropertiesArgs| {
            edit(&v, &a.path, |b| {
                model::set_order(b, a.view.as_deref(), &a.order)
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_FORMULA,
        Effect::Mutating,
        move |_: &ActionCaller, a: FormulaArgs| {
            edit(&v, &a.path, |b| {
                model::set_formula(b, &a.name, a.expression.as_deref().unwrap_or_default())
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_OPTION,
        Effect::Mutating,
        move |_: &ActionCaller, a: OptionArgs| {
            let value =
                serde_json::from_str(&a.value).unwrap_or_else(|_| Value::String(a.value.clone()));
            edit(&v, &a.path, |b| {
                model::set_option(b, a.view.as_deref(), &a.key, value)
            })
            .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &ADD_VIEW,
        Effect::Mutating,
        move |_: &ActionCaller, a: AddViewArgs| {
            let mut added = String::new();
            let mut base = edit(&v, &a.path, |b| {
                added = model::add_view(b, a.name.as_deref().unwrap_or_default(), &a.r#type)?;
                Ok(())
            })?;
            put(&mut base, "added", json!(added));
            Ok::<_, String>(raw(&base))
        },
    )?;
    let t = types.clone();
    list.register_with(
        &VIEW_TYPES,
        Effect::ReadOnly,
        move |_: &ActionCaller, (): ()| Ok::<_, String>(types_json(&t)),
    )?;
    Ok(())
}

/// Opens the base at `path` in the workspace, beside the pane on show:
/// whether it was opened (false when it is open already).
async fn open_pane(list: &ActionList, path: &str, view: Option<&str>) -> Result<bool, String> {
    let layout = list
        .run_json_async("WORKSPACE_LAYOUT", b"")
        .await
        .map_err(|e| e.to_string())?;
    let layout: Value = serde_json::from_slice(&layout).unwrap_or(Value::Null);
    let active = layout
        .get("active")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let at = if active.is_empty() {
        json!({ "float": {} })
    } else {
        json!({ "beside": { "pane": active } })
    };
    let title = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    let state = json!({ "path": path, "view": view }).to_string();
    let args = json!({
        "view": VIEW_KIND,
        "id": pane_id(path),
        "state": state,
        "title": title,
        "at": at,
        "focus": true,
    });
    let out = list
        .run_json_async("WORKSPACE_OPEN", args.to_string().as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let out: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
    Ok(out.get("changed").and_then(Value::as_bool).unwrap_or(true))
}
