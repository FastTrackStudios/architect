+++
title = "Scheduling & resilience"
description = "architect::schedule retry/repeat policies and the architect::platform clock they run on."
weight = 59
+++

`architect::schedule` is a small, composable retry/repeat engine — the
Effect `Schedule` idea ported to plain `async`, no `Effect<A,E,R>` monad.
It stands on `architect::platform`, a `Clock` + `spawn` abstraction that
papers over the native (tokio) ↔ wasm (browser timers) split.

Both are opt-in. Enable the `schedule` feature (it pulls `platform`):

```toml
architect = { version = "…", features = ["schedule"] }
```

## The model

A [`Schedule`] is a **value** describing a sequence of delays. It does no
I/O — its one operation, `next(attempt) -> Option<Decision>`, is pure and
attempt-based, so the policy is trivially unit-testable:

```rust
use architect::Schedule;
use std::time::Duration;

// 200ms, 400ms, 800ms … (±20% jitter), capped at 5s, at most 5 retries.
let mut policy = Schedule::exponential(Duration::from_millis(200))
    .max_delay(Duration::from_secs(5))
    .jittered()
    .take(5);

let first = policy.next(1).unwrap().delay; // ~200ms
assert!(first >= Duration::from_millis(160) && first <= Duration::from_millis(240));
```

`Some(decision)` → wait `decision.delay`, then attempt again. `None` →
stop. That's the whole contract.

### Constructors

| Constructor | Behaviour |
| --- | --- |
| `Schedule::never()` | one attempt, no retry |
| `Schedule::recurs(n)` | up to `n` immediate (zero-delay) recurrences |
| `Schedule::spaced(d)` | fixed delay `d`, forever |
| `Schedule::exponential(base)` | `base`, `base·2`, `base·4`, … |
| `Schedule::exponential_factor(base, f)` | `base·fⁿ` |
| `Schedule::linear(base)` | `base`, `base·2`, `base·3`, … |

### Combinators (builder style)

| Combinator | Effect |
| --- | --- |
| `.max_delay(cap)` | clamp every delay to `cap` |
| `.take(n)` | stop after `n` recurrences |
| `.jittered()` / `.jittered_by(frac)` | scale by a deterministic factor in `[1±frac]` (default ±20%) |
| `.map_delay(f)` | transform each delay (the escape hatch for true RNG, floors, …) |
| `.and_then(next)` | run `self` to exhaustion, then `next` (attempts rebased) |
| `.intersect(other)` | recur only while **both** recur; delay = the larger |
| `.union(other)` | recur while **either** recurs; delay = the smaller |

Jitter is **deterministic** (a splitmix64 of the attempt number), so it's
reproducible and needs no `getrandom` — which keeps `Schedule` wasm-clean.
Reach for `.map_delay` when you genuinely want entropy.

## The drivers

`retry` / `repeat` run a plain `async FnMut() -> Result<T, E>` under a
schedule:

- **`retry`** — re-run while it returns `Err`. Returns the first `Ok`, or
  the last `Err` once the schedule is exhausted.
- **`repeat`** — re-run while it returns `Ok`. Returns the last `Ok` once
  exhausted, or the first `Err` that interrupts it.

```rust,ignore
use architect::{schedule, Schedule};
use std::time::Duration;

let client = schedule::retry(
    || async { ExampleRepoClient::establish(&link).await },
    Schedule::exponential(Duration::from_millis(200))
        .max_delay(Duration::from_secs(5))
        .jittered()
        .take(5),
)
.await?;
```

Both default to the real [`SystemClock`]. The `_with` variants
(`retry_with` / `repeat_with`) take a `&Clock`, which is how tests inject a
deterministic one.

## The payoff: resilient clients

