//! [`PubSub`] — server-side stream fan-out, modelled on effect's `PubSub`
//! + `SubscriptionRef`.
//!
//! A hub of vox stream subscribers: [`attach`](PubSub::attach) the
//! `Tx` sinks that subscribe RPCs receive, [`publish`](PubSub::publish)
//! events to all of them. Three effect-named **overflow strategies**
//! (chosen at construction) govern what happens when a subscriber can't
//! keep up, an optional **replay window** hands late subscribers the last
//! `n` events, and the **buffered attach** protocol gives a subscribe
//! implementation effect-`SubscriptionRef` semantics — *current state
//! first, then every change* — without holding a lock across the
//! snapshot read.
//!
//! ## Strategies
//!
//! Every subscriber gets its own mailbox (effect's per-subscription
//! cursor, adapted to wire subscribers). `publish` never waits — the
//! writer can be a host's main thread — so the bounded strategies resolve
//! overflow by dropping:
//!
//! - [`PubSub::sliding`] — drop the **oldest** queued event for that
//!   subscriber. Right default for state-shaped events (`Upserted(row)`
//!   supersedes older news).
//! - [`PubSub::dropping`] — drop the **incoming** event for that
//!   subscriber (the backlog wins).
//! - [`PubSub::unbounded`] — never drop; the mailbox grows.
//!
//! (effect's fourth strategy — `bounded` back-pressure that suspends the
//! publisher — is deliberately absent: blocking the writer is exactly
//! what a host integration can't afford.)
//!
//! ## A sink that is full
//!
//! The mailbox is the hub's; the sink behind it (a vox `Tx`, a local
//! channel) has its own room, and `publish` only ever `try_send`s into
//! it. When the sink is full, the subscriber's **pump** takes over: one
//! background task ([`crate::platform::spawn`]) that awaits room for the
//! refused event, then drains the rest of the mailbox, so the backlog
//! moves on without waiting for the next publish. Publishes made while it
//! runs queue behind it, which keeps the order. The pump runs on the
//! publishing thread's tokio runtime, or, from a thread without one,
//! on the runtime the subscriber attached on. Without the `platform`
//! feature (which `local` and `server-axum` pull), or for a subscriber
//! attached outside any runtime, the backlog waits for the next publish.
//!
//! ## Snapshot-then-changes (`SubscriptionRef` semantics)
//!
//! effect's `SubscriptionRef.changes` atomically reads the current value
//! and subscribes, emitting `concat(current, changes)`. The wire
//! adaptation is the buffered attach: [`begin_attach`](PubSub::begin_attach)
//! parks the subscriber (its mailbox collects every publish from that
//! instant), the host then captures its snapshot at leisure (an async
//! repo read), and [`complete_attach`](PubSub::complete_attach) prepends
//! the snapshot and opens the tap. Changes that landed while the snapshot
//! was being read are delivered *after* it — possibly already contained
//! in it, which is why event payloads should carry full state
//! (idempotent re-application) rather than diffs.
//!
//! ```ignore
//! async fn subscribe(&self, sink: Tx<ExampleEvent>) -> Result<(), RepoError> {
//!     let pending = self.hub.begin_attach(sink);
//!     let rows = self.inner.list(everything()).await?;     // no lock held
//!     self.hub.complete_attach(pending, Some(ExampleEvent::Snapshot(rows.items)));
//!     Ok(())
//! }
//! ```

use crate::lock::lock;
use std::collections::VecDeque;
#[cfg(feature = "platform")]
use std::future::Future;
#[cfg(feature = "platform")]
use std::sync::Weak;
use std::sync::{Arc, Mutex};

/// What a bounded mailbox does when it's full.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Strategy {
    /// Drop the oldest queued event (new events win).
    Sliding(usize),
    /// Drop the incoming event (the backlog wins).
    Dropping(usize),
    /// Never drop.
    Unbounded,
}

/// How many events a `#[subscribe]` stream with arguments holds between
/// its backend and the wire.
///
/// The backend's [`EventSink`] is a local one this deep, relayed onto the
/// request's channel until the backend drops it (which ends the request;
/// `architect::rpc`'s stream sibling).
pub const SUBSCRIPTION_RELAY: usize = 16;

