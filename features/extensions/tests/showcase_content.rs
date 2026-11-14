//! The extension showcase's content examples (#710, `examples/showcase`),
//! each loaded from its own folder as a development copy, run against
//! stand-ins for the vault's own actions with the shapes those answer
//! (`features/bases/src/actions.rs` `BASES_ROWS`, `TAGS_NOTES`,
//! `LINKS_BACK`, `VAULT_LIST`, `VAULT_CREATE`):
//!
//! - the Openers sampler (C6, C7, C8): a palette source that finds a
//!   capability note by its id and opens it, a URL claim, a project type
//!   with its template, a capture target that makes a note, and a base
//!   view type drawn by a page;
//! - Dataview-lite (C10, H3): a `showcase-query` fence claimed, its query
//!   answered from a base's rows, a tag's notes and a note's backlinks;
//! - Chart (U4): a `showcase-chart` fence claimed, a base's rows counted by
//!   a property.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use architect_extensions::contrib::Claim;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostui::FakeUi;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use facet::Facet;
use futures::executor::block_on;
use serde_json::{Value, json};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

fn load(list: &ActionList, name: &str) -> Host {
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(FakeUi::new());
    let package = Package::read(&example(name)).unwrap();
    host.load(&package, Origin::Dev).unwrap();
    host
}

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: id,
        display_name: id,
        description: "",
        category: "Vault",
        group: "Vault",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

#[derive(Facet)]
struct ListArgs {
    #[facet(default)]
    folder: Option<String>,
}

#[derive(Facet)]
struct PathArgs {
    #[facet(default)]
    path: Option<String>,
}

#[derive(Facet)]
struct TagArgs {
    tag: String,
}

#[derive(Facet)]
struct CreateArgs {
    path: String,
    #[facet(default)]
    body: Option<String>,
    #[facet(default)]
    properties: Option<String>,
}

static LIST: ActionMeta = meta("VAULT_LIST");
static CREATE: ActionMeta = meta("VAULT_CREATE");
static ROWS: ActionMeta = meta("BASES_ROWS");
static TAGGED: ActionMeta = meta("TAGS_NOTES");
static BACK: ActionMeta = meta("LINKS_BACK");

/// The showcase vault's notes, as `VAULT_LIST` answers.
const NOTES: &[(&str, &str)] = &[
    ("C Contributions/C6 Openers.md", "C Contributions"),
    (
        "C Contributions/C1 Actions everywhere.md",
        "C Contributions",
    ),
    ("H Host services/H3 Vault queries.md", "H Host services"),
    ("Samples/Tables and lists.md", "Samples"),
];

/// `BASES_ROWS` on `Bases/Showcase tasks.base`: a view grouped by status.
fn rows() -> Value {
    let row = |name: &str, status: &str, priority: &str| {
        json!({
            "path": format!("Samples/Tasks/{name}.md"), "name": name,
            "values": { "file.name": name, "status": status, "priority": priority },
            "formulas": {},
        })
    };
    json!({
        "base": "Bases/Showcase tasks.base",
        "rows": {
            "name": "Tasks", "kind": "table",
            "columns": ["file.name", "status", "priority"],
            "groups": [
                { "label": "done", "rows": [row("Ship the vault", "done", "normal")] },
                { "label": "open", "rows": [
                    row("Write the release notes", "open", "high"),
                    row("Record the walkthrough", "open", "normal"),
                ] },
            ],
        },
    })
}

