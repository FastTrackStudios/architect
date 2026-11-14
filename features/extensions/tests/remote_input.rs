//! Rich input in a process extension's view (#176): pointer events with
//! the press holding the pointer, touch, wheel, scroll, an input
//! method's composition and the window's IME, and moves coalesced while
//! the view is busy. Each kind reaches a guest handler through the host's
//! Blitz document, as in `remote_view.rs`.
//!
//! The extension is this test binary, started again as a child (the
//! `guest` test). `round_trips` (ignored; `--ignored --nocapture`) prints
//! the key-to-echo and pointer-to-echo round trips and a 500-row frame's.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::significant_drop_tightening,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::suboptimal_flops
)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteView, mounted_views};
use architect_extensions::{Origin, Package};
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzImeEvent, BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta,
    BlitzWheelEvent, Code, Key, KeyState, Location, Modifiers, MouseEventButton, MouseEventButtons,
    Point, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, ShellProvider, Viewport};
use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const ID: &str = "test.input";

/// The extension's view: a card to drag, a lane beside it, a scrolling
/// list, a field, and what each saw.
#[component]
fn Input() -> Element {
    let mut log = use_signal(Vec::<String>::new);
    let mut moves = use_signal(|| 0_u32);
    let mut last_x = use_signal(|| 0.0_f64);
    let mut text = use_signal(String::new);
    let mut rows = use_signal(|| 0_usize);
    let mut say = move |s: String| log.write().push(s);
    rsx! {
        div { style: "position: relative; width: 400px; height: 300px;",
            div {
                id: "card",
                style: "position: absolute; left: 0; top: 0; width: 100px; height: 40px;",
                onpointerdown: move |e| say(format!("down:{}", e.pointer_type())),
                onpointermove: move |e| {
                    if !e.held_buttons().is_empty() || e.pointer_type() == "touch" {
                        moves += 1;
                        last_x.set(e.client_coordinates().x);
                    }
                },
                onpointerup: move |_| say("up".to_owned()),
                onpointercancel: move |_| say("cancel".to_owned()),
                ontouchstart: move |e| say(format!("touchstart:{}", e.touches().len())),
                ontouchend: move |_| say("touchend".to_owned()),
                "card"
            }
            div {
                id: "lane",
                style: "position: absolute; left: 200px; top: 0; width: 200px; height: 100px;",
                onpointermove: move |_| say("lane-move".to_owned()),
                "lane"
            }
            div {
                id: "scroller",
                style: "position: absolute; left: 0; top: 120px; width: 200px; height: 60px; overflow-y: scroll;",
                onwheel: move |e| {
                    let WheelDelta::Pixels(v) = e.delta() else { return };
                    say(format!("wheel:{}", v.y));
                },
                onscroll: move |e| say(format!("scroll:{}", e.scroll_top())),
                div { style: "height: 600px;", "tall" }
            }
            input {
                id: "field",
                style: "position: absolute; left: 0; top: 200px; width: 200px; height: 30px;",
                value: "{text}",
                oninput: move |e| text.set(e.value()),
                oncompositionstart: move |_| say("compose".to_owned()),
                oncompositionupdate: move |e| say(format!("composing:{}", e.data().data())),
                oncompositionend: move |e| say(format!("composed:{}", e.data().data())),
            }
            button {
                id: "rows",
                style: "position: absolute; left: 250px; top: 200px; width: 60px; height: 30px;",
                onclick: move |_| rows.set(if rows() == 0 { 500 } else { 0 }),
                "rows"
            }
            p { id: "log", style: "position: absolute; top: 240px;", {log.read().join(" ")} }
            p { id: "moves", style: "position: absolute; top: 260px;", "{moves}" }
            p { id: "lastx", style: "position: absolute; top: 280px;", "{last_x}" }
            p { id: "echo", style: "position: absolute; top: 300px;", "{text}" }
            p { id: "count", "{rows}" }
            for i in 0..rows() {
                span { key: "{i}", class: "row", "{i}" }
            }
        }
    }
}

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new().view("input", Input).serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Input"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "input"
title = "Input"
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

