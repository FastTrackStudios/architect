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
//! - `timeline`: the same clips with a playhead moving over them, as a
//!   [`PaintView`] that returns layers: the clips are sent once, and the
//!   playhead once with how it moves (on the app's playhead, or wall time
//!   when nothing plays), so the host runs playback and a frame of it
//!   sends nothing.

// Geometry in floats and small colour sums: nothing to overflow, and
// written as the formulas read.
#![allow(clippy::arithmetic_side_effects, clippy::suboptimal_flops)]

use std::rc::Rc;

use architect_extensions::linked::{Builder, Extension};
use architect_extensions::scene::{Affine, RemoteScene, RenderCommand};
use architect_workspace::paint::anim::{Animation, Clock, Input, Linear, Name, Property};
use architect_workspace::paint::peniko::kurbo::{BezPath, Circle, Line, Point, Rect, Stroke};
use architect_workspace::paint::peniko::{Color, Fill};
use architect_workspace::paint::region::{Axis, Cursor, Drag, Region, RegionEvent, Shape};
use architect_workspace::paint::{Layer, LayerKey, PaintScene as _, PaintView, Scene};
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
            .paint::<Graph>("graph")
            .paint::<Timeline>("timeline");
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
            onpointercancel: move |_| gesture.set(Gesture::Idle),
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
            onpointercancel: move |_| grab.set(None),
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
        let (w, h) = (
            f64::from(ctx.geometry.width),
            f64::from(ctx.geometry.height),
        );
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
            ViewInput::PointerMove { x, y, .. } => {
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

/// How far the timeline's playhead moves a second, in content space.
pub const PLAYHEAD_SPEED: f64 = 120.0;

/// How wide the timeline's clips are, in content space: its playhead
/// loops over them on wall time.
pub const TIMELINE_WIDTH: f64 = 40.0 * 64.0;

/// The timeline's clips layer, and its playhead's.
pub const CLIPS: LayerKey = 1;
pub const PLAYHEAD: LayerKey = 2;
/// Its mixer strip: the fader's track, its knob, and a box to drag.
pub const TRACK: LayerKey = 3;
pub const KNOB: LayerKey = 4;
pub const BOX: LayerKey = 5;
/// Its meter, drawn from the engine's value.
pub const METER: LayerKey = 6;
/// The drag box as it looks hovered: sent once, hidden, and shown by the
/// host while the pointer is on the box (`Region::hover_layer`).
pub const BOX_HOVER: LayerKey = 7;

/// The fader's region, and the box's.
pub const FADER: u64 = 1;
pub const DRAG_BOX: u64 = 2;

/// Where the fader's track is in the pane, and how tall: its value runs
/// from 0 at the bottom to 1 at the top.
pub const TRACK_AT: (f64, f64) = (8.0, 8.0);
pub const TRACK_HEIGHT: f64 = 120.0;
/// Where the box starts.
pub const BOX_AT: (f64, f64) = (48.0, 8.0);

/// The knob's colour: what a test finds it by.
pub const KNOB_COLOR: Color = Color::from_rgb8(0xf2, 0xc1, 0x2e);
/// The box's.
pub const BOX_COLOR: Color = Color::from_rgb8(0x3c, 0xb3, 0x71);
/// The drag box hovered: redrawn in this colour, not a box over it.
pub const BOX_HOVER_COLOR: Color = Color::from_rgb8(0x6e, 0xe0, 0x9a);
/// The meter's.
pub const METER_COLOR: Color = Color::from_rgb8(0x4c, 0xd9, 0x64);

/// The values the host keeps for them: the pointer writes these.
#[must_use]
pub fn fader_value() -> Option<Name> {
    Name::new("host.timeline.fader").ok()
}

#[must_use]
pub fn box_values() -> Option<(Name, Name)> {
    Some((
        Name::new("host.timeline.box.x").ok()?,
        Name::new("host.timeline.box.y").ok()?,
    ))
}

/// What the engine writes for the meter, lock-free
/// (`architect_scene::engine`): read only with the `engine` capability.
#[must_use]
pub fn meter_value() -> Option<Name> {
    Name::new("engine.timeline.meter").ok()
}

/// What the view keeps of its fader once it is let go: its own value.
#[must_use]
pub fn level_value() -> Option<Name> {
    Name::new("view.timeline.level").ok()
}

fn filled(
    shape: &impl architect_workspace::paint::peniko::kurbo::Shape,
    color: Color,
) -> std::sync::Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, shape);
    std::sync::Arc::new(scene.commands)
}

