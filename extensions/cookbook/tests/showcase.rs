//! The showcase vault's recipes (`apps/ui/showcase-vault/Cookbook`):
//! each one reads as Cooklang, with its frontmatter, and Chicken and
//! Waffles' Hot Honey cooks out into its own ingredients.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::path::Path;

use architect_cookbook::backend::Backend;
use architect_cookbook::files::Files;
use architect_cookbook::{Portion, Recipes as _, Search};
use futures::executor::block_on;

fn vault() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/ui/showcase-vault/Cookbook");
    std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "cook"))
        .map(|p| {
            (
                format!("Cookbook/{}", p.file_name().unwrap().to_string_lossy()),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn every_showcase_recipe_reads() {
    let files = vault();
    assert!(files.len() >= 6, "{} recipes", files.len());
    for (path, text) in &files {
        let r =
            architect_cookbook::parse::check(path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(!r.ingredients.is_empty(), "{path}");
        assert!(r.servings.is_some(), "{path}");
    }
    let b = Backend::new(Files::memory(files));
    let breakfast = block_on(b.list(Search {
        course: Some("breakfast".into()),
        ..Search::default()
    }))
    .unwrap();
    assert_eq!(breakfast.len(), 2);
    let need = block_on(b.ingredients(vec![Portion {
        path: "Cookbook/Chicken and Waffles.cook".into(),
        servings: Some(4.0),
    }]))
    .unwrap();
    let honey = need.iter().find(|n| n.name == "honey").unwrap();
    // Three of Hot Honey's six servings.
    assert_eq!(honey.qty, Some(60.0));
}