/// What the window's shell was asked: the IME on or off, its caret area.
#[derive(Default)]
struct Window {
    ime: Mutex<Vec<bool>>,
    area: Mutex<Option<(f32, f32)>>,
}

#[derive(Clone)]
struct Shell(Arc<Window>);

impl ShellProvider for Shell {
    fn set_ime_enabled(&self, on: bool) {
        self.0.ime.lock().unwrap().push(on);
    }
    fn set_ime_cursor_area(&self, x: f32, y: f32, _: f32, _: f32) {
        *self.0.area.lock().unwrap() = Some((x, y));
    }
}

#[derive(Props, Clone, PartialEq)]
struct PaneProps {
    processes: Processes,
}

/// The app: one pane, 30 points down and 50 in, the view in it.
#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "position: absolute; left: 50px; top: 30px; width: 400px; height: 340px;",
            RemoteView { processes: props.processes, extension: ID, view: "input" }
        }
    }
}

fn app(processes: Processes, window: &Arc<Window>) -> DioxusDocument {
    let vdom = VirtualDom::new_with_props(Pane, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            shell_provider: Some(Arc::new(Shell(Arc::clone(window)))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
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
            "timed out waiting for {what}: the log says {:?}",
            in_view(&doc.inner.borrow(), "#log")
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn in_view(doc: &BaseDocument, selector: &str) -> Option<String> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector(selector).ok()??;
    Some(view.get_node(node)?.text_content())
}

fn logged(doc: &BaseDocument) -> String {
    in_view(doc, "#log").unwrap_or_default()
}

/// Where the view's `selector`'s middle is, in the app's window.
fn view_point(doc: &BaseDocument, selector: &str) -> (f32, f32) {
    let at = mounted_views(doc)[0];
    let host = doc.get_node(at).unwrap().absolute_position(0.0, 0.0);
    let view = doc.subdoc(at).unwrap().inner();
    let node = view.query_selector(selector).unwrap().unwrap();
    let n = view.get_node(node).unwrap();
    let p = n.absolute_position(0.0, 0.0);
    let size = n.final_layout().size;
    (
        host.x + p.x + size.width / 2.0,
        host.y + p.y + size.height / 2.0,
    )
}

fn pointer(
    id: BlitzPointerId,
    (x, y): (f32, f32),
    buttons: MouseEventButtons,
) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id,
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

const MOUSE: BlitzPointerId = BlitzPointerId::Mouse;
const HELD: MouseEventButtons = MouseEventButtons::Primary;
const FREE: MouseEventButtons = MouseEventButtons::None;

fn click(doc: &mut DioxusDocument, at: (f32, f32)) {
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, at, FREE)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(MOUSE, at, HELD)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(MOUSE, at, FREE)));
}

/// The extension loaded and its view showing.
fn start() -> (tempfile::TempDir, Host, DioxusDocument, Arc<Window>) {
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    host.load(&package(dir.path()), Origin::Installed).unwrap();
    let window = Arc::new(Window::default());
    let mut doc = app(processes, &window);
    pump(&mut doc, "the view", |d| in_view(d, "#moves").is_some());
    (dir, host, doc, window)
}

