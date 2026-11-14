//! The person's vault as extensions' documents (#706): the sandbox's
//! `lab.tidy` (`apps/sandbox/extensions/tidy`) lists, creates, edits,
//! renames and files notes in a vault on disk through
//! `documents::VaultDocuments`; the person's own edits through the same
//! source (an editor's save, a new note, a rename and a delete in the
//! navigator) are `doc.*` events it hears, each once; an extension's edit
//! merges into an open note's typing; what it keeps survives a restart.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::{Origin, Package};
use architect_navigator::documents::VaultDocuments;
use architect_navigator::{FileOp, NavigatorSource, SourceEvent, Subscription, VaultSource};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};
use vault::actions::OpenVault;

fn tidy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/sandbox/extensions/tidy")
}

/// A vault on disk with the lab's two folders and a note.
fn vault(root: &Path) -> VaultSource {
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::create_dir_all(root.join("archive")).unwrap();
    std::fs::write(root.join("notes/Welcome.md"), "# Welcome\n").unwrap();
    VaultSource::named(OpenVault::open(root).unwrap(), "Vault")
}

/// The app as the Architect app sets it up: a host over the vault's
/// documents, hearing the app's list, where the person's edits are raised;
/// `lab.tidy` loaded, its data under `data`.
struct App {
    list: ActionList,
    /// The app's own list, where the person's edits are raised.
    app_list: ActionList,
    host: Host,
    _events: Subscription,
}

fn app(source: &VaultSource, data: &Path) -> App {
    let docs = VaultDocuments::new(source.clone());
    let app_list = ActionList::new();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_documents(docs.clone())
        .with_data_root(data.to_path_buf());
    host.watch(&app_list);
    let events = docs.raise_on(&app_list);
    host.load(&Package::read(&tidy_dir()).unwrap(), Origin::Dev)
        .unwrap();
    App {
        list,
        app_list,
        host,
        _events: events,
    }
}

