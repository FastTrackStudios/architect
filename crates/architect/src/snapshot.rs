//! Write gate and WAL checkpoint for consistent on-disk snapshots.
//!
//! A server that copies or commits its own sqlite files (a git snapshot,
//! a backup, a file sync) needs a moment where no new write starts and
//! the WAL tail is folded into the main db file. This module is that
//! moment, without the copy step: what happens to the files (git commit,
//! tarball, rsync) stays in the app.
//!
//! - [`WriteGate`]: a global door. Request handlers call
//!   [`WriteGate::wait_open`] at entry; a snapshot holds
//!   [`WriteGate::pause`] and new requests park until it drops.
//! - [`GatedLayer`]: a vox handler wrapper that parks every call at the
//!   gate before dispatching. [`LayerRouter::gated`](crate::LayerRouter::gated)
//!   wraps every handler of a built router in one call.
//! - [`quiesce`]: pause, run the `checkpoint` callback, run the app's
//!   step, release. With `server-seaorm`, [`wal_checkpoint`] is the
//!   stock sqlite callback (`PRAGMA wal_checkpoint(TRUNCATE)`).
//!
//! # The maximum hold
//!
//! A gate hold is bounded by [`WriteGate::max_hold`] (default
//! [`DEFAULT_MAX_HOLD`], 30 s). Two things enforce it:
//!
//! 1. a parked request waits at most until the hold's deadline; past it
//!    [`WriteGate::wait_open`] returns [`Admission::Expired`] and the
//!    request proceeds (with a warning), so a stuck snapshot can never
//!    stall the write path for longer than the maximum;
//! 2. [`quiesce`] cancels the checkpoint + step at the deadline and
//!    returns [`GateTimeout`], so a cycle never reports a "consistent"
//!    snapshot taken after writes resumed.
//!
//! Keep slow, network-bound work (a `git push`) **after** the gate: commits
//! are immutable, so only the local commit needs the quiesce.
//!
//! # What the gate does not do
//!
//! The gate closes the door; it does not drain the room. Requests already
//! past [`WriteGate::wait_open`] when the gate closes finish normally, and
//! the gate is never held across a request body (a long-lived subscribe
//! stream would otherwise pin it forever). sqlite's own locking serializes
//! those stragglers against the checkpoint.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, OwnedMutexGuard, watch};
use tokio::time::Instant;
use vox::{DriverReplySink, Handler, MethodId, RequestCall, SchemaRecvTracker, SelfRef};

use crate::layer::DynHandler;

/// The default bound on one gate hold: 30 seconds.
///
/// A local `git commit` of a few sqlite files takes well under a second;
/// 30 s leaves room for a slow disk while keeping a stuck cycle from
/// looking like an outage. Override with [`WriteGate::with_max_hold`].
pub const DEFAULT_MAX_HOLD: Duration = Duration::from_secs(30);

// ── Write gate ────────────────────────────────────────────────────

/// Global write quiesce for snapshot cycles. Cheap to clone (one `Arc`).
///
/// [`wait_open`](Self::wait_open) is a lock-free read of a
/// `watch` channel when no snapshot runs. While a [`GateHold`] is alive,
/// new callers park until it drops or its deadline passes.
#[derive(Clone)]
pub struct WriteGate {
    inner: Arc<Inner>,
}

struct Inner {
    /// `Some(deadline)` while a hold is alive, `None` when open.
    state: watch::Sender<Option<Instant>>,
    /// Serializes holders: one pause at a time.
    holder: Arc<Mutex<()>>,
    max_hold: Duration,
}

impl Default for WriteGate {
    fn default() -> Self {
        Self::with_max_hold(DEFAULT_MAX_HOLD)
    }
}

impl std::fmt::Debug for WriteGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteGate")
            .field("open", &self.is_open())
            .field("max_hold", &self.inner.max_hold)
            .finish()
    }
}

/// How a caller got through [`WriteGate::wait_open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The gate was open, or the hold was released.
    Open,
    /// The hold outlived [`WriteGate::max_hold`]; the caller proceeds
    /// anyway rather than stall the write path.
    Expired,
}

impl WriteGate {
    /// An open gate with the [`DEFAULT_MAX_HOLD`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An open gate whose holds last at most `max_hold`.
    #[must_use]
    pub fn with_max_hold(max_hold: Duration) -> Self {
        let (state, _) = watch::channel(None);
        Self {
            inner: Arc::new(Inner {
                state,
                holder: Arc::new(Mutex::new(())),
                max_hold,
            }),
        }
    }

    /// The bound on one hold.
    #[must_use]
    pub fn max_hold(&self) -> Duration {
        self.inner.max_hold
    }

    /// True when no hold is alive (or the live one has expired).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner
            .state
            .borrow()
            .is_none_or(|deadline| Instant::now() >= deadline)
    }

    /// Park until no snapshot holds the gate, or until the hold's
    /// deadline. Holds nothing on return.
    pub async fn wait_open(&self) -> Admission {
        let mut rx = self.inner.state.subscribe();
        loop {
            let Some(deadline) = *rx.borrow_and_update() else {
                return Admission::Open;
            };
            if Instant::now() >= deadline {
                return expired();
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                // State moved (released, or a new hold): look again.
                Ok(Ok(())) => {}
                // The sender lives in `Inner`, which `self` keeps alive,
                // so this arm is unreachable; treat it as open.
                Ok(Err(_)) => return Admission::Open,
                Err(_) => return expired(),
            }
        }
    }

    /// Close the gate. New callers of [`wait_open`](Self::wait_open)
    /// park until the returned hold drops or [`max_hold`](Self::max_hold)
    /// passes. A second `pause` waits for the first hold to drop.
    pub async fn pause(&self) -> GateHold {
        let holder = Arc::clone(&self.inner.holder).lock_owned().await;
        let deadline = deadline_after(self.inner.max_hold);
        self.inner.state.send_replace(Some(deadline));
        GateHold {
            gate: self.clone(),
            deadline,
            _holder: holder,
        }
    }
}

