//! Schedule layer — composable, testable retry/repeat **policies**.
//!
//! A [`Schedule`] is a value describing a *sequence of delays*: fixed spacing,
//! exponential backoff, jitter, a delay cap, a recurrence limit, and the
//! combinators to glue them together. It carries no I/O of its own — the
//! [`retry`] / [`repeat`] drivers run a plain `async FnMut() -> Result` under a
//! schedule, sleeping the prescribed delay between attempts through a
//! [`platform::Clock`](crate::platform::Clock).
//!
//! This is the Effect `Schedule` idea, ported **without** the `Effect<A,E,R>`
//! monad — the decision is attempt-based and pure
//! ([`next`](Schedule::next)), so the clock is only ever an async sleep
//! provider, and the policy itself is trivially unit-testable.
//!
//! ```
//! use architect::Schedule;
//! use std::time::Duration;
//!
//! // 200ms, 400ms, 800ms (±20% jitter), capped at 5s, at most 5 retries.
//! let mut policy = Schedule::exponential(Duration::from_millis(200))
//!     .max_delay(Duration::from_secs(5))
//!     .jittered()
//!     .take(5);
//!
//! // `next(attempt)` is pure — walk it without any async or wall-clock.
//! let first = policy.next(1).unwrap().delay;
//! assert!(first >= Duration::from_millis(160) && first <= Duration::from_millis(240));
//! ```
//!
//! Driving a real operation (e.g. a flaky RPC connect):
//!
//! ```ignore
//! use architect::{schedule, Schedule};
//! use std::time::Duration;
//!
//! let client = schedule::retry(
//!     || async { ExampleRepoClient::establish(&link).await },
//!     Schedule::exponential(Duration::from_millis(200))
//!         .max_delay(Duration::from_secs(5))
//!         .jittered()
//!         .take(5),
//! )
//! .await?;
//! ```
//!
//! In tests, inject a [`TestClock`](crate::platform::TestClock) via
//! [`retry_with`] / [`repeat_with`] and the delays resolve the instant you
//! advance it — no real waiting, fully deterministic. Driving a fallible op
//! to success, end to end (with a clock whose sleeps resolve immediately):
//!
//! ```
//! use architect::platform::{BoxFuture, Clock, Instant};
//! use architect::{schedule, Schedule};
//! use std::sync::atomic::{AtomicU32, Ordering};
//! use std::time::Duration;
//!
//! // Resolve every sleep instantly — run the policy without real waiting.
//! #[derive(Clone)]
//! struct InstantClock;
//! impl Clock for InstantClock {
//!     fn now(&self) -> Instant {
//!         Instant::now()
//!     }
//!     fn sleep(&self, _: Duration) -> BoxFuture<'static, ()> {
//!         Box::pin(async {})
//!     }
//! }
//!
//! let attempts = AtomicU32::new(0);
//! let out: Result<u32, &str> = futures_lite::future::block_on(schedule::retry_with(
//!     &InstantClock,
//!     || {
//!         let n = attempts.fetch_add(1, Ordering::SeqCst) + 1; // fail twice, then succeed
//!         async move { if n < 3 { Err("server down") } else { Ok(n) } }
//!     },
//!     Schedule::exponential(Duration::from_millis(50)).jittered().take(5),
//! ));
//! assert_eq!(out, Ok(3));
//! assert_eq!(attempts.load(Ordering::SeqCst), 3);
//! ```

use std::time::Duration;

use crate::platform::{Clock, SystemClock};

// A custom delay transform. `MaybeSend`-style cfg-split (mirror
// `resource::Finalizer`): native is `Send + Sync` so a `Schedule` stays
// `Send + Sync` and can cross an `.await` on a multi-thread executor; wasm
// drops the bound.
#[cfg(not(target_arch = "wasm32"))]
type DelayMap = Box<dyn Fn(Duration) -> Duration + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type DelayMap = Box<dyn Fn(Duration) -> Duration>;

/// One step of a [`Schedule`]: wait `delay`, then attempt again. A `None`
/// from [`Schedule::next`] (rather than a `Decision`) means *stop*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// How long to wait before the next attempt.
    pub delay: Duration,
}