/// Where a subscriber's events go.
///
/// A vox channel (the wire) or a local async channel (an in-process
/// consumer — the HTTP Server-Sent-Events bridge, a test, a poller).
/// Every attach verb takes `impl Into<EventSink<T>>`, so backends written
/// against `vox::Tx` keep compiling unchanged.
pub enum EventSink<T> {
    /// A vox channel sender — events cross the wire.
    Vox(vox::Tx<T>),
    /// A local bounded channel — events stay in-process.
    Local(async_channel::Sender<T>),
}

/// An [`EventSink::send`] whose receiving end is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkClosed;

impl std::fmt::Display for SinkClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the event sink's receiver is gone")
    }
}

impl std::error::Error for SinkClosed {}

impl<T> From<vox::Tx<T>> for EventSink<T> {
    fn from(tx: vox::Tx<T>) -> Self {
        Self::Vox(tx)
    }
}

impl<T> From<async_channel::Sender<T>> for EventSink<T> {
    fn from(tx: async_channel::Sender<T>) -> Self {
        Self::Local(tx)
    }
}

impl<T> EventSink<T> {
    /// A local sink and its receiving end, with a mailbox of `capacity`.
    /// The hub's own per-subscriber mailbox sits in front of it, so this
    /// only needs to cover the consumer's read latency.
    #[must_use]
    pub fn local(capacity: usize) -> (Self, async_channel::Receiver<T>) {
        let (tx, rx) = async_channel::bounded(capacity.max(1));
        (Self::Local(tx), rx)
    }

    /// Send one event, waiting while the receiver is behind — the flow
    /// control a producer that streams on its own (a byte stream, a
    /// filtered relay) wants, where the hub's own mailbox would drop.
    ///
    /// # Errors
    ///
    /// [`SinkClosed`]: the receiving end is gone (or its wire failed).
    pub async fn send(&self, event: T) -> Result<(), SinkClosed>
    where
        T: facet::Facet<'static> + Send + Sync,
    {
        match self {
            Self::Vox(tx) => tx.send(event).await.map_err(|_| SinkClosed),
            Self::Local(tx) => tx.send(event).await.map_err(|_| SinkClosed),
        }
    }

    fn try_send(&self, event: T) -> Result<(), vox::TrySendError<T>>
    where
        T: facet::Facet<'static>,
    {
        match self {
            Self::Vox(tx) => tx.try_send(event),
            Self::Local(tx) => tx.try_send(event).map_err(|err| match err {
                async_channel::TrySendError::Full(event) => vox::TrySendError::Full(event),
                async_channel::TrySendError::Closed(event) => vox::TrySendError::Closed(event),
            }),
        }
    }
}

struct Subscriber<T> {
    /// Shared with the subscriber's pump while one runs.
    sink: Arc<EventSink<T>>,
    mailbox: VecDeque<T>,
    /// Buffered-attach parking: while `true`, the mailbox collects but
    /// nothing is sent (the snapshot hasn't been prepended yet).
    parked: bool,
    /// A pump holds the event the sink refused and is waiting for room;
    /// publishes only queue until it finishes, so nothing overtakes it.
    pumping: bool,
    /// Set when the sink reports closed; pruned on the next sweep.
    dead: bool,
    /// The runtime the subscriber attached on, for arming its pump from
    /// a thread that has none (a host's main-thread poller).
    #[cfg(all(feature = "platform", not(target_arch = "wasm32")))]
    runtime: Option<tokio::runtime::Handle>,
    id: u64,
}

struct Inner<T> {
    subscribers: Vec<Subscriber<T>>,
    /// Replay window: the last `replay_capacity` published events, handed
    /// to new subscribers ahead of live traffic.
    replay: VecDeque<T>,
    next_id: u64,
}

/// A pending buffered attach — returned by [`PubSub::begin_attach`],
/// consumed by [`PubSub::complete_attach`] / [`PubSub::abort_attach`].
#[must_use = "complete_attach (or abort_attach) must be called, or the subscriber stays parked forever"]
pub struct PendingAttach(u64);

/// Cloneable fan-out hub. All clones share the subscriber list. Fully
/// synchronous — publishable from non-async contexts (main-thread
/// pollers) as long as a vox runtime drives the connections underneath.
pub struct PubSub<T> {
    inner: Arc<Mutex<Inner<T>>>,
    strategy: Strategy,
    replay_capacity: usize,
}

impl<T> Clone for PubSub<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            strategy: self.strategy,
            replay_capacity: self.replay_capacity,
        }
    }
}