/// An arrangement's clips with a playhead moving over them: a paint view
/// that keeps what it draws.
///
/// The clips are recorded once and the playhead once, with how it moves:
/// [`PLAYHEAD_SPEED`] content units a second of the app's playhead, or
/// of wall time (looping over the clips) when nothing is on it. Its host
/// runs that every frame, so playback, a pause or a seek asks nothing of
/// the view; a pan or a zoom is a new transform for each layer, and from
/// a process nothing but that crosses.
#[derive(Debug)]
pub struct Timeline {
    clips: std::sync::Arc<Vec<RenderCommand>>,
    playhead: std::sync::Arc<Vec<RenderCommand>>,
    track: std::sync::Arc<Vec<RenderCommand>>,
    knob: std::sync::Arc<Vec<RenderCommand>>,
    drag_box: std::sync::Arc<Vec<RenderCommand>>,
    box_hover: std::sync::Arc<Vec<RenderCommand>>,
    meter: std::sync::Arc<Vec<RenderCommand>>,
    camera: Camera,
    panning: Option<(f64, f64)>,
    /// The fader's level as the view last heard it, and once let go.
    heard: f64,
    level: Option<f64>,
}

impl Default for Timeline {
    fn default() -> Self {
        let mut line = Scene::new();
        line.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            NODE,
            None,
            &Line::new((0.0, 0.0), (0.0, 50.0 * 24.0)),
        );
        Self {
            clips: std::sync::Arc::new(arrangement_scene(50, 40)),
            playhead: std::sync::Arc::new(line.commands),
            track: filled(
                &Rect::new(0.0, 0.0, 24.0, TRACK_HEIGHT),
                Color::from_rgb8(0x2a, 0x2d, 0x33),
            ),
            knob: filled(&Rect::new(0.0, -4.0, 24.0, 4.0), KNOB_COLOR),
            drag_box: filled(&Rect::new(0.0, 0.0, 40.0, 24.0), BOX_COLOR),
            box_hover: filled(&Rect::new(0.0, 0.0, 40.0, 24.0), BOX_HOVER_COLOR),
            // Its bottom at the layer's origin: scaled down toward it.
            meter: filled(&Rect::new(0.0, -TRACK_HEIGHT, 6.0, 0.0), METER_COLOR),
            camera: Camera::default(),
            panning: None,
            heard: 0.75,
            level: None,
        }
    }
}

