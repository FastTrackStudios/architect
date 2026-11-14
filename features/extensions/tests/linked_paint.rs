//! #591: a linked extension's paint view. `Builder::paint` registers a
//! view that records an anyrender scene for its pane and takes the pane's
//! input; mounted in a workspace pane on Blitz, a press reaches it in the
//! pane's own coordinates and what it draws is in the window's scene.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::suboptimal_flops
)]

use std::sync::Mutex;

use architect_extensions::linked::{Builder, Extension};
use architect_workspace::paint::peniko::kurbo::{Affine, Circle};
use architect_workspace::paint::peniko::{Color, Fill};
use architect_workspace::paint::{PaintScene as _, PaintView, Scene};
use architect_workspace::{ViewCtx, ViewInput};
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

/// Where the view was pressed, in its own pixels.
static PRESSED: Mutex<Vec<(f32, f32)>> = Mutex::new(Vec::new());

const INK: Color = Color::from_rgb8(10, 200, 30);

/// A dot where it is pressed.
#[derive(Default)]
struct Dots(Vec<(f32, f32)>);

impl PaintView for Dots {
    fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
        if let ViewInput::PointerDown { x, y, .. } = event {
            self.0.push((*x, *y));
            PRESSED.lock().unwrap().push((*x, *y));
            return true;
        }
        false
    }

    fn paint(&mut self, _: &ViewCtx, scene: &mut Scene) {
        for (x, y) in &self.0 {
            let dot = Circle::new((f64::from(*x), f64::from(*y)), 6.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, INK, None, &dot);
        }
    }
}

struct Sketchy;

impl Extension for Sketchy {
    fn manifest(&self) -> &'static str {
        r#"
id = "test.sketchy"
name = "Sketchy"
version = "0.1.0"
kind = "linked"
api = "0.1"

[[views]]
id = "dots"
title = "Dots"
kind = "paint"
"#
    }

    fn build(&self, ext: &mut Builder) {
        ext.paint::<Dots>("dots");
    }
}

fn app() -> Element {
    let workspace = use_signal(|| {
        let mut w = architect_workspace::Workspace::new();
        w.open(
            architect_workspace::Pane::new("dots", "dots", "Dots"),
            architect_workspace::Target::Tab,
        )
        .unwrap();
        w
    });
    let views = use_hook(|| {
        let root = Builder::of(&Sketchy).view_root("dots").unwrap();
        architect_workspace::Views::new().view(architect_workspace::View::new(
            "dots",
            "Dots",
            move |_| root(),
        ))
    });
    rsx! {
        div { style: "width: 400px; height: 300px;",
            architect_workspace::WorkspaceView { workspace, views }
        }
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

/// The widget's node, and where it is in the window.
fn widget(doc: &DioxusDocument) -> (f32, f32, f32, f32) {
    let d = doc.inner.borrow();
    let node = d
        .query_selector("object")
        .unwrap()
        .expect("the paint widget");
    let n = d.get_node(node).unwrap();
    let at = n.absolute_position(0.0, 0.0);
    let size = n.final_layout().size;
    (at.x, at.y, size.width, size.height)
}

#[test]
fn a_linked_paint_view_takes_the_panes_input_and_draws_in_the_window() {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app),
        DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().resolve(0.0);
    let (x, y, w, h) = widget(&doc);
    assert!(w > 100.0 && h > 100.0, "the widget fills its pane: {w}×{h}");

    // A press in the middle reaches the view in its own pixels.
    let (px, py) = (x + w / 2.0, y + h / 2.0);
    doc.handle_ui_event(UiEvent::PointerMove(press(px, py, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(press(
        px,
        py,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(press(px, py, MouseEventButtons::None)));
    let pressed = PRESSED.lock().unwrap().clone();
    assert_eq!(pressed.len(), 1, "one press: {pressed:?}");
    let (vx, vy) = pressed[0];
    assert!(
        (vx - w / 2.0).abs() < 1.0 && (vy - h / 2.0).abs() < 1.0,
        "{pressed:?}"
    );

    // What it draws is in the window's scene.
    let mut scene = Scene::new();
    blitz_paint::paint_scene(&mut scene, &mut doc.inner.borrow_mut(), 1.0, 400, 300, 0, 0);
    let drew = format!("{:?}", scene.commands);
    assert!(
        drew.contains(&format!("{INK:?}")),
        "no dot in the window's scene"
    );
}
