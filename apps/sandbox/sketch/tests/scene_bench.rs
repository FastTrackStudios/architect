//! What a scene costs to cross today (#175): the size of its content on
//! the wire, and the time to encode, decode and re-send it, for the
//! scenes a runtime extension has to hold native frame rates on: an
//! arrangement's clips and a chart's glyphs. The baseline the fragment
//! wire is measured against. p50/p99 of each, printed.
//!
//! Not in the default run (it times things):
//! `cargo test -p architect-sandbox-sketch --test scene_bench -- --ignored --nocapture`.
//!
//! The chart is synthetic: glyph runs don't cross today, so a chart's
//! text is drawn as outlines, and each glyph here is a closed outline of
//! 24 curves with coordinates as a font's would be. A real chart
//! (Keyflow's *Drive To LA*) is about 2,200 elements.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::default_trait_access,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use anyrender::Glyph;
use anyrender::recording::GlyphRunCommand;
use architect_extensions::scene::{
    Affine, PaintScene as _, RenderCommand, Scene, SceneCache, decode, encode,
};
use architect_sandbox_sketch::arrangement_scene;
use architect_workspace::paint::peniko::kurbo::{BezPath, Stroke};
use architect_workspace::paint::peniko::{Blob, Color, Fill, FontData, Style};

/// One frame at 240 Hz.
const FRAME_240: f64 = 1.0e6 / 240.0;

fn time(n: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..n)
        .map(|_| {
            let began = Instant::now();
            f();
            began.elapsed()
        })
        .collect()
}

/// p50 and p99 of `samples` in µs, and p99 as a share of a 240 Hz frame.
fn report(scene: &str, what: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let at = |q: usize| samples[(samples.len() - 1) * q / 100].as_secs_f64() * 1e6;
    println!(
        "| {scene:<22} | {what:<22} | {:>9.1} | {:>9.1} | {:>7.0}% |",
        at(50),
        at(99),
        at(99) / FRAME_240 * 100.0
    );
}

/// A glyph-shaped outline at (`x`, `y`): a closed path of 24 curves.
fn glyph(x: f64, y: f64, seed: u32) -> BezPath {
    let mut path = BezPath::new();
    let wobble = |i: u32| f64::from((seed.wrapping_mul(31).wrapping_add(i * 17)) % 97) / 97.0;
    path.move_to((x, y));
    for i in 0..24_u32 {
        let angle = f64::from(i) / 24.0 * std::f64::consts::TAU;
        let radius = f64::mul_add(wobble(i), 3.0, 4.0);
        let (sx, sy) = (
            angle.cos().mul_add(radius, x),
            angle.sin().mul_add(radius, y),
        );
        path.curve_to(
            (sx + wobble(i + 1), sy - wobble(i + 2)),
            (sx - wobble(i + 3), sy + wobble(i + 4)),
            (sx + 0.37, sy + 0.61),
        );
    }
    path.close_path();
    path
}

/// A chart's staff lines and barlines, for `systems` systems.
fn staves(scene: &mut Scene, systems: u32, ink: Color) {
    for s in 0..systems {
        let top = f64::mul_add(f64::from(s), 90.0, 40.0);
        for line in 0..5_u32 {
            let y = f64::mul_add(f64::from(line), 6.0, top);
            let mut staff = BezPath::new();
            staff.move_to((36.0, y));
            staff.line_to((1164.0, y));
            scene.stroke(&Stroke::new(0.6), Affine::IDENTITY, ink, None, &staff);
        }
        for bar in 0..5_u32 {
            let x = f64::mul_add(f64::from(bar), 282.0, 36.0);
            let mut line = BezPath::new();
            line.move_to((x, top));
            line.line_to((x, top + 24.0));
            scene.stroke(&Stroke::new(0.9), Affine::IDENTITY, ink, None, &line);
        }
    }
}

