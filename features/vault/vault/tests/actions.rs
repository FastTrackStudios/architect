//! The `VAULT_*` actions over the lab's sample vault (`apps/sandbox/vault`),
//! copied to a temporary folder so the writes land there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use serde_json::Value;
use vault::actions::{OpenVault, register};

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/sandbox/vault")
}

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

fn lab() -> (tempfile::TempDir, ActionList) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&sample(), tmp.path());
    let list = ActionList::new();
    register(&list, &OpenVault::open(tmp.path()).unwrap()).unwrap();
    (tmp, list)
}

fn run(list: &ActionList, id: &str, args: &str) -> Value {
    let out = list
        .run_json(id, args.as_bytes())
        .unwrap_or_else(|e| panic!("{id} {args}: {e:?}"));
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn a_notes_properties_links_and_tags_read_back() {
    let (_tmp, list) = lab();
    let status = run(&list, "VAULT_STATUS", "");
    assert_eq!(status["notes"], 5, "{status}");
    assert_eq!(status["bases"], 1, "{status}");
    let unresolved = status["unresolved_links"].as_array().unwrap();
    assert!(
        unresolved
            .iter()
            .any(|u| u["link"] == "A note nobody wrote"),
        "{status}"
    );

    let note = run(&list, "VAULT_READ", r#"{"path":"Welcome.md"}"#);
    assert_eq!(note["properties"]["status"], "active", "{note}");
    assert_eq!(
        note["tags"],
        serde_json::json!(["lab", "lab/sample", "start"])
    );
    let links: Vec<&str> = note["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["resolves_to"].as_str())
        .collect();
    assert!(links.contains(&"Notes/The app shell.md"), "{note}");
    assert!(links.contains(&"Notes/Actions everywhere.md"), "{note}");
    assert!(links.contains(&"Songs/Amazing Grace.md"), "{note}");
    assert!(
        note["backlinks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "Notes/The app shell.md"),
        "{note}"
    );
    assert!(note["body"].as_str().unwrap().starts_with("# Welcome"));
}

#[test]
fn list_search_and_a_base_run() {
    let (_tmp, list) = lab();
    let notes = run(&list, "VAULT_LIST", r#"{"folder":"Notes"}"#);
    assert_eq!(notes.as_array().unwrap().len(), 3, "{notes}");
    let hits = run(&list, "VAULT_SEARCH", r#"{"query":"screensets"}"#);
    assert_eq!(hits[0]["path"], "Notes/The workspace.md", "{hits}");
    let tables = run(
        &list,
        "VAULT_QUERY_BASE",
        r#"{"path":"Bases/Notes by status.base"}"#,
    );
    let groups = tables[0]["groups"].as_array().unwrap();
    let labels: Vec<&str> = groups.iter().filter_map(|g| g["label"].as_str()).collect();
    assert!(
        labels.contains(&"active") && labels.contains(&"draft"),
        "{tables}"
    );
}

#[test]
fn writes_land_on_disk_and_read_back() {
    let (tmp, list) = lab();
    run(
        &list,
        "VAULT_CREATE",
        r#"{"path":"Notes/New.md","body":"Links [[Welcome]].\n","properties":"{\"status\":\"draft\"}"}"#,
    );
    run(
        &list,
        "VAULT_SET_PROPERTY",
        r#"{"path":"Notes/New.md","key":"rating","value":"4"}"#,
    );
    run(
        &list,
        "VAULT_APPEND",
        r#"{"path":"Notes/New.md","text":"More. #fresh"}"#,
    );
    let disk = std::fs::read_to_string(tmp.path().join("Notes/New.md")).unwrap();
    assert!(
        disk.contains("rating: 4") && disk.contains("More. #fresh"),
        "{disk}"
    );

    // Read back after a reload from disk.
    run(&list, "VAULT_RELOAD", "");
    let note = run(&list, "VAULT_READ", r#"{"path":"Notes/New.md"}"#);
    assert_eq!(note["properties"]["rating"], 4, "{note}");
    assert!(
        note["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "fresh")
    );
    let welcome = run(&list, "VAULT_READ", r#"{"path":"Welcome.md"}"#);
    assert!(
        welcome["backlinks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "Notes/New.md"),
        "{welcome}"
    );

    run(
        &list,
        "VAULT_REMOVE_PROPERTY",
        r#"{"path":"Notes/New.md","key":"rating"}"#,
    );
    run(&list, "VAULT_DELETE", r#"{"path":"Notes/New.md"}"#);
    assert!(!tmp.path().join("Notes/New.md").exists());
    // A taken path is refused.
    assert!(
        list.run_json("VAULT_CREATE", br#"{"path":"Welcome.md"}"#)
            .is_err()
    );
}
