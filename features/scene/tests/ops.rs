//! A view's sender and its host's receiver: what crosses, and what the
//! host's layers are after.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use std::sync::Arc;

use anyrender::recording::RenderCommand;
use anyrender::{Glyph, PaintScene as _, Scene};
use architect_scene::Error;
use architect_scene::layers::Layer;
use architect_scene::ops::{Receiver, Sender};
use peniko::kurbo::{Affine, Rect};
use peniko::{Blob, Color, Fill, FontData, Style};

fn squares(n: u32) -> Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    for i in 0..n {
        let x = f64::from(i) * 12.0;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(40, 90, 160),
            None,
            &Rect::new(x, 0.0, x + 10.0, 10.0),
        );
    }
    Arc::new(scene.commands)
}

fn text(font: &FontData) -> Arc<Vec<RenderCommand>> {
    let mut scene = Scene::new();
    scene.draw_glyphs(
        font,
        12.0,
        false,
        &[],
        peniko::kurbo::Vec2::ZERO,
        &Style::Fill(Fill::NonZero),
        Color::BLACK,
        1.0,
        Affine::IDENTITY,
        None,
        std::iter::once(Glyph {
            id: 3,
            x: 0.0,
            y: 0.0,
        }),
    );
    Arc::new(scene.commands)
}

#[test]
fn the_first_frame_crosses_and_an_unchanged_one_does_not() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let content = squares(100);
    let layers = [(1, Layer::new(Arc::clone(&content)))];
    let first = sender.frame(&layers).unwrap();
    receiver.apply(&first).unwrap();
    assert_eq!(receiver.layers().len(), 1);
    assert_eq!(*receiver.layers().get(1).unwrap().commands, *content);
    assert!(sender.frame(&layers).is_none());
}

#[test]
fn a_pan_crosses_as_a_transform_alone() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let content = squares(2000);
    let first = sender
        .frame(&[(1, Layer::new(Arc::clone(&content)))])
        .unwrap();
    receiver.apply(&first).unwrap();
    let panned = Layer::new(Arc::clone(&content)).at(Affine::translate((-64.0, 0.0)));
    let moved = sender.frame(&[(1, panned)]).unwrap();
    assert!(moved.len() < 80, "a move is {} bytes", moved.len());
    assert!(first.len() > 100 * moved.len());
    receiver.apply(&moved).unwrap();
    assert_eq!(
        receiver.layers().get(1).unwrap().transform,
        Affine::translate((-64.0, 0.0))
    );
}

#[test]
fn the_same_content_recorded_again_is_not_sent_again() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let first = sender.frame(&[(1, Layer::new(squares(50)))]).unwrap();
    receiver.apply(&first).unwrap();
    // An immediate-mode view records the same thing into a new list.
    assert!(sender.frame(&[(1, Layer::new(squares(50)))]).is_none());
}

#[test]
fn one_layer_changing_sends_only_that_layer() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let base = squares(2000);
    let first = sender
        .frame(&[
            (1, Layer::new(Arc::clone(&base))),
            (2, Layer::new(squares(1)).z(1)),
        ])
        .unwrap();
    receiver.apply(&first).unwrap();
    let second = sender
        .frame(&[
            (1, Layer::new(Arc::clone(&base))),
            (2, Layer::new(squares(2)).z(1)),
        ])
        .unwrap();
    assert!(second.len() < first.len() / 50);
    receiver.apply(&second).unwrap();
    assert_eq!(receiver.layers().get(2).unwrap().commands.len(), 2);
    assert_eq!(receiver.layers().get(1).unwrap().commands.len(), 2000);
}

#[test]
fn a_layer_no_longer_shown_is_removed_and_its_fragment_let_go() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let first = sender
        .frame(&[(1, Layer::new(squares(10))), (2, Layer::new(squares(500)))])
        .unwrap();
    receiver.apply(&first).unwrap();
    let held = receiver.bytes();
    let second = sender.frame(&[(1, Layer::new(squares(10)))]).unwrap();
    receiver.apply(&second).unwrap();
    assert_eq!(receiver.layers().len(), 1);
    assert!(receiver.bytes() < held);
}

#[test]
fn a_font_crosses_once_however_many_layers_draw_with_it() {
    let (mut sender, mut receiver) = (Sender::new(), Receiver::new());
    let font = FontData::new(Blob::from(vec![7_u8; 50_000]), 0);
    let first = sender.frame(&[(1, Layer::new(text(&font)))]).unwrap();
    assert!(first.len() > 50_000);
    receiver.apply(&first).unwrap();
    let second = sender
        .frame(&[
            (1, Layer::new(text(&font))),
            (
                2,
                Layer::new(text(&font)).at(Affine::translate((0.0, 20.0))),
            ),
        ])
        .unwrap();
    assert!(second.len() < 200, "the font was sent again");
    receiver.apply(&second).unwrap();
    assert!(matches!(
        receiver.layers().get(2).unwrap().commands[0],
        RenderCommand::GlyphRun(_)
    ));
}

