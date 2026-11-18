//! One `PaintView`, unchanged, linked in and as a process (#840): the same
//! `ViewCtx` and the same input.
//!
//! The probe view is mounted the way an extension mounts one in its own
//! Dioxus view, `architect_extensions::linked::PaintPane`: linked, that is
//! the workspace's `PaintPane`; in the process, `scene::RemotePaint`. Both
//! runs put it in a workspace pane of the same size, with the same theme
//! store, and send the window the same events. The view writes what it
//! was given (its pane, kind, theme, extras, geometry, every input, and a
//! frame after a `PaintWake` from another thread) to a log, and the two
//! logs must be the same.
//!
//! The process is this test binary started again (`guest`).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::print_stdout,
    clippy::significant_drop_tightening,
    // Dioxus components take their props by value.
    clippy::needless_pass_by_value,
    clippy::option_if_let_else
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::linked::{Builder, Extension, PaintPane};
use architect_extensions::process::{ProcessRunner, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::RemotePaneView;
use architect_extensions::{Origin, Package};
use architect_theme::{Registry, ThemeStore};
use architect_workspace::paint::region::{Region, RegionEvent, Shape};
use architect_workspace::paint::{Layer, LayerKey, PaintMount, PaintView, PaintWake, Scene};
use architect_workspace::{Extras, HostData, ViewCtx, ViewInput};
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzImeEvent, BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta,
    BlitzWheelEvent, Code, Key, KeyState, Location, Modifiers, MouseEventButton, MouseEventButtons,
    Point, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const ID: &str = "test.probe";
/// Where the probe writes its log: a file per run, so the process's
/// lines come back to the test.
const OUT: &str = "PAINT_PARITY_OUT";

/// One run at a time: each sets [`OUT`] for itself and its process.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::extension(&Probe).serve().unwrap();
}