#[test]
fn a_press_holds_the_pointer_and_its_moves_and_release_reach_the_element_pressed() {
    let (_dir, _host, mut doc, _) = start();
    let card = view_point(&doc.inner.borrow(), "#card");
    let lane = view_point(&doc.inner.borrow(), "#lane");
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, card, FREE)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(MOUSE, card, HELD)));
    pump(&mut doc, "the press", |d| logged(d).contains("down:mouse"));
    // Dragged over the lane: the card still gets the moves.
    for i in 0..=10 {
        let t = i as f32 / 10.0;
        let at = (
            card.0 + (lane.0 - card.0) * t,
            card.1 + (lane.1 - card.1) * t,
        );
        doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, at, HELD)));
    }
    let want = format!("{}", lane.0 - 50.0);
    pump(&mut doc, "the last move", |d| {
        in_view(d, "#lastx").as_deref() == Some(want.as_str())
    });
    doc.handle_ui_event(UiEvent::PointerUp(pointer(MOUSE, lane, FREE)));
    pump(&mut doc, "the release", |d| logged(d).contains("up"));
    let log = logged(&doc.inner.borrow());
    assert!(
        !log.contains("lane-move"),
        "the lane saw a held move: {log}"
    );

    // Free, a move over the lane is the lane's.
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, lane, FREE)));
    pump(&mut doc, "a free move", |d| logged(d).contains("lane-move"));
}

#[test]
fn a_release_lost_outside_the_pane_cancels_the_press() {
    let (_dir, _host, mut doc, _) = start();
    let card = view_point(&doc.inner.borrow(), "#card");
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, card, FREE)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(MOUSE, card, HELD)));
    // Out of the pane, released there, and back with no button held.
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, (700.0, 500.0), HELD)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(MOUSE, (700.0, 500.0), FREE)));
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, card, FREE)));
    pump(&mut doc, "the cancel", |d| logged(d).contains("cancel"));
}

#[test]
fn moves_while_the_view_is_busy_are_coalesced_to_the_latest() {
    let (_dir, _host, mut doc, _) = start();
    let card = view_point(&doc.inner.borrow(), "#card");
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, card, FREE)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(MOUSE, card, HELD)));
    // 200 moves before the view has answered any: it is sent far fewer,
    // and the last.
    for i in 1..=200 {
        let at = (card.0 + i as f32, card.1);
        doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, at, HELD)));
    }
    let want = format!("{}", card.0 + 200.0 - 50.0);
    pump(&mut doc, "the latest move", |d| {
        in_view(d, "#lastx").as_deref() == Some(want.as_str())
    });
    let moves: u32 = in_view(&doc.inner.borrow(), "#moves")
        .unwrap()
        .parse()
        .unwrap();
    assert!(moves < 200, "{moves} of 200 moves were sent");
    println!("200 moves reached the view as {moves}");
}

#[test]
fn a_finger_is_a_touch_and_a_touch_pointer() {
    let (_dir, _host, mut doc, _) = start();
    let card = view_point(&doc.inner.borrow(), "#card");
    let finger = BlitzPointerId::Finger(1);
    doc.handle_ui_event(UiEvent::PointerDown(pointer(finger, card, HELD)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(finger, card, FREE)));
    pump(&mut doc, "the touch", |d| {
        let log = logged(d);
        log.contains("down:touch") && log.contains("touchstart:1") && log.contains("touchend")
    });
}

#[test]
fn a_wheel_reaches_the_view_and_scrolls_its_list() {
    let (_dir, _host, mut doc, _) = start();
    let list = view_point(&doc.inner.borrow(), "#scroller");
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, list, FREE)));
    // Blitz's sign is winit's: down is negative. The view is sent the
    // web's.
    doc.handle_ui_event(UiEvent::Wheel(BlitzWheelEvent {
        delta: BlitzWheelDelta::Pixels(0.0, -40.0),
        coords: pointer(MOUSE, list, FREE).coords,
        buttons: FREE,
        mods: Modifiers::empty(),
        element: Point { x: 0.0, y: 0.0 },
    }));
    pump(&mut doc, "the wheel and the scroll", |d| {
        let log = logged(d);
        log.contains("wheel:40") && log.contains("scroll:40")
    });
}

fn key(text: &str) -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: Key::Character(text.into()),
        code: Code::KeyA,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: Some(text.into()),
    }
}

