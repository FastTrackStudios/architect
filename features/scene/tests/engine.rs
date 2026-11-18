//! The engine's values, written lock-free: what a table holds, what a
//! pane draws from it, and that the writer never waits or allocates.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::arithmetic_side_effects
)]

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene as _, Scene};
use architect_scene::Error;
use architect_scene::anim::{Animation, Input, Linear, Name, Property, Readings, Values};
use architect_scene::engine::{EngineTable, MAX_SLOTS, watch};
use architect_scene::layers::{Layer, Layers};
use architect_scene::ops::{Receiver, Sender};
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};

/// Counts the allocations each thread makes: the writer's must be none.
struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator, counting on the way.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: the caller's contract, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: the caller's contract, passed on.
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations() -> u64 {
    ALLOCATIONS.with(Cell::get)
}

fn name(n: &str) -> Name {
    Name::new(n).unwrap()
}

/// A meter 100 high at full scale, its bottom at y 100.
fn meter() -> Layer {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(40, 200, 90),
        None,
        &Rect::new(0.0, -100.0, 10.0, 0.0),
    );
    Layer::new(Arc::new(scene.commands))
        .at(Affine::translate((0.0, 100.0)))
        .animated(vec![Animation::linear(
            Property::ScaleY,
            Input::Value(name("engine.meter")),
            Linear::new(1.0, 0.0).clamp(0.0, 1.0),
        )])
}

fn height(layers: &Layers, values: &Values) -> f64 {
    let mut scene = Scene::new();
    layers.compose_at(&mut scene, Affine::IDENTITY, &Readings::NONE, values);
    match &scene.flattened()[0] {
        RenderCommand::Fill(f) => f.transform.as_coeffs()[3] * 100.0,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_table_holds_the_engines_names_alone_and_a_value_once_written() {
    assert!(matches!(
        EngineTable::new([name("host.fader")]),
        Err(Error::Denied("write", _))
    ));
    let too_many = (0..=MAX_SLOTS).map(|i| name(&format!("engine.v{i}")));
    assert_eq!(
        EngineTable::new(too_many).unwrap_err(),
        Error::TooManyValues(MAX_SLOTS)
    );
    let (table, writer) =
        EngineTable::new([name("engine.a"), name("engine.a"), name("engine.b")]).unwrap();
    assert_eq!(table.names().count(), 2);
    let a = table.slot(&name("engine.a")).unwrap();
    assert_eq!(table.read(&name("engine.a")), None, "unwritten");
    assert_eq!(table.read(&name("engine.c")), None, "not in it");
    writer.write(a, 0.5);
    assert_eq!(table.read(&name("engine.a")), Some(0.5));
    // What a renderer can't take reads as nothing.
    writer.write(a, f64::NAN);
    assert_eq!(table.read(&name("engine.a")), None);
    writer.write(a, 1e300);
    assert_eq!(table.read(&name("engine.a")), None);
    assert_eq!(table.generation(), 3);
}

#[test]
fn a_meter_is_drawn_at_the_level_the_engine_last_wrote() {
    let (table, writer) = EngineTable::new([name("engine.meter")]).unwrap();
    let slot = table.slot(&name("engine.meter")).unwrap();
    let mut layers = Layers::new();
    layers.set(1, meter());
    let values = Values::new().with_engine(Arc::clone(&table));
    // Unwritten: as sent.
    assert_eq!(height(&layers, &values), 100.0);
    let before = values.revision();
    writer.write(slot, 0.25);
    assert_eq!(height(&layers, &values), 25.0);
    assert_ne!(values.revision(), before, "a host sees it moved");
    writer.write(slot, 3.0);
    assert_eq!(height(&layers, &values), 100.0, "clamped");
}

#[test]
fn only_a_pane_granted_the_engine_reads_it() {
    let (table, writer) = EngineTable::new([name("engine.meter")]).unwrap();
    writer.write(table.slot(&name("engine.meter")).unwrap(), 0.5);
    let batch = Sender::new().frame(&[(1, meter())]).unwrap();
    // Ungranted: its meter stands as sent, and the view still draws.
    let mut ungranted = Receiver::new();
    ungranted.apply(&batch).unwrap();
    assert_eq!(height(ungranted.layers(), ungranted.values()), 100.0);
    let mut granted = Receiver::new();
    granted.grant(Some(Arc::clone(&table)));
    granted.apply(&batch).unwrap();
    assert_eq!(height(granted.layers(), granted.values()), 50.0);
}

#[test]
fn the_writer_never_waits_or_allocates_while_a_host_composes() {
    let (table, writer) = EngineTable::new([name("engine.meter")]).unwrap();
    let slot = table.slot(&name("engine.meter")).unwrap();
    let mut layers = Layers::new();
    layers.set(1, meter());
    let values = Values::new().with_engine(Arc::clone(&table));
    let done = Arc::new(AtomicBool::new(false));
    // An audio callback: 128 frames at 48 kHz, every 2.67 ms, for a
    // second, timing each write.
    let engine = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let period = Duration::from_micros(2_667);
            let mut took = Vec::with_capacity(400);
            let before = allocations();
            for i in 0..375_u32 {
                let level = (f64::from(i) * 0.05).sin().abs();
                let t = Instant::now();
                writer.write(slot, level);
                took.push(t.elapsed());
                std::thread::sleep(period);
            }
            // Its pushes were made before it started: none since.
            let made = allocations() - before;
            done.store(true, Ordering::Release);
            (took, made)
        })
    };
    // The host: composing as fast as it can, the meter at what it reads.
    let mut frames = 0_u64;
    let mut heights = std::collections::BTreeSet::new();
    while !done.load(Ordering::Acquire) {
        heights.insert((height(&layers, &values) * 1000.0) as i64);
        frames += 1;
    }
    let (mut took, made) = engine.join().unwrap();
    assert_eq!(made, 0, "the writer allocated {made} times");
    took.sort();
    let (p50, p99, max) = (
        took[took.len() / 2],
        took[took.len() * 99 / 100],
        took[took.len() - 1],
    );
    println!(
        "375 writes at audio-callback rate beside {frames} composed frames: \
         p50 {p50:?}, p99 {p99:?}, max {max:?}; {} distinct meter heights drawn",
        heights.len()
    );
    // Two atomic stores: well under a microsecond, but a test machine's
    // scheduler may preempt one; a lock held by the host would show as
    // waits of the host's frame time.
    assert!(p99 < Duration::from_micros(50), "p99 {p99:?}");
    assert!(max < Duration::from_millis(2), "max {max:?}");
    assert!(heights.len() > 50, "the host drew the meter moving");
}

#[test]
fn a_resting_host_is_woken_when_the_engine_writes() {
    let (table, writer) = EngineTable::new([name("engine.meter")]).unwrap();
    let slot = table.slot(&name("engine.meter")).unwrap();
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    watch(&table, &waker);
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(
        woken.load(Ordering::Relaxed),
        0,
        "nothing written: it rests"
    );
    writer.write(slot, 0.5);
    let began = Instant::now();
    while woken.load(Ordering::Relaxed) == 0 {
        assert!(began.elapsed() < Duration::from_secs(5), "never woken");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(began.elapsed() < Duration::from_millis(100));
}
