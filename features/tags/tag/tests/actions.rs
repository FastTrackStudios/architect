//! The `TAGS_*` actions over the lab's sample vault.

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use serde_json::Value;
use vault::actions::OpenVault;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// The lab's sample vault (`apps/lab/vault`), copied to a temporary folder.
fn sample() -> (tempfile::TempDir, OpenVault) {
    let tmp = tempfile::tempdir().unwrap();
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/lab/vault");
    copy_dir(&from, tmp.path());
    let open = OpenVault::open(tmp.path()).unwrap();
    (tmp, open)
}

fn run(list: &ActionList, id: &str, args: &str) -> Value {
    let out = list
        .run_json(id, args.as_bytes())
        .unwrap_or_else(|e| panic!("{id} {args}: {e:?}"));
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn the_sample_vaults_tags_and_their_notes() {
    let (_tmp, vault) = sample();
    let list = ActionList::new();
    tag::actions::register(&list, &vault).unwrap();
    let tags = run(&list, "TAGS_LIST", "");
    let shell = tags
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tag"] == "shell")
        .unwrap_or_else(|| panic!("{tags}"));
    assert_eq!(shell["notes"], 2, "{tags}");
    let notes = run(&list, "TAGS_NOTES", r##"{"tag":"#lab"}"##);
    assert_eq!(notes.as_array().unwrap().len(), 1, "{notes}");
    assert_eq!(notes[0]["path"], "Welcome.md", "{notes}");
    let agents = run(&list, "TAGS_NOTES", r#"{"tag":"agents"}"#);
    assert_eq!(agents[0]["path"], "Notes/Actions everywhere.md", "{agents}");
}
