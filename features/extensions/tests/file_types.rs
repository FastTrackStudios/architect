//! A file type an extension declares (`[[file_types]]`) is a vault note
//! from the moment the host loads it: a linked extension's registers it in
//! the vault's registry (`vault_proto::note_types`), `EXT_FILE_TYPES`
//! lists it, and a manifest that declares a bad one is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension};
use architect_extensions::manifest::Manifest;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::Value;

const MANIFEST: &str = r#"
id = "lab.recipes"
name = "Recipes"
version = "0.1.0"
kind = "process"

[entry]
path = "recipes-bin"

[[file_types]]
extension = "labcook"
name = "Recipe"
icon = "chef-hat"
note_type = "recipe"
"#;

struct Recipes;

impl Extension for Recipes {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, _: &mut Builder) {}
}

#[test]
fn a_linked_extensions_file_type_is_a_note_once_it_loads() {
    assert!(!vault_proto::note_types::is_note_path(
        "Recipes/Pancakes.labcook"
    ));

    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Recipes)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    assert!(ext.host().get("lab.recipes").is_some());

    assert!(vault_proto::note_types::is_note_path(
        "Recipes/Pancakes.labcook"
    ));
    assert_eq!(
        vault_proto::note_types::note_type_of("Recipes/Pancakes.labcook").as_deref(),
        Some("recipe")
    );

    let out = block_on(list.run_json_async("EXT_FILE_TYPES", b"")).unwrap();
    let all: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(all[0]["extension"], "md");
    let cook = all
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["extension"] == "labcook")
        .expect("listed");
    assert_eq!(cook["name"], "Recipe");
    assert_eq!(cook["icon"], "chef-hat");
    assert_eq!(cook["note_type"], "recipe");
    assert_eq!(cook["from"], "lab.recipes");
    assert_eq!(cook["registered"], true);

    // Turned off, its files stay notes.
    let _ = block_on(list.run_json_async("EXT_DISABLE", br#"{"id":"lab.recipes"}"#)).unwrap();
    assert!(vault_proto::note_types::is_note_path(
        "Recipes/Pancakes.labcook"
    ));
}

#[test]
fn a_bad_file_type_is_refused() {
    let with = |table: &str| {
        Manifest::parse(&format!(
            "id = \"lab.bad\"\nname = \"Bad\"\nversion = \"0.1.0\"\nkind = \"luau\"\n\n[entry]\npath = \"main.luau\"\n\n[[file_types]]\n{table}\n"
        ))
    };
    assert!(with("extension = \"labok\"\nname = \"Ok\"\nnote_type = \"ok\"").is_ok());
    assert!(
        with("extension = \"labtext\"\nname = \"Ok\"\nnote_type = \"ok\"\nbody = \"text\"").is_ok()
    );
    for bad in [
        "extension = \"md\"\nname = \"Note\"\nnote_type = \"note\"",
        "extension = \"base\"\nname = \"Base\"\nnote_type = \"base\"",
        "extension = \".cook\"\nname = \"Recipe\"\nnote_type = \"recipe\"",
        "extension = \"Cook\"\nname = \"Recipe\"\nnote_type = \"recipe\"",
        "extension = \"cook\"\nname = \"Recipe\"",
        "extension = \"cook\"\nnote_type = \"recipe\"",
        "name = \"Recipe\"\nnote_type = \"recipe\"",
        "extension = \"cook\"\nname = \"Recipe\"\nnote_type = \"recipe\"\nbody = \"html\"",
    ] {
        let refused = with(bad).expect_err(bad).to_string();
        assert!(refused.contains("file type"), "{bad}: {refused}");
    }
}
