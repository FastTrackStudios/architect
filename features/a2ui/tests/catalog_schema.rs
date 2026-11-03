//! The catalog's JSON Schema is generated from the catalog: the
//! committed `catalog.schema.json` is that output, and this test fails
//! when a component changes without it. `A2UI_UPDATE_SCHEMA=1` rewrites
//! it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

#[test]
fn the_committed_schema_is_the_catalogs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog.schema.json");
    let now = architect_a2ui::catalog::schema_text();
    if std::env::var_os("A2UI_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &now).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        kept == now,
        "catalog.schema.json is not the catalog's: run `A2UI_UPDATE_SCHEMA=1 cargo test --workspace --test catalog_schema` and commit it"
    );
}

#[test]
fn every_component_in_the_schema_is_drawn_and_nothing_else_is() {
    let schema = architect_a2ui::catalog::schema();
    let defs = schema["$defs"].as_object().unwrap();
    for name in defs.keys() {
        assert!(
            architect_a2ui::ui::draws(name).is_some(),
            "{name} is in the schema but nothing draws it"
        );
    }
    for name in ["Script", "Iframe", "Html", "WebView", ""] {
        assert!(architect_a2ui::ui::draws(name).is_none(), "{name} is drawn");
    }
}
