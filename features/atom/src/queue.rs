//! The durable call queue: service calls a person made while the server
//! could not be reached, kept in order on the device and replayed when it
//! can be.
//!
//! For *computed* records — a timer's start and stop, a mode switch —
//! where there is no document to merge and the write is a call, not a
//! field. The rules:
//!
//! - A call that fails because the server is **unreachable**
//!   ([`Failure::Unreachable`]) is queued and its optimistic state stays
//!   ([`Mutation::run_queued`](crate::Mutation::run_queued)). A call the
//!   server **refused** ([`Failure::Refused`]) rolls back with a
//!   notification, exactly as [`Mutation::run`](crate::Mutation::run)
//!   does. [`classify`] is the one place that tells them apart.
//! - [`CallQueue::replay`] sends calls oldest first, one at a time,
//!   through a [`CallTransport`] (the same service calls a live write
//!   makes). A call leaves the queue only when the server accepts it; a
//!   refusal drops it as [`Settled::Lost`] — the Dioxus side names it in
//!   a notification; a transport failure stops the replay with the queue
//!   intact.
//! - A queued call that created something under a temporary key
//!   ([`QueuedCall::temp_key`]) is remapped: when the server answers with
//!   the real key, every later queued call naming the temporary one is
//!   rewritten before it is sent.
//!
//! The queue lives in [`LocalRows`] (`queue_*`), so it survives the app
//! closing.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::persist::{LocalError, LocalRows, MaybeSendSync};

/// One queued service call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedCall {
    /// Position in the queue — set by [`CallQueue::push`].
    #[serde(default)]
    pub seq: u64,
    /// Which call this is (`"timer.start"`); the transport dispatches on it.
    pub call: String,
    /// Where it goes (an org slug, a server) — opaque to the queue.
    pub route: String,
    /// The call's arguments.
    pub payload: Value,
    /// What a person would call it ("Start the timer on “Mix”"), for the
    /// notification if it is lost.
    pub what: String,
    /// The temporary key this call's result was shown under, when it
    /// creates something; replaced in later calls by the server's key.
    #[serde(default)]
    pub temp_key: Option<String>,
}

impl QueuedCall {
    /// A call with no temporary key.
    #[must_use]
    pub fn new(
        call: impl Into<String>,
        route: impl Into<String>,
        payload: Value,
        what: impl Into<String>,
    ) -> Self {
        Self {
            seq: 0,
            call: call.into(),
            route: route.into(),
            payload,
            what: what.into(),
            temp_key: None,
        }
    }

    /// This call creates something shown under `key` until it lands.
    #[must_use]
    pub fn creating(mut self, key: impl Into<String>) -> Self {
        self.temp_key = Some(key.into());
        self
    }
}

/// Why a call did not land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No answer — a dropped connection, a dial that failed. Queue and
    /// retry.
    Unreachable(String),
    /// The server ran the call and said no. Roll back / drop and tell.
    Refused(String),
}

impl Failure {
    #[must_use]
    pub const fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable(_))
    }

    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Unreachable(m) | Self::Refused(m) => m,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Markers of an answer *from a handler* — checked first, since a
/// refusal's own text may mention anything.
const REFUSAL_MARKERS: &[&str] = &["User(", "InvalidPayload", "UnknownMethod"];

/// Markers of a call that never reached a handler: vox's connection
/// errors (`VoxError::{ConnectionClosed, ConnectionShutdown, SendFailed,
/// TimedOut, Indeterminate}`, in `{:?}` or `Display` form) and the dial
/// failures of the native and browser WebSocket and iroh transports.
const UNREACHABLE_MARKERS: &[&str] = &[
    "ConnectionClosed",
    "ConnectionShutdown",
    "SendFailed",
    "TimedOut",
    "Indeterminate",
    "vox connection closed",
    "vox connection shutdown",
    "vox send failed",
    "vox request timed out",
    "ws connect",
    "ws request",
    "WebSocket open failed",
    "WebSocket::new",
    "establish `",
    "dial cancelled",
    "iroh dial",
    "vox establish over iroh",
    "Connection refused",
    "connection refused",
];

