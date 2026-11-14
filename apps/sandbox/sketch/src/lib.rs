//! Sketch (#175, #176; #710 U3): drawing in an installed extension's view.
//!
//! - `sketch`: freehand strokes, drawn with a held pointer, kept in
//!   content space. Shift-drag (or the middle button) pans and the wheel
//!   zooms about the pointer: both change only the scene's transform, so
//!   a frame of panning is a few dozen bytes. Stands in for Excalidraw and
//!   Canvas.
//! - `graph`: a small link graph whose nodes drag. A [`PaintView`] given
//!   to `Builder::paint`, the same code linked into the app or here in
//!   its process. Stands in for the graph view.
//! - `arrangement`: 50 tracks of 40 clips, panned and zoomed: the
//!   arrangement-sized scene #175 measures.

// Geometry in floats and small colour sums: nothing to overflow, and
// written as the formulas read.
#![allow(clippy::arithmetic_side_effects, clippy::suboptimal_flops)]

use std::rc::Rc;

use architect_extensions::linked::{Builder, Extension};
use architect_extensions::scene::{Affine, RemoteScene, RenderCommand};
use architect_workspace::paint::peniko::kurbo::{BezPath, Circle, Line, Point, Rect, Stroke};
use architect_workspace::paint::peniko::{Color, Fill};
use architect_workspace::paint::{PaintScene as _, PaintView, Scene};
use architect_workspace::{ViewCtx, ViewInput};
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

/// The extension's id.
pub const ID: &str = "lab.sketch";

const INK: Color = Color::from_rgb8(0x2f, 0x6f, 0xeb);
const EDGE: Color = Color::from_rgb8(0x8a, 0x8f, 0x98);
const NODE: Color = Color::from_rgb8(0xe8, 0x6a, 0x3c);

/// The extension: its views, linked in or in its process.
pub struct Sketch;

impl Extension for Sketch {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("sketch", SketchView)
            .view("arrangement", ArrangementView)
            .paint::<Graph>("graph");
    }
}

/// Strokes as scene commands, in content space.
#[must_use]
pub fn strokes_scene(strokes: &[Vec<(f64, f64)>]) -> Vec<RenderCommand> {
    let mut scene = Scene::new();
    let pen = Stroke::new(2.5);
    for stroke in strokes {
        let mut path = BezPath::new();
        let mut points = stroke.iter();
        let Some(first) = points.next() else { continue };
        path.move_to(*first);
        if stroke.len() == 1 {
            path.line_to((first.0 + 0.01, first.1));
        }
        for p in points {
            path.line_to(*p);
        }
        scene.stroke(&pen, Affine::IDENTITY, INK, None, &path);
    }
    scene.commands
}

/// Pan and zoom: content to the element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    #[must_use]
    pub fn transform(self) -> Affine {
        Affine::translate((self.x, self.y)) * Affine::scale(self.zoom)
    }

    /// The content point under element point `(x, y)`.
    #[must_use]
    pub fn content(self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x) / self.zoom, (y - self.y) / self.zoom)
    }

    /// Zoomed by `factor` about element point `(x, y)`, which stays put.
    #[must_use]
    pub fn zoomed(self, factor: f64, x: f64, y: f64) -> Self {
        let zoom = (self.zoom * factor).clamp(0.05, 40.0);
        let f = zoom / self.zoom;
        Self {
            x: x - (x - self.x) * f,
            y: y - (y - self.y) * f,
            zoom,
        }
    }
}

/// What a held pointer is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    Idle,
    Drawing,
    Panning(f64, f64),
}

fn wants_pan(e: &PointerEvent) -> bool {
    e.modifiers().shift() || e.trigger_button() == Some(MouseButton::Auxiliary)
}

/// A wheel's turn as a zoom factor: down zooms out.
fn zoom_of(e: &WheelEvent) -> f64 {
    (-e.delta().strip_units().y * 0.002).exp()
}

/// Freehand drawing with pan and zoom.
#[component]
pub fn SketchView() -> Element {
    let mut strokes = use_signal(Vec::<Vec<(f64, f64)>>::new);
    let mut camera = use_signal(Camera::default);
    let mut gesture = use_signal(|| Gesture::Idle);
    let content = use_memo(move || Rc::new(strokes_scene(&strokes.read())));
    rsx! {
        RemoteScene {
            content: content(),
            transform: camera().transform(),
            onpointerdown: move |e: PointerEvent| {
                let p = e.element_coordinates();
                if wants_pan(&e) {
                    gesture.set(Gesture::Panning(p.x, p.y));
                } else {
                    gesture.set(Gesture::Drawing);
                    let at = camera().content(p.x, p.y);
                    strokes.write().push(vec![at]);
                }
            },
            onpointermove: move |e: PointerEvent| {
                let p = e.element_coordinates();
                match gesture() {
                    Gesture::Drawing => {
                        let at = camera().content(p.x, p.y);
                        if let Some(last) = strokes.write().last_mut() {
                            last.push(at);
                        }
                    }
                    Gesture::Panning(x, y) => {
                        let mut c = camera();
                        c.x += p.x - x;
                        c.y += p.y - y;
                        camera.set(c);
                        gesture.set(Gesture::Panning(p.x, p.y));
                    }
                    Gesture::Idle => {}
                }
            },
            onpointerup: move |_| gesture.set(Gesture::Idle),
            onwheel: move |e: WheelEvent| {
                let p = e.element_coordinates();
                camera.set(camera().zoomed(zoom_of(&e), p.x, p.y));
            },
        }
    }
}

