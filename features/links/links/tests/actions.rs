//! The `LINKS_*` actions over the lab's sample vault.

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
fn the_sample_vaults_link_graph() {
    let (_tmp, vault) = sample();
    let list = ActionList::new();
    links::actions::register(&list, &vault).unwrap();
    let graph = run(&list, "LINKS_GRAPH", "");
    let edges = graph.as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == "Welcome.md" && e["to"] == "Notes/The app shell.md"),
        "{graph}"
    );
    let back = run(&list, "LINKS_BACK", r#"{"path":"Notes/The app shell.md"}"#);
    let back: Vec<&str> = back
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        back.contains(&"Welcome.md") && back.contains(&"Notes/The workspace.md"),
        "{back:?}"
    );
    let unresolved = run(&list, "LINKS_UNRESOLVED", "");
    assert_eq!(unresolved[0]["link"], "A note nobody wrote", "{unresolved}");
    assert_eq!(
        unresolved[0]["from"], "Notes/The workspace.md",
        "{unresolved}"
    );
    // Nothing links to Actions everywhere's neighbours but Welcome; the
    // sample has no orphans.
    let orphans = run(&list, "LINKS_ORPHANS", "");
    assert!(orphans.as_array().unwrap().is_empty(), "{orphans}");
    assert!(
        list.run_json("LINKS_BACK", br#"{"path":"Nope.md"}"#)
            .is_err()
    );
}
