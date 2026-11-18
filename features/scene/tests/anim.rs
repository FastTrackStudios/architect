//! Animations a host runs: what they evaluate to at a clock or a value,
//! what crosses for them, and what a host refuses.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use std::sync::Arc;

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene as _, Scene};
use architect_scene::Error;
use architect_scene::anim::{
    Animation, Clock, Input, Linear, MAX_ANIMATIONS, Map, Name, Property, Reading, Readings, Space,
    Steps, Values, place,
};
use architect_scene::layers::{Layer, Layers};
use architect_scene::ops::{Receiver, Sender};
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};

fn square() -> Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(200, 30, 30),
        None,
        &Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    Arc::new(scene.commands)
}

/// A playhead's line: 120 content units a second on the playhead, on
/// wall time when nothing plays.
const fn playhead() -> Animation {
    Animation::linear(
        Property::X,
        Input::clock_or(Clock::Playhead, Clock::Wall),
        Linear::new(120.0, 0.0),
    )
}

const fn wall(t: f64) -> Readings {
    Readings {
        wall: Some(t),
        playhead: None,
    }
}

const fn at(wall: f64, time: f64, running: bool) -> Readings {
    Readings {
        wall: Some(wall),
        playhead: Some(Reading {
            time,
            rate: if running { 1.0 } else { 0.0 },
        }),
    }
}

fn x(layer: &Layer, r: &Readings) -> f64 {
    place(layer, r, &Values::new()).transform.as_coeffs()[4]
}

#[test]
fn a_playheads_line_follows_the_playhead_whatever_it_does() {
    let line = Layer::new(square()).animated(vec![playhead()]);
    // Playing: it moves with the playhead's time.
    assert_eq!(x(&line, &at(100.0, 1.0, true)), 120.0);
    assert_eq!(x(&line, &at(100.5, 1.5, true)), 180.0);
    // Paused: the playhead's time stands, whatever the wall clock does.
    assert_eq!(x(&line, &at(200.0, 1.5, false)), 180.0);
    assert_eq!(x(&line, &at(300.0, 1.5, false)), 180.0);
    // Seeking back, or at another rate: only the reading changes.
    assert_eq!(x(&line, &at(301.0, 0.25, false)), 30.0);
    // Nothing on the playhead: wall time.
    assert_eq!(x(&line, &wall(2.0)), 240.0);
    // No clock at all: where it was sent.
    assert_eq!(x(&line, &Readings::NONE), 0.0);
}

#[test]
fn a_move_is_in_content_space_after_the_layers_own_transform() {
    // Zoomed 2× and panned 10: a second of playback is 240 pixels on.
    let line = Layer::new(square())
        .at(Affine::translate((10.0, 0.0)) * Affine::scale(2.0))
        .animated(vec![playhead()]);
    assert_eq!(x(&line, &at(0.0, 1.0, true)), 250.0);
}

#[test]
fn a_line_wraps_then_clamps() {
    let map = Map::Linear(Linear::new(100.0, 0.0).wrap(250.0));
    let looping = Layer::new(square()).animated(vec![Animation::new(
        Property::X,
        Input::clock(Clock::Wall),
        map,
    )]);
    assert_eq!(x(&looping, &wall(3.0)), 50.0);
    let clamped = Layer::new(square()).animated(vec![Animation::linear(
        Property::Y,
        Input::clock(Clock::Wall),
        Linear::new(-10.0, 5.0).clamp(-20.0, 0.0),
    )]);
    let y = |t| {
        place(&clamped, &wall(t), &Values::new())
            .transform
            .as_coeffs()[5]
    };
    assert_eq!(y(0.0), 0.0);
    assert_eq!(y(1.0), -5.0);
    assert_eq!(y(9.0), -20.0);
}

