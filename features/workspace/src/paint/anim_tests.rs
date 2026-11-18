//! Layers that move on their own: the painter draws them at each frame's
//! clocks without asking its view, only while a clock they read runs (or
//! was moved), and a view linked in composes to what the same view does
//! from across the wire.

#![allow(clippy::arithmetic_side_effects, clippy::unnecessary_wraps)]

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
        let here = linked.frame(AT).commands;
        let mut there = Scene::new();
        receiver.layers().compose_at(
            &mut there,
            Affine::scale(2.0),
            &clocks.read(),
            receiver.values(),
        );
        assert_eq!(here, there.commands, "at {wall} s, playhead {playhead:?}");
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
            .commands
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
    let bound_to = |name: &str| {
        Layer::new(Arc::clone(&content)).animated(vec![Animation::linear(
            Property::X,
            Input::Value(Name::new(name).unwrap()),
            Linear::new(1.0, 0.0),
        )])
    };
    let cases: Vec<(Vec<(LayerKey, Layer)>, Vec<(Name, f64)>)> = vec![
        // A wrap of nothing.
        (vec![(1, wrapped(0.0))], vec![]),
        // Bound to the engine's value, ungranted.
        (vec![(1, bound_to("engine.peak"))], vec![]),
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
        assert!(drawn.commands.is_empty(), "a failed view draws nothing");
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
