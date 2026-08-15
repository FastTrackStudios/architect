//! [`Mutation`] — the optimistic write primitive, the centerpiece.
//!
//! effect-atom's `Atom.fn` runs an effectful mutation and tracks its
//! pending/result state; here, [`Mutation::run`] ties the whole optimistic
//! lifecycle into one call so a call site *cannot* forget the rollback arm:
//!
//! 1. apply an optimistic `patch` to the [`Store`] (synchronously, in the
//!    event handler — never during render) → a rollback ticket,
//! 2. spawn the async server `call`,
//! 3. on `Ok` → [`Store::reconcile`] (swap temp→real / store the server
//!    copy); on `Err` → [`Store::rollback`] + surface the error.
//!
//! If a [`Notifications`](crate::Notifications) queue is provided at the
//! app root, rollback failures are reported there too — so a mutation that
//! navigated away from its screen (the common optimistic pattern) still
//! tells the user it failed.

use std::fmt::Display;
use std::future::Future;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use crate::notify::{Notifications, try_use_notifications};
use crate::reactivity::{Reactivity, try_use_reactivity};
use crate::store::{MutationId, Store, StoreEntity};

/// A `Copy` handle to one mutation's transient UI state (pending flag +
/// last error). Create with [`use_mutation`]; read in render to disable a
/// button or show an inline error.
pub struct Mutation<E = String> {
    pending: Signal<bool>,
    error: Signal<Option<E>>,
    notify: Option<Notifications>,
    reactivity: Option<Reactivity>,
    invalidate_keys: &'static [&'static str],
}

impl<E: 'static> Clone for Mutation<E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E: 'static> Copy for Mutation<E> {}

/// Hook: one mutation's state for a component. Call once per logical
/// mutation (create, delete, …); reuse the returned handle across renders.
pub fn use_mutation<E: 'static>() -> Mutation<E> {
    Mutation {
        pending: use_signal(|| false),
        error: use_signal(|| None),
        notify: try_use_notifications(),
        reactivity: try_use_reactivity(),
        invalidate_keys: &[],
    }
}

impl<E: Clone + 'static> Mutation<E> {
    /// Invalidate these [`Reactivity`](crate::Reactivity) keys after every
    /// run settles (success *or* rollback) — refreshing derived server
    /// data (search results, counts) the store can't reconcile itself.
    /// No-op when no `Reactivity` was provided at the root.
    pub fn invalidating(mut self, keys: &'static [&'static str]) -> Self {
        self.invalidate_keys = keys;
        self
    }

    /// True while the server call is in flight.
    pub fn is_pending(&self) -> bool {
        *self.pending.read()
    }

    /// The error from the last failed run, if any.
    pub fn error(&self) -> Option<E> {
        self.error.read().clone()
    }

    /// Clear a previously surfaced error (e.g. when the user edits a form).
    pub fn clear_error(&self) {
        let mut error = self.error;
        error.set(None);
    }
}

impl<E: Clone + Display + 'static> Mutation<E> {
    /// Run the optimistic lifecycle: `patch` the store now, then `call` the
    /// server; reconcile on success, roll back + record the error on
    /// failure. `call` returns `Ok(Some(value))` to fold the server's
    /// authoritative row in (create/update), or `Ok(None)` for a body-less
    /// confirmation (delete).
    ///
    /// `Store` and `Mutation` are both `Copy`, so they move into the
    /// spawned future for free — no `clone()` ceremony.
    ///
    /// The server call runs on a `spawn_forever` task so it **survives the
    /// component unmounting** — a delete or create that navigates away the
    /// instant it's issued still reconciles (or rolls back) the shared
    /// store. The store and its rollback snapshots live at the app root, so
    /// they're always there to fold the result into; the per-screen
    /// `pending`/`error` signals may already be gone, so those writes are
    /// guarded (a no-op once the screen is unmounted) and the failure is
    /// *also* reported to the app's [`Notifications`](crate::Notifications)
    /// queue when one was provided.
    pub fn run<F, Fut, T>(
        self,
        store: Store<T, E>,
        patch: impl FnOnce(&Store<T, E>) -> MutationId,
        call: F,
    ) where
        T: StoreEntity,
        F: FnOnce() -> Fut + 'static,
        Fut: Future<Output = Result<Option<T>, E>> + 'static,
    {
        let pending = self.pending;
        let error = self.error;
        let notify = self.notify;
        let reactivity = self.reactivity;
        let invalidate_keys = self.invalidate_keys;
        // Optimistic write happens synchronously, in the handler.
        let ticket = patch(&store);
        quiet_set(pending, true);
        quiet_set(error, None);
        spawn_forever(async move {
            match call().await {
                Ok(value) => store.reconcile(ticket, value),
                Err(e) => {
                    store.rollback(ticket);
                    if let Some(notify) = notify {
                        notify.error(e.to_string());
                    }
                    quiet_set(error, Some(e));
                }
            }
            quiet_set(pending, false);
            // After settling either way, refresh whatever derived reads
            // this mutation declared stale.
            if let Some(reactivity) = reactivity {
                for key in invalidate_keys {
                    reactivity.invalidate(key);
                }
            }
        });
    }
}

/// Write a signal only if its backing scope is still alive. After the
/// owning component unmounts the signal is dropped, and a plain `set`
/// would panic; a mutation that navigated away just no-ops its UI writes.
fn quiet_set<T: 'static>(mut signal: Signal<T>, value: T) {
    if let Ok(mut slot) = signal.try_write() {
        *slot = value;
    }
}