fn run(list: &ActionList, id: &str, args: &Value) -> Value {
    let out =
        block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

fn lines(list: &ActionList) -> Vec<String> {
    run(list, "LAB_TIDY_LIST", &Value::Null)["heard"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Waits until what `lab.tidy` heard holds `line`.
fn heard(list: &ActionList, line: &str) -> Vec<String> {
    let started = Instant::now();
    loop {
        let lines = lines(list);
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

fn on_disk(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

#[test]
fn an_extension_lists_creates_edits_renames_and_files_notes_in_the_vault() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let source = vault(&root);
    let app = app(&source, &tmp.path().join("data"));
    let list = &app.list;

    let listed = run(list, "LAB_TIDY_LIST", &Value::Null);
    assert_eq!(listed["documents"], json!(["notes/Welcome.md"]));
    let keys = run(list, "LAB_TIDY_WATCH", &Value::Null);
    assert_eq!(keys.as_array().map(Vec::len), Some(4), "{keys}");

    let made = run(list, "LAB_TIDY_NEW", &json!({ "title": "Inbox" }));
    assert_eq!(made["path"], "notes/Inbox.md", "{made}");
    assert_eq!(on_disk(&root, "notes/Inbox.md").as_deref(), Some("# Inbox\n"));
    heard(list, "doc.created notes/Inbox.md");

    let after = run(list, "LAB_TIDY_TIDY", &json!({ "path": "notes/Inbox.md" }));
    let tidied = "---\ntidy: true\n---\n# Inbox\n\n— tidied\n";
    assert_eq!(after, tidied);
    assert_eq!(on_disk(&root, "notes/Inbox.md").as_deref(), Some(tidied));
    heard(list, "doc.saved notes/Inbox.md");

    run(
        list,
        "LAB_TIDY_RENAME",
        &json!({ "path": "notes/Inbox.md", "to": "notes/Today.md" }),
    );
    heard(list, "doc.renamed notes/Inbox.md -> notes/Today.md");
    let filed = run(list, "LAB_TIDY_FILE", &json!({ "path": "notes/Today.md" }));
    assert_eq!(filed["path"], "archive/Today.md", "{filed}");
    assert_eq!(on_disk(&root, "archive/Today.md").as_deref(), Some(tidied));
    assert!(on_disk(&root, "notes/Today.md").is_none());
    // Each heard once: the host raised its own, the source's were left out.
    assert_eq!(
        heard(list, "doc.renamed notes/Today.md -> archive/Today.md"),
        [
            "doc.created notes/Inbox.md",
            "doc.saved notes/Inbox.md",
            "doc.renamed notes/Inbox.md -> notes/Today.md",
            "doc.renamed notes/Today.md -> archive/Today.md",
        ]
    );
}

#[test]
fn the_persons_own_edits_are_document_events() {
    let tmp = tempfile::tempdir().unwrap();
    let source = vault(&tmp.path().join("vault"));
    let app = app(&source, &tmp.path().join("data"));
    let list = &app.list;
    run(list, "LAB_TIDY_WATCH", &Value::Null);

    // Typing in a note, saved as the editor saves it.
    source
        .save("notes/Welcome.md", "# Welcome\n\nHello\n")
        .unwrap();
    heard(list, "doc.saved notes/Welcome.md");
    // A new note, a rename and a delete in the navigator.
    let made = source.create_note("notes").unwrap();
    heard(list, &format!("doc.created {made}"));
    source
        .write(&[FileOp::Move {
            from: made.clone(),
            to: "notes/Plan.md".to_owned(),
        }])
        .unwrap();
    heard(list, &format!("doc.renamed {made} -> notes/Plan.md"));
    source
        .write(&[FileOp::Trash("notes/Plan.md".to_owned())])
        .unwrap();
    let all = heard(list, "doc.deleted notes/Plan.md");
    assert_eq!(all.len(), 4, "each once: {all:?}");
}

#[test]
fn an_extensions_edit_merges_into_an_open_notes_typing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let source = vault(&root);
    let app = app(&source, &tmp.path().join("data"));
    // An open note as the Vault's editor keeps one (`apps/ui/src/vault.rs`
    // `VaultNote`): what the file held, and the typing not yet saved.
    let base = source.read_text("notes/Welcome.md").unwrap();
    let ours = "# Big Welcome\n".to_owned();
    let heard: Arc<Mutex<Option<String>>> = Arc::default();
    let h = Arc::clone(&heard);
    let s = source.clone();
    let _sub = source.subscribe(Box::new(move |e| {
        if let SourceEvent::FileModified(f) = e
            && f.path == "notes/Welcome.md"
        {
            *h.lock().unwrap() = s.read_text(&f.path);
        }
    }));
    run(
        &app.list,
        "LAB_TIDY_TIDY",
        &json!({ "path": "notes/Welcome.md" }),
    );
    let theirs = heard.lock().unwrap().clone().expect("the editor heard it");
    let merged = vault_doc::merge3(&base, &ours, &theirs);
    assert_eq!(merged, "---\ntidy: true\n---\n# Big Welcome\n\n— tidied\n");

    // The person saved first: the extension's edit merges with what is
    // on disk now, not the text it read.
    source
        .save("notes/Welcome.md", "---\ntidy: true\n---\n# Big Welcome\n\n— tidied\n")
        .unwrap();
    let docs = VaultDocuments::new(source.clone());
    let by = architect::action::Principal::extension("lab.tidy", None);
    use architect_extensions::documents::{DocEdit, Documents as _};
    let after = docs
        .edit(
            "notes/Welcome.md",
            &[DocEdit::Append {
                text: "- [ ] call Sam\n".to_owned(),
            }],
            "Add a task",
            &by,
        )
        .unwrap();
    assert!(after.changed);
    assert!(after.text.ends_with("— tidied\n- [ ] call Sam\n"), "{}", after.text);
    assert_eq!(on_disk(&root, "notes/Welcome.md").as_deref(), Some(after.text.as_str()));
}

#[test]
fn what_an_extension_keeps_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let source = vault(&tmp.path().join("vault"));
    let data = tmp.path().join("data");
    {
        let app = app(&source, &data);
        run(&app.list, "LAB_TIDY_WATCH", &Value::Null);
        source.save("notes/Welcome.md", "# Hello\n").unwrap();
        heard(&app.list, "doc.saved notes/Welcome.md");
    }
    // A new host, as the app makes at its next start, over the same data.
    let again = app(&source, &data);
    assert_eq!(lines(&again.list), ["doc.saved notes/Welcome.md"]);
}

#[test]
fn a_path_that_is_not_a_note_is_refused_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let source = vault(&root);
    let docs = VaultDocuments::new(source);
    let by = architect::action::Principal::extension("lab.tidy", None);
    use architect_extensions::documents::{DocEdit, DocError, Documents as _};
    assert!(matches!(
        docs.create("notes/x.txt", "", &by),
        Err(DocError::Invalid(_))
    ));
    assert!(matches!(
        docs.create("notes/Welcome.md", "", &by),
        Err(DocError::Exists(_))
    ));
    assert!(matches!(
        docs.read("notes/None.md"),
        Err(DocError::NotFound(_))
    ));
    assert!(matches!(
        docs.edit(
            "notes/Welcome.md",
            &[DocEdit::Delete { at: 5, len: 100 }],
            "x",
            &by
        ),
        Err(DocError::Invalid(_))
    ));
    assert_eq!(on_disk(&root, "notes/Welcome.md").as_deref(), Some("# Welcome\n"));
}

fn ext() -> architect::action::Principal {
    architect::action::Principal::extension("lab.tidy", None)
}

/// What `lab.tidy` heard, once `line` is among it and a moment more has
/// passed (so a second copy of anything would be there too).
fn settled(list: &ActionList, line: &str) -> Vec<String> {
    heard(list, line);
    std::thread::sleep(Duration::from_millis(1500));
    lines(list)
}

/// With the vault's watcher on (as the app runs it), an extension's
/// create, edit, rename and delete are each heard once: the host raised
/// them, the bridge left them out, and the watcher did not hear the
/// app's own writes as outside edits.
#[test]
fn on_a_watched_vault_each_extension_change_is_heard_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let source = vault(&root);
    source.watch().unwrap();
    let app = app(&source, &tmp.path().join("data"));
    let list = &app.list;
    run(list, "LAB_TIDY_WATCH", &Value::Null);
    // Each step heard before the next: `lab.tidy` keeps what it heard
    // with a read and a write of its data, so two of its hooks at once
    // would lose a line of its own.
    run(list, "LAB_TIDY_NEW", &json!({ "title": "Inbox" }));
    heard(list, "doc.created notes/Inbox.md");
    run(list, "LAB_TIDY_TIDY", &json!({ "path": "notes/Inbox.md" }));
    heard(list, "doc.saved notes/Inbox.md");
    run(
        list,
        "LAB_TIDY_RENAME",
        &json!({ "path": "notes/Inbox.md", "to": "notes/Today.md" }),
    );
    heard(list, "doc.renamed notes/Inbox.md -> notes/Today.md");
    block_on(app.host.api("lab.tidy").call(
        architect_extensions::hostcall::HostRequest::DeleteDocument {
            path: "notes/Today.md".to_owned(),
        },
    ))
    .unwrap();
    assert_eq!(
        settled(list, "doc.deleted notes/Today.md"),
        [
            "doc.created notes/Inbox.md",
            "doc.saved notes/Inbox.md",
            "doc.renamed notes/Inbox.md -> notes/Today.md",
            "doc.deleted notes/Today.md",
        ]
    );
}

/// The document events raised on the app's list since `since`, as
/// `<event> <path>` lines, once there are `n` of them.
fn raised(list: &ActionList, since: u64, n: usize) -> Vec<String> {
    let started = Instant::now();
    let mut at = since;
    let mut out = Vec::new();
    while out.len() < n {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "only {out:?} were raised"
        );
        let (events, next) = list.events_since(at, 64, Duration::from_millis(200));
        at = next;
        for e in events {
            if let architect::action::EventKind::Changed { topic, detail } = e.kind
                && topic.starts_with("doc.")
            {
                let d: Value = serde_json::from_str(&detail).unwrap();
                out.push(match d["path"].as_str() {
                    Some(p) => format!("{topic} {p}"),
                    None => format!("{topic} {} -> {}", d["from"].as_str().unwrap(), d["to"].as_str().unwrap()),
                });
            }
        }
    }
    out
}

