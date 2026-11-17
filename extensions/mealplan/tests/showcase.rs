//! The showcase vault (`apps/ui/demo-vault`) on disk, the Cookbook
//! and Mealplan linked in as the app links them: next week's plan
//! (`Meal plans/2026-W47.md`) becomes a grocery list, the pantry taken
//! off, a substitution applied, grouped by aisle, written as a checklist
//! note. What the showcase rows' captures show.
//!
//! One test: the file type registry is the process's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
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

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap();
        if p.is_dir() {
            copy(&p, &to.join(name));
        } else {
            std::fs::copy(&p, to.join(name)).unwrap();
        }
    }
}

async fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = list
        .run_json_async(id, args.to_string().as_bytes())
        .await
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_showcase_weeks_grocery_list() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let showcase = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/ui/demo-vault");
    for dir in ["Cookbook", "Meal plans", "Pantry"] {
        copy(&showcase.join(dir), &root.join(dir));
    }

    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host.clone(), Installs::new(tmp.path().join("installs")))
        .with_linked(&architect_cookbook::Cookbook)
        .unwrap()
        .with_linked(&architect_mealplan::Mealplan)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();

    let open = OpenVault::open(&root).unwrap();
    let scope = architect::resource::Scope::new();
    let engine = QueryEngine::new(Arc::new(Built::new(open.clone())));
    let queries: vault_proto::query::VaultQueryClient = architect::serve_local(engine, &scope)
        .establish()
        .await
        .unwrap();
    host.set_vault(
        Arc::new(VaultDocuments::new(VaultSource::named(open, "Vault"))),
        queries,
        VaultScope::Person,
    );

    let week = run(&list, "MEALPLAN_WEEK", json!({ "week": "2026-W47" })).await;
    let meals: usize = week["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["meals"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(meals, 8, "{week}");

    run(&list, "MEALPLAN_GROCERIES", json!({ "week": "2026-W47" })).await;
    let note = std::fs::read_to_string(root.join("Groceries/2026-W47.md")).unwrap();
    // Every planned recipe's ingredients, by aisle, as a checklist.
    assert!(note.contains("- [ ] "), "{note}");
    assert!(note.contains("## "), "{note}");
    assert!(note.contains("minced beef"), "{note}");
    assert!(note.contains("red lentils"), "{note}");
    // Hot Honey, which Chicken and Waffles calls for, cooked out.
    assert!(note.contains("chilli flakes"), "{note}");
    // In the pantry, enough: not bought.
    let to_buy: Vec<&str> = note.lines().filter(|l| l.starts_with("- [ ] ")).collect();
    assert!(
        !to_buy
            .iter()
            .any(|l| l.ends_with(" eggs") || l.ends_with(" flour")),
        "{note}"
    );
    // A count is bought whole (Lentil Soup's 2.67 carrots).
    assert!(note.contains("- [ ] 3 carrots"), "{note}");
    // Teaspoons of cumin against grams in the pantry: no conversion, so
    // it is bought, with what the pantry has said.
    assert!(note.contains("cumin (have 40 g cumin)"), "{note}");
    assert!(to_buy.len() >= 10, "{note}");
    scope.close().await;
}