/// A retry/repeat policy. Build one with a constructor
/// ([`exponential`](Schedule::exponential), [`spaced`](Schedule::spaced),
/// [`recurs`](Schedule::recurs), …) and refine it with the builder
/// combinators ([`max_delay`](Schedule::max_delay), [`jittered`](Schedule::jittered),
/// [`take`](Schedule::take), [`and_then`](Schedule::and_then), …).
///
/// Faithful to Effect / id_effect's enum model: combinators box their
/// sub-schedules. `Schedule` is **not** `Clone` (it may hold a closure and
/// carries combinator state) — build a fresh one per drive.
pub enum Schedule {
    /// Never recur — one attempt only.
    Never,
    /// Recur up to `n` times, immediately (zero delay). A pure count limiter.
    Recurs(u64),
    /// Always recur with a fixed delay.
    Spaced(Duration),
    /// Always recur; delay = `base * factor.powi(attempt - 1)`.
    Exponential { base: Duration, factor: f64 },
    /// Always recur; delay = `base * attempt`.
    Linear(Duration),
    /// Cap the inner schedule's delay at `cap`.
    MaxDelay(Box<Schedule>, Duration),
    /// Stop after the inner schedule has recurred `n` times.
    Take(Box<Schedule>, u64),
    /// Scale the inner schedule's delay by a deterministic jitter factor in
    /// `[1 - frac, 1 + frac]` (derived purely from the attempt number).
    Jittered(Box<Schedule>, f64),
    /// Transform the inner schedule's delay through a custom function (the
    /// escape hatch for true RNG, deadlines, etc.).
    MapDelay(Box<Schedule>, DelayMap),
    /// Run `first` until it stops, then `second` (attempt count rebased).
    AndThen {
        first: Box<Schedule>,
        second: Box<Schedule>,
        in_second: bool,
        consumed: u64,
    },
    /// Recur only while **both** recur; delay = the larger of the two.
    Intersect(Box<Schedule>, Box<Schedule>),
    /// Recur while **either** recurs; delay = the smaller of the two.
    Union(Box<Schedule>, Box<Schedule>),
}

/// `d * f`, saturating to `Duration::{ZERO, MAX}` instead of panicking on
/// non-finite / overflowing results (exponential backoff blows up fast).
fn mul_saturating(d: Duration, f: f64) -> Duration {
    let secs = d.as_secs_f64() * f;
    if !secs.is_finite() || secs >= Duration::MAX.as_secs_f64() {
        Duration::MAX
    } else if secs <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(secs)
    }
}

/// Deterministic unit float in `[0, 1)` from a seed (splitmix64). Keeps
/// jitter reproducible + wasm-clean (no `getrandom`).
fn jitter_unit(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // top 53 bits → [0, 1)
    (z >> 11) as f64 / ((1u64 << 53) as f64)
}

impl Schedule {
    // ── constructors ────────────────────────────────────────────────────

    /// Never recur — the driven operation runs exactly once.
    pub fn never() -> Self {
        Schedule::Never
    }

    /// Recur up to `n` times with no delay between attempts.
    pub fn recurs(n: u64) -> Self {
        Schedule::Recurs(n)
    }

    /// Recur forever with a fixed `delay` between attempts.
    pub fn spaced(delay: Duration) -> Self {
        Schedule::Spaced(delay)
    }

    /// Exponential backoff: `base`, `base*2`, `base*4`, … (factor 2). Pair
    /// with [`take`](Schedule::take) / [`max_delay`](Schedule::max_delay) to
    /// bound it.
    pub fn exponential(base: Duration) -> Self {
        Schedule::Exponential { base, factor: 2.0 }
    }

    /// Exponential backoff with a custom growth `factor`.
    pub fn exponential_factor(base: Duration, factor: f64) -> Self {
        Schedule::Exponential { base, factor }
    }

    /// Linear backoff: `base`, `base*2`, `base*3`, … (delay grows by `base`
    /// each attempt).
    pub fn linear(base: Duration) -> Self {
        Schedule::Linear(base)
    }

    // ── combinators ──────────────────────────────────────────────────────

    /// Cap every delay at `cap`.
    pub fn max_delay(self, cap: Duration) -> Self {
        Schedule::MaxDelay(Box::new(self), cap)
    }

    /// Stop after at most `n` recurrences (independent of the inner policy).
    pub fn take(self, n: u64) -> Self {
        Schedule::Take(Box::new(self), n)
    }

