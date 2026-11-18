//! The documents service over `architect::serve_local`, as the app's host
//! reaches it: the generated client, the gate in the service.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect::resource::Scope;
use architect_documents::{
    Access, DocsError, DocumentsClient, DocumentsServer, Edit, Gate, MemoryStore, MetaQuery,
};

/// A person's grants: `lab.reader` reads, `lab.writer` reads and writes.
struct Grants;

impl Gate for Grants {
    fn allows(&self, extension: &str, access: Access) -> bool {
        match access {
            Access::Own => true,
            Access::Read => matches!(extension, "lab.reader" | "lab.writer"),
            Access::Write => extension == "lab.writer",
        }
    }
}

async fn client() -> (std::sync::Arc<Scope>, DocumentsClient) {
    let store = MemoryStore::new().with("notes/Welcome.md", "# Welcome\n");
    let server = DocumentsServer::new(store.clone(), store, Grants);
    let scope = Scope::new();
    let local = architect::serve_local(server, &scope);
    let client: DocumentsClient = local.establish().await.expect("establish");
    (scope, client)
}

/// The service's own error, past the transport's.
fn user<T: std::fmt::Debug>(r: Result<T, vox::VoxError<DocsError>>) -> DocsError {
    match r {
        Err(vox::VoxError::User(e)) => *e,
        other => panic!("no service error: {other:?}"),
    }
}

fn refused<T: std::fmt::Debug>(r: Result<T, vox::VoxError<DocsError>>) -> String {
    match user(r) {
        DocsError::Refused { capability, .. } => capability,
        other => panic!("not refused: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_reaches_the_documents_through_the_gate() {
    let (_scope, docs) = client().await;
    let s = |x: &str| x.to_owned();

    // Reading needs reads; changing needs writes, on every transport.
    assert_eq!(
        refused(docs.list(s("lab.bare"), None).await),
        "reads.documents"
    );
    assert_eq!(
        refused(docs.create(s("lab.reader"), s("notes/x.md"), s("")).await),
        "writes.documents"
    );
    let listed = docs.list(s("lab.reader"), None).await.unwrap();
    assert_eq!(listed[0].path, "notes/Welcome.md");
    assert_eq!(listed[0].space.as_deref(), Some("notes"));

    let made = docs
        .create(s("lab.writer"), s("notes/Inbox.md"), s("# Inbox\n"))
        .await
        .unwrap();
    assert_eq!(made.name, "Inbox.md");
    let edited = docs
        .edit(
            s("lab.writer"),
            s("notes/Inbox.md"),
            vec![
                Edit::Insert {
                    at: 0,
                    text: s("> "),
                },
                Edit::Append {
                    text: s("- [ ] call Sam\n"),
                },
            ],
            s("Tidy"),
        )
        .await
        .unwrap();
    assert_eq!(edited.text, "> # Inbox\n- [ ] call Sam\n");
    assert!(edited.changed);
    let same = docs
        .edit(
            s("lab.writer"),
            s("notes/Inbox.md"),
            vec![Edit::Replace {
                text: edited.text.clone(),
            }],
            s("Same"),
        )
        .await
        .unwrap();
    assert!(!same.changed);
    docs.rename(s("lab.writer"), s("notes/Inbox.md"), s("archive/Inbox.md"))
        .await
        .unwrap();
    assert_eq!(
        docs.read(s("lab.reader"), s("archive/Inbox.md"))
            .await
            .unwrap(),
        edited.text
    );
    // A path that is not one is refused before the store sees it.
    assert!(matches!(
        user(docs.read(s("lab.reader"), s("../etc/passwd")).await),
        DocsError::Invalid(_)
    ));
    docs.delete(s("lab.writer"), s("archive/Inbox.md"))
        .await
        .unwrap();

    // Each extension's own data, apart from the others'.
    docs.set_data(s("lab.reader"), s("k"), Some(s("1")))
        .await
        .unwrap();
    assert_eq!(
        docs.data(s("lab.reader"), s("k")).await.unwrap(),
        Some(s("1"))
    );
    assert_eq!(docs.data(s("lab.writer"), s("k")).await.unwrap(), None);

    // The memory store has no metadata: said, not faked.
    assert!(matches!(
        user(
            docs.metadata(
                s("lab.reader"),
                MetaQuery {
                    path: Some(s("notes/Welcome.md")),
                    ..MetaQuery::default()
                }
            )
            .await
        ),
        DocsError::Unsupported(_)
    ));
}

/// A document's undo through the service (#844): each edit one step, by
/// its name; undo and redo walk them, gated as a write; a step is taken
/// back only while the text is as it left it (in memory: no merge).
#[tokio::test(flavor = "multi_thread")]
async fn undo_and_redo_walk_a_documents_steps() {
    let (_scope, docs) = client().await;
    let s = |x: &str| x.to_owned();
    let path = || s("notes/Welcome.md");
    let append = |t: &str| vec![Edit::Append { text: s(t) }];
    // Nothing to undo yet.
    assert_eq!(docs.undo(s("lab.writer"), path()).await.unwrap(), None);
    assert_eq!(
        refused(docs.undo(s("lab.reader"), path()).await),
        "writes.documents"
    );
    docs.edit(s("lab.writer"), path(), append("A\n"), s("Note A"))
        .await
        .unwrap();
    docs.edit(s("lab.writer"), path(), append("B\n"), s("Note B"))
        .await
        .unwrap();
    let undone = docs.undo(s("lab.writer"), path()).await.unwrap().unwrap();
    assert_eq!(
        (undone.step.as_str(), undone.text.as_str()),
        ("Note B", "# Welcome\nA\n")
    );
    let undone = docs.undo(s("lab.writer"), path()).await.unwrap().unwrap();
    assert_eq!(undone.text, "# Welcome\n");
    let redone = docs.redo(s("lab.writer"), path()).await.unwrap().unwrap();
    assert_eq!(
        (redone.step.as_str(), redone.text.as_str()),
        ("Note A", "# Welcome\nA\n")
    );
    // A new edit: nothing to redo any more.
    docs.edit(s("lab.writer"), path(), append("C\n"), s("Note C"))
        .await
        .unwrap();
    assert_eq!(docs.redo(s("lab.writer"), path()).await.unwrap(), None);
    assert_eq!(
        docs.read(s("lab.reader"), path()).await.unwrap(),
        "# Welcome\nA\nC\n"
    );
}
