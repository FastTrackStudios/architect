//! Real-time publisher bridge — wait-free event handoff from an audio
//! callback (or any real-time thread) to the [`PubSub`](crate::PubSub)
//! fan-out.
//!
//! [`PubSub::publish`](crate::PubSub::publish) takes a mutex. That is
//! fine from a main thread or a tokio worker, and **never acceptable
//! from a real-time thread**: if the publisher blocks on a lock held by
//! a lower-priority thread, the scheduler can leave the audio callback
//! waiting past its deadline (priority inversion → dropouts). The same
//! goes for allocation, channel sends that can park, and syscalls.
//!
//! The bridge splits the publish into two halves:
//!
//! - [`RtProducer::push`] — wait-free, allocation-free, lock-free. Call
//!   it from the audio callback. When the ring is full the event is
//!   dropped and counted, never blocked on.
//! - [`RtConsumer::drain`] / [`drain_into`](RtConsumer::drain_into) —
//!   called from a normal thread (the same UI-rate tick that already
//!   drives pollers), pulling everything the real-time side queued and
//!   handing it on — typically into a `PubSub` hub feeding
//!   `#[subscribe]` stream subscribers.
//!
//! ```ignore
//! // setup (non-RT): ring sized for ~2 ticks of worst-case event rate
//! let (mut rt_tx, mut rt_rx) = architect::rt::rt_channel::<PeakEvent>(1024);
//!
//! // audio callback (RT): wait-free push, drop-on-full
//! rt_tx.push(PeakEvent { l, r });
//!
//! // UI-rate tick (non-RT): fan out to subscribers
//! rt_rx.drain_into(&backend.peaks_hub());
//! ```
//!
//! For latest-value streams (meters, transport position) where only the
//! freshest sample matters, drain with [`RtConsumer::latest`] and
//! publish the single survivor — a full ring then costs staleness, not
//! correctness.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Create a wait-free single-producer single-consumer ring of `capacity`
/// events. The producer half goes to the real-time thread; the consumer
/// half stays on a normal thread.
///
/// Size the ring for the worst-case event burst between two drains —
/// overflow is dropped (and counted via [`RtProducer::dropped`] /
/// [`RtConsumer::dropped`]), never waited on.
pub fn rt_channel<T: Send>(capacity: usize) -> (RtProducer<T>, RtConsumer<T>) {
    let (tx, rx) = rtrb::RingBuffer::new(capacity.max(1));
    let dropped = Arc::new(AtomicU64::new(0));
    (
        RtProducer {
            tx,
            dropped: Arc::clone(&dropped),
        },
        RtConsumer { rx, dropped },
    )
}

/// Real-time half: wait-free pushes. `!Clone` — strictly single
/// producer; move it into the audio callback's state.
pub struct RtProducer<T> {
    tx: rtrb::Producer<T>,
    dropped: Arc<AtomicU64>,
}

impl<T> RtProducer<T> {
    /// Queue `event` for the consumer. Wait-free: no locks, no
    /// allocation, no syscalls. Returns `false` (and bumps the drop
    /// counter) when the ring is full — the event is discarded rather
    /// than ever blocking the caller.
    pub fn push(&mut self, event: T) -> bool {
        match self.tx.push(event) {
            Ok(()) => true,
            Err(rtrb::PushError::Full(_)) => {
                // Relaxed: the counter is diagnostic, not a
                // synchronization edge.
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Events dropped on overflow since creation. Diagnostic — read it
    /// from either side to size the ring or detect a stalled drain.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Whether the consumer half is still alive. RT-safe.
    pub fn is_abandoned(&self) -> bool {
        self.tx.is_abandoned()
    }
}

/// Normal-thread half: drains what the real-time side queued.
pub struct RtConsumer<T> {
    rx: rtrb::Consumer<T>,
    dropped: Arc<AtomicU64>,
}

impl<T> RtConsumer<T> {
    /// Pull every queued event, in order, into `f`. Returns how many
    /// were delivered. Call at UI rate (the same tick that drives
    /// pollers) — NOT from a real-time thread.
    pub fn drain(&mut self, mut f: impl FnMut(T)) -> usize {
        let mut n = 0;
        while let Ok(event) = self.rx.pop() {
            f(event);
            n += 1;
        }
        n
    }

    /// Drain, keeping only the freshest event. Right for latest-value
    /// streams (meters, position ticks) where intermediate samples are
    /// superseded — a full ring then costs staleness, not correctness.
    pub fn latest(&mut self) -> Option<T> {
        let mut last = None;
        while let Ok(event) = self.rx.pop() {
            last = Some(event);
        }
        last
    }

    /// Drain straight into a [`PubSub`](crate::PubSub) hub — the
    /// glue between an audio-thread producer and `#[subscribe]` stream
    /// subscribers. Returns how many events were published.
    #[cfg(feature = "vox")]
    pub fn drain_into(&mut self, hub: &crate::PubSub<T>) -> usize
    where
        T: Clone + facet::Facet<'static>,
    {
        self.drain(|event| {
            hub.publish(event);
        })
    }

    /// Events dropped on overflow since creation (see
    /// [`RtProducer::dropped`]).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_drain_round_trips_in_order() {
        let (mut tx, mut rx) = rt_channel::<u32>(8);
        for i in 0..5 {
            assert!(tx.push(i));
        }
        let mut seen = Vec::new();
        assert_eq!(rx.drain(|v| seen.push(v)), 5);
        assert_eq!(seen, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn overflow_drops_and_counts_instead_of_blocking() {
        let (mut tx, mut rx) = rt_channel::<u32>(2);
        assert!(tx.push(1));
        assert!(tx.push(2));
        assert!(!tx.push(3));
        assert_eq!(tx.dropped(), 1);
        let mut seen = Vec::new();
        rx.drain(|v| seen.push(v));
        // The backlog won; the overflow event is gone.
        assert_eq!(seen, vec![1, 2]);
        assert_eq!(rx.dropped(), 1);
    }

    #[test]
    fn latest_keeps_only_the_freshest() {
        let (mut tx, mut rx) = rt_channel::<u32>(8);
        for i in 0..5 {
            tx.push(i);
        }
        assert_eq!(rx.latest(), Some(4));
        assert_eq!(rx.latest(), None);
    }

    #[test]
    fn cross_thread_smoke() {
        let (mut tx, mut rx) = rt_channel::<u64>(1024);
        let producer = std::thread::spawn(move || {
            for i in 0..10_000u64 {
                while !tx.push(i) {
                    std::hint::spin_loop();
                }
            }
        });
        let mut next = 0u64;
        while next < 10_000 {
            rx.drain(|v| {
                assert_eq!(v, next);
                next += 1;
            });
        }
        producer.join().unwrap();
    }
}