/// A folder moved or deleted in the navigator is each note's event.
#[test]
fn a_folders_move_and_delete_are_each_notes_events() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("notes/Two.md"), "two\n").unwrap();
    let source = vault(&root);
    let app = app(&source, &tmp.path().join("data"));
    let since = app.app_list.latest();
    source
        .write(&[FileOp::MoveFolder {
            from: "notes".to_owned(),
            to: "moved".to_owned(),
        }])
        .unwrap();
    let mut moved = raised(&app.app_list, since, 2);
    moved.sort();
    assert_eq!(
        moved,
        [
            "doc.renamed notes/Two.md -> moved/Two.md",
            "doc.renamed notes/Welcome.md -> moved/Welcome.md",
        ]
    );
    let since = app.app_list.latest();
    source
        .write(&[FileOp::Trash("moved".to_owned())])
        .unwrap();
    let mut gone = raised(&app.app_list, since, 2);
    gone.sort();
    assert_eq!(gone, ["doc.deleted moved/Two.md", "doc.deleted moved/Welcome.md"]);
}

/// An edit that changes nothing raises no `doc.saved`, so a
/// format-on-save hook that finds nothing to do is not run again.
#[test]
fn an_edit_that_changes_nothing_raises_no_save() {
    let tmp = tempfile::tempdir().unwrap();
    let source = vault(&tmp.path().join("vault"));
    let app = app(&source, &tmp.path().join("data"));
    let list = &app.list;
    run(list, "LAB_TIDY_WATCH", &Value::Null);
    let out = block_on(app.host.api("lab.tidy").call(
        architect_extensions::hostcall::HostRequest::EditDocument {
            path: "notes/Welcome.md".to_owned(),
            edits: vec![architect_extensions::hostcall::DocEdit::Replace {
                text: "# Welcome\n".to_owned(),
            }],
            name: None,
        },
    ))
    .unwrap();
    assert_eq!(out, "# Welcome\n");
    run(list, "LAB_TIDY_NEW", &json!({ "title": "After" }));
    assert_eq!(settled(list, "doc.created notes/After.md"), ["doc.created notes/After.md"]);
}

