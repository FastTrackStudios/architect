//! Fragments round trip, and what a host is sent is checked.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::useless_conversion
)]

use std::sync::Arc;

use anyrender::recording::{
    BoxShadowCommand, ClipCommand, FillCommand, GlyphRunCommand, LayerCommand, RenderCommand,
    StrokeCommand,
};
use anyrender::{Glyph, Paint};
use architect_scene::{Dropped, Encoded, Encoder, Error, Resources, decode};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Rect, Shape, Stroke, Vec2};
use peniko::{
    BlendMode, Blob, Color, Compose, Extend, Fill, FontData, Gradient, ImageAlphaType, ImageBrush,
    ImageData, ImageFormat, ImageQuality, ImageSampler, Mix, Style,
};

fn red() -> Paint {
    Paint::Solid(Color::from_rgba8(255, 0, 0, 255))
}

fn triangle() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((10.0, 0.0));
    p.quad_to((12.0, 4.0), (10.0, 8.0));
    p.curve_to((8.0, 9.0), (4.0, 9.0), (0.0, 8.0));
    p.close_path();
    p
}

fn fill_of(shape: BezPath, brush: Paint) -> RenderCommand {
    RenderCommand::Fill(FillCommand {
        fill: Fill::EvenOdd,
        transform: Affine::translate((3.0, 4.5)),
        brush,
        brush_transform: Some(Affine::scale(2.0)),
        shape,
    })
}

/// What the receiver holds after it was sent `encoded`'s resources.
fn held(encoded: &Encoded) -> Resources {
    let mut resources = Resources::new();
    for (id, resource) in &encoded.resources {
        assert_eq!(resources.insert(resource.clone()), *id);
    }
    resources
}

/// `commands` across and back; what comes back encodes to the same bytes.
fn round_trip(commands: &[RenderCommand]) -> Vec<RenderCommand> {
    let encoded = Encoder::new().encode(commands);
    assert_eq!(encoded.dropped, Dropped::default());
    let resources = held(&encoded);
    let back = decode(&encoded.bytes, &resources).unwrap();
    assert_eq!(back.len(), commands.len());
    let again = Encoder::new().encode(&back);
    assert_eq!(again.bytes, encoded.bytes);
    assert_eq!(again.id, encoded.id);
    back
}

fn solid(paint: &Paint) -> Color {
    match paint {
        Paint::Solid(c) => *c,
        _ => panic!("not a solid paint"),
    }
}

#[test]
fn fill_round_trips() {
    let back = round_trip(&[fill_of(triangle(), red())]);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    assert_eq!(f.fill, Fill::EvenOdd);
    assert_eq!(f.transform, Affine::translate((3.0, 4.5)));
    assert_eq!(f.brush_transform, Some(Affine::scale(2.0)));
    assert_eq!(solid(&f.brush), Color::from_rgba8(255, 0, 0, 255));
    assert_eq!(f.shape, triangle());
}

#[test]
fn stroke_round_trips_dashes_caps_and_joins() {
    let style = Stroke::new(3.25)
        .with_join(Join::Round)
        .with_miter_limit(7.0)
        .with_start_cap(Cap::Square)
        .with_end_cap(Cap::Butt)
        .with_dashes(1.5, [4.0, 2.0, 0.5]);
    let back = round_trip(&[RenderCommand::Stroke(StrokeCommand {
        style: style.clone(),
        transform: Affine::new([1.0, 0.5, -0.5, 1.0, 7.0, 8.0]),
        brush: Paint::Solid(Color::new([0.1, 0.2, 0.3, 0.4])),
        brush_transform: None,
        shape: triangle(),
    })]);
    let RenderCommand::Stroke(s) = &back[0] else {
        panic!("not a stroke")
    };
    assert_eq!(s.style.width, 3.25);
    assert_eq!(s.style.join, Join::Round);
    assert_eq!(s.style.miter_limit, 7.0);
    assert_eq!(s.style.start_cap, Cap::Square);
    assert_eq!(s.style.end_cap, Cap::Butt);
    assert_eq!(s.style.dash_offset, 1.5);
    assert_eq!(&s.style.dash_pattern[..], &[4.0, 2.0, 0.5]);
    assert_eq!(s.style.dash_pattern[..], style.dash_pattern[..]);
    assert_eq!(s.transform, Affine::new([1.0, 0.5, -0.5, 1.0, 7.0, 8.0]));
    assert_eq!(s.brush_transform, None);
    assert_eq!(solid(&s.brush), Color::new([0.1, 0.2, 0.3, 0.4]));
    assert_eq!(s.shape, triangle());
}

