//! A process extension gets the app's theme as data (wire 3): as it
//! connects, before any view opens, and again on every switch, to
//! `Guest::theme`, `Guest::on_theme` and a view's `guest::use_theme`.
//!
//! The extension is this test binary, started again as a child (as in
//! `remote_view.rs`): the `guest` test serves when the host's socket is
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
use std::sync::Mutex;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::{Guest, use_theme};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::wire::{Edit, Frame, THEMED_WIRE, WIRE, WIRE_MIN};
use architect_extensions::{Origin, Package};
use architect_theme::{Appearance, LayerKind, ThemeStore};
use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::executor::block_on;
use serde_json::json;

const ID: &str = "test.themed";
const SEEN: &str = "TEST_THEMED_SEEN";

/// Every theme the process was told, as `revision brand`, in order.
static TOLD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A view that colours itself from the theme as data.
#[component]
fn Swatch() -> Element {
    let text = use_theme().map_or_else(
        || "no theme".to_owned(),
        |t| format!("swatch {} {}", t.revision(), t.value("brand")),
    );
    rsx! { p { "{text}" } }
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new()
        .view("swatch", Swatch)
        .on_theme(|t| {
            TOLD.lock()
                .unwrap()
                .push(format!("{} {}", t.revision(), t.value("brand")));
        })
        .action(SEEN, |_| async {
            let now = Guest::theme().map(|t| format!("{} {}", t.revision(), t.value("brand")));
            Ok(json!({ "told": *TOLD.lock().unwrap(), "now": now }))
        })
        .serve()
        .unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Themed"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[actions]]
id = "{SEEN}"
name = "What it was told"
effect = "read_only"

[[views]]
id = "swatch"
title = "Swatch"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

fn seen(host: &Host) -> serde_json::Value {
    let out = block_on(host.list().run_json_async(SEEN, b"")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

/// Asks until `ok` holds.
fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn host(sockets: &Path, store: &ThemeStore) -> (Host, Processes) {
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(sockets)
        .with_restart(Restart::never())
        .with_theme(store.clone());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    (host, processes)
}

/// Every text the view wrote, in order.
fn texts(frame: &Frame) -> Vec<String> {
    let Frame::Edits { edits } = frame else {
        return Vec::new();
    };
    edits
        .iter()
        .filter_map(|e| match e {
            Edit::CreateText { value } | Edit::SetText { value } => Some(value.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_guest_gets_the_theme_on_open_and_on_every_switch() {
    assert_eq!((WIRE_MIN, THEMED_WIRE, WIRE), (2, 3, 3));
    let dir = tempfile::tempdir().unwrap();
    let store = ThemeStore::default();
    let (host, processes) = host(dir.path(), &store);
    host.load(&package(dir.path()), Origin::Dev).unwrap();

    // As it connected, before anything asked: the theme showing.
    let first = format!("{} {}", store.revision(), store.current().value("brand"));
    let now = seen(&host);
    assert_eq!(now["told"], json!([&first]));
    assert_eq!(now["now"], json!(&first));

    // A view opened now has it from its first render.
    let process = processes.get(ID).unwrap();
    let mut session = process.open("swatch").unwrap();
    let mut written = Vec::new();
    block_on(async {
        while !written
            .iter()
            .any(|t: &String| t == &format!("swatch {first}"))
        {
            let frame = tokio_timeout(session.frames.next()).await.expect("a frame");
            written.extend(texts(&frame));
        }
    });

    // A switch (THEME_SET's effect): told once more, the view redrawn.
    store.update(LayerKind::Pick, |l| l.appearance = Some(Appearance::Light));
    let second = format!("{} {}", store.revision(), store.current().value("brand"));
    assert_ne!(first, second);
    until("the switch to reach it", || {
        seen(&host)["told"] == json!([first.clone(), second.clone()])
    });
    block_on(async {
        while !written
            .iter()
            .any(|t: &String| t == &format!("swatch {second}"))
        {
            let frame = tokio_timeout(session.frames.next()).await.expect("a frame");
            written.extend(texts(&frame));
        }
    });

    // A write that changes nothing sends nothing.
    assert!(!store.set_system(architect_theme::Scheme::Dark));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(seen(&host)["told"].as_array().unwrap().len(), 2);
    drop(session);
    assert!(host.unload(ID));
}

/// `next` within ten seconds, or `None`.
async fn tokio_timeout<T>(next: impl Future<Output = Option<T>>) -> Option<T> {
    let deadline = futures_timer_like(Duration::from_secs(10));
    futures::pin_mut!(next, deadline);
    match futures::future::select(next, deadline).await {
        futures::future::Either::Left((got, _)) => got,
        futures::future::Either::Right(_) => None,
    }
}

/// A timer without a runtime: a thread that sleeps.
fn futures_timer_like(after: Duration) -> impl Future<Output = ()> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let _ = tx.send(());
    });
    async move {
        let _ = rx.await;
    }
}
