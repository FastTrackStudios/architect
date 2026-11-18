//! Hit regions and drags, handled by the host: what a pointer finds,
//! what the view hears, what a drag writes, and what crosses.

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
use architect_scene::anim::{Animation, Clock, Input, Linear, Name, Property, Readings, Values};
use architect_scene::layers::{Layer, Layers};
use architect_scene::ops::{Receiver, Sender};
use architect_scene::region::{
    Axis, Cursor, Drag, Hit, Interaction, Lit, MAX_REGIONS, Mods, Region, RegionEvent, Shape,
};
use peniko::kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};

fn content() -> Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(1, 2, 3),
        None,
        &Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    Arc::new(scene.commands)
}

const fn button(id: u64) -> Region {
    Region::new(id, Shape::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)))
        .cursor(Cursor::Pointer)
        .hover(Color::from_rgba8(255, 255, 255, 40))
        .press(Color::from_rgba8(0, 0, 0, 80))
}

fn level() -> Name {
    Name::new("host.fader").unwrap()
}

/// A fader: a 100-unit track whose value runs 0 (bottom) to 1 (top),
/// stepped by hundredths.
fn fader() -> Region {
    Region::new(7, Shape::Rect(Rect::new(0.0, 0.0, 20.0, 100.0)))
        .cursor(Cursor::NsResize)
        .drag(Drag {
            value: level(),
            axis: Axis::Y,
            scale: -0.01,
            min: 0.0,
            max: 1.0,
            step: Some(0.01),
            initial: 0.5,
            coast: None,
        })
}

const NONE: Readings = Readings::NONE;

#[test]
fn a_pointer_finds_the_topmost_region_where_its_layer_is_drawn() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![button(10)]));
    // Above it, moved right 5 and scaled 2: 5..25 in the pane.
    layers.set(
        2,
        Layer::new(content())
            .at(Affine::translate((5.0, 0.0)) * Affine::scale(2.0))
            .z(1)
            .regions(vec![button(20)]),
    );
    let v = Values::new();
    let id = |layers: &Layers, x, y| layers.hit(Point::new(x, y), &NONE, &v).map(|(h, _)| h.id);
    assert_eq!(id(&layers, 2.0, 2.0), Some(10));
    assert_eq!(id(&layers, 6.0, 2.0), Some(20), "the higher layer wins");
    assert_eq!(id(&layers, 24.0, 19.0), Some(20), "at its scale");
    assert_eq!(id(&layers, 30.0, 2.0), None);
    // Clipped away, or not drawn: not found.
    layers.update(2, |l| l.clip = Some(Rect::new(0.0, 0.0, 8.0, 8.0)));
    assert_eq!(id(&layers, 24.0, 19.0), None);
    assert_eq!(id(&layers, 6.0, 2.0), Some(20));
    layers.update(2, |l| l.visible = false);
    assert_eq!(id(&layers, 6.0, 2.0), Some(10));
}

#[test]
fn a_region_is_found_where_its_animation_has_moved_it() {
    let mut layers = Layers::new();
    layers.set(
        1,
        Layer::new(content())
            .regions(vec![button(1)])
            .animated(vec![Animation::linear(
                Property::X,
                Input::clock(Clock::Wall),
                Linear::new(100.0, 0.0),
            )]),
    );
    let v = Values::new();
    let at = |t: f64| Readings {
        wall: Some(t),
        playhead: None,
    };
    assert!(layers.hit(Point::new(5.0, 5.0), &at(0.0), &v).is_some());
    assert!(layers.hit(Point::new(5.0, 5.0), &at(1.0), &v).is_none());
    assert!(layers.hit(Point::new(105.0, 5.0), &at(1.0), &v).is_some());
}

