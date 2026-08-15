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

use std::collections::VecDeque;
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

struct Subscriber<T> {
    sink: vox::Tx<T>,
    mailbox: VecDeque<T>,
    /// Buffered-attach parking: while `true`, the mailbox collects but
    /// nothing is sent (the snapshot hasn't been prepended yet).
    parked: bool,
    /// Set when the sink reports closed; pruned on the next sweep.
    dead: bool,
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
    pub fn sliding(capacity: usize) -> Self {
        Self::with_strategy(Strategy::Sliding(capacity.max(1)))
    }

    /// Bounded per-subscriber mailboxes; on overflow the **incoming**
    /// event is dropped for that subscriber.
    pub fn dropping(capacity: usize) -> Self {
        Self::with_strategy(Strategy::Dropping(capacity.max(1)))
    }

    /// Unbounded per-subscriber mailboxes — nothing is ever dropped.
    pub fn unbounded() -> Self {
        Self::with_strategy(Strategy::Unbounded)
    }

    /// Keep the last `n` published events and hand them to every new
    /// subscriber before live traffic (effect's `replay` option).
    pub fn with_replay(mut self, n: usize) -> Self {
        self.replay_capacity = n;
        self
    }

    /// Live subscriber count (as of the last sweep).
    pub fn subscriber_count(&self) -> usize {
        self.inner.lock().expect("pubsub lock").subscribers.len()
    }
}

impl<T> PubSub<T>
where
    T: Clone + facet::Facet<'static>,
{
    /// Attach a subscriber: replay window first, then every subsequent
    /// publish. For snapshot-then-changes semantics use
    /// [`begin_attach`](Self::begin_attach) instead.
    pub fn attach(&self, sink: vox::Tx<T>) {
        let pending = self.begin_attach(sink);
        self.complete_attach(pending, None);
    }

    /// Start a buffered attach: from this instant every publish lands in
    /// the subscriber's mailbox, but nothing is delivered until
    /// [`complete_attach`](Self::complete_attach) — giving the caller a
    /// race-free window to capture a snapshot.
    pub fn begin_attach(&self, sink: vox::Tx<T>) -> PendingAttach {
        let mut inner = self.inner.lock().expect("pubsub lock");
        let id = inner.next_id;
        inner.next_id += 1;
        let mailbox = inner.replay.iter().cloned().collect();
        inner.subscribers.push(Subscriber {
            sink,
            mailbox,
            parked: true,
            dead: false,
            id,
        });
        PendingAttach(id)
    }

    /// Finish a buffered attach: prepend `intro` (the snapshot) ahead of
    /// everything the mailbox collected, and open the tap.
    pub fn complete_attach(&self, pending: PendingAttach, intro: Option<T>) {
        let mut inner = self.inner.lock().expect("pubsub lock");
        if let Some(sub) = inner.subscribers.iter_mut().find(|s| s.id == pending.0) {
            if let Some(intro) = intro {
                sub.mailbox.push_front(intro);
            }
            sub.parked = false;
            Self::drain_one(sub);
        }
        inner.subscribers.retain(|s| !s.dead);
    }

    /// Abandon a buffered attach (e.g. the snapshot read failed) — the
    /// subscriber is removed without receiving anything.
    pub fn abort_attach(&self, pending: PendingAttach) {
        let mut inner = self.inner.lock().expect("pubsub lock");
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
        let mut inner = self.inner.lock().expect("pubsub lock");
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
            if !sub.parked {
                Self::drain_one(sub);
            }
        }
        inner.subscribers.retain(|s| !s.dead);
        inner.subscribers.len()
    }

    /// Push as much of one subscriber's backlog as the wire will take
    /// right now. `Full` keeps the rest queued for the next drain; a
    /// closed sink marks the subscriber dead.
    fn drain_one(sub: &mut Subscriber<T>) {
        while let Some(event) = sub.mailbox.pop_front() {
            match sub.sink.try_send(event) {
                Ok(()) => {}
                Err(vox::TrySendError::Full(event)) => {
                    sub.mailbox.push_front(event);
                    break;
                }
                Err(vox::TrySendError::Closed(_)) => {
                    sub.dead = true;
                    break;
                }
            }
        }
    }
}
