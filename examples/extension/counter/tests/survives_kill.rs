//! The counter, run by the example's host: its view in a pane reacts to
//! a click, its action answers over vox, and `kill -9` of its process
//! leaves the host running: the host starts it again (with backoff, as
//! `architect_extensions::process::Restart` says), and the view comes
//! back, drawn by the new process.
//!
//! The host's window is a headless Dioxus document on Blitz, mounting
//! `RemoteView` as the host's pane does; the click goes in through that
//! document, which forwards it to the view's sub-document.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::significant_drop_tightening
)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::host::SharedSections;
use architect_extensions::install::FolderRegistry;
use architect_extensions::process::{Processes, State};
use architect_extensions::remote::{RemoteView, mounted_views};
use blitz_dom::{BaseDocument, Document, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use example_extension_host::{COUNTER, counter_package, extensions};
use futures::executor::block_on;

const CLICKS: &str = "EXAMPLE_COUNTER_CLICKS";

#[derive(Props, Clone, PartialEq)]
struct PaneProps {
    processes: Processes,
}

/// The host's pane: the counter's view in it.
#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "width: 320px; height: 240px;",
            RemoteView { processes: props.processes, extension: COUNTER, view: "counter" }
        }
    }
}

fn window(processes: Processes) -> DioxusDocument {
    let vdom = VirtualDom::new_with_props(Pane, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
}

/// Runs the window (its tasks, its layout) until `until` holds.
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
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: the pane says {:?}, the view {:?}",
            in_window(&doc.inner.borrow(), ".remote-view-why"),
            in_view(&doc.inner.borrow(), "#count"),
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The text of `selector` in the pane's view.
fn in_view(doc: &BaseDocument, selector: &str) -> Option<String> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector(selector).ok()??;
    Some(view.get_node(node)?.text_content())
}

/// The text of `selector` in the host's own document.
fn in_window(doc: &BaseDocument, selector: &str) -> Option<String> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

/// Where the view's `selector` is, in the host's window.
fn view_point(doc: &BaseDocument, selector: &str) -> (f32, f32) {
    let at = mounted_views(doc)[0];
    let host = doc.get_node(at).unwrap().absolute_position(0.0, 0.0);
    let view = doc.subdoc(at).unwrap().inner();
    let node = view.query_selector(selector).unwrap().unwrap();
    let n = view.get_node(node).unwrap();
    let p = n.absolute_position(0.0, 0.0);
    let size = n.final_layout.size;
    (
        host.x + p.x + size.width / 2.0,
        host.y + p.y + size.height / 2.0,
    )
}

fn pointer(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
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
        active_pointers: std::sync::Arc::default(),
    }
}

fn click(doc: &mut DioxusDocument, (x, y): (f32, f32)) {
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        x,
        y,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, MouseEventButtons::None)));
}

fn clicks(list: &ActionList) -> Result<String, String> {
    block_on(list.run_json_async(CLICKS, b""))
        .map(|out| String::from_utf8(out).unwrap())
        .map_err(|e| e.to_string())
}

fn pid(processes: &Processes) -> Option<u32> {
    match processes.get(COUNTER)?.state() {
        State::Running { pid, .. } => Some(pid),
        _ => None,
    }
}

#[test]
fn the_counter_reacts_to_a_click_and_comes_back_after_kill_9() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, processes) = (ActionList::new(), Processes::default());
    let registry = Path::new(env!("CARGO_MANIFEST_DIR")).join("../registry");
    let ext = extensions(
        list.clone(),
        SharedSections::default(),
        processes.clone(),
        tmp.path().join("installed"),
        FolderRegistry::new(registry),
    );

    // The counter as a development copy: its manifest and this build of
    // its program.
    let program = Path::new(env!("CARGO_BIN_EXE_example-extension-counter"));
    let dir = counter_package(program, &tmp.path().join("dev")).unwrap();
    let events = ext.dev(&dir);
    assert!(
        matches!(
            events.as_slice(),
            [architect_extensions::host::Event::Loaded(_)]
        ),
        "{events:?}"
    );
    let first = pid(&processes).expect("it runs");

    // Its view, in the host's pane, drawn by its process.
    let mut doc = window(processes.clone());
    pump(&mut doc, "the view", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 0 times")
    });
    assert_eq!(
        in_view(&doc.inner.borrow(), "#pid"),
        Some(format!("Drawn by process {first}"))
    );

    // A click in the host reaches its handler; what it changed comes back.
    let more = view_point(&doc.inner.borrow(), "#more");
    click(&mut doc, more);
    pump(&mut doc, "the click", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 1 times")
    });
    // Its action ran in the same process, over vox.
    assert_eq!(clicks(&list).unwrap(), "1");

    // kill -9: the host runs on, and starts it again.
    let killed = std::process::Command::new("kill")
        .args(["-9", &first.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    pump(&mut doc, "a new process's view", |d| {
        pid(&processes).is_some_and(|p| p != first)
            && in_view(d, "#count").as_deref() == Some("Clicked 0 times")
    });
    let second = pid(&processes).unwrap();
    assert_eq!(
        in_view(&doc.inner.borrow(), "#pid"),
        Some(format!("Drawn by process {second}"))
    );
    assert_eq!(clicks(&list).unwrap(), "0", "a new process, a new count");
    assert_eq!(ext.host().ids(), [COUNTER], "still loaded");

    // Development over: unloaded, its process stopped, the pane says so.
    let _ = ext.end_dev(&dir);
    pump(&mut doc, "the unloaded placeholder", |d| {
        mounted_views(d).is_empty() && in_window(d, ".remote-view-why").is_some()
    });
    assert!(processes.get(COUNTER).is_none());
    assert!(list.info(CLICKS).is_none());
}