fn log(line: &str) {
    let Some(path) = std::env::var_os(OUT) else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

/// What the probe's host shares with it: a number another thread bumps.
struct Count(u32);

/// A view that writes down what it is given.
struct Recorder {
    wake: Option<PaintWake>,
    seen: Option<u32>,
    size: (f32, f32),
    /// Draws as layers, with [`REGION`] over [`REGION_RECT`]: what a
    /// pointer does there is the region's.
    regions: bool,
}

/// The region the `regions` probe draws.
const REGION: u64 = 7;
/// Where it is, in the view's pixels.
const REGION_RECT: (f64, f64, f64, f64) = (20.0, 20.0, 140.0, 100.0);

impl Recorder {
    fn new(ctx: &ViewCtx) -> Self {
        let theme: Vec<String> = ctx
            .theme
            .0
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        log(&format!(
            "ctx pane={} kind={:?} host={:?} placement={:?} caller={:?} doc={} data={} wake={} tokens={} theme=[{}]",
            ctx.pane.0,
            ctx.kind,
            ctx.host,
            ctx.placement,
            ctx.caller,
            ctx.doc.is_some(),
            ctx.data::<Count>().is_some(),
            ctx.extras.get::<PaintWake>().is_some(),
            ctx.theme.0.len(),
            theme.join(";"),
        ));
        Self {
            wake: ctx.extras.get::<PaintWake>().cloned(),
            seen: None,
            size: (0.0, 0.0),
            regions: false,
        }
    }

    fn with_regions(ctx: &ViewCtx) -> Self {
        Self {
            regions: true,
            ..Self::new(ctx)
        }
    }
}

/// An input as the log says it: its numbers to a tenth of a pixel.
fn said(input: &ViewInput) -> String {
    let r = |v: f32| (v * 10.0).round() / 10.0;
    match input {
        ViewInput::PointerDown { x, y, button, mods } => {
            format!("down {} {} {button:?} {mods:?}", r(*x), r(*y))
        }
        ViewInput::PointerMove { x, y, mods } => format!("move {} {} {mods:?}", r(*x), r(*y)),
        ViewInput::PointerUp { x, y, button, mods } => {
            format!("up {} {} {button:?} {mods:?}", r(*x), r(*y))
        }
        ViewInput::Wheel { dx, dy, x, y, mods } => format!(
            "wheel {} {} at {} {} {mods:?}",
            r(*dx),
            r(*dy),
            r(*x),
            r(*y)
        ),
        other => format!("{other:?}"),
    }
}

impl PaintView for Recorder {
    fn input(&mut self, ctx: &ViewCtx, event: &ViewInput) -> bool {
        log(&format!("input {}", said(event)));
        // `p` has another thread change the host's data and wake the
        // pane: the frame after it sees the new number.
        if let ViewInput::Key {
            key, down: true, ..
        } = event
            && key == "p"
            && let (Some(wake), Some(data)) = (self.wake.clone(), ctx.data::<Count>())
        {
            std::thread::spawn(move || {
                data.update(|c| c.0 += 1);
                wake.wake();
            });
        }
        false
    }

    fn region(&mut self, _: &ViewCtx, event: &RegionEvent) -> bool {
        log(&format!("region {event:?}"));
        false
    }

    fn layers(&mut self, ctx: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        if !self.regions {
            return None;
        }
        let mut scene = Scene::new();
        self.paint(ctx, &mut scene);
        let (x0, y0, x1, y1) = REGION_RECT;
        let region = Region::new(
            REGION,
            Shape::Rect(architect_workspace::paint::peniko::kurbo::Rect::new(
                x0, y0, x1, y1,
            )),
        );
        Some(vec![(
            1,
            Layer::new(Arc::new(scene.commands)).regions(vec![region]),
        )])
    }

    fn paint(&mut self, ctx: &ViewCtx, _: &mut Scene) {
        let count = ctx.data::<Count>().map(|d| d.read(|c| c.0));
        if count != self.seen {
            self.seen = count;
            log(&format!("frame count={count:?}"));
        }
        let size = (ctx.geometry.width, ctx.geometry.height);
        if size.0 > 0.0 && size != self.size {
            self.size = size;
            log(&format!(
                "size {}x{}",
                ctx.geometry.width.round(),
                ctx.geometry.height.round()
            ));
        }
    }
}

/// The probe's view: its `PaintView` in its own Dioxus view, as an
/// extension mounts one (Keyflow's chart beside its palettes).
#[allow(non_snake_case)]
fn ProbeView() -> Element {
    probe(Recorder::new)
}

/// The probe with a hit region (a chord on a chart).
#[allow(non_snake_case)]
fn RegionsView() -> Element {
    probe(Recorder::with_regions)
}

/// A view that keeps where it is in its pane (`linked::use_view_state`):
/// it logs what it was given when it first has its pane (in a process,
/// the pane's input arrives just after the first render) and on every
/// change, and keeps `zoom=2` the first time it starts with nothing kept.
#[allow(non_snake_case)]
fn MemoView() -> Element {
    let kept = architect_extensions::linked::use_view_state();
    let last = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<String>)));
    if let Some(k) = &kept {
        let was = last.borrow().clone();
        match was {
            None => log(&format!("start view_state={}", k.value)),
            Some(v) if v != k.value => log(&format!("view_state={}", k.value)),
            Some(_) => {}
        }
        last.replace(Some(k.value.clone()));
    }
    use_effect(use_reactive((&kept,), |(kept,)| {
        if let Some(k) = kept
            && k.value.is_empty()
        {
            k.set("zoom=2");
        }
    }));
    rsx! { div { {kept.map(|k| k.value).unwrap_or_default()} } }
}

/// What a process view gets that a linked one gets from the app (#840):
/// the theme store's theme, `document::Style` in its document, and the
/// actions it may run in its `ViewCtx`.
#[allow(non_snake_case)]
fn ThemedView() -> Element {
    let ink = architect_theme::use_resolved()
        .get("ink")
        .map(str::to_owned);
    let mount = use_hook(|| PaintMount::new(Recorder::new));
    // The paint view's ctx, as a process gives it: its actions.
    let probe = use_hook(|| {
        PaintMount::new(|ctx: &ViewCtx| {
            let actions = ctx.actions.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    if actions.info("TEST_PROBE_PING").is_some() {
                        log("actions: TEST_PROBE_PING");
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                log("actions: none");
            });
            Recorder::new(ctx)
        })
    });
    if let Some(ink) = ink {
        log(&format!("theme ink={ink}"));
    }
    let _ = mount;
    rsx! {
        document::Style { "#themed-probe {{ color: rgb(1, 2, 3); }}" }
        div { id: "themed-probe", style: "width: 100%; height: 100%;",
            PaintPane { mount: probe }
        }
    }
}