#[test]
fn rect_path_round_trips_and_is_written_short() {
    let rect = Rect::new(1.0, 2.0, 30.0, 40.0).to_path(0.1);
    let commands = [fill_of(rect.clone(), red())];
    let back = round_trip(&commands);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    assert_eq!(f.shape, rect);
    // A rectangle is four numbers, not five elements.
    let general = Encoder::new().encode(&[fill_of(triangle(), red())]);
    assert!(Encoder::new().encode(&commands).bytes.len() < general.bytes.len());

    // One whose sides are not f32s keeps them.
    let wide = Rect::new(0.1, 0.2, 30.3, 40.4).to_path(0.1);
    let back = round_trip(&[fill_of(wide.clone(), red())]);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    assert_eq!(f.shape, wide);
}

#[test]
fn f64_only_coordinates_round_trip_exactly() {
    let mut p = BezPath::new();
    p.move_to((0.1, 1.0 / 3.0));
    p.line_to((1.0e-9, 2.0));
    p.curve_to(
        (0.7, 0.3),
        (std::f64::consts::PI, 5.0),
        (9.9, 0.123_456_789_012_345),
    );
    p.close_path();
    assert_ne!(f64::from(0.1_f64 as f32), 0.1_f64);
    let back = round_trip(&[fill_of(p.clone(), red())]);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    assert_eq!(f.shape, p);
}

#[test]
fn clip_round_trips() {
    let back = round_trip(&[
        RenderCommand::PushClipLayer(ClipCommand {
            fill: Fill::NonZero,
            transform: Affine::translate((1.0, 2.0)),
            clip: triangle(),
        }),
        RenderCommand::PopLayer,
    ]);
    let RenderCommand::PushClipLayer(c) = &back[0] else {
        panic!("not a clip")
    };
    assert_eq!(c.fill, Fill::NonZero);
    assert_eq!(c.transform, Affine::translate((1.0, 2.0)));
    assert_eq!(c.clip, triangle());
    assert!(matches!(back[1], RenderCommand::PopLayer));
}

fn layer() -> RenderCommand {
    RenderCommand::PushLayer(LayerCommand {
        fill: Fill::EvenOdd,
        blend: BlendMode {
            mix: Mix::Multiply,
            compose: Compose::SrcAtop,
        },
        alpha: 0.375,
        transform: Affine::IDENTITY,
        clip: Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1),
        filter: None,
        backdrop_filter: None,
    })
}

#[test]
fn layer_round_trips_blend_and_alpha() {
    let back = round_trip(&[layer(), RenderCommand::PopLayer]);
    let RenderCommand::PushLayer(l) = &back[0] else {
        panic!("not a layer")
    };
    assert_eq!(l.fill, Fill::EvenOdd);
    assert_eq!(l.blend.mix, Mix::Multiply);
    assert_eq!(l.blend.compose, Compose::SrcAtop);
    assert_eq!(l.alpha, 0.375);
    assert_eq!(l.transform, Affine::IDENTITY);
    assert_eq!(l.clip, Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1));
    assert!(matches!(back[1], RenderCommand::PopLayer));
}

#[test]
fn box_shadow_round_trips() {
    let back = round_trip(&[RenderCommand::BoxShadow(BoxShadowCommand {
        transform: Affine::translate((5.0, 6.0)),
        rect: Rect::new(0.1, 0.2, 20.0, 10.0),
        brush: Color::from_rgba8(0, 0, 0, 128),
        radius: 4.5,
        std_dev: 2.25,
    })]);
    let RenderCommand::BoxShadow(b) = &back[0] else {
        panic!("not a box shadow")
    };
    assert_eq!(b.transform, Affine::translate((5.0, 6.0)));
    assert_eq!(b.rect, Rect::new(0.1, 0.2, 20.0, 10.0));
    assert_eq!(b.brush, Color::from_rgba8(0, 0, 0, 128));
    assert_eq!(b.radius, 4.5);
    assert_eq!(b.std_dev, 2.25);
}

