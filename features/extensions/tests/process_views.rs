//! A process extension shows more than 64 views at once, gives a closed
//! view's place on its connection back, and past `wire::MAX_VIEWS` at
//! once says so instead of waiting.
//!
//! Each open view is a request in flight on the process's connection, and
//! vox caps those (64 by default): before wire 6 the 65th view waited
//! forever, and a closed view's request was never given back.
//!
//! The extension is this test binary, started again as a child (as in
//! `process_theme.rs`): the `guest` test serves when the host's socket is
//! in its environment and does nothing otherwise.
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
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{
    Process, ProcessRunner, Processes, Restart, State, ViewSession,
};
use architect_extensions::proto::SOCKET;
use architect_extensions::wire::{Edit, Frame, MAX_VIEWS, VIEWS_WIRE, WIRE};
use architect_extensions::{Origin, Package};
use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::executor::block_on;

const ID: &str = "test.tiles";

#[component]
fn Tile() -> Element {
    rsx! { p { "tile" } }
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new().view("tile", Tile).serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Tiles"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "tile"
title = "Tile"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

fn host(sockets: &Path) -> (Host, Processes) {
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(sockets)
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    (host, processes)
}

/// The process, once it runs.
fn running(processes: &Processes) -> Process {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(p) = processes.get(ID)
            && matches!(p.state(), State::Running { .. })
        {
            return p;
        }
        assert!(Instant::now() < deadline, "the process did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `n` views of the tile, each drawn once (its first frame).
fn open_drawn(process: &Process, n: usize) -> Vec<ViewSession> {
    let mut sessions: Vec<ViewSession> = (0..n)
        .map(|i| {
            process
                .open("tile")
                .unwrap_or_else(|e| panic!("view {i}: {e}"))
        })
        .collect();
    for (i, session) in sessions.iter_mut().enumerate() {
        let frame = block_on(within(Duration::from_secs(20), session.frames.next()))
            .flatten()
            .unwrap_or_else(|| panic!("view {i} of {n} never drew"));
        assert!(drew_tile(&frame), "view {i}: {frame:?}");
    }
    sessions
}

/// Waits until the process has heard every close.
fn drained(process: &Process) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while process.open_views() > 0 {
        assert!(
            Instant::now() < deadline,
            "{} views never closed",
            process.open_views()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn drew_tile(frame: &Frame) -> bool {
    let Frame::Edits { edits } = frame else {
        return false;
    };
    edits
        .iter()
        .any(|e| matches!(e, Edit::CreateText { value } if value == "tile"))
}

#[test]
fn more_than_64_views_at_once_closed_ones_given_back_and_a_clear_limit() {
    assert_eq!((VIEWS_WIRE, WIRE), (6, 6));
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    host.load(&package(dir.path()), Origin::Dev).unwrap();
    let process = running(&processes);

    // Well past vox's default of 64, all at once.
    let many = open_drawn(&process, 100);
    assert_eq!(process.open_views(), 100);
    drop(many);
    drained(&process);

    // Closed views are given back: more opens over the connection's life
    // than its lane takes requests at once, a hundred at a time.
    for _ in 0..3 {
        drop(open_drawn(&process, 100));
        drained(&process);
    }

    // The documented limit, refused with a reason, not a wait; closing one
    // makes room once the process has heard the close.
    let mut full = open_drawn(&process, MAX_VIEWS);
    let refused = process.open("tile").expect_err("one past MAX_VIEWS");
    assert!(
        refused.contains(&format!("shows {MAX_VIEWS} views already")),
        "{refused}"
    );
    full.pop();
    let deadline = Instant::now() + Duration::from_secs(10);
    let again = loop {
        match process.open("tile") {
            Ok(session) => break session,
            Err(e) => assert!(Instant::now() < deadline, "no room after a close: {e}"),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    full.push(again);
    let last = full.last_mut().unwrap();
    let frame = block_on(within(Duration::from_secs(20), last.frames.next())).flatten();
    assert!(
        frame.as_ref().is_some_and(drew_tile),
        "the view in the freed place drew: {frame:?}"
    );
    drop(full);
    assert!(host.unload(ID));
}

/// `next` within `limit`, or `None`.
async fn within<T>(limit: Duration, next: impl Future<Output = T>) -> Option<T> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(limit);
        let _ = tx.send(());
    });
    futures::pin_mut!(next);
    match futures::future::select(next, rx).await {
        futures::future::Either::Left((got, _)) => Some(got),
        futures::future::Either::Right(_) => None,
    }
}
