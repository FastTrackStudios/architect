//! Live documents (`architect_extensions::live`): an extension edits the
//! node's copy of a vault document of a kind it declares, not the file.
//!
//! The node is a `vault_collab::VaultCollab` over a vault holding a
//! `Songs/a.session` of kind `test.session` (a `crdt::file::DocFile`, the
//! file its own storage). A process extension (this test binary, started
//! again as a child, as in `process_theme.rs`) names it by path through
//! `guest::document`, and its replica's edit lands in the node's document
//! and the file, over the connection it already has to the host; an edit
//! made on the node reaches its replica. A linked one that may only read
//! follows the node and its edits stay its own. An extension that does
//! not declare the kind, or may not read documents, is refused, and so is
//! a session for a document the host never answered.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    clippy::arithmetic_side_effects
)]

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension, Handle};
use architect_extensions::live::{KindDoc, LiveDoc, RegistryDocuments, Replica};
use architect_extensions::process::{ProcessRunner, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::{Origin, Package};
use crdt::file::{DiskStorage, DocFile, MAIN};
use crdt::sync::SyncedDoc;
use crdt::{CrdtDoc, DocKind, SharedDocKinds};
use futures::executor::block_on;
use serde_json::{Value, json};
use vault_collab::VaultCollab;
use vault_live::Backend;

const KIND: &str = "test.session";
const PROCESS: &str = "test.sessions";
const LINKED: &str = "test.follower";
const OPEN: &str = "TEST_SESSIONS_OPEN";
const TYPE: &str = "TEST_SESSIONS_TYPE";
const TEXT: &str = "TEST_SESSIONS_TEXT";

fn kind() -> DocKind {
    DocKind::new(KIND, "Session").extension("session")
}

// ── The process extension (this binary, as a child) ────────────────────

/// The replica the process keeps.
static REPLICA: Mutex<Option<Replica>> = Mutex::new(None);

fn body(doc: &CrdtDoc) -> String {
    doc.loro().get_text("body").to_string()
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new()
        .action(OPEN, |args| async move {
            let path = args["path"].as_str().unwrap_or_default().to_owned();
            let lane = architect_extensions::guest::document(&path)
                .await
                .map_err(|e| e.to_string())?;
            let replica = lane.replica();
            replica.attached().await?;
            let info = serde_json::to_value(replica.info()).map_err(|e| e.to_string())?;
            *REPLICA.lock().unwrap() = Some(replica);
            Ok(info)
        })
        .action(TYPE, |args| async move {
            let text = args["text"].as_str().unwrap_or_default().to_owned();
            let doc = REPLICA
                .lock()
                .unwrap()
                .as_ref()
                .map(|r| r.doc().clone())
                .ok_or("nothing open")?;
            let body = doc.loro().get_text("body");
            body.insert(body.len_unicode(), &text)
                .map_err(|e| e.to_string())?;
            doc.loro().commit();
            Ok(Value::Null)
        })
        .action(TEXT, |_| async {
            let held = REPLICA.lock().unwrap();
            Ok(json!(held.as_ref().map(|r| body(r.doc()))))
        })
        .serve()
        .unwrap();
}

fn process_package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{PROCESS}"
name = "Sessions"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[capabilities]
reads = ["documents"]
writes = ["documents"]

[[doc_kinds]]
id = "{KIND}"
name = "Session"
files = ["session"]

[[actions]]
id = "{OPEN}"
name = "Open a session"
params = '{{"type":"object","properties":{{"path":{{"type":"string"}}}}}}'

[[actions]]
id = "{TYPE}"
name = "Type into it"
params = '{{"type":"object","properties":{{"text":{{"type":"string"}}}}}}'

[[actions]]
id = "{TEXT}"
name = "Its text"
effect = "read_only"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

// ── The linked follower ────────────────────────────────────────────────

static FOLLOWER: OnceLock<Handle> = OnceLock::new();

struct Follower;

impl Extension for Follower {
    fn manifest(&self) -> &'static str {
        r#"
id = "test.follower"
name = "Follower"
version = "0.1.0"
kind = "process"

[entry]
path = "follower-bin"

[capabilities]
reads = ["documents"]

[[doc_kinds]]
id = "test.session"
name = "Session"
files = ["session"]
"#
    }

    fn build(&self, ext: &mut Builder) {
        let _ = FOLLOWER.set(ext.handle());
    }
}