#[test]
fn hover_and_press_are_the_hosts_and_the_view_hears_them_by_id() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![button(10)]));
    let mut values = Values::new();
    let mut i = Interaction::new();
    // In: a highlight, the region's cursor, and not consumed (the view
    // may still follow the pointer).
    let o = i.moved(&layers, &NONE, &mut values, Point::new(5.0, 5.0));
    assert!(o.redraw && !o.consumed);
    assert_eq!(o.cursor, Some(Cursor::Pointer));
    // Moving within it: nothing new.
    let o = i.moved(&layers, &NONE, &mut values, Point::new(6.0, 6.0));
    assert!(!o.redraw && o.cursor.is_none());
    // A press: the host's, highlighted as pressed.
    let o = i.pressed(&layers, &NONE, &mut values, Point::new(6.0, 6.0));
    assert!(o.consumed);
    assert_eq!(
        i.lit(),
        Some(Lit {
            hit: Hit { key: 1, id: 10 },
            pressed: true
        })
    );
    // Held, a move outside is still the host's, and leaves nothing.
    let o = i.moved(&layers, &NONE, &mut values, Point::new(50.0, 6.0));
    assert!(o.consumed && !o.redraw);
    // Released outside: release, then leave, and the pane's cursor.
    let o = i.released(&layers, &NONE, &mut values, Point::new(50.0, 6.0));
    assert!(o.consumed);
    assert_eq!(o.cursor, Some(Cursor::Default));
    assert_eq!(
        i.take(),
        [
            RegionEvent::Enter(10),
            RegionEvent::Press(10, Mods::NONE),
            RegionEvent::Release(10, Mods::NONE),
            RegionEvent::Leave(10),
        ]
    );
    // A press off every region is the view's.
    assert!(
        !i.pressed(&layers, &NONE, &mut values, Point::new(50.0, 6.0))
            .consumed
    );
}

#[test]
fn a_hovered_region_is_drawn_lit() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![button(10)]));
    let draw = |lit| {
        let mut scene = Scene::new();
        layers.compose_lit(&mut scene, Affine::IDENTITY, &NONE, &Values::new(), lit);
        scene.commands.len()
    };
    let hit = Hit { key: 1, id: 10 };
    assert_eq!(draw(None), 1);
    assert_eq!(
        draw(Some(Lit {
            hit,
            pressed: false
        })),
        2
    );
    assert_eq!(draw(Some(Lit { hit, pressed: true })), 2);
}

#[test]
fn a_drag_writes_the_hosts_value_and_a_layer_bound_to_it_follows() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![fader()]));
    // The knob: 100 units down at 0, at the top at 1.
    layers.set(
        2,
        Layer::new(content()).z(1).animated(vec![Animation::linear(
            Property::Y,
            Input::Value(level()),
            Linear::new(-100.0, 100.0),
        )]),
    );
    let mut values = Values::new();
    architect_scene::region::seed(None, &[fader()], &mut values).unwrap();
    assert_eq!(values.get(&level()), Some(0.5));
    let knob_y = |values: &Values| {
        let mut scene = Scene::new();
        layers.compose_at(&mut scene, Affine::IDENTITY, &NONE, values);
        match &scene.commands[1] {
            RenderCommand::Fill(f) => f.transform.as_coeffs()[5],
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(knob_y(&values), 50.0);
    let mut i = Interaction::new();
    assert!(
        i.pressed(&layers, &NONE, &mut values, Point::new(10.0, 50.0))
            .consumed
    );
    // Up 20: up a fifth.
    let o = i.moved(&layers, &NONE, &mut values, Point::new(10.0, 30.0));
    assert!(o.consumed && o.redraw);
    assert!((values.get(&level()).unwrap() - 0.7).abs() < 1e-9);
    assert!((knob_y(&values) - 30.0).abs() < 1e-9);
    // Past the top: clamped. Past the region, still the drag's.
    let _ = i.moved(&layers, &NONE, &mut values, Point::new(300.0, -500.0));
    assert_eq!(values.get(&level()), Some(1.0));
    // A step's worth less than a step: rounded to the nearest.
    let _ = i.moved(&layers, &NONE, &mut values, Point::new(10.0, 33.4));
    assert!((values.get(&level()).unwrap() - 0.67).abs() < 1e-9);
    let _ = i.released(&layers, &NONE, &mut values, Point::new(10.0, 33.4));
    // The view hears the values coalesced to the last of the run, then
    // the release.
    let heard = i.take();
    assert_eq!(heard.first(), Some(&RegionEvent::Enter(7)));
    assert_eq!(heard[1], RegionEvent::Press(7, Mods::NONE));
    match heard[2] {
        RegionEvent::Value(7, Axis::Y, v) => assert!((v - 0.67).abs() < 1e-9),
        ref other => panic!("{other:?}"),
    }
    assert_eq!(heard[3], RegionEvent::Release(7, Mods::NONE));
}

#[test]
fn a_frames_values_are_heard_once() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![fader()]));
    let mut values = Values::new();
    let mut i = Interaction::new();
    let _ = i.pressed(&layers, &NONE, &mut values, Point::new(10.0, 50.0));
    let _ = i.take();
    for y in [49.0, 48.0, 47.0, 46.0] {
        let _ = i.moved(&layers, &NONE, &mut values, Point::new(10.0, y));
    }
    let heard = i.take();
    assert_eq!(heard.len(), 1, "{heard:?}");
    // Nothing moved since: nothing to hear.
    assert!(i.take().is_empty());
}

