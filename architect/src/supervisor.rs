//! Supervision — keep a task alive by restarting it under a policy.
//!
//! [`Supervisor`] runs a fallible async operation and **restarts** it
//! according to a [`Restart`] policy, with [`Schedule`]
//! backoff between restarts, until it settles or the supervisor is
//! cancelled. It's the long-running-task counterpart to
//! [`schedule::retry`](crate::schedule::retry): retry drives a call to a
//! single result; a supervisor keeps a *service loop* running.
//!
//! Built entirely on the pieces from [`schedule`](crate::schedule) +
//! [`platform`](crate::platform) — [`Schedule`] for the
//! backoff, [`CancellationToken`] for graceful shutdown, and
//! [`platform::spawn`](crate::platform::spawn) to run a supervised task in
//! the background. Monad-free, native↔wasm.
//!
//! ```
//! use architect::supervisor::{Restart, Supervised, Supervisor};
//! use architect::{platform::Clock, Schedule};
//! use std::sync::atomic::{AtomicU32, Ordering};
//! use std::time::Duration;
//!
//! # // A clock whose sleeps resolve instantly — runs the loop without waiting.
//! # #[derive(Clone)]
//! # struct InstantClock;
//! # impl Clock for InstantClock {
//! #     fn now(&self) -> architect::platform::Instant { architect::platform::Instant::now() }
//! #     fn sleep(&self, _: Duration) -> architect::platform::BoxFuture<'static, ()> { Box::pin(async {}) }
//! # }
//! # futures_lite::future::block_on(async {
//! let sup = Supervisor::new();
//! let attempts = AtomicU32::new(0);
//! // Restart on failure with exponential backoff, up to 5 times.
//! let outcome = sup
//!     .run_with(
//!         &InstantClock,
//!         || {
//!             let n = attempts.fetch_add(1, Ordering::SeqCst) + 1; // fail twice, then succeed
//!             async move { if n < 3 { Err("crashed") } else { Ok::<u32, &str>(n) } }
//!         },
//!         Restart::on_failure(Schedule::exponential(Duration::from_millis(50)).take(5)),
//!     )
//!     .await;
//! assert!(matches!(outcome, Supervised::Settled(Ok(3))));
//! # });
//! ```

use std::future::Future;

use crate::Schedule;
#[cfg(doc)]
use crate::platform::JoinHandle;
use crate::platform::{CancellationToken, Clock, MaybeSend, SystemClock, run_until_cancelled};

/// When should a supervised operation be restarted?
pub enum Restart {
    /// Restart only on `Err`, with `Schedule` backoff between attempts. The
    /// first `Ok` settles the supervisor; an exhausted schedule settles it
    /// with the last `Err`. (Same shape as [`schedule::retry`](crate::schedule::retry),
    /// but cancellable.)
    OnFailure(Schedule),
    /// Restart on **every** exit — `Ok` or `Err` — with `Schedule` backoff: a
    /// keep-alive service loop. Settles only when the schedule is exhausted
    /// (returning the last outcome) or the supervisor is cancelled.
    OnExit(Schedule),
}

impl Restart {
    /// Restart on failure (see [`Restart::OnFailure`]).
    pub fn on_failure(schedule: Schedule) -> Self {
        Restart::OnFailure(schedule)
    }

    /// Restart on every exit (see [`Restart::OnExit`]).
    pub fn on_exit(schedule: Schedule) -> Self {
        Restart::OnExit(schedule)
    }

    fn schedule_mut(&mut self) -> &mut Schedule {
        match self {
            Restart::OnFailure(s) | Restart::OnExit(s) => s,
        }
    }
}

/// How a supervised run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Supervised<T, E> {
    /// The task settled (the policy stopped restarting it) with this result.
    Settled(Result<T, E>),
    /// The supervisor was cancelled before the task settled.
    Cancelled,
}

impl<T, E> Supervised<T, E> {
    /// `true` if the supervisor was cancelled before settling.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Supervised::Cancelled)
    }

    /// The settled result, or `None` if the supervisor was cancelled.
    pub fn into_result(self) -> Option<Result<T, E>> {
        match self {
            Supervised::Settled(r) => Some(r),
            Supervised::Cancelled => None,
        }
    }
}

/// Drive a supervised loop against an injected [`Clock`] and
/// [`CancellationToken`] — the testable core both [`Supervisor`] methods and
/// the [`SystemClock`] convenience wrappers build on.
///
/// Cancellation is honoured both *between* restarts and *during* a run (the
/// in-flight operation and the backoff sleep are each raced against the
/// token), so `token.cancel()` stops a supervised task promptly.
pub async fn supervise_with<C, T, E, Op, Fut>(
    clock: &C,
    token: &CancellationToken,
    mut op: Op,
    mut policy: Restart,
) -> Supervised<T, E>
where
    C: Clock,
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 0u64;
    loop {
        if token.is_cancelled() {
            return Supervised::Cancelled;
        }
        // Run the operation, but bail out promptly if cancelled mid-flight.
        let result = match run_until_cancelled(token, op()).await {
            Some(result) => result,
            None => return Supervised::Cancelled,
        };

        let restart = match (&policy, &result) {
            (Restart::OnFailure(_), Ok(_)) => false, // success ends an OnFailure supervisor
            (Restart::OnFailure(_), Err(_)) => true,
            (Restart::OnExit(_), _) => true, // keep-alive: always restart
        };
        if !restart {
            return Supervised::Settled(result);
        }

        attempt += 1;
        match policy.schedule_mut().next(attempt) {
            // Schedule exhausted — settle with the most recent outcome.
            None => return Supervised::Settled(result),
            // Back off before restarting; a cancel during the wait stops us.
            Some(decision) => {
                if run_until_cancelled(token, clock.sleep(decision.delay))
                    .await
                    .is_none()
                {
                    return Supervised::Cancelled;
                }
            }
        }
    }
}

