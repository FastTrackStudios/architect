//! A paint view's picture and its hit test are one list: rasterised on
//! the CPU, the pixel under any point is the colour of the mark `hit`
//! finds there (or nothing where it finds none), at 1× and 2×, with
//! marks overlapping and after live input restyled one.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::print_stdout,
    clippy::use_debug
)]

use std::time::{Duration, Instant};

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use architect::action::{ActionCaller, ActionList};
use peniko::Color;
use peniko::kurbo::{Affine, Point, Rect};

use super::*;
use crate::PaneId;
use crate::kinds::{KindId, Placement, ViewHost};

const COLS: u64 = 100;
const ROWS: u64 = 100;
/// A cell's pitch and its rect's size, in logical px (a 1 px gap).
const W: f64 = 8.0;
const H: f64 = 6.0;
const HOVER: Color = Color::from_rgba8(255, 255, 255, 255);

/// A distinct opaque colour per id.
fn shade(id: u64) -> Color {
    let id = id + 1;
    Color::from_rgba8(
        (id % 200) as u8 + 20,
        ((id / 200) % 200) as u8 + 20,
        ((id * 7) % 50) as u8,
        255,
    )
}

/// 10 000 cells, then three bars over them (on top, so they win).
fn grid() -> Marks {
    let mut marks: Marks = (0..COLS * ROWS)
        .map(|id| {
            let (col, row) = ((id % COLS) as f64, (id / COLS) as f64);
            Mark {
                id,
                rect: Rect::new(col * W, row * H, col * W + W - 1.0, row * H + H - 1.0),
                fill: shade(id),
            }
        })
        .collect();
    for (i, y) in [40.0, 200.0, 420.0].into_iter().enumerate() {
        let id = COLS * ROWS + i as u64;
        marks.push(Mark {
            id,
            rect: Rect::new(30.0, y, 610.0, y + 23.0),
            fill: shade(id),
        });
    }
    marks
}

/// The lab's view in miniature: the grid, the mark under the pointer lit.
struct Grid {
    marks: Marks,
    hover: Option<(usize, Color)>,
}

impl PaintView for Grid {
    fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
        let ViewInput::PointerMove { x, y } = *event else {
            return false;
        };
        if let Some((at, fill)) = self.hover.take() {
            self.marks.as_mut_slice()[at].fill = fill;
        }
        let at = Point::new(f64::from(x), f64::from(y));
        let under = self
            .marks
            .as_slice()
            .iter()
            .rposition(|m| m.rect.contains(at));
        if let Some(i) = under {
            let mark = &mut self.marks.as_mut_slice()[i];
            self.hover = Some((i, mark.fill));
            mark.fill = HOVER;
        }
        true
    }

    fn paint(&mut self, _: &ViewCtx, scene: &mut Scene) {
        self.marks.paint(scene, Affine::IDENTITY);
    }

    fn hit(&self, _: &ViewCtx, at: Point) -> Option<u64> {
        self.marks.hit(at).map(|m| m.id)
    }
}

fn painter() -> Painter {
    let began = Instant::now();
    let ctx = ViewCtx::new(
        PaneId::new("paint"),
        KindId::paint(),
        (ViewHost::Blitz, Placement::Pane),
        (ActionList::new(), ActionCaller::app()),
        crate::ctx::FrameClock::new(Duration::from_millis(16), move || began.elapsed()),
    );
    Painter::new(
        ctx,
        Grid {
            marks: grid(),
            hover: None,
        },
    )
}

const SIZE: Geometry = Geometry {
    width: (COLS as f32) * 8.0,
    height: (ROWS as f32) * 6.0,
    scale: 1.0,
};

/// `scene` on the CPU: RGBA, `w` × `h` physical px.
fn raster(scene: Scene, w: u32, h: u32) -> Vec<u8> {
    render_to_buffer::<VelloCpuImageRenderer, _>(|s| s.append_scene(scene, Affine::IDENTITY), w, h)
}

/// Every cell's middle, every gap, and a scatter over the bars' edges.
fn probes() -> Vec<Point> {
    let mut at = Vec::new();
    for id in 0..COLS * ROWS {
        let (col, row) = ((id % COLS) as f64, (id / COLS) as f64);
        at.push(Point::new(col * W + 3.5, row * H + 2.5));
        at.push(Point::new(col * W + 7.5, row * H + 5.5));
    }
    let mut seed = 0x2545_f491_u64;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let x = f64::from((seed >> 33) as u32 % 800) + 0.5;
        let y = f64::from((seed >> 13) as u32 % 600) + 0.5;
        at.push(Point::new(x, y));
    }
    at
}