#[test]
fn a_lyric_line_is_drawn_between_its_times() {
    let line = Layer::new(square()).animated(vec![Animation::new(
        Property::Visible,
        Input::clock(Clock::Playhead),
        Map::Steps(Steps {
            before: 0.0,
            steps: vec![(12.0, 1.0), (15.5, 0.0)],
        }),
    )]);
    let shown = |t| place(&line, &at(0.0, t, true), &Values::new()).visible;
    assert!(!shown(11.9));
    assert!(shown(12.0));
    assert!(shown(15.4));
    assert!(!shown(15.5));
    let mut layers = Layers::new();
    layers.set(1, line);
    let mut scene = Scene::new();
    layers.compose_at(
        &mut scene,
        Affine::IDENTITY,
        &at(0.0, 13.0, false),
        &Values::new(),
    );
    assert_eq!(scene.commands.len(), 1);
    let mut scene = Scene::new();
    layers.compose_at(
        &mut scene,
        Affine::IDENTITY,
        &at(0.0, 16.0, false),
        &Values::new(),
    );
    assert!(scene.commands.is_empty());
}

#[test]
fn a_meter_is_drawn_from_a_named_value() {
    let level = Name::new("view.meter.left").unwrap();
    // A bar 100 high at full scale: its height is the level, clamped.
    let bar = Layer::new(square()).animated(vec![Animation::linear(
        Property::ScaleY,
        Input::Value(level.clone()),
        Linear::new(10.0, 0.0).clamp(0.0, 10.0),
    )]);
    let mut values = Values::new();
    let height = |v: &Values| place(&bar, &Readings::NONE, v).transform.as_coeffs()[3];
    // Never written: as sent.
    assert_eq!(height(&values), 1.0);
    assert!(values.write(Space::View, &level, 0.5).unwrap());
    assert_eq!(height(&values), 5.0);
    assert!(values.write(Space::View, &level, 3.0).unwrap());
    assert_eq!(height(&values), 10.0);
    // The same number again changes nothing.
    let revision = values.revision();
    assert!(!values.write(Space::View, &level, 3.0).unwrap());
    assert_eq!(values.revision(), revision);
}

#[test]
fn opacity_multiplies_and_stays_between_zero_and_one() {
    let fade = Layer::new(square())
        .opacity(0.5)
        .animated(vec![Animation::linear(
            Property::Opacity,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0),
        )]);
    let o = |t| place(&fade, &wall(t), &Values::new()).opacity;
    assert_eq!(o(0.5), 0.25);
    assert_eq!(o(7.0), 0.5);
    assert_eq!(o(-1.0), 0.0);
}

#[test]
fn a_host_draws_every_frame_only_while_a_clock_one_reads_runs() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(square()));
    assert!(!layers.moving(&wall(1.0)));
    layers.set(2, Layer::new(square()).animated(vec![playhead()]));
    // Playing, or nothing on the playhead (wall time): every frame.
    assert!(layers.moving(&at(0.0, 1.0, true)));
    assert!(layers.moving(&wall(1.0)));
    // Paused: not every frame...
    let paused = at(5.0, 1.0, false);
    assert!(!layers.moving(&paused));
    assert!(!layers.stale(&paused, &at(9.0, 1.0, false)));
    // ...but a seek that moves it is drawn.
    assert!(layers.stale(&paused, &at(9.0, 2.0, false)));
    // A layer bound to a value alone never is: its value's change is.
    let mut bound = Layers::new();
    bound.set(
        1,
        Layer::new(square()).animated(vec![Animation::linear(
            Property::X,
            Input::Value(Name::new("view.x").unwrap()),
            Linear::new(1.0, 0.0),
        )]),
    );
    assert!(!bound.moving(&wall(1.0)));
    assert!(!bound.stale(&wall(1.0), &wall(2.0)));
}

fn composed(r: &Receiver, readings: &Readings) -> Vec<f64> {
    let mut scene = Scene::new();
    r.layers()
        .compose_at(&mut scene, Affine::IDENTITY, readings, r.values());
    scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::Fill(f) => Some(f.transform.as_coeffs()[4]),
            _ => None,
        })
        .collect()
}

