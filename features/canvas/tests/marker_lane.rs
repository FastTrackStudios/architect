//! The issue's bar: a script draws a marker lane over an arrangement and
//! opens a small tool window with sliders.
//!
//! The arrangement is a stand-in for Studio's: a view of kind
//! `session.arrange` publishing time across and two lanes down, and two
//! actions (`MARKERS_LIST`, read-only; `TRANSPORT_SEEK`, recorded). The
//! script reads the markers, draws a flag for each in the view's time, a
//! wash across the lane it is asked for, and a window whose sliders set
//! the lane's height and opacity; clicking a flag seeks to it. The same
//! script then runs as an extension's overlay, inside its capabilities.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use architect_canvas::imgui::{Input, WindowPointer};
use architect_canvas::luau::Session;
use architect_canvas::{
    Color, Lane, Layer, Plane, Point, Shape, Size, Spaces, TimeAxis, ViewPointer,
};
use architect_extensions::{Package, host};
use architect_scripting::luau::Runtime;
use facet::Facet;
use futures::executor::block_on;
use serde_json::{Value, json};

const ARRANGE: &str = "session.arrange";
const WINDOW: &str = "Markers/Marker lane";

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Session",
        method_name: id,
        display_name: id,
        description: "a test action",
        category: "Session",
        group: "Session",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta("MARKERS_LIST");
static SEEK: ActionMeta = meta("TRANSPORT_SEEK");

#[derive(Facet)]
struct Seek {
    time: f64,
}

#[derive(Facet)]
struct Marker {
    name: String,
    time: f64,
}

const SCRIPT: &str = include_str!("marker_lane.luau");

/// The app: two markers, and the seeks it was asked for.
fn app() -> (ActionList, Arc<Mutex<Vec<f64>>>) {
    let list = ActionList::new();
    list.register(&LIST, Effect::ReadOnly, |(): ()| {
        Ok::<_, String>(vec![
            Marker {
                name: "Verse".into(),
                time: 4.0,
            },
            Marker {
                name: "Chorus".into(),
                time: 12.0,
            },
        ])
    })
    .expect("registered");
    let seeks = Arc::new(Mutex::new(Vec::new()));
    let to = Arc::clone(&seeks);
    list.register(&SEEK, Effect::Mutating, move |s: Seek| {
        to.lock().unwrap().push(s.time);
        Ok::<_, String>(())
    })
    .expect("registered");
    (list, seeks)
}

/// The arrangement scrolled to `start` seconds, at 50 points a second.
fn arrangement(start: f64) -> Spaces {
    Spaces::sized(800.0, 200.0)
        .with_time(TimeAxis {
            start,
            pixels_per_second: 50.0,
        })
        .with_lane(Lane::new("t1", "Kick", 20.0, 60.0))
        .with_lane(Lane::new("t2", "Vocals", 80.0, 40.0))
}

fn flags(layer: &Layer) -> Vec<(String, f32)> {
    layer
        .draw
        .shapes()
        .iter()
        .filter_map(|s| match s {
            Shape::Text { text, at, .. } => Some((text.clone(), at.x)),
            _ => None,
        })
        .collect()
}

fn wash(layer: &Layer) -> Option<(f32, f32, Color)> {
    layer.draw.shapes().iter().find_map(|s| match s {
        Shape::Rect {
            rect,
            color,
            fill: true,
            ..
        } if rect.w >= 800.0 => Some((rect.y, rect.h, *color)),
        _ => None,
    })
}

fn sized(pointer: Option<(f32, f32)>, down: bool) -> Input {
    Input {
        pointer: pointer.map(|(x, y)| WindowPointer {
            window: WINDOW.into(),
            at: Point::new(x, y),
        }),
        down,
        sizes: [(WINDOW.to_owned(), Size::new(300.0, 140.0))].into(),
        ..Input::default()
    }
}

fn start(runtime: &Runtime) -> Session {
    let (report, session) = block_on(Session::start(
        runtime,
        &ActionCaller::app(),
        SCRIPT,
        json!({ "lane": "Vocals" }),
        "Marker lane",
    ));
    assert_eq!(report.result, Ok(Value::Null), "{:?}", report.log);
    session.expect("it started")
}