/// The arrangement's clips, in content space: `tracks` lanes of `clips`.
#[must_use]
pub fn arrangement_scene(tracks: u32, clips: u32) -> Vec<RenderCommand> {
    let mut scene = Scene::new();
    for t in 0..tracks {
        let y = f64::from(t) * 24.0;
        for c in 0..clips {
            let x = f64::from(c) * 64.0 + f64::from(t % 3) * 8.0;
            let shade = u8::try_from((t * 7 + c * 13) % 120).unwrap_or(0);
            let fill = Color::from_rgb8(40 + shade, 90, 160 - shade / 2);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                fill,
                None,
                &Rect::new(x, y + 2.0, x + 56.0, y + 22.0),
            );
        }
    }
    scene.commands
}

/// 50 tracks of 40 clips; drag or the wheel pans, Ctrl-wheel zooms.
#[component]
pub fn ArrangementView() -> Element {
    let content = use_hook(|| Rc::new(arrangement_scene(50, 40)));
    let mut camera = use_signal(Camera::default);
    let mut grab = use_signal(|| None::<(f64, f64)>);
    rsx! {
        RemoteScene {
            content,
            transform: camera().transform(),
            onpointerdown: move |e: PointerEvent| {
                let p = e.element_coordinates();
                grab.set(Some((p.x, p.y)));
            },
            onpointermove: move |e: PointerEvent| {
                if let Some((x, y)) = grab() {
                    let p = e.element_coordinates();
                    let mut c = camera();
                    c.x += p.x - x;
                    c.y += p.y - y;
                    camera.set(c);
                    grab.set(Some((p.x, p.y)));
                }
            },
            onpointerup: move |_| grab.set(None),
            onwheel: move |e: WheelEvent| {
                let p = e.element_coordinates();
                if e.modifiers().ctrl() {
                    camera.set(camera().zoomed(zoom_of(&e), p.x, p.y));
                } else {
                    let d = e.delta().strip_units();
                    let mut c = camera();
                    c.x -= d.x;
                    c.y -= d.y;
                    camera.set(c);
                }
            },
        }
    }
}

/// A small link graph whose nodes drag: a paint view.
#[derive(Debug)]
pub struct Graph {
    nodes: Vec<Point>,
    edges: Vec<(usize, usize)>,
    dragging: Option<usize>,
    placed: bool,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: vec![
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (0, 4),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
                (2, 6),
            ],
            dragging: None,
            placed: false,
        }
    }
}

/// A node's radius.
pub const NODE_RADIUS: f64 = 10.0;

impl Graph {
    fn place(&mut self, ctx: &ViewCtx) {
        if self.placed {
            return;
        }
        let (w, h) = (f64::from(ctx.geometry.width), f64::from(ctx.geometry.height));
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = w.min(h) * 0.35;
        self.nodes = (0..8)
            .map(|i| {
                let a = f64::from(i) / 8.0 * std::f64::consts::TAU;
                Point::new(w / 2.0 + r * a.cos(), h / 2.0 + r * a.sin())
            })
            .collect();
        self.placed = true;
    }

    /// The node under `at`, if any.
    #[must_use]
    pub fn node_at(&self, at: Point) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.distance(at) <= NODE_RADIUS + 2.0)
    }

    /// Where the nodes are now.
    #[must_use]
    pub fn nodes(&self) -> &[Point] {
        &self.nodes
    }
}

impl PaintView for Graph {
    fn input(&mut self, ctx: &ViewCtx, event: &ViewInput) -> bool {
        self.place(ctx);
        match *event {
            ViewInput::PointerDown { x, y, .. } => {
                self.dragging = self.node_at(Point::new(f64::from(x), f64::from(y)));
                false
            }
            ViewInput::PointerMove { x, y } => {
                let Some(n) = self.dragging else { return false };
                if let Some(node) = self.nodes.get_mut(n) {
                    *node = Point::new(f64::from(x), f64::from(y));
                }
                true
            }
            ViewInput::PointerUp { .. } => {
                self.dragging = None;
                false
            }
            _ => false,
        }
    }

    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
        self.place(ctx);
        let pen = Stroke::new(1.5);
        for (a, b) in &self.edges {
            if let (Some(a), Some(b)) = (self.nodes.get(*a), self.nodes.get(*b)) {
                scene.stroke(&pen, Affine::IDENTITY, EDGE, None, &Line::new(*a, *b));
            }
        }
        for n in &self.nodes {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                NODE,
                None,
                &Circle::new(*n, NODE_RADIUS),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let c = Camera::default().zoomed(2.0, 100.0, 50.0);
        let (x, y) = c.content(100.0, 50.0);
        assert!((x - 100.0).abs() < 1e-9 && (y - 50.0).abs() < 1e-9);
        assert!((c.zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_stroke_is_one_command_and_the_arrangement_two_thousand() {
        assert_eq!(strokes_scene(&[vec![(0.0, 0.0), (5.0, 5.0)]]).len(), 1);
        assert_eq!(arrangement_scene(50, 40).len(), 2000);
    }
}
