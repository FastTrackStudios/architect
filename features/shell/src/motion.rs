//! Motion (feature `ui`): a value that moves to where it is sent on a
//! spring, coasts after a flick, or is held where a finger puts it, on
//! one clock (`docs/content/architecture/motion.md`).
//!
//! - **One clock.** In a window it is the wall clock, a frame about every
//!   16 ms. Under a [`VirtualClock`] (`architect-guide`'s runner, a test) it
//!   is the runner's: frame *n* is `n / fps` seconds, so a film shows the
//!   same in-between frames on every run.
//! - **Interruptible.** A [`Motion`] sent somewhere new while it moves
//!   keeps its velocity: a drawer flung open and caught again turns,
//!   it does not stop dead. A held one ([`Motion::hold`]) measures the
//!   finger's speed, and the spring or the coast it is let go into starts
//!   from it.
//! - **Exact.** A spring is solved, not stepped, so a slow frame moves it
//!   further, never slower: where it is is a function of the time alone.
//! - **Reduced motion.** With it on ([`set_reduced_motion`], else the OS's
//!   setting where there is one) every motion jumps to its end.
//!
//! A motion writes its value into a signal each frame it moves; the
//! component that reads [`Motion::value`] draws it. Drive what Blitz
//! draws cheaply: a `transform`, an `opacity`, a clipped box's `width`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use dioxus::prelude::*;
use web_time::Instant;

// ── The curves ─────────────────────────────────────────────────────────────

/// A spring, said as a designer says it: how long one swing takes
/// (`response`, seconds) and how much it is held back (`damping`: 1 comes
/// to rest without passing its target, below 1 overshoots).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub response: f64,
    pub damping: f64,
}

impl Spring {
    /// A sidebar, a dock, a split: there in about a quarter of a second,
    /// no overshoot.
    pub const PANEL: Self = Self {
        response: 0.32,
        damping: 1.0,
    };
    /// A tab making room, a dropped tab settling: quicker.
    pub const SNAPPY: Self = Self {
        response: 0.22,
        damping: 1.0,
    };
    /// A highlight tracking the pointer: quicker still.
    pub const TRACK: Self = Self {
        response: 0.16,
        damping: 1.0,
    };

    /// Where a spring that is `offset` from its target moving at
    /// `velocity` is `dt` seconds on: the new offset and velocity.
    #[must_use]
    // The textbook's names: ω, ζ, x₀, v₀, t.
    #[allow(clippy::many_single_char_names)]
    pub fn step(self, offset: f64, velocity: f64, dt: f64) -> (f64, f64) {
        let w = std::f64::consts::TAU / self.response.max(1e-3);
        let z = self.damping.max(0.0);
        let (x0, v0, t) = (offset, velocity, dt.max(0.0));
        if (z - 1.0).abs() < 1e-6 {
            let b = w.mul_add(x0, v0);
            let e = (-w * t).exp();
            (b.mul_add(t, x0) * e, (-w * b).mul_add(t, v0) * e)
        } else if z < 1.0 {
            let wd = w * z.mul_add(-z, 1.0).sqrt();
            let a = (z * w).mul_add(x0, v0) / wd;
            let e = (-z * w * t).exp();
            let (s, c) = (wd * t).sin_cos();
            let x = e * x0.mul_add(c, a * s);
            let v = (z * w).mul_add(-x, e * (wd * a).mul_add(c, -(wd * x0 * s)));
            (x, v)
        } else {
            let root = z.mul_add(z, -1.0).sqrt();
            let (r1, r2) = (-w * (z - root), -w * (z + root));
            let c2 = r1.mul_add(-x0, v0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1.mul_add(e1, c2 * e2), (c1 * r1).mul_add(e1, c2 * r2 * e2))
        }
    }
}

/// How long a coast keeps going: its speed falls by `e` every this many
/// seconds (a trackpad's flick, a scroll view's deceleration).
pub const COAST: f64 = 0.325;

/// Where a coast from `velocity` comes to rest, from where it starts.
#[must_use]
pub fn coast_distance(velocity: f64) -> f64 {
    velocity * COAST
}

// ── The clock ──────────────────────────────────────────────────────────────

/// About a display frame: how often a motion moves on the wall clock.
pub const FRAME: Duration = Duration::from_micros(16_667);

