//! Layers that move on their own: the painter draws them at each frame's
//! clocks without asking its view, only while a clock they read runs (or
//! was moved), and a view linked in composes to what the same view does
//! from across the wire.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::unnecessary_wraps,
    clippy::panic
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyrender::recording::RenderCommand;
use architect::action::{ActionCaller, ActionList};
use architect_scene::anim::{Animation, Clock, Input, Linear, Map, Name, Property, Reading, Steps};
use architect_scene::ops::{Receiver, Sender};
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};

use super::*;
use crate::PaneId;
use crate::kinds::{KindId, Placement, ViewHost};

/// What the test's clocks read: wall seconds, and the playhead.
#[derive(Clone, Default)]
struct Hands(Arc<Mutex<(f64, Option<Reading>)>>);

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

const fn playing(time: f64) -> Option<Reading> {
    Some(Reading::moving(time, 1.0))
}

const fn paused(time: f64) -> Option<Reading> {
    Some(Reading::still(time))
}

/// A playhead's line, a lyric line shown from 1 s to 2 s, a meter bound
/// to `view.level`, and a still layer under them.
struct Song {
    asked: Arc<AtomicUsize>,
    content: Arc<Vec<RenderCommand>>,
    level: f64,
}

impl Song {
    fn new(asked: Arc<AtomicUsize>) -> Self {
        let mut scene = Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(10, 120, 200),
            None,
            &Rect::new(0.0, 0.0, 4.0, 4.0),
        );
        Self {
            asked,
            content: Arc::new(scene.commands),
            level: 0.25,
        }
    }

    fn level() -> Name {
        Name::new("view.level").unwrap()
    }
}

impl PaintView for Song {
    fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
        false
    }

    fn paint(&mut self, _: &ViewCtx, _: &mut Scene) {}

    fn layers(&mut self, _: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        let layer = || Layer::new(Arc::clone(&self.content));
        Some(vec![
            (0, layer().at(Affine::translate((0.0, 50.0)))),
            (
                1,
                layer()
                    .at(Affine::translate((10.0, 0.0)) * Affine::scale(2.0))
                    .animated(vec![Animation::linear(
                        Property::X,
                        Input::clock_or(Clock::Playhead, Clock::Wall),
                        Linear::new(120.0, 0.0).wrap(400.0),
                    )])
                    .z(2),
            ),
            (
                2,
                layer().animated(vec![Animation::new(
                    Property::Visible,
                    Input::clock(Clock::Playhead),
                    Map::Steps(Steps {
                        before: 0.0,
                        steps: vec![(1.0, 1.0), (2.0, 0.0)],
                    }),
                )]),
            ),
            (
                3,
                layer().opacity(0.5).animated(vec![Animation::linear(
                    Property::ScaleY,
                    Input::Value(Self::level()),
                    Linear::new(10.0, 0.0).clamp(0.0, 10.0),
                )]),
            ),
        ])
    }

    fn values(&mut self, _: &ViewCtx) -> Vec<(Name, f64)> {
        vec![(Self::level(), self.level)]
    }
}

fn ctx() -> ViewCtx {
    ViewCtx::new(
        PaneId::new("song"),
        KindId::paint(),
        (ViewHost::Blitz, Placement::Pane),
        (ActionList::new(), ActionCaller::app()),
        crate::ctx::FrameClock::new(Duration::from_millis(4), || Duration::ZERO),
    )
}

const AT: Geometry = Geometry {
    width: 400.0,
    height: 300.0,
    scale: 2.0,
};

