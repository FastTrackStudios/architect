//! A project's files the org's node holds, through the host (`file_list`,
//! `file_url`, `crate::held`): the host hands the call to the app's
//! source (the web app's, over the node's `architect_files::held` face)
//! only for an extension that may read the files; one that may not is
//! refused before the source is asked; a host with no source (a share
//! link's guest, the demo) answers `unsupported`, so a guest reads none.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use architect::action::ActionList;
use architect_extensions::held::{HeldFilesSource, HeldFuture};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use serde_json::json;

const ENGINE: &str = r#"
id = "lab.engine"
name = "Engine"
version = "0.1.0"
kind = "luau"
api = "0.1"
[capabilities]
reads = ["files"]
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
    host.load(&Package::read(dir).unwrap(), Origin::Dev)
        .unwrap();
}

/// A node that answers what it was asked, and remembers it.
#[derive(Default)]
struct Node(Mutex<Vec<String>>);

impl HeldFilesSource for Node {
    fn list(&self, place: String, folder: Option<String>) -> HeldFuture {
        self.0
            .lock()
            .unwrap()
            .push(format!("list {place} {folder:?}"));
        Box::pin(async move {
            Ok(json!({
                "place": place,
                "folder": folder.unwrap_or_default(),
                "files": [{"path": "Songs/Joy/a.wav", "size": 4, "sha256": "ab", "url": "https://n.test/x?ticket=t"}],
                "expires_us": 1,
            }))
        })
    }

    fn url(&self, place: String, path: String) -> HeldFuture {
        self.0.lock().unwrap().push(format!("url {place} {path}"));
        Box::pin(async move {
            Ok(json!({"url": format!("https://n.test/{path}?ticket=t"), "expires_us": 1}))
        })
    }
}

fn list() -> HostRequest {
    HostRequest::FileList {
        place: "vault:notes".to_owned(),
        folder: Some("Songs/Joy".to_owned()),
    }
}

fn url() -> HostRequest {
    HostRequest::FileUrl {
        place: "vault:notes".to_owned(),
        path: "Songs/Joy/a.wav".to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_extension_that_reads_files_lists_and_reads_a_project_through_the_app() {
    let tmp = tempfile::tempdir().unwrap();
    let node = Arc::new(Node::default());
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()))
        .with_held_files(node.clone());
    load(&host, &tmp.path().join("engine"), ENGINE);
    load(&host, &tmp.path().join("reader"), READER);

    let listed = host.api("lab.engine").call(list()).await.unwrap();
    assert_eq!(listed["files"][0]["path"], "Songs/Joy/a.wav", "{listed}");
    let ticket = host.api("lab.engine").call(url()).await.unwrap();
    assert_eq!(ticket["url"], "https://n.test/Songs/Joy/a.wav?ticket=t");
    assert_eq!(
        *node.0.lock().unwrap(),
        [
            "list vault:notes Some(\"Songs/Joy\")",
            "url vault:notes Songs/Joy/a.wav"
        ]
    );

    // Reading the documents is not reading the files: refused, and the
    // node is never asked.
    for request in [list(), url()] {
        match host.api("lab.reader").call(request).await {
            Err(HostError::Refused { capability, .. }) => assert_eq!(capability, "reads.files"),
            other => panic!("not refused: {other:?}"),
        }
    }
    assert_eq!(node.0.lock().unwrap().len(), 2);

    // Signed out: taken away.
    host.set_held_files(None);
    assert!(matches!(
        host.api("lab.engine").call(list()).await,
        Err(HostError::Unsupported { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_with_no_project_files_answers_unsupported() {
    // A share link's guest, or the demo: the app gives no source, so an
    // extension granted the files reads nothing past what the link hands.
    let tmp = tempfile::tempdir().unwrap();
    let guest = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    load(&guest, &tmp.path().join("engine"), ENGINE);
    for request in [list(), url()] {
        assert!(matches!(
            guest.api("lab.engine").call(request).await,
            Err(HostError::Unsupported { .. })
        ));
    }
}

#[test]
fn the_calls_are_plain_json() {
    let r: HostRequest =
        serde_json::from_str(r#"{"op":"file_list","place":"vault:notes"}"#).unwrap();
    assert_eq!(
        r,
        HostRequest::FileList {
            place: "vault:notes".to_owned(),
            folder: None
        }
    );
    assert_eq!(r.op(), "file_list");
    let r: HostRequest =
        serde_json::from_str(r#"{"op":"file_url","place":"root:x","path":"a.wav"}"#).unwrap();
    assert_eq!(r.op(), "file_url");
}