/// The vault's actions these examples run, answering as the app's do;
/// what `VAULT_CREATE` was asked, kept.
fn vault(list: &ActionList) -> Arc<Mutex<Vec<Value>>> {
    list.register_with(&LIST, Effect::ReadOnly, |_: &ActionCaller, _: ListArgs| {
        Ok::<_, String>(raw(&Value::Array(
            NOTES
                .iter()
                .map(|(path, folder)| json!({ "path": path, "title": path, "folder": folder }))
                .collect(),
        )))
    })
    .unwrap();
    let made: Arc<Mutex<Vec<Value>>> = Arc::default();
    let seen = Arc::clone(&made);
    list.register_with(
        &CREATE,
        Effect::Mutating,
        move |_: &ActionCaller, a: CreateArgs| {
            seen.lock()
                .unwrap()
                .push(json!({ "path": a.path, "body": a.body, "properties": a.properties }));
            Ok::<_, String>(raw(&json!({ "path": a.path })))
        },
    )
    .unwrap();
    list.register_with(&ROWS, Effect::ReadOnly, |_: &ActionCaller, a: PathArgs| {
        if a.path.as_deref() != Some("Bases/Showcase tasks.base") {
            return Err(format!("{:?}: no such base", a.path));
        }
        Ok(raw(&rows()))
    })
    .unwrap();
    list.register_with(&TAGGED, Effect::ReadOnly, |_: &ActionCaller, a: TagArgs| {
        assert_eq!(a.tag, "task");
        Ok::<_, String>(raw(&json!([
            { "path": "Samples/Tasks/Ship the vault.md", "tags": ["task", "showcase"] },
        ])))
    })
    .unwrap();
    list.register_with(&BACK, Effect::ReadOnly, |_: &ActionCaller, _: PathArgs| {
        Ok::<_, String>(raw(&json!(["Samples/Dataview-lite demo.md"])))
    })
    .unwrap();
    made
}

// ---- the Openers sampler ------------------------------------------------

#[test]
fn the_openers_sampler_finds_opens_and_adds_its_types() {
    let list = ActionList::new();
    let made = vault(&list);
    let host = load(&list, "openers-sampler");
    let opened: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = Arc::clone(&opened);
    host.set_path_opener(move |path| {
        seen.lock().unwrap().push(path.to_owned());
        Ok(json!({ "opened_by": "app", "path": path }))
    });
    let reg = host.contributions().clone();

    // C6: its palette source, `show:`, finds a capability note by its id.
    let sources = reg.palette_sources();
    let show = sources.iter().find(|s| s.item.id == "show").expect("show:");
    assert_eq!(show.item.action, "SHOWCASE_OPENERS_FIND");
    let found = run(&list, "SHOWCASE_OPENERS_FIND", &json!({ "query": "c6" })).unwrap();
    let items = found["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{found}");
    assert_eq!(items[0]["id"], "C Contributions/C6 Openers.md");
    // A sample note is not a capability note.
    let all = run(&list, "SHOWCASE_OPENERS_FIND", &json!({ "query": "" })).unwrap();
    assert_eq!(all["items"].as_array().unwrap().len(), 3);
    // Picking it opens the note, as the app opens one.
    run(
        &list,
        "SHOWCASE_OPENERS_GO",
        &json!({ "id": "C Contributions/C6 Openers.md" }),
    )
    .unwrap();

    // C6: a link it claims opens that capability's note; another is not
    // its.
    assert_eq!(reg.open_url("https://showcase.example/H3").len(), 1);
    assert!(reg.open_url("https://example.com/H3").is_empty());
    run(
        &list,
        "SHOWCASE_OPENERS_URL",
        &json!({ "url": "https://showcase.example/H3" }),
    )
    .unwrap();
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        [
            "C Contributions/C6 Openers.md",
            "H Host services/H3 Vault queries.md"
        ]
    );

    // C7: a project type with its template, and a capture target that
    // makes a note in Inbox/.
    let kind = reg.project_type("showcase").expect("the Showcase type");
    assert_eq!(kind.item.properties.len(), 2);
    assert!(
        kind.item
            .template_text
            .as_deref()
            .is_some_and(|t| t.starts_with("# Showcase"))
    );
    assert!(
        reg.capture_targets()
            .iter()
            .any(|t| t.item.id == "capability")
    );
    let out = run(
        &list,
        "SHOWCASE_OPENERS_CAPTURE",
        &json!({ "title": "Hover previews", "body": "Obsidian's page preview" }),
    )
    .unwrap();
    assert_eq!(out["path"], "Inbox/Hover previews.md");
    let body = made.lock().unwrap()[0]["body"].clone();
    assert!(body.as_str().unwrap().contains("Obsidian's page preview"));

    // C8: a base view type, drawn by its page.
    let checklist = reg.base_view("checklist").expect("the Checklist view");
    assert_eq!(checklist.item.page.as_deref(), Some("pages/checklist.tsx"));
    assert!(example("openers-sampler/pages/checklist.tsx").is_file());
}