#[test]
fn a_view_linked_in_and_one_across_the_wire_compose_the_same_at_every_clock() {
    let hands = Hands::default();
    let mut linked = Painter::new(ctx(), Song::new(Arc::default())).with_clocks(hands.clocks());
    // The same view as a process runs it: its layers and values sent
    // once, its host's receiver composing them.
    let mut remote = Song::new(Arc::default());
    let ctx = ctx();
    let mut sender = Sender::new();
    let batch = sender
        .frame_with(&remote.layers(&ctx).unwrap(), &remote.values(&ctx))
        .unwrap();
    let mut receiver = Receiver::new();
    receiver.apply(&batch).unwrap();
    let clocks = hands.clocks();

    let mut seen = Vec::new();
    for (wall, playhead) in [
        (0.0, None),
        (0.75, None),
        (5.0, playing(0.5)),
        (5.1, playing(1.25)),
        (9.0, paused(1.4)),
        (9.5, paused(3.0)),
        (12.0, playing(2.5)),
    ] {
        hands.set(wall, playhead);
        let here = linked.frame(AT).flattened();
        let mut there = Scene::new();
        receiver.layers().compose_at(
            &mut there,
            Affine::scale(2.0),
            &clocks.read(),
            receiver.values(),
        );
        assert_eq!(
            here,
            there.flattened(),
            "at {wall} s, playhead {playhead:?}"
        );
        seen.push(format!("{here:?}"));
    }
    // ...and each of those clocks drew something else.
    seen.dedup();
    assert_eq!(seen.len(), 7);
}

#[test]
fn the_painter_draws_moving_layers_without_asking_its_view() {
    let hands = Hands::default();
    let asked = Arc::new(AtomicUsize::new(0));
    let mut painter =
        Painter::new(ctx(), Song::new(Arc::clone(&asked))).with_clocks(hands.clocks());
    let line_x = |scene: &Scene| -> Vec<f64> {
        scene
            .flattened()
            .iter()
            .filter_map(|c| match c {
                RenderCommand::Fill(f) => Some(f.transform.as_coeffs()[4]),
                _ => None,
            })
            .collect()
    };

    // Nothing plays: the line runs on wall time, every frame.
    hands.set(0.0, None);
    let first = line_x(&painter.frame(AT));
    let mut last = first;
    for frame in 1..=240 {
        hands.set(f64::from(frame) / 240.0, None);
        assert!(painter.animating());
        assert!(painter.wants_frame());
        let now = line_x(&painter.frame(AT));
        assert_ne!(now, last, "frame {frame} drew what the last did");
        last = now;
    }
    assert_eq!(asked.load(Ordering::Relaxed), 1, "the view was asked again");

    // Paused: nothing moves, so no frame is wanted...
    hands.set(2.0, paused(1.5));
    assert!(painter.wants_frame());
    let paused_at = line_x(&painter.frame(AT));
    hands.set(3.0, paused(1.5));
    assert!(!painter.animating());
    assert!(!painter.wants_frame());
    // ...until the playhead is moved, still with no word from the view.
    hands.set(3.0, paused(0.25));
    assert!(painter.wants_frame());
    let sought = line_x(&painter.frame(AT));
    assert_ne!(sought, paused_at);
    assert!(!painter.wants_frame());
    assert_eq!(asked.load(Ordering::Relaxed), 1);

    // What the view draws changed: it is asked.
    painter.invalidate();
    let _ = painter.frame(AT);
    assert_eq!(asked.load(Ordering::Relaxed), 2);
    // So is a view at a new size.
    let _ = painter.frame(Geometry { width: 200.0, ..AT });
    assert_eq!(asked.load(Ordering::Relaxed), 3);
}

#[test]
fn a_painters_values_are_its_views_own() {
    struct Writes(Vec<(Name, f64)>);
    impl PaintView for Writes {
        fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
            false
        }
        fn paint(&mut self, _: &ViewCtx, _: &mut Scene) {}
        fn layers(&mut self, _: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
            Some(Vec::new())
        }
        fn values(&mut self, _: &ViewCtx) -> Vec<(Name, f64)> {
            self.0.clone()
        }
    }
    let mine = Name::new("view.a").unwrap();
    let mut painter = Painter::new(ctx(), Writes(vec![(mine.clone(), 1.0)]));
    let _ = painter.frame(AT);
    assert_eq!(painter.values().unwrap().get(&mine), Some(1.0));
    // Another space's value fails the view, as from a process
    // (`a_view_fails_the_same_way_linked_in_or_across_the_wire`).
    let hosts = Name::new("host.fader").unwrap();
    let mut painter = Painter::new(ctx(), Writes(vec![(hosts, 2.0)]));
    let _ = painter.frame(AT);
    assert!(matches!(
        painter.failed(),
        Some(architect_scene::Error::Denied("write", _))
    ));
}

/// A view that returns `layers` and `values`, counting the times asked.
struct Returns {
    layers: Vec<(LayerKey, Layer)>,
    values: Vec<(Name, f64)>,
    asked: Arc<AtomicUsize>,
}

