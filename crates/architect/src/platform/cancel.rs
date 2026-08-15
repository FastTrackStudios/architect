//! Cooperative, hierarchical cancellation.
//!
//! A [`CancellationToken`] is a shared "stop" signal. Hand clones to long-
//! running work; when something calls [`cancel`](CancellationToken::cancel),
//! every holder observing the token (via [`is_cancelled`](CancellationToken::is_cancelled)
//! or `.cancelled().await`) sees it. [`child_token`](CancellationToken::child_token)
//! builds a tree: cancelling a parent cancels all descendants, but a child
//! cancel stays local. Pure async — wasm-clean (`tokio::sync::Notify`).

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::Notify;

use super::task::{Either, race};

struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
    children: Mutex<Vec<Weak<Inner>>>,
}

impl Inner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
            children: Mutex::new(Vec::new()),
        })
    }

    fn cancel(&self) -> bool {
        let first = !self.cancelled.swap(true, Ordering::SeqCst);
        if first {
            self.notify.notify_waiters();
            // Propagate downward, then forget the children (cancellation is
            // one-shot, so we never need to revisit them).
            let drained: Vec<_> = self.children.lock().unwrap().drain(..).collect();
            for weak in drained {
                if let Some(child) = weak.upgrade() {
                    child.cancel();
                }
            }
        }
        first
    }
}

/// A shared cancellation signal. Cheap to clone (an `Arc` bump); all clones
/// of the same token share one state.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

impl CancellationToken {
    /// A fresh, un-cancelled token.
    pub fn new() -> Self {
        Self {
            inner: Inner::new(),
        }
    }

    /// A child token. Cancelling `self` (or any ancestor) cancels this child;
    /// cancelling the child does **not** affect the parent.
    pub fn child_token(&self) -> Self {
        let child = Inner::new();
        if self.inner.cancelled.load(Ordering::SeqCst) {
            // Parent already cancelled — propagate immediately rather than
            // registering (the parent will never drain again).
            child.cancel();
        } else {
            self.inner
                .children
                .lock()
                .unwrap()
                .push(Arc::downgrade(&child));
        }
        Self { inner: child }
    }

    /// Trigger cancellation. Returns `true` if this call was the one that
    /// flipped the token (idempotent — later calls return `false`).
    pub fn cancel(&self) -> bool {
        self.inner.cancel()
    }

    /// Non-blocking: has this token been cancelled?
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Resolve once this token is cancelled (immediately if already so).
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            // Arm the waiter, *then* re-check, so a `cancel` racing between
            // the first check and the await can't be lost.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `fut` until it completes (`Some`) or `token` is cancelled (`None`).
/// The future is dropped on cancellation.
pub async fn run_until_cancelled<F: Future>(
    token: &CancellationToken,
    fut: F,
) -> Option<F::Output> {
    match race(fut, token.cancelled()).await {
        Either::Left(value) => Some(value),
        Either::Right(()) => None,
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use futures_lite::future::{block_on, or, pending};

    #[test]
    fn cancel_is_idempotent() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        assert!(t.cancel());
        assert!(t.is_cancelled());
        assert!(!t.cancel()); // second call is a no-op
    }

    #[test]
    fn cancel_propagates_to_children_and_grandchildren() {
        let root = CancellationToken::new();
        let child = root.child_token();
        let grandchild = child.child_token();
        root.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn child_cancel_does_not_affect_parent() {
        let root = CancellationToken::new();
        let child = root.child_token();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!root.is_cancelled());
    }

    #[test]
    fn child_of_already_cancelled_parent_is_born_cancelled() {
        let root = CancellationToken::new();
        root.cancel();
        let child = root.child_token();
        assert!(child.is_cancelled());
    }

    #[test]
    fn cancelled_future_wakes_on_cancel() {
        let token = CancellationToken::new();
        let woke = std::sync::Arc::new(AtomicBool::new(false));
        let w2 = woke.clone();
        let t2 = token.clone();
        block_on(or(
            async move {
                t2.cancelled().await;
                w2.store(true, Ordering::SeqCst);
            },
            async {
                assert!(!woke.load(Ordering::SeqCst));
                token.cancel();
                pending::<()>().await
            },
        ));
        assert!(woke.load(Ordering::SeqCst));
    }

    #[test]
    fn run_until_cancelled_reports_both_outcomes() {
        let token = CancellationToken::new();
        // completes before any cancel
        assert_eq!(
            block_on(run_until_cancelled(&token, std::future::ready(7u32))),
            Some(7)
        );
        // cancelled mid-flight
        let t2 = token.clone();
        let out = block_on(or(
            run_until_cancelled(&token, pending::<u32>()),
            async move {
                t2.cancel();
                pending::<Option<u32>>().await
            },
        ));
        assert_eq!(out, None);
    }
}
