//! A pane's layers: paint order, changes in place, and what they
//! compose to.

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
use architect_scene::layers::{Layer, Layers};
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};

/// A layer of one filled square whose left edge is `x`.
fn square(x: f64) -> Layer {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(200, 30, 30),
        None,
        &Rect::new(x, 0.0, x + 10.0, 10.0),
    );
    Layer::new(Arc::new(scene.commands))
}

/// The translation of each fill in `scene`, in paint order.
fn fills(scene: &Scene) -> Vec<(f64, f64)> {
    scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::Fill(f) => {
                let t = f.transform.as_coeffs();
                Some((t[4], t[5]))
            }
            _ => None,
        })
        .collect()
}

fn composed(layers: &Layers, root: Affine) -> Scene {
    let mut scene = Scene::new();
    layers.compose(&mut scene, root);
    scene
}

#[test]
fn layers_paint_by_z_then_by_key() {
    let mut layers = Layers::new();
    layers.set(3, square(0.0).at(Affine::translate((3.0, 0.0))));
    layers.set(1, square(0.0).at(Affine::translate((1.0, 0.0))).z(5));
    layers.set(2, square(0.0).at(Affine::translate((2.0, 0.0))));
    let order: Vec<u64> = layers.in_order().map(|(key, _)| key).collect();
    assert_eq!(order, [2, 3, 1]);
    assert_eq!(
        fills(&composed(&layers, Affine::IDENTITY)),
        [(2.0, 0.0), (3.0, 0.0), (1.0, 0.0)]
    );
}

#[test]
fn moving_a_layer_changes_its_transform_and_not_its_content() {
    let mut layers = Layers::new();
    layers.set(1, square(0.0));
    let content = Arc::clone(&layers.get(1).unwrap().commands);
    let before = layers.revision();
    assert!(layers.update(1, |l| l.transform = Affine::translate((40.0, 8.0))));
    assert!(layers.revision() > before);
    assert!(Arc::ptr_eq(&content, &layers.get(1).unwrap().commands));
    assert_eq!(fills(&composed(&layers, Affine::IDENTITY)), [(40.0, 8.0)]);
}

#[test]
fn the_root_transform_is_applied_over_each_layers() {
    let mut layers = Layers::new();
    layers.set(1, square(0.0).at(Affine::translate((5.0, 0.0))));
    let scene = composed(&layers, Affine::scale(2.0));
    assert_eq!(fills(&scene), [(10.0, 0.0)]);
}

#[test]
fn a_hidden_or_transparent_layer_is_not_drawn() {
    let mut layers = Layers::new();
    layers.set(1, square(0.0));
    layers.set(2, square(0.0).opacity(0.0));
    layers.update(1, |l| l.visible = false);
    assert!(composed(&layers, Affine::IDENTITY).commands.is_empty());
}

#[test]
fn a_clip_and_a_fade_wrap_the_layer_and_are_closed() {
    let mut layers = Layers::new();
    layers.set(
        1,
        square(0.0)
            .opacity(0.5)
            .clipped(Rect::new(0.0, 0.0, 4.0, 4.0)),
    );
    let scene = composed(&layers, Affine::IDENTITY);
    let kinds: Vec<&str> = scene
        .commands
        .iter()
        .map(|c| match c {
            RenderCommand::PushClipLayer(_) => "clip",
            RenderCommand::PushLayer(_) => "layer",
            RenderCommand::PopLayer => "pop",
            RenderCommand::Fill(_) => "fill",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["clip", "layer", "fill", "pop", "pop"]);
}

#[test]
fn removing_and_changing_z_reorder() {
    let mut layers = Layers::new();
    layers.set(1, square(0.0));
    layers.set(2, square(0.0));
    assert!(layers.update(1, |l| l.z = 9));
    assert_eq!(
        layers.in_order().map(|(k, _)| k).collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(layers.remove(2));
    assert!(!layers.remove(2));
    assert!(!layers.update(2, |_| {}));
    assert_eq!(layers.len(), 1);
    let before = layers.revision();
    layers.clear();
    assert!(layers.is_empty());
    assert!(layers.revision() > before);
}
