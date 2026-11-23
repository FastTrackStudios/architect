//! An installed extension's view as an overlay layer over another view
//! (#175, `[[overlays]] draws`): the process draws a scene in the view's
//! coordinate spaces, the host mounts it over the view's box, it follows
//! the view as it scrolls, and the pointer passes through to the view.
//!
//! The extension is this test binary, started again as a child (`guest`).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::significant_drop_tightening
)]

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::interface::{ExtensionViewOverlays, use_extension_ui};
use architect_extensions::overlay::{Lane, OverlaySpaces, TimeAxis};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::scene::{Affine, PaintScene as _, RemoteScene, RenderCommand, Scene};
use architect_extensions::{Origin, Package};
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use peniko::kurbo::{Rect, Shape as _};
use peniko::{Color, Fill};

const ID: &str = "test.marks";
const VIEW: &str = "test.arrange";
const MARK: Color = Color::from_rgb8(200, 30, 120);

/// The overlay: a mark on lane `kick` from 4 s to 6 s, in the view's terms.
#[component]
fn Marks() -> Element {
    let spaces = architect_extensions_dioxus::overlay::spaces();
    let content = use_memo(use_reactive!(|spaces| {
        let mut scene = Scene::new();
        if let Some(s) = &spaces
            && let (Some(time), Some(lane)) = (s.time, s.lane("kick"))
        {
            let (x0, x1) = (time.x(4.0), time.x(6.0));
            let (y0, y1) = (f64::from(lane.top), f64::from(lane.top + lane.height));
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                MARK,
                None,
                &Rect::new(x0, y0, x1, y1),
            );
        }
        Rc::new(scene.commands)
    }));
    rsx! { RemoteScene { content: content() } }
}

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new().view("marks", Marks).serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Marks"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[capabilities]
over = ["{VIEW}"]

[[views]]
id = "marks"
title = "Marks"

[[overlays]]
id = "marks"
view = "{VIEW}"
draws = "marks"
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

static VIEW_CLICKS: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static START: std::cell::RefCell<Option<Signal<f64>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Props, Clone, PartialEq)]
struct AppProps {
    host: Host,
    processes: Processes,
}

/// The app: a stand-in arrangement at (50, 40), 400 × 300, its lane
/// `kick` at 60 points down, scrolled by `START`, 50 points a second.
#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn App(props: AppProps) -> Element {
    use_context_provider(|| props.processes.clone());
    let ui = use_extension_ui(props.host, Arc::new(ActionList::new()));
    use_context_provider(|| ui);
    let start = use_signal(|| 0.0_f64);
    START.with(|s| *s.borrow_mut() = Some(start));
    let spaces = OverlaySpaces {
        width: 400.0,
        height: 300.0,
        time: Some(TimeAxis {
            start: start(),
            pixels_per_second: 50.0,
        }),
        lanes: vec![Lane {
            id: "kick".into(),
            name: "Kick".into(),
            top: 60.0,
            height: 40.0,
        }],
    };
    rsx! {
        div {
            id: "arrange",
            style: "position: absolute; left: 50px; top: 40px; width: 400px; height: 300px;",
            onclick: move |_| {
                VIEW_CLICKS.fetch_add(1, Ordering::Relaxed);
            },
            ExtensionViewOverlays { view: VIEW, plane: "over", spaces }
        }
    }
}

fn pump(doc: &mut DioxusDocument, rounds: u32) {
    let waker = futures::task::noop_waker();
    for _ in 0..rounds {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        // Painting lays out the scene's widget: it tells the view its size.
        let _ = window(doc);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn window(doc: &DioxusDocument) -> Vec<RenderCommand> {
    let mut scene = Scene::new();
    blitz_paint::paint_scene(&mut scene, &mut doc.inner.borrow_mut(), 1.0, 800, 600, 0, 0);
    scene.flattened()
}

/// Where the mark is in the window, if drawn.
fn mark(doc: &DioxusDocument) -> Option<Rect> {
    window(doc).into_iter().find_map(|c| match c {
        RenderCommand::Fill(f) if matches!(f.brush, anyrender::Paint::Solid(c) if c == MARK) => {
            Some(f.transform.transform_rect_bbox(f.shape.bounding_box()))
        }
        _ => None,
    })
}

fn wait_for(doc: &mut DioxusDocument, what: &str, until: impl Fn(&DioxusDocument) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !until(doc) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            mark(doc)
        );
        pump(doc, 1);
    }
}