    /// Add ±20% deterministic jitter to each delay (the common default that
    /// de-synchronizes a thundering herd without going unbounded).
    pub fn jittered(self) -> Self {
        self.jittered_by(0.2)
    }

    /// Add ±`frac` deterministic jitter (e.g. `0.5` → `[0.5×, 1.5×]`).
    pub fn jittered_by(self, frac: f64) -> Self {
        Schedule::Jittered(Box::new(self), frac.abs())
    }

    /// Transform each delay through `f` — the hook for custom logic such as
    /// true RNG jitter or a fixed floor.
    pub fn map_delay<F>(self, f: F) -> Self
    where
        F: Fn(Duration) -> Duration + MaybeSendSync + 'static,
    {
        Schedule::MapDelay(Box::new(self), Box::new(f))
    }

    /// Run `self` to exhaustion, then continue with `next` (its attempt count
    /// rebased to start fresh).
    pub fn and_then(self, next: Schedule) -> Self {
        Schedule::AndThen {
            first: Box::new(self),
            second: Box::new(next),
            in_second: false,
            consumed: 0,
        }
    }

    /// Recur only while **both** `self` and `other` would recur; the delay is
    /// the larger of the two (the more conservative).
    pub fn intersect(self, other: Schedule) -> Self {
        Schedule::Intersect(Box::new(self), Box::new(other))
    }

    /// Recur while **either** `self` or `other` would recur; the delay is the
    /// smaller of the two (the more eager).
    pub fn union(self, other: Schedule) -> Self {
        Schedule::Union(Box::new(self), Box::new(other))
    }

    // ── decision ──────────────────────────────────────────────────────────

    /// The decision after `attempt` attempts have completed (1-based: `1`
    /// after the first run). `Some(decision)` → wait `decision.delay` then
    /// attempt again; `None` → stop.
    pub fn next(&mut self, attempt: u64) -> Option<Decision> {
        match self {
            Schedule::Never => None,
            Schedule::Recurs(n) => (attempt <= *n).then_some(Decision {
                delay: Duration::ZERO,
            }),
            Schedule::Spaced(d) => Some(Decision { delay: *d }),
            Schedule::Exponential { base, factor } => Some(Decision {
                delay: mul_saturating(*base, factor.powi((attempt as i32) - 1)),
            }),
            Schedule::Linear(base) => Some(Decision {
                delay: mul_saturating(*base, attempt as f64),
            }),
            Schedule::MaxDelay(inner, cap) => inner.next(attempt).map(|d| Decision {
                delay: d.delay.min(*cap),
            }),
            Schedule::Take(inner, n) => {
                if attempt > *n {
                    None
                } else {
                    inner.next(attempt)
                }
            }
            Schedule::Jittered(inner, frac) => inner.next(attempt).map(|d| {
                let unit = jitter_unit(attempt); // [0,1)
                let scale = 1.0 - *frac + 2.0 * *frac * unit; // [1-frac, 1+frac)
                Decision {
                    delay: mul_saturating(d.delay, scale),
                }
            }),
            Schedule::MapDelay(inner, f) => {
                inner.next(attempt).map(|d| Decision { delay: f(d.delay) })
            }
            Schedule::AndThen {
                first,
                second,
                in_second,
                consumed,
            } => {
                if !*in_second {
                    if let Some(d) = first.next(attempt) {
                        return Some(d);
                    }
                    // First declined at `attempt` — it accounted for the
                    // prior `attempt - 1` recurrences; hand off to `second`.
                    *in_second = true;
                    *consumed = attempt - 1;
                }
                second.next(attempt - *consumed)
            }
            Schedule::Intersect(a, b) => {
                // Call both (they may carry state) before combining.
                let da = a.next(attempt);
                let db = b.next(attempt);
                match (da, db) {
                    (Some(x), Some(y)) => Some(Decision {
                        delay: x.delay.max(y.delay),
                    }),
                    _ => None,
                }
            }
            Schedule::Union(a, b) => {
                let da = a.next(attempt);
                let db = b.next(attempt);
                match (da, db) {
                    (Some(x), Some(y)) => Some(Decision {
                        delay: x.delay.min(y.delay),
                    }),
                    (Some(x), None) => Some(x),
                    (None, Some(y)) => Some(y),
                    (None, None) => None,
                }
            }
        }
    }
}

