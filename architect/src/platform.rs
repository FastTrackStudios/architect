//! Platform layer — the portable async-runtime seam architect builds on.
//!
//! One place for the primitives that otherwise force a native↔wasm cfg-split
//! at every call site: a [`Clock`] (async [`sleep`] + monotonic [`now`]),
//! task [`spawn`] (with an awaitable/abortable [`JoinHandle`]), a [`timeout`]
//! combinator, cooperative [`CancellationToken`]s, and the everyday
//! concurrency primitives [`Deferred`], [`Semaphore`], and [`Queue`].
//!
//! Everything here is **monad-free** (plain `async`) and works on both
//! targets:
//!
//! - **native** — tokio (`tokio::time`, `tokio::task::spawn`, `tokio::sync`).
//! - **wasm** — browser timers (`gloo_timers`), `spawn_local`, and
//!   runtime-agnostic primitives (`tokio::sync` + `async-channel`, neither of
//!   which assumes a tokio reactor).
//!
//! Because call sites program against *these* types rather than tokio
//! directly, the underlying implementation is a swappable seam — an
//! instrumented runtime (moiré) or an alternate executor can be slotted in
//! without touching consumers.
//!
//! [`web_time::Instant`] provides a monotonic clock on both targets.
//!
//! For tests, [`TestClock`] resolves sleeps *manually* via
//! [`advance`](TestClock::advance) — scheduled code runs instantly and
//! deterministically, with zero wall-clock flakiness.
//!
//! ```
//! # #[cfg(not(target_arch = "wasm32"))]
//! # futures_lite::future::block_on(async {
//! use architect::platform::{Clock, TestClock};
//! use std::time::Duration;
//!
//! let clock = TestClock::new();
//! let slept = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
//! let s2 = slept.clone();
//! let c2 = clock.clone();
//! let task = async move {
//!     c2.sleep(Duration::from_secs(10)).await;
//!     s2.store(true, std::sync::atomic::Ordering::SeqCst);
//! };
//! futures_lite::future::or(task, async {
//!     // not yet elapsed
//!     assert!(!slept.load(std::sync::atomic::Ordering::SeqCst));
//!     clock.advance(Duration::from_secs(10)); // wakes the sleep
//!     std::future::pending::<()>().await;
//! })
//! .await;
//! assert!(slept.load(std::sync::atomic::Ordering::SeqCst));
//! # });
//! ```
//!
//! The concurrency primitives at a glance:
//!
//! ```
//! # #[cfg(not(target_arch = "wasm32"))]
//! # futures_lite::future::block_on(async {
//! use architect::platform::{CancellationToken, Deferred, Queue, run_until_cancelled};
//!
//! // A write-once value many tasks can await.
//! let ready = Deferred::<u32>::new();
//! assert!(ready.complete(7));
//! assert_eq!(ready.wait().await, 7);
//!
//! // A bounded MPMC queue.
//! let q = Queue::<&str>::bounded(8);
//! q.send("hi").await.unwrap();
//! assert_eq!(q.recv().await.unwrap(), "hi");
//!
//! // Cooperative cancellation short-circuits a pending future to `None`.
//! let token = CancellationToken::new();
//! token.cancel();
//! let out = run_until_cancelled(&token, std::future::pending::<u32>()).await;
//! assert_eq!(out, None);
//! # });
//! ```

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

// The time types, re-exported so durations + instants come from one place
// alongside the `Clock`. `Duration` is just `std::time::Duration` (with its
// `from_millis`/`from_secs`/… constructors); `Instant` is `web_time::Instant`,
// monotonic on both native and wasm.
pub use std::time::Duration;
pub use web_time::Instant;

// ── Send / wasm cfg-split ───────────────────────────────────────────────
//
// Mirrors `resource::bounds` / `layer::DynHandler`: native futures are
// `+ Send` (tokio multi-thread executors), wasm futures are not. `MaybeSend`
// lets one signature serve both targets.

#[cfg(not(target_arch = "wasm32"))]
mod bounds {
    pub trait MaybeSend: Send {}
    impl<T: Send> MaybeSend for T {}
    pub type BoxFuture<'a, T> =
        std::pin::Pin<Box<dyn core::future::Future<Output = T> + Send + 'a>>;
}
#[cfg(target_arch = "wasm32")]
mod bounds {
    pub trait MaybeSend {}
    impl<T> MaybeSend for T {}
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn core::future::Future<Output = T> + 'a>>;
}

pub use bounds::{BoxFuture, MaybeSend};

// ── Submodules ────────────────────────────────────────────────────────────
//
// The clock lives in this file (it's the foundation); the runtime primitives
// that build on it live alongside.

mod cancel;
mod sync;
mod task;

pub use cancel::{CancellationToken, run_until_cancelled};
pub use sync::{Deferred, Permit, Queue, RecvError, Semaphore, SendError};
pub use task::{Aborted, Either, Elapsed, JoinHandle, race, spawn, timeout, timeout_with};

// Channel + pub/sub surface. Re-exported straight from tokio (both live under
// `tokio/sync`, which is wasm-clean and makes no reactor assumptions) so
// consumers reach point-to-point and broadcast channels through one common
// `architect::platform` import path rather than a direct tokio dependency:
//
//   architect::platform::mpsc::channel(64)        // MPSC point-to-point
//   architect::platform::broadcast::channel(64)   // fan-out pub/sub
//
// (Unlike the wrapped primitives above, these expose tokio's types directly —
// a deliberate trade: tokio's channel API is rich and well-understood, and
// moiré's instrumentation already wraps these same primitives.)
pub use tokio::sync::{broadcast, mpsc};

