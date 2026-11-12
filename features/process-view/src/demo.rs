//! The lab's process view scene (feature `demo`): one [`PaintView`] the
//! lab shows both ways — linked, in a `PaintPane`, and from the
//! `process-view-demo` program — so the two can be measured against each
//! other on the same picture.
//!
//! Three thousand particles orbit the middle and a bar sweeps along the
//! top (it moves every frame). The pointer lights the particles near it,
//! a press sends out a ring, the arrow keys move a square, and other keys
//! add a chip along the bottom.

use std::collections::VecDeque;
use std::f64::consts::TAU;
use std::time::Duration;

use architect_workspace::paint::peniko::kurbo::{Affine, Circle, Point, Rect, Stroke};
use architect_workspace::paint::peniko::{Color, Fill};
use architect_workspace::paint::{PaintScene as _, Scene};
use architect_workspace::{FrameStats, PaintView, ViewCtx, ViewInput};

/// Particles in orbit.
pub const PARTICLES: u32 = 3000;

/// How long a press's ring grows.
const RING: Duration = Duration::from_millis(800);

/// What the view measures for whoever hosts it: from an input event
/// reaching the view to the frame recorded after it, on the view's
/// clock. A host passes one in its extras (`Extras::with`).
#[derive(Clone, Default, Debug)]
pub struct Probe(pub FrameStats);

/// The scene.
#[derive(Debug, Default)]
pub struct Orbits {
    pointer: Option<Point>,
    rings: VecDeque<(Point, Duration)>,
    square: Point,
    chips: VecDeque<u32>,
    /// When the oldest input not yet painted reached the view.
    pending: Option<Duration>,
    probe: Option<Probe>,
    focused: bool,
}

impl Orbits {
    #[must_use]
    pub fn new(ctx: &ViewCtx) -> Self {
        Self {
            probe: ctx.extras.get::<Probe>().cloned(),
            square: Point::new(40.0, 60.0),
            ..Self::default()
        }
    }
}

/// The fractional part of `i` times the golden ratio: a spread in 0..1.
fn spread(i: u32, k: f64) -> f64 {
    (f64::from(i) * 0.618_033_988_75 * k).fract()
}

impl PaintView for Orbits {
    fn input(&mut self, ctx: &ViewCtx, event: &ViewInput) -> bool {
        let now = ctx.clock.now();
        match event {
            ViewInput::PointerMove { x, y } => {
                self.pointer = Some(Point::new(f64::from(*x), f64::from(*y)));
            }
            ViewInput::PointerDown { x, y, .. } => {
                let at = Point::new(f64::from(*x), f64::from(*y));
                self.pointer = Some(at);
                self.rings.push_back((at, now));
                if self.rings.len() > 16 {
                    self.rings.pop_front();
                }
            }
            ViewInput::Key {
                key, down: true, ..
            } => match key.as_str() {
                "ArrowLeft" => self.square.x -= 20.0,
                "ArrowRight" => self.square.x += 20.0,
                "ArrowUp" => self.square.y -= 20.0,
                "ArrowDown" => self.square.y += 20.0,
                k => {
                    let code = k.chars().next().map_or(0, u32::from);
                    self.chips.push_back(code);
                    if self.chips.len() > 48 {
                        self.chips.pop_front();
                    }
                }
            },
            ViewInput::Focus(on) => self.focused = *on,
            _ => return false,
        }
        self.pending.get_or_insert(now);
        true
    }

    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
        let now = ctx.clock.now();
        if let (Some(at), Some(probe)) = (self.pending.take(), &self.probe) {
            probe.0.record(now.saturating_sub(at));
        }
        let t = now.as_secs_f64();
        let (w, h) = (
            f64::from(ctx.geometry.width),
            f64::from(ctx.geometry.height),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(14, 14, 20, 255),
            None,
            &Rect::new(0.0, 0.0, w, h),
        );
        // The sweep: moves every frame.
        let x = (t * 0.5).fract() * w;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(255, 122, 26, 255),
            None,
            &Rect::new(x - 40.0, 0.0, x, 4.0),
        );
        let centre = Point::new(w / 2.0, h / 2.0);
        let reach = w.min(h) / 2.0;
        for i in 0..PARTICLES {
            let radius = (0.08 + 0.9 * spread(i, 1.0)) * reach;
            let speed = 0.15 + 0.6 * spread(i, 3.0);
            let angle = spread(i, 7.0).mul_add(TAU, t * speed);
            let p = Point::new(
                radius.mul_add(angle.cos(), centre.x),
                (radius * 0.62).mul_add(angle.sin(), centre.y),
            );
            let near = self.pointer.is_some_and(|q| q.distance(p) < 60.0);
            let (size, color) = if near {
                (3.5, Color::from_rgba8(255, 170, 60, 255))
            } else {
                let shade = 120.0 + 120.0 * spread(i, 11.0);
                let blue = shade.clamp(0.0, 255.0);
                (1.6, Color::from_rgba8(90, 140, u8_of(blue), 255))
            };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new(p, size),
            );
        }
        while self
            .rings
            .front()
            .is_some_and(|(_, at)| now.saturating_sub(*at) > RING)
        {
            self.rings.pop_front();
        }
        for (at, began) in &self.rings {
            let k = now.saturating_sub(*began).as_secs_f64() / RING.as_secs_f64();
            let alpha = u8_of(255.0 * (1.0 - k));
            scene.stroke(
                &Stroke::new(3.0),
                Affine::IDENTITY,
                Color::from_rgba8(250, 250, 250, alpha),
                None,
                &Circle::new(*at, 8.0 + 90.0 * k),
            );
        }
        let square = Rect::from_origin_size(self.square, (28.0, 28.0));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            if self.focused {
                Color::from_rgba8(110, 231, 183, 255)
            } else {
                Color::from_rgba8(82, 82, 91, 255)
            },
            None,
            &square,
        );
        for (i, code) in self.chips.iter().enumerate() {
            let x = 12.0 + 14.0 * f64::from(u32::try_from(i).unwrap_or(0));
            let hue = f64::from(code % 7) / 7.0;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(
                    u8_of(120.0 + 135.0 * hue),
                    120,
                    u8_of(255.0 - 135.0 * hue),
                    255,
                ),
                None,
                &Rect::new(x, h - 22.0, x + 10.0, h - 10.0),
            );
        }
    }

    fn animating(&self, _: &ViewCtx) -> bool {
        true
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
fn u8_of(v: f64) -> u8 {
    v.clamp(0.0, 255.0) as u8
}