struct ClockState {
    /// Where the wall clock started (its zero).
    wall: Instant,
    /// The virtual clock's time, while there is one.
    virtual_now: Option<f64>,
    /// The virtual clock's frame, bumped by each [`VirtualClock::advance_to`].
    frame: u64,
    /// What waits for the virtual clock's next frame.
    waiting: Vec<Waker>,
}

thread_local! {
    static CLOCK: RefCell<ClockState> = RefCell::new(ClockState {
        wall: Instant::now(),
        virtual_now: None,
        frame: 0,
        waiting: Vec::new(),
    });
}

/// The clock's time, in seconds: the virtual clock's while there is one
/// on this thread, else the wall clock's since it was first read.
#[must_use]
pub fn now() -> f64 {
    CLOCK.with(|c| {
        let c = c.borrow();
        c.virtual_now
            .unwrap_or_else(|| c.wall.elapsed().as_secs_f64())
    })
}

/// Whether this thread runs on a [`VirtualClock`].
#[must_use]
pub fn is_virtual() -> bool {
    CLOCK.with(|c| c.borrow().virtual_now.is_some())
}

/// The clock of a film or a test: time moves only when it is told to.
/// While one is alive every motion on this thread (every document the
/// runner opens) moves on it; dropped, the wall clock is back.
pub struct VirtualClock {
    /// Not `Send`: the clock is the thread's.
    _thread: std::marker::PhantomData<Rc<()>>,
}

impl VirtualClock {
    /// Starts the virtual clock at 0.
    #[must_use]
    pub fn start() -> Self {
        CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            c.virtual_now = Some(0.0);
            c.frame = 0;
        });
        Self {
            _thread: std::marker::PhantomData,
        }
    }

    /// Moves the clock to `seconds` and wakes every motion waiting for its
    /// next frame. Poll the documents after, then lay them out.
    #[allow(clippy::unused_self)]
    pub fn advance_to(&self, seconds: f64) {
        let waiting = CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            c.virtual_now = Some(seconds);
            c.frame = c.frame.wrapping_add(1);
            std::mem::take(&mut c.waiting)
        });
        for w in waiting {
            w.wake();
        }
    }
}

impl Drop for VirtualClock {
    fn drop(&mut self) {
        let waiting = CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            c.virtual_now = None;
            std::mem::take(&mut c.waiting)
        });
        for w in waiting {
            w.wake();
        }
    }
}

/// The next frame: the virtual clock's next, or about [`FRAME`] on.
/// Every motion waiting wakes together (one redraw for them all).
#[must_use]
pub fn next_frame() -> NextFrame {
    let frame = CLOCK.with(|c| c.borrow().frame);
    if is_virtual() {
        NextFrame::Virtual(frame)
    } else {
        // To the next frame boundary of the wall clock, so motions started
        // apart still move in the same frames.
        let at = now();
        let frame_s = FRAME.as_secs_f64();
        let next = ((at / frame_s).floor() + 1.0) * frame_s;
        let wait = Duration::from_secs_f64((next - at).max(0.001));
        NextFrame::Wall(futures_timer::Delay::new(wait))
    }
}

/// What [`next_frame`] waits on.
pub enum NextFrame {
    Virtual(u64),
    Wall(futures_timer::Delay),
}

impl Future for NextFrame {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.get_mut() {
            Self::Virtual(from) => CLOCK.with(|c| {
                let mut c = c.borrow_mut();
                if c.frame != *from || c.virtual_now.is_none() {
                    Poll::Ready(())
                } else {
                    c.waiting.push(cx.waker().clone());
                    Poll::Pending
                }
            }),
            Self::Wall(delay) => Pin::new(delay).poll(cx),
        }
    }
}

// ── Reduced motion ─────────────────────────────────────────────────────────

/// 0: as the OS says; 1: full motion; 2: reduced.
static REDUCED: AtomicU8 = AtomicU8::new(0);

/// Turns reduced motion on or off for the app (its setting), or back to
/// the OS's preference with `None`.
pub fn set_reduced_motion(reduced: Option<bool>) {
    let v = match reduced {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    REDUCED.store(v, Ordering::Relaxed);
}

/// Whether motion is reduced: the app's setting, else the OS's (macOS's
/// "Reduce motion"). A virtual clock reads only the app's, so a film is the
/// same on every machine.
#[must_use]
pub fn reduced_motion() -> bool {
    match REDUCED.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => !is_virtual() && os_reduced(),
    }
}

#[cfg(target_os = "macos")]
fn os_reduced() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
const fn os_reduced() -> bool {
    false
}

// ── A motion ───────────────────────────────────────────────────────────────