// ---- Dataview-lite ------------------------------------------------------

#[test]
fn dataview_lite_claims_its_fence_and_answers_each_kind_of_query() {
    let list = ActionList::new();
    vault(&list);
    let host = load(&list, "dataview-lite");

    // C10: the fence is its.
    let claimed = host
        .contributions()
        .embed_for(&Claim::fence("showcase-query").unwrap());
    assert_eq!(
        claimed.unwrap().item.page.as_deref(),
        Some("pages/query.tsx")
    );

    // H3: a base's rows, every group's, with the view's columns.
    let base = run(
        &list,
        "SHOWCASE_DATAVIEW_QUERY",
        &json!({ "source": "base: Bases/Showcase tasks.base\n" }),
    )
    .unwrap();
    assert_eq!(base["kind"], "base");
    assert_eq!(base["columns"], json!(["file.name", "status", "priority"]));
    let names: Vec<&str> = base["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Ship the vault",
            "Write the release notes",
            "Record the walkthrough"
        ]
    );
    assert_eq!(base["rows"][1]["values"]["priority"], "high");

    // A tag's notes, and a note's backlinks.
    let tagged = run(
        &list,
        "SHOWCASE_DATAVIEW_QUERY",
        &json!({ "source": "tag: task" }),
    )
    .unwrap();
    assert_eq!(tagged["rows"][0]["name"], "Ship the vault");
    assert_eq!(tagged["rows"][0]["values"]["tags"], "task, showcase");
    let back = run(
        &list,
        "SHOWCASE_DATAVIEW_QUERY",
        &json!({ "source": "backlinks: Samples/Tasks/Ship the vault.md" }),
    )
    .unwrap();
    assert_eq!(back["rows"][0]["path"], "Samples/Dataview-lite demo.md");

    // A query it does not know says what it takes.
    let bad = run(
        &list,
        "SHOWCASE_DATAVIEW_QUERY",
        &json!({ "source": "where: x" }),
    );
    assert!(bad.is_err_and(|e| e.to_string().contains("base, tag or backlinks")));
}

// ---- Chart --------------------------------------------------------------

#[test]
fn chart_claims_its_fence_and_counts_a_bases_rows_by_a_property() {
    let list = ActionList::new();
    vault(&list);
    let host = load(&list, "chart");
    let claimed = host
        .contributions()
        .embed_for(&Claim::fence("showcase-chart").unwrap());
    assert_eq!(
        claimed.unwrap().item.page.as_deref(),
        Some("pages/chart.tsx")
    );

    let out = run(
        &list,
        "SHOWCASE_CHART_COUNT",
        &json!({ "base": "Bases/Showcase tasks.base", "by": "status" }),
    )
    .unwrap();
    assert_eq!(
        out,
        json!({ "by": "status", "bars": [
            { "label": "done", "count": 1 },
            { "label": "open", "count": 2 },
        ] })
    );
}

#[test]
fn each_content_example_says_what_it_stands_in_for_and_reaches_only_queries() {
    for name in ["openers-sampler", "dataview-lite", "chart"] {
        let m = Package::read(&example(name)).unwrap().manifest;
        assert!(m.description.contains("Stands in for"), "{name}");
        assert!(m.capabilities.hosts.is_empty(), "{name}: no network");
    }
    let dataview = Package::read(&example("dataview-lite")).unwrap().manifest;
    assert_eq!(
        dataview.capabilities.actions,
        ["BASES_ROWS", "TAGS_NOTES", "LINKS_BACK"]
    );
}