fn probe(make: fn(&ViewCtx) -> Recorder) -> Element {
    let mount = use_hook(|| {
        let mount = PaintMount::new(make);
        let wake = mount.wake_handle();
        mount.extras(
            Extras::default()
                .with_data(HostData::new(Count(1)))
                .with(wake),
        )
    });
    rsx! {
        div { style: "width: 100%; height: 100%;",
            PaintPane { mount }
        }
    }
}

struct Probe;

impl Extension for Probe {
    fn manifest(&self) -> &'static str {
        r#"
id = "test.probe"
name = "Probe"
version = "0.1.0"
kind = "process"
api = "0.1"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[views]]
id = "probe"
title = "Probe"

[[views]]
id = "regions"
title = "Regions"

[[views]]
id = "memo"
title = "Memo"

[[views]]
id = "themed"
title = "Themed"

[[actions]]
id = "TEST_PROBE_PING"
name = "Ping"
description = "Answers pong."
effect = "read_only"
"#
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[("icons/probe.svg", "<svg viewBox=\"0 0 1 1\"/>")]
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("probe", ProbeView)
            .view("regions", RegionsView)
            .view("memo", MemoView)
            .view("themed", ThemedView)
            .action("TEST_PROBE_PING", |_| async {
                Ok(serde_json::json!("pong"))
            });
    }
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    std::fs::write(dir.join("extension.toml"), Probe.manifest()).unwrap();
    Package::read(dir).unwrap()
}

#[derive(Props, Clone, PartialEq)]
struct AppProps {
    /// The process's view, or (`None`) the probe linked in.
    processes: Option<architect_extensions::process::Processes>,
    store: Store,
    /// The probe's view: `probe` or `regions`.
    view: &'static str,
}

#[derive(Clone)]
struct Store(ThemeStore);