This is the direct answer to the "client connect is fragile" thread. The
example app wraps its vox connect in `schedule::retry`
([`examples/app/ui/src/transport.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/ui/src/transport.rs)),
so a just-booting or briefly-flaky server is tolerated instead of failing
the first frame:

```rust,ignore
async fn establish_ws<C: vox_core::FromVoxSession>(url: &str) -> Result<C, String> {
    architect::schedule::retry(
        || establish_ws_once::<C>(url),
        architect::Schedule::exponential(Duration::from_millis(200))
            .max_delay(Duration::from_secs(5))
            .jittered()
            .take(5),
    )
    .await
}
```

Because the policy's clock is platform-portable, the same code is resilient
on **both** targets — `tokio::time` on desktop, browser timers on web.

## The platform layer

`architect::platform` is the foundation `schedule` waits on — and the
portable async-runtime **seam** the rest of the crate sits on. Everything
here is monad-free (plain `async`) and works native↔wasm, so call sites
program against *these* types, not tokio directly. That makes the backing
implementation swappable: an instrumented runtime (moiré) or an alternate
executor can be slotted in without touching consumers.

**Time & tasks**

| Item | Role |
| --- | --- |
| `Clock` (`now` + async `sleep`) | the trait the schedule drivers are generic over |
| `SystemClock` | the real clock — `tokio::time` / browser `setTimeout` |
| `TestClock` | deterministic; time only moves on `advance()` |
| `sleep` / `now` (free fns) | portable wait + monotonic instant |
| `spawn(fut) -> JoinHandle<T>` | background task; `await` the handle for the result, `abort()` to cancel (cooperative, even on wasm) |
| `timeout(dur, fut)` / `timeout_with(&clock, …)` | bound a future against the clock → `Result<T, Elapsed>` (deterministic under `TestClock`) |
| `race(a, b) -> Either` | drive two futures; first to finish wins, the other is dropped |
| `Instant` | re-export of `web_time::Instant` (monotonic on both targets) |

**Cancellation**

| Item | Role |
| --- | --- |
| `CancellationToken` | cooperative stop signal; `cancel()` / `is_cancelled()` / `.cancelled().await` |
| `token.child_token()` | hierarchy — cancelling a parent cancels all descendants; a child cancel stays local |
| `run_until_cancelled(&token, fut)` | run `fut` until it finishes (`Some`) or the token fires (`None`) |

`CancellationToken` is the tool for superseding in-flight work — e.g. a
search-as-you-type that cancels the previous query, or aborting all of a
screen's requests when the user navigates away.

**Concurrency primitives** (wasm-clean; `tokio::sync` + `async-channel`)

| Item | Role |
| --- | --- |
| `Deferred<T>` | write-once value many tasks can await (broadcast one-shot) |
| `Semaphore` / `Permit` | N-permit async gate to bound concurrency |
| `Queue<T>` | bounded/unbounded MPMC channel (`send`/`recv`/`try_*`/`close`) |
| `mpsc` / `broadcast` (re-exports) | point-to-point + fan-out pub/sub, via `architect::platform::{mpsc, broadcast}` |

`mpsc`/`broadcast` are re-exported straight from `tokio::sync` (both
wasm-clean) so consumers reach channels through one common
`architect::platform` import path. Unlike the wrapped primitives, these
expose tokio's types directly — a deliberate trade: tokio's channel API is
rich and well-understood, and moiré's instrumentation already wraps these.

It also centralizes the native↔wasm cfg-split that the rest of the crate
(`resource`, `local`, `axum_ws`) would otherwise each repeat.

One thing deliberately *not* here: real-time-thread handoff. The
platform primitives lock or park, which a real-time (audio) thread can
never do — audio-rate producers use the wait-free `architect::rt`
bridge instead. See the real-time-safety section of
[server push](@/architecture/streams.md).

## Supervision

[`Schedule`] drives a *call* to a result; a [`Supervisor`] keeps a
*service loop* running — restarting a long-lived task under a [`Restart`]
policy, with `Schedule` backoff between restarts, until it settles or the
supervisor is cancelled. It composes the pieces above: `Schedule` for the
backoff, `CancellationToken` for shutdown, `spawn` to run in the background.

```rust,ignore
use architect::{Schedule, Supervisor, supervisor::Restart};

let sup = Supervisor::new();

// Keep a connection/worker loop alive: restart on *any* exit, backing off,
// running in the background. `shutdown()` stops it gracefully.
let handle = sup.spawn(
    || async { run_worker().await },                 // Result<(), Error>
    Restart::on_exit(Schedule::exponential(Duration::from_millis(200))
        .max_delay(Duration::from_secs(30))
        .jittered()),
);
// … later, on app shutdown …
sup.shutdown();                                       // cancels the token
```

Two policies: `Restart::on_failure(schedule)` (retry-shaped — stop on the
first `Ok`, or the last `Err` when the schedule exhausts) and
`Restart::on_exit(schedule)` (keep-alive — restart on every exit). Both are
cancellation-aware: `Supervisor::shutdown()` (or a shared
`token.child_token()`) stops the in-flight run *and* interrupts the backoff
wait. `run`/`run_with` drive it inline (the latter takes a `TestClock` for
deterministic tests); `spawn` returns a [`JoinHandle`] over the
[`Supervised`] outcome (`Settled(result)` or `Cancelled`).

## Testing scheduled code

Inject a `TestClock` and drive time by hand — sleeps resolve the instant
you advance past their deadline, so retry/backoff loops run **instantly and
deterministically**, with zero wall-clock flakiness:

```rust,ignore
use architect::platform::{Clock, TestClock};

let clock = TestClock::new();
let fut = schedule::retry_with(&clock, op, Schedule::exponential(/* … */));
// poll `fut`, then `clock.advance(Duration::from_millis(200))` to release
// the first backoff sleep, and so on — no real time elapses.
```

For policy logic itself you don't even need a clock: `Schedule::next` is
pure, so assert the delay sequence directly (this is how `schedule`'s own
unit tests cover `exponential`, `max_delay`, `intersect`, `and_then`, …).
