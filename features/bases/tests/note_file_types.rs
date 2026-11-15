//! A base finds the notes of a file type an extension declares by the
//! `type` they read as (`vault::note_types`): `BASES_ROWS` over a vault
//! with a `.cook` recipe, before and after `cook` is registered.
//!
//! One test: the registry is the process's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use architect::action::ActionList;
use architect_bases::model::ViewTypes;
use serde_json::{Value, json};

fn recipes(list: &ActionList) -> Vec<String> {
    let args = json!({ "source": "filters: 'type == \"recipe\"'" });
    let out =
        futures::executor::block_on(list.run_json_async("BASES_ROWS", args.to_string().as_bytes()))
            .unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    out["rows"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["rows"].as_array().unwrap().clone())
        .map(|r| r["path"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn app(dir: &std::path::Path) -> Arc<ActionList> {
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &open).unwrap();
    architect_bases::actions::register(
        &list,
        &open,
        &ViewTypes::new(),
        architect_bases::actions::local_queries(&open),
    )
    .unwrap();
    list
}

#[test]
fn a_base_finds_a_declared_file_types_notes_by_their_type() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("Recipes")).unwrap();
    std::fs::write(
        tmp.path().join("Recipes/Pancakes.cook"),
        "---\nservings: 4\n---\nMix @flour{200%g}.\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("Plain.md"), "# Plain\n").unwrap();

    assert!(recipes(&app(tmp.path())).is_empty(), "not a note yet");

    vault::note_types::register("cook", "recipe");
    assert_eq!(recipes(&app(tmp.path())), ["Recipes/Pancakes.cook"]);
}
