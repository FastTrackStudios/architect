//! A process view in the active pane hears the keys without a click
//! (#889): when it has just opened (Keyflow's chart, opened and typed
//! into at once), and after the layout around it changed (Studio's mixer
//! closed and opened under the arrangement).
//!
//! The extension is this test binary, started again as a child (the
//! `guest` test).
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::missing_panics_doc)]

use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteView, mounted_views};
use architect_extensions::{Origin, Package};
use architect_workspace::{Pane, PaneId, Side, Target, View, Views, Workspace, WorkspaceView};
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, Code, Key, KeyState, Location, Modifiers,
    MouseEventButton, MouseEventButtons, Point, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const ID: &str = "test.keys";

/// The extension's view: how many keys it heard.
#[component]
fn Keys() -> Element {
    let mut heard = use_signal(|| 0_u32);
    rsx! {
        div {
            id: "keys",
            tabindex: "0",
            style: "width: 100%; height: 100%;",
            onkeydown: move |_| heard += 1,
            "{heard}"
        }
    }
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new().view("keys", Keys).serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Keys"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "keys"
title = "Keys"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

thread_local! {
    static PROCESSES: RefCell<Option<Processes>> = const { RefCell::new(None) };
    static WORKSPACE: RefCell<Option<Signal<Workspace>>> = const { RefCell::new(None) };
}

#[allow(non_snake_case)]
fn App() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(Pane::new("keys", "keys", "Keys"), Target::Tab)
            .unwrap();
        w
    });
    WORKSPACE.with(|w| *w.borrow_mut() = Some(workspace));
    let views = use_hook(|| {
        let processes = PROCESSES.with(|p| p.borrow().clone()).unwrap();
        Views::new()
            .view(View::new("keys", "Keys", move |_| {
                rsx! { RemoteView { processes: processes.clone(), extension: ID, view: "keys" } }
            }))
            .view(View::new("other", "Other", |_| rsx! { div { "other" } }))
    });
    rsx! {
        div { style: "width: 600px; height: 400px;",
            WorkspaceView { workspace, views }
        }
    }
}

fn key() -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: Key::Character("x".into()),
        code: Code::KeyX,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: Some("x".into()),
    }
}

fn press(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
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

/// A click in the middle of the view.
fn click_view(doc: &mut DioxusDocument) {
    let (x, y) = {
        let d = doc.inner.borrow();
        let at = mounted_views(&d)[0];
        let n = d.get_node(at).unwrap();
        let p = n.absolute_position(0.0, 0.0);
        let s = n.final_layout().size;
        (p.x + s.width / 2.0, p.y + s.height / 2.0)
    };
    doc.handle_ui_event(UiEvent::PointerMove(press(x, y, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(press(
        x,
        y,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(press(x, y, MouseEventButtons::None)));
}

/// How many keys the view says it heard.
fn heard(doc: &BaseDocument) -> Option<u32> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector("#keys").ok()??;
    view.get_node(node)?.text_content().trim().parse().ok()
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
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: the view heard {:?}",
            heard(&doc.inner.borrow())
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A key to the window, and the view hearing it (no click first).
fn type_one(doc: &mut DioxusDocument, now: u32, what: &str) {
    doc.handle_ui_event(UiEvent::KeyDown(key()));
    pump(doc, what, |d| heard(d) == Some(now));
}

#[test]
fn the_active_panes_process_view_hears_keys_without_a_click() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    let pkg = dir.path().join("pkg");
    std::fs::create_dir(&pkg).unwrap();
    host.load(&package(&pkg), Origin::Installed).unwrap();
    PROCESSES.with(|p| *p.borrow_mut() = Some(processes));

    let mut doc = DioxusDocument::new(
        VirtualDom::new(App),
        DocumentConfig {
            viewport: Some(Viewport::new(600, 400, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    pump(&mut doc, "the view", |d| heard(d) == Some(0));

    if std::env::var_os("KEYS_BASELINE").is_some() {
        click_view(&mut doc);
        pump(&mut doc, "the click", |_| true);
        type_one(&mut doc, 1, "BASELINE a key after a click");
        return;
    }
    // Just opened, active, never clicked: its keys reach it.
    type_one(&mut doc, 1, "a key to the view just opened");

    // A pane opens beside it and closes: the layout around it changes,
    // and the keys still reach it.
    let mut workspace = WORKSPACE.with(|w| *w.borrow()).unwrap();
    workspace.with_mut(|w| {
        w.open(
            Pane::new("other", "other", "Other"),
            Target::Split(Side::Bottom),
        )
        .unwrap();
    });
    pump(&mut doc, "the split", |_| true);
    workspace.with_mut(|w| {
        w.close(&PaneId::new("other")).unwrap();
        w.activate(&PaneId::new("keys")).unwrap();
    });
    pump(&mut doc, "the split closed", |d| heard(d).is_some());
    type_one(&mut doc, 2, "a key after the layout changed");
}