#[test]
fn linear_gradient_round_trips() {
    let gradient = Gradient::new_linear((0.0, 0.5), (10.0, 5.0))
        .with_extend(Extend::Reflect)
        .with_stops([
            Color::from_rgba8(255, 0, 0, 255),
            Color::from_rgba8(0, 255, 0, 128),
            Color::from_rgba8(0, 0, 255, 255),
        ]);
    let back = round_trip(&[fill_of(triangle(), Paint::Gradient(gradient.clone()))]);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    let Paint::Gradient(g) = &f.brush else {
        panic!("not a gradient")
    };
    assert_eq!(g.kind, gradient.kind);
    assert_eq!(g.extend, Extend::Reflect);
    assert_eq!(g.interpolation_cs, gradient.interpolation_cs);
    assert_eq!(g.hue_direction, gradient.hue_direction);
    assert_eq!(g.stops.len(), 3);
    assert_eq!(*g, gradient);
}

fn image() -> ImageData {
    ImageData {
        data: Blob::from((0_u8..16).collect::<Vec<_>>()),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: 2,
        height: 2,
    }
}

#[test]
fn image_brush_round_trips_as_a_resource() {
    let brush = ImageBrush {
        image: image(),
        sampler: ImageSampler {
            x_extend: Extend::Repeat,
            y_extend: Extend::Pad,
            quality: ImageQuality::High,
            alpha: 0.5,
        },
    };
    let commands = [fill_of(triangle(), Paint::Image(brush))];
    let encoded = Encoder::new().encode(&commands);
    assert_eq!(encoded.resources.len(), 1);
    // The fragment names its pixels; it doesn't carry them.
    assert_eq!(
        decode(&encoded.bytes, &Resources::new()).err(),
        Some(Error::MissingResource(encoded.resources[0].0))
    );
    let back = round_trip(&commands);
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    let Paint::Image(i) = &f.brush else {
        panic!("not an image")
    };
    assert_eq!(i.image.width, 2);
    assert_eq!(i.image.height, 2);
    assert_eq!(i.image.format, ImageFormat::Rgba8);
    assert_eq!(i.image.alpha_type, ImageAlphaType::Alpha);
    assert_eq!(i.image.data.data(), image().data.data());
    assert_eq!(i.sampler.x_extend, Extend::Repeat);
    assert_eq!(i.sampler.y_extend, Extend::Pad);
    assert_eq!(i.sampler.quality, ImageQuality::High);
    assert_eq!(i.sampler.alpha, 0.5);
}

fn glyph_run() -> RenderCommand {
    RenderCommand::GlyphRun(GlyphRunCommand {
        font_data: FontData::new(Blob::from(b"not a font, only its bytes".to_vec()), 2),
        font_size: 14.5,
        hint: true,
        normalized_coords: vec![1_i16, -2, 300].into(),
        embolden: Vec2::new(0.25, 0.5),
        style: Style::Fill(Fill::NonZero),
        brush: red(),
        brush_alpha: 0.75,
        transform: Affine::translate((10.0, 20.0)),
        glyph_transform: Some(Affine::skew(0.2, 0.0)),
        glyphs: vec![
            Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
            },
            Glyph {
                id: 65_000,
                x: 8.25,
                y: -1.5,
            },
        ]
        .into(),
    })
}

#[test]
fn glyph_run_round_trips() {
    let back = round_trip(&[glyph_run()]);
    let RenderCommand::GlyphRun(g) = &back[0] else {
        panic!("not a glyph run")
    };
    assert_eq!(g.font_data.data.data(), b"not a font, only its bytes");
    assert_eq!(g.font_data.index, 2);
    assert_eq!(g.font_size, 14.5);
    assert!(g.hint);
    assert_eq!(&g.normalized_coords[..], &[1_i16, -2, 300]);
    assert_eq!(g.embolden, Vec2::new(0.25, 0.5));
    assert!(matches!(g.style, Style::Fill(Fill::NonZero)));
    assert_eq!(solid(&g.brush), Color::from_rgba8(255, 0, 0, 255));
    assert_eq!(g.brush_alpha, 0.75);
    assert_eq!(g.transform, Affine::translate((10.0, 20.0)));
    assert_eq!(g.glyph_transform, Some(Affine::skew(0.2, 0.0)));
    let glyphs: Vec<(u32, f32, f32)> = g.glyphs.iter().map(|g| (g.id, g.x, g.y)).collect();
    assert_eq!(glyphs, vec![(7, 0.0, 0.0), (65_000, 8.25, -1.5)]);
}

#[test]
fn one_font_is_one_resource_however_often_it_is_drawn() {
    let encoded = Encoder::new().encode(&[glyph_run(), glyph_run()]);
    assert_eq!(encoded.resources.len(), 1);
}