#[test]
fn the_view_moves_a_fader_by_its_initial_value_and_not_otherwise() {
    use architect_scene::region::seed;
    let mut values = Values::new();
    seed(None, &[fader()], &mut values).unwrap();
    // Dragged to 0.8 by the pointer, then the view sends the same region:
    // the pointer's value stands.
    let _ = values.write(architect_scene::anim::Space::Host, &level(), 0.8);
    seed(Some(&[fader()]), &[fader()], &mut values).unwrap();
    assert_eq!(values.get(&level()), Some(0.8));
    // Automation: another initial value moves it.
    let mut automated = fader();
    automated.drags[0].initial = 0.25;
    seed(Some(&[fader()]), &[automated], &mut values).unwrap();
    assert_eq!(values.get(&level()), Some(0.25));
}

#[test]
fn regions_cross_once_and_seed_the_hosts_value() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let layer = Layer::new(content()).regions(vec![fader(), button(3)]);
    let first = sender.frame(&[(1, layer.clone())]).unwrap();
    assert_eq!(&first[..4], b"ASO\x02");
    receiver.apply(&first).unwrap();
    assert_eq!(receiver.layers().region_count(), 2);
    assert_eq!(receiver.values().get(&level()), Some(0.5));
    assert!(sender.frame(&[(1, layer.clone())]).is_none());
    // Moved: its transform alone; its regions stay.
    let moved = sender
        .frame(&[(1, layer.at(Affine::translate((4.0, 0.0))))])
        .unwrap();
    assert!(moved.len() < 64);
    receiver.apply(&moved).unwrap();
    assert_eq!(receiver.layers().region_count(), 2);
    let hit = receiver
        .layers()
        .hit(Point::new(5.0, 5.0), &NONE, receiver.values());
    // Both are under it there; the later region is on top.
    assert_eq!(hit.map(|(h, _)| h.id), Some(3));
}

#[test]
fn a_malformed_region_fails_the_view() {
    let refused = |r: Region| {
        let batch = Sender::new()
            .frame(&[(1, Layer::new(content()).regions(vec![r]))])
            .unwrap();
        Receiver::new().apply(&batch).unwrap_err()
    };
    let with_drag = |change: &dyn Fn(&mut Drag)| {
        let mut r = fader();
        change(&mut r.drags[0]);
        r
    };
    // A drag that writes the view's value, not the host's.
    assert!(matches!(
        refused(with_drag(&|d| d.value = Name::new("view.level").unwrap())),
        Error::Denied("drag", _)
    ));
    assert!(matches!(
        refused(with_drag(&|d| d.step = Some(0.0))),
        Error::BadRegion(_)
    ));
    assert!(matches!(
        refused(with_drag(&|d| d.min = 2.0)),
        Error::BadRegion(_)
    ));
    assert!(matches!(
        refused(with_drag(&|d| d.initial = 3.0)),
        Error::BadRegion(_)
    ));
    assert_eq!(
        refused(with_drag(&|d| d.scale = f64::NAN)),
        Error::OutOfRange
    );
    assert!(matches!(
        refused(Region::new(1, Shape::Rect(Rect::new(5.0, 0.0, 0.0, 1.0)))),
        Error::BadRegion(_)
    ));
    assert!(matches!(
        refused(Region::new(1, Shape::Circle(Point::new(0.0, 0.0), -1.0))),
        Error::BadRegion(_)
    ));
    assert_eq!(
        refused(Region::new(1, Shape::Rect(Rect::new(0.0, 0.0, 1e300, 1.0)))),
        Error::OutOfRange
    );
    // An unknown cursor, by hand.
    let mut batch = Sender::new()
        .frame(&[(1, Layer::new(content()).regions(vec![button(9)]))])
        .unwrap();
    let cursor_at = batch.len() - 1 - 1 - 4 - 4;
    assert_eq!(batch[cursor_at], 1, "the pointer cursor");
    batch[cursor_at] = 99;
    assert_eq!(
        Receiver::new().apply(&batch),
        Err(Error::Unknown("cursor", 99))
    );
}