/// Where a motion is headed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Goal {
    /// Where it is (held, or settled).
    Still,
    /// On a spring to a target.
    To(f64),
    /// Coasting, kept between `min` and `max` (a spring takes it back
    /// inside when it reaches one).
    Coast { min: f64, max: f64 },
}

/// A motion's state: where it is, how fast, where to. Plain data, so it is
/// tested without a renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub at: f64,
    pub velocity: f64,
    goal: Goal,
    spring: Spring,
    /// Close enough to call it there, in the value's units.
    rest: f64,
    /// The clock when it was last moved on.
    last: f64,
    /// The last hold: when, and where (for the finger's speed).
    held: Option<(f64, f64)>,
}

impl Track {
    #[must_use]
    pub const fn new(at: f64, spring: Spring, rest: f64) -> Self {
        Self {
            at,
            velocity: 0.0,
            goal: Goal::Still,
            spring,
            rest,
            last: 0.0,
            held: None,
        }
    }

    /// Where it is headed: its target, or where a coast comes to rest.
    #[must_use]
    pub fn target(&self) -> f64 {
        match self.goal {
            Goal::Still => self.at,
            Goal::To(to) => to,
            Goal::Coast { min, max } => (self.at + coast_distance(self.velocity)).clamp(min, max),
        }
    }

    /// Whether it is moving on its own.
    #[must_use]
    pub fn moving(&self) -> bool {
        self.goal != Goal::Still
    }

    /// Sends it to `to` at `time`, keeping its velocity.
    pub fn to(&mut self, to: f64, time: f64) {
        self.advance(time);
        self.held = None;
        self.goal = Goal::To(to);
        self.last = time;
    }

    /// Sends it coasting at `time` with the velocity it has, kept within
    /// `min..=max`.
    pub fn coast(&mut self, min: f64, max: f64, time: f64) {
        self.advance(time);
        self.held = None;
        self.goal = Goal::Coast { min, max };
        self.last = time;
    }

    /// Puts it at `at` at `time`, measuring how fast it was moved there.
    pub fn hold(&mut self, at: f64, time: f64) {
        if let Some((t0, x0)) = self.held {
            let dt = time - t0;
            if dt > 1e-4 {
                // The finger's speed, smoothed over its last few moves.
                let v = (at - x0) / dt;
                self.velocity = self.velocity.mul_add(0.4, v * 0.6);
            }
        } else {
            self.velocity = 0.0;
        }
        self.at = at;
        self.goal = Goal::Still;
        self.held = Some((time, at));
        self.last = time;
    }

    /// Puts it at `at`, still.
    pub const fn jump(&mut self, at: f64, time: f64) {
        self.at = at;
        self.velocity = 0.0;
        self.goal = Goal::Still;
        self.held = None;
        self.last = time;
    }

    /// Moves it on to `time`. True once it has come to rest.
    pub fn advance(&mut self, time: f64) -> bool {
        let dt = time - self.last;
        self.last = time;
        match self.goal {
            Goal::Still => return true,
            Goal::To(to) => {
                let (x, v) = self.spring.step(self.at - to, self.velocity, dt);
                self.at = to + x;
                self.velocity = v;
                if x.abs() < self.rest && v.abs() < self.rest * 10.0 {
                    self.jump(to, time);
                    return true;
                }
            }
            Goal::Coast { min, max } => {
                let e = (-dt / COAST).exp();
                let x = self.velocity.mul_add(COAST * (1.0 - e), self.at);
                self.velocity *= e;
                self.at = x;
                if x <= min || x >= max {
                    // Past an end: a spring takes it back to it.
                    self.goal = Goal::To(x.clamp(min, max));
                } else if self.velocity.abs() < self.rest * 10.0 {
                    self.goal = Goal::Still;
                    self.velocity = 0.0;
                    return true;
                }
            }
        }
        false
    }
}

/// A value that moves: read it in a render with [`Motion::value`], send
/// it with [`Motion::to`], hold it with [`Motion::hold`]. Copy it into any
/// handler; its frames run in the component that made it.
#[derive(Clone, Copy)]
pub struct Motion {
    value: Signal<f64>,
    track: CopyValue<Track>,
    /// A frame loop is running.
    running: CopyValue<bool>,
    /// Called once it comes to rest where it was sent (replaced by the
    /// next send, dropped by a hold).
    done: CopyValue<Option<Rc<RefCell<dyn FnMut()>>>>,
    owner: CopyValue<ScopeId>,
}