impl PartialEq for Store {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// The action the probe's view binds `N` to in its context
/// (`[[keys]] when = "view:…"`): how many times it ran.
static NOTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

static NOTE: architect::action::ActionMeta = architect::action::ActionMeta {
    id: "TEST_PROBE_NOTE",
    trait_name: "",
    method_name: "",
    display_name: "Note",
    description: "",
    category: "",
    group: "",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The keymap's actions, with `N` bound in the pane's view context.
fn view_keys() -> Arc<ActionList> {
    let list = ActionList::new();
    list.register(&NOTE, architect::action::Effect::Mutating, |(): ()| {
        NOTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    Arc::new(list)
}

/// One workspace pane, the probe's view in it: linked, or its process's.
#[allow(non_snake_case)]
fn App(props: AppProps) -> Element {
    let commands = architect_commands::use_commands_provider();
    architect_commands::use_action_list(use_hook(view_keys));
    use_hook(move || {
        commands.set_view_keys(vec![(
            "view:probe".to_owned(),
            "TEST_PROBE_NOTE".to_owned(),
            "N".to_owned(),
        )]);
    });
    let store = props.store.0.clone();
    architect_theme::use_theme_store(move || store);
    let workspace = use_signal(|| {
        let mut w = architect_workspace::Workspace::new();
        w.open(
            architect_workspace::Pane::new("probe-pane", "probe", "Probe"),
            architect_workspace::Target::Tab,
        )
        .unwrap();
        w
    });
    let processes = props.processes;
    let view = props.view;
    let views = use_hook(move || {
        architect_workspace::Views::new().view(architect_workspace::View::new(
            "probe",
            "Probe",
            move |_| match (processes.clone(), view) {
                (Some(processes), _) => rsx! {
                    RemotePaneView { processes, extension: ID, view }
                },
                (None, "regions") => rsx! { RegionsView {} },
                (None, "memo") => rsx! { MemoView {} },
                (None, "themed") => rsx! { ThemedView {} },
                (None, _) => rsx! { ProbeView {} },
            },
        ))
    });
    rsx! {
        div { style: "width: 600px; height: 400px;",
            architect_workspace::WorkspaceView { workspace, views }
        }
    }
}

/// A run: the document, and what keeps its host alive.
struct Run {
    doc: DioxusDocument,
    out: PathBuf,
    host: Option<Host>,
    processes: Option<architect_extensions::process::Processes>,
}

impl Run {
    fn pump_until(&mut self, what: &str, until: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let waker = futures::task::noop_waker();
        loop {
            for _ in 0..16 {
                if !self.doc.poll(Some(std::task::Context::from_waker(&waker))) {
                    break;
                }
            }
            self.doc.inner.borrow_mut().resolve(0.0);
            // Painted, as a window would: a paint view draws a frame.
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
            let log = std::fs::read_to_string(&self.out).unwrap_or_default();
            if until(&log) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the log:\n{log}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.out).unwrap_or_default()
    }

    fn send(&mut self, event: UiEvent) {
        self.doc.handle_ui_event(event);
    }
}

fn start(process: bool, dir: &Path) -> Run {
    start_view(process, dir, "probe")
}

fn start_view(process: bool, dir: &Path, view: &'static str) -> Run {
    let out = dir.join(format!(
        "{view}-{}.log",
        if process { "process" } else { "linked" }
    ));
    // The process inherits it as it starts; the linked view reads it here.
    unsafe { std::env::set_var(OUT, &out) };
    let store = ThemeStore::new(Registry::builtin());
    let (processes, host) = if process {
        let runner = ProcessRunner::new()
            .unwrap()
            .with_socket_dir(dir)
            .with_restart(Restart::never())
            .with_theme(store.clone());
        let processes = runner.processes();
        let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
        let pkg = dir.join(format!("pkg-{view}"));
        std::fs::create_dir_all(&pkg).unwrap();
        host.load(&package(&pkg), Origin::Installed).unwrap();
        (Some(processes), Some(host))
    } else {
        (None, None)
    };
    let kept = processes.clone();
    let vdom = VirtualDom::new_with_props(
        App,
        AppProps {
            processes,
            store: Store(store),
            view,
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
    let mut run = Run {
        doc,
        out,
        host,
        processes: kept,
    };
    if view == "memo" || view == "themed" {
        run.pump_until("the view", |l| {
            l.contains("start view_state=") || l.contains("ctx pane=")
        });
    } else {
        run.pump_until("the view at its size", |l| l.contains("size "));
    }
    run
}

/// Where the paint view's box is in the window: the paint widget's
/// `object` linked, the remote view's element in a process.
fn origin(doc: &DioxusDocument) -> (f32, f32) {
    let d = doc.inner.borrow();
    let node = architect_extensions::remote::mounted_views(&d)
        .first()
        .copied()
        .or_else(|| d.query_selector("object").unwrap())
        .expect("the view's box");
    let at = d.get_node(node).unwrap().absolute_position(0.0, 0.0);
    (at.x, at.y)
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

fn key(text: &str, state: KeyState, modifiers: Modifiers) -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: Key::Character(text.into()),
        code: Code::KeyP,
        modifiers,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state,
        text: Some(text.into()),
    }
}

/// The same events, at the same place in the view, each answered.
fn script(run: &mut Run) {
    let (ox, oy) = origin(&run.doc);
    let at = |x: f32, y: f32| (ox + x, oy + y);
    let free = MouseEventButtons::None;
    let held = MouseEventButtons::Primary;
    let steps: Vec<(UiEvent, &str)> = vec![
        (
            UiEvent::PointerMove(pointer(at(50.0, 40.0), free, Modifiers::empty())),
            "input move 50 40",
        ),
        (
            UiEvent::PointerDown(pointer(at(60.0, 45.0), held, Modifiers::SHIFT)),
            "input down 60 45",
        ),
        (
            UiEvent::PointerMove(pointer(at(90.0, 70.0), held, Modifiers::SHIFT)),
            "input move 90 70",
        ),
        (
            UiEvent::PointerUp(pointer(at(90.0, 70.0), free, Modifiers::SHIFT)),
            "input up 90 70",
        ),
        (
            UiEvent::Wheel(BlitzWheelEvent {
                delta: BlitzWheelDelta::Lines(0.0, 2.0),
                coords: pointer(at(100.0, 80.0), free, Modifiers::CONTROL).coords,
                buttons: free,
                mods: Modifiers::CONTROL,
                element: Point { x: 0.0, y: 0.0 },
            }),
            "input wheel",
        ),
        (
            UiEvent::Wheel(BlitzWheelEvent {
                delta: BlitzWheelDelta::Pixels(3.0, -12.5),
                coords: pointer(at(100.0, 80.0), free, Modifiers::empty()).coords,
                buttons: free,
                mods: Modifiers::empty(),
                element: Point { x: 0.0, y: 0.0 },
            }),
            "input wheel -3 12.5",
        ),
        (
            UiEvent::KeyDown(key("q", KeyState::Pressed, Modifiers::CONTROL)),
            "\"q\", down: true",
        ),
        (
            UiEvent::KeyUp(key("q", KeyState::Released, Modifiers::CONTROL)),
            "\"q\", down: false",
        ),
        (
            UiEvent::Ime(BlitzImeEvent::Preedit("か".into(), None)),
            "preedit: \"か\"",
        ),
        (
            UiEvent::Ime(BlitzImeEvent::Commit("かな".into())),
            "commit: Some(\"かな\")",
        ),
        (
            UiEvent::KeyDown(key("p", KeyState::Pressed, Modifiers::empty())),
            "\"p\", down: true",
        ),
    ];
    for (event, expect) in steps {
        run.send(event);
        run.pump_until(expect, |l| l.contains(expect));
    }
    // `p`'s wake, from another thread: a frame with the new number.
    run.pump_until("the woken frame", |l| l.contains("frame count=Some(2)"));
}

/// The log without what differs by design: nothing yet.
fn comparable(log: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in log.lines() {
        // A frame at the same size is drawn as often as each host draws.
        if lines.last().is_some_and(|l| l == line) {
            continue;
        }
        lines.push(line.to_owned());
    }
    lines
        .into_iter()
        .filter(|l| !l.starts_with("size ") || l.contains('x'))
        .collect()
}

#[test]
fn one_paint_view_gets_the_same_ctx_and_input_linked_and_as_a_process() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let mut linked = start(false, dir.path());
    script(&mut linked);
    let linked_log = linked.log();
    drop(linked);

    let mut process = start(true, dir.path());
    script(&mut process);
    let process_log = process.log();

    println!("linked:\n{linked_log}\nprocess:\n{process_log}");
    let (a, b) = (comparable(&linked_log), comparable(&process_log));
    assert!(
        a.iter()
            .any(|l| l.starts_with("ctx ") && l.contains("tokens=") && !l.contains("tokens=0")),
        "the linked view had no theme: {a:?}"
    );
    assert_eq!(a, b, "linked and process differ");
}

/// A first press that lands on a hit region (a chord, a note), with no
/// click on the view before it: the view takes the keyboard focus, as a
/// press anywhere else in it does, and the key typed next is the view's.
/// Its region hears the press; the view hears no pointer press.
fn press_a_region_then_type(run: &mut Run) {
    let (ox, oy) = origin(&run.doc);
    let (x0, y0, _, _) = REGION_RECT;
    let at = (ox + x0 as f32 + 30.0, oy + y0 as f32 + 20.0);
    let free = MouseEventButtons::None;
    let held = MouseEventButtons::Primary;
    run.send(UiEvent::PointerMove(pointer(at, free, Modifiers::empty())));
    run.send(UiEvent::PointerDown(pointer(at, held, Modifiers::empty())));
    run.pump_until("the region's press", |l| l.contains("region Press("));
    run.send(UiEvent::PointerUp(pointer(at, free, Modifiers::empty())));
    run.pump_until("the region's release", |l| l.contains("region Release("));
    run.send(UiEvent::KeyDown(key(
        "k",
        KeyState::Pressed,
        Modifiers::empty(),
    )));
    run.pump_until("the key, after a press on a region", |l| {
        l.contains("\"k\", down: true")
    });
}

#[test]
fn a_press_on_a_region_focuses_the_view_linked_and_as_a_process() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    for process in [false, true] {
        let mut run = start_view(process, dir.path(), "regions");
        press_a_region_then_type(&mut run);
        let log = run.log();
        let how = if process { "process" } else { "linked" };
        assert!(
            log.contains("input Focus(true)"),
            "{how}: never focused:\n{log}"
        );
        assert!(
            !log.contains("input down"),
            "{how}: the region's press reached the view as a pointer press:\n{log}"
        );
    }
}

/// A plain letter bound in the view's context (`[[keys]] when =
/// "view:…"`) runs its action through the keymap while the view has the
/// focus, and the view never hears it; a letter bound there by nothing is
/// still the view's. Linked and as a process alike.
#[test]
fn a_key_bound_in_a_views_context_runs_its_action_and_skips_the_view() {
    use std::sync::atomic::Ordering;
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    for process in [false, true] {
        let how = if process { "process" } else { "linked" };
        let mut run = start_view(process, dir.path(), "probe");
        let (ox, oy) = origin(&run.doc);
        let at = (ox + 50.0, oy + 40.0);
        run.send(UiEvent::PointerDown(pointer(
            at,
            MouseEventButtons::Primary,
            Modifiers::empty(),
        )));
        run.send(UiEvent::PointerUp(pointer(
            at,
            MouseEventButtons::None,
            Modifiers::empty(),
        )));
        run.pump_until("the focus", |l| l.contains("input Focus(true)"));
        let before = NOTED.load(Ordering::SeqCst);
        run.send(UiEvent::KeyDown(key(
            "n",
            KeyState::Pressed,
            Modifiers::empty(),
        )));
        run.send(UiEvent::KeyDown(key(
            "m",
            KeyState::Pressed,
            Modifiers::empty(),
        )));
        run.pump_until("the unbound key", |l| l.contains("key: \"m\", down: true"));
        assert_eq!(
            NOTED.load(Ordering::SeqCst),
            before + 1,
            "{how}: N didn't run"
        );
        let log = run.log();
        assert!(
            !log.contains("key: \"n\""),
            "{how}: the view heard the bound key:\n{log}"
        );
    }
}

/// Where a view is (`linked::use_view_state`) is kept by its pane and
/// given back on start: linked in, and in a process whose program is
/// killed and started again (a development copy rebuilt), which starts
/// where it was.
#[test]
fn a_views_state_is_kept_by_its_pane_and_given_back_when_its_program_restarts() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    for process in [false, true] {
        let how = if process { "process" } else { "linked" };
        let mut run = start_view(process, dir.path(), "memo");
        // It started with nothing kept, kept zoom=2, and was given it back.
        run.pump_until(&format!("{how}: the kept state"), |l| {
            l.contains("\nview_state=zoom=2")
        });
        assert!(
            run.log().starts_with("start view_state=\n"),
            "{how}: {}",
            run.log()
        );
        if !process {
            continue;
        }
        let processes = run.processes.clone().unwrap();
        let p = processes.get(ID).unwrap();
        let pid = p.pid().unwrap();
        let killed = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while p.pid() == Some(pid) {
            assert!(Instant::now() < deadline, "the process never died");
            run.pump_until("a moment", |_| true);
            std::thread::sleep(Duration::from_millis(10));
        }
        p.restart();
        // The new program's view starts where the old one was.
        run.pump_until("the view started again", |l| {
            l.matches("start view_state=zoom=2").count() == 1
        });
    }
}

/// The pointer leaving the view (#844): the view hears a move to where it
/// went and then `PointerLeave`, linked in and as a process alike, so a
/// view that hit-tests for itself can clear its hover.
#[test]
fn a_view_hears_the_pointer_leave_linked_and_as_a_process() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let mut logs = Vec::new();
    for process in [false, true] {
        let how = if process { "process" } else { "linked" };
        let mut run = start_view(process, dir.path(), "probe");
        let (ox, oy) = origin(&run.doc);
        let free = MouseEventButtons::None;
        run.send(UiEvent::PointerMove(pointer(
            (ox + 50.0, oy + 40.0),
            free,
            Modifiers::empty(),
        )));
        run.pump_until(&format!("{how}: the move in"), |l| {
            l.contains("input move 50 40")
        });
        // Out of the window past the view's left edge.
        run.send(UiEvent::PointerLeave(pointer(
            (ox - 5.0, oy + 40.0),
            free,
            Modifiers::empty(),
        )));
        run.pump_until(&format!("{how}: the leave"), |l| {
            l.contains("input PointerLeave")
        });
        let log = run.log();
        let tail: Vec<&str> = log.lines().filter(|l| l.starts_with("input ")).collect();
        logs.push(tail.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>());
        assert_eq!(tail.last(), Some(&"input PointerLeave"), "{how}: {log}");
    }
    assert_eq!(logs[0], logs[1], "linked and process differ");
}

/// A Shift-click on a region (#844): the view hears the press and the
/// release with Shift held, linked in and as a process alike.
#[test]
fn a_press_on_a_region_carries_its_modifiers_linked_and_as_a_process() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let shift = format!(
        "{:?}",
        architect_workspace::paint::region::Mods::from_names(&["Shift"])
    );
    for process in [false, true] {
        let how = if process { "process" } else { "linked" };
        let mut run = start_view(process, dir.path(), "regions");
        let (ox, oy) = origin(&run.doc);
        let (x0, y0, _, _) = REGION_RECT;
        let at = (ox + x0 as f32 + 30.0, oy + y0 as f32 + 20.0);
        run.send(UiEvent::PointerMove(pointer(
            at,
            MouseEventButtons::None,
            Modifiers::SHIFT,
        )));
        run.send(UiEvent::PointerDown(pointer(
            at,
            MouseEventButtons::Primary,
            Modifiers::SHIFT,
        )));
        run.send(UiEvent::PointerUp(pointer(
            at,
            MouseEventButtons::None,
            Modifiers::SHIFT,
        )));
        let mods = format!("region Modifiers({REGION}, {shift})");
        let press = format!("region Press({REGION})");
        let release = format!("region Release({REGION})");
        run.pump_until(&format!("{how}: the shifted release"), |l| {
            // The keys held, just before the press, and again before the
            // release.
            l.contains(&format!("{mods}\n{press}")) && l.contains(&format!("{mods}\n{release}"))
        });
    }
}

