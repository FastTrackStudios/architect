//! The document API (#309): the lab's `lab.tidy` extension
//! (`apps/lab/extensions/tidy`) creates, tidies (one transaction, one
//! undo step), renames, files away and watches documents in the lab's
//! spaces, kept as `crdt` documents (`documents::CrdtDocuments`); the
//! person's concurrent typing merges with its edits; an extension that
//! was not given `reads` / `writes = ["documents"]` is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, Principal};
use architect_extensions::documents::{CrdtDocuments, Documents as _};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{DocEdit, HostError, HostRequest};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use crdt::loro::{ExportMode, LoroDoc};
use futures::executor::block_on;
use serde_json::{Value, json};

const TIDY: &str = "lab.tidy";

fn tidy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/lab/extensions/tidy")
}

/// The lab's spaces, and a note in them.
fn lab_documents() -> CrdtDocuments {
    let docs = CrdtDocuments::new()
        .with_space("notes")
        .with_space("archive");
    let app = Principal::Service {
        name: "lab".to_owned(),
    };
    docs.create("notes/Welcome.md", "# Welcome\n", &app).unwrap();
    docs
}

/// A host with `docs`, its data under `tmp`, and `lab.tidy` loaded.
fn host(docs: &CrdtDocuments, tmp: &Path) -> (ActionList, Host) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_documents(docs.clone())
        .with_data_root(tmp.join("data"));
    let package = Package::read(&tidy_dir()).unwrap();
    host.load(&package, Origin::Dev).unwrap();
    (list, host)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Value {
    let out = block_on(list.run_json_async_with(
        &ActionCaller::app(),
        id,
        args.to_string().as_bytes(),
    ))
    .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// Waits until what `lab.tidy` heard holds `line`.
fn heard(list: &ActionList, line: &str) -> Vec<String> {
    let started = Instant::now();
    loop {
        let out = run(list, "LAB_TIDY_LIST", &Value::Null);
        let lines: Vec<String> = out["heard"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if lines.iter().any(|l| l == line) {
            return lines;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it never heard {line:?}: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_lab_extension_creates_tidies_renames_and_watches_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = lab_documents();
    let (list, _host) = host(&docs, tmp.path());

    let keys = run(&list, "LAB_TIDY_WATCH", &Value::Null);
    assert_eq!(keys.as_array().map(Vec::len), Some(4), "{keys}");

    let made = run(&list, "LAB_TIDY_NEW", &json!({ "title": "Inbox" }));
    assert_eq!(made["path"], "notes/Inbox.md", "{made}");
    assert_eq!(made["space"], "notes", "{made}");
    assert_eq!(docs.text("notes/Inbox.md").as_deref(), Some("# Inbox\n"));
    heard(&list, "doc.created notes/Inbox.md");

    // One transaction: three edits, one undo step, made as the extension.
    let after = run(&list, "LAB_TIDY_TIDY", &json!({ "path": "notes/Inbox.md" }));
    let tidied = "---\ntidy: true\n---\n# Inbox\n\n— tidied\n";
    assert_eq!(after, tidied);
    assert_eq!(docs.text("notes/Inbox.md").as_deref(), Some(tidied));
    assert_eq!(docs.steps("notes/Inbox.md"), ["Tidy notes/Inbox.md"]);
    assert!(
        docs.authors("notes/Inbox.md")
            .contains(&"extension:lab.tidy".to_owned()),
        "{:?}",
        docs.authors("notes/Inbox.md")
    );
    heard(&list, "doc.saved notes/Inbox.md");
    assert_eq!(
        docs.undo("notes/Inbox.md").as_deref(),
        Some("Tidy notes/Inbox.md")
    );
    assert_eq!(
        docs.text("notes/Inbox.md").as_deref(),
        Some("# Inbox\n"),
        "undone as one step"
    );

    let renamed = run(
        &list,
        "LAB_TIDY_RENAME",
        &json!({ "path": "notes/Inbox.md", "to": "notes/Today.md" }),
    );
    assert_eq!(renamed["path"], "notes/Today.md", "{renamed}");
    heard(&list, "doc.renamed notes/Inbox.md -> notes/Today.md");
    // Its history went with it.
    assert_eq!(docs.redo("notes/Today.md").as_deref(), Some("Tidy notes/Inbox.md"));

    let filed = run(&list, "LAB_TIDY_FILE", &json!({ "path": "notes/Today.md" }));
    assert_eq!(filed["path"], "archive/Today.md", "{filed}");
    let lines = heard(&list, "doc.renamed notes/Today.md -> archive/Today.md");
    assert_eq!(
        lines,
        [
            "doc.created notes/Inbox.md",
            "doc.saved notes/Inbox.md",
            "doc.renamed notes/Inbox.md -> notes/Today.md",
            "doc.renamed notes/Today.md -> archive/Today.md",
        ]
    );
    let listed = run(&list, "LAB_TIDY_LIST", &Value::Null);
    assert_eq!(
        listed["documents"],
        json!(["archive/Today.md", "notes/Welcome.md"])
    );
}

#[test]
fn the_persons_concurrent_edits_merge_with_the_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = lab_documents();
    let (list, _host) = host(&docs, tmp.path());
    let ours = docs.doc("notes/Welcome.md").unwrap();
    // The person's editor: a replica of the note, typing while the
    // extension tidies it.
    let person = LoroDoc::new();
    person
        .import(&ours.loro().export(ExportMode::Snapshot).unwrap())
        .unwrap();
    CrdtDocuments::text_of(&person)
        .insert(2, "Big ")
        .unwrap();
    person.commit();
    run(&list, "LAB_TIDY_TIDY", &json!({ "path": "notes/Welcome.md" }));
    // Each side's changes to the other.
    let from_person = person
        .export(ExportMode::updates(&ours.loro().oplog_vv()))
        .unwrap();
    let from_ext = ours
        .loro()
        .export(ExportMode::updates(&person.oplog_vv()))
        .unwrap();
    ours.apply_remote(&from_person).unwrap();
    person.import(&from_ext).unwrap();
    let merged = "---\ntidy: true\n---\n# Big Welcome\n\n— tidied\n";
    assert_eq!(docs.text("notes/Welcome.md").as_deref(), Some(merged));
    assert_eq!(CrdtDocuments::text_of(&person).to_string(), merged);
    // The person's replica tells the extension's change from theirs.
    assert!(
        architect_extensions::documents::authors_of(&person)
            .contains(&"extension:lab.tidy".to_owned())
    );
    // Undo takes back the extension's step only.
    docs.undo("notes/Welcome.md");
    assert_eq!(
        docs.text("notes/Welcome.md").as_deref(),
        Some("# Big Welcome\n")
    );
}

const BARE: &str = r#"
id = "lab.bare"
name = "Bare"
version = "0.1.0"
kind = "luau"
api = "0.1"
"#;

const READER: &str = r#"
id = "lab.reader"
name = "Reader"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
reads = ["documents"]
"#;

fn load(host: &Host, dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    host.load(&Package::read(dir).unwrap(), Origin::Dev).unwrap();
}

fn refused(r: Result<Value, HostError>) -> String {
    match r {
        Err(HostError::Refused { capability, .. }) => capability,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn without_the_capability_it_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = lab_documents();
    let (_list, host) = host(&docs, tmp.path());
    load(&host, &tmp.path().join("bare"), BARE);
    load(&host, &tmp.path().join("reader"), READER);
    let bare = host.api("lab.bare");
    let reader = host.api("lab.reader");
    let create = || HostRequest::CreateDocument {
        path: "notes/x.md".to_owned(),
        text: String::new(),
    };
    let list = || HostRequest::ListDocuments { under: None };
    assert_eq!(refused(block_on(bare.call(list()))), "reads.documents");
    assert_eq!(
        refused(block_on(bare.call(HostRequest::WatchDocuments {
            action: Some("LAB_BARE_X".to_owned()),
            script: None,
            function: None,
        }))),
        "reads.documents"
    );
    // Reading is not changing.
    let listed = block_on(reader.call(list())).unwrap();
    assert_eq!(listed[0]["path"], "notes/Welcome.md", "{listed}");
    assert_eq!(refused(block_on(reader.call(create()))), "writes.documents");
    for request in [
        HostRequest::EditDocument {
            path: "notes/Welcome.md".to_owned(),
            edits: vec![DocEdit::Append {
                text: "!".to_owned(),
            }],
            name: None,
        },
        HostRequest::RenameDocument {
            path: "notes/Welcome.md".to_owned(),
            to: "notes/W.md".to_owned(),
        },
        HostRequest::MoveDocument {
            path: "notes/Welcome.md".to_owned(),
            folder: "archive".to_owned(),
        },
        HostRequest::DeleteDocument {
            path: "notes/Welcome.md".to_owned(),
        },
    ] {
        assert_eq!(refused(block_on(reader.call(request))), "writes.documents");
    }
    assert_eq!(docs.text("notes/Welcome.md").as_deref(), Some("# Welcome\n"));
    assert!(docs.list(None).unwrap().len() == 1, "nothing changed");

    // With it: a path outside the spaces, or `..`, is not one.
    let tidy = host.api(TIDY);
    let bad = block_on(tidy.call(HostRequest::CreateDocument {
        path: "../etc/x".to_owned(),
        text: String::new(),
    }));
    assert!(matches!(bad, Err(HostError::Failed { .. })), "{bad:?}");
    let nowhere = block_on(tidy.call(HostRequest::CreateDocument {
        path: "elsewhere/x.md".to_owned(),
        text: String::new(),
    }));
    assert!(matches!(nowhere, Err(HostError::NotFound { .. })), "{nowhere:?}");
    block_on(tidy.call(HostRequest::DeleteDocument {
        path: "notes/Welcome.md".to_owned(),
    }))
    .unwrap();
    assert!(docs.list(None).unwrap().is_empty());
}
