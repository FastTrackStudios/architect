//! The `BASES_*` actions over a vault on disk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use architect::action::ActionList;
use architect_bases::model::{ViewType, ViewTypes};
use serde_json::{Value, json};

fn run(list: &ActionList, id: &str, args: &Value) -> Value {
    let out = list
        .run_json(id, args.to_string().as_bytes())
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap()
}

fn vault() -> (tempfile::TempDir, Arc<ActionList>, ViewTypes) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    for (path, text) in [
        ("A.md", "---\nstatus: todo\nrank: 2\n---\nA"),
        ("B.md", "---\nstatus: done\nrank: 1\n---\nB"),
        ("C.md", "---\nstatus: todo\nrank: 3\n---\nC"),
        ("D.md", "No properties."),
        (
            "Bases/Status.base",
            "filters:\n  and:\n    - 'status != null'\nviews:\n  - type: table\n    name: By status\n    order: [file.name, status]\n    groupBy:\n      property: status\n",
        ),
    ] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &open).unwrap();
    let types = ViewTypes::new();
    architect_bases::actions::register(&list, &open, &types).unwrap();
    (tmp, list, types)
}

#[test]
fn a_base_lists_reads_and_runs_its_views() {
    let (_tmp, list, _) = vault();
    let bases = run(&list, "BASES_LIST", &Value::Null);
    assert_eq!(bases[0]["path"], "Bases/Status.base");
    assert_eq!(
        bases[0]["views"][0],
        json!({ "name": "By status", "type": "table" })
    );
    let rows = run(&list, "BASES_ROWS", &json!({ "path": "Bases/Status.base" }));
    let groups = rows["rows"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "{rows}");
    assert_eq!(groups[0]["label"], "todo");
    assert_eq!(groups[0]["rows"].as_array().unwrap().len(), 2);
    assert!(rows["columns"].as_array().unwrap().contains(&json!("rank")));
    // A fence's YAML runs the same, over the same notes.
    let inline = run(
        &list,
        "BASES_ROWS",
        &json!({ "source": "filters: 'rank > 1'" }),
    );
    assert_eq!(inline["view"]["type"], "table");
    assert_eq!(
        inline["rows"]["groups"][0]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Without a workspace on the list, BASES_OPEN only reads it.
    let opened = futures::executor::block_on(
        list.run_json_async("BASES_OPEN", br#"{"path":"Bases/Status.base"}"#),
    )
    .unwrap();
    let opened: Value = serde_json::from_slice(&opened).unwrap();
    assert_eq!(opened["opened"], false);
}

#[test]
fn edits_write_the_base_file_and_the_next_run_shows_them() {
    let (tmp, list, _) = vault();
    let path = json!("Bases/Status.base");
    run(
        &list,
        "BASES_SET_FILTERS",
        &json!({ "path": path, "clauses": ["rank >= 2"] }),
    );
    run(
        &list,
        "BASES_SET_SORT",
        &json!({ "path": path, "sort": ["rank desc"] }),
    );
    run(&list, "BASES_SET_GROUP", &json!({ "path": path }));
    run(
        &list,
        "BASES_SET_PROPERTIES",
        &json!({ "path": path, "order": ["file.name", "rank", "formula.double"] }),
    );
    run(
        &list,
        "BASES_SET_FORMULA",
        &json!({ "path": path, "name": "double", "expression": "rank * 2" }),
    );
    let rows = run(&list, "BASES_ROWS", &json!({ "path": path }));
    let names: Vec<_> = rows["rows"]["groups"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["C", "A"], "{rows}");
    assert_eq!(
        rows["rows"]["groups"][0]["rows"][0]["values"]["formula.double"],
        6.0
    );
    let added = run(
        &list,
        "BASES_ADD_VIEW",
        &json!({ "path": path, "type": "cards" }),
    );
    assert_eq!(added["added"], "cards");
    run(
        &list,
        "BASES_SET_OPTION",
        &json!({ "path": path, "view": "cards", "key": "imageProperty", "value": "cover" }),
    );
    let text = std::fs::read_to_string(tmp.path().join("Bases/Status.base")).unwrap();
    assert!(
        text.contains("rank >= 2")
            && text.contains("type: cards")
            && text.contains("imageProperty"),
        "{text}"
    );
    // A cell's edit is the note's property.
    run(
        &list,
        "VAULT_SET_PROPERTY",
        &json!({ "path": "B.md", "key": "rank", "value": "9" }),
    );
    let rows = run(&list, "BASES_ROWS", &json!({ "path": path }));
    assert_eq!(rows["rows"]["groups"][0]["rows"][0]["name"], "B");
    assert!(
        list.run_json(
            "BASES_SET_SORT",
            br#"{"path":"Bases/Status.base","view":"nope","sort":[]}"#
        )
        .is_err()
    );
}

#[test]
fn view_types_are_bases_own_and_what_extensions_add() {
    let (_tmp, list, types) = vault();
    assert_eq!(
        run(&list, "BASES_VIEW_TYPES", &Value::Null)
            .as_array()
            .unwrap()
            .len(),
        3
    );
    types.set_source(|| {
        vec![ViewType {
            id: "board".into(),
            name: "Board".into(),
            owner: Some("lab.board".into()),
            ..ViewType::default()
        }]
    });
    let all = run(&list, "BASES_VIEW_TYPES", &Value::Null);
    assert_eq!(all[3]["owner"], "lab.board");
}

/// Copies `from` into `to`, files and folders.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn every_base_in_the_labs_demo_vault_runs_every_view() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/ui/demo-vault");
    copy_dir(&demo, tmp.path());
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(tmp.path()).unwrap();
    architect_bases::actions::register(&list, &open, &ViewTypes::new()).unwrap();
    let bases = run(&list, "BASES_LIST", &Value::Null);
    let bases = bases.as_array().unwrap();
    assert_eq!(bases.len(), 5, "{bases:?}");
    for b in bases {
        assert!(b.get("error").is_none(), "{b}");
        for v in b["views"].as_array().unwrap() {
            let rows = run(
                &list,
                "BASES_ROWS",
                &json!({ "path": b["path"], "view": v["name"] }),
            );
            assert_eq!(rows["view"]["type"], v["type"], "{rows}");
            let found: usize = rows["rows"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["rows"].as_array().unwrap().len())
                .sum();
            assert!(found > 0, "{} {} has no rows: {rows}", b["path"], v["name"]);
        }
    }
}
