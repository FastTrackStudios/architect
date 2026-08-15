//! Everyday async concurrency primitives, portable across native↔wasm.
//!
//! - [`Deferred`] — a write-once value many tasks can await (a broadcast
//!   one-shot).
//! - [`Semaphore`] — an N-permit async gate for bounding concurrency.
//! - [`Queue`] — a bounded/unbounded MPMC channel.
//!
//! All wasm-clean: [`Deferred`]/[`Semaphore`] ride `tokio::sync` (which makes
//! no reactor assumptions), [`Queue`] rides `async-channel`. Consumers
//! program against these types, so the backing implementation stays a
//! swappable seam.

use std::sync::Arc;

use tokio::sync::watch;

// ── Deferred ──────────────────────────────────────────────────────────────

/// A write-once cell that tasks can await. The first [`complete`](Deferred::complete)
/// sets the value (and wins); every waiter — past or future — then resolves
/// to a clone of it. Think `tokio::sync::oneshot` but multi-consumer and
/// non-consuming.
#[derive(Clone)]
pub struct Deferred<T> {
    tx: Arc<watch::Sender<Option<T>>>,
    rx: watch::Receiver<Option<T>>,
}

impl<T: Clone + Send + Sync + 'static> Deferred<T> {
    /// A fresh, unset deferred.
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(None);
        Self {
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Set the value if it isn't already set. Returns `true` if this call set
    /// it, `false` if it was already complete (first writer wins).
    pub fn complete(&self, value: T) -> bool {
        let mut set = false;
        let mut slot = Some(value);
        self.tx.send_if_modified(|current| {
            if current.is_none() {
                *current = slot.take();
                set = true;
                true
            } else {
                false
            }
        });
        set
    }

    /// The value if already set, else `None` (non-blocking).
    pub fn try_get(&self) -> Option<T> {
        self.rx.borrow().clone()
    }

    /// `true` once a value has been set.
    pub fn is_complete(&self) -> bool {
        self.rx.borrow().is_some()
    }

    /// Await the value, resolving as soon as it's set (immediately if already
    /// set).
    pub async fn wait(&self) -> T {
        let mut rx = self.rx.clone();
        loop {
            if let Some(value) = rx.borrow().clone() {
                return value;
            }
            // `changed` errors only if every sender dropped — but `self`
            // holds one in an `Arc`, so while a caller can await this, a
            // sender is alive. Re-loop to read the freshly-set value.
            if rx.changed().await.is_err() {
                if let Some(value) = rx.borrow().clone() {
                    return value;
                }
                std::future::pending::<()>().await;
            }
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Default for Deferred<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ── Semaphore ──────────────────────────────────────────────────────────────

/// An async counting semaphore — bound how many tasks run a section at once.
/// Acquire a [`Permit`]; drop it to release.
#[derive(Clone)]
pub struct Semaphore {
    inner: Arc<tokio::sync::Semaphore>,
}

/// A held semaphore permit. Releases its slot back to the [`Semaphore`] on
/// drop.
pub struct Permit(#[allow(dead_code)] tokio::sync::OwnedSemaphorePermit);

impl Semaphore {
    /// A semaphore with `permits` slots.
    pub fn new(permits: usize) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Semaphore::new(permits)),
        }
    }

    /// Acquire one permit, waiting if none are free.
    pub async fn acquire(&self) -> Permit {
        Permit(
            self.inner
                .clone()
                .acquire_owned()
                .await
                .expect("architect::platform::Semaphore is never closed"),
        )
    }

    /// Try to acquire one permit without waiting.
    pub fn try_acquire(&self) -> Option<Permit> {
        self.inner.clone().try_acquire_owned().ok().map(Permit)
    }

    /// Permits currently available.
    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }
}

// ── Queue ──────────────────────────────────────────────────────────────────

/// Sending on a [`Queue`] whose receivers are all gone. Carries the value
/// back so the caller can recover it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> std::fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendError(..)")
    }
}
impl<T> std::fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("send on a closed queue")
    }
}
impl<T> std::error::Error for SendError<T> {}

