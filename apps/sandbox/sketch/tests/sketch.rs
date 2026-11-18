//! Sketch's bar (#175, #176; #710 U3): scenes recorded in an extension's
//! own process and drawn in the app's pane.
//!
//! - a stroke drawn with a held pointer is in the window's scene;
//! - panning and zooming send only the transform, not the scene again;
//! - the graph, a paint view (`Builder::paint`) running in the process,
//!   moves a node it is dragged by;
//! - the timeline's playhead is run by the host: a frame of playback
//!   sends nothing, and a pause, a seek or another rate on the app's
//!   playhead moves the line with no message from the view;
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
use architect_extensions::scene::{Affine, RenderCommand, Scene, layers_value};
use architect_extensions::{Origin, Package};
use architect_sandbox_sketch::{ID, PLAYHEAD_SPEED, Sketch, arrangement_scene};
use architect_scene::anim::{Clocks, Reading};
use architect_scene::layers::Layer;
use architect_scene::ops::Sender;
use architect_workspace::paint::peniko::kurbo::Shape as _;
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
    overlay: bool,
}

/// One pane, 40 in and 20 down, 400 × 300.
#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "position: absolute; left: 40px; top: 20px; width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: ID, view: props.view, overlay: props.overlay }
        }
    }
}

struct App {
    doc: DioxusDocument,
    _host: Host,
    _dir: tempfile::TempDir,
}

fn open(view: &str) -> App {
    open_with(view, false)
}

/// View `view`, its scenes drawing over the whole app when `overlay`.
fn open_with(view: &str, overlay: bool) -> App {
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
            overlay,
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
        // Layers drawn by reference, copied in to be read.
        scene.flattened()
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

fn view_mut(doc: &mut BaseDocument) -> Option<&mut RemoteDocument> {
    let at = *mounted_views(doc).first()?;
    let sub: &mut dyn Any = doc.subdoc_mut(at)?;
    sub.downcast_mut::<RemoteDocument>()
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
    let resizes = view(&app.doc.inner.borrow()).unwrap().scene_resizes();
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
    let _ = app.window_scene();
    let per_frame = (bytes(&app.doc.inner.borrow()) - before) / 21;
    // A pan is not a resize: the widget kept its size (#729 M1).
    assert_eq!(
        view(&app.doc.inner.borrow()).unwrap().scene_resizes(),
        resizes,
        "panning told the view a new size"
    );
    println!("panning and zooming: {per_frame} bytes a frame");
    assert!(
        per_frame < 120,
        "a frame of panning sent {per_frame} bytes: the scene went again"
    );
}

/// The middle of the node fills of `color`'s shapes in `commands`.
fn fills_near(commands: &[RenderCommand], x: f64, y: f64) -> bool {
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

/// Where the playhead's line is in `commands`: its transform's x.
fn playhead_x(commands: &[RenderCommand]) -> Option<f64> {
    commands.iter().find_map(|c| match c {
        RenderCommand::Stroke(s) => Some(s.transform.as_coeffs()[4]),
        _ => None,
    })
}

fn fills(commands: &[RenderCommand]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::Fill(_)))
        .count()
}

/// What a test's clocks read: wall seconds, and the playhead.
#[derive(Clone, Default)]
struct Hands(Arc<std::sync::Mutex<(f64, Option<Reading>)>>);

impl Hands {
    fn set(&self, wall: f64, playhead: Option<Reading>) {
        *self.0.lock().unwrap() = (wall, playhead);
    }

    fn clocks(&self) -> Clocks {
        let (wall, playhead) = (self.clone(), self.clone());
        Clocks::new(
            move || wall.0.lock().unwrap().0,
            move || playhead.0.lock().unwrap().1,
        )
    }
}

const fn reading(time: f64, rate: f64) -> Reading {
    Reading { time, rate }
}

/// A second at 240 frames.
const FRAMES: u32 = 240;

/// How many frames a fader is dragged for.
const DRAGGED: u32 = 60;

/// The timeline, its clips in the window, its animations on `hands`.
fn timeline(hands: &Hands) -> App {
    let mut app = open("timeline");
    let deadline = Instant::now() + Duration::from_secs(10);
    while fills(&app.window_scene()) < 2000 {
        assert!(
            Instant::now() < deadline,
            "the clips never reached the window"
        );
        app.pump("the clips", |_| true);
    }
    view_mut(&mut app.doc.inner.borrow_mut())
        .unwrap()
        .set_clocks(hands.clocks());
    app
}

