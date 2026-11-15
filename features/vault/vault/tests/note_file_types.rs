//! A file type an extension declares is a vault note like `.md`
//! (`vault_proto::note_types`): a vault on disk with a `.cook` recipe
//! reads it as a note once `cook` is registered, and not before, in the
//! open vault, its index (the recipe's `type` defaulted, its frontmatter
//! kept) and the node's folder source (`DiskSource`).
//!
//! One test: the registry is the process's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use vault::actions::OpenVault;
use vault::actions::query::DiskSource;
use vault_query::Built;

const PANCAKES: &str = "---\nservings: 4\n---\nMix @flour{200%g} and @milk{300%ml}.\n";

fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("Recipes/Pancakes.cook", PANCAKES),
        ("Notes/Shopping.md", "---\ntype: list\n---\n- eggs\n"),
    ] {
        let abs = dir.path().join(path);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, text).unwrap();
    }
    dir
}

fn pages(open: &OpenVault) -> Vec<String> {
    let mut all: Vec<String> = open.with(|v| v.pages.iter().map(|p| p.rel_path.clone()).collect());
    all.sort();
    all
}

#[test]
fn a_declared_file_type_is_a_note_once_registered() {
    // Not registered: a `.cook` file is an attachment, nowhere a note.
    let dir = vault();
    let open = OpenVault::open(dir.path()).unwrap();
    assert_eq!(pages(&open), ["Notes/Shopping.md"]);
    assert_eq!(open.text("Recipes/Pancakes.cook"), None);
    let index = Built::new(open);
    index.sync();
    assert!(!index.index().entries.contains_key("Recipes/Pancakes.cook"));
    let disk = Built::new(DiskSource::open(dir.path()));
    disk.sync();
    assert!(!disk.index().entries.contains_key("Recipes/Pancakes.cook"));

    assert!(vault_proto::note_types::register("cook", "recipe"));

    // Registered: the same folder, opened again.
    let open = OpenVault::open(dir.path()).unwrap();
    assert_eq!(pages(&open), ["Notes/Shopping.md", "Recipes/Pancakes.cook"]);
    assert_eq!(
        open.text("Recipes/Pancakes.cook").as_deref(),
        Some(PANCAKES)
    );
    let index = Built::new(open.clone());
    index.sync();
    let now = index.index();
    let entry = &now.entries["Recipes/Pancakes.cook"];
    assert_eq!(entry.props["type"], "recipe", "its type, defaulted");
    assert_eq!(entry.props["servings"], 4, "its frontmatter, kept");
    assert_eq!(now.entries["Notes/Shopping.md"].props["type"], "list");

    let disk = Built::new(DiskSource::open(dir.path()));
    disk.sync();
    assert_eq!(
        disk.index().entries["Recipes/Pancakes.cook"].props["type"],
        "recipe"
    );

    // A note in every sense: its properties are written as a note's are.
    open.set_property("Recipes/Pancakes.cook", "servings", serde_json::json!(6))
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("Recipes/Pancakes.cook")).unwrap();
    assert!(text.contains("servings: 6"), "{text}");
    assert!(text.contains("Mix @flour{200%g}"), "its body kept: {text}");
}