#[test]
fn a_script_draws_a_marker_lane_over_the_arrangement_and_a_tool_window_with_sliders() {
    let (list, seeks) = app();
    let mut s = start(&Runtime::new(list));
    assert!(s.running());

    // The view shows: the lane is drawn in its coordinates.
    s.publish(ARRANGE, Some(arrangement(0.0)));
    block_on(s.frame(sized(None, false)));
    let out = s.output().clone();
    let lane = out.layers_on(ARRANGE, Plane::Over).next().expect("a layer");
    assert_eq!(
        flags(lane),
        vec![
            ("Verse".to_owned(), 200.0 + 4.0),
            ("Chorus".to_owned(), 600.0 + 4.0)
        ],
        "a flag at each marker's time, its name beside it"
    );
    let (y, h, color) = wash(lane).expect("a wash across the lane");
    assert_eq!((y, h), (80.0, 24.0), "on Vocals, 24 high");
    assert_eq!(color.channels()[3], 0x4D, "at 30%");
    assert_eq!(lane.regions.len(), 2, "each flag takes the pointer");
    assert_eq!(
        lane.hit(Point::new(300.0, 90.0)),
        None,
        "the rest passes through"
    );

    // The tool window: two sliders, labelled.
    let window = out.window(WINDOW).expect("the window");
    assert_eq!(window.title, "Marker lane");
    let labels: Vec<&str> = window
        .draw
        .shapes()
        .iter()
        .filter_map(|s| match s {
            Shape::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["24 pt", "Height", "30%", "Opacity", "2 markers"]);

    // Drag Opacity (the second slider, y = 8 + 19 + 6) to its right end.
    let frame_w = (300.0 - 16.0) * 0.65;
    block_on(s.frame(sized(Some((10.0, 36.0)), true)));
    block_on(s.frame(sized(Some((8.0 + frame_w, 36.0)), true)));
    block_on(s.frame(sized(Some((8.0 + frame_w, 36.0)), false)));
    let lane = s.output().layers[0].clone();
    assert_eq!(
        wash(&lane).map(|w| w.2.channels()[3]),
        Some(0xFF),
        "opaque now"
    );

    // Scrolled by two seconds, the flags move with the view.
    s.publish(ARRANGE, Some(arrangement(2.0)));
    block_on(s.frame(sized(None, false)));
    assert_eq!(flags(&s.output().layers[0])[0].1, 100.0 + 4.0);

    // Clicking Chorus's flag (at x 500) seeks there.
    let flag = Some(Point::new(502.0, 85.0));
    for down in [false, true, false] {
        s.pointer(
            ARRANGE,
            ViewPointer {
                at: flag,
                down,
                context: false,
            },
        );
        block_on(s.frame(sized(None, false)));
    }
    assert_eq!(*seeks.lock().unwrap(), vec![12.0]);

    // Its pane closed, the window says so and the script stops.
    let mut closed = sized(None, false);
    closed.closed.insert(WINDOW.into());
    block_on(s.frame(closed));
    assert!(s.output().windows.is_empty());
    assert!(!s.running());
}

#[test]
fn imgui_scripts_keep_reaimgui_shapes() {
    let (list, _) = app();
    let source = r#"
local ImGui = require 'imgui' '0.10'
assert(ImGui == require("imgui"))
local ctx = ImGui.CreateContext('Shape')
assert(ImGui.WindowFlags_AlwaysAutoResize == 64 and ImGui.Cond_FirstUseEver == 4)
local ok, why = pcall(ImGui.Text, ctx, 'no window')
assert(not ok and string.find(tostring(why), 'call Begin first'))
local visible, open = ImGui.Begin(ctx, 'Plain')
assert(visible and open == nil)
local changed, v = ImGui.SliderInt(ctx, 'Bars', 4, 1, 8)
assert(not changed and v == 4)
ImGui.End(ctx)
"#;
    let (report, session) = block_on(Session::start(
        &Runtime::new(list),
        &ActionCaller::app(),
        source,
        Value::Null,
        "S",
    ));
    assert_eq!(report.result, Ok(Value::Null), "{report:?}");
    let window = session
        .unwrap()
        .output()
        .window("Shape/Plain")
        .cloned()
        .unwrap();
    assert!(!window.closable);
}

const OVERLAY: &str = r#"
id = "fts.markers"
name = "Marker lane"
version = "0.1.0"
kind = "luau"
description = "Markers over the arrangement, and a window to tune them."

[capabilities]
actions = ["MARKERS_", "TRANSPORT_"]
effect = "mutating"

[[overlays]]
id = "lane"
view = "session.arrange"
script = "marker_lane.luau"
"#;

#[test]
fn an_extension_overlay_runs_inside_its_capabilities() {
    let package = Package::embedded(OVERLAY, &[("marker_lane.luau", SCRIPT)]).unwrap();
    let overlay = &package.manifest.overlays[0];
    let source = package.text(&overlay.script).unwrap();
    let (list, seeks) = app();
    let runtime = Runtime::new(list.clone()).with_gate(host::gate(&package.manifest));
    let (report, session) = block_on(Session::start(
        &runtime,
        &ActionCaller::app(),
        &source,
        json!({ "lane": "Kick" }),
        &package.manifest.name,
    ));
    assert!(report.result.is_ok(), "{report:?}");
    let mut s = session.unwrap();
    s.publish(&overlay.view, Some(arrangement(0.0)));
    block_on(s.frame(sized(None, false)));
    assert_eq!(
        wash(&s.output().layers[0]).map(|w| w.0),
        Some(20.0),
        "on Kick"
    );

    // Read-only, it may not seek: the click fails the frame, which stops it.
    let read_only = OVERLAY.replace("effect = \"mutating\"", "effect = \"read_only\"");
    let package = Package::embedded(&read_only, &[("marker_lane.luau", SCRIPT)]).unwrap();
    let runtime = Runtime::new(list).with_gate(host::gate(&package.manifest));
    let (_, s) = block_on(Session::start(
        &runtime,
        &ActionCaller::app(),
        &source,
        Value::Null,
        "S",
    ));
    let mut s = s.unwrap();
    s.publish(ARRANGE, Some(arrangement(0.0)));
    block_on(s.frame(sized(None, false)));
    let flag = Some(Point::new(202.0, 30.0));
    let mut report = None;
    for down in [false, true, false] {
        s.pointer(
            ARRANGE,
            ViewPointer {
                at: flag,
                down,
                context: false,
            },
        );
        report = Some(block_on(s.frame(sized(None, false))));
    }
    let error = report.unwrap().result.expect_err("refused");
    assert!(error.message.contains("TRANSPORT_SEEK"), "{error}");
    assert!(!s.running());
    assert_eq!(seeks.lock().unwrap().len(), 0);
}
