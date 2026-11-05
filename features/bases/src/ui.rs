//! Bases' views (feature `ui`): a base as a workspace view, its toolbar,
//! its view picker, and the view types that draw it.
//!
//! - [`view`]: the workspace view kind `base` (`actions::VIEW_KIND`); a
//!   pane's state is `{"path": …, "view"?: …}`, what `BASES_OPEN` opens.
//! - [`BaseView`]: one base (a file's, or a fence's YAML), its views as
//!   tabs, a `+` that adds one of any registered type, and its toolbar:
//!   filters, sort, group-by, properties (show, hide, reorder) and
//!   formulas. Every change runs a `BASES_*` action; a cell's edit runs
//!   `VAULT_SET_PROPERTY` on its note.
//! - Table, cards and list are Bases' own; a type an extension adds
//!   (`[[base_views]]`) is drawn by it, through
//!   `architect_extensions::embeds::BaseViewEmbed`, and is gone (a
//!   placeholder in its place) once the extension is off.
//! - [`Bases`]: the linked extension that draws a note's ```` ```base ````
//!   fence (its YAML, or one line naming a `.base` file) as an inline
//!   embed.
//!
//! The view reads the Action List in context (`Arc<ActionList>`), the one
//! the `BASES_*` and `VAULT_*` actions are on, and draws again when one of
//! their writes runs, from here or from an agent.

use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionList, EventKind};
use architect_extensions::contrib::BaseViewInput;
use architect_extensions::embeds::BaseViewEmbed;
use architect_extensions::host::Host;
use architect_extensions::interface::ExtensionUi;
use architect_extensions::linked::{Builder, Extension};
use architect_workspace::{View, use_pane};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::actions::{VIEW_KIND, ids};
use crate::model::{self, At as _, ViewType, put};

/// The workspace view a base opens in (kind `base`): more than one may be
/// open, one pane per base (`actions::pane_id`).
#[must_use]
pub fn view() -> View {
    View::new(VIEW_KIND, "Base", |_| rsx! { BasePane {} }).many()
}

/// What a base's pane keeps: the file, and the view on show.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PaneState {
    pub path: String,
    #[serde(default)]
    pub view: Option<String>,
}

impl PaneState {
    /// A pane's state text: JSON, or a bare path.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|_| Self {
            path: text.trim().to_owned(),
            view: None,
        })
    }
}

#[component]
fn BasePane() -> Element {
    let pane = use_pane();
    let state = PaneState::parse(&pane.state());
    if state.path.is_empty() {
        return rsx! { Note { text: "No base: open a .base file." } };
    }
    rsx! {
        BaseView { key: "{state.path}", path: state.path, view: state.view }
    }
}

/// The view types extensions loaded in `host` add (`[[base_views]]`).
#[must_use]
pub fn contributed_types(host: &Host) -> Vec<ViewType> {
    host.contributions()
        .base_views()
        .into_iter()
        .map(|by| ViewType {
            id: by.item.id.clone(),
            name: if by.item.name.is_empty() { by.item.id.clone() } else { by.item.name.clone() },
            icon: by.item.icon.clone(),
            owner: Some(by.owner),
            options: by
                .item
                .options
                .iter()
                .filter_map(|o| serde_json::to_value(o).ok())
                .collect(),
        })
        .collect()
}

/// Every view type now: Bases' own, then what loaded extensions add
/// (redrawn as one loads or unloads).
fn use_types() -> Vec<ViewType> {
    let mut out = model::builtin_types();
    if let Some(ui) = try_use_context::<ExtensionUi>() {
        let _ = ui.revision();
        for t in contributed_types(&ui.host()) {
            if !out.iter().any(|o| o.id.eq_ignore_ascii_case(&t.id)) {
                out.push(t);
            }
        }
    }
    out
}

/// Bumped when a write to the vault or a base runs on `list` (from here,
/// a script, an agent).
fn use_revision(list: Arc<ActionList>) -> Signal<u64> {
    let mut rev = use_signal(|| 0_u64);
    use_future(move || {
        let list = list.clone();
        async move {
            let (_, mut since) = list.events_since(0, usize::MAX, Duration::ZERO);
            loop {
                futures_timer::Delay::new(Duration::from_millis(400)).await;
                let (events, next) = list.events_since(since, 256, Duration::ZERO);
                since = next;
                let wrote = events.iter().any(|e| match &e.kind {
                    EventKind::Ran { id, ok } => *ok && writes(id),
                    _ => false,
                });
                if wrote {
                    let now = *rev.peek();
                    rev.set(now.wrapping_add(1));
                }
            }
        }
    });
    rev
}

