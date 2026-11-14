//! Kanban-lite's bar (#176, #710 U2): in its own process, shown in a pane,
//! a card dragged from "To do" over "Doing" and dropped in "Done" moves
//! there, and the process's action says so.
//!
//! The extension is this test binary started again as a child (the
//! `guest` test). The host is a headless Dioxus document on Blitz,
//! mounting `RemoteView` as the app's pane does.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::as_conversions
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteView, mounted_views};
use architect_extensions::{Origin, Package};
use architect_sandbox_kanban::{ID, STATE};
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use futures::executor::block_on;

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    architect_sandbox_kanban::guest().serve().unwrap();
}

/// Its package, its program this binary.
fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/extension.toml"))
        .unwrap()
        .replace(
            "path = \"architect-sandbox-kanban\"",
            "path = \"guest\"\nargs = [\"guest\", \"--exact\", \"--nocapture\", \"--test-threads=1\"]",
        );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

#[derive(Props, Clone, PartialEq)]
struct PaneProps {
    processes: Processes,
}

#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "position: absolute; left: 40px; top: 20px; width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: ID, view: "board" }
        }
    }
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let waker = futures::task::noop_waker();
    loop {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        if until(&doc.inner.borrow()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The titles in column `n` (1-based), in order.
fn column(doc: &BaseDocument, n: usize) -> Vec<String> {
    let Some(at) = mounted_views(doc).first().copied() else {
        return Vec::new();
    };
    let view = doc.subdoc(at).unwrap().inner();
    view.query_selector_all(&format!(".column:nth-child({n}) .card"))
        .unwrap()
        .into_iter()
        .map(|c| view.get_node(c).unwrap().text_content())
        .collect()
}

/// The middle of the card titled `title`, in the window.
fn card_at(doc: &BaseDocument, title: &str) -> (f32, f32) {
    let at = mounted_views(doc)[0];
    let host = doc.get_node(at).unwrap().absolute_position(0.0, 0.0);
    let view = doc.subdoc(at).unwrap().inner();
    let card = view
        .query_selector_all(".card")
        .unwrap()
        .into_iter()
        .find(|c| view.get_node(*c).unwrap().text_content() == title)
        .unwrap();
    let n = view.get_node(card).unwrap();
    let p = n.absolute_position(0.0, 0.0);
    let s = n.final_layout().size;
    (host.x + p.x + s.width / 2.0, host.y + p.y + s.height / 2.0)
}

fn pointer((x, y): (f32, f32), buttons: MouseEventButtons) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons,
        mods: Modifiers::empty(),
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: Arc::default(),
    }
}

#[test]
fn a_card_drags_from_to_do_over_doing_into_done() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    host.load(&package(dir.path()), Origin::Installed).unwrap();

    let vdom = VirtualDom::new_with_props(Pane, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    pump(&mut doc, "the board", |d| column(d, 1).len() == 3);

    // Picked up in "To do", carried across "Doing", dropped in "Done".
    let from = card_at(&doc.inner.borrow(), "Write the chart");
    let to = (40.0 + 2.5 * 120.0, from.1);
    let held = MouseEventButtons::Primary;
    doc.handle_ui_event(UiEvent::PointerMove(pointer(from, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(from, held)));
    for i in 1..=30 {
        let t = i as f32 / 30.0;
        let at = (from.0 + (to.0 - from.0) * t, from.1);
        doc.handle_ui_event(UiEvent::PointerMove(pointer(at, held)));
        pump(&mut doc, "a step", |_| true);
    }
    doc.handle_ui_event(UiEvent::PointerUp(pointer(to, MouseEventButtons::None)));
    pump(&mut doc, "the drop", |d| {
        column(d, 3) == ["Write the chart"] && column(d, 1).len() == 2
    });

    let state = block_on(host.list().run_json_async(STATE, b"")).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&state).unwrap();
    assert_eq!(state[2][0], "Write the chart");
}