#[test]
fn a_font_the_receiver_does_not_hold_is_missing() {
    let encoded = Encoder::new().encode(&[glyph_run()]);
    let font = encoded.resources[0].0;
    assert_eq!(
        decode(&encoded.bytes, &Resources::new()).err(),
        Some(Error::MissingResource(font))
    );
    // Held, then let go of: missing again.
    let mut resources = held(&encoded);
    assert!(decode(&encoded.bytes, &resources).is_ok());
    assert!(resources.remove(font));
    assert_eq!(
        decode(&encoded.bytes, &resources).err(),
        Some(Error::MissingResource(font))
    );
}

#[test]
fn truncated_bytes_fail() {
    let encoded = Encoder::new().encode(&[fill_of(triangle(), red()), glyph_run()]);
    let resources = held(&encoded);
    // Every cut after the magic is refused as truncated, never a panic.
    for len in 4..encoded.bytes.len() {
        assert_eq!(
            decode(&encoded.bytes[..len], &resources).err(),
            Some(Error::Truncated),
            "cut at {len}"
        );
    }
}

#[test]
fn bad_magic_is_not_a_fragment() {
    let mut bytes = Encoder::new().encode(&[fill_of(triangle(), red())]).bytes;
    bytes[3] = 2;
    assert_eq!(
        decode(&bytes, &Resources::new()).err(),
        Some(Error::NotAFragment)
    );
    assert_eq!(
        decode(b"AS", &Resources::new()).err(),
        Some(Error::NotAFragment)
    );
    assert_eq!(
        decode(&[], &Resources::new()).err(),
        Some(Error::NotAFragment)
    );
}

#[test]
fn a_pop_with_no_layer_fails() {
    let encoded = Encoder::new().encode(&[RenderCommand::PopLayer]);
    assert_eq!(
        decode(&encoded.bytes, &Resources::new()).err(),
        Some(Error::UnbalancedPop)
    );
    let encoded =
        Encoder::new().encode(&[layer(), RenderCommand::PopLayer, RenderCommand::PopLayer]);
    assert_eq!(
        decode(&encoded.bytes, &Resources::new()).err(),
        Some(Error::UnbalancedPop)
    );
}

#[test]
fn an_open_layer_is_closed() {
    let clip = RenderCommand::PushClipLayer(ClipCommand {
        fill: Fill::NonZero,
        transform: Affine::IDENTITY,
        clip: triangle(),
    });
    let encoded = Encoder::new().encode(&[layer(), clip, fill_of(triangle(), red())]);
    let back = decode(&encoded.bytes, &Resources::new()).unwrap();
    assert_eq!(back.len(), 5);
    assert!(matches!(back[0], RenderCommand::PushLayer(_)));
    assert!(matches!(back[1], RenderCommand::PushClipLayer(_)));
    assert!(matches!(back[2], RenderCommand::Fill(_)));
    assert!(matches!(back[3], RenderCommand::PopLayer));
    assert!(matches!(back[4], RenderCommand::PopLayer));
}

#[test]
fn a_nan_coordinate_is_out_of_range() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((f64::NAN, 1.0));
    let encoded = Encoder::new().encode(&[fill_of(p, red())]);
    assert_eq!(
        decode(&encoded.bytes, &Resources::new()).err(),
        Some(Error::OutOfRange)
    );

    // And one past the largest a renderer is handed, or not finite.
    for bad in [1.0e8, f64::INFINITY] {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((1.0, bad));
        let encoded = Encoder::new().encode(&[fill_of(p, red())]);
        assert_eq!(
            decode(&encoded.bytes, &Resources::new()).err(),
            Some(Error::OutOfRange),
            "{bad}"
        );
    }
}

#[test]
fn a_custom_paint_is_counted_and_drawn_transparent() {
    let any: Arc<dyn std::any::Any + Send + Sync> = Arc::new(7_u8);
    let custom: Paint = Paint::Custom(any);
    let encoded = Encoder::new().encode(&[fill_of(triangle(), custom), fill_of(triangle(), red())]);
    assert_eq!(
        encoded.dropped,
        Dropped {
            filters: 0,
            paints: 1
        }
    );
    let back = decode(&encoded.bytes, &Resources::new()).unwrap();
    let RenderCommand::Fill(f) = &back[0] else {
        panic!("not a fill")
    };
    assert_eq!(solid(&f.brush), Color::TRANSPARENT);
}