impl PaintView for Returns {
    fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
        false
    }

    fn paint(&mut self, _: &ViewCtx, _: &mut Scene) {}

    fn layers(&mut self, _: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Some(self.layers.clone())
    }

    fn values(&mut self, _: &ViewCtx) -> Vec<(Name, f64)> {
        self.values.clone()
    }
}

#[test]
fn a_view_fails_the_same_way_linked_in_or_across_the_wire() {
    let content = Song::new(Arc::default()).content;
    let wrapped = |every: f64| {
        Layer::new(Arc::clone(&content)).animated(vec![Animation::linear(
            Property::X,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0).wrap(every),
        )])
    };
    let cases: Vec<(Vec<(LayerKey, Layer)>, Vec<(Name, f64)>)> = vec![
        // A wrap of nothing.
        (vec![(1, wrapped(0.0))], vec![]),
        // A wrap below nothing.
        (vec![(1, wrapped(-1.0))], vec![]),
        // Writing the host's value.
        (vec![], vec![(Name::new("host.fader").unwrap(), 1.0)]),
    ];
    for (layers, values) in cases {
        let across = Receiver::new()
            .apply(&Sender::new().frame_with(&layers, &values).unwrap())
            .unwrap_err();
        let asked = Arc::new(AtomicUsize::new(0));
        let mut linked = Painter::new(
            ctx(),
            Returns {
                layers,
                values,
                asked: Arc::clone(&asked),
            },
        );
        let drawn = linked.frame(AT);
        assert_eq!(linked.failed(), Some(&across));
        assert!(drawn.flattened().is_empty(), "a failed view draws nothing");
        // ...and is asked nothing again, and wants no frame.
        linked.invalidate();
        assert!(!linked.wants_frame());
        let _ = linked.frame(AT);
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }
    // A well-formed one doesn't fail.
    let mut fine = Painter::new(
        ctx(),
        Returns {
            layers: vec![(1, wrapped(10.0))],
            values: vec![],
            asked: Arc::default(),
        },
    );
    let _ = fine.frame(AT);
    assert_eq!(fine.failed(), None);
}

#[test]
fn a_stepped_layer_rests_until_its_step_and_a_line_moves_every_frame() {
    use architect_scene::anim::Next;
    let hands = Hands::default();
    let content = Song::new(Arc::default()).content;
    let lyric = Layer::new(Arc::clone(&content)).animated(vec![Animation::new(
        Property::Visible,
        Input::clock(Clock::Playhead),
        Map::Steps(Steps {
            before: 0.0,
            steps: vec![(1.0, 1.0), (2.0, 0.0)],
        }),
    )]);
    let mut painter = Painter::new(
        ctx(),
        Returns {
            layers: vec![(1, lyric)],
            values: vec![],
            asked: Arc::default(),
        },
    )
    .with_clocks(hands.clocks());
    hands.set(0.0, playing(0.5));
    let _ = painter.frame(AT);
    // Playing, but nothing drawn changes for half a second: no frames.
    assert_eq!(painter.next(), Next::In(0.5));
    assert!(!painter.animating());
    hands.set(0.1, playing(0.75));
    assert!(!painter.wants_frame());
    // Its step came: a frame.
    hands.set(0.6, playing(1.1));
    assert!(painter.wants_frame());
}

/// A fader as a linked view: a track with a drag, a knob bound to it, and
/// what it heard.
struct Fader {
    content: Arc<Vec<RenderCommand>>,
    heard: Arc<Mutex<Vec<RegionEvent>>>,
    asked: Arc<AtomicUsize>,
}

fn fader_name() -> Name {
    Name::new("host.fader").unwrap()
}