impl<T> PubSub<T> {
    fn with_strategy(strategy: Strategy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                subscribers: Vec::new(),
                replay: VecDeque::new(),
                next_id: 0,
            })),
            strategy,
            replay_capacity: 0,
        }
    }

    /// Bounded per-subscriber mailboxes; on overflow the **oldest** queued
    /// event for that subscriber is dropped. The default for state-shaped
    /// events.
    #[must_use]
    pub fn sliding(capacity: usize) -> Self {
        Self::with_strategy(Strategy::Sliding(capacity.max(1)))
    }

    /// Bounded per-subscriber mailboxes; on overflow the **incoming**
    /// event is dropped for that subscriber.
    #[must_use]
    pub fn dropping(capacity: usize) -> Self {
        Self::with_strategy(Strategy::Dropping(capacity.max(1)))
    }

    /// Unbounded per-subscriber mailboxes — nothing is ever dropped.
    #[must_use]
    pub fn unbounded() -> Self {
        Self::with_strategy(Strategy::Unbounded)
    }

    /// Keep the last `n` published events and hand them to every new
    /// subscriber before live traffic (effect's `replay` option).
    #[must_use]
    pub const fn with_replay(mut self, n: usize) -> Self {
        self.replay_capacity = n;
        self
    }

    /// Subscribe from in-process: a receiver that sees the replay window
    /// and then every publish, until it is dropped. The local twin of
    /// handing a `vox::Tx` to [`attach`](Self::attach).
    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> async_channel::Receiver<T>
    where
        T: Clone + facet::Facet<'static> + Send + Sync + 'static,
    {
        let (sink, rx) = EventSink::local(capacity);
        self.attach(sink);
        rx
    }

    /// Live subscriber count (as of the last sweep).
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner).subscribers.len()
    }
}