#[test]
fn an_input_method_composes_into_a_field_in_the_view() {
    let (_dir, _host, mut doc, window) = start();
    let field = view_point(&doc.inner.borrow(), "#field");
    click(&mut doc, field);
    // The field's focus turned the window's IME on, its caret area
    // where the field is in the window, not in the view.
    pump(&mut doc, "the IME on", |_| {
        window.ime.lock().unwrap().last() == Some(&true)
    });
    let (_, y) = window.area.lock().unwrap().unwrap();
    assert!(
        y >= 30.0 + 200.0,
        "the caret area is at {y}, not in the window"
    );

    doc.handle_ui_event(UiEvent::Ime(BlitzImeEvent::Preedit(
        "にほ".to_owned(),
        Some((0, 6)),
    )));
    doc.handle_ui_event(UiEvent::Ime(BlitzImeEvent::Preedit(String::new(), None)));
    doc.handle_ui_event(UiEvent::Ime(BlitzImeEvent::Commit("日本".to_owned())));
    pump(&mut doc, "the composition", |d| {
        let log = logged(d);
        log.contains("compose")
            && log.contains("composing:にほ")
            && log.contains("composed:日本")
            && in_view(d, "#echo").as_deref() == Some("日本")
    });
    // Then a plain key, into the same field.
    doc.handle_ui_event(UiEvent::KeyDown(key("a")));
    pump(&mut doc, "the key", |d| {
        in_view(d, "#echo").as_deref() == Some("日本a")
    });
}

fn p(samples: &mut [Duration], q: usize) -> f64 {
    samples.sort();
    samples[(samples.len() - 1) * q / 100].as_secs_f64() * 1e3
}

/// The round trips #176 budgets: a key into a field and its echo back, a
/// held move and its echo, and a frame of 500 new rows. Printed as
/// milliseconds, p50 and p99.
#[test]
#[ignore = "it times things: --ignored --nocapture"]
fn round_trips() {
    let (_dir, _host, mut doc, _) = start();
    let field = view_point(&doc.inner.borrow(), "#field");
    click(&mut doc, field);
    let mut keys = Vec::new();
    for i in 0..200 {
        let t = Instant::now();
        doc.handle_ui_event(UiEvent::KeyDown(key("a")));
        let want = i + 1;
        pump(&mut doc, "the echo", |d| {
            in_view(d, "#echo").is_some_and(|e| e.len() == want)
        });
        keys.push(t.elapsed());
    }
    let card = view_point(&doc.inner.borrow(), "#card");
    doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, card, FREE)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(MOUSE, card, HELD)));
    let mut moves = Vec::new();
    for i in 1..=200 {
        let t = Instant::now();
        let x = card.0 + (i % 50) as f32 + 1.0;
        doc.handle_ui_event(UiEvent::PointerMove(pointer(MOUSE, (x, card.1), HELD)));
        let want = format!("{}", x - 50.0);
        pump(&mut doc, "the move's echo", |d| {
            in_view(d, "#lastx").as_deref() == Some(want.as_str())
        });
        moves.push(t.elapsed());
    }
    doc.handle_ui_event(UiEvent::PointerUp(pointer(MOUSE, card, FREE)));
    let rows = view_point(&doc.inner.borrow(), "#rows");
    let mut frames = Vec::new();
    for i in 0..40 {
        let t = Instant::now();
        click(&mut doc, rows);
        let want = if i % 2 == 0 { "500" } else { "0" };
        pump(&mut doc, "the rows", |d| {
            in_view(d, "#count").as_deref() == Some(want)
        });
        frames.push(t.elapsed());
    }
    println!("| round trip | n | p50 ms | p99 ms |");
    println!("|---|---|---|---|");
    for (what, s) in [
        ("key to echo", &mut keys),
        ("held move to echo", &mut moves),
        ("500 rows in and out", &mut frames),
    ] {
        println!(
            "| {what} | {} | {:.2} | {:.2} |",
            s.len(),
            p(s, 50),
            p(s, 99)
        );
    }
}