#[test]
fn an_animation_crosses_once_and_playback_sends_nothing() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let line = Layer::new(square()).animated(vec![playhead()]);
    let first = sender.frame(&[(1, line.clone())]).unwrap();
    receiver.apply(&first).unwrap();
    // Every frame after: the same layer, so nothing.
    for _ in 0..240 {
        assert!(sender.frame(&[(1, line.clone())]).is_none());
    }
    // ...while the host draws it on at each reading.
    assert_eq!(composed(&receiver, &at(0.0, 0.0, true)), [0.0]);
    assert_eq!(composed(&receiver, &at(0.0, 0.5, true)), [60.0]);
    assert_eq!(composed(&receiver, &at(0.0, 0.25, false)), [30.0]);
    // A pan keeps it moving: a transform alone crosses.
    let panned = line.clone().at(Affine::translate((5.0, 0.0)));
    let moved = sender.frame(&[(1, panned)]).unwrap();
    assert!(moved.len() < 64, "{} bytes", moved.len());
    receiver.apply(&moved).unwrap();
    assert_eq!(composed(&receiver, &at(0.0, 0.5, true)), [65.0]);
    // Stopping it: its animations, as none.
    let still = sender
        .frame(&[(
            1,
            Layer::new(Arc::clone(&line.commands)).at(Affine::translate((5.0, 0.0))),
        )])
        .unwrap();
    receiver.apply(&still).unwrap();
    assert_eq!(composed(&receiver, &at(0.0, 0.5, true)), [5.0]);
    assert_eq!(receiver.layers().animation_count(), 0);
}

#[test]
fn a_value_crosses_as_its_name_and_number_when_it_changes() {
    let level = Name::new("view.level").unwrap();
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let bar = Layer::new(square()).animated(vec![Animation::linear(
        Property::X,
        Input::Value(level.clone()),
        Linear::new(100.0, 0.0),
    )]);
    let layers = [(1, bar)];
    receiver
        .apply(&sender.frame_with(&layers, &[(level.clone(), 0.2)]).unwrap())
        .unwrap();
    assert_eq!(composed(&receiver, &Readings::NONE), [20.0]);
    let changed = sender.frame_with(&layers, &[(level.clone(), 0.7)]).unwrap();
    // The magic, the operation, the name and the number.
    assert_eq!(changed.len(), 4 + 1 + 1 + "view.level".len() + 8);
    receiver.apply(&changed).unwrap();
    assert_eq!(composed(&receiver, &Readings::NONE), [70.0]);
    assert!(sender.frame_with(&layers, &[(level, 0.7)]).is_none());
}

/// A batch: a layer at key 1, then `animate`'s bytes as an `ANIMATE` of
/// it.
fn with_animate(animate: &[u8]) -> Vec<u8> {
    let mut batch = Sender::new().frame(&[(1, Layer::new(square()))]).unwrap();
    // An `ANIMATE` is of the second version.
    batch[3] = 2;
    batch.push(8);
    batch.extend_from_slice(&1_u64.to_le_bytes());
    batch.extend_from_slice(animate);
    batch
}

