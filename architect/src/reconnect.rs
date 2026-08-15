//! Connection supervision — death detection + reconnect with backoff.
//!
//! Two pieces:
//!
//! - [`Backoff`] — the reconnect delay policy: exponential growth with
//!   **full jitter** (AWS style: each delay is drawn uniformly from
//!   `[floor, min(cap, floor·2^attempt))`), floor 250ms, cap 10s by
//!   default, reset on success. Pure state machine, unit-testable,
//!   wasm-clean.
//! - [`use_connect_supervised`] — the supervised sibling of
//!   `use_connect_reactive`: establish, then **watch the established
//!   connection for death** and reconnect under a [`Backoff`] when it
//!   dies. The `Connection<C>` it provides flips `Ready → Connecting`
//!   the moment the death-watch resolves, and its
//!   [`generation`](architect_atom::Connection::generation) bumps on
//!   every successful (re-)establish so caches and drive loops can
//!   invalidate.
//!
//! With vox, the death-watch is one line — `Caller::closed()` resolves
//! when the underlying session closes (transport EOF, error, shutdown):
//!
//! ```ignore
//! // app root: survives server restarts, reconnects with backoff.
//! let conn = use_app_supervised(
//!     move || { let slug = active_org(); async move { caller_for(&slug).await } },
//!     |caller: vox::Caller| async move { caller.closed().await },
//! );
//! ```
//!
//! Logging contract: exactly one `warn!` when a live connection is
//! lost, one `warn!` for the *first* failed reconnect attempt of an
//! outage (later attempts log at `debug!`), and one `info!` when the
//! connection (re-)establishes — one line per state change, never one
//! per retry.

use std::time::Duration;

// ── Backoff policy ──────────────────────────────────────────────────────

/// Reconnect delay policy: exponential with full jitter.
///
/// Attempt `n` (0-based since the last [`reset`](Backoff::reset)) draws a
/// delay uniformly from `[floor, min(cap, floor * 2^n))` — the classic
/// "full jitter" scheme that de-synchronizes a thundering herd of
/// reconnecting clients while keeping a hard floor and cap. Defaults:
/// floor 250ms, cap 10s.
///
/// The RNG is a tiny splitmix64 stream seeded from wall-clock entropy
/// (per-instance), so two browsers that died together don't retry in
/// lockstep. Tests use [`Backoff::seeded`] for determinism.
#[derive(Debug, Clone)]
pub struct Backoff {
    floor: Duration,
    cap: Duration,
    attempt: u32,
    rng: u64,
}

impl Default for Backoff {
    /// The reconnect default: floor 250ms, cap 10s.
    fn default() -> Self {
        Self::new(Duration::from_millis(250), Duration::from_secs(10))
    }
}

impl Backoff {
    /// A policy with a custom floor/cap, seeded from wall-clock entropy.
    pub fn new(floor: Duration, cap: Duration) -> Self {
        // Per-instance entropy: sub-second wall-clock nanos (different per
        // client and per construction) mixed with a process-global counter
        // (different per instance within one client). `web_time::SystemTime`
        // works on both native and wasm (Date.now() in the browser).
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let nanos = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
            .unwrap_or(0);
        let seed = nanos ^ NONCE.fetch_add(1, Ordering::Relaxed).rotate_left(32);
        Self::seeded(floor, cap, seed)
    }

    /// A policy with an explicit RNG seed — deterministic, for tests.
    pub fn seeded(floor: Duration, cap: Duration, seed: u64) -> Self {
        Self {
            floor: floor.min(cap),
            cap,
            attempt: 0,
            rng: seed,
        }
    }