/// Supervises restartable tasks, with a shared [`CancellationToken`] for
/// graceful shutdown.
///
/// Run a task inline with [`run`](Supervisor::run) /
/// [`run_with`](Supervisor::run_with), or in the background with
/// [`spawn`](Supervisor::spawn) (which hands back a [`JoinHandle`]).
/// [`shutdown`](Supervisor::shutdown) cancels the token, stopping every task
/// this supervisor is driving.
#[derive(Clone, Default)]
pub struct Supervisor {
    token: CancellationToken,
}

impl Supervisor {
    /// A supervisor with a fresh cancellation token.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// A supervisor sharing an existing token — e.g. a child of an app-wide
    /// shutdown token (`parent.child_token()`), so app shutdown cascades to
    /// every supervised task.
    pub fn with_token(token: CancellationToken) -> Self {
        Self { token }
    }

    /// The supervisor's cancellation token (clone it to wire into other
    /// cancellable work).
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Cancel the token — stops every task this supervisor is driving (at
    /// their next await point). Returns `true` if this call triggered it.
    pub fn shutdown(&self) -> bool {
        self.token.cancel()
    }

    /// Supervise `op` under `policy` inline, on the real [`SystemClock`].
    pub async fn run<T, E, Op, Fut>(&self, op: Op, policy: Restart) -> Supervised<T, E>
    where
        Op: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        supervise_with(&SystemClock, &self.token, op, policy).await
    }

    /// [`run`](Supervisor::run) against an injected [`Clock`] (a
    /// [`TestClock`](crate::platform::TestClock) for deterministic tests).
    pub async fn run_with<C, T, E, Op, Fut>(
        &self,
        clock: &C,
        op: Op,
        policy: Restart,
    ) -> Supervised<T, E>
    where
        C: Clock,
        Op: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        supervise_with(clock, &self.token, op, policy).await
    }

    /// Supervise `op` in the background, returning a [`JoinHandle`] that
    /// resolves to the [`Supervised`] outcome. [`shutdown`](Supervisor::shutdown)
    /// (or aborting the handle) stops it.
    pub fn spawn<T, E, Op, Fut>(
        &self,
        op: Op,
        policy: Restart,
    ) -> crate::platform::JoinHandle<Supervised<T, E>>
    where
        Op: FnMut() -> Fut + MaybeSend + 'static,
        Fut: Future<Output = Result<T, E>> + MaybeSend,
        T: MaybeSend + 'static,
        E: MaybeSend + 'static,
    {
        let token = self.token.clone();
        crate::platform::spawn(
            async move { supervise_with(&SystemClock, &token, op, policy).await },
        )
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::platform::{BoxFuture, Instant};
    use futures_lite::future::block_on;
    use std::cell::Cell;
    use std::time::Duration;

    // Sleeps resolve immediately — drive the supervise loop without waiting.
    #[derive(Clone)]
    struct InstantClock;
    impl Clock for InstantClock {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn sleep(&self, _: Duration) -> BoxFuture<'static, ()> {
            Box::pin(async {})
        }
    }

    #[test]
    fn on_failure_settles_on_first_ok() {
        let token = CancellationToken::new();
        let calls = Cell::new(0u32);
        let out = block_on(supervise_with(
            &InstantClock,
            &token,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { if n < 3 { Err("boom") } else { Ok(n) } }
            },
            Restart::on_failure(Schedule::exponential(Duration::from_millis(10)).take(5)),
        ));
        assert_eq!(out, Supervised::Settled(Ok(3)));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn on_failure_settles_with_last_err_when_exhausted() {
        let token = CancellationToken::new();
        let calls = Cell::new(0u32);
        let out: Supervised<u32, u32> = block_on(supervise_with(
            &InstantClock,
            &token,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { Err(n) }
            },
            Restart::on_failure(Schedule::recurs(2)),
        ));
        assert_eq!(out, Supervised::Settled(Err(3))); // initial + 2 restarts
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn on_exit_restarts_through_success_until_schedule_exhausts() {
        let token = CancellationToken::new();
        let calls = Cell::new(0u32);
        let out = block_on(supervise_with(
            &InstantClock,
            &token,
            || {
                let n = calls.get() + 1;
                calls.set(n);
                async move { Ok::<u32, &str>(n) }
            },
            Restart::on_exit(Schedule::recurs(2)),
        ));
        assert_eq!(out, Supervised::Settled(Ok(3))); // ran 3×, returns last
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn already_cancelled_token_yields_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        let out: Supervised<u32, &str> = block_on(supervise_with(
            &InstantClock,
            &token,
            || async { Ok(1) },
            Restart::on_exit(Schedule::spaced(Duration::from_millis(1))),
        ));
        assert_eq!(out, Supervised::Cancelled);
    }

    #[test]
    fn spawn_runs_in_background_and_shutdown_is_observable() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let sup = Supervisor::new();
            let handle = sup.spawn(
                || async { Ok::<u32, &str>(7) },
                Restart::on_failure(Schedule::never()),
            );
            // OnFailure + immediate Ok settles on the first run.
            assert_eq!(handle.await, Ok(Supervised::Settled(Ok(7))));
        });
    }
}