// cfg-split marker so `map_delay`'s closure bound reads once for both targets
// (native needs `Send + Sync` to keep `Schedule` shareable across awaits).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSendSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSendSync for T {}

// ── Drivers ────────────────────────────────────────────────────────────

use std::future::Future;

/// Run `op`, retrying while it returns `Err`, under `schedule` on the real
/// [`SystemClock`]. Returns the first `Ok`, or the last `Err` once the
/// schedule is exhausted.
pub async fn retry<T, E, Op, Fut>(op: Op, schedule: Schedule) -> Result<T, E>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    retry_with(&SystemClock, op, schedule).await
}

/// [`retry`] against an injected [`Clock`] — pass a
/// [`TestClock`](crate::platform::TestClock) to drive delays deterministically
/// in tests.
pub async fn retry_with<C, T, E, Op, Fut>(
    clock: &C,
    mut op: Op,
    mut schedule: Schedule,
) -> Result<T, E>
where
    C: Clock,
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 0u64;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                attempt += 1;
                match schedule.next(attempt) {
                    Some(decision) => clock.sleep(decision.delay).await,
                    None => return Err(err),
                }
            }
        }
    }
}

/// Run `op`, repeating while it returns `Ok`, under `schedule` on the real
/// [`SystemClock`]. Returns the last `Ok` once the schedule is exhausted, or
/// the first `Err` that interrupts it.
pub async fn repeat<T, E, Op, Fut>(op: Op, schedule: Schedule) -> Result<T, E>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    repeat_with(&SystemClock, op, schedule).await
}