    /// Back to attempt zero — call on every successful (re-)establish.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// The next delay: full jitter over the current exponential ceiling,
    /// then advance the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self.ceiling(self.attempt);
        self.attempt = self.attempt.saturating_add(1);
        let span = ceiling.saturating_sub(self.floor);
        if span.is_zero() {
            return self.floor;
        }
        // delay = floor + unit·span, unit ∈ [0, 1)
        let unit = self.next_unit();
        self.floor + Duration::from_secs_f64(span.as_secs_f64() * unit)
    }

    /// `min(cap, floor * 2^attempt)`, saturating.
    fn ceiling(&self, attempt: u32) -> Duration {
        let factor = 2f64.powi(attempt.min(63) as i32);
        let secs = self.floor.as_secs_f64() * factor;
        if !secs.is_finite() || secs >= self.cap.as_secs_f64() {
            self.cap
        } else {
            Duration::from_secs_f64(secs)
        }
    }

    /// splitmix64 step → unit float in `[0, 1)`.
    fn next_unit(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

// ── Supervised connect hooks ────────────────────────────────────────────

#[cfg(feature = "atom")]
mod hooks {
    use std::rc::Rc;

    use architect_atom::dioxus::prelude::*;
    use architect_atom::{Connection, use_connection_root};

    use super::Backoff;
    use crate::platform;

    /// Supervised sibling of `use_connect_reactive`: establish `C`, then
    /// **supervise** it — when `watch_death(c)` resolves, flip the
    /// provided [`Connection`] back to `Connecting` and reconnect under a
    /// [`Backoff`] (exponential, full jitter, floor 250ms, cap 10s, reset
    /// on success). Retries forever; a failed attempt parks the state at
    /// `Failed(e)` (so pages can render the typed connect error) while
    /// the loop keeps going.
    ///
    /// Reactivity contract is the same as `use_connect_reactive`: signals
    /// read **synchronously** by `connect` (org switcher, server URL) are
    /// dependencies — when one changes the whole supervisor restarts with
    /// a fresh state machine. The
    /// [`generation`](architect_atom::Connection::generation) counter
    /// bumps on every transition into `Ready` (initial, reconnect, and
    /// reactive re-trigger alike).
    ///
    /// `watch_death` receives a clone of the established bundle and must
    /// resolve when — and only when — the connection is dead. With vox:
    /// `|caller: vox::Caller| async move { caller.closed().await }`.
    pub fn use_connect_supervised<C, F, Fut, W, WFut>(connect: F, watch_death: W) -> Connection<C>
    where
        C: Clone + 'static,
        F: Fn() -> Fut + 'static,
        Fut: std::future::Future<Output = Result<C, String>> + 'static,
        W: Fn(C) -> WFut + 'static,
        WFut: std::future::Future<Output = ()> + 'static,
    {
        let conn = use_connection_root::<C>();
        let connect = Rc::new(connect);
        let watch_death = Rc::new(watch_death);
        let _resource = use_resource(move || {
            let connect = Rc::clone(&connect);
            let watch_death = Rc::clone(&watch_death);
            // Synchronous part: signal reads here register as
            // dependencies — a change drops this future (and the
            // session it supervises) and restarts the supervisor.
            let first = connect();
            let mut first = Some(first);
            async move {
                let mut backoff = Backoff::default();
                // Log-on-change guards: one line per outage, not per attempt.
                let mut outage_logged = false;
                let mut had_connection = false;
                conn.set_connecting();
                loop {
                    let attempt = match first.take() {
                        Some(f) => f,
                        // Later attempts re-run the closure. Note dioxus
                        // polls resource futures in a reactive context, so
                        // signal reads here are tracked too — that's why
                        // this loop only ever *peeks* the connection's own
                        // signals (a reactive read of a signal it writes
                        // would restart the supervisor on its own writes).
                        None => connect(),
                    };
                    match attempt.await {
                        Ok(c) => {
                            backoff.reset();
                            outage_logged = false;
                            conn.set_ready(c.clone());
                            // Peek, never read: dioxus polls resource
                            // futures in a reactive context, so a reactive
                            // `generation()` here would restart this very
                            // supervisor on its own bump.
                            let generation = conn.generation_now();
                            if had_connection {
                                tracing::info!(generation, "connection re-established");
                            } else {
                                tracing::info!(generation, "connection established");
                            }
                            had_connection = true;
                            // Park here for the connection's lifetime.
                            watch_death(c).await;
                            conn.set_connecting();
                            tracing::warn!(generation, "connection lost; reconnecting");
                        }
                        Err(e) => {
                            if !outage_logged {
                                tracing::warn!("connect failed: {e}; retrying with backoff");
                                outage_logged = true;
                            } else {
                                tracing::debug!("reconnect attempt failed: {e}");
                            }
                            conn.set_failed(e);
                            platform::sleep(backoff.next_delay()).await;
                        }
                    }
                }
            }
        });
        conn
    }
}

#[cfg(feature = "atom")]
pub use hooks::use_connect_supervised;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn delays_stay_within_floor_and_cap() {
        let floor = Duration::from_millis(250);
        let cap = Duration::from_secs(10);
        let mut b = Backoff::seeded(floor, cap, 42);
        for i in 0..50 {
            let d = b.next_delay();
            assert!(d >= floor, "attempt {i}: {d:?} below floor");
            assert!(d <= cap, "attempt {i}: {d:?} above cap");
        }
    }

    #[test]
    fn ceiling_doubles_until_the_cap() {
        let floor = Duration::from_millis(250);
        let cap = Duration::from_secs(10);
        let b = Backoff::seeded(floor, cap, 0);
        assert_eq!(b.ceiling(0), Duration::from_millis(250));
        assert_eq!(b.ceiling(1), Duration::from_millis(500));
        assert_eq!(b.ceiling(2), Duration::from_secs(1));
        assert_eq!(b.ceiling(5), Duration::from_secs(8));
        assert_eq!(b.ceiling(6), cap); // 16s > cap
        assert_eq!(b.ceiling(60), cap);
    }

    #[test]
    fn delay_is_bounded_by_the_attempts_exponential_ceiling() {
        // Full jitter: attempt n draws from [floor, min(cap, floor·2^n)).
        let floor = Duration::from_millis(250);
        let cap = Duration::from_secs(10);
        for seed in 0..20 {
            let mut b = Backoff::seeded(floor, cap, seed);
            for attempt in 0u32..8 {
                let ceiling = b.ceiling(attempt);
                let d = b.next_delay();
                assert!(
                    d <= ceiling,
                    "seed {seed} attempt {attempt}: {d:?} > ceiling {ceiling:?}"
                );
            }
        }
    }

    #[test]
    fn reset_returns_to_the_first_band() {
        let floor = Duration::from_millis(250);
        let cap = Duration::from_secs(10);
        let mut b = Backoff::seeded(floor, cap, 7);
        for _ in 0..6 {
            let _ = b.next_delay();
        }
        b.reset();
        // Post-reset the very next delay is back in the attempt-0 band.
        let d = b.next_delay();
        assert!(d >= floor && d <= b.ceiling(0), "{d:?}");
    }

    #[test]
    fn seeded_policies_are_deterministic_and_seeds_diverge() {
        let floor = Duration::from_millis(250);
        let cap = Duration::from_secs(10);
        let seq = |seed: u64| {
            let mut b = Backoff::seeded(floor, cap, seed);
            (0..10).map(|_| b.next_delay()).collect::<Vec<_>>()
        };
        assert_eq!(seq(1), seq(1), "same seed must replay identically");
        assert_ne!(seq(1), seq(2), "different seeds must jitter differently");
    }

    #[test]
    fn degenerate_floor_equals_cap_is_fixed_delay() {
        let d = Duration::from_millis(500);
        let mut b = Backoff::seeded(d, d, 3);
        for _ in 0..5 {
            assert_eq!(b.next_delay(), d);
        }
    }

    #[test]
    fn fresh_policies_jitter_differently() {
        // Entropy seeding: two same-millisecond constructions must not
        // produce identical delay sequences (the nonce diverges them).
        let mut a = Backoff::default();
        let mut b = Backoff::default();
        let sa: Vec<_> = (0..8).map(|_| a.next_delay()).collect();
        let sb: Vec<_> = (0..8).map(|_| b.next_delay()).collect();
        assert_ne!(sa, sb);
    }
}