impl PaintView for Timeline {
    fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
        match *event {
            ViewInput::PointerDown { x, y, .. } => {
                self.panning = Some((f64::from(x), f64::from(y)));
                false
            }
            ViewInput::PointerMove { x, y, .. } => {
                let Some((px, py)) = self.panning else {
                    return false;
                };
                let (x, y) = (f64::from(x), f64::from(y));
                self.camera.x += x - px;
                self.camera.y += y - py;
                self.panning = Some((x, y));
                true
            }
            ViewInput::PointerUp { .. } => {
                self.panning = None;
                false
            }
            ViewInput::Wheel { dy, x, y, .. } => {
                let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
                self.camera = self.camera.zoomed(factor, f64::from(x), f64::from(y));
                true
            }
            _ => false,
        }
    }

    fn paint(&mut self, _: &ViewCtx, _: &mut Scene) {}

    // The fader and the box are the host's to move; the view hears where
    // they went, and keeps the fader's level once it is let go.
    fn region(&mut self, _: &ViewCtx, event: &RegionEvent) -> bool {
        match *event {
            RegionEvent::Value(FADER, _, v) => {
                self.heard = v;
                false
            }
            RegionEvent::Release(FADER) => {
                self.level = Some(self.heard);
                true
            }
            _ => false,
        }
    }

    fn values(&mut self, _: &ViewCtx) -> Vec<(Name, f64)> {
        self.level
            .zip(level_value())
            .map(|(level, name)| vec![(name, level)])
            .unwrap_or_default()
    }

    fn layers(&mut self, _: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        let view = self.camera.transform();
        let strip = Affine::translate(TRACK_AT);
        let fader = Region::new(FADER, Shape::Rect(Rect::new(0.0, 0.0, 24.0, TRACK_HEIGHT)))
            .cursor(Cursor::NsResize)
            .hover(Color::from_rgba8(255, 255, 255, 24))
            .drag(Drag {
                value: fader_value()?,
                axis: Axis::Y,
                scale: -1.0 / TRACK_HEIGHT,
                min: 0.0,
                max: 1.0,
                step: None,
                initial: 0.75,
                coast: None,
            });
        let (bx, by) = box_values()?;
        let along = |value: Name, axis, max| Drag {
            value,
            axis,
            scale: 1.0,
            min: 0.0,
            max,
            step: None,
            initial: 0.0,
            coast: None,
        };
        let handle = Region::new(DRAG_BOX, Shape::Rect(Rect::new(0.0, 0.0, 40.0, 24.0)))
            .cursor(Cursor::Move)
            .hover_layer(BOX_HOVER)
            .press(Color::from_rgba8(0, 0, 0, 60))
            .drag(along(bx.clone(), Axis::X, 400.0))
            .drag(along(by.clone(), Axis::Y, 300.0));
        let follow =
            |property, name| Animation::linear(property, Input::Value(name), Linear::new(1.0, 0.0));
        Some(vec![
            (
                CLIPS,
                Layer::new(std::sync::Arc::clone(&self.clips)).at(view),
            ),
            (
                PLAYHEAD,
                Layer::new(std::sync::Arc::clone(&self.playhead))
                    .at(view)
                    .animated(vec![playhead_motion()])
                    .z(1),
            ),
            (
                TRACK,
                Layer::new(std::sync::Arc::clone(&self.track))
                    .at(strip)
                    .z(2)
                    .regions(vec![fader]),
            ),
            (
                KNOB,
                Layer::new(std::sync::Arc::clone(&self.knob))
                    .at(strip)
                    .z(3)
                    .animated(vec![Animation::linear(
                        Property::Y,
                        Input::Value(fader_value()?),
                        Linear::new(-TRACK_HEIGHT, TRACK_HEIGHT),
                    )]),
            ),
            (
                METER,
                Layer::new(std::sync::Arc::clone(&self.meter))
                    .at(Affine::translate((
                        TRACK_AT.0 + 28.0,
                        TRACK_AT.1 + TRACK_HEIGHT,
                    )))
                    .z(2)
                    .animated(vec![Animation::linear(
                        Property::ScaleY,
                        Input::Value(meter_value()?),
                        Linear::new(1.0, 0.0).clamp(0.0, 1.0),
                    )]),
            ),
            (
                BOX,
                Layer::new(std::sync::Arc::clone(&self.drag_box))
                    .at(Affine::translate(BOX_AT))
                    .z(4)
                    .regions(vec![handle])
                    .animated(vec![follow(Property::X, bx), follow(Property::Y, by)]),
            ),
            (BOX_HOVER, {
                let mut hovered = Layer::new(std::sync::Arc::clone(&self.box_hover));
                hovered.visible = false;
                hovered
            }),
        ])
    }
}

/// How the timeline's playhead moves: its `x` in content space is
/// [`PLAYHEAD_SPEED`] × the app's playhead, or × wall time when nothing
/// is on it, looping over the clips.
#[must_use]
pub const fn playhead_motion() -> Animation {
    Animation::linear(
        Property::X,
        Input::clock_or(Clock::Playhead, Clock::Wall),
        Linear::new(PLAYHEAD_SPEED, 0.0).wrap(TIMELINE_WIDTH),
    )
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