fn fader_layers(content: &Arc<Vec<RenderCommand>>) -> Vec<(LayerKey, Layer)> {
    use architect_scene::region::{Axis, Drag, Region, Shape};
    let mut hovered = Layer::new(Arc::clone(content));
    hovered.visible = false;
    vec![
        (3, hovered),
        (
            1,
            Layer::new(Arc::clone(content)).regions(vec![
                Region::new(9, Shape::Rect(Rect::new(0.0, 0.0, 20.0, 100.0)))
                    .cursor(Cursor::NsResize)
                    .hover(Color::from_rgba8(255, 255, 255, 30))
                    // Its content redrawn hovered: layer 3, hidden.
                    .hover_layer(3)
                    .drag(Drag {
                        value: fader_name(),
                        axis: Axis::Y,
                        scale: -0.01,
                        min: 0.0,
                        max: 1.0,
                        step: None,
                        initial: 0.5,
                        coast: None,
                    }),
            ]),
        ),
        (
            2,
            Layer::new(Arc::clone(content))
                .z(1)
                .animated(vec![Animation::linear(
                    Property::Y,
                    Input::Value(fader_name()),
                    Linear::new(-100.0, 100.0),
                )]),
        ),
    ]
}

impl PaintView for Fader {
    fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
        panic!("a press on a region is the region's");
    }

    fn paint(&mut self, _: &ViewCtx, _: &mut Scene) {}

    fn layers(&mut self, _: &ViewCtx) -> Option<Vec<(LayerKey, Layer)>> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Some(fader_layers(&self.content))
    }

    fn region(&mut self, _: &ViewCtx, event: &RegionEvent) -> bool {
        self.heard.lock().unwrap().push(*event);
        false
    }
}

#[test]
fn a_linked_fader_follows_the_pointer_without_its_view_and_draws_as_one_across_the_wire() {
    use architect_scene::region::{Axis, Interaction};
    let content = Song::new(Arc::default()).content;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let asked = Arc::new(AtomicUsize::new(0));
    let mut painter = Painter::new(
        ctx(),
        Fader {
            content: Arc::clone(&content),
            heard: Arc::clone(&heard),
            asked: Arc::clone(&asked),
        },
    )
    .with_clocks(Hands::default().clocks());
    // The same view across the wire: a receiver, and the host's pointer.
    let mut receiver = Receiver::new();
    receiver
        .apply(&Sender::new().frame(&fader_layers(&content)).unwrap())
        .unwrap();
    let mut remote = Interaction::new();
    let clocks = Hands::default().clocks();
    let _ = painter.frame(AT);
    let point = |y: f32| ViewInput::PointerMove {
        x: 10.0,
        y,
        mods: vec![],
    };
    let mut both = |input: &ViewInput, y: f32| {
        let consumed = painter.pointer(input);
        let (layers, values) = receiver.parts_mut();
        let at = peniko::kurbo::Point::new(10.0, f64::from(y));
        let now = clocks.read();
        let o = match input {
            ViewInput::PointerDown { .. } => remote.pressed(layers, &now, values, at),
            ViewInput::PointerUp { .. } => remote.released(layers, &now, values, at),
            _ => remote.moved(layers, &now, values, at),
        };
        assert_eq!(consumed, o.consumed);
        let here = painter.frame(AT).flattened();
        let mut there = Scene::new();
        receiver.layers().compose_lit(
            &mut there,
            Affine::scale(2.0),
            &clocks.read(),
            receiver.values(),
            remote.lit(),
        );
        assert_eq!(here, there.flattened(), "at y {y}");
        here
    };
    let _ = both(&point(50.0), 50.0);
    let down = ViewInput::PointerDown {
        x: 10.0,
        y: 50.0,
        button: crate::ctx::Button::Primary,
        mods: vec![],
    };
    let _ = both(&down, 50.0);
    let mut last = Vec::new();
    for step in 1..=30_u8 {
        let y = 50.0 - f32::from(step);
        let drawn = both(&point(y), y);
        assert_ne!(drawn, last, "step {step} drew the last frame again");
        last = drawn;
    }
    let up = ViewInput::PointerUp {
        x: 10.0,
        y: 20.0,
        button: crate::ctx::Button::Primary,
        mods: vec![],
    };
    let _ = both(&up, 20.0);
    assert!((painter.values().unwrap().get(&fader_name()).unwrap() - 0.8).abs() < 1e-9);
    assert_eq!(asked.load(Ordering::Relaxed), 1, "the view was asked again");
    // It heard each frame's value once, and the release.
    let heard = heard.lock().unwrap();
    assert_eq!(heard[0], RegionEvent::Enter(9));
    assert_eq!(heard[1], RegionEvent::Press(9));
    let values = heard
        .iter()
        .filter(|e| matches!(e, RegionEvent::Value(9, Axis::Y, _)))
        .count();
    assert_eq!(values, 30);
    assert_eq!(heard.last(), Some(&RegionEvent::Release(9)));
}

