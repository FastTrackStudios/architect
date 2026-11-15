//! The cookbook linked into a host over the app's documents: its
//! actions run through the host, each one call of the `Recipes` client
//! served in process; a recipe saved, found, read scaled, its
//! ingredients for a set of portions with a nested recipe cooked out,
//! renamed and deleted.
//!
//! Run with nextest: the extension keeps process-wide statics.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::needless_pass_by_value
)]

use std::path::Path;

use architect::action::{ActionList, Principal};
use architect_cookbook::{Cookbook, ID};
use architect_extensions::documents::{CrdtDocuments, Documents as _};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

fn try_run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn docs() -> CrdtDocuments {
    let docs = CrdtDocuments::new().with_space("Cookbook");
    let app = Principal::Service {
        name: "test".to_owned(),
    };
    docs.create(
        "Cookbook/Hot Honey.cook",
        "---\nservings: 6\n---\nWarm @honey{120%g} with @chilli flakes{1%tsp}.\n",
        &app,
    )
    .unwrap();
    docs
}

fn extensions(tmp: &Path, docs: &CrdtDocuments) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let host =
        Host::new(list.clone(), SharedSections::new(Registry::new())).with_documents(docs.clone());
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&Cookbook)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (ext, list)
}

#[test]
fn a_recipe_saved_found_scaled_cooked_out_and_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = docs();
    let (ext, list) = extensions(tmp.path(), &docs);
    assert!(ext.host().get(ID).is_some());

    let source = "---\ntitle: Chicken and Waffles\nservings: 4\ncourse: dinner\ntags: [comfort]\n---\nFry @chicken thighs{8} for ~{12%minutes}.\n\nDrizzle [[Hot Honey]]{3} over.\n";
    let saved = run(
        &list,
        "COOKBOOK_SAVE",
        json!({ "source": source, "create": true }),
    );
    assert_eq!(saved["path"], "Cookbook/Chicken and Waffles.cook");
    // Kept exactly as written.
    assert_eq!(
        docs.read("Cookbook/Chicken and Waffles.cook").unwrap(),
        source
    );
    // Cooklang that does not parse is refused, and nothing is written.
    assert!(
        try_run(
            &list,
            "COOKBOOK_SAVE",
            json!({ "path": "Cookbook/Bad.cook", "source": "---\ntitle: [unclosed\n---\nMix @flour{2%g}.\n" })
        )
        .is_err()
    );
    assert!(docs.read("Cookbook/Bad.cook").is_err());

    let found = run(&list, "COOKBOOK_LIST", json!({ "tag": "comfort" }));
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found[0]["name"], "Chicken and Waffles");
    assert_eq!(found[0]["course"], "dinner");

    let r = run(
        &list,
        "COOKBOOK_GET",
        json!({ "path": "Cookbook/Chicken and Waffles.cook", "servings": 8 }),
    );
    assert_eq!(r["servings"], 8);
    assert_eq!(r["ingredients"][0]["qty"], 16.0);
    assert_eq!(r["cookSteps"][0]["timers"][0]["seconds"], 720, "{r}");

    let need = run(
        &list,
        "COOKBOOK_INGREDIENTS",
        json!({ "portions": [{ "path": "Cookbook/Chicken and Waffles.cook", "servings": 8 }] }),
    );
    let honey = need
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "honey")
        .unwrap();
    assert_eq!(honey["qty"], 120.0);
    assert_eq!(honey["recipe"], "Cookbook/Hot Honey.cook");

    let moved = run(
        &list,
        "COOKBOOK_RENAME",
        json!({ "path": "Cookbook/Chicken and Waffles.cook", "to": "Cookbook/Dinner/Chicken and Waffles" }),
    );
    assert_eq!(moved["path"], "Cookbook/Dinner/Chicken and Waffles.cook");
    run(
        &list,
        "COOKBOOK_DELETE",
        json!({ "path": "Cookbook/Dinner/Chicken and Waffles.cook" }),
    );
    assert!(
        docs.read("Cookbook/Dinner/Chicken and Waffles.cook")
            .is_err()
    );
    let left = run(&list, "COOKBOOK_LIST", Value::Null);
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
}