impl PartialEq for Motion {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

/// A motion starting at `at`, on `spring`, at rest within `rest` of its
/// target (in the value's units: 0.001 of a 0–1 amount, half a point).
#[must_use]
pub fn use_motion(at: f64, spring: Spring, rest: f64) -> Motion {
    use_hook(|| Motion::new(at, spring, rest))
}

impl Motion {
    /// A motion owned by the current component (for a hook of your own;
    /// [`use_motion`] is the usual way).
    #[must_use]
    pub fn new(at: f64, spring: Spring, rest: f64) -> Self {
        Self {
            value: Signal::new(at),
            track: CopyValue::new(Track::new(at, spring, rest)),
            running: CopyValue::new(false),
            done: CopyValue::new(None),
            owner: CopyValue::new(dioxus::core::current_scope_id()),
        }
    }

    /// Where it is now (a render that reads it redraws as it moves).
    #[must_use]
    pub fn value(self) -> f64 {
        (self.value)()
    }

    /// Where it is now, without subscribing.
    #[must_use]
    pub fn peek(self) -> f64 {
        *self.value.peek()
    }

    /// Where it is headed.
    #[must_use]
    pub fn target(self) -> f64 {
        self.track.peek().target()
    }

    /// Whether it is moving on its own.
    #[must_use]
    pub fn moving(self) -> bool {
        self.track.peek().moving()
    }

    /// Its speed, in its units a second.
    #[must_use]
    pub fn velocity(self) -> f64 {
        self.track.peek().velocity
    }

    /// Changes the spring for what it is sent to next.
    pub fn set_spring(mut self, spring: Spring) {
        self.track.write().spring = spring;
    }

    /// Sends it to `to` on its spring, keeping the velocity it has.
    pub fn to(self, to: f64) {
        self.send(to, None);
    }

    /// [`Motion::to`], then `done` once it is there (not if it is sent
    /// elsewhere first).
    pub fn to_then(self, to: f64, done: impl FnMut() + 'static) {
        self.send(to, Some(Rc::new(RefCell::new(done))));
    }

    fn send(mut self, to: f64, done: Option<Rc<RefCell<dyn FnMut()>>>) {
        let mut d = self.done;
        d.set(done);
        if reduced_motion() {
            self.jump(to);
            self.finish();
            return;
        }
        let same = { *self.track.peek() };
        if !same.moving() && (same.at - to).abs() < same.rest {
            self.jump(to);
            self.finish();
            return;
        }
        self.track.write().to(to, now());
        self.run();
    }

    /// Lets it coast from the speed it was held at, kept within
    /// `min..=max` (a scroll flicked).
    pub fn coast(mut self, min: f64, max: f64) {
        self.forget();
        if reduced_motion() {
            let to = self.track.peek().at.clamp(min, max);
            self.jump(to);
            return;
        }
        self.track.write().coast(min, max, now());
        self.run();
    }

    /// Puts it at `at` and keeps it there (a finger has it), measuring the
    /// finger's speed for a [`Motion::to`] or [`Motion::coast`] after.
    pub fn hold(mut self, at: f64) {
        self.forget();
        self.track.write().hold(at, now());
        self.show(at);
    }

    /// Puts it at `at`, still: no motion, no speed.
    pub fn jump(mut self, at: f64) {
        self.track.write().jump(at, now());
        self.show(at);
    }

    /// Moves it by `by` at once, still headed where it was (a layout
    /// change under it: what moves is the distance left to go).
    pub fn shift(mut self, by: f64) {
        let mut track = self.track.write();
        track.at += by;
        let at = track.at;
        drop(track);
        self.show(at);
    }

    /// Drops what was to run when the last send got there.
    fn forget(self) {
        let mut d = self.done;
        if d.peek().is_some() {
            d.set(None);
        }
    }

    fn show(self, at: f64) {
        let mut value = self.value;
        if value.try_peek().is_ok_and(|v| (*v - at).abs() > f64::EPSILON) {
            value.set(at);
        }
    }

    fn finish(self) {
        let mut d = self.done;
        let done = d.write().take();
        if let Some(done) = done {
            (done.borrow_mut())();
        }
    }