/// A data extension of `capabilities` and `doc_kinds` tables, loaded.
fn data_extension(host: &Host, dir: &Path, id: &str, tables: &str) {
    let dir = dir.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\nkind = \"data\"\n{tables}"),
    )
    .unwrap();
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
}

// ── The node ───────────────────────────────────────────────────────────

/// `main`'s text in the document file at `path`, read from its bytes as
/// another process would.
async fn file_text(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    let file = DocFile::from_bytes(bytes, &kind()).await.unwrap();
    let main = file.open_branch(MAIN).await.unwrap();
    main.loro().get_text("body").to_string()
}

/// A vault `v1` under `root` with `Songs/a.session` holding "made here",
/// its node's documents of kind `test.session`, and the document's id.
fn node(root: &Path, rt: &tokio::runtime::Runtime) -> (VaultCollab, uuid::Uuid) {
    let _in = rt.enter();
    let backend = Backend::single("v1", root.join("vault")).unwrap();
    let collab = VaultCollab::with_kinds(backend, root.join("crdt"), SharedDocKinds::new());
    collab.kinds().register(kind());
    std::fs::create_dir_all(root.join("vault/Songs")).unwrap();
    let id = rt.block_on(async {
        let file = DocFile::create(
            DiskStorage::open(root.join("vault/Songs/a.session")).unwrap(),
            &kind(),
        )
        .await
        .unwrap();
        let mut main = file.open_branch(MAIN).await.unwrap();
        main.transact("Type", |doc| {
            doc.get_text("body").insert(0, "made here").unwrap();
        })
        .await
        .unwrap();
        file.id().await.unwrap()
    });
    (collab, id)
}

/// What the host's extensions reach of `collab`: a vault path's document.
fn kind_documents(collab: &VaultCollab) -> RegistryDocuments {
    let resolve = collab.clone();
    RegistryDocuments::new(collab.registry().clone(), move |path| {
        let doc_id = resolve.document_of("v1", path)?;
        Some(KindDoc {
            doc_id,
            kind: resolve.kind_of(path)?.id,
        })
    })
}

/// The node's document `id`'s text now.
fn node_text(collab: &VaultCollab, rt: &tokio::runtime::Runtime, id: uuid::Uuid) -> String {
    rt.block_on(async { body(&collab.registry().doc(id).await.unwrap()) })
}