/// `now + max_hold`, saturating at ~136 years out for absurd durations
/// (`Duration::MAX` overflows `Instant`).
fn deadline_after(max_hold: Duration) -> Instant {
    let far = Duration::from_secs(u32::MAX.into());
    let now = Instant::now();
    now.checked_add(max_hold)
        .or_else(|| now.checked_add(far))
        .unwrap_or(now)
}

fn expired() -> Admission {
    tracing::warn!("write gate hold passed its maximum; admitting the request");
    Admission::Expired
}

/// A live gate hold. Dropping it reopens the gate.
#[must_use = "the gate reopens as soon as the hold is dropped"]
pub struct GateHold {
    gate: WriteGate,
    deadline: Instant,
    _holder: OwnedMutexGuard<()>,
}

impl GateHold {
    /// When waiting callers stop waiting.
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// True once the hold has outlived its maximum (callers are being
    /// admitted again).
    #[must_use]
    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

impl Drop for GateHold {
    fn drop(&mut self) {
        self.gate.inner.state.send_replace(None);
    }
}

impl std::fmt::Debug for GateHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateHold")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

// ── Gated layer ───────────────────────────────────────────────────

/// A vox handler that parks every incoming call at a [`WriteGate`]
/// before dispatching to the wrapped handler.
///
/// Entry gate only: the gate is never held across the call body, so
/// streaming and long-lived calls cannot pin it. Shape queries
/// (`args_have_channels`, `response_wire_shape`) pass straight through.
#[derive(Clone)]
pub struct GatedLayer {
    inner: Arc<dyn DynHandler>,
    gate: WriteGate,
}

impl GatedLayer {
    /// Wrap any vox handler (a [`LayerRouter`](crate::LayerRouter), a
    /// generated service dispatcher, ...).
    #[must_use]
    pub fn new<H>(inner: H, gate: WriteGate) -> Self
    where
        H: Handler<DriverReplySink> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(inner),
            gate,
        }
    }

    pub(crate) fn from_dyn(inner: Arc<dyn DynHandler>, gate: WriteGate) -> Self {
        Self { inner, gate }
    }

    /// The gate this layer waits on.
    #[must_use]
    pub const fn gate(&self) -> &WriteGate {
        &self.gate
    }
}

impl Handler<DriverReplySink> for GatedLayer {
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        self.inner.args_have_channels(method_id)
    }

    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        self.inner.response_wire_shape(method_id)
    }

    async fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) {
        let _admission = self.gate.wait_open().await;
        self.inner.handle(call, reply, schemas).await;
    }
}

// ── Quiesce cycle ─────────────────────────────────────────────────

/// The checkpoint + step outlived the gate's maximum hold and was
/// cancelled. Writes resumed at the deadline, so nothing the step was
/// producing can be trusted as a consistent snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateTimeout {
    /// The bound that was exceeded.
    pub max_hold: Duration,
}

impl std::fmt::Display for GateTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "snapshot step exceeded the write gate's maximum hold ({:?}) and was cancelled",
            self.max_hold
        )
    }
}

impl std::error::Error for GateTimeout {}

/// One quiesced cycle: close `gate`, run `checkpoint` (fold the WAL),
/// run `step` (the app's local commit/copy), reopen the gate.
///
/// Both futures run under the hold and are cancelled at its deadline
/// ([`GateTimeout`]), so make `step` cancel-safe or have it own its child
/// processes (`kill_on_drop`). Do slow network work (a push) after this
/// returns.
///
/// # Errors
///
/// [`GateTimeout`] when checkpoint + step outlive [`WriteGate::max_hold`].
pub async fn quiesce<C, CF, S, SF, T>(
    gate: &WriteGate,
    checkpoint: C,
    step: S,
) -> Result<T, GateTimeout>
where
    C: FnOnce() -> CF,
    CF: Future<Output = ()>,
    S: FnOnce() -> SF,
    SF: Future<Output = T>,
{
    let hold = gate.pause().await;
    let outcome = tokio::time::timeout_at(hold.deadline(), async {
        checkpoint().await;
        step().await
    })
    .await;
    drop(hold);
    outcome.map_err(|_| GateTimeout {
        max_hold: gate.max_hold(),
    })
}

/// The stock sqlite `checkpoint` callback:
/// `PRAGMA wal_checkpoint(TRUNCATE)` on every connection, so the main db
/// files hold everything and the WAL sidecars are empty.
///
/// Best-effort per connection: a failure is logged and returned, and the
/// rest still run (the main file stays consistent in WAL mode; it just
/// lacks the un-checkpointed tail).
#[cfg(feature = "server-seaorm")]
pub async fn wal_checkpoint<'a, C>(conns: impl IntoIterator<Item = &'a C>) -> Vec<sea_orm::DbErr>
where
    C: sea_orm::ConnectionTrait + 'a,
{
    let mut failures = Vec::new();
    for conn in conns {
        if let Err(e) = conn
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE);")
            .await
        {
            tracing::warn!(error = %e, "wal_checkpoint failed");
            failures.push(e);
        }
    }
    failures
}
