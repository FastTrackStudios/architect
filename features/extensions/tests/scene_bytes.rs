//! Scene batches as bytes on the remote-view wire (#835, `wire::BYTES_WIRE`).
//!
//! A view whose host opened it with `scene_view` puts each layers batch
//! in its thread's outbox (`scene::outbox`) and sets its element's
//! `data-architect-layers` to the batch's number alone; its frame is a
//! `Frame::Batches`, the bytes beside the edits. The host takes a batch
//! at the edit that names it, so a batch lands exactly where its base64
//! attribute did: after the edits before it, before those after it.
//! Sketch's process tests (`apps/sandbox/sketch/tests/sketch.rs`) run the
//! whole path through a real process.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::missing_panics_doc
)]

use std::sync::Arc;

use architect_extensions::guest::WireWriter;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::scene::{LAYERS_ATTR, layers_marker, layers_value, outbox};
use architect_extensions::wire::{Batch, Edit, Frame, Value};
use architect_scene::layers::Layer;
use architect_scene::ops::Sender;
use dioxus::core::{ElementId, WriteMutations as _};

/// A batch that brings one layer of `fills` rectangles.
fn batch(fills: u32) -> Vec<u8> {
    let mut scene = anyrender::Scene::new();
    for i in 0..fills {
        let x = f64::from(i) * 4.0;
        anyrender::PaintScene::fill(
            &mut scene,
            peniko::Fill::NonZero,
            peniko::kurbo::Affine::IDENTITY,
            peniko::Color::BLACK,
            None,
            &peniko::kurbo::Rect::new(x, 0.0, x + 3.0, 3.0),
        );
    }
    let mut sender = Sender::new();
    sender
        .frame(&[(1, Layer::new(Arc::new(scene.commands)))])
        .unwrap()
}

/// `<div>` as element 1 under the root, its layers attribute `value`.
fn with_layers(value: String) -> Vec<Edit> {
    vec![
        Edit::PushId { id: 0 },
        Edit::CreateElement {
            tag: "div".to_owned(),
            namespace: None,
        },
        Edit::SetId { id: 1 },
        Edit::SetAttribute {
            name: LAYERS_ATTR.to_owned(),
            namespace: None,
            value: Value::Text(value),
        },
        Edit::AppendChildren { m: 1 },
        Edit::Pop,
    ]
}

fn view() -> RemoteDocument {
    let (events, _) = futures::channel::mpsc::unbounded();
    RemoteDocument::new(events)
}

#[test]
fn a_batch_as_bytes_lands_at_the_edit_that_names_it_as_the_attribute_did() {
    let bytes = batch(200);
    // Before: the batch in the attribute, base64.
    let mut was = view();
    was.apply(Frame::Edits {
        edits: with_layers(layers_value(1, &bytes)),
    })
    .unwrap();
    // Now: its number in the attribute, its bytes beside the edits.
    let mut now = view();
    now.apply(Frame::Batches {
        edits: with_layers(layers_marker(7)),
        batches: vec![Batch {
            n: 7,
            bytes: bytes.clone(),
        }],
    })
    .unwrap();
    assert_eq!(
        now.scene_bytes(),
        u64::try_from(bytes.len()).unwrap(),
        "the bytes, not an encoding of them"
    );
    assert_eq!(
        was.scene_bytes(),
        now.scene_bytes(),
        "the same batch applied either way"
    );
}

#[test]
fn a_batch_no_edit_names_or_a_name_with_no_batch_fails_the_view() {
    let mut unnamed = view();
    let error = unnamed
        .apply(Frame::Batches {
            edits: vec![],
            batches: vec![Batch {
                n: 3,
                bytes: batch(1),
            }],
        })
        .unwrap_err();
    assert!(error.contains("no edit of their frame named"), "{error}");

    let mut missing = view();
    let error = missing
        .apply(Frame::Batches {
            edits: with_layers(layers_marker(4)),
            batches: vec![],
        })
        .unwrap_err();
    assert!(error.contains("did not carry"), "{error}");

    let mut twice = view();
    let error = twice
        .apply(Frame::Batches {
            edits: with_layers(layers_marker(5)),
            batches: vec![
                Batch {
                    n: 5,
                    bytes: batch(1),
                },
                Batch {
                    n: 5,
                    bytes: batch(1),
                },
            ],
        })
        .unwrap_err();
    assert!(error.contains("twice"), "{error}");
}

#[test]
fn a_views_thread_sends_its_batches_as_bytes_only_when_asked_and_numbers_them() {
    std::thread::spawn(|| {
        // Not asked: the batch comes back, to go in the attribute.
        assert!(!outbox::enabled());
        assert_eq!(outbox::put(vec![1, 2]), Err(vec![1, 2]));
        outbox::enable();
        assert_eq!(outbox::put(vec![1]).unwrap(), layers_marker(1));
        assert_eq!(outbox::put(vec![2, 2]).unwrap(), layers_marker(2));
        // The writer takes them with the frame's edits.
        let mut writer = WireWriter::default();
        writer.push_id(ElementId::from_raw(0));
        let Frame::Batches { edits, batches } = writer.take() else {
            panic!("a frame with batches is Frame::Batches");
        };
        assert_eq!(edits, vec![Edit::PushId { id: 0 }]);
        assert_eq!(
            batches,
            vec![
                Batch {
                    n: 1,
                    bytes: vec![1]
                },
                Batch {
                    n: 2,
                    bytes: vec![2, 2]
                },
            ]
        );
        // None since: a frame of edits alone.
        assert!(matches!(writer.take(), Frame::Edits { .. }));
    })
    .join()
    .unwrap();
}
