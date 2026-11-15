//! The documents API (`documents::VaultDocuments`, what an extension's
//! `read_document` / `edit_document` reach) over a vault with a `.cook`
//! recipe: refused as no note before `cook` is registered
//! (`vault::note_types`), read and edited as one after.
//!
//! One test: the registry is the process's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_extensions::documents::{DocEdit, DocError, Documents as _};
use architect_navigator::VaultSource;
use architect_navigator::documents::VaultDocuments;
use vault::actions::OpenVault;

const PATH: &str = "Recipes/Pancakes.cook";
const PANCAKES: &str = "---\nservings: 4\n---\nMix @flour{200%g}.\n";

fn docs(root: &std::path::Path) -> VaultDocuments {
    VaultDocuments::new(VaultSource::named(OpenVault::open(root).unwrap(), "Vault"))
}

#[test]
fn the_documents_api_reads_and_edits_a_declared_file_type() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("Recipes")).unwrap();
    std::fs::write(tmp.path().join(PATH), PANCAKES).unwrap();

    let before = docs(tmp.path());
    assert!(matches!(before.read(PATH), Err(DocError::Invalid(_))));
    assert!(before.list(None).unwrap().iter().all(|d| d.path != PATH));

    vault::note_types::register("cook", "recipe");
    let docs = docs(tmp.path());
    assert!(docs.list(None).unwrap().iter().any(|d| d.path == PATH));
    assert_eq!(docs.read(PATH).unwrap(), PANCAKES);
    let by = architect::action::Principal::extension("lab.recipes", None);
    let after = docs
        .edit(
            PATH,
            &[DocEdit::Append {
                text: "Fry in @butter{}.\n".to_owned(),
            }],
            "Add a step",
            &by,
        )
        .unwrap();
    assert!(after.changed);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(PATH)).unwrap(),
        format!("{PANCAKES}Fry in @butter{{}}.\n")
    );
}