#[test]
fn a_batch_that_is_not_one_and_a_layer_of_no_fragment_fail() {
    let mut receiver = Receiver::new();
    assert_eq!(receiver.apply(b"nope"), Err(Error::NotAFragment));
    let mut sender = Sender::new();
    let batch = sender.frame(&[(1, Layer::new(squares(3)))]).unwrap();
    // Cut inside the fragment: what is left ends early.
    assert_eq!(
        receiver.apply(&batch[..batch.len() - 70]),
        Err(Error::Truncated)
    );
    // A host that never saw the first batch is told of a layer it can't
    // draw: the view fails, and a view opened again starts both afresh.
    let moved = sender
        .frame(&[(1, Layer::new(squares(3)).at(Affine::translate((1.0, 0.0))))])
        .unwrap();
    let mut fresh = Receiver::new();
    fresh.apply(&moved).unwrap();
    assert!(fresh.layers().is_empty());
}

/// A large resource in a batch given whole is drawn from the batch's own
/// bytes (#835): the font's bytes are where the batch's are, not a copy.
#[test]
fn a_large_font_in_a_batch_given_whole_is_not_copied() {
    use architect_scene::ops::{Receiver, SHARE_AT};
    let font = FontData::new(Blob::from(vec![7_u8; SHARE_AT + 10]), 0);
    let mut scene = Scene::new();
    scene.draw_glyphs(
        &font,
        12.0,
        false,
        &[],
        peniko::kurbo::Vec2::ZERO,
        &Style::Fill(Fill::NonZero),
        Color::BLACK,
        1.0,
        Affine::IDENTITY,
        None,
        std::iter::once(Glyph {
            id: 3,
            x: 0.0,
            y: 0.0,
        }),
    );
    let mut sender = architect_scene::ops::Sender::new();
    let batch = sender
        .frame(&[(
            1,
            architect_scene::layers::Layer::new(std::sync::Arc::new(scene.commands)),
        )])
        .unwrap();
    let within = batch.as_ptr_range();
    let id = architect_scene::resource::Resource::Font(font.data.clone()).id();
    let mut receiver = Receiver::new();
    receiver.apply_owned(batch).unwrap();
    let kept = receiver.resources().font(id).unwrap();
    assert!(
        within.contains(&kept.data().as_ptr()),
        "the font was copied out of its batch"
    );
}

/// A scroll of many layers (Keyflow's pages and systems, 1,851 of them):
/// each crosses as its transform alone, 57 bytes, and nothing else, and the
/// sender spends a pointer comparison on each one's content.
#[test]
fn a_frame_that_moves_every_layer_writes_a_move_each_and_nothing_else() {
    let content: Vec<_> = (0..2000).map(|_| squares(2)).collect();
    let at = |dy: f64| -> Vec<(u64, Layer)> {
        content
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let y = f64::from(u32::try_from(i).unwrap()).mul_add(20.0, dy);
                (
                    u64::try_from(i).unwrap(),
                    Layer::new(Arc::clone(c)).at(Affine::translate((0.0, y))),
                )
            })
            .collect()
    };
    let mut sender = Sender::new();
    let mut receiver = Receiver::new();
    receiver.apply(&sender.frame(&at(0.0)).unwrap()).unwrap();
    let moved = sender.frame(&at(-7.0)).unwrap();
    // The header, then a MOVE (1), a key (8) and a transform (48) a layer.
    assert_eq!(moved.len(), 4 + 2000 * 57);
    receiver.apply(&moved).unwrap();
    // Still: nothing.
    assert!(sender.frame(&at(-7.0)).is_none());
}

/// After frames that change nothing, a layer left out is still removed
/// and the fragment only it drew let go.
#[test]
fn a_layer_left_out_after_still_frames_is_removed() {
    let (a, b) = (squares(1), squares(3));
    let both = vec![
        (1, Layer::new(Arc::clone(&a))),
        (2, Layer::new(Arc::clone(&b))),
    ];
    let mut sender = Sender::new();
    let mut receiver = Receiver::new();
    receiver.apply(&sender.frame(&both).unwrap()).unwrap();
    assert!(sender.frame(&both).is_none());
    assert!(sender.frame(&both).is_none());
    let one = sender.frame(&[(1, Layer::new(Arc::clone(&a)))]).unwrap();
    receiver.apply(&one).unwrap();
    assert_eq!(receiver.layers().len(), 1);
    assert!(receiver.layers().get(2).is_none());
    // Shown again, it crosses whole: its fragment was let go.
    let again = sender.frame(&both).unwrap();
    assert!(again.len() > 4 + 57, "{} bytes", again.len());
}

/// What a frame that moves every one of Keyflow's 1,851 layers costs the
/// sender (`--ignored --nocapture`): p50 and p99 over 400 frames.
#[test]
#[ignore = "it times things: --ignored --nocapture"]
#[allow(clippy::print_stdout)]
fn sender_frame_cost_for_a_scroll_of_1851_layers() {
    let content: Vec<_> = (0..1851).map(|_| squares(4)).collect();
    let at = |dy: f64| -> Vec<(u64, Layer)> {
        content
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let y = f64::from(u32::try_from(i).unwrap()).mul_add(20.0, dy);
                (
                    u64::try_from(i).unwrap(),
                    Layer::new(Arc::clone(c)).at(Affine::translate((0.0, y))),
                )
            })
            .collect()
    };
    let mut sender = Sender::new();
    let _ = sender.frame(&at(0.0));
    let mut times = Vec::new();
    for frame in 1..=400_u32 {
        let layers = at(-f64::from(frame));
        let t = std::time::Instant::now();
        let _ = sender.frame(&layers);
        times.push(t.elapsed());
    }
    times.sort();
    println!(
        "sender, 1,851 layers moved: p50 {:?}, p99 {:?}",
        times[times.len() / 2],
        times[times.len() * 99 / 100]
    );
}
