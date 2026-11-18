//! Two process views of one group show an edit in the same frame (#891,
//! `architect_extensions::barrier`).
//!
//! The extension is this test binary, started again as a child (the
//! `guest` test). Its two views stand in for Studio's arrangement and
//! mixer: both show one shared edit counter, which moves every 150 ms;
//! the arrangement draws each new edit at once, the mixer 30 ms later.
//! The host must never show them apart.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::arithmetic_side_effects
)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::barrier::{EPOCH_ATTR, GROUP_ATTR};
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteView, mounted_views};
use architect_extensions::{Origin, Package};
use blitz_dom::{BaseDocument, Document, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const ID: &str = "test.barrier";

/// The guest's shared state: which edit it is at.
static EDIT: AtomicU64 = AtomicU64::new(1);

/// One pane's view of the edit: what it last drew, `late` after the edit.
fn view(late: Duration) -> Element {
    let mut shown = use_signal(|| EDIT.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_millis(2)).await;
            let now = EDIT.load(Ordering::Relaxed);
            if now != *shown.peek() {
                futures_timer::Delay::new(late).await;
                shown.set(now);
            }
        }
    });
    let e = shown();
    rsx! {
        div {
            "data-architect-group": "studio",
            "data-architect-epoch": "{e}",
            p { id: "edit", "{e}" }
        }
    }
}

#[component]
fn Arrangement() -> Element {
    view(Duration::ZERO)
}

#[component]
fn Mixer() -> Element {
    view(Duration::from_millis(30))
}

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    // The shared state moves on its own, as an engine's does.
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_millis(150));
            EDIT.fetch_add(1, Ordering::Relaxed);
        }
    });
    Guest::new()
        .view("arrangement", Arrangement)
        .view("mixer", Mixer)
        .serve()
        .unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Barrier"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "arrangement"
title = "Arrangement"

[[views]]
id = "mixer"
title = "Mixer"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

#[derive(Props, Clone, PartialEq)]
struct PanesProps {
    processes: Processes,
}

#[allow(non_snake_case)]
fn Panes(props: PanesProps) -> Element {
    rsx! {
        div { style: "display: flex; width: 800px; height: 300px;",
            div { style: "width: 400px; height: 300px;",
                RemoteView { processes: props.processes.clone(), extension: ID, view: "arrangement" }
            }
            div { style: "width: 400px; height: 300px;",
                RemoteView { processes: props.processes, extension: ID, view: "mixer" }
            }
        }
    }
}

/// What each mounted view shows, in mount order.
fn shown(doc: &BaseDocument) -> Vec<Option<u64>> {
    mounted_views(doc)
        .into_iter()
        .map(|at| {
            let view = doc.subdoc(at)?.inner();
            let node = view.query_selector("#edit").ok()??;
            view.get_node(node)?.text_content().trim().parse().ok()
        })
        .collect()
}

#[test]
fn an_edit_shows_in_both_panes_of_a_group_in_the_same_frame() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    let pkg_dir = dir.path().join("pkg");
    std::fs::create_dir(&pkg_dir).unwrap();
    host.load(&package(&pkg_dir), Origin::Installed).unwrap();

    let vdom = VirtualDom::new_with_props(Panes, PanesProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 300, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    let waker = futures::task::noop_waker();
    let turn = |doc: &mut DioxusDocument| {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        shown(&doc.inner.borrow())
    };
    // Both views up.
    let up = Instant::now() + Duration::from_secs(30);
    while !matches!(turn(&mut doc).as_slice(), [Some(_), Some(_)]) {
        assert!(Instant::now() < up, "the views did not mount");
        std::thread::sleep(Duration::from_millis(2));
    }
    // A second of edits: after every turn of the window's loop (where a
    // frame would be drawn), both show the same one.
    let (mut edits, mut apart) = (std::collections::BTreeSet::new(), Vec::new());
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        let now = turn(&mut doc);
        if let [Some(a), Some(b)] = now.as_slice() {
            edits.insert(*a);
            if a != b {
                apart.push((*a, *b));
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(edits.len() >= 4, "only edits {edits:?} were shown");
    assert!(
        apart.is_empty(),
        "shown apart (arrangement, mixer): {apart:?}"
    );
    // The attributes the views set are the ones the host reads.
    assert_eq!(
        (GROUP_ATTR, EPOCH_ATTR),
        ("data-architect-group", "data-architect-epoch")
    );
}
