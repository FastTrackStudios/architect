//! The lab's demo window (`apps/lab/extensions/imgui-demo`), Dear
//! `ImGui`'s demo in architect's: it starts, and every section opens and
//! draws, the ones it cannot draw yet naming the API they need.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::{ActionCaller, ActionList};
use architect_canvas::imgui::{Input, WindowPointer};
use architect_canvas::luau::Session;
use architect_canvas::{Point, Shape, Size};
use architect_extensions::Package;
use architect_scripting::luau::Runtime;
use futures::executor::block_on;
use serde_json::Value;

const MANIFEST: &str = include_str!("../../../apps/lab/extensions/imgui-demo/extension.toml");
const SCRIPT: &str = include_str!("../../../apps/lab/extensions/imgui-demo/demo.luau");
const WINDOW: &str = "imgui-demo/Dear ImGui Demo";

fn input(pointer: Option<Point>, down: bool) -> Input {
    Input {
        pointer: pointer.map(|at| WindowPointer {
            window: WINDOW.into(),
            at,
        }),
        down,
        // Tall, so every section's buttons fit.
        sizes: [(WINDOW.to_owned(), Size::new(550.0, 2000.0))].into(),
        ..Input::default()
    }
}

/// The window's text, and where each is.
fn texts(s: &Session) -> Vec<(String, Point)> {
    let window = s.output().window(WINDOW).expect("the demo's window");
    window
        .draw
        .shapes()
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { text, at, .. } => Some((text.clone(), *at)),
            _ => None,
        })
        .collect()
}

fn frame(s: &mut Session, pointer: Option<Point>, down: bool) {
    let report = block_on(s.frame(input(pointer, down)));
    assert_eq!(report.result, Ok(Value::Null), "{:?}", report.log);
    assert!(s.running(), "the demo keeps running");
}

/// Clicks the button labelled `label` (its last, for a breadcrumb).
fn click(s: &mut Session, label: &str) {
    let (_, at) = texts(s)
        .into_iter()
        .rev()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("no {label:?} in {:?}", texts(s)));
    let at = Some(Point::new(at.x + 2.0, at.y + 4.0));
    frame(s, at, false);
    frame(s, at, true);
    frame(s, at, false);
    // A frame after the click, drawn with the section it opened.
    frame(s, None, false);
}

/// Opens each section under the one open, its own sections too, and goes
/// back; the names of the ones that say what they lack.
fn visit(s: &mut Session, path: &str, lacking: &mut Vec<String>) {
    let sections: Vec<String> = texts(s)
        .into_iter()
        .filter_map(|(text, _)| text.strip_prefix("▸ ").map(str::to_owned))
        .collect();
    for name in sections {
        click(s, &format!("▸ {name}"));
        let here = format!("{path}/{name}");
        let shown = texts(s);
        assert!(
            shown.iter().any(|(text, _)| *text == format!("▾ {name}")),
            "{here} opened: {shown:?}"
        );
        if shown
            .iter()
            .any(|(text, _)| text.starts_with("not implemented:"))
        {
            lacking.push(here.clone());
        }
        visit(s, &here, lacking);
        click(s, &format!("▾ {name}"));
    }
}

#[test]
fn the_demo_is_an_extension() {
    let package = Package::embedded(MANIFEST, &[("demo.luau", SCRIPT)]).expect("a package");
    assert_eq!(package.manifest.id, "fts.imgui-demo");
}

#[test]
fn every_section_of_the_demo_opens_and_names_what_it_lacks() {
    let (report, session) = block_on(Session::start(
        &Runtime::new(ActionList::new()),
        &ActionCaller::app(),
        SCRIPT,
        Value::Null,
        "ImGui demo",
    ));
    assert_eq!(report.result, Ok(Value::Null), "{:?}", report.log);
    let mut s = session.expect("it started");
    frame(&mut s, None, false);
    let shown: Vec<String> = texts(&s).into_iter().map(|(text, _)| text).collect();
    assert!(shown.contains(&"Click a section to open it.".to_owned()));
    for top in [
        "Help",
        "Widgets",
        "Layout & Scrolling",
        "Popups & Modal windows",
        "Tables & Columns",
        "Inputs & Focus",
    ] {
        assert!(shown.contains(&format!("▸ {top}")), "{top}: {shown:?}");
    }

    let mut lacking = Vec::new();
    visit(&mut s, "", &mut lacking);
    for section in [
        "/Help",
        "/Widgets/Combo",
        "/Widgets/Tree Nodes",
        "/Popups & Modal windows/Popups",
        "/Tables & Columns/Tables",
    ] {
        assert!(
            lacking.iter().any(|l| l == section),
            "{section}: {lacking:?}"
        );
    }

    // Basic's button answers.
    click(&mut s, "▸ Widgets");
    click(&mut s, "▸ Basic");
    click(&mut s, "Button");
    assert!(
        texts(&s)
            .iter()
            .any(|(text, _)| text == "Thanks for clicking me!")
    );
}
