//! The vault's query service through the host (`query_vault`): the host
//! holds only the service's client (`Host::with_vault_query`), here served
//! in process (`architect::serve_local`) over an index kept from markdown,
//! as an app over its own vault index. An extension that may read the
//! documents queries; one that may not is refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::documents::CrdtDocuments;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::VaultScope;
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use vault_query::Built;
use vault_query::index::{Note, NoteSource};
use vault_query::QueryEngine;

/// Notes held as markdown: an app's vault, as the index sees it.
struct Markdown(Vec<(&'static str, &'static str)>);

impl NoteSource for Markdown {
    fn revision(&self) -> u64 {
        0
    }
    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        for (path, text) in self.0.iter().skip(from).take(n) {
            each(Note {
                path,
                stamp: 1,
                text: vault_query::Text::Here(text),
                parsed: None,
                mtime: 0,
                is_base: std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("base")),
            });
        }
        (0, self.0.len())
    }
}

const READER: &str = r#"
id = "lab.reader"
name = "Reader"
version = "0.1.0"
kind = "luau"
api = "0.1"
[capabilities]
reads = ["documents"]
"#;

const BARE: &str = r#"
id = "lab.bare"
name = "Bare"
version = "0.1.0"
kind = "luau"
api = "0.1"
"#;

fn load(host: &Host, dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    host.load(&Package::read(dir).unwrap(), Origin::Dev).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_extension_queries_the_vault_through_the_hosts_client() {
    let tmp = tempfile::tempdir().unwrap();
    let scope = architect::resource::Scope::new();
    let notes = Markdown(vec![
        ("Songs/A.md", "---\nstatus: todo\n---\n# A\n"),
        ("Songs/B.md", "---\nstatus: done\n---\n# B\n"),
        ("Notes/C.md", "# C\n"),
        ("Songs.base", "filters:\n  and:\n    - status != null\nviews:\n  - type: table\n    name: All\n    order: [file.name, status]\n"),
    ]);
    let engine = QueryEngine::new(Arc::new(Built::new(notes)));
    let client: vault_proto::query::VaultQueryClient =
        architect::serve_local(engine, &scope).establish().await.unwrap();

    let docs = || -> Arc<dyn architect_extensions::documents::Documents> {
        Arc::new(CrdtDocuments::new().with_space("notes"))
    };
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new())).with_vault(
        docs(),
        client.clone(),
        VaultScope::Person,
    );
    load(&host, &tmp.path().join("reader"), READER);
    load(&host, &tmp.path().join("bare"), BARE);

    let query = || HostRequest::QueryVault {
        path: Some("Songs.base".to_owned()),
        source: None,
        view: None,
        if_changed: None,
        limit: None,
    };
    let answer = host.api("lab.reader").call(query()).await.unwrap();
    assert_eq!(answer["total"], 2, "{answer}");
    assert_eq!(answer["rows"]["groups"][0]["rows"][0]["name"], "A");
    let hash = answer["hash"].as_str().unwrap().to_owned();
    let again = host
        .api("lab.reader")
        .call(HostRequest::QueryVault {
            if_changed: Some(hash),
            path: Some("Songs.base".to_owned()),
            source: None,
            view: None,
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(again["unchanged"], true);

    match host.api("lab.bare").call(query()).await {
        Err(HostError::Refused { capability, .. }) => assert_eq!(capability, "reads.documents"),
        other => panic!("not refused: {other:?}"),
    }
    // A host with no index says so.
    let none = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    load(&none, &tmp.path().join("reader2"), READER);
    assert!(matches!(
        none.api("lab.reader").call(query()).await,
        Err(HostError::Unsupported { .. })
    ));
    // A share guest's host, given the same client, never queries with it:
    // its documents are the share's, and the index is the whole vault's.
    let guest = Host::new(ActionList::new(), SharedSections::new(Registry::new())).with_vault(
        docs(),
        client.clone(),
        VaultScope::Guest,
    );
    load(&guest, &tmp.path().join("reader3"), READER);
    assert!(matches!(
        guest.api("lab.reader").call(query()).await,
        Err(HostError::Unsupported { .. })
    ));
    // Other documents in its place: the query service goes with the old.
    host.set_documents(docs());
    assert!(matches!(
        host.api("lab.reader").call(query()).await,
        Err(HostError::Unsupported { .. })
    ));
    scope.close().await;
}