#[test]
fn a_pane_holds_at_most_so_many_regions() {
    let many: Vec<Region> = (0..1000).map(button).collect();
    let layers: Vec<_> = (0..u64::try_from(MAX_REGIONS / 1000 + 1).unwrap())
        .map(|key| (key, Layer::new(content()).regions(many.clone())))
        .collect();
    let batch = Sender::new().frame(&layers).unwrap();
    assert_eq!(
        Receiver::new().apply(&batch),
        Err(Error::TooManyRegions(MAX_REGIONS))
    );
}

#[test]
fn a_box_drags_on_both_axes_and_each_value_is_heard_once_a_frame() {
    let x = Name::new("host.box.x").unwrap();
    let y = Name::new("host.box.y").unwrap();
    let along = |value: &Name, axis| Drag {
        value: value.clone(),
        axis,
        scale: 1.0,
        min: 0.0,
        max: 300.0,
        step: None,
        initial: 0.0,
        coast: None,
    };
    let handle = Region::new(5, Shape::Rect(Rect::new(0.0, 0.0, 20.0, 20.0)))
        .cursor(Cursor::Move)
        .drag(along(&x, Axis::X))
        .drag(along(&y, Axis::Y));
    // The box moves with both values: the region rides on it.
    let mut layers = Layers::new();
    layers.set(
        1,
        Layer::new(content())
            .regions(vec![handle.clone()])
            .animated(vec![
                Animation::linear(Property::X, Input::Value(x.clone()), Linear::new(1.0, 0.0)),
                Animation::linear(Property::Y, Input::Value(y.clone()), Linear::new(1.0, 0.0)),
            ]),
    );
    let mut values = Values::new();
    architect_scene::region::seed(None, std::slice::from_ref(&handle), &mut values).unwrap();
    let mut i = Interaction::new();
    assert!(
        i.pressed(&layers, &NONE, &mut values, Point::new(10.0, 10.0))
            .consumed
    );
    let _ = i.take();
    // It follows the pointer, though it moves under it as it does.
    for step in 1..=10 {
        let d = f64::from(step) * 7.0;
        let _ = i.moved(
            &layers,
            &NONE,
            &mut values,
            Point::new(10.0 + d, 10.0 + d / 2.0),
        );
        assert_eq!(values.get(&x), Some(d));
        assert_eq!(values.get(&y), Some(d / 2.0));
        assert!(
            layers
                .hit(Point::new(10.0 + d, 10.0 + d / 2.0), &NONE, &values)
                .is_some()
        );
    }
    assert_eq!(
        i.take(),
        [
            RegionEvent::Value(5, Axis::X, 70.0),
            RegionEvent::Value(5, Axis::Y, 35.0)
        ]
    );
    // Two drags along one axis: refused.
    let mut twice = handle;
    twice.drags[1].axis = Axis::X;
    assert!(architect_scene::region::check(&[twice]).is_err());
}

