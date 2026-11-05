//! The dev loop on Navigator (#272): `EXT_CLONE` of the linked built-in
//! writes its crate (from this checkout, its `[source]`) to the
//! development folder and builds it as a process, which overrides the
//! built-in under its own id; a save rebuilds it and swaps the new program
//! in; and the built-in comes back.
//!
//! It builds Navigator's binary from a crate of its own (in
//! `target/ext-dev`, kept between runs), so it is not in the default run:
//! `cargo test -p architect-navigator-extension --test dev_loop -- --ignored --nocapture`.

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
use architect_navigator_extension::{ID, NavigatorExtension};
use architect_settings::Registry;
use futures::executor::block_on;

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

/// What `NAVIGATOR_STATE` answers with no navigator on show.
fn answer(list: &ActionList) -> String {
    match block_on(list.run_json_async("NAVIGATOR_STATE", b"")) {
        Ok(v) => String::from_utf8_lossy(&v).into_owned(),
        Err(e) => e.to_string(),
    }
}

#[test]
#[ignore = "builds Navigator's crate on its own; run with --ignored"]
fn clone_the_linked_navigator_edit_it_and_go_back() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&NavigatorExtension)
        .unwrap()
        .with_dev_root(tmp.path().join("dev"))
        .with_vendored(GIT, repo())
        .with_build_target(repo().join("target/ext-dev"));
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert!(answer(&list).contains("not on show"));

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
    poll(&ext, "its process answers", Duration::from_secs(20), || {
        answer(&list).contains("not on show")
    });

    // A save: rebuilt and swapped in, saying it differently.
    let bridge = folder.join("src/bridge.rs");
    let text = std::fs::read_to_string(&bridge).unwrap();
    let edited = text.replacen("is not on show", "is not on show (dev)", 1);
    assert_ne!(edited, text);
    std::fs::write(&bridge, &edited).unwrap();
    let swap = poll(
        &ext,
        "the edit swapped in",
        Duration::from_secs(900),
        || answer(&list).contains("(dev)"),
    );
    println!("save to swapped in: {swap:?}");

    let _ = ext.end_dev(&folder);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    poll(&ext, "the built-in back", Duration::from_secs(10), || {
        !answer(&list).contains("(dev)")
    });
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
}