/// What is drawn under each probe is what `hit` finds there.
fn agrees(painter: &Painter, pixels: &[u8], scale: f64) {
    let w = (f64::from(SIZE.width) * scale) as usize;
    let marks = grid();
    let mut checked = 0;
    for at in probes() {
        let (px, py) = ((at.x * scale) as usize, (at.y * scale) as usize);
        let i = (py * w + px) * 4;
        let drawn = &pixels[i..i + 4];
        let want = painter.hit(at).map_or([0, 0, 0, 0], |id| {
            let fill = marks
                .as_slice()
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.fill)
                .unwrap();
            // A lit mark is drawn lit.
            let fill = if painter_lit(painter, at) {
                HOVER
            } else {
                fill
            };
            fill.to_rgba8().to_u8_array()
        });
        assert!(
            drawn.iter().zip(want).all(|(d, w)| d.abs_diff(w) <= 1),
            "at {at:?}: drawn {drawn:?}, hit says {want:?} ({:?})",
            painter.hit(at)
        );
        checked += 1;
    }
    assert!(checked > 20_000);
}

/// Whether the mark under `at` is the lit one (the probe's own pointer).
fn painter_lit(painter: &Painter, at: Point) -> bool {
    LIT.with(|l| l.get()) == painter.hit(at)
}

thread_local! {
    static LIT: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[test]
fn paint_and_hit_testing_read_one_list() {
    for scale in [1.0, 2.0] {
        let mut painter = painter();
        LIT.with(|l| l.set(None));
        let size = Geometry { scale, ..SIZE };
        let (w, h) = ((SIZE.width * scale) as u32, (SIZE.height * scale) as u32);
        let pixels = raster(painter.frame(size), w, h);
        agrees(&painter, &pixels, f64::from(scale));

        // Live input: the pointer over a bar's cell lights the bar, not the
        // cell under it, in the picture and in the hit test alike.
        painter
            .ctx()
            .input
            .push(ViewInput::PointerMove { x: 300.5, y: 45.5 });
        assert!(painter.wants_frame());
        let lit = painter.hit(Point::new(300.5, 45.5));
        assert_eq!(lit, Some(COLS * ROWS));
        LIT.with(|l| l.set(lit));
        let pixels = raster(painter.frame(size), w, h);
        assert!(!painter.wants_frame());
        agrees(&painter, &pixels, f64::from(scale));
    }
}

#[test]
fn frame_times_for_ten_thousand_rects() {
    let mut painter = painter();
    let stats = painter.stats();
    let mut raster_times = Vec::new();
    for i in 0..240_u16 {
        let x = f32::from(i % 100) * 8.0 + 3.5;
        painter
            .ctx()
            .input
            .push(ViewInput::PointerMove { x, y: 200.5 });
        let scene = painter.frame(SIZE);
        if i % 8 == 0 {
            let began = Instant::now();
            let _ = raster(scene, 800, 600);
            raster_times.push(began.elapsed());
        }
    }
    let s = stats.summary();
    assert_eq!(s.frames, 240);
    assert!(s.p50 <= s.p99 && s.p99 <= s.max && s.p50 > Duration::ZERO);
    raster_times.sort_unstable();
    let pct = |p: usize| raster_times[(raster_times.len() * p).div_ceil(100).max(1) - 1];
    println!(
        "paint 10k rects (input + record), {} frames: p50 {:?} p99 {:?} max {:?}; \
         + CPU raster 800x600, {} frames: p50 {:?} p99 {:?}",
        s.frames,
        s.p50,
        s.p99,
        s.max,
        raster_times.len(),
        pct(50),
        pct(99)
    );
}

#[test]
fn stats_keep_the_last_frames_and_rank_them() {
    let stats = FrameStats::new();
    for ms in 1..=100 {
        stats.record(Duration::from_millis(ms));
    }
    let s = stats.summary();
    assert_eq!(
        (s.frames, s.p50, s.p99, s.max),
        (
            100,
            Duration::from_millis(50),
            Duration::from_millis(99),
            Duration::from_millis(100)
        )
    );
    for _ in 0..FrameStats::KEPT {
        stats.record(Duration::from_millis(2));
    }
    assert_eq!(stats.summary().max, Duration::from_millis(2));
}

#[test]
fn external_changes_can_invalidate_an_idle_painter() {
    let mut painter = painter();
    assert!(painter.wants_frame());
    let _ = painter.frame(SIZE);
    assert!(!painter.wants_frame());
    painter.invalidate();
    assert!(painter.wants_frame());
    let _ = painter.frame(SIZE);
    assert!(!painter.wants_frame());
}

#[test]
fn theme_tokens_read_as_colours() {
    let theme = ThemeTokens::default()
        .with("accent", "#4f7cff")
        .with("font", "Inter, sans-serif");
    assert_eq!(
        theme.color("accent").map(|c| c.to_rgba8().to_u8_array()),
        Some([0x4f, 0x7c, 0xff, 0xff])
    );
    assert_eq!(theme.color("font"), None);
    assert_eq!(theme.color("missing"), None);
}