/// The same chart with its text as glyph runs of about 20 glyphs, which
/// only the fragment format carries: the glyphs sit where
/// [`chart_scene`]'s outlines do.
fn chart_glyph_runs(glyphs: u32, font: &FontData) -> Vec<RenderCommand> {
    let mut scene = Scene::new();
    let ink = Color::from_rgb8(20, 20, 24);
    let per_system = 110;
    staves(&mut scene, glyphs.div_ceil(per_system), ink);
    let mut commands = scene.commands;
    let all: Vec<u32> = (0..glyphs).collect();
    for run in all.chunks(20) {
        commands.push(RenderCommand::GlyphRun(GlyphRunCommand {
            font_data: font.clone(),
            font_size: 12.0,
            hint: false,
            normalized_coords: Default::default(),
            embolden: Default::default(),
            style: Style::Fill(Fill::NonZero),
            brush: ink.into(),
            brush_alpha: 1.0,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            glyphs: run
                .iter()
                .map(|&g| {
                    let (s, i) = (g / per_system, g % per_system);
                    Glyph {
                        id: g % 500 + 1,
                        x: (i as f32).mul_add(10.2, 44.0),
                        y: (s as f32).mul_add(90.0, 40.0) + ((g * 7) % 30) as f32,
                    }
                })
                .collect(),
        }));
    }
    commands
}

/// The fragment format's rows for `commands`: bytes, then encode and
/// decode. `first` also times an encoder that has not seen the scene's
/// resources (it hashes them).
fn measure_fragment(name: &str, commands: &[RenderCommand], first: bool) {
    let encoded = architect_scene::Encoder::new().encode(commands);
    println!(
        "| {name:<22} | {:<22} | {:>9} | {:>9} | {:>8} |",
        "fragment: cmds, bytes",
        commands.len(),
        encoded.bytes.len(),
        ""
    );
    if first {
        report(
            name,
            "fragment encode, first",
            time(50, || {
                drop(architect_scene::Encoder::new().encode(commands));
            }),
        );
    }
    let mut warm = architect_scene::Encoder::new();
    let _ = warm.encode(commands);
    report(
        name,
        if first {
            "fragment encode, later"
        } else {
            "fragment encode"
        },
        time(200, || drop(warm.encode(commands))),
    );
    let mut resources = architect_scene::Resources::new();
    for (_, resource) in encoded.resources {
        resources.insert(resource);
    }
    match architect_scene::decode(&encoded.bytes, &resources) {
        Ok(_) => report(
            name,
            "fragment decode",
            time(200, || {
                drop(architect_scene::decode(&encoded.bytes, &resources));
            }),
        ),
        Err(why) => println!("| {name:<22} | fragment decode: refused ({why}) |"),
    }
}

/// A chart as it crosses today: `glyphs` outlines in systems of staff
/// lines and barlines.
fn chart_scene(glyphs: u32) -> Vec<RenderCommand> {
    let mut scene = Scene::new();
    let ink = Color::from_rgb8(20, 20, 24);
    let per_system = 110;
    staves(&mut scene, glyphs.div_ceil(per_system), ink);
    for g in 0..glyphs {
        let (s, i) = (g / per_system, g % per_system);
        let x = f64::mul_add(f64::from(i), 10.2, 44.0);
        let y = f64::mul_add(f64::from(s), 90.0, 40.0) + f64::from((g * 7) % 30);
        scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &glyph(x, y, g));
    }
    scene.commands
}

fn measure(name: &str, commands: &[RenderCommand]) {
    let (_, json) = encode(commands);
    println!(
        "| {name:<22} | {:<22} | {:>9} | {:>9} | {:>8} |",
        "commands, bytes",
        commands.len(),
        json.len(),
        ""
    );
    report(name, "encode", time(200, || drop(encode(commands))));
    match decode(&json) {
        Ok(_) => report(name, "decode", time(200, || drop(decode(&json)))),
        // Past a limit, the host refuses it: the view fails.
        Err(why) => println!("| {name:<22} | decode: refused ({why}) |"),
    }
    // A frame whose content did not change: what the guest still pays.
    let mut cache = SceneCache::new();
    let _ = cache.send(commands);
    report(
        name,
        "send, unchanged",
        time(200, || drop(cache.send(commands))),
    );
    measure_fragment(name, commands, false);
}

#[test]
#[ignore = "it times things: run it with --ignored --nocapture"]
fn what_a_scene_costs_to_cross() {
    println!(
        "| scene                  | what                   |  p50 (µs) |  p99 (µs) | of 240Hz |"
    );
    println!("|---|---|---:|---:|---:|");
    measure("arrangement 50x40", &arrangement_scene(50, 40));
    measure("arrangement 200x100", &arrangement_scene(200, 100));
    measure("chart, 2200 glyphs", &chart_scene(2200));
    let font = FontData::new(
        Blob::from(include_bytes!("../../../ui/assets/fonts/Inter-Variable.ttf").to_vec()),
        0,
    );
    measure_fragment("chart, glyph runs", &chart_glyph_runs(2200, &font), true);
}