/// Whether action `id` writes what a base shows.
fn writes(id: &str) -> bool {
    const READS: &[&str] = &[
        ids::LIST, ids::READ, ids::ROWS, ids::VIEW_TYPES, ids::OPEN,
        "VAULT_STATUS", "VAULT_LIST", "VAULT_READ", "VAULT_SEARCH", "VAULT_QUERY_BASE",
    ];
    (id.starts_with("BASES_") || id.starts_with("VAULT_")) && !READS.contains(&id)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    let out = list
        .run_json(id, args.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
}

/// Which toolbar panel is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    Filters,
    Sort,
    Group,
    Properties,
    Formulas,
    AddView,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// One base: the file at `path`, or the YAML in `source` (a fence's: its
/// toolbar is not shown, as there is no file to write), on view `view`
/// (the first when unset). `embedded` draws it inside a note, its
/// contributed views at most an embed's height.
// `rsx!` clones the panel's closure where its expansion ends; nothing here does.
#[allow(clippy::redundant_clone)]
#[component]
pub fn BaseView(
    #[props(default)] path: Option<String>,
    #[props(default)] source: Option<String>,
    #[props(default)] view: Option<String>,
    #[props(default)] embedded: bool,
) -> Element {
    let list = use_context::<Arc<ActionList>>();
    let rev = use_revision(list.clone());
    let mut chosen = use_signal(|| view.clone());
    let mut panel = use_signal(|| None::<Panel>);
    let mut problem = use_signal(|| None::<String>);
    let mut bump = use_signal(|| 0_u64);
    let types = use_types();
    let _ = (rev(), bump());
    let ask = |view: Option<String>| {
        let mut args = json!({ "view": view });
        if let Some(p) = &path {
            put(&mut args, "path", json!(p));
        }
        if let Some(s) = &source {
            put(&mut args, "source", json!(s));
        }
        run(&list, ids::ROWS, &args)
    };
    let data = match ask(chosen()) {
        Ok(d) => d,
        // The view it was on is gone (renamed, or the file changed): the
        // first.
        Err(_) if chosen.peek().is_some() => match ask(None) {
            Ok(d) => d,
            Err(e) => return rsx! { Note { text: e } },
        },
        Err(e) => return rsx! { Note { text: e } },
    };
    let file = path.clone().filter(|_| source.is_none());
    let spec = data.at("view").clone();
    let current = spec.at("name").as_str().unwrap_or_default().to_owned();
    let kind = spec.at("type").as_str().unwrap_or("table").to_owned();
    let views: Vec<(String, String)> = data.at("views")
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| {
                    (
                        v.at("name").as_str().unwrap_or_default().to_owned(),
                        v.at("type").as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let count: usize = data.at("rows").at("groups")
        .as_array()
        .map_or(0, |g| g.iter().map(|g| g.at("rows").as_array().map_or(0, Vec::len)).sum());
    // A write: its action, then everything drawn again.
    let write = {
        let list = list.clone();
        let file = file.clone();
        let current = current.clone();
        move |id: &'static str, mut args: Value| {
            if let Some(p) = &file {
                put(&mut args, "path", json!(p));
                if args.get("view").is_none() && id != ids::ADD_VIEW && id != ids::SET_FORMULA {
                    put(&mut args, "view", json!(current));
                }
            }
            match run(&list, id, &args) {
                Ok(out) => {
                    problem.set(None);
                    bump.with_mut(|n| *n = n.wrapping_add(1));
                    Some(out)
                }
                Err(e) => {
                    problem.set(Some(e));
                    None
                }
            }
        }
    };
    let set_cell = {
        let list = list.clone();
        Callback::new(move |(note, key, value): (String, String, String)| {
            match run(&list, "VAULT_SET_PROPERTY", &json!({ "path": note, "key": key, "value": value })) {
                Ok(_) => {
                    problem.set(None);
                    bump.with_mut(|n| *n = n.wrapping_add(1));
                }
                Err(e) => problem.set(Some(e)),
            }
        })
    };
    let input = BaseViewInput {
        base: path.clone().unwrap_or_default(),
        view: spec,
        rows: data.at("rows").clone(),
    };
    let contributed = types.iter().find(|t| t.id.eq_ignore_ascii_case(&kind) && t.owner.is_some()).cloned();
    let body = match kind.as_str() {
        "table" => rsx! { Table { data: data.clone(), on_cell: set_cell, sortable: file.is_some(), on_sort: {
            let mut write = write.clone();
            move |sort: Vec<String>| { let _ = write(ids::SET_SORT, json!({ "sort": sort })); }
        } } },
        "cards" | "gallery" => rsx! { Cards { data: data.clone(), on_cell: set_cell } },
        "list" => rsx! { List { data: data.clone() } },
        other => match contributed {
            Some(_) => rsx! {
                BaseViewEmbed {
                    kind: other.to_owned(),
                    input,
                    max_height: if embedded { 480 } else { u32::MAX },
                    fallback: rsx! { Note { text: format!("No loaded extension draws {other} views.") } },
                }
            },
            None => rsx! { Note { text: format!("No loaded extension adds the view type {other:?}: turn on the extension that does, or pick another view.") } },
        },
    };
    let lit = |on: bool| if on { "var(--accent, var(--muted))" } else { "transparent" };
    rsx! {
        div { class: "base", "data-base": "{path.clone().unwrap_or_default()}",
            style: "display: flex; flex-direction: column; gap: 8px; min-height: 0; color: var(--foreground); font-size: 13px; {pad(embedded)}",
            // The view picker: the base's views, and `+` for a new one of any type.
            div { class: "base-views", style: "display: flex; flex-wrap: wrap; align-items: center; gap: 4px;",
                for (name, ty) in views.clone() {
                    button { key: "{name}", class: "lp", "data-view": "{name}", "data-type": "{ty}",
                        "aria-current": if name == current { "page" } else { "false" },
                        style: "{CHIP} background: {lit(name == current)}; font-weight: {weight(name == current)};",
                        onclick: move |_| { chosen.set(Some(name.clone())); panel.set(None); },
                        {view_label(&types, &ty, &name)}
                    }
                }
                if file.is_some() {
                    button { class: "lp", "data-tool": "add-view", "aria-label": "Add a view", style: "{CHIP}",
                        onclick: move |_| panel.set(if panel() == Some(Panel::AddView) { None } else { Some(Panel::AddView) }),
                        "+"
                    }
                }
                div { style: "flex: 1;" }
                span { style: "color: var(--muted-foreground);", "{count} results" }
            }
            if file.is_some() {
                div { class: "base-toolbar", style: "display: flex; flex-wrap: wrap; gap: 4px;",
                    for (p, label) in [(Panel::Filters, "Filter"), (Panel::Sort, "Sort"), (Panel::Group, "Group"), (Panel::Properties, "Properties"), (Panel::Formulas, "Formulas")] {
                        button { key: "{label}", class: "lp", "data-tool": "{label.to_lowercase()}",
                            style: "{CHIP} background: {lit(panel() == Some(p))};",
                            onclick: move |_| panel.set(if panel() == Some(p) { None } else { Some(p) }),
                            "{label}"
                        }
                    }
                }
            }
            if let Some(p) = panel() {
                div { class: "base-panel", "data-panel": "{p:?}",
                    style: "display: flex; flex-direction: column; gap: 6px; padding: 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--card);",
                    {panel_body(p, &data, &types, write, move |name: String| { chosen.set(Some(name)); panel.set(None); })}
                }
            }
            if let Some(e) = problem() {
                div { class: "base-problem", style: "color: var(--destructive, #c33);", "{e}" }
            }
            div { style: "display: flex; flex-direction: column; min-height: 0; flex: 1; overflow: auto;", {body} }
        }
    }
}

/// The sort after `column`'s header is pressed: by it ascending, then
/// descending, then the sort without it.
fn header_sort(sorted: &[(String, bool)], column: &str) -> Vec<String> {
    match sorted.first() {
        Some((p, false)) if p == column => vec![format!("{column} desc")],
        Some((p, true)) if p == column => sorted
            .iter()
            .skip(1)
            .map(|(p, d)| if *d { format!("{p} desc") } else { p.clone() })
            .collect(),
        _ => vec![column.to_owned()],
    }
}

const CHIP: &str = "min-height: 26px; padding: 0 10px; border-radius: 6px; border: 1px solid var(--border); \
                    background: transparent; color: var(--foreground); font-size: 12px; cursor: pointer;";

const fn pad(embedded: bool) -> &'static str {
    if embedded { "padding: 8px; border: 1px solid var(--border); border-radius: 8px;" } else { "flex: 1; padding: 16px 20px;" }
}

const fn weight(on: bool) -> u32 {
    if on { 650 } else { 450 }
}

/// A view's tab: its name, with its type's when the name does not say it.
fn view_label(types: &[ViewType], kind: &str, name: &str) -> String {
    let ty = type_label(types, kind);
    if ty.eq_ignore_ascii_case(name) || kind.eq_ignore_ascii_case(name) {
        name.to_owned()
    } else {
        format!("{name} · {ty}")
    }
}

fn type_label(types: &[ViewType], id: &str) -> String {
    types
        .iter()
        .find(|t| t.id.eq_ignore_ascii_case(id))
        .map_or_else(|| id.to_owned(), |t| t.name.clone())
}

/// A toolbar panel's rows.
#[allow(clippy::too_many_lines)]
fn panel_body(
    p: Panel,
    data: &Value,
    types: &[ViewType],
    write: impl FnMut(&'static str, Value) -> Option<Value> + Clone + 'static,
    choose: impl FnMut(String) + Clone + 'static,
) -> Element {
    let spec = &data.at("view");
    let columns = strings(&data.at("columns"));
    match p {
        Panel::Filters => {
            let clauses = strings(&spec.at("filters").at("clauses"));
            let join = spec.at("filters").at("join").as_str().unwrap_or("and").to_owned();
            let other = if join == "or" { "and" } else { "or" };
            let base_clauses = strings(&data.at("filters").at("clauses"));
            rsx! {
                if !base_clauses.is_empty() {
                    div { style: "color: var(--muted-foreground);", "The whole base: {base_clauses.join(\", \")}" }
                }
                div { style: "display: flex; gap: 6px; align-items: center;",
                    span { "This view's rows, where" }
                    button { class: "lp", "data-tool": "join", style: "{CHIP}",
                        onclick: { let mut w = write; let clauses = clauses; let other = other.to_owned();
                            move |_| { let _ = w(ids::SET_FILTERS, json!({ "clauses": clauses, "join": other })); } },
                        if join == "or" { "any of" } else { "all of" }
                    }
                }
                for (i, c) in clauses.clone().into_iter().enumerate() {
                    div { key: "{i}-{c}", style: "display: flex; gap: 6px;",
                        Field { class: "base-clause", value: c, grow: true,
                            on_commit: { let mut w = write.clone(); let clauses = clauses.clone(); let join = join.clone();
                                move |v: String| { let mut all = clauses.clone(); if let Some(slot) = all.get_mut(i) { *slot = v; } let _ = w(ids::SET_FILTERS, json!({ "clauses": all, "join": join })); } },
                        }
                        button { class: "lp", "aria-label": "Remove the filter", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); let mut all = clauses.clone(); let join = join.clone();
                                move |_| { all.remove(i); let _ = w(ids::SET_FILTERS, json!({ "clauses": all, "join": join })); } },
                            "✕"
                        }
                    }
                }
                Field { class: "base-clause-new", placeholder: "Add a filter: status != null, file.hasTag(\"song\")… then Enter", clear: true,
                    on_commit: { let mut w = write.clone(); let clauses = clauses.clone(); let join = join;
                        move |text: String| {
                            if !text.trim().is_empty() {
                                let mut all = clauses.clone();
                                all.push(text);
                                let _ = w(ids::SET_FILTERS, json!({ "clauses": all, "join": join }));
                            }
                        } },
                }
            }
        }
        Panel::Sort => {
            let keys: Vec<(String, bool)> = spec.at("sort")
                .as_array()
                .map(|a| a.iter().map(|k| (k.at("property").as_str().unwrap_or_default().to_owned(), k.at("direction") == "DESC")).collect())
                .unwrap_or_default();
            let words = |keys: &[(String, bool)]| -> Vec<String> {
                keys.iter().map(|(p, d)| if *d { format!("{p} desc") } else { p.clone() }).collect()
            };
            rsx! {
                if keys.is_empty() { div { style: "color: var(--muted-foreground);", "Not sorted: by name." } }
                for (i, (prop, desc)) in keys.clone().into_iter().enumerate() {
                    div { key: "{prop}", style: "display: flex; gap: 6px; align-items: center;",
                        span { style: "flex: 1;", "{prop}" }
                        button { class: "lp", "data-tool": "direction", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); let mut k = keys.clone();
                                move |_| { if let Some(key) = k.get_mut(i) { key.1 = !key.1; } let _ = w(ids::SET_SORT, json!({ "sort": words(&k) })); } },
                            if desc { "Z → A" } else { "A → Z" }
                        }
                        button { class: "lp", "aria-label": "Remove the sort", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); let mut k = keys.clone();
                                move |_| { k.remove(i); let _ = w(ids::SET_SORT, json!({ "sort": words(&k) })); } },
                            "✕"
                        }
                    }
                }
                div { style: "color: var(--muted-foreground);", "Sort by" }
                Chips { items: columns.iter().filter(|c| !keys.iter().any(|(k, _)| k == *c)).cloned().collect::<Vec<_>>(), on_pick: {
                    let mut w = write; let keys = keys;
                    move |c: String| { let mut k = keys.clone(); k.push((c, false)); let _ = w(ids::SET_SORT, json!({ "sort": words(&k) })); }
                } }
            }
        }
        Panel::Group => {
            let now = spec.at("groupBy").as_str().unwrap_or_default().to_owned();
            rsx! {
                div { style: "display: flex; gap: 6px; align-items: center;",
                    span { if now.is_empty() { "Not grouped." } else { "Grouped by {now}." } }
                    if !now.is_empty() {
                        button { class: "lp", "data-tool": "ungroup", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); move |_| { let _ = w(ids::SET_GROUP, json!({})); } },
                            "No grouping"
                        }
                    }
                }
                Chips { items: columns.iter().filter(|c| **c != now).cloned().collect::<Vec<_>>(), on_pick: {
                    let mut w = write;
                    move |c: String| { let _ = w(ids::SET_GROUP, json!({ "property": c })); }
                } }
            }
        }
        Panel::Properties => {
            let shown = strings(&spec.at("order"));
            rsx! {
                for (i, c) in shown.clone().into_iter().enumerate() {
                    div { key: "{c}", class: "base-property", "data-property": "{c}", style: "display: flex; gap: 6px; align-items: center;",
                        span { style: "flex: 1;", "{c}" }
                        button { class: "lp", "aria-label": "Move up", style: "{CHIP}", disabled: i == 0,
                            onclick: { let mut w = write.clone(); let mut o = shown.clone();
                                move |_| { if i > 0 { o.swap(i, i - 1); let _ = w(ids::SET_PROPERTIES, json!({ "order": o })); } } },
                            "↑"
                        }
                        button { class: "lp", "aria-label": "Move down", style: "{CHIP}", disabled: i + 1 == shown.len(),
                            onclick: { let mut w = write.clone(); let mut o = shown.clone();
                                move |_| { if i + 1 < o.len() { o.swap(i, i + 1); let _ = w(ids::SET_PROPERTIES, json!({ "order": o })); } } },
                            "↓"
                        }
                        button { class: "lp", "aria-label": "Hide", "data-tool": "hide", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); let mut o = shown.clone();
                                move |_| { o.remove(i); let _ = w(ids::SET_PROPERTIES, json!({ "order": o })); } },
                            "Hide"
                        }
                    }
                }
                div { style: "color: var(--muted-foreground);", "Show" }
                Chips { items: columns.iter().filter(|c| !shown.contains(c)).cloned().collect::<Vec<_>>(), on_pick: {
                    let mut w = write; let shown = shown;
                    move |c: String| { let mut o = shown.clone(); o.push(c); let _ = w(ids::SET_PROPERTIES, json!({ "order": o })); }
                } }
            }
        }
        Panel::Formulas => {
            let formulas: Vec<(String, String)> = data.at("formulas")
                .as_array()
                .map(|a| a.iter().map(|f| (f.at("name").as_str().unwrap_or_default().to_owned(), f.at("expression").as_str().unwrap_or_default().to_owned())).collect())
                .unwrap_or_default();
            let mut name = use_signal(String::new);
            rsx! {
                for (n, x) in formulas {
                    div { key: "{n}", style: "display: flex; gap: 6px; align-items: center;",
                        span { style: "min-width: 80px;", "formula.{n} =" }
                        Field { value: x, grow: true,
                            on_commit: { let mut w = write.clone(); let n = n.clone();
                                move |v: String| { let _ = w(ids::SET_FORMULA, json!({ "name": n, "expression": v })); } },
                        }
                        button { class: "lp", "aria-label": "Remove the formula", style: "{CHIP}",
                            onclick: { let mut w = write.clone(); let n = n;
                                move |_| { let _ = w(ids::SET_FORMULA, json!({ "name": n, "expression": "" })); } },
                            "✕"
                        }
                    }
                }
                div { style: "display: flex; gap: 6px;",
                    input { class: "base-formula-name", placeholder: "Name", value: "{name}", style: "{FIELD} width: 100px;",
                        oninput: move |e: FormEvent| name.set(e.value()) }
                    Field { class: "base-formula-new", placeholder: "Expression: price * qty — then Enter", grow: true, clear: true,
                        on_commit: { let mut w = write;
                            move |v: String| {
                                let n = name.peek().clone();
                                if !n.trim().is_empty() && w(ids::SET_FORMULA, json!({ "name": n, "expression": v })).is_some() {
                                    name.set(String::new());
                                }
                            } },
                    }
                }
            }
        }
        Panel::AddView => rsx! {
            div { style: "color: var(--muted-foreground);", "Add a view of type" }
            div { class: "base-types", style: "display: flex; flex-wrap: wrap; gap: 4px;",
                for t in types.iter() {
                    button { key: "{t.id}", class: "lp", "data-type": "{t.id}", style: "{CHIP}",
                        title: t.owner.clone().map_or_else(|| "Bases".to_owned(), |o| format!("From {o}")),
                        onclick: { let mut w = write.clone(); let mut choose = choose.clone(); let id = t.id.clone();
                            move |_| {
                                if let Some(out) = w(ids::ADD_VIEW, json!({ "type": id })) {
                                    if let Some(name) = out.at("added").as_str() { choose(name.to_owned()); }
                                }
                            } },
                        "{t.name}"
                        if let Some(o) = &t.owner { span { style: "color: var(--muted-foreground);", " · {o}" } }
                    }
                }
            }
        },
    }
}

