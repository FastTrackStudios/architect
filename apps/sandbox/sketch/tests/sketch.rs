//! Sketch's bar (#175, #176; #710 U3): scenes recorded in an extension's
//! own process and drawn in the app's pane.
//!
//! - a stroke drawn with a held pointer is in the window's scene;
//! - panning and zooming send only the transform, not the scene again;
//! - the graph, a paint view (`Builder::paint`) running in the process,
//!   moves a node it is dragged by;
//! - `arrangement_budget` (ignored; `--ignored --nocapture`) measures a
//!   50-track arrangement: its first scene's bytes, the bytes and the round
//!   trip of each frame of panning, and the host's time to build its scene.
//!
//! The extension is this test binary started again as a child (`guest`).
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
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::print_stdout,
    clippy::significant_drop_tightening,
    clippy::suboptimal_flops
)]

use std::any::Any;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteDocument, RemoteView, mounted_views};
use architect_extensions::scene::{RenderCommand, Scene};
use architect_extensions::{Origin, Package};
use architect_sandbox_sketch::{ID, Sketch, arrangement_scene};
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, Modifiers,
    MouseEventButton, MouseEventButtons, Point, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::extension(&Sketch).serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = include_str!("../extension.toml")
        .replace(
            "path = \"architect-sandbox-sketch\"",
            "path = \"guest\"\nargs = [\"guest\", \"--exact\", \"--nocapture\", \"--test-threads=1\"]",
        )
        // This host draws the graph as a process view: no paint kind.
        .replace("kind = \"paint\"\n", "");
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

#[derive(Props, Clone, PartialEq)]
struct PaneProps {
    processes: Processes,
    view: String,
}

/// One pane, 40 in and 20 down, 400 × 300.
#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "position: absolute; left: 40px; top: 20px; width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: ID, view: props.view }
        }
    }
}

struct App {
    doc: DioxusDocument,
    _host: Host,
    _dir: tempfile::TempDir,
}

fn open(view: &str) -> App {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    host.load(&package(dir.path()), Origin::Installed).unwrap();
    let vdom = VirtualDom::new_with_props(
        Pane,
        PaneProps {
            processes,
            view: view.to_owned(),
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
    let mut app = App {
        doc,
        _host: host,
        _dir: dir,
    };
    // Shown, painted once (so the view hears its size), and drawn again.
    app.pump("the scene", |d| bytes(d) > 0);
    let _ = app.window_scene();
    app.pump("a frame at its size", |_| true);
    let _ = app.window_scene();
    app
}

impl App {
    fn pump(&mut self, what: &str, until: impl Fn(&BaseDocument) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let waker = futures::task::noop_waker();
        loop {
            for _ in 0..16 {
                if !self.doc.poll(Some(std::task::Context::from_waker(&waker))) {
                    break;
                }
            }
            self.doc.inner.borrow_mut().resolve(0.0);
            if until(&self.doc.inner.borrow()) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The window's scene, as its renderer would get it.
    fn window_scene(&self) -> Vec<RenderCommand> {
        let mut scene = Scene::new();
        blitz_paint::paint_scene(
            &mut scene,
            &mut self.doc.inner.borrow_mut(),
            1.0,
            800,
            600,
            0,
            0,
        );
        scene.commands
    }

    fn send(&mut self, event: UiEvent) {
        self.doc.handle_ui_event(event);
    }
}

/// The view's document.
fn view(doc: &BaseDocument) -> Option<&RemoteDocument> {
    let at = *mounted_views(doc).first()?;
    let sub: &dyn Any = doc.subdoc(at)?;
    sub.downcast_ref::<RemoteDocument>()
}

/// How many bytes of scenes and transforms the view sent.
fn bytes(doc: &BaseDocument) -> u64 {
    view(doc).map_or(0, RemoteDocument::scene_bytes)
}

fn pointer((x, y): (f32, f32), buttons: MouseEventButtons, mods: Modifiers) -> BlitzPointerEvent {
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
        mods,
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: Arc::default(),
    }
}

const HELD: MouseEventButtons = MouseEventButtons::Primary;
const FREE: MouseEventButtons = MouseEventButtons::None;

/// A drag from `from` to `to` in `steps`, each step answered.
fn drag(app: &mut App, from: (f32, f32), to: (f32, f32), steps: u32, mods: Modifiers) {
    app.send(UiEvent::PointerMove(pointer(from, FREE, mods)));
    app.send(UiEvent::PointerDown(pointer(from, HELD, mods)));
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        app.send(UiEvent::PointerMove(pointer(at, HELD, mods)));
        app.pump("a step", |_| true);
    }
    app.send(UiEvent::PointerUp(pointer(to, FREE, mods)));
}

fn strokes(commands: &[RenderCommand]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::Stroke(_)))
        .count()
}