impl<T> PubSub<T>
where
    T: Clone + facet::Facet<'static> + Send + Sync + 'static,
{
    /// Attach a subscriber: replay window first, then every subsequent
    /// publish. For snapshot-then-changes semantics use
    /// [`begin_attach`](Self::begin_attach) instead.
    pub fn attach(&self, sink: impl Into<EventSink<T>>) {
        let pending = self.begin_attach(sink);
        self.complete_attach(pending, None);
    }

    /// Start a buffered attach: from this instant every publish lands in
    /// the subscriber's mailbox, but nothing is delivered until
    /// [`complete_attach`](Self::complete_attach) — giving the caller a
    /// race-free window to capture a snapshot.
    // The guard spans the whole body on purpose: allocating the id,
    // seeding the replay mailbox and pushing the subscriber must be one
    // atomic step, or a concurrent `publish` can land between them.
    #[allow(clippy::significant_drop_tightening)]
    pub fn begin_attach(&self, sink: impl Into<EventSink<T>>) -> PendingAttach {
        let sink = sink.into();
        let mut inner = lock(&self.inner);
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1);
        let mailbox = inner.replay.iter().cloned().collect();
        inner.subscribers.push(Subscriber {
            sink: Arc::new(sink),
            mailbox,
            parked: true,
            pumping: false,
            dead: false,
            #[cfg(all(feature = "platform", not(target_arch = "wasm32")))]
            runtime: tokio::runtime::Handle::try_current().ok(),
            id,
        });
        PendingAttach(id)
    }

    /// Finish a buffered attach: prepend `intro` (the snapshot) ahead of
    /// everything the mailbox collected, and open the tap.
    // `PendingAttach` by value on purpose: it is a linear token handed
    // out by `begin_attach`, and consuming it is what stops the same
    // attach being completed (or aborted) twice.
    #[allow(clippy::needless_pass_by_value)]
    pub fn complete_attach(&self, pending: PendingAttach, intro: Option<T>) {
        let mut inner = lock(&self.inner);
        if let Some(sub) = inner.subscribers.iter_mut().find(|s| s.id == pending.0) {
            if let Some(intro) = intro {
                sub.mailbox.push_front(intro);
            }
            sub.parked = false;
            self.deliver(sub);
        }
        inner.subscribers.retain(|s| !s.dead);
    }

    /// [`complete_attach`](Self::complete_attach) with several intros —
    /// a snapshot sent in parts — delivered in order, ahead of everything
    /// the mailbox collected.
    #[allow(clippy::needless_pass_by_value)]
    pub fn complete_attach_all(&self, pending: PendingAttach, intros: Vec<T>) {
        let mut inner = lock(&self.inner);
        if let Some(sub) = inner.subscribers.iter_mut().find(|s| s.id == pending.0) {
            for intro in intros.into_iter().rev() {
                sub.mailbox.push_front(intro);
            }
            sub.parked = false;
            self.deliver(sub);
        }
        inner.subscribers.retain(|s| !s.dead);
    }

    /// Abandon a buffered attach (e.g. the snapshot read failed) — the
    /// subscriber is removed without receiving anything.
    // By value for the same reason as `complete_attach`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn abort_attach(&self, pending: PendingAttach) {
        let mut inner = lock(&self.inner);
        inner.subscribers.retain(|s| s.id != pending.0);
    }

    /// Send `event` to every subscriber without blocking, applying the
    /// hub's overflow strategy per mailbox. Returns how many subscribers
    /// are live afterwards.
    ///
    /// # Not real-time safe
    ///
    /// This takes the hub's mutex (and clones the event per
    /// subscriber). **Never call it from a real-time thread** — an
    /// audio callback blocking on a lock held by a lower-priority
    /// thread is a priority inversion, and the deadline miss is an
    /// audible dropout. Real-time producers push into a wait-free
    /// `architect::rt::rt_channel` (the `rt` feature) and a normal
    /// thread drains it into the hub via `RtConsumer::drain_into`.
    pub fn publish(&self, event: T) -> usize {
        let mut inner = lock(&self.inner);
        if self.replay_capacity > 0 {
            if inner.replay.len() == self.replay_capacity {
                inner.replay.pop_front();
            }
            inner.replay.push_back(event.clone());
        }
        let strategy = self.strategy;
        for sub in &mut inner.subscribers {
            match strategy {
                Strategy::Sliding(cap) => {
                    if sub.mailbox.len() == cap {
                        sub.mailbox.pop_front();
                    }
                    sub.mailbox.push_back(event.clone());
                }
                Strategy::Dropping(cap) => {
                    if sub.mailbox.len() < cap {
                        sub.mailbox.push_back(event.clone());
                    }
                }
                Strategy::Unbounded => sub.mailbox.push_back(event.clone()),
            }
            self.deliver(sub);
        }
        inner.subscribers.retain(|s| !s.dead);
        inner.subscribers.len()
    }

    /// Hand one open subscriber's backlog to its sink. What the sink
    /// won't take right now goes to the subscriber's pump. Does nothing
    /// while the subscriber is parked or a pump already runs.
    // `self` is only read to arm a pump, which takes `platform`.
    #[cfg_attr(not(feature = "platform"), allow(clippy::unused_self))]
    fn deliver(&self, sub: &mut Subscriber<T>) {
        if sub.parked || sub.pumping || sub.dead {
            return;
        }
        let Some(refused) = drain_ready(sub) else {
            return;
        };
        #[cfg(feature = "platform")]
        if let Some(runtime) = PumpRuntime::find(sub) {
            sub.pumping = true;
            runtime.spawn(Self::pump(
                Arc::downgrade(&self.inner),
                sub.id,
                Arc::clone(&sub.sink),
                refused,
            ));
            return;
        }
        // Nothing to run a pump on: the event waits for the next drain.
        sub.mailbox.push_front(refused);
    }

    /// A subscriber's pump: wait for room for `event`, then drain the
    /// mailbox again, until the sink takes everything without waiting,
    /// the sink closes, or the hub (or the subscriber) is gone.
    #[cfg(feature = "platform")]
    async fn pump(hub: Weak<Mutex<Inner<T>>>, id: u64, sink: Arc<EventSink<T>>, event: T) {
        let mut event = event;
        loop {
            let sent = sink.send(event).await;
            match Self::after_pumped(&hub, id, sent) {
                Some(refused) => event = refused,
                None => return,
            }
        }
    }

    /// The pump's step after a send: the next event the sink refused, or
    /// `None` when the pump is done (its `pumping` flag cleared).
    #[cfg(feature = "platform")]
    fn after_pumped(
        hub: &Weak<Mutex<Inner<T>>>,
        id: u64,
        sent: Result<(), SinkClosed>,
    ) -> Option<T> {
        let hub = hub.upgrade()?;
        let mut inner = lock(&hub);
        let sub = inner.subscribers.iter_mut().find(|s| s.id == id)?;
        let refused = if sent.is_ok() {
            drain_ready(sub)
        } else {
            sub.dead = true;
            None
        };
        if refused.is_none() {
            sub.pumping = false;
            inner.subscribers.retain(|s| !s.dead);
        }
        refused
    }
}

