//! Another extension's view held in a remote view (`crate::embedded`, for
//! Bases): extension `test.holder`'s process view holds `test.cell`'s
//! view in a placeholder; the host draws the cell over the placeholder's
//! box, gives it the holder's input (again when it changes), and the
//! cell takes its own clicks.
//!
//! Both extensions are this test binary, started again as a child each
//! (`guest`).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value
)]

use std::any::Any;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::contrib::EmbedInput;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteDocument, RemoteView, mounted_views};
use architect_extensions::wire::{Edit, Frame, Value};
use architect_extensions::{Origin, Package};
use architect_extensions_dioxus::EmbeddedView;
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

/// The holder: a counter, and the cell's view held at (20, 30), 200 × 100,
/// given the count as its input.
#[component]
fn Holder() -> Element {
    let mut n = use_signal(|| 0_u32);
    rsx! {
        div { style: "position: relative; width: 400px; height: 300px;",
            button {
                id: "bump",
                style: "position: absolute; left: 300px; top: 0; width: 80px; height: 30px;",
                onclick: move |_| n += 1,
                "bump"
            }
            EmbeddedView {
                extension: "test.cell",
                view: "cell",
                input: EmbedInput {
                    claim: "embed_type:base_view".to_owned(),
                    source: format!("n={n}"),
                    props: serde_json::Value::Null,
                },
                style: "position: absolute; left: 20px; top: 30px; width: 200px; height: 100px;",
            }
        }
    }
}

/// The cell: what it was given, and its own button.
#[component]
fn Cell() -> Element {
    let input = architect_extensions::linked::use_embed();
    let mut clicks = use_signal(|| 0_u32);
    let text = input.map(|i| i.source).unwrap_or_default();
    rsx! {
        div { style: "width: 100%; height: 100%;",
            p { id: "cell-text", style: "margin: 0; height: 20px;", "{text}" }
            button {
                id: "cell-btn",
                style: "display: block; width: 100px; height: 40px;",
                onclick: move |_| clicks += 1,
                "cell"
            }
            p { id: "cell-clicks", style: "margin: 0;", "{clicks}" }
        }
    }
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new()
        .view("holder", Holder)
        .view("cell", Cell)
        .serve()
        .unwrap();
}

fn package(dir: &Path, id: &str, view: &str) -> Package {
    let dir = dir.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), dir.join("guest")).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        format!(
            r#"id = "{id}"
name = "{id}"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "{view}"
title = "{view}"
"#
        ),
    )
    .unwrap();
    Package::read(&dir).unwrap()
}

#[derive(Props, Clone, PartialEq)]
struct AppProps {
    processes: Processes,
}

/// The holder's view in a pane at (50, 40), 400 × 300.
#[allow(non_snake_case)]
fn App(props: AppProps) -> Element {
    rsx! {
        div { style: "position: absolute; left: 50px; top: 40px; width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: "test.holder", view: "holder" }
        }
    }
}

/// The text of `selector` in whichever mounted view has it, and where
/// that view's box is in the window.
fn find(doc: &BaseDocument, selector: &str) -> Option<(String, (f32, f32))> {
    mounted_views(doc).into_iter().find_map(|at| {
        let view = doc.subdoc(at)?.inner();
        let node = view.query_selector(selector).ok()??;
        let text = view.get_node(node)?.text_content();
        let p = doc.get_node(at)?.absolute_position(0.0, 0.0);
        Some((text, (p.x, p.y)))
    })
}

/// Where `selector`'s middle is in the window.
fn point(doc: &BaseDocument, selector: &str) -> (f32, f32) {
    for at in mounted_views(doc) {
        let Some(sub) = doc.subdoc(at) else { continue };
        let view = sub.inner();
        if let Ok(Some(node)) = view.query_selector(selector) {
            let host = doc.get_node(at).unwrap().absolute_position(0.0, 0.0);
            let n = view.get_node(node).unwrap();
            let p = n.absolute_position(0.0, 0.0);
            let s = n.final_layout().size;
            return (host.x + p.x + s.width / 2.0, host.y + p.y + s.height / 2.0);
        }
    }
    panic!("no {selector}");
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn press(doc: &mut DioxusDocument, (x, y): (f32, f32)) {
    let at = |buttons| BlitzPointerEvent {
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
    };
    doc.handle_ui_event(UiEvent::PointerMove(at(MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(at(MouseEventButtons::Primary)));
    doc.handle_ui_event(UiEvent::PointerUp(at(MouseEventButtons::None)));
}

#[test]
fn a_remote_view_holds_another_extensions_view_which_takes_its_own_input() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    host.load(
        &package(dir.path(), "test.holder", "holder"),
        Origin::Installed,
    )
    .unwrap();
    host.load(&package(dir.path(), "test.cell", "cell"), Origin::Installed)
        .unwrap();

    let mut doc = DioxusDocument::new(
        VirtualDom::new_with_props(App, AppProps { processes }),
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();

    // The cell's view, drawn by its own process, over the placeholder:
    // the pane at (50, 40), the placeholder 20 in and 30 down.
    pump(&mut doc, "the held view", |d| {
        find(d, "#cell-text").is_some_and(|(text, (x, y))| {
            text == "n=0" && (x - 70.0).abs() < 1.0 && (y - 70.0).abs() < 1.0
        })
    });

    // Its own button takes the click: the held view is in front.
    let btn = point(&doc.inner.borrow(), "#cell-btn");
    press(&mut doc, btn);
    pump(&mut doc, "the cell's click", |d| {
        find(d, "#cell-clicks").is_some_and(|(t, _)| t == "1")
    });

    // The holder changes the input it gives: the cell draws it.
    let bump = point(&doc.inner.borrow(), "#bump");
    press(&mut doc, bump);
    pump(&mut doc, "the new input", |d| {
        find(d, "#cell-text").is_some_and(|(t, _)| t == "n=1")
    });
    drop(host);
}

/// A frame with a placeholder whose view is `view`.
fn holding(view: &str) -> Frame {
    Frame::Edits {
        edits: vec![
            Edit::PushId { id: 0 },
            Edit::CreateElement {
                tag: "div".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 1 },
            Edit::SetAttribute {
                name: architect_extensions::embedded::VIEW_ATTR.to_owned(),
                namespace: None,
                value: Value::Text(view.to_owned()),
            },
            Edit::AppendChildren { m: 1 },
            Edit::Pop,
        ],
    }
}

#[test]
fn a_placeholder_naming_no_view_fails_the_view_not_the_app() {
    let fresh = || RemoteDocument::new(futures::channel::mpsc::unbounded().0);
    let mut ok = fresh();
    ok.apply(holding("test.cell/cell")).unwrap();
    let held: &dyn Any = &ok;
    assert!(held.is::<RemoteDocument>());
    let why = fresh().apply(holding("nothing")).unwrap_err();
    assert!(why.contains("names no view"), "{why}");
}