#[test]
fn a_stroke_drawn_in_the_process_is_in_the_windows_scene() {
    let mut app = open("sketch");
    assert_eq!(strokes(&app.window_scene()), 0);
    drag(
        &mut app,
        (100.0, 100.0),
        (300.0, 200.0),
        20,
        Modifiers::empty(),
    );
    app.pump("the stroke", |_| true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while strokes(&app.window_scene()) == 0 {
        assert!(Instant::now() < deadline, "no stroke in the window's scene");
        app.pump("the stroke", |_| true);
    }
}

#[test]
fn panning_and_zooming_send_the_transform_not_the_scene() {
    let mut app = open("sketch");
    drag(
        &mut app,
        (100.0, 100.0),
        (300.0, 200.0),
        10,
        Modifiers::empty(),
    );
    app.pump("drawn", |_| true);
    let before = bytes(&app.doc.inner.borrow());
    // 20 frames of Shift-drag panning.
    drag(
        &mut app,
        (200.0, 200.0),
        (260.0, 240.0),
        20,
        Modifiers::SHIFT,
    );
    // A wheel's zoom.
    let at = (200.0_f32, 150.0_f32);
    app.send(UiEvent::Wheel(BlitzWheelEvent {
        delta: BlitzWheelDelta::Pixels(0.0, 50.0),
        coords: pointer(at, FREE, Modifiers::empty()).coords,
        buttons: FREE,
        mods: Modifiers::empty(),
        element: Point { x: 0.0, y: 0.0 },
    }));
    app.pump("panned", |_| true);
    std::thread::sleep(Duration::from_millis(50));
    app.pump("zoomed", |_| true);
    let per_frame = (bytes(&app.doc.inner.borrow()) - before) / 21;
    println!("panning and zooming: {per_frame} bytes a frame");
    assert!(
        per_frame < 120,
        "a frame of panning sent {per_frame} bytes: the scene went again"
    );
}

/// The middle of the node fills of `color`'s shapes in `commands`.
fn fills_near(commands: &[RenderCommand], x: f64, y: f64) -> bool {
    use architect_workspace::paint::peniko::kurbo::Shape as _;
    commands.iter().any(|c| match c {
        RenderCommand::Fill(f) => {
            let b = f.transform.transform_rect_bbox(f.shape.bounding_box());
            let (cx, cy) = (f64::midpoint(b.x0, b.x1), f64::midpoint(b.y0, b.y1));
            (cx - x).abs() < 3.0 && (cy - y).abs() < 3.0 && b.width() < 30.0
        }
        _ => false,
    })
}

#[test]
fn a_graph_node_drags_in_a_paint_view_served_by_its_process() {
    let mut app = open("graph");
    // The first node sits at the right of its circle: (w/2 + r, h/2).
    let r = 300.0 * 0.35;
    let node = (40.0 + 200.0 + r, 20.0 + 150.0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fills_near(&app.window_scene(), node.0, node.1) {
        assert!(
            Instant::now() < deadline,
            "the first node isn't where it was placed"
        );
        app.pump("the placed graph", |_| true);
    }
    let to = (120.0, 80.0);
    drag(
        &mut app,
        (node.0 as f32, node.1 as f32),
        to,
        15,
        Modifiers::empty(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fills_near(&app.window_scene(), f64::from(to.0), f64::from(to.1)) {
        assert!(Instant::now() < deadline, "the node didn't follow the drag");
        app.pump("the node", |_| true);
    }
}

fn p(samples: &mut [Duration], q: usize) -> f64 {
    samples.sort();
    samples[(samples.len() - 1) * q / 100].as_secs_f64() * 1e3
}

#[test]
#[ignore = "it times things: --ignored --nocapture"]
fn arrangement_budget() {
    // The host's side alone: a 2000-clip scene received and built.
    let commands = arrangement_scene(50, 40);
    let mut sent = architect_extensions::scene::SceneCache::new();
    let mut got = architect_extensions::scene::SceneCache::new();
    let value = sent.send(&commands);
    let t = Instant::now();
    let received = got.receive(&value).unwrap();
    let decode = t.elapsed();
    let mut builds = Vec::new();
    for _ in 0..50 {
        let t = Instant::now();
        let s = architect_extensions::scene::scene_of(
            &received,
            architect_extensions::scene::Affine::IDENTITY,
        );
        assert_eq!(s.commands.len(), 2000);
        builds.push(t.elapsed());
    }

    // Through the process: the first frame, then 100 frames of panning.
    let mut app = open("arrangement");
    let first = bytes(&app.doc.inner.borrow());
    let before = first;
    let mut frames = Vec::new();
    let from = (200.0_f32, 150.0_f32);
    app.send(UiEvent::PointerMove(pointer(
        from,
        FREE,
        Modifiers::empty(),
    )));
    app.send(UiEvent::PointerDown(pointer(
        from,
        HELD,
        Modifiers::empty(),
    )));
    for i in 1..=100 {
        let at = (from.0 - i as f32, from.1);
        let t = Instant::now();
        let was = bytes(&app.doc.inner.borrow());
        app.send(UiEvent::PointerMove(pointer(at, HELD, Modifiers::empty())));
        app.pump("the pan's frame", |d| bytes(d) > was);
        frames.push(t.elapsed());
    }
    app.send(UiEvent::PointerUp(pointer(from, FREE, Modifiers::empty())));
    let per_frame = (bytes(&app.doc.inner.borrow()) - before) / 100;
    let mut paint = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let _ = app.window_scene();
        paint.push(t.elapsed());
    }
    println!("| 50 tracks × 40 clips | value |");
    println!("|---|---|");
    println!("| first scene | {first} bytes |");
    println!("| a frame of panning | {per_frame} bytes |");
    println!(
        "| host: the scene decoded | {:.2} ms |",
        decode.as_secs_f64() * 1e3
    );
    println!(
        "| host: the scene built (p50 / p99) | {:.2} / {:.2} ms |",
        p(&mut builds, 50),
        p(&mut builds, 99)
    );
    println!(
        "| the window's scene recorded (p50 / p99) | {:.2} / {:.2} ms |",
        p(&mut paint, 50),
        p(&mut paint, 99)
    );
    println!(
        "| a pan's round trip (p50 / p99) | {:.2} / {:.2} ms |",
        p(&mut frames, 50),
        p(&mut frames, 99)
    );
}