/// Push as much of one subscriber's backlog as the sink will take right
/// now. Returns the event a full sink refused (taken off the mailbox);
/// a closed sink marks the subscriber dead.
fn drain_ready<T>(sub: &mut Subscriber<T>) -> Option<T>
where
    T: facet::Facet<'static>,
{
    while let Some(event) = sub.mailbox.pop_front() {
        match sub.sink.try_send(event) {
            Ok(()) => {}
            Err(vox::TrySendError::Full(event)) => return Some(event),
            Err(vox::TrySendError::Closed(_)) => {
                sub.dead = true;
                return None;
            }
        }
    }
    None
}

/// Where a subscriber's pump runs: the publishing thread's tokio runtime,
/// else the one the subscriber attached on. On wasm, the browser's
/// executor, which is always there.
#[cfg(feature = "platform")]
struct PumpRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    handle: tokio::runtime::Handle,
}

#[cfg(feature = "platform")]
impl PumpRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    fn find<T>(sub: &Subscriber<T>) -> Option<Self> {
        let handle = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| sub.runtime.clone())?;
        Some(Self { handle })
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unnecessary_wraps)]
    const fn find<T>(_sub: &Subscriber<T>) -> Option<Self> {
        Some(Self {})
    }

    /// Run `pump` in the background, detached: it ends on its own (the
    /// sink took everything, the sink closed, or the hub is gone).
    fn spawn<F>(self, pump: F)
    where
        F: Future<Output = ()> + crate::platform::MaybeSend + 'static,
    {
        #[cfg(not(target_arch = "wasm32"))]
        let _entered = self.handle.enter();
        drop(crate::platform::spawn(pump));
    }
}

#[cfg(all(test, feature = "platform"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    const PATIENCE: Duration = Duration::from_secs(5);

    async fn next(rx: &async_channel::Receiver<String>) -> String {
        crate::platform::timeout(PATIENCE, rx.recv())
            .await
            .expect("an event in time")
            .expect("the hub still holds the sink")
    }

    #[tokio::test]
    async fn an_event_that_meets_a_full_sink_arrives_without_another_publish() {
        let hub = PubSub::<String>::unbounded();
        let rx = hub.subscribe(1);
        hub.publish("first".into()); // fills the sink
        hub.publish("second".into()); // meets it full

        assert_eq!(next(&rx).await, "first"); // room again; no publish follows
        assert_eq!(next(&rx).await, "second");
    }

    #[tokio::test]
    async fn publishes_behind_a_pump_keep_their_order() {
        let hub = PubSub::<String>::unbounded();
        let rx = hub.subscribe(1);
        for n in 0..20 {
            hub.publish(n.to_string());
        }
        for n in 0..20 {
            assert_eq!(next(&rx).await, n.to_string());
        }
    }

    #[tokio::test]
    async fn a_sink_that_closes_under_its_pump_is_pruned() {
        let hub = PubSub::<String>::unbounded();
        let rx = hub.subscribe(1);
        hub.publish("first".into());
        hub.publish("second".into());
        drop(rx);

        crate::platform::timeout(PATIENCE, async {
            while hub.subscriber_count() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the pump prunes a closed sink");
    }

    #[test]
    fn a_thread_without_a_runtime_publishes_through_the_attached_runtime() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .build()
            .unwrap();
        let hub = PubSub::<String>::unbounded();
        let rx = runtime.block_on(async { hub.subscribe(1) });

        std::thread::scope(|scope| {
            scope.spawn(|| {
                hub.publish("first".into());
                hub.publish("second".into());
            });
        });

        runtime.block_on(async {
            assert_eq!(next(&rx).await, "first");
            assert_eq!(next(&rx).await, "second");
        });
    }

    #[test]
    fn without_any_runtime_a_full_sink_waits_for_the_next_publish() {
        let hub = PubSub::<String>::unbounded();
        let rx = hub.subscribe(1);
        hub.publish("first".into());
        hub.publish("second".into()); // no runtime: stays queued, no panic

        assert_eq!(rx.try_recv().unwrap(), "first");
        assert!(rx.try_recv().is_err());
        hub.publish("third".into());
        assert_eq!(rx.try_recv().unwrap(), "second");
    }
}
