//! Surfaces kept in a file are there again when the file is opened again:
//! what the lab's "Agent UI" shows after a restart (#254).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use architect::action::Principal;
use architect_a2ui::store::{self, parse};
use architect_a2ui::{CATALOG_ID, Surfaces};
use serde_json::json;

fn hello() -> String {
    json!([ { "createSurface": { "surfaceId": "hello", "catalogId": CATALOG_ID,
        "components": [ { "id": "root", "component": "TextField", "label": "Name", "value": { "path": "/name" } } ] } } ])
    .to_string()
}

/// A file of its own, removed at the end.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn surfaces_in_a_file_are_there_after_a_restart() {
    let path = std::env::temp_dir().join(format!("a2ui-{}.a2ui", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let scratch = Scratch(path.clone());

    let agent = Principal::Service {
        name: "agent".to_owned(),
    };
    {
        let surfaces = Surfaces::open_file(&path).await.unwrap();
        assert!(surfaces.ids().is_empty());
        surfaces
            .apply(&parse(&hello()).unwrap(), Some(&agent))
            .unwrap();
        // What the person typed is the surface's too.
        surfaces
            .set_data("hello", "/name", Some(&json!("Ada")))
            .unwrap();
    }

    // The writer is the runtime's; give it the steps' moment to land.
    let mut again = None;
    for _ in 0..200 {
        let surfaces = Surfaces::open_file(&path).await.unwrap();
        if surfaces
            .get("hello")
            .is_some_and(|s| s.data["name"] == json!("Ada"))
        {
            again = Some(surfaces);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let again = again.expect("the surface, after a restart");
    let hello = again.get("hello").unwrap();
    assert_eq!(hello.author, Some(agent));
    assert_eq!(hello.components.len(), 1);
    drop(scratch);
}

#[tokio::test]
async fn a_file_of_another_kind_is_refused() {
    let path = std::env::temp_dir().join(format!("a2ui-other-{}.a2ui", std::process::id()));
    std::fs::write(&path, b"not a document").unwrap();
    let scratch = Scratch(path.clone());
    let err = Surfaces::open_file(&path).await.unwrap_err();
    assert!(matches!(err, store::SurfaceError::File(_)), "{err}");
    // Left as it was: nothing replaced what was there.
    assert_eq!(std::fs::read(&path).unwrap(), b"not a document");
    drop(scratch);
}
