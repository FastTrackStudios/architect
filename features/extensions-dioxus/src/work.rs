//! Heavy work from a view (`architect_extensions::work`, feature `guest`):
//! a handler starts it on a thread of its own, the view redraws from its
//! state as news comes.
//!
//! ```ignore
//! let primes = use_work();
//! rsx! {
//!     button { onclick: move |_| primes.start(|work| count_primes(work, 2_000_000)), "Find primes" }
//!     progress { value: primes.state().fraction() }
//! }
//! ```

use architect_extensions::work::{Work, WorkHandle, WorkState, WorkStatus, spawn_work};
use serde_json::Value as Json;

/// A piece of work in a view: start it from a handler, read its state
/// as it goes. Copy; reading [`Self::state`] subscribes the component.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct UseWork {
    state: dioxus::prelude::Signal<WorkState>,
    handle: dioxus::prelude::Signal<Option<WorkHandle>>,
}

/// One piece of work for this component, cancelled when it unmounts
/// (its view closed) or when another is started.
#[must_use]
pub fn use_work() -> UseWork {
    use dioxus::prelude::use_signal;
    UseWork {
        state: use_signal(WorkState::default),
        handle: use_signal(|| None),
    }
}

impl UseWork {
    /// Starts `work` on a thread of its own, cancelling one running.
    pub fn start<F>(&self, work: F)
    where
        F: FnOnce(&Work) -> Result<Json, String> + Send + 'static,
    {
        use dioxus::prelude::{WritableExt as _, spawn};
        let handle = spawn_work(work);
        let mut news = handle.news();
        let shared = handle.observer();
        let mut state = self.state;
        state.set(shared.state());
        // The old one, if any, is dropped (cancelled) here.
        let mut slot = self.handle;
        slot.set(Some(handle));
        spawn(async move {
            loop {
                if news.changed().await.is_err() {
                    return;
                }
                let now = shared.state();
                let over = !matches!(now.status, WorkStatus::Running);
                state.set(now);
                if over {
                    return;
                }
            }
        });
    }

    /// Cancels it.
    pub fn cancel(&self) {
        use dioxus::prelude::ReadableExt as _;
        if let Some(h) = self.handle.peek().as_ref() {
            h.cancel();
        }
    }

    /// Where it is. Reactive.
    #[must_use]
    pub fn state(&self) -> WorkState {
        use dioxus::prelude::ReadableExt as _;
        self.state.read().clone()
    }

    /// Whether it is running. Reactive.
    #[must_use]
    pub fn running(&self) -> bool {
        use dioxus::prelude::ReadableExt as _;
        self.state.read().status == WorkStatus::Running
    }
}
