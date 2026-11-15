//! The cookbook over a real vault on disk: its manifest declares the
//! `.cook` file type, so its recipes are notes; `COOKBOOK_LIST`
//! answers from the vault's index (as a browser's does), `COOKBOOK_GET`
//! reads a file through the documents, a save is written to disk, and
//! `COOKBOOK_TIDY` moves Task's `>>` metadata into frontmatter.
//!
//! One test: the file type registry is the process's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_cookbook::Cookbook;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::VaultScope;
use architect_extensions::install::Installs;
use architect_navigator::VaultSource;
use architect_navigator::documents::VaultDocuments;
use architect_settings::Registry;
use serde_json::{Value, json};
use vault::actions::OpenVault;
use vault_query::{Built, QueryEngine};

const RECIPES: &[(&str, &str)] = &[
    (
        "Buttermilk Pancakes.cook",
        "---\ntitle: Buttermilk Pancakes\nservings: 4\ncourse: breakfast\nprep time: 10 min\ntags: [breakfast, quick]\n---\nWhisk @flour{250%g} with @buttermilk{400%ml}.\n",
    ),
    (
        "Overnight Oats.cook",
        "---\ntitle: Overnight Oats\nservings: 2\ntags: [breakfast, quick]\n---\nStir @rolled oats{100%g} into @milk{250%ml}.\n",
    ),
    (
        "Lentil Soup.cook",
        "---\ntitle: Lentil Soup\nservings: 6\ncourse: lunch\n---\nSoften @onion{1} in @olive oil{2%tbsp}.\n",
    ),
    (
        "Old Loaf.cook",
        ">> title: Old Loaf\n>> servings: 1\n\nMix @flour{450%g}.\n",
    ),
];

fn recipes(to: &Path) {
    std::fs::create_dir_all(to.join("Cookbook")).unwrap();
    for (name, text) in RECIPES {
        std::fs::write(to.join("Cookbook").join(name), text).unwrap();
    }
    std::fs::write(to.join("Notes.md"), "# Notes\n").unwrap();
}

async fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = list
        .run_json_async(id, args.to_string().as_bytes())
        .await
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cookbook_over_a_vault_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    recipes(&root);

    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    // Loading it registers `.cook` as a note type, before the vault is
    // read.
    let ext = Extensions::new(host.clone(), Installs::new(tmp.path().join("installs")))
        .with_linked(&Cookbook)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    assert!(vault::note_types::is_note_path("Cookbook/Lentil Soup.cook"));

    let open = OpenVault::open(&root).unwrap();
    let scope = architect::resource::Scope::new();
    let engine = QueryEngine::new(Arc::new(Built::new(open.clone())));
    let queries: vault_proto::query::VaultQueryClient = architect::serve_local(engine, &scope)
        .establish()
        .await
        .unwrap();
    let docs = VaultDocuments::new(VaultSource::named(open, "Vault"));
    host.set_vault(Arc::new(docs), queries, VaultScope::Person);

    // From the index: every recipe, by name, with its properties.
    let all = run(&list, "COOKBOOK_LIST", Value::Null).await;
    let names: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 4, "{all}");
    assert!(names.contains(&"Old Loaf"), "{all}");
    let pancakes = all
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Buttermilk Pancakes")
        .unwrap();
    assert_eq!(pancakes["course"], "breakfast");
    assert_eq!(pancakes["prepMinutes"], 10, "{pancakes}");
    let quick = run(&list, "COOKBOOK_LIST", json!({ "tag": "quick" })).await;
    assert_eq!(quick.as_array().unwrap().len(), 2, "{quick}");

    // Through the documents: a recipe read and scaled.
    let r = run(
        &list,
        "COOKBOOK_GET",
        json!({ "path": "Cookbook/Lentil Soup.cook", "servings": 3 }),
    )
    .await;
    assert_eq!(r["ingredients"][0]["name"], "onion");
    assert_eq!(r["ingredients"][0]["qty"], 0.5);

    // A save lands on disk.
    run(&list, "COOKBOOK_SAVE", json!({ "source": "---\ntitle: Toast\nservings: 1\n---\nToast @bread{2%slices}.\n", "create": true })).await;
    assert!(
        std::fs::read_to_string(root.join("Cookbook/Toast.cook"))
            .unwrap()
            .contains("@bread{2%slices}")
    );

    // Task's metadata, as frontmatter.
    let tidy = run(&list, "COOKBOOK_TIDY", Value::Null).await;
    assert_eq!(tidy["changed"], json!(["Cookbook/Old Loaf.cook"]), "{tidy}");
    assert!(
        std::fs::read_to_string(root.join("Cookbook/Old Loaf.cook"))
            .unwrap()
            .starts_with("---\n")
    );
    scope.close().await;
}