/// Nothing is written through a symlinked folder: not a new note, not a
/// note renamed into it, not an edit of a file behind it.
#[cfg(unix)]
#[test]
fn nothing_is_written_through_a_symlinked_folder() {
    use architect_extensions::documents::{DocEdit, Documents as _};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("Theirs.md"), "theirs\n").unwrap();
    let source = vault(&root);
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
    source.vault().reload().unwrap();
    let docs = VaultDocuments::new(source);
    assert!(docs.create("linked/New.md", "x", &ext()).is_err());
    assert!(docs.rename("notes/Welcome.md", "linked/Welcome.md", &ext()).is_err());
    assert!(
        docs.edit(
            "linked/Theirs.md",
            &[DocEdit::Append { text: "!".to_owned() }],
            "x",
            &ext()
        )
        .is_err()
    );
    assert!(docs.delete("linked/Theirs.md", &ext()).is_err());
    let mut names: Vec<_> = std::fs::read_dir(&outside)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["Theirs.md"]);
    assert_eq!(on_disk(&outside, "Theirs.md").as_deref(), Some("theirs\n"));
    assert_eq!(on_disk(&root, "notes/Welcome.md").as_deref(), Some("# Welcome\n"));
}

/// An edit replaces the file whole (a new file renamed over it, nothing
/// left beside it), so a crash mid-write never leaves half a note.
#[cfg(unix)]
#[test]
fn an_edit_replaces_the_file_whole() {
    use std::os::unix::fs::MetadataExt as _;
    use architect_extensions::documents::{DocEdit, Documents as _};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let docs = VaultDocuments::new(vault(&root));
    let before = std::fs::metadata(root.join("notes/Welcome.md")).unwrap().ino();
    docs.edit(
        "notes/Welcome.md",
        &[DocEdit::Append { text: "more\n".to_owned() }],
        "x",
        &ext(),
    )
    .unwrap();
    let after = std::fs::metadata(root.join("notes/Welcome.md")).unwrap().ino();
    assert_ne!(before, after, "a new file, renamed over the old");
    let names: Vec<_> = std::fs::read_dir(root.join("notes"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["Welcome.md"], "no temporary file left");
}

/// A write is compare-and-swap: one made against text that is no longer
/// there writes nothing.
#[test]
fn a_save_against_stale_text_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    let source = vault(&root);
    source.save("notes/Welcome.md", "# Welcome\nnew typing\n").unwrap();
    assert!(!source
        .save_if("notes/Welcome.md", "# Welcome\n", "# Welcome\nextension\n")
        .unwrap());
    assert_eq!(
        on_disk(&root, "notes/Welcome.md").as_deref(),
        Some("# Welcome\nnew typing\n")
    );
    assert!(source
        .save_if("notes/Welcome.md", "# Welcome\nnew typing\n", "# Welcome\nboth\n")
        .unwrap());
}

/// Hidden parts and paths that are one file on a case- or
/// normalisation-blind file system are refused.
#[test]
fn hidden_and_colliding_paths_are_refused() {
    use architect_extensions::documents::{DocError, Documents as _};
    let tmp = tempfile::tempdir().unwrap();
    let docs = VaultDocuments::new(vault(&tmp.path().join("vault")));
    for bad in [".obsidian/x.md", "notes/.hidden.md", ".trash/Welcome.md"] {
        assert!(
            matches!(docs.create(bad, "", &ext()), Err(DocError::Invalid(_))),
            "{bad}"
        );
    }
    assert!(matches!(
        docs.create("notes/welcome.md", "", &ext()),
        Err(DocError::Exists(_))
    ));
    docs.create("notes/Caf\u{e9}.md", "", &ext()).unwrap();
    assert!(matches!(
        docs.create("notes/cafe\u{301}.md", "", &ext()),
        Err(DocError::Exists(_))
    ));
    assert!(matches!(
        docs.rename("notes/Welcome.md", "notes/CAF\u{c9}.md", &ext()),
        Err(DocError::Exists(_))
    ));
    // Its own name in another case is the same note: a rename.
    docs.rename("notes/Welcome.md", "notes/WELCOME.md", &ext())
        .unwrap();
}
