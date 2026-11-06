//! One trait, two ways to run it (#183): the same `Board` linked into a
//! host and run as its own process adds the same things to the
//! interface, runs its startup hook, is told a context menu's target, and
//! fills its status item at run time — in process through its handle, out
//! of process over the wire.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, Invocation, InvocationSource};
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, now_ms};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_sandbox_board::{Board, ID};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::Value;

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn status(ext: &Extensions) -> String {
    ext.host()
        .contributions()
        .status(now_ms())
        .into_iter()
        .find(|s| s.owner == ID && s.item.id == "pins")
        .map(|s| s.item.text)
        .unwrap_or_default()
}

/// Runs the board through `ext`: what it adds, its hook, a pin from a
/// note's context menu. What it added.
fn exercise(ext: &Extensions, list: &ActionList, origin: Origin) -> BTreeSet<String> {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    until("its startup hook", || {
        status(ext).starts_with("Board ready")
    });
    let caller = ActionCaller::app()
        .with_invocation(Invocation::new(InvocationSource::ContextMenu).on("view:note", "note-1"));
    let out = block_on(list.run_json_async_with(&caller, "LAB_BOARD_PIN", b"")).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["pinned"], "note-1", "the context menu's target");
    assert_eq!(out["state"]["last_invoked"]["source"], "context_menu");
    until("its status item", || status(ext) == "1 pin: note-1");
    let reg = ext.host().contributions();
    // Its status item joins the shell's bar; it fills no slot.
    assert!(!reg.slots().contains_key("status"));
    assert_eq!(reg.open_path("intro.kf")[0].item.key(), "kf");
    reg.of(ID).unwrap().keys()
}

/// Linked in, then as a process: one after the other, as the board's
/// state is its process's.
#[test]
fn one_board_two_ways() {
    linked_in();
    as_a_process();
}

fn linked_in() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let keys = exercise(&ext, &list, Origin::Builtin);
    // Its Bases view type, drawn by its code in the app's document.
    let reg = ext.host().contributions();
    assert_eq!(reg.base_view("board").unwrap().owner, ID);
    assert!(ext.host().linked_view(ID, "base.board").is_some());
    assert!(ext.host().embeds().base_view("board").is_some());
    let _ = block_on(list.run_json_async("LAB_BOARD_CLEAR", b"")).unwrap();
    let _ = ext.disable(ID);
    assert!(
        ext.host().contributions().base_view("board").is_none(),
        "its base view type went with it"
    );
    assert!(ext.host().contributions().of(ID).is_none());
    assert!(status(&ext).is_empty(), "its status went with it");
    assert!(
        keys.contains("tab:board")
            && keys.contains("base_view:board")
            && keys.contains("context:view:note/LAB_BOARD_PIN")
    );
}

/// The same board as a process, overriding the linked one.
fn as_a_process() {
    let tmp = tempfile::tempdir().unwrap();
    let dev = tmp.path().join("board");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    // Its tour: the manifest names it, so its package has it.
    std::fs::create_dir_all(dev.join("tours")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tours/board-tour.styx"),
        dev.join("tours/board-tour.styx"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-sandbox-board"),
        dev.join("architect-sandbox-board"),
    )
    .unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let linked = exercise(&ext, &list, Origin::Builtin);
    let _ = block_on(list.run_json_async("LAB_BOARD_CLEAR", b"")).unwrap();

    // The development copy of its id: its program, the same value.
    let _ = ext.dev(&dev);
    let process = exercise(&ext, &list, Origin::Dev);
    assert!(
        ext.host().linked_view(ID, "board").is_none(),
        "drawn remotely now"
    );
    let out = block_on(list.run_json_async("LAB_BOARD_STATE", b"")).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_ne!(
        out["process"].as_u64(),
        Some(u64::from(std::process::id())),
        "its own process"
    );
    assert_eq!(linked, process, "the same contributions both ways");
    let _ = ext.end_dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
}