#[test]
fn a_flicked_drag_coasts_on_the_hosts_wall_clock_and_stops() {
    let scroll = Name::new("host.list.scroll").unwrap();
    let list = Region::new(4, Shape::Rect(Rect::new(0.0, 0.0, 100.0, 400.0))).drag(Drag {
        value: scroll.clone(),
        axis: Axis::Y,
        scale: -1.0,
        min: 0.0,
        max: 2000.0,
        step: None,
        initial: 0.0,
        coast: Some(0.3),
    });
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![list.clone()]));
    let mut values = Values::new();
    architect_scene::region::seed(None, &[list], &mut values).unwrap();
    let at = |t: f64| Readings {
        wall: Some(t),
        playhead: None,
    };
    let mut i = Interaction::new();
    // A flick up: 10 px every 10 ms, 1000 px a second, let go moving.
    let _ = i.pressed(&layers, &at(0.0), &mut values, Point::new(50.0, 300.0));
    for n in 1..=5_u8 {
        let t = f64::from(n) * 0.01;
        let _ = i.moved(
            &layers,
            &at(t),
            &mut values,
            Point::new(50.0, f64::from(n).mul_add(-10.0, 300.0)),
        );
    }
    assert_eq!(values.get(&scroll), Some(50.0));
    let _ = i.released(&layers, &at(0.05), &mut values, Point::new(50.0, 250.0));
    assert!(i.coasting());
    let _ = i.take();
    // It goes on without the pointer, slowing: each frame less than the last.
    let mut was = 50.0;
    let mut step = f64::INFINITY;
    for frame in 1..=60_u8 {
        let t = 0.05 + f64::from(frame) / 240.0;
        assert!(i.coast(&at(t), &mut values), "frame {frame}");
        let now = values.get(&scroll).unwrap();
        assert!(
            now > was && now - was < step,
            "frame {frame}: {was} to {now}"
        );
        step = now - was;
        was = now;
        // The view hears it go, a value a frame.
        assert_eq!(i.take().len(), 1);
    }
    // ...towards 50 + 1000 × 0.3 = 350, where it stops.
    for frame in 61..=2000_u16 {
        let _ = i.coast(&at(0.05 + f64::from(frame) / 240.0), &mut values);
        if !i.coasting() {
            break;
        }
    }
    assert!(!i.coasting());
    let end = values.get(&scroll).unwrap();
    assert!((end - 350.0).abs() < 1.0, "it stopped at {end}");
}

#[test]
fn a_coast_stops_at_its_end_and_a_press_stops_it() {
    let scroll = Name::new("host.list.scroll").unwrap();
    let list = Region::new(4, Shape::Rect(Rect::new(0.0, 0.0, 100.0, 400.0))).drag(Drag {
        value: scroll.clone(),
        axis: Axis::Y,
        scale: -1.0,
        min: 0.0,
        max: 100.0,
        step: None,
        initial: 0.0,
        coast: Some(1.0),
    });
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![list]));
    let mut values = Values::new();
    let at = |t: f64| Readings {
        wall: Some(t),
        playhead: None,
    };
    let flick = |i: &mut Interaction, values: &mut Values, t0: f64| {
        let _ = i.pressed(&layers, &at(t0), values, Point::new(50.0, 300.0));
        let _ = i.moved(&layers, &at(t0 + 0.01), values, Point::new(50.0, 290.0));
        let _ = i.moved(&layers, &at(t0 + 0.02), values, Point::new(50.0, 280.0));
        let _ = i.released(&layers, &at(t0 + 0.02), values, Point::new(50.0, 280.0));
    };
    let mut i = Interaction::new();
    flick(&mut i, &mut values, 0.0);
    // 1000 a second for a second's constant: well past 100. It stops there.
    let _ = i.coast(&at(0.5), &mut values);
    assert_eq!(values.get(&scroll), Some(100.0));
    assert!(!i.coasting());
    // A flick, then a press: the coast stops where it was.
    let _ = values.write(architect_scene::anim::Space::Host, &scroll, 0.0);
    flick(&mut i, &mut values, 10.0);
    let _ = i.coast(&at(10.03), &mut values);
    let _ = i.pressed(&layers, &at(10.04), &mut values, Point::new(50.0, 200.0));
    assert!(!i.coasting());
    // Held still before the release: no flick.
    let _ = i.released(&layers, &at(10.5), &mut values, Point::new(50.0, 200.0));
    assert!(!i.coasting());
}

fn ink(color: Color) -> Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &Rect::new(2.0, 2.0, 8.0, 8.0),
    );
    Arc::new(scene.commands)
}