/// A process view gets what a linked view gets from the app (#840): the
/// app's theme through the theme store (`architect_theme::use_resolved`),
/// its `document::Style` applied in its document, and in its paint
/// view's `ViewCtx` the actions it may run (its own here).
#[test]
fn a_process_view_has_the_theme_its_styles_and_its_actions() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let mut run = start_view(true, dir.path(), "themed");
    run.pump_until("the theme's ink", |l| l.contains("theme ink=#"));
    run.pump_until("its actions", |l| l.contains("actions: "));
    assert!(
        run.log().contains("actions: TEST_PROBE_PING"),
        "{}",
        run.log()
    );
    // Its style, in the view's own document.
    let styled = {
        let d = run.doc.inner.borrow();
        let at = *architect_extensions::remote::mounted_views(&d)
            .first()
            .unwrap();
        let sub: &dyn std::any::Any = d.subdoc(at).unwrap();
        let remote = sub
            .downcast_ref::<architect_extensions::remote::RemoteDocument>()
            .unwrap();
        let doc = blitz_dom::Document::inner(remote);
        doc.query_selector_all("style")
            .unwrap()
            .iter()
            .filter_map(|n| doc.get_node(*n))
            .map(blitz_dom::Node::text_content)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(styled.contains("rgb(1, 2, 3)"), "no style: {styled:?}");
    // The files its code holds, sent to its host as it connected.
    let host = run.host.clone().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while host
        .contributions()
        .runtime_file(ID, "icons/probe.svg")
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "its files never reached the host"
        );
        run.pump_until("a moment", |_| true);
    }
}