/// Tell an unreachable server from a refusal, from a call's error message.
///
/// Unknown messages are refusals: rolling back is recoverable
/// (the person sees it and tries again); queueing a call the server will
/// never take only defers the bad news.
#[must_use]
pub fn classify(message: &str) -> Failure {
    if REFUSAL_MARKERS.iter().any(|m| message.contains(m)) {
        return Failure::Refused(message.to_owned());
    }
    if UNREACHABLE_MARKERS.iter().any(|m| message.contains(m)) {
        return Failure::Unreachable(message.to_owned());
    }
    Failure::Refused(message.to_owned())
}

/// The service calls a replay makes. `Ok(Some(key))` reports the real key
/// of what a creating call made.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait CallTransport: MaybeSendSync + 'static {
    async fn send(&self, call: &QueuedCall) -> Result<Option<String>, Failure>;
}

/// What became of one call the server answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// Accepted; `key` is the real key a creating call made.
    Accepted {
        call: QueuedCall,
        key: Option<String>,
    },
    /// Refused on replay: dropped, and the person must be told.
    Lost { call: QueuedCall, reason: String },
}

impl Settled {
    #[must_use]
    pub const fn call(&self) -> &QueuedCall {
        match self {
            Self::Accepted { call, .. } | Self::Lost { call, .. } => call,
        }
    }
}

/// What one replay did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Every call the server answered, in order.
    pub settled: Vec<Settled>,
    /// Set when an unreachable server stopped the replay; the rest are
    /// still queued.
    pub stopped: Option<String>,
}

/// Replace every string equal to `from` with `to`, anywhere in `value`.
fn remap(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(s) if s == from => to.clone_into(s),
        Value::Array(items) => items.iter_mut().for_each(|v| remap(v, from, to)),
        Value::Object(map) => map.values_mut().for_each(|v| remap(v, from, to)),
        _ => {}
    }
}

/// The durable queue, over the device's [`LocalRows`].
#[derive(Clone)]
pub struct CallQueue {
    rows: Arc<dyn LocalRows>,
}

// On wasm the rows are single-threaded (`?Send`), so these futures are
// too — the browser has one thread.
#[cfg_attr(target_arch = "wasm32", allow(clippy::future_not_send))]
impl CallQueue {
    #[must_use]
    pub fn new(rows: Arc<dyn LocalRows>) -> Self {
        Self { rows }
    }

    /// Append `call`; returns its sequence number.
    pub async fn push(&self, mut call: QueuedCall) -> Result<u64, LocalError> {
        call.seq = 0;
        let body = serde_json::to_string(&call).map_err(|e| LocalError(e.to_string()))?;
        self.rows.queue_push(body).await
    }

    /// Every queued call, oldest first. A body an older build wrote in
    /// another shape is skipped.
    pub async fn list(&self) -> Result<Vec<QueuedCall>, LocalError> {
        Ok(self
            .rows
            .queue_list()
            .await?
            .into_iter()
            .filter_map(|(seq, body)| {
                let mut call: QueuedCall = serde_json::from_str(&body).ok()?;
                call.seq = seq;
                Some(call)
            })
            .collect())
    }

    /// Replay oldest first until the queue is empty or the server stops
    /// answering. See the module docs for the rules.
    pub async fn replay<T: CallTransport + ?Sized>(
        &self,
        transport: &T,
    ) -> Result<ReplayReport, LocalError> {
        let mut report = ReplayReport::default();
        loop {
            let Some(call) = self.list().await?.into_iter().next() else {
                break;
            };
            match transport.send(&call).await {
                Ok(key) => {
                    self.rows.queue_remove(call.seq).await?;
                    if let (Some(from), Some(to)) = (&call.temp_key, &key)
                        && from != to
                    {
                        self.remap_later(from, to).await?;
                    }
                    report.settled.push(Settled::Accepted { call, key });
                }
                Err(Failure::Refused(reason)) => {
                    self.rows.queue_remove(call.seq).await?;
                    report.settled.push(Settled::Lost { call, reason });
                }
                Err(Failure::Unreachable(e)) => {
                    report.stopped = Some(e);
                    break;
                }
            }
        }
        Ok(report)
    }