#[test]
fn a_linked_regions_cursor_is_the_hosts_to_show() {
    let content = Song::new(Arc::default()).content;
    let mut painter = Painter::new(
        ctx(),
        Fader {
            content,
            heard: Arc::default(),
            asked: Arc::default(),
        },
    );
    let _ = painter.frame(AT);
    let at = |x: f32| ViewInput::PointerMove {
        x,
        y: 50.0,
        mods: vec![],
    };
    assert!(
        !painter.pointer(&at(10.0)),
        "a hover is still the view's to see"
    );
    assert_eq!(painter.take_cursor(), Some(Cursor::NsResize));
    assert!(painter.wants_frame(), "its highlight is drawn");
    let _ = painter.frame(AT);
    assert!(!painter.wants_frame());
    let _ = painter.pointer(&at(15.0));
    assert_eq!(painter.take_cursor(), None, "unchanged");
    let _ = painter.pointer(&at(300.0));
    assert_eq!(painter.take_cursor(), Some(Cursor::Default));
}

#[test]
fn a_flicked_list_coasts_on_the_host_without_its_view() {
    use architect_scene::region::{Axis, Drag, Region, Shape};
    let content = Song::new(Arc::default()).content;
    let scroll = Name::new("host.list").unwrap();
    let layers = vec![
        (
            1,
            Layer::new(Arc::clone(&content)).regions(vec![
                Region::new(3, Shape::Rect(Rect::new(0.0, 0.0, 100.0, 300.0))).drag(Drag {
                    value: scroll.clone(),
                    axis: Axis::Y,
                    scale: -1.0,
                    min: 0.0,
                    max: 5000.0,
                    step: None,
                    initial: 0.0,
                    coast: Some(0.4),
                }),
            ]),
        ),
        // The list's rows, scrolled by the value.
        (
            2,
            Layer::new(Arc::clone(&content))
                .z(1)
                .animated(vec![Animation::linear(
                    Property::Y,
                    Input::Value(scroll.clone()),
                    Linear::new(-1.0, 0.0),
                )]),
        ),
    ];
    let hands = Hands::default();
    let asked = Arc::new(AtomicUsize::new(0));
    let mut painter = Painter::new(
        ctx(),
        Returns {
            layers,
            values: vec![],
            asked: Arc::clone(&asked),
        },
    )
    .with_clocks(hands.clocks());
    let _ = painter.frame(AT);
    let at = |y: f32| (10.0_f32, y);
    // Pressed, flicked up 60 px in 30 ms, let go.
    hands.set(0.0, None);
    let (x, y) = at(250.0);
    assert!(painter.pointer(&ViewInput::PointerDown {
        x,
        y,
        button: crate::ctx::Button::Primary,
        mods: vec![],
    }));
    for n in 1..=3_u8 {
        hands.set(f64::from(n) * 0.01, None);
        let (x, y) = at(f32::from(n).mul_add(-20.0, 250.0));
        let _ = painter.pointer(&ViewInput::PointerMove { x, y, mods: vec![] });
    }
    let (x, y) = at(190.0);
    let _ = painter.pointer(&ViewInput::PointerUp {
        x,
        y,
        button: crate::ctx::Button::Primary,
        mods: vec![],
    });
    let _ = painter.frame(AT);
    let let_go = painter.values().unwrap().get(&scroll).unwrap();
    // It coasts on, every frame, the view asked nothing.
    let mut was = let_go;
    for frame in 1..=120_u8 {
        hands.set(0.03 + f64::from(frame) / 240.0, None);
        assert!(
            painter.animating(),
            "frame {frame}: it rests while it coasts"
        );
        let _ = painter.frame(AT);
        let now = painter.values().unwrap().get(&scroll).unwrap();
        assert!(now >= was, "frame {frame}");
        was = now;
    }
    assert!(was > let_go + 500.0, "it coasted from {let_go} to {was}");
    assert_eq!(asked.load(Ordering::Relaxed), 1);
    // Long after: still, and resting.
    hands.set(30.0, None);
    let _ = painter.frame(AT);
    assert!(!painter.animating());
}
