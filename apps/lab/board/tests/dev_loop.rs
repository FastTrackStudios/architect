//! The dev loop (#272): `EXT_CLONE` of the linked Board writes its crate
//! (from this checkout, its `[source]`) to the development folder and
//! builds it as a process, which overrides the built-in under its own id;
//! a save rebuilds it and swaps the new program in; a build that fails
//! keeps the last good one and says why; and the built-in comes back.
//!
//! It builds the Board's binary from a crate of its own (in
//! `target/ext-dev`, kept between runs), so it is not in the default run:
//! `cargo test -p architect-lab-board --test dev_loop -- --ignored --nocapture`.

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
use architect_lab_board::{Board, ID};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::Value;

const GIT: &str = "https://github.com/FastTrackStudios/architect";

fn repo() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
}

/// Polls like the app does until `ok`: how long it took.
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

fn state(list: &ActionList) -> Option<Value> {
    block_on(list.run_json_async("LAB_BOARD_STATE", b""))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn built(ext: &Extensions) -> bool {
    matches!(
        ext.build_of(ID).map(|b| b.state),
        Some(BuildState::Built { .. })
    )
}

#[test]
#[ignore = "builds the Board's crate on its own; run with --ignored"]
fn clone_the_linked_board_edit_it_break_it_and_go_back() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Board)
        .unwrap()
        .with_dev_root(tmp.path().join("dev"))
        .with_vendored(GIT, repo())
        .with_build_target(repo().join("target/ext-dev"));
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);

    // Cloned: its crate, under its own id; the built-in runs meanwhile.
    let cloned = ext.clone_to_dev(ID, None).unwrap();
    assert_eq!(cloned.id, ID, "the copy keeps the built-in's id");
    let folder = cloned.folder;
    assert!(folder.join("Cargo.toml").is_file() && folder.join("src/lib.rs").is_file());
    let says = ext.describe(&ext.chosen().into_iter().find(|c| c.runs.id == ID).unwrap());
    assert!(
        says.starts_with("built in (0.1.0), overridden by") && says.contains("building…"),
        "{says}"
    );
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);

    let first = poll(&ext, "the first build", Duration::from_mins(30), || {
        built(&ext)
    });
    println!("first build and swap: {first:?}");
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Dev);
    let pid = |list: &ActionList| state(list).and_then(|s| s["process"].as_u64());
    poll(&ext, "its process", Duration::from_secs(20), || {
        pid(&list).is_some_and(|p| p != u64::from(std::process::id()))
    });
    let first_pid = pid(&list);

    // A save: rebuilt and swapped in.
    let lib = folder.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    let edited = text.replacen(
        "\"process\": std::process::id(),",
        "\"process\": std::process::id(), \"edited\": true,",
        1,
    );
    assert_ne!(edited, text);
    std::fs::write(&lib, &edited).unwrap();
    let swap = poll(
        &ext,
        "the edit swapped in",
        Duration::from_secs(600),
        || state(&list).is_some_and(|s| s["edited"] == true),
    );
    println!("save to swapped in: {swap:?}");
    assert_ne!(pid(&list), first_pid, "a new process on the new program");

    // Broken: the problem, and the last good build runs on.
    std::fs::write(
        &lib,
        format!("{edited}\ncompile_error!(\"broken on purpose\");\n"),
    )
    .unwrap();
    poll(&ext, "the failed build", Duration::from_secs(600), || {
        matches!(
            ext.build_of(ID).map(|b| b.state),
            Some(BuildState::Failed { .. })
        )
    });
    let problems = ext.problems();
    let problem = problems
        .iter()
        .find(|p| p.message.starts_with("lab.board: its build failed"))
        .unwrap_or_else(|| panic!("{problems:?}"));
    assert!(problem.message.contains("broken on purpose"), "{problem:?}");
    assert!(problem.message.contains("the last good build runs"));
    assert_eq!(
        problem.file.as_deref(),
        Some(lib.display().to_string().as_str())
    );
    let says = ext.describe(&ext.chosen().into_iter().find(|c| c.runs.id == ID).unwrap());
    assert!(says.contains("failed: "), "{says}");
    assert_eq!(state(&list).unwrap()["edited"], true, "the last good build");
    let logs = ext.host().logs().since(ID, None, 50);
    assert!(
        logs.iter()
            .any(|l| l.source == "build" && l.text.contains("broken on purpose")),
        "{logs:?}"
    );

    // Back to the built-in.
    let _ = ext.end_dev(&folder);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert!(ext.build_of(ID).is_none());
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
    assert_eq!(
        state(&list).unwrap()["process"].as_u64(),
        Some(u64::from(std::process::id()))
    );
}