/// Asks until `ok` holds.
fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn run(host: &Host, id: &str, args: &Value) -> Result<Value, String> {
    block_on(host.list().run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

#[test]
fn a_process_extension_edits_the_nodes_copy_and_follows_it() {
    let tmp = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (collab, id) = node(tmp.path(), &rt);
    let at = tmp.path().join("vault/Songs/a.session");

    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(tmp.path())
        .with_restart(Restart::never());
    let host = Host::new(ActionList::new(), SharedSections::default())
        .with_runner(runner)
        .with_kind_documents(kind_documents(&collab));
    let pkg = tmp.path().join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    host.load(&process_package(&pkg), Origin::Dev).unwrap();

    // Named by its vault path: its id, its kind, writable.
    let info: LiveDoc =
        serde_json::from_value(run(&host, OPEN, &json!({ "path": "Songs/a.session" })).unwrap())
            .unwrap();
    assert_eq!(
        info,
        LiveDoc {
            doc_id: id,
            kind: KIND.to_owned(),
            path: "Songs/a.session".to_owned(),
            writable: true,
            node: None,
        }
    );
    assert_eq!(run(&host, TEXT, &Value::Null).unwrap(), json!("made here"));

    // Its edit lands in the node's document, and the node's file.
    run(&host, TYPE, &json!({ "text": ", typed by the extension" })).unwrap();
    let want = "made here, typed by the extension";
    until("the extension's edit in the node", || {
        node_text(&collab, &rt, id) == want
    });
    until("the extension's edit in the file", || {
        rt.block_on(file_text(&at)) == want
    });

    // An edit made on the node reaches its replica.
    rt.block_on(async {
        let doc = collab.registry().doc(id).await.unwrap();
        let body = doc.loro().get_text("body");
        body.insert(0, "Node: ").unwrap();
        doc.loro().commit();
    });
    until("the node's edit in the replica", || {
        run(&host, TEXT, &Value::Null).unwrap() == json!(format!("Node: {want}"))
    });

    // A path of no document of a kind is none.
    let err = run(&host, OPEN, &json!({ "path": "Songs/missing.session" })).unwrap_err();
    assert!(err.contains("missing.session"), "{err}");
    assert!(host.unload(PROCESS));
}

#[test]
fn a_linked_reader_follows_and_others_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (collab, id) = node(tmp.path(), &rt);
    let host = Host::new(ActionList::new(), SharedSections::default())
        .with_kind_documents(kind_documents(&collab));
    let ext = Extensions::new(host.clone(), Installs::new(tmp.path().join("installs")))
        .with_linked(&Follower)
        .unwrap();
    let _ = ext.sync();
    assert!(host.get(LINKED).is_some());

    // Linked in: the same call, over an in-memory lane. It may only read:
    // it follows the node, and its own edits stay its own.
    let handle = FOLLOWER.get().unwrap().clone();
    let replica = rt.block_on(async {
        let lane = handle.document("Songs/a.session").await.unwrap();
        assert_eq!(lane.doc().doc_id, id);
        assert!(!lane.doc().writable);
        let replica = lane.replica();
        replica.attached().await.unwrap();
        replica
    });
    assert_eq!(body(replica.doc()), "made here");
    rt.block_on(async {
        let doc = collab.registry().doc(id).await.unwrap();
        doc.loro().get_text("body").insert(0, "Node: ").unwrap();
        doc.loro().commit();
    });
    until("the node's edit in the follower", || {
        body(replica.doc()) == "Node: made here"
    });
    let mine = replica.doc().loro().get_text("body");
    mine.insert(0, "Mine: ").unwrap();
    replica.doc().loro().commit();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(node_text(&collab, &rt, id), "Node: made here");

    // Not its kind: refused, as the capability `doc_kinds.<kind>`.
    data_extension(
        &host,
        tmp.path(),
        "test.other",
        "[capabilities]\nreads = [\"documents\"]\nwrites = [\"documents\"]\n",
    );
    let open = HostRequest::LiveDocument {
        path: "Songs/a.session".to_owned(),
    };
    match block_on(host.api("test.other").call(open.clone())) {
        Err(HostError::Refused { capability, .. }) => {
            assert_eq!(capability, "doc_kinds.test.session");
        }
        other => panic!("not refused: {other:?}"),
    }
    // Its kind, but it may not read documents.
    data_extension(
        &host,
        tmp.path(),
        "test.blind",
        "[[doc_kinds]]\nid = \"test.session\"\nname = \"Session\"\nfiles = [\"session\"]\n",
    );
    match block_on(host.api("test.blind").call(open)) {
        Err(HostError::Refused { capability, .. }) => {
            assert_eq!(capability, "reads.documents");
        }
        other => panic!("not refused: {other:?}"),
    }
    // A session for a document the host never answered it: refused like
    // a document that does not exist, and nothing reaches the node.
    rt.block_on(async {
        let lane = architect_extensions::live::serve_in_memory(host.api("test.other"))
            .await
            .unwrap();
        let doc = CrdtDoc::ephemeral();
        let mut synced = SyncedDoc::new(id, doc.clone());
        doc.loro().get_text("body").insert(0, "sneaked").unwrap();
        doc.loro().commit();
        let end = tokio::time::timeout(Duration::from_secs(10), synced.run(lane.client()))
            .await
            .expect("an answer");
        assert!(end.unwrap_err().to_string().contains("unknown doc"));
    });
    assert_eq!(node_text(&collab, &rt, id), "Node: made here");
    drop(replica);
}