/// One animation of `property` on wall time, as a line of `scale`.
fn line_bytes(property: u8, scale: f64, flags: u8, extra: &[f64]) -> Vec<u8> {
    let mut b = vec![1, property, 0, 0, 0xff, 0];
    b.extend_from_slice(&scale.to_le_bytes());
    b.extend_from_slice(&0.0_f64.to_le_bytes());
    b.push(flags);
    for v in extra {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

#[test]
fn a_well_formed_animate_by_hand_runs() {
    let mut r = Receiver::new();
    r.apply(&with_animate(&line_bytes(0, 2.0, 0, &[]))).unwrap();
    assert_eq!(composed(&r, &wall(3.0)), [6.0]);
}

#[test]
fn a_malformed_animation_fails_the_view() {
    let refused = |bytes: Vec<u8>| Receiver::new().apply(&with_animate(&bytes)).unwrap_err();
    // Numbers that aren't, or out of range.
    assert_eq!(refused(line_bytes(0, f64::NAN, 0, &[])), Error::OutOfRange);
    assert_eq!(refused(line_bytes(0, 1e300, 0, &[])), Error::OutOfRange);
    // An unknown property, clock, input, map or flag.
    assert_eq!(
        refused(line_bytes(9, 1.0, 0, &[])),
        Error::Unknown("property", 9)
    );
    assert_eq!(refused(vec![1, 0, 0, 7, 0xff]), Error::Unknown("clock", 7));
    assert_eq!(refused(vec![1, 0, 3]), Error::Unknown("input", 3));
    assert_eq!(refused(vec![1, 0, 0, 0, 0xff, 4]), Error::Unknown("map", 4));
    assert_eq!(
        refused(line_bytes(0, 1.0, 8, &[])),
        Error::Unknown("linear", 8)
    );
    // A wrap of nothing, a clamp backwards.
    assert!(matches!(
        refused(line_bytes(0, 1.0, 1, &[0.0])),
        Error::BadAnimation(_)
    ));
    assert!(matches!(
        refused(line_bytes(0, 1.0, 6, &[5.0, 1.0])),
        Error::BadAnimation(_)
    ));
    // The same property twice; more than there are.
    let mut twice = line_bytes(0, 1.0, 0, &[]);
    twice[0] = 2;
    twice.extend_from_slice(&line_bytes(0, 1.0, 0, &[])[1..]);
    assert!(matches!(refused(twice), Error::BadAnimation(_)));
    assert!(matches!(refused(vec![7]), Error::BadAnimation(_)));
    // Steps out of order, or too many.
    let mut steps = vec![1, 5, 0, 0, 0xff, 1];
    steps.extend_from_slice(&0.0_f64.to_le_bytes());
    steps.extend_from_slice(&2_u16.to_le_bytes());
    for v in [3.0_f64, 1.0, 2.0, 0.0] {
        steps.extend_from_slice(&v.to_le_bytes());
    }
    assert!(matches!(refused(steps), Error::BadAnimation(_)));
    let mut many = vec![1, 5, 0, 0, 0xff, 1];
    many.extend_from_slice(&0.0_f64.to_le_bytes());
    many.extend_from_slice(&u16::MAX.to_le_bytes());
    assert!(matches!(refused(many), Error::BadAnimation(_)));
    // Cut short.
    let whole = line_bytes(0, 1.0, 0, &[]);
    assert_eq!(refused(whole[..whole.len() - 3].to_vec()), Error::Truncated);
}

#[test]
fn a_value_a_pane_may_not_write_fails_the_view_and_one_it_may_not_read_reads_nothing() {
    // Bound to the engine's value, ungranted: taken, and drawn as sent.
    let engine = Name::new("engine.track.1.peak").unwrap();
    let mut sender = Sender::new();
    let bound = Layer::new(square()).animated(vec![Animation::linear(
        Property::X,
        Input::Value(engine.clone()),
        Linear::new(1.0, 0.0),
    )]);
    let batch = sender.frame(&[(1, bound)]).unwrap();
    let mut ungranted = Receiver::new();
    ungranted.apply(&batch).unwrap();
    assert_eq!(composed(&ungranted, &Readings::NONE), [0.0]);
    // A view writing another space's value.
    let host = Name::new("host.fader.1").unwrap();
    let batch = Sender::new()
        .frame_with(&[], &[(host.clone(), 1.0)])
        .unwrap();
    assert!(matches!(
        Receiver::new().apply(&batch),
        Err(Error::Denied("write", _))
    ));
    // ...which the host may write itself.
    let mut values = Values::new();
    assert!(values.write(Space::Host, &host, 1.0).unwrap());
    assert!(values.write(Space::View, &host, 1.0).is_err());
    assert!(values.write(Space::Engine, &engine, 1.0).unwrap());
}

#[test]
fn a_name_is_its_space_then_a_short_plain_path() {
    for good in ["view.a", "host.fader.1", "engine.track-3.peak_l"] {
        assert!(Name::new(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "view.",
        "meter",
        "views.a",
        "view.A",
        "view.a b",
        "view.é",
        &format!("view.{}", "a".repeat(64)),
    ] {
        assert!(Name::new(bad).is_err(), "{bad:?}");
    }
    // A bad name on the wire.
    let mut batch = Sender::new()
        .frame_with(&[], &[(Name::new("view.ab").unwrap(), 1.0)])
        .unwrap();
    let at = batch.iter().position(|b| *b == b'a').unwrap();
    batch[at] = b'A';
    assert!(matches!(
        Receiver::new().apply(&batch),
        Err(Error::BadName(_))
    ));
}

#[test]
fn a_pane_holds_at_most_so_many_animations_and_values() {
    let mut sender = Sender::new();
    let content = square();
    // Six properties a layer: past the cap over enough layers.
    let all = vec![
        Animation::linear(
            Property::X,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0),
        ),
        Animation::linear(
            Property::Y,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0),
        ),
        Animation::linear(
            Property::ScaleX,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0),
        ),
        Animation::linear(
            Property::ScaleY,
            Input::clock(Clock::Wall),
            Linear::new(1.0, 0.0),
        ),
    ];
    let layers: Vec<_> = (0..u64::try_from(MAX_ANIMATIONS / 4 + 1).unwrap())
        .map(|key| (key, Layer::new(Arc::clone(&content)).animated(all.clone())))
        .collect();
    let batch = sender.frame(&layers).unwrap();
    assert_eq!(
        Receiver::new().apply(&batch),
        Err(Error::TooManyAnimations(MAX_ANIMATIONS))
    );
    // Exactly at the cap is fine.
    let mut sender = Sender::new();
    let batch = sender.frame(&layers[..MAX_ANIMATIONS / 4]).unwrap();
    let mut r = Receiver::new();
    r.apply(&batch).unwrap();
    assert_eq!(r.layers().animation_count(), MAX_ANIMATIONS);
    // Replacing a layer gives its count back.
    let batch = sender
        .frame(&[(0, Layer::new(Arc::clone(&content)))])
        .unwrap();
    r.apply(&batch).unwrap();
    assert_eq!(r.layers().animation_count(), 0);

    let names: Vec<_> = (0..=architect_scene::anim::MAX_VALUES)
        .map(|i| (Name::new(&format!("view.v{i}")).unwrap(), 1.0))
        .collect();
    let batch = Sender::new().frame_with(&[], &names).unwrap();
    assert!(matches!(
        Receiver::new().apply(&batch),
        Err(Error::TooManyValues(_))
    ));
}

#[test]
fn a_batch_without_animations_is_the_bytes_it_was() {
    // The first seven operations are unchanged: a still layer's batch
    // has no `ANIMATE`, and an older host reads it.
    let batch = Sender::new()
        .frame(&[(1, Layer::new(square()).at(Affine::translate((3.0, 4.0))))])
        .unwrap();
    assert_eq!(&batch[..4], b"ASO\x01");
    let mut r = Receiver::new();
    r.apply(&batch).unwrap();
    assert_eq!(composed(&r, &wall(5.0)), [3.0]);
}

#[test]
fn a_step_on_a_running_clock_sleeps_until_it_is_due() {
    use architect_scene::anim::Next;
    let lyric = Animation::new(
        Property::Visible,
        Input::clock(Clock::Playhead),
        Map::Steps(Steps {
            before: 0.0,
            steps: vec![(12.0, 1.0), (15.5, 0.0)],
        }),
    );
    let on = |time, rate| Readings {
        wall: Some(0.0),
        playhead: Some(Reading { time, rate }),
    };
    assert_eq!(lyric.next(&on(10.0, 1.0)), Next::In(2.0));
    // At half speed, twice as long; between its steps, the next one.
    assert_eq!(lyric.next(&on(10.0, 0.5)), Next::In(4.0));
    assert_eq!(lyric.next(&on(13.0, 1.0)), Next::In(2.5));
    // Past the last, paused, or no playhead: never on its own.
    assert_eq!(lyric.next(&on(16.0, 1.0)), Next::Never);
    assert_eq!(lyric.next(&on(10.0, 0.0)), Next::Never);
    assert_eq!(lyric.next(&wall(3.0)), Next::Never);
    // Playing backwards: when it drops below the step it is on.
    assert_eq!(lyric.next(&on(13.0, -1.0)), Next::In(1.0));
    // A line moves every frame; held at its clamp it doesn't, and short
    // of the clamp it moves into, it does once it gets there.
    let line = |l: Linear| Animation::linear(Property::X, Input::clock(Clock::Wall), l);
    assert_eq!(line(Linear::new(10.0, 0.0)).next(&wall(1.0)), Next::Every);
    assert_eq!(
        line(Linear::new(10.0, 0.0).clamp(0.0, 5.0)).next(&wall(1.0)),
        Next::Never
    );
    assert_eq!(
        line(Linear::new(10.0, -30.0).clamp(0.0, 50.0)).next(&wall(1.0)),
        Next::In(2.0)
    );
    assert_eq!(line(Linear::new(0.0, 4.0)).next(&wall(1.0)), Next::Never);
    // A pane: the soonest of its layers.
    let mut layers = Layers::new();
    layers.set(1, Layer::new(square()).animated(vec![lyric]));
    assert_eq!(layers.next(&on(14.0, 1.0)), Next::In(1.5));
    assert!(!layers.moving(&on(14.0, 1.0)));
    layers.set(2, Layer::new(square()).animated(vec![playhead()]));
    assert!(layers.moving(&on(14.0, 1.0)));
}

#[test]
fn a_seek_is_drawn_only_when_it_changes_what_is_drawn() {
    let mut layers = Layers::new();
    layers.set(
        1,
        Layer::new(square()).animated(vec![Animation::new(
            Property::Visible,
            Input::clock(Clock::Playhead),
            Map::Steps(Steps {
                before: 0.0,
                steps: vec![(12.0, 1.0), (15.5, 0.0)],
            }),
        )]),
    );
    let paused = |t| at(0.0, t, false);
    assert!(
        !layers.stale(&paused(13.0), &paused(14.0)),
        "within one step"
    );
    assert!(layers.stale(&paused(13.0), &paused(16.0)), "past it");
    assert!(
        layers.stale(&paused(13.0), &wall(1.0)),
        "nothing on the playhead"
    );
}

#[test]
fn a_batch_is_written_at_the_lowest_version_that_holds_it() {
    use architect_scene::ops::{MAGIC, VERSION};
    let still = Sender::new().frame(&[(1, Layer::new(square()))]).unwrap();
    assert_eq!(&still[..4], &MAGIC);
    let moving = Sender::new()
        .frame(&[(1, Layer::new(square()).animated(vec![playhead()]))])
        .unwrap();
    assert_eq!(&moving[..4], b"ASO\x02");
    assert_eq!(VERSION, 3);
    // A host that reads the first version only matches `MAGIC` exactly:
    // this batch is refused whole, as not one it reads.
    assert_ne!(&moving[..4], &MAGIC);
    // One from a later host than this: refused by its version.
    let mut later = moving.clone();
    later[3] = 4;
    assert_eq!(Receiver::new().apply(&later), Err(Error::Version(4, 3)));
    // A first-version batch can't hold a later operation.
    let mut mislabelled = moving;
    mislabelled[3] = 1;
    assert!(matches!(
        Receiver::new().apply(&mislabelled),
        Err(Error::Unknown(_, 8))
    ));
}

#[test]
fn a_clock_moved_wakes_every_host_that_asked_while_it_lives() {
    use architect_scene::wake;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let waker: wake::Waker = Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    wake::on_clocks_moved(&waker);
    wake::clocks_moved();
    assert!(woken.load(Ordering::Relaxed) >= 1);
    // Gone: never called again.
    let before = woken.load(Ordering::Relaxed);
    drop(waker);
    wake::clocks_moved();
    assert_eq!(woken.load(Ordering::Relaxed), before);
}

#[test]
fn an_alarm_wakes_its_host_at_the_next_step_and_not_every_frame() {
    use architect_scene::anim::Next;
    use architect_scene::wake::Alarm;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let alarm = Alarm::new(Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    }));
    assert!(alarm.drawn(Next::Every), "every frame");
    assert!(!alarm.drawn(Next::Never));
    let began = Instant::now();
    assert!(!alarm.drawn(Next::In(0.03)), "it rests until then");
    // Asked again for the same step: one wake, not two.
    assert!(!alarm.drawn(Next::In(0.029)));
    while woken.load(Ordering::Relaxed) == 0 {
        assert!(began.elapsed() < Duration::from_secs(5), "never woken");
        std::thread::sleep(Duration::from_millis(1));
    }
    // The timer itself, with a waker no clock move reaches: not before.
    let timed = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&timed);
    let waker: architect_scene::wake::Waker = Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let began = Instant::now();
    assert!(architect_scene::wake::after(
        Duration::from_millis(30),
        &waker
    ));
    while timed.load(Ordering::Relaxed) == 0 {
        assert!(began.elapsed() < Duration::from_secs(5), "never woken");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(began.elapsed() >= Duration::from_millis(30));
}