// ── Free primitives ─────────────────────────────────────────────────────

/// Sleep for `dur`, portably. Native: [`tokio::time::sleep`] (needs a tokio
/// runtime). Wasm: a `setTimeout`-backed `TimeoutFuture`. Sub-millisecond
/// resolution is lost on wasm (the browser timer is millisecond-grained).
pub async fn sleep(dur: Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(dur).await;
    }
    #[cfg(target_arch = "wasm32")]
    {
        // `as u32` saturates anything past ~49 days, which no sane backoff
        // reaches; round up so a sub-ms delay still yields to the event loop.
        let ms = dur.as_millis().min(u128::from(u32::MAX)) as u32;
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
}

/// The current monotonic instant ([`web_time::Instant`] — `std` on native,
/// `performance.now()` on wasm).
pub fn now() -> Instant {
    Instant::now()
}

// ── Clock ────────────────────────────────────────────────────────────────

/// An async clock: read [`now`](Clock::now) and [`sleep`](Clock::sleep).
///
/// Abstracting the clock is what makes scheduled code testable —
/// the `schedule` module's drivers are generic over `Clock`, so a
/// test injects a [`TestClock`] and drives time by hand instead of waiting
/// on the wall clock.
pub trait Clock: Clone + Send + Sync + 'static {
    /// The current monotonic instant.
    fn now(&self) -> Instant;
    /// A future that resolves after `dur` has elapsed *on this clock*.
    fn sleep(&self, dur: Duration) -> BoxFuture<'static, ()>;
}

/// The real clock — [`now`](Clock::now) reads the system monotonic clock and
/// [`sleep`](Clock::sleep) waits real wall-clock time via [`sleep`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, dur: Duration) -> BoxFuture<'static, ()> {
        Box::pin(sleep(dur))
    }
}

// ── TestClock ──────────────────────────────────────────────────────────

/// A deterministic clock whose time only moves when you call
/// [`advance`](TestClock::advance). [`sleep`](Clock::sleep) parks until
/// enough time has been advanced past its deadline — so scheduled code
/// (retries, backoff, repeat-every-N) runs instantly and reproducibly in
/// tests, with no real waiting.
#[derive(Clone)]
pub struct TestClock {
    base: Instant,
    state: Arc<Mutex<TestState>>,
}

struct TestState {
    /// Total time advanced since construction.
    elapsed: Duration,
    /// Pending sleeps: their absolute (elapsed-relative) deadline + waker.
    /// A still-pending sleep re-registers on each poll; matured entries are
    /// dropped by `advance`.
    waiters: Vec<(Duration, Waker)>,
}

impl TestClock {
    /// A fresh clock at elapsed-zero.
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            state: Arc::new(Mutex::new(TestState {
                elapsed: Duration::ZERO,
                waiters: Vec::new(),
            })),
        }
    }

    /// Total time advanced so far.
    pub fn elapsed(&self) -> Duration {
        self.state.lock().unwrap().elapsed
    }

    /// Move time forward by `by`, waking every [`sleep`](Clock::sleep) whose
    /// deadline is now reached.
    pub fn advance(&self, by: Duration) {
        let mut s = self.state.lock().unwrap();
        s.elapsed += by;
        let elapsed = s.elapsed;
        let mut still = Vec::new();
        for (deadline, waker) in s.waiters.drain(..) {
            if deadline <= elapsed {
                waker.wake();
            } else {
                still.push((deadline, waker));
            }
        }
        s.waiters = still;
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.base + self.elapsed()
    }
    fn sleep(&self, dur: Duration) -> BoxFuture<'static, ()> {
        let deadline = self.elapsed() + dur;
        Box::pin(TestSleep {
            clock: self.clone(),
            deadline,
        })
    }
}

struct TestSleep {
    clock: TestClock,
    deadline: Duration,
}

impl Future for TestSleep {
    type Output = ();
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = self.clock.state.lock().unwrap();
        if s.elapsed >= self.deadline {
            Poll::Ready(())
        } else {
            s.waiters.push((self.deadline, cx.waker().clone()));
            Poll::Pending
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use futures_lite::future::{block_on, or};

    #[test]
    fn system_clock_now_is_monotonic() {
        let c = SystemClock;
        let a = c.now();
        let b = c.now();
        assert!(b >= a);
    }

    #[test]
    fn system_clock_sleep_elapses_real_time() {
        // tokio's timer needs a reactor — build a current-thread runtime with
        // the time driver enabled and confirm a short sleep actually waits.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let start = now();
            SystemClock.sleep(Duration::from_millis(5)).await;
            assert!(now() - start >= Duration::from_millis(5));
        });
    }

    #[test]
    fn test_clock_advance_wakes_pending_sleep() {
        block_on(async {
            let clock = TestClock::new();
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let d2 = done.clone();
            let c2 = clock.clone();
            let sleeper = async move {
                c2.sleep(Duration::from_secs(5)).await;
                d2.store(true, std::sync::atomic::Ordering::SeqCst);
            };
            or(sleeper, async {
                assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
                clock.advance(Duration::from_secs(2));
                assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
                clock.advance(Duration::from_secs(3)); // total 5s — wakes it
                std::future::pending::<()>().await;
            })
            .await;
            assert!(done.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(clock.elapsed(), Duration::from_secs(5));
        });
    }

    #[test]
    fn test_clock_now_tracks_elapsed() {
        let clock = TestClock::new();
        let start = clock.now();
        clock.advance(Duration::from_millis(250));
        assert_eq!(clock.now() - start, Duration::from_millis(250));
    }
}
