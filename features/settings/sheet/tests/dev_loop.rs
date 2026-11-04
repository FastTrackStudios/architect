//! The dev loop on the settings page (#272): `EXT_CLONE` of the linked
//! built-in writes its crate (from this checkout, its `[source]`) to the
//! development folder and builds it as a process, which overrides the
//! built-in under its own id (and so still owns `APP_SETTINGS`, the app's
//! id the app lets `settings` own) and is told the same sections; a save
//! rebuilds it and swaps the new program in; and the built-in comes back.
//!
//! It builds the sheet's binary from a crate of its own (in
//! `target/ext-dev`, kept between runs), so it is not in the default run:
//! `cargo test -p architect-settings-sheet --test dev_loop -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_extensions::source::BuildState;
use architect_settings::Registry;
use architect_settings_sheet::{ID, SettingsSheet};
use futures::executor::block_on;
use serde_json::{Value, json};

const GIT: &str = "https://github.com/FastTrackStudios/architect";

fn repo() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
}

fn poll(ext: &Extensions, what: &str, within: Duration, ok: impl Fn() -> bool) -> Duration {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < within,
            "waited {within:?} for {what}: {:?}",
            ext.builds_now()
        );
        let _ = ext.reload_changed();
        std::thread::sleep(Duration::from_millis(100));
    }
    start.elapsed()
}

/// What `SETTINGS_LIST` says of its first section.
fn first(list: &ActionList) -> Option<Value> {
    let out = block_on(list.run_json_async("SETTINGS_LIST", b"")).ok()?;
    let v: Value = serde_json::from_slice(&out).ok()?;
    v["sections"].get(0).cloned()
}

#[test]
#[ignore = "builds the sheet's crate on its own; run with --ignored"]
fn clone_the_linked_sheet_edit_it_and_go_back() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    host.allow_reserved(ID, &["APP_SETTINGS"]);
    host.contribute_app(
        "settings.sections",
        vec![json!({"id": "workspace", "title": "Workspace", "group": ""})],
    );
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&SettingsSheet)
        .unwrap()
        .with_dev_root(tmp.path().join("dev"))
        .with_vendored(GIT, repo())
        .with_build_target(repo().join("target/ext-dev"));
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    poll(&ext, "the built-in told", Duration::from_secs(10), || {
        first(&list).is_some_and(|s| s["id"] == "workspace")
    });

    let cloned = ext.clone_to_dev(ID, None).unwrap();
    assert_eq!(cloned.id, ID, "the copy keeps the built-in's id");
    let folder = cloned.folder;
    let built = poll(&ext, "the first build", Duration::from_mins(30), || {
        matches!(
            ext.build_of(ID).map(|b| b.state),
            Some(BuildState::Built { .. })
        )
    });
    println!("first build and swap: {built:?}");
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Dev);
    poll(&ext, "its process told", Duration::from_secs(20), || {
        first(&list).is_some_and(|s| s["id"] == "workspace")
    });
    // ⌘, is still its window's.
    let opened = block_on(list.run_json_async("APP_SETTINGS", b"")).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&opened).unwrap(),
        json!({"open": "settings/settings"})
    );

    // A save: rebuilt and swapped in, its list saying more.
    let lib = folder.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    let edited = text.replacen(
        "\"settings\": s.settings().count(),",
        "\"settings\": s.settings().count(), \"dev\": true,",
        1,
    );
    assert_ne!(edited, text);
    std::fs::write(&lib, &edited).unwrap();
    let swap = poll(
        &ext,
        "the edit swapped in",
        Duration::from_secs(600),
        || first(&list).is_some_and(|s| s["dev"] == true),
    );
    println!("save to swapped in: {swap:?}");

    let _ = ext.end_dev(&folder);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    poll(&ext, "the built-in back", Duration::from_secs(10), || {
        first(&list).is_some_and(|s| s["dev"].is_null())
    });
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
}