const FIELD: &str = "height: 28px; box-sizing: border-box; padding: 0 8px; border-radius: 6px; border: 1px solid var(--input, var(--border)); \
                     background: var(--background); color: var(--foreground); font-size: 12px;";

/// A row of columns to pick one from.
#[component]
fn Chips(items: Vec<String>, on_pick: EventHandler<String>) -> Element {
    rsx! {
        div { style: "display: flex; flex-wrap: wrap; gap: 4px;",
            for c in items {
                button { key: "{c}", class: "lp", "data-pick": "{c}", style: "{CHIP}",
                    onclick: { let c = c; move |_| on_pick.call(c.clone()) },
                    "{c}"
                }
            }
        }
    }
}

/// A view's groups: `(label, rows)`.
fn groups(data: &Value) -> Vec<(String, Vec<Value>)> {
    data.at("rows").at("groups")
        .as_array()
        .map(|a| {
            a.iter()
                .map(|g| (g.at("label").as_str().unwrap_or_default().to_owned(), g.at("rows").as_array().cloned().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default()
}

/// A cell's value as text.
fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn grouped(data: &Value) -> bool {
    data.at("view").at("groupBy").as_str().is_some_and(|g| !g.is_empty())
}

/// Bases' table: a header per column (pressed, sorts by it), a row per
/// note, a group's label over its rows; a note's own property is edited
/// in its cell.
#[component]
fn Table(
    data: Value,
    on_cell: Callback<(String, String, String)>,
    sortable: bool,
    on_sort: EventHandler<Vec<String>>,
) -> Element {
    let columns = strings(&data.at("rows").at("columns"));
    let sorted: Vec<(String, bool)> = data.at("view").at("sort")
        .as_array()
        .map(|a| a.iter().map(|k| (k.at("property").as_str().unwrap_or_default().to_owned(), k.at("direction") == "DESC")).collect())
        .unwrap_or_default();
    let width = columns.len().max(1);
    let grid = format!("display: grid; grid-template-columns: repeat({width}, minmax(120px, 1fr)); gap: 0;");
    let grouped = grouped(&data);
    rsx! {
        div { class: "base-table", style: "display: flex; flex-direction: column; min-width: fit-content;",
            div { style: "{grid} border-bottom: 1px solid var(--border); font-weight: 650; color: var(--muted-foreground);",
                for c in columns.clone() {
                    div { key: "{c}", class: "base-th", "data-column": "{c}", style: "padding: 6px 8px; cursor: pointer;",
                        onclick: { let c = c; let sorted = sorted.clone();
                            move |_| if sortable { on_sort.call(header_sort(&sorted, &c)); } },
                        "{c}"
                        {sorted.iter().find(|(p, _)| *p == c).map(|(_, d)| if *d { " ↓" } else { " ↑" }).unwrap_or_default()}
                    }
                }
            }
            for (label, rows) in groups(&data) {
                if grouped {
                    div { key: "g-{label}", class: "base-group", "data-group": "{label}",
                        style: "padding: 10px 8px 4px; font-weight: 700;",
                        if label.is_empty() { "(none)" } else { "{label}" }
                        span { style: "color: var(--muted-foreground); font-weight: 450;", " {rows.len()}" }
                    }
                }
                for row in rows {
                    div { key: "{label}-{row.at(\"path\")}", class: "base-row", "data-path": "{text(&row.at(\"path\"))}",
                        style: "{grid} border-bottom: 1px solid var(--border);",
                        for c in columns.clone() {
                            Cell { key: "{c}", row: row.clone(), column: c, on_cell }
                        }
                    }
                }
            }
        }
    }
}

/// One cell: an input for a note's own property, its value as text for
/// the rest.
#[component]
fn Cell(row: Value, column: String, on_cell: Callback<(String, String, String)>) -> Element {
    let value = text(&row.at("values").at(&column));
    let Some(key) = model::editable(&column).map(str::to_owned) else {
        return rsx! { div { class: "base-td", style: "padding: 6px 8px; overflow: hidden;", "{value}" } };
    };
    let note = text(&row.at("path"));
    rsx! {
        Field { class: "base-cell", value: value.clone(), cell: true, grow: true,
            on_commit: move |now: String| {
                if now != value {
                    on_cell.call((note.clone(), key.clone(), now));
                }
            },
        }
    }
}

/// A text field that commits on Enter or when it loses the focus: what
/// it holds as typed, `on_commit` given it then. `clear` empties it after
/// (a field that adds); `cell` draws it as a table cell.
#[component]
fn Field(
    #[props(default)] value: String,
    on_commit: EventHandler<String>,
    #[props(default)] class: String,
    #[props(default)] placeholder: String,
    #[props(default)] grow: bool,
    #[props(default)] clear: bool,
    #[props(default)] cell: bool,
) -> Element {
    let mut draft = use_signal(|| value.clone());
    let mut shown = use_signal(|| value.clone());
    // The value changed underneath (a write, an agent): show it.
    if *shown.peek() != value {
        shown.set(value.clone());
        draft.set(value);
    }
    let commit = move || {
        let now = draft.peek().clone();
        if now != *shown.peek() || clear {
            if clear && now.trim().is_empty() {
                return;
            }
            on_commit.call(now);
            if clear {
                draft.set(String::new());
            }
        }
    };
    let look = if cell {
        "height: 30px; box-sizing: border-box; padding: 0 8px; border: none; background: transparent; \
         color: var(--foreground); font-size: 13px; min-width: 0;"
    } else {
        FIELD
    };
    let flex = if grow { "flex: 1;" } else { "" };
    rsx! {
        input { class: "{class}", value: "{draft}", placeholder: "{placeholder}", style: "{look} {flex}",
            oninput: move |e: FormEvent| draft.set(e.value()),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Enter) {
                    let mut commit = commit;
                    commit();
                }
            },
            onblur: move |_| {
                let mut commit = commit;
                commit();
            },
        }
    }
}

/// Bases' cards: a card per note, its name and the view's other
/// properties; a group's label over its cards.
#[component]
fn Cards(data: Value, on_cell: Callback<(String, String, String)>) -> Element {
    let columns: Vec<String> = strings(&data.at("rows").at("columns")).into_iter().filter(|c| c != "file.name").collect();
    let grouped = grouped(&data);
    rsx! {
        div { class: "base-cards", style: "display: flex; flex-direction: column; gap: 10px;",
            for (label, rows) in groups(&data) {
                div { key: "c-{label}", style: "display: flex; flex-direction: column; gap: 6px;",
                if grouped {
                    div { class: "base-group", "data-group": "{label}", style: "font-weight: 700;",
                        if label.is_empty() { "(none)" } else { "{label}" }
                    }
                }
                div { style: "display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 10px;",
                    for row in rows {
                        div { key: "{row.at(\"path\")}", class: "base-card", "data-path": "{text(&row.at(\"path\"))}",
                            style: "display: flex; flex-direction: column; gap: 4px; padding: 12px; border-radius: 10px; border: 1px solid var(--border); background: var(--card);",
                            div { style: "font-weight: 650;", "{text(&row.at(\"name\"))}" }
                            for c in columns.clone() {
                                div { key: "{c}", style: "display: flex; gap: 6px; align-items: center; color: var(--muted-foreground);",
                                    span { style: "min-width: 60px;", "{c}" }
                                    Cell { row: row.clone(), column: c, on_cell }
                                }
                            }
                        }
                    }
                }
                }
            }
        }
    }
}

/// Bases' list: a line per note, its name and the view's other
/// properties after it.
#[component]
fn List(data: Value) -> Element {
    let columns: Vec<String> = strings(&data.at("rows").at("columns")).into_iter().filter(|c| c != "file.name").collect();
    let grouped = grouped(&data);
    rsx! {
        div { class: "base-list", style: "display: flex; flex-direction: column; gap: 2px;",
            for (label, rows) in groups(&data) {
                if grouped {
                    div { key: "g-{label}", class: "base-group", "data-group": "{label}", style: "padding-top: 8px; font-weight: 700;",
                        if label.is_empty() { "(none)" } else { "{label}" }
                    }
                }
                for row in rows {
                    div { key: "{label}-{row.at(\"path\")}", class: "base-line", "data-path": "{text(&row.at(\"path\"))}",
                        style: "display: flex; gap: 10px; align-items: baseline; padding: 4px 0;",
                        span { "• {text(&row.at(\"name\"))}" }
                        for c in columns.clone() {
                            span { key: "{c}", style: "color: var(--muted-foreground);",
                                {text(&row.at("values").at(&c))}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Note(text: String) -> Element {
    rsx! { div { class: "base-note", style: "padding: 16px; color: var(--muted-foreground); font-size: 13px;", "{text}" } }
}

/// Bases as a linked extension (`features/bases/extension.toml`, id
/// `architect.bases`): it claims ```` ```base ```` fences and draws each as
/// an inline base. Link it in with `Extensions::with_linked(&Bases)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bases;

/// The extension's id.
pub const ID: &str = "architect.bases";

impl Extension for Bases {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("fence", FenceView);
    }
}

/// A ```` ```base ```` fence: its YAML run over the vault, or, when it is
/// one line naming a `.base` file, that file's base.
#[component]
fn FenceView() -> Element {
    let Some(input) = architect_extensions::linked::use_embed() else {
        return rsx! {};
    };
    let body = input.source.trim().to_owned();
    let one_file = !body.contains('\n')
        && std::path::Path::new(&body)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("base"));
    if one_file {
        rsx! { BaseView { key: "{body}", path: body, embedded: true } }
    } else {
        rsx! { BaseView { source: body, embedded: true } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panes_state_is_json_or_a_path() {
        assert_eq!(PaneState::parse(r#"{"path":"B.base","view":"All"}"#).view.as_deref(), Some("All"));
        assert_eq!(PaneState::parse("B.base").path, "B.base");
        assert!(writes("BASES_SET_SORT") && writes("VAULT_SET_PROPERTY"));
        let s = |v: &[(&str, bool)]| v.iter().map(|(p, d)| ((*p).to_owned(), *d)).collect::<Vec<_>>();
        assert_eq!(header_sort(&[], "a"), ["a"]);
        assert_eq!(header_sort(&s(&[("a", false)]), "a"), ["a desc"]);
        assert_eq!(header_sort(&s(&[("a", true), ("b", true)]), "a"), ["b desc"]);
        assert!(!writes(ids::ROWS) && !writes("VAULT_READ") && !writes("WORKSPACE_OPEN"));
    }
}