    async fn remap_later(&self, from: &str, to: &str) -> Result<(), LocalError> {
        for mut later in self.list().await? {
            let before = later.clone();
            remap(&mut later.payload, from, to);
            if later.temp_key.as_deref() == Some(from) {
                later.temp_key = Some(to.to_owned());
            }
            if later != before {
                let seq = later.seq;
                later.seq = 0;
                let body = serde_json::to_string(&later).map_err(|e| LocalError(e.to_string()))?;
                self.rows.queue_replace(seq, body).await?;
            }
        }
        Ok(())
    }
}

// ── the Dioxus side ─────────────────────────────────────────────────

mod hooks {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use dioxus::dioxus_core::spawn_forever;
    use dioxus::prelude::*;

    use super::{CallQueue, CallTransport, QueuedCall, Settled};
    use crate::notify::try_use_notifications;

    type FollowUp = Box<dyn FnOnce(&Settled)>;

    /// The app-root handle to the call queue: how many calls are waiting
    /// (for the shell's "N changes waiting to sync"), and a way to ask
    /// for a replay (on reconnect, after a push).
    #[derive(Clone, Copy)]
    pub struct CallQueueHandle {
        waiting: Signal<usize>,
        kick: Signal<u64>,
        queue: Signal<Option<CallQueue>>,
        follow_ups: Signal<Rc<RefCell<HashMap<u64, FollowUp>>>>,
    }

    impl CallQueueHandle {
        /// Calls queued and not yet answered.
        #[must_use]
        pub fn waiting(&self) -> usize {
            *self.waiting.read()
        }

        /// Ask for a replay now. Cheap; coalesces with one in flight.
        pub fn kick(&self) {
            let mut kick = self.kick;
            let n = kick.peek().wrapping_add(1);
            kick.set(n);
        }

        /// Queue `call`, and run `follow_up` once it settles in this
        /// session (a queued insert removes its placeholder there).
        pub fn enqueue(&self, call: QueuedCall, follow_up: Option<FollowUp>) {
            let Some(queue) = self.queue.peek().clone() else {
                return;
            };
            let this = *self;
            spawn_forever(async move {
                if let Ok(seq) = queue.push(call).await {
                    if let Some(f) = follow_up {
                        this.follow_ups.peek().borrow_mut().insert(seq, f);
                    }
                }
                this.refresh_count().await;
                this.kick();
            });
        }

        // Dioxus signals are single-threaded; this runs on the UI
        // executor (`spawn_forever`), never across threads.
        #[allow(clippy::future_not_send)]
        async fn refresh_count(&self) {
            let Some(queue) = self.queue.peek().clone() else {
                return;
            };
            if let Ok(calls) = queue.list().await {
                let mut waiting = self.waiting;
                if let Ok(mut w) = waiting.try_write() {
                    *w = calls.len();
                }
            }
        }
    }

    /// Provide the call queue at the app root, replaying through
    /// `transport` on mount and on every [`CallQueueHandle::kick`]. A
    /// lost call is named in the app's notifications.
    pub fn provide_call_queue<T: CallTransport>(
        queue: CallQueue,
        transport: Rc<T>,
    ) -> CallQueueHandle {
        let notify = try_use_notifications();
        let handle = CallQueueHandle {
            waiting: use_signal(|| 0),
            kick: use_signal(|| 0),
            queue: use_signal(|| Some(queue)),
            follow_ups: use_signal(|| Rc::new(RefCell::new(HashMap::new()))),
        };
        let handle = use_context_provider(|| handle);
        let running = use_hook(|| Rc::new(RefCell::new(false)));
        use_effect(move || {
            let _ = handle.kick.read();
            if *running.borrow() {
                return;
            }
            *running.borrow_mut() = true;
            let transport = transport.clone();
            let running = running.clone();
            spawn_forever(async move {
                if let Some(queue) = handle.queue.peek().clone()
                    && let Ok(report) = queue.replay(&*transport).await
                {
                    for s in &report.settled {
                        if let Settled::Lost { call, reason } = s
                            && let Some(n) = notify
                        {
                            n.error(format!("Couldn't sync “{}”: {reason}", call.what));
                        }
                        let f = handle.follow_ups.peek().borrow_mut().remove(&s.call().seq);
                        if let Some(f) = f {
                            f(s);
                        }
                    }
                }
                handle.refresh_count().await;
                *running.borrow_mut() = false;
            });
        });
        handle
    }

