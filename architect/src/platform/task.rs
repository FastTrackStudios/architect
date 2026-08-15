//! Task spawning, racing, and timeouts — the portable execution primitives.
//!
//! [`spawn`] runs a future in the background and hands back an awaitable,
//! abortable [`JoinHandle`]; [`race`] drives two futures and yields whichever
//! finishes first; [`timeout`] bounds a future against the [`Clock`]. All
//! plain `async`, all native↔wasm.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::oneshot;

use super::cancel::CancellationToken;
use super::{Clock, MaybeSend, SystemClock};

// ── race / Either ─────────────────────────────────────────────────────────

/// The outcome of a [`race`] — which side finished first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Either<A, B> {
    /// The first future completed first.
    Left(A),
    /// The second future completed first.
    Right(B),
}

/// Drive two futures concurrently; resolve with the first to complete (the
/// other is dropped, i.e. cancelled). Left-biased: if both are ready on the
/// same poll, [`Either::Left`] wins.
pub async fn race<A, B>(a: A, b: B) -> Either<A::Output, B::Output>
where
    A: Future,
    B: Future,
{
    Race { a, b }.await
}

struct Race<A, B> {
    a: A,
    b: B,
}

impl<A, B> Future for Race<A, B>
where
    A: Future,
    B: Future,
{
    type Output = Either<A::Output, B::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we never move `a`/`b` out of `self`; each is re-pinned in
        // place for the duration of its `poll` call only.
        let this = unsafe { self.get_unchecked_mut() };
        let a = unsafe { Pin::new_unchecked(&mut this.a) };
        if let Poll::Ready(v) = a.poll(cx) {
            return Poll::Ready(Either::Left(v));
        }
        let b = unsafe { Pin::new_unchecked(&mut this.b) };
        if let Poll::Ready(v) = b.poll(cx) {
            return Poll::Ready(Either::Right(v));
        }
        Poll::Pending
    }
}

// ── timeout ─────────────────────────────────────────────────────────────

/// The error from [`timeout`] / [`timeout_with`] — the future didn't finish
/// in time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("operation timed out")
    }
}
impl std::error::Error for Elapsed {}

/// Run `fut`, failing with [`Elapsed`] if it doesn't complete within `dur`
/// (measured on the real [`SystemClock`]).
pub async fn timeout<F: Future>(dur: Duration, fut: F) -> Result<F::Output, Elapsed> {
    timeout_with(&SystemClock, dur, fut).await
}

/// [`timeout`] against an injected [`Clock`] — a
/// [`TestClock`](super::TestClock) makes the deadline deterministic in tests.
pub async fn timeout_with<C: Clock, F: Future>(
    clock: &C,
    dur: Duration,
    fut: F,
) -> Result<F::Output, Elapsed> {
    match race(fut, clock.sleep(dur)).await {
        Either::Left(value) => Ok(value),
        Either::Right(()) => Err(Elapsed),
    }
}

// ── spawn / JoinHandle ────────────────────────────────────────────────────

/// The error from awaiting a [`JoinHandle`] whose task was
/// [`abort`](JoinHandle::abort)ed (or panicked) before producing a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aborted;

impl std::fmt::Display for Aborted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("spawned task was aborted before completion")
    }
}
impl std::error::Error for Aborted {}

/// A handle to a [`spawn`]ed task. `await` it for the result, or
/// [`abort`](JoinHandle::abort) to cancel it cooperatively. Dropping the
/// handle **detaches** the task (it keeps running); only `abort` stops it.
pub struct JoinHandle<T> {
    rx: oneshot::Receiver<T>,
    token: CancellationToken,
}

impl<T> JoinHandle<T> {
    /// Request cancellation. The task stops at its next `.await` point (it's
    /// cooperative — the running future is raced against the abort signal,
    /// not forcibly killed). A subsequent `await` resolves to [`Aborted`].
    pub fn abort(&self) {
        self.token.cancel();
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, Aborted>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `oneshot::Receiver` is `Unpin`.
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(Ok(value)) => Poll::Ready(Ok(value)),
            Poll::Ready(Err(_)) => Poll::Ready(Err(Aborted)), // sender dropped (abort/panic)
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Spawn `fut` as a background task, returning an awaitable/abortable
/// [`JoinHandle`].
///
/// - **native** — [`tokio::task::spawn`] (the caller must be inside a tokio
///   runtime).
/// - **wasm** — `wasm_bindgen_futures::spawn_local`.
///
/// Abort is uniform across both: the future is raced against an internal
/// [`CancellationToken`], so `handle.abort()` drops it at the next await
/// point even on wasm (where the browser executor has no native abort).
pub fn spawn<F, T>(fut: F) -> JoinHandle<T>
where
    F: Future<Output = T> + MaybeSend + 'static,
    T: MaybeSend + 'static,
{
    let (tx, rx) = oneshot::channel();
    let token = CancellationToken::new();
    let task_token = token.clone();
    let driver = async move {
        if let Either::Left(value) = race(fut, task_token.cancelled()).await {
            // Receiver may be gone (handle dropped) — that's a detach, ignore.
            let _ = tx.send(value);
        }
        // Either::Right => aborted: drop `tx`, so the handle resolves `Aborted`.
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::task::spawn(driver);
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(driver);
    }

    JoinHandle { rx, token }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::platform::TestClock;
    use futures_lite::future::{block_on, or, pending};
    use std::future::ready;

    #[test]
    fn race_is_left_biased_when_both_ready() {
        let out = block_on(race(ready(1u32), ready(2u32)));
        assert_eq!(out, Either::Left(1));
    }

    #[test]
    fn race_yields_the_ready_side() {
        let out = block_on(race(pending::<u32>(), ready(9u32)));
        assert_eq!(out, Either::Right(9));
    }

    #[test]
    fn timeout_returns_value_when_fut_completes() {
        let clock = TestClock::new();
        let out = block_on(timeout_with(&clock, Duration::from_secs(5), ready(42u32)));
        assert_eq!(out, Ok(42));
    }

    #[test]
    fn timeout_elapses_when_clock_advances_past_deadline() {
        let clock = TestClock::new();
        let out = block_on(or(
            timeout_with(&clock, Duration::from_secs(5), pending::<u32>()),
            async {
                clock.advance(Duration::from_secs(5));
                pending::<Result<u32, Elapsed>>().await
            },
        ));
        assert_eq!(out, Err(Elapsed));
    }

    #[test]
    fn spawn_join_returns_value() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let handle = spawn(async { 1u32 + 2 });
            assert_eq!(handle.await, Ok(3));
        });
    }

    #[test]
    fn spawn_abort_resolves_aborted() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let handle = spawn(async {
                std::future::pending::<u32>().await // never completes
            });
            handle.abort();
            assert_eq!(handle.await, Err(Aborted));
        });
    }
}
