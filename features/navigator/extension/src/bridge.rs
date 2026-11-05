//! Between the actions and the view.
//!
//! An action runs wherever the host runs it (any thread, linked in or in
//! its own process); the navigator lives in the view that draws it. An
//! action posts an [`Ask`] and waits;
//! the view mounted takes each one in turn, does it to its navigator, and
//! answers. No view on show, no navigator: the action says so.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_channel::oneshot;
use serde_json::Value;

/// How long an action waits for the view to answer.
pub const WAIT: Duration = Duration::from_secs(5);

/// One action, for the view to do.
pub struct Ask {
    pub action: String,
    pub args: Value,
    pub answer: oneshot::Sender<Result<Value, String>>,
}

static ASKS: Mutex<VecDeque<Ask>> = Mutex::new(VecDeque::new());

/// How many views are mounted now.
static MOUNTED: AtomicU64 = AtomicU64::new(0);

fn asks() -> MutexGuard<'static, VecDeque<Ask>> {
    ASKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asks the mounted view to do `action` with `args`, and waits for it.
///
/// # Errors
///
/// No navigator is on show, or it did not answer in [`WAIT`], or the
/// action failed.
pub async fn ask(action: &str, args: Value) -> Result<Value, String> {
    if MOUNTED.load(Ordering::Acquire) == 0 {
        return Err("the navigator is not on show (open its sidebar view)".to_owned());
    }
    let (tx, rx) = oneshot::channel();
    asks().push_back(Ask {
        action: action.to_owned(),
        args,
        answer: tx,
    });
    let timeout = futures_timer::Delay::new(WAIT);
    first(rx, timeout).await
}

/// The answer, or why there is none once `timeout` fires.
async fn first(
    rx: oneshot::Receiver<Result<Value, String>>,
    timeout: futures_timer::Delay,
) -> Result<Value, String> {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct Either {
        rx: oneshot::Receiver<Result<Value, String>>,
        timeout: futures_timer::Delay,
    }
    impl Future for Either {
        type Output = Result<Value, String>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if let Poll::Ready(r) = Pin::new(&mut self.rx).poll(cx) {
                return Poll::Ready(
                    r.unwrap_or_else(|_| Err("the navigator went away".to_owned())),
                );
            }
            if Pin::new(&mut self.timeout).poll(cx).is_ready() {
                return Poll::Ready(Err("the navigator did not answer".to_owned()));
            }
            Poll::Pending
        }
    }
    Either { rx, timeout }.await
}

/// The asks waiting, taken.
#[must_use]
pub fn take() -> Vec<Ask> {
    asks().drain(..).collect()
}

/// Held while a view is mounted.
pub struct Mounted;

impl Mounted {
    #[must_use]
    pub fn new() -> Self {
        MOUNTED.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Default for Mounted {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        MOUNTED.fetch_sub(1, Ordering::AcqRel);
    }
}