    /// The app-root call queue, if one was provided.
    #[must_use]
    pub fn try_use_call_queue() -> Option<CallQueueHandle> {
        try_consume_context::<CallQueueHandle>()
    }
}

pub use hooks::{CallQueueHandle, provide_call_queue, try_use_call_queue};

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::persist::MemoryRows;

    /// A server behind a switch: answers in order, mints its own keys,
    /// refuses the calls it is told to.
    struct Fake {
        up: Mutex<bool>,
        refuse: Vec<String>,
        log: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                up: Mutex::new(true),
                refuse: vec![],
                log: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait::async_trait]
    impl CallTransport for Fake {
        async fn send(&self, call: &QueuedCall) -> Result<Option<String>, Failure> {
            if !*self.up.lock().unwrap() {
                return Err(classify(
                    "ws connect `ws://127.0.0.1:1/vox`: Io(Connection refused)",
                ));
            }
            if self.refuse.contains(&call.what) {
                return Err(classify("timer.stop: User(NoRunningTimer)"));
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("{} {}", call.call, call.payload));
            Ok(call.temp_key.as_ref().map(|_| "srv-1".to_owned()))
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        crate::persist::tests::futures_executor_block_on(f)
    }

    // t[verify offline.outbox]
    #[test]
    fn transport_errors_queue_and_refusals_do_not() {
        for unreachable in [
            "acme: start timer: ConnectionClosed",
            "acme: stop timer: SendFailed",
            "acme: stop timer: TimedOut",
            "ws connect `ws://127.0.0.1:18080/org/acme/vox`: Io(Os { code: 61, kind: ConnectionRefused })",
            "WebSocket open failed: `wss://x/org/acme/vox`",
            "establish `ws://x/vox`: Handshake",
        ] {
            assert!(classify(unreachable).is_unreachable(), "{unreachable}");
        }
        for refused in [
            "acme: stop timer: User(NoRunningTimer)",
            "acme: start timer: InvalidPayload(\"bad\")",
            "no such task",
            "User(Conflict(\"ConnectionClosed while saving\"))",
        ] {
            assert!(!classify(refused).is_unreachable(), "{refused}");
        }
    }

    // t[verify offline.outbox]
    #[test]
    fn replay_is_in_order_stops_when_unreachable_and_drops_refusals() {
        let rows: Arc<dyn LocalRows> = Arc::new(MemoryRows::in_memory());
        let q = CallQueue::new(rows.clone());
        let mut fake = Fake::new();
        fake.refuse.push("stop twice".into());
        block_on(async {
            q.push(
                QueuedCall::new("timer.start", "acme", json!({"task": "tmp-1"}), "start")
                    .creating("tmp-1"),
            )
            .await
            .unwrap();
            q.push(QueuedCall::new(
                "timer.stop",
                "acme",
                json!({"session": "tmp-1"}),
                "stop",
            ))
            .await
            .unwrap();
            q.push(QueuedCall::new(
                "timer.stop",
                "acme",
                json!({}),
                "stop twice",
            ))
            .await
            .unwrap();

            *fake.up.lock().unwrap() = false;
            let report = q.replay(&fake).await.unwrap();
            assert!(report.stopped.is_some() && report.settled.is_empty());
            assert_eq!(
                q.list().await.unwrap().len(),
                3,
                "nothing leaves unanswered"
            );

            // "Restart": a new queue over the same rows.
            let q = CallQueue::new(rows.clone());
            *fake.up.lock().unwrap() = true;
            let report = q.replay(&fake).await.unwrap();
            assert!(report.stopped.is_none());
            assert!(q.list().await.unwrap().is_empty());
            assert_eq!(
                *fake.log.lock().unwrap(),
                vec![
                    r#"timer.start {"task":"tmp-1"}"#.to_owned(),
                    r#"timer.stop {"session":"srv-1"}"#.to_owned(),
                ],
                "in order, the later call naming the server's key"
            );
            assert!(
                matches!(report.settled.last(), Some(Settled::Lost { call, .. }) if call.what == "stop twice")
            );
        });
    }
}