fn near(r: Rect, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> bool {
    (r.x0 - x0).abs() < 1.0
        && (r.y0 - y0).abs() < 1.0
        && (r.x1 - x1).abs() < 1.0
        && (r.y1 - y1).abs() < 1.0
}

fn click(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
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
fn an_extensions_view_draws_over_another_view_in_its_spaces_and_follows_it() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    host.load(&package(dir.path()), Origin::Installed).unwrap();
    assert_eq!(host.view_overlays(VIEW, "over").len(), 1);

    let vdom = VirtualDom::new_with_props(
        App,
        AppProps {
            host: host.clone(),
            processes,
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();

    // 4 s to 6 s at 50 pt/s from 0: x 200 to 300; lane kick: y 60 to 100;
    // the view at (50, 40) in the window.
    wait_for(&mut doc, "the mark", |d| {
        mark(d).is_some_and(|r| near(r, (250.0, 100.0, 350.0, 140.0)))
    });

    // The view scrolls 2 s on: the mark follows, 100 points left.
    START.with(|s| s.borrow().unwrap().set(2.0));
    wait_for(&mut doc, "the mark to follow the scroll", |d| {
        mark(d).is_some_and(|r| near(r, (150.0, 100.0, 250.0, 140.0)))
    });

    // A click on the mark is the view's: the layer lets it through.
    let at = (200.0, 120.0);
    doc.handle_ui_event(UiEvent::PointerMove(click(
        at.0,
        at.1,
        MouseEventButtons::None,
    )));
    doc.handle_ui_event(UiEvent::PointerDown(click(
        at.0,
        at.1,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(click(
        at.0,
        at.1,
        MouseEventButtons::None,
    )));
    pump(&mut doc, 2);
    assert_eq!(
        VIEW_CLICKS.load(Ordering::Relaxed),
        1,
        "the view didn't get the click"
    );
    drop(host);
}

#[test]
fn an_overlay_names_a_view_it_draws_or_a_script_never_both() {
    let base = r#"id = "t.o"
name = "O"
version = "1.0.0"
kind = "process"
[entry]
path = "o"
[[views]]
id = "marks"
title = "Marks"
[capabilities]
over = ["v"]
[[overlays]]
id = "o"
view = "v"
"#;
    let parse =
        |extra: &str| architect_extensions::manifest::Manifest::parse(&format!("{base}{extra}"));
    assert!(parse("draws = \"marks\"\n").is_ok());
    let missing = parse("draws = \"nope\"\n").unwrap_err().to_string();
    assert!(missing.contains("not one of its [[views]]"), "{missing}");
    let both = parse("draws = \"marks\"\nscript = \"o.luau\"\n")
        .unwrap_err()
        .to_string();
    assert!(both.contains("not both"), "{both}");
    let neither = parse("").unwrap_err().to_string();
    assert!(neither.contains("not both"), "{neither}");
}

/// The owner's rule for #762: drawing over another extension's or the
/// app's own view is asked for, in words, and not mounted until granted.
#[test]
fn drawing_over_another_view_is_asked_for_and_not_mounted_without_the_grant() {
    use architect_extensions::consent::{Grants, Scope};
    let dir = tempfile::tempdir().unwrap();
    let package = package(dir.path());
    let asks: Vec<String> = package
        .manifest
        .capabilities
        .asks()
        .into_iter()
        .map(|a| a.says)
        .collect();
    assert_eq!(asks, ["Draw over the arrangement"]);

    // Installed, nothing granted: not mounted.
    let grants = Grants::open(dir.path().join("grants.json"));
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(dir.path())
        .with_restart(Restart::never());
    let host = Host::new(ActionList::new(), SharedSections::default())
        .with_runner(runner)
        .with_grants(grants.clone());
    host.load(&package, Origin::Installed).unwrap();
    assert!(
        host.view_overlays(VIEW, "over").is_empty(),
        "drawn without the grant"
    );
    // Granted: mounted.
    grants.grant(
        ID,
        "1.0.0",
        package.manifest.capabilities,
        Scope::Device,
        "test",
    );
    assert_eq!(host.view_overlays(VIEW, "over").len(), 1);

    // Undeclared, it does not even load.
    let undeclared = std::fs::read_to_string(dir.path().join("extension.toml"))
        .unwrap()
        .replace("over = [\"test.arrange\"]", "");
    let why = architect_extensions::manifest::Manifest::parse(&undeclared)
        .unwrap_err()
        .to_string();
    assert!(why.contains("capabilities.over"), "{why}");
}
