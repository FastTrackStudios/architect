//! A demo vault's file of a type an extension declares
//! (`vault_proto::note_types`) is a note there once registered: the demo
//! builder (`app::vault::demo_vault`) reads a file's text, and makes it a
//! note, only when `demo::File::text` has it.
//!
//! One test: the registry is the process's.

#![allow(clippy::unwrap_used)]

use architect_web::demo::File;

#[test]
fn a_declared_file_type_in_the_demo_vault_is_a_note_once_registered() {
    let recipe = File {
        path: "Recipes/Pancakes.webcook",
        bytes: b"---\nservings: 4\n---\nMix @flour{200%g}.\n",
    };
    assert_eq!(recipe.text(), None, "an attachment, not yet a note");
    assert!(!vault_proto::note_types::is_note_path(recipe.path));

    vault_proto::note_types::register("webcook", "recipe");
    assert_eq!(
        recipe.text(),
        Some("---\nservings: 4\n---\nMix @flour{200%g}.\n")
    );
    assert!(vault_proto::note_types::is_note_path(recipe.path));
    assert_eq!(recipe.media_type(), None, "drawn as a note, not media");
}