    /// Starts its frames, unless they are running.
    fn run(mut self) {
        if *self.running.peek() {
            return;
        }
        let mut running = self.running;
        running.set(true);
        let owner = *self.owner.peek();
        let frames = async move {
            loop {
                next_frame().await;
                let Ok(mut track) = self.track.try_write() else {
                    return;
                };
                let rested = track.advance(now());
                let at = track.at;
                drop(track);
                self.show(at);
                if rested {
                    let mut running = self.running;
                    running.set(false);
                    self.finish();
                    return;
                }
            }
        };
        dioxus::core::Runtime::current().spawn(owner, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::{Spring, Track, coast_distance};

    /// Moves `track` on `n` 60 fps frames, its place each frame.
    fn frames(track: &mut Track, from: f64, n: u32) -> Vec<f64> {
        (1..=n)
            .map(|i| {
                track.advance(from + f64::from(i) / 60.0);
                track.at
            })
            .collect()
    }

    #[test]
    fn a_panel_spring_eases_out_without_overshoot() {
        let mut t = Track::new(0.0, Spring::PANEL, 0.001);
        t.to(1.0, 0.0);
        let at = frames(&mut t, 0.0, 36);
        // Fast at first, slower each frame, never past the end.
        let steps: Vec<f64> = at.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.first().copied().unwrap_or(0.0) > 0.03);
        assert!(at.iter().all(|x| *x <= 1.0 + 1e-9));
        // Most of the way in a quarter second, settled by 0.6.
        assert!(at[14] > 0.9, "at 0.25 s it was {}", at[14]);
        assert!(!t.moving());
        assert!((t.at - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_spring_is_the_same_however_the_frames_fall() {
        let mut a = Track::new(0.0, Spring::SNAPPY, 0.0001);
        let mut b = a;
        a.to(100.0, 0.0);
        b.to(100.0, 0.0);
        a.advance(0.1);
        for i in 1..=12 {
            b.advance(f64::from(i) * 0.1 / 12.0);
        }
        assert!((a.at - b.at).abs() < 1e-6, "{} vs {}", a.at, b.at);
    }

    #[test]
    fn sent_elsewhere_midway_it_keeps_its_velocity() {
        let mut t = Track::new(0.0, Spring::PANEL, 0.001);
        t.to(1.0, 0.0);
        t.advance(0.08);
        let (at, v) = (t.at, t.velocity);
        assert!(v > 0.0);
        // Turned back: it carries on forward a little before it turns.
        t.to(0.0, 0.08);
        assert!((t.velocity - v).abs() < f64::EPSILON);
        t.advance(0.09);
        assert!(t.at > at);
    }

    #[test]
    fn a_held_value_lets_go_at_the_fingers_speed() {
        // Points: at rest within half of one.
        let mut t = Track::new(0.0, Spring::PANEL, 0.5);
        for i in 0..6 {
            t.hold(f64::from(i) * 10.0, f64::from(i) / 60.0);
        }
        // 10 a frame: 600 a second.
        assert!((t.velocity - 600.0).abs() < 20.0, "{}", t.velocity);
        t.coast(0.0, 1000.0, 5.0 / 60.0);
        let rest = t.target();
        assert!((rest - (50.0 + coast_distance(t.velocity))).abs() < 1e-9);
        frames(&mut t, 5.0 / 60.0, 180);
        assert!(!t.moving());
        assert!(t.at > 200.0 && t.at < 300.0, "{}", t.at);
    }

    #[test]
    fn a_coast_past_an_end_springs_back_to_it() {
        let mut t = Track::new(90.0, Spring::SNAPPY, 0.01);
        t.velocity = 2000.0;
        t.coast(0.0, 100.0, 0.0);
        let at = frames(&mut t, 0.0, 120);
        assert!(at.iter().any(|x| *x > 100.0));
        assert!((t.at - 100.0).abs() < 0.02);
        assert!(!t.moving());
    }

    #[test]
    fn an_underdamped_spring_overshoots_and_settles() {
        let s = Spring {
            response: 0.3,
            damping: 0.5,
        };
        let mut t = Track::new(0.0, s, 0.001);
        t.to(1.0, 0.0);
        let at = frames(&mut t, 0.0, 120);
        assert!(at.iter().any(|x| *x > 1.05));
        assert!((t.at - 1.0).abs() < 0.002);
        let over = Spring {
            response: 0.3,
            damping: 1.6,
        };
        let mut t = Track::new(0.0, over, 0.001);
        t.to(1.0, 0.0);
        let at = frames(&mut t, 0.0, 180);
        assert!(at.iter().all(|x| *x <= 1.0 + 1e-9));
        assert!((t.at - 1.0).abs() < 0.002);
    }
}