/// Receiving from a [`Queue`] that is closed and drained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecvError;

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("receive on a closed, empty queue")
    }
}
impl std::error::Error for RecvError {}

/// A multi-producer, multi-consumer queue. Clone it to hand out more
/// producer/consumer handles; all clones share one channel.
pub struct Queue<T> {
    tx: async_channel::Sender<T>,
    rx: async_channel::Receiver<T>,
}

// Manual `Clone` — the channel handles are always cloneable regardless of
// `T`, so don't impose `T: Clone` the way `derive` would.
impl<T> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: self.rx.clone(),
        }
    }
}

impl<T> Queue<T> {
    /// A queue that holds at most `cap` items; [`send`](Queue::send) waits
    /// when full (backpressure).
    pub fn bounded(cap: usize) -> Self {
        let (tx, rx) = async_channel::bounded(cap);
        Self { tx, rx }
    }

    /// A queue with no capacity bound; [`send`](Queue::send) never waits.
    pub fn unbounded() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self { tx, rx }
    }

    /// Push a value, waiting if the queue is full. Errors only if every
    /// receiver has been dropped / the queue closed.
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.tx.send(value).await.map_err(|e| SendError(e.0))
    }

    /// Try to push without waiting. Errors if full or closed (the value is
    /// returned either way).
    pub fn try_send(&self, value: T) -> Result<(), SendError<T>> {
        self.tx
            .try_send(value)
            .map_err(|e| SendError(e.into_inner()))
    }

    /// Pop a value, waiting if the queue is empty. Errors once the queue is
    /// closed and drained.
    pub async fn recv(&self) -> Result<T, RecvError> {
        self.rx.recv().await.map_err(|_| RecvError)
    }

    /// Try to pop without waiting. `None` if empty (or closed-and-empty).
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Close the queue: pending and future `send`s fail; `recv` drains the
    /// remaining items then errors. Returns `true` if this call closed it.
    pub fn close(&self) -> bool {
        self.tx.close()
    }

    /// Items currently buffered.
    pub fn len(&self) -> usize {
        self.rx.len()
    }

    /// `true` if no items are buffered.
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use futures_lite::future::block_on;

    #[test]
    fn deferred_first_complete_wins_and_waiters_see_it() {
        let d = Deferred::<u32>::new();
        assert!(!d.is_complete());
        assert_eq!(d.try_get(), None);
        assert!(d.complete(1));
        assert!(!d.complete(2)); // already set
        assert_eq!(d.try_get(), Some(1));
        assert_eq!(block_on(d.wait()), 1);
    }

    #[test]
    fn semaphore_bounds_concurrency() {
        let s = Semaphore::new(2);
        assert_eq!(s.available_permits(), 2);
        let p1 = s.try_acquire().expect("permit 1");
        let _p2 = s.try_acquire().expect("permit 2");
        assert_eq!(s.available_permits(), 0);
        assert!(s.try_acquire().is_none()); // exhausted
        drop(p1);
        assert_eq!(s.available_permits(), 1);
        assert!(block_on(async {
            s.acquire().await;
            true
        }));
    }

    #[test]
    fn queue_send_recv_roundtrip() {
        let q = Queue::<u32>::bounded(4);
        block_on(async {
            q.send(1).await.unwrap();
            q.send(2).await.unwrap();
            assert_eq!(q.len(), 2);
            assert_eq!(q.recv().await.unwrap(), 1);
            assert_eq!(q.recv().await.unwrap(), 2);
        });
    }

    #[test]
    fn queue_try_and_close() {
        let q = Queue::<u32>::unbounded();
        assert!(q.is_empty());
        q.try_send(7).unwrap();
        assert_eq!(q.try_recv(), Some(7));
        assert_eq!(q.try_recv(), None);
        assert!(q.close());
        assert!(block_on(q.recv()).is_err()); // closed + empty
        assert!(matches!(q.try_send(1), Err(SendError(1))));
    }
}