/// Whether the window would draw another frame now, unasked.
fn animating(app: &mut App) -> bool {
    app.pump("a frame", |_| true);
    app.doc.inner.borrow().is_animating()
}

#[test]
fn playback_in_a_process_is_run_by_the_host_and_sends_nothing() {
    let hands = Hands::default();
    let mut app = timeline(&hands);
    // What the clips and the playhead's motion cost to cross, once.
    let once = bytes(&app.doc.inner.borrow());
    assert!(once > 50_000, "the clips crossed in {once} bytes");
    let mut last = playhead_x(&app.window_scene()).unwrap();
    // A second at 240 frames: nothing on the playhead, so wall time. The
    // view's process is alive and is sent nothing, and sends nothing;
    // the line moves every frame.
    for frame in 1..=FRAMES {
        hands.set(f64::from(frame) / 240.0, None);
        assert!(animating(&mut app), "frame {frame}: the window went idle");
        let x = playhead_x(&app.window_scene()).unwrap();
        assert!(
            (x - last - PLAYHEAD_SPEED / 240.0).abs() < 1e-9,
            "frame {frame}: the line went from {last} to {x}"
        );
        last = x;
        assert!(fills(&app.window_scene()) >= 2000);
    }
    let playing = bytes(&app.doc.inner.borrow()) - once;
    assert_eq!(playing, 0, "playback sent {playing} bytes");
    // What a frame cost when the view sent its playhead's transform: a
    // `MOVE` in the attribute, as the same encoder writes it.
    let line = || Layer::new(Arc::new(arrangement_scene(1, 1)));
    let mut sender = Sender::new();
    let _ = sender.frame(&[(1, line())]);
    let moved = sender
        .frame(&[(1, line().at(Affine::translate((0.5, 0.0))))])
        .unwrap();
    let was = layers_value(u64::from(FRAMES), &moved).len();
    println!(
        "clips and motion once: {once} bytes; {FRAMES} frames of playback: {playing} bytes \
         (a transform a frame was {was} bytes, {} over these frames)",
        was * FRAMES as usize
    );
    // Still the view's: a pan crosses as before.
    drag(
        &mut app,
        (200.0, 150.0),
        (230.0, 150.0),
        3,
        Modifiers::empty(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while bytes(&app.doc.inner.borrow()) == once {
        assert!(Instant::now() < deadline, "the pan never crossed");
        app.pump("the pan", |_| true);
    }
    let panned = playhead_x(&app.window_scene()).unwrap();
    assert!(
        (panned - last - 30.0).abs() < 1e-6,
        "{last} panned to {panned}"
    );
}

#[test]
fn the_playhead_moves_the_line_with_no_message_from_the_view() {
    let hands = Hands::default();
    let mut app = timeline(&hands);
    let once = bytes(&app.doc.inner.borrow());
    // Paused one second in: the line stands, and the window rests.
    hands.set(10.0, Some(reading(1.0, 0.0)));
    let at = playhead_x(&app.window_scene()).unwrap();
    hands.set(11.0, Some(reading(1.0, 0.0)));
    assert!(
        !animating(&mut app),
        "a paused playhead kept the window drawing"
    );
    assert_eq!(playhead_x(&app.window_scene()).unwrap(), at);
    // A seek: drawn, the line where the playhead is.
    hands.set(12.0, Some(reading(3.5, 0.0)));
    assert!(animating(&mut app), "a seek wasn't drawn");
    let sought = playhead_x(&app.window_scene()).unwrap();
    assert!((sought - at - 2.5 * PLAYHEAD_SPEED).abs() < 1e-9);
    assert!(!animating(&mut app));
    // Playing at half speed: the playhead's time, not the wall's.
    hands.set(20.0, Some(reading(3.75, 0.5)));
    assert!(animating(&mut app));
    let played = playhead_x(&app.window_scene()).unwrap();
    assert!((played - sought - 0.25 * PLAYHEAD_SPEED).abs() < 1e-9);
    // None of it from the view.
    assert_eq!(bytes(&app.doc.inner.borrow()), once);
}

/// A window that counts the redraws it is asked for, and shows a cursor.
#[derive(Default)]
struct Window(
    std::sync::atomic::AtomicUsize,
    std::sync::Mutex<Option<cursor_icon::CursorIcon>>,
);

impl blitz_traits::shell::ShellProvider for Window {
    fn request_redraw(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn set_cursor(&self, icon: Option<cursor_icon::CursorIcon>) {
        *self.1.lock().unwrap() = icon;
    }
}

/// The y of the first fill in `commands` of `color`: where that layer is.
fn fill_at(
    commands: &[RenderCommand],
    color: architect_workspace::paint::peniko::Color,
) -> Option<(f64, f64)> {
    commands.iter().find_map(|c| match c {
        RenderCommand::Fill(f) if matches!(&f.brush, anyrender::Paint::Solid(c) if *c == color) => {
            let t = f.transform.as_coeffs();
            Some((t[4], t[5]))
        }
        _ => None,
    })
}

/// The timeline with its window's shell, nothing moving on its own.
fn mixer() -> (App, Arc<Window>) {
    let hands = Hands::default();
    hands.set(0.0, Some(reading(0.0, 0.0)));
    let app = timeline(&hands);
    let window = Arc::new(Window::default());
    view_mut(&mut app.doc.inner.borrow_mut())
        .unwrap()
        .set_host_shell(architect_extensions::remote::HostShell::new(
            window.clone(),
            0.0,
            0.0,
        ));
    let _ = app.window_scene();
    (app, window)
}

#[test]
fn a_fader_follows_the_pointer_with_nothing_from_the_view_and_hears_its_release() {
    use architect_sandbox_sketch::{KNOB_COLOR, TRACK_AT, TRACK_HEIGHT, fader_value, level_value};
    let (mut app, window) = mixer();
    let pane = (40.0, 20.0);
    let knob = |app: &App| fill_at(&app.window_scene(), KNOB_COLOR).unwrap().1;
    // At its initial 0.75: a quarter down the track.
    let top = pane.1 + TRACK_AT.1;
    assert!((knob(&app) - (top + TRACK_HEIGHT * 0.25)).abs() < 1e-9);
    let once = bytes(&app.doc.inner.borrow());
    // Over it: its cursor, from the host.
    let x = (pane.0 + TRACK_AT.0 + 12.0) as f32;
    let mut y = (top + TRACK_HEIGHT * 0.25) as f32;
    app.send(UiEvent::PointerMove(pointer(
        (x, y),
        FREE,
        Modifiers::empty(),
    )));
    assert_eq!(
        *window.1.lock().unwrap(),
        Some(cursor_icon::CursorIcon::NsResize)
    );
    app.send(UiEvent::PointerDown(pointer(
        (x, y),
        HELD,
        Modifiers::empty(),
    )));
    // Down a pixel a frame for 60 frames: the knob is under the pointer
    // at every one, and the view's process sends nothing.
    for frame in 1..=DRAGGED {
        y += 1.0;
        app.send(UiEvent::PointerMove(pointer(
            (x, y),
            HELD,
            Modifiers::empty(),
        )));
        app.pump("a frame", |_| true);
        assert!(
            (knob(&app) - f64::from(y)).abs() < 1e-6,
            "frame {frame}: the knob is at {}, the pointer at {y}",
            knob(&app)
        );
    }
    let dragging = bytes(&app.doc.inner.borrow()) - once;
    assert_eq!(dragging, 0, "the view sent {dragging} bytes while dragged");
    let level = {
        let doc = app.doc.inner.borrow();
        view(&doc)
            .unwrap()
            .scene_value(&fader_value().unwrap())
            .unwrap()
    };
    assert!((level - (0.75 - f64::from(DRAGGED) / TRACK_HEIGHT)).abs() < 1e-6);
    // Past the end of its track: it stops at 0, where the pointer leaves it.
    app.send(UiEvent::PointerMove(pointer(
        (x, y + 200.0),
        HELD,
        Modifiers::empty(),
    )));
    app.pump("past the end", |_| true);
    assert!((knob(&app) - (top + TRACK_HEIGHT)).abs() < 1e-6);
    app.send(UiEvent::PointerMove(pointer(
        (x, y),
        HELD,
        Modifiers::empty(),
    )));
    app.pump("back", |_| true);
    // Let go: the view hears where it was left, and keeps it as its own.
    app.send(UiEvent::PointerUp(pointer(
        (x, y),
        FREE,
        Modifiers::empty(),
    )));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let kept = {
            let doc = app.doc.inner.borrow();
            view(&doc).unwrap().scene_value(&level_value().unwrap())
        };
        if kept.is_some_and(|k| (k - level).abs() < 1e-9) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the view never heard the release"
        );
        let _ = app.window_scene();
        app.pump("the release", |_| true);
    }
    let released = bytes(&app.doc.inner.borrow()) - once;
    assert!(released < 100, "the release sent {released} bytes");
    println!(
        "fader: {DRAGGED} frames dragged, {dragging} bytes from the view; its release, {released} bytes"
    );
}

#[test]
fn a_box_follows_the_pointer_anywhere_with_nothing_from_the_view() {
    use architect_sandbox_sketch::{BOX_AT, BOX_COLOR};
    let (mut app, _window) = mixer();
    let pane = (40.0_f32, 20.0_f32);
    let start = fill_at(&app.window_scene(), BOX_COLOR).unwrap();
    let once = bytes(&app.doc.inner.borrow());
    let at = (
        pane.0 + BOX_AT.0 as f32 + 20.0,
        pane.1 + BOX_AT.1 as f32 + 12.0,
    );
    app.send(UiEvent::PointerMove(pointer(at, FREE, Modifiers::empty())));
    app.send(UiEvent::PointerDown(pointer(at, HELD, Modifiers::empty())));
    for frame in 1..=40_u8 {
        let (dx, dy) = (f32::from(frame) * 3.0, f32::from(frame) * 2.0);
        app.send(UiEvent::PointerMove(pointer(
            (at.0 + dx, at.1 + dy),
            HELD,
            Modifiers::empty(),
        )));
        app.pump("a frame", |_| true);
        let now = fill_at(&app.window_scene(), BOX_COLOR).unwrap();
        assert!(
            (now.0 - start.0 - f64::from(dx)).abs() < 1e-6
                && (now.1 - start.1 - f64::from(dy)).abs() < 1e-6,
            "frame {frame}: the box is at {now:?}"
        );
    }
    app.send(UiEvent::PointerUp(pointer(
        (at.0 + 120.0, at.1 + 80.0),
        FREE,
        Modifiers::empty(),
    )));
    app.pump("the release", |_| true);
    assert_eq!(bytes(&app.doc.inner.borrow()), once);
}

/// The drag box hovered is its content redrawn in the hover colour, by
/// the host, with nothing from the view (#844): a layer sent once,
/// hidden, shown while the pointer is on the box, in the box's own space.
#[test]
fn a_hovered_box_is_redrawn_in_its_hover_colour_with_nothing_from_the_view() {
    use architect_sandbox_sketch::{BOX_AT, BOX_COLOR, BOX_HOVER_COLOR};
    let (mut app, _window) = mixer();
    let pane = (40.0_f32, 20.0_f32);
    let once = bytes(&app.doc.inner.borrow());
    assert!(fill_at(&app.window_scene(), BOX_HOVER_COLOR).is_none());
    let at = (
        pane.0 + BOX_AT.0 as f32 + 20.0,
        pane.1 + BOX_AT.1 as f32 + 12.0,
    );
    app.send(UiEvent::PointerMove(pointer(at, FREE, Modifiers::empty())));
    app.pump("the hover", |_| true);
    let scene = app.window_scene();
    let (box_at, hover_at) = (
        fill_at(&scene, BOX_COLOR).unwrap(),
        fill_at(&scene, BOX_HOVER_COLOR).expect("the hover layer, drawn"),
    );
    assert_eq!(box_at, hover_at, "in the box's own space");
    // Off it: gone again.
    app.send(UiEvent::PointerMove(pointer(
        (pane.0 + 300.0, pane.1 + 250.0),
        FREE,
        Modifiers::empty(),
    )));
    app.pump("the leave", |_| true);
    assert!(fill_at(&app.window_scene(), BOX_HOVER_COLOR).is_none());
    assert_eq!(
        bytes(&app.doc.inner.borrow()),
        once,
        "the view sent nothing"
    );
}

#[test]
fn a_seek_wakes_the_window_a_resting_process_view_is_in() {
    use std::sync::atomic::Ordering;
    let hands = Hands::default();
    let mut app = timeline(&hands);
    hands.set(0.0, Some(reading(1.0, 0.0)));
    let window = Arc::new(Window::default());
    view_mut(&mut app.doc.inner.borrow_mut())
        .unwrap()
        .set_host_shell(architect_extensions::remote::HostShell::new(
            window.clone(),
            0.0,
            0.0,
        ));
    let _ = app.window_scene();
    assert!(!animating(&mut app), "paused, it rests");
    let before = window.0.load(Ordering::Relaxed);
    // Sought from elsewhere (the bar in another window): its window is
    // woken, and draws.
    hands.set(1.0, Some(reading(2.0, 0.0)));
    architect_scene::wake::clocks_moved();
    assert!(window.0.load(Ordering::Relaxed) > before);
    assert!(animating(&mut app));
}

/// The meter's height, as a fraction of its track: its fill's scale.
fn meter_scale(commands: &[RenderCommand]) -> Option<f64> {
    use architect_sandbox_sketch::METER_COLOR;
    commands.iter().find_map(|c| match c {
        RenderCommand::Fill(f) if matches!(&f.brush, anyrender::Paint::Solid(c) if *c == METER_COLOR) => {
            Some(f.transform.as_coeffs()[3])
        }
        _ => None,
    })
}

#[test]
fn a_meter_moves_at_the_engines_rate_with_nothing_from_the_view() {
    use architect_sandbox_sketch::meter_value;
    use architect_scene::engine::EngineTable;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut app, window) = mixer();
    // The engine's table, granted to the pane (`capabilities.ui =
    // ["engine"]`): what the app does when an engine publishes one.
    let (table, writer) = EngineTable::new([meter_value().unwrap()]).unwrap();
    let slot = table.slot(&meter_value().unwrap()).unwrap();
    view_mut(&mut app.doc.inner.borrow_mut())
        .unwrap()
        .set_engine(Some(Arc::clone(&table)));
    assert_eq!(
        meter_scale(&app.window_scene()),
        Some(1.0),
        "unwritten: as sent"
    );
    assert!(!animating(&mut app), "nothing moves: it rests");
    let once = bytes(&app.doc.inner.borrow());
    // An audio callback writing the meter every 2.67 ms for half a
    // second, while the window draws.
    let done = Arc::new(AtomicBool::new(false));
    let engine = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            for i in 0..188_u32 {
                writer.write(slot, (f64::from(i) * 0.07).sin().abs());
                std::thread::sleep(Duration::from_micros(2_667));
            }
            done.store(true, Ordering::Release);
            writer
        })
    };
    let mut frames = 0_u32;
    let mut levels = std::collections::BTreeSet::new();
    while !done.load(Ordering::Acquire) {
        app.pump("a frame", |_| true);
        let level = meter_scale(&app.window_scene()).unwrap();
        levels.insert((level * 1e6) as i64);
        frames += 1;
    }
    let writer = engine.join().unwrap();
    let metering = bytes(&app.doc.inner.borrow()) - once;
    assert_eq!(metering, 0, "metering sent {metering} bytes");
    assert!(
        levels.len() > 20,
        "{} levels in {frames} frames",
        levels.len()
    );
    println!(
        "meter: {frames} frames over 188 engine writes, {} levels drawn, {metering} bytes from the view",
        levels.len()
    );
    // The engine stopped: once the last level is drawn, the window rests.
    let _ = app.window_scene();
    assert!(
        !animating(&mut app),
        "a still meter kept the window drawing"
    );
    // It writes again: the window that rested is woken.
    let before = window.0.load(std::sync::atomic::Ordering::Relaxed);
    writer.write(slot, 0.5);
    let began = Instant::now();
    while window.0.load(std::sync::atomic::Ordering::Relaxed) == before {
        assert!(began.elapsed() < Duration::from_secs(5), "never woken");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!((meter_scale(&app.window_scene()).unwrap() - 0.5).abs() < 1e-9);
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
        assert_eq!(s.flattened().len(), 2000);
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

/// Whether every stroke in `commands` is drawn inside a clip to the
/// pane's box (40, 20, 400 × 300); `None` when there is no stroke.
fn strokes_clipped_to_the_pane(commands: &[RenderCommand]) -> Option<bool> {
    let mut open: Vec<bool> = Vec::new();
    let mut seen = None;
    for c in commands {
        match c {
            RenderCommand::PushClipLayer(l) => {
                let b = l.transform.transform_rect_bbox(l.clip.bounding_box());
                open.push(
                    (b.x0 - 40.0).abs() < 1.0
                        && (b.y0 - 20.0).abs() < 1.0
                        && (b.width() - 400.0).abs() < 1.0
                        && (b.height() - 300.0).abs() < 1.0,
                );
            }
            RenderCommand::PushLayer(_) => open.push(false),
            RenderCommand::PopLayer => {
                open.pop();
            }
            RenderCommand::Stroke(_) => {
                let inside = open.iter().any(|&p| p);
                seen = Some(seen.unwrap_or(true) && inside);
            }
            _ => {}
        }
    }
    seen
}

#[test]
fn a_scene_is_clipped_to_its_pane_unless_it_may_draw_over_the_app() {
    let mut app = open_with("sketch", false);
    drag(
        &mut app,
        (100.0, 100.0),
        (300.0, 200.0),
        5,
        Modifiers::empty(),
    );
    app.pump("drawn", |_| true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while strokes(&app.window_scene()) == 0 {
        assert!(Instant::now() < deadline, "no stroke");
        app.pump("the stroke", |_| true);
    }
    assert_eq!(
        strokes_clipped_to_the_pane(&app.window_scene()),
        Some(true),
        "not clipped to its pane"
    );

    let mut app = open_with("sketch", true);
    drag(
        &mut app,
        (100.0, 100.0),
        (300.0, 200.0),
        5,
        Modifiers::empty(),
    );
    app.pump("drawn", |_| true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while strokes(&app.window_scene()) == 0 {
        assert!(Instant::now() < deadline, "no stroke");
        app.pump("the stroke", |_| true);
    }
    assert_eq!(
        strokes_clipped_to_the_pane(&app.window_scene()),
        Some(false),
        "clipped though it may draw over the app"
    );
    // Drawn over the window where the pointer drew it.
    let stroke_at = |app: &App| {
        app.window_scene().into_iter().find_map(|c| match c {
            RenderCommand::Stroke(s) => {
                Some(s.transform.transform_rect_bbox(s.shape.bounding_box()))
            }
            _ => None,
        })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stroke = stroke_at(&app).unwrap();
        if (stroke.x0 - 100.0).abs() < 3.0
            && (stroke.y0 - 100.0).abs() < 3.0
            && (stroke.x1 - 300.0).abs() < 3.0
            && (stroke.y1 - 200.0).abs() < 3.0
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the overlay's stroke is at {stroke:?}"
        );
        app.pump("the whole stroke", |_| true);
    }
}

/// A frame making an element with scene `value` on a fresh document.
fn scene_frame(value: String) -> architect_extensions::wire::Frame {
    use architect_extensions::wire::{Edit, Frame, Value};
    Frame::Edits {
        edits: vec![
            Edit::PushId { id: 0 },
            Edit::CreateElement {
                tag: "object".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 1 },
            Edit::SetAttribute {
                name: architect_extensions::scene::SCENE_ATTR.to_owned(),
                namespace: None,
                value: Value::Text(value),
            },
            Edit::AppendChildren { m: 1 },
            Edit::Pop,
        ],
    }
}

#[test]
fn an_oversized_or_unbalanced_scene_fails_the_view_not_the_app() {
    let fresh = || RemoteDocument::new(futures::channel::mpsc::unbounded().0);
    let huge = format!("ab\n[{}]", "{\"k\":\"o\"},".repeat(500_000));
    let why = fresh().apply(scene_frame(huge)).unwrap_err();
    assert!(why.contains("past its limit"), "{why}");
    let stray = "ab\n[{\"k\":\"o\"}]".to_owned();
    let why = fresh().apply(scene_frame(stray)).unwrap_err();
    assert!(why.contains("popped"), "{why}");
    let never = "abc".to_owned();
    let why = fresh().apply(scene_frame(never)).unwrap_err();
    assert!(why.contains("never sent"), "{why}");
}

/// #175's budget, enforced generously: a pan's round trip on the
/// 2000-clip arrangement stays under `REMOTE_BUDGET` (33 ms) at p99.
#[test]
fn a_pans_round_trip_stays_inside_the_remote_budget() {
    let mut app = open("arrangement");
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
    let mut frames = Vec::new();
    for i in 1..=40 {
        let at = (from.0 - i as f32, from.1);
        let was = bytes(&app.doc.inner.borrow());
        let t = Instant::now();
        app.send(UiEvent::PointerMove(pointer(at, HELD, Modifiers::empty())));
        app.pump("the pan's frame", |d| bytes(d) > was);
        frames.push(t.elapsed());
    }
    frames.sort();
    let p99 = frames[(frames.len() - 1) * 99 / 100];
    assert!(
        p99 < architect_extensions::remote::REMOTE_BUDGET,
        "a pan's p99 round trip is {p99:?}"
    );
}