/// [`repeat`] against an injected [`Clock`].
pub async fn repeat_with<C, T, E, Op, Fut>(
    clock: &C,
    mut op: Op,
    mut schedule: Schedule,
) -> Result<T, E>
where
    C: Clock,
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 0u64;
    let mut last;
    loop {
        match op().await {
            Ok(value) => {
                last = value;
                attempt += 1;
                match schedule.next(attempt) {
                    Some(decision) => clock.sleep(decision.delay).await,
                    None => return Ok(last),
                }
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::platform::{BoxFuture, Instant};
    use futures_lite::future::block_on;
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    // A clock that records requested sleep durations and resolves each one
    // immediately — lets a driver run to completion synchronously while we
    // assert the exact delay sequence.
    #[derive(Clone, Default)]
    struct RecordingClock {
        delays: Arc<Mutex<Vec<Duration>>>,
    }
    impl RecordingClock {
        fn delays(&self) -> Vec<Duration> {
            self.delays.lock().unwrap().clone()
        }
    }
    impl Clock for RecordingClock {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn sleep(&self, dur: Duration) -> BoxFuture<'static, ()> {
            self.delays.lock().unwrap().push(dur);
            Box::pin(async {})
        }
    }

    // ── pure decision sequences ──────────────────────────────────────────

    fn delays_of(mut s: Schedule, attempts: u64) -> Vec<Duration> {
        (1..=attempts)
            .map_while(|a| s.next(a).map(|d| d.delay))
            .collect()
    }

    #[test]
    fn recurs_limits_count_with_zero_delay() {
        assert_eq!(delays_of(Schedule::recurs(3), 10), vec![Duration::ZERO; 3]);
    }

    #[test]
    fn spaced_is_fixed_and_unbounded() {
        let d = Duration::from_millis(50);
        assert_eq!(delays_of(Schedule::spaced(d), 4), vec![d; 4]);
    }

    #[test]
    fn exponential_doubles() {
        let got = delays_of(Schedule::exponential(Duration::from_millis(100)), 4);
        assert_eq!(
            got,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(800),
            ]
        );
    }

    #[test]
    fn linear_grows_by_base() {
        let got = delays_of(Schedule::linear(Duration::from_secs(1)), 3);
        assert_eq!(
            got,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(3),
            ]
        );
    }

    #[test]
    fn max_delay_caps() {
        let got = delays_of(
            Schedule::exponential(Duration::from_millis(100)).max_delay(Duration::from_millis(250)),
            4,
        );
        assert_eq!(
            got,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(250),
                Duration::from_millis(250),
            ]
        );
    }

    #[test]
    fn take_bounds_recurrences() {
        assert_eq!(
            delays_of(Schedule::spaced(Duration::from_millis(10)).take(2), 10).len(),
            2
        );
    }

    #[test]
    fn jittered_stays_within_band_and_is_deterministic() {
        let base = Duration::from_millis(1000);
        let a = delays_of(Schedule::spaced(base).jittered_by(0.2), 8);
        let b = delays_of(Schedule::spaced(base).jittered_by(0.2), 8);
        assert_eq!(a, b, "jitter must be deterministic");
        for d in a {
            assert!(
                d >= Duration::from_millis(800) && d <= Duration::from_millis(1200),
                "{d:?}"
            );
        }
    }

    #[test]
    fn and_then_switches_after_first_exhausts() {
        // recurs(2) [zero,zero] then spaced(7ms) forever.
        let got = delays_of(
            Schedule::recurs(2).and_then(Schedule::spaced(Duration::from_millis(7))),
            5,
        );
        assert_eq!(
            got,
            vec![
                Duration::ZERO,
                Duration::ZERO,
                Duration::from_millis(7),
                Duration::from_millis(7),
                Duration::from_millis(7),
            ]
        );
    }

    #[test]
    fn intersect_stops_with_shorter_and_takes_max_delay() {
        // spaced(100) ∩ take(2 via recurs over spaced) — stops at 2, delay=max.
        let got = delays_of(
            Schedule::spaced(Duration::from_millis(100))
                .intersect(Schedule::spaced(Duration::from_millis(10)).take(2)),
            10,
        );
        assert_eq!(
            got,
            vec![Duration::from_millis(100), Duration::from_millis(100)]
        );
    }

    #[test]
    fn union_continues_with_longer_and_takes_min_delay() {
        let got = delays_of(
            Schedule::spaced(Duration::from_millis(100))
                .take(1)
                .union(Schedule::spaced(Duration::from_millis(30)).take(3)),
            10,
        );
        // attempt1: min(100,30)=30; attempts2-3: only second alive → 30; stops at 3.
        assert_eq!(
            got,
            vec![
                Duration::from_millis(30),
                Duration::from_millis(30),
                Duration::from_millis(30),
            ]
        );
    }

    // ── drivers ───────────────────────────────────────────────────────────

    #[test]
    fn retry_returns_first_ok_and_records_delays() {
        let clock = RecordingClock::default();
        let calls = Cell::new(0u32);
        let out: Result<u32, &str> = block_on(retry_with(
            &clock,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { if n < 3 { Err("boom") } else { Ok(n) } }
            },
            Schedule::exponential(Duration::from_millis(100)),
        ));
        assert_eq!(out, Ok(3));
        assert_eq!(calls.get(), 3);
        // two failures → two sleeps: 100ms, 200ms.
        assert_eq!(
            clock.delays(),
            vec![Duration::from_millis(100), Duration::from_millis(200)]
        );
    }

    #[test]
    fn retry_returns_last_err_when_exhausted() {
        let clock = RecordingClock::default();
        let calls = Cell::new(0u32);
        let out: Result<u32, u32> = block_on(retry_with(
            &clock,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { Err(n) }
            },
            Schedule::recurs(3),
        ));
        assert_eq!(out, Err(4)); // 1 initial + 3 retries, last err carries n=4
        assert_eq!(calls.get(), 4);
        assert_eq!(clock.delays().len(), 3);
    }

    #[test]
    fn repeat_stops_on_first_err() {
        let clock = RecordingClock::default();
        let calls = Cell::new(0u32);
        let out: Result<u32, &str> = block_on(repeat_with(
            &clock,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { if n < 3 { Ok(n) } else { Err("stop") } }
            },
            Schedule::spaced(Duration::from_millis(5)),
        ));
        assert_eq!(out, Err("stop"));
        assert_eq!(calls.get(), 3);
        assert_eq!(clock.delays().len(), 2); // two Oks → two sleeps before the Err
    }

    #[test]
    fn repeat_returns_last_ok_when_schedule_exhausts() {
        let clock = RecordingClock::default();
        let calls = Cell::new(0u32);
        let out: Result<u32, &str> = block_on(repeat_with(
            &clock,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { Ok(n) }
            },
            Schedule::recurs(2),
        ));
        assert_eq!(out, Ok(3)); // ran 3×: initial + 2 repeats
        assert_eq!(clock.delays(), vec![Duration::ZERO, Duration::ZERO]);
    }
}
