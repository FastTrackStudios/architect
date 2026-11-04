//! The dev loop on the theme picker (#272): `EXT_CLONE` of the linked
//! built-in writes its crate (from this checkout, its `[source]`) to the
//! development folder and builds it as a process, which overrides the
//! built-in under its own id and answers through the host service; a
//! save rebuilds it and swaps the new program in; and the built-in comes
//! back.
//!
//! It builds the picker's binary from a crate of its own (in
//! `target/ext-dev`, kept between runs), so it is not in the default run:
//! `cargo test -p architect-theme-picker --test dev_loop -- --ignored --nocapture`.

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
use architect_theme::ThemeStore;
use architect_theme_picker::{ID, ThemePicker};
use futures::executor::block_on;
use serde_json::Value;

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

/// What `THEME_PICKER_FIND` says of Catppuccin Light.
fn detail(list: &ActionList) -> Option<String> {
    let out =
        block_on(list.run_json_async("THEME_PICKER_FIND", br#"{"query":"catppuccin light"}"#))
            .ok()?;
    let v: Value = serde_json::from_slice(&out).ok()?;
    v["items"][0]["detail"].as_str().map(str::to_owned)
}

#[test]
#[ignore = "builds the picker's crate on its own; run with --ignored"]
fn clone_the_linked_picker_edit_it_and_go_back() {
    let tmp = tempfile::tempdir().unwrap();
    let app = ActionList::new();
    architect_theme::actions::register(&app, &ThemeStore::default()).unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    host.set_app_list(app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&ThemePicker)
        .unwrap()
        .with_dev_root(tmp.path().join("dev"))
        .with_vendored(GIT, repo())
        .with_build_target(repo().join("target/ext-dev"));
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert_eq!(detail(&list).as_deref(), Some("Light"));

    let cloned = ext.clone_to_dev(ID, None).unwrap();
    assert_eq!(cloned.id, ID, "the copy keeps the built-in's id");
    let folder = cloned.folder;
    let first = poll(&ext, "the first build", Duration::from_mins(30), || {
        matches!(
            ext.build_of(ID).map(|b| b.state),
            Some(BuildState::Built { .. })
        )
    });
    println!("first build and swap: {first:?}");
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Dev);
    poll(
        &ext,
        "its process answering",
        Duration::from_secs(20),
        || detail(&list).as_deref() == Some("Light"),
    );

    // A save: rebuilt and swapped in, the picker's words its own now.
    let lib = folder.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    let edited = text.replacen(
        "\"light\" => \"Light\",",
        "\"light\" => \"Light (dev)\",",
        1,
    );
    assert_ne!(edited, text);
    std::fs::write(&lib, &edited).unwrap();
    let swap = poll(
        &ext,
        "the edit swapped in",
        Duration::from_secs(600),
        || detail(&list).as_deref() == Some("Light (dev)"),
    );
    println!("save to swapped in: {swap:?}");

    let _ = ext.end_dev(&folder);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert_eq!(detail(&list).as_deref(), Some("Light"));
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
}