fn fills_of(commands: &[RenderCommand], color: Color) -> usize {
    commands
        .iter()
        .filter(|c| {
            matches!(c, RenderCommand::Fill(f)
                if matches!(&f.brush, anyrender::Paint::Solid(c) if *c == color))
        })
        .count()
}

/// A region whose hover is its content redrawn (#844): a layer sent once,
/// hidden, that the host draws in the region's layer's space while the
/// region is hovered, and the press's while pressed. No box over it, and
/// nothing from the view; across a process it is the same layer, and the
/// batch that says so is of version 3.
#[test]
fn a_hovered_region_shows_its_hover_layer_in_its_own_space() {
    const HOVERED: u64 = 50;
    const PRESSED: u64 = 51;
    let blue = Color::from_rgb8(0, 0, 255);
    let red = Color::from_rgb8(255, 0, 0);
    let note = Region::new(9, Shape::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)))
        .hover_layer(HOVERED)
        .press_layer(PRESSED);
    let page = Layer::new(content())
        .at(Affine::translate((100.0, 0.0)))
        .regions(vec![note]);
    let hidden = |c| {
        let mut l = Layer::new(ink(c));
        l.visible = false;
        l
    };
    let all = vec![(1, page), (HOVERED, hidden(blue)), (PRESSED, hidden(red))];
    // Across a process: the same regions, at version 3.
    let batch = Sender::new().frame(&all).unwrap();
    assert_eq!(&batch[..4], b"ASO\x03");
    let mut receiver = Receiver::new();
    receiver.apply(&batch).unwrap();
    let layers = receiver.layers();
    assert_eq!(
        layers.region(Hit { key: 1, id: 9 }).unwrap().hover_layer,
        Some(HOVERED)
    );
    let v = Values::new();
    let drawn = |lit: Option<Lit>| {
        let mut scene = Scene::new();
        layers.compose_lit(&mut scene, Affine::IDENTITY, &NONE, &v, lit);
        scene.commands
    };
    // Not lit: neither is drawn (they are hidden).
    assert_eq!(
        fills_of(&drawn(None), blue) + fills_of(&drawn(None), red),
        0
    );
    let hit = Hit { key: 1, id: 9 };
    let hovered = drawn(Some(Lit {
        hit,
        pressed: false,
    }));
    assert_eq!((fills_of(&hovered, blue), fills_of(&hovered, red)), (1, 0));
    // In the page's space: moved 100 right with it.
    let at = hovered
        .iter()
        .find_map(|c| match c {
            RenderCommand::Fill(f) if matches!(&f.brush, anyrender::Paint::Solid(c) if *c == blue) => {
                Some(f.transform.as_coeffs()[4])
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(at, 100.0);
    let pressed = drawn(Some(Lit { hit, pressed: true }));
    assert_eq!((fills_of(&pressed, blue), fills_of(&pressed, red)), (0, 1));
    // A batch with no layer swaps stays at version 2.
    let plain = Sender::new()
        .frame(&[(1, Layer::new(content()).regions(vec![button(1)]))])
        .unwrap();
    assert_eq!(&plain[..4], b"ASO\x02");
}

/// A region pressed with Shift or ⌘ held (#844): the view hears the press
/// and the release with them, by the names its input uses.
#[test]
fn a_press_carries_the_modifiers_held() {
    let mut layers = Layers::new();
    layers.set(1, Layer::new(content()).regions(vec![button(10)]));
    let mut v = Values::new();
    let mut i = Interaction::new();
    let at = Point::new(5.0, 5.0);
    let range = Mods::from_names(&["Shift"]);
    let _ = i.pressed_with(&layers, &NONE, &mut v, at, range);
    let _ = i.released_with(&layers, &NONE, &mut v, at, range);
    let heard = i.take();
    assert!(heard.contains(&RegionEvent::Press(10, range)), "{heard:?}");
    assert!(heard.contains(&RegionEvent::Release(10, range)));
    assert!(range.has("Shift") && !range.has("Mod"));
    assert_eq!(
        Mods::from_names(&["Mod", "Shift"]).names(),
        ["Mod", "Shift"]
    );
    let command = if cfg!(target_os = "macos") {
        Mods::held(false, false, false, true)
    } else {
        Mods::held(false, false, true, false)
    };
    assert!(command.has("Mod"));
}
