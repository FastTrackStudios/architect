//! [`use_async`] — a refreshable async value, the single-resource
//! counterpart to [`Store`](crate::Store).
//!
//! effect-atom exposes an async atom you read with `useAtomValue` (getting
//! an `AsyncResult`) and re-run with `useAtomRefresh`. [`use_async`] is the
//! Dioxus analog: it wraps `use_resource` (which already re-runs when any
//! signal the loader reads changes — the "atom family by key" pattern: read
//! the key signal in the loader) and folds its transitions into an
//! [`AtomResult`], retaining the previous value across a refresh. The
//! returned [`Async`] handle exposes [`Async::state`] (read in render and
//! `match` over it) and [`Async::refresh`] (call from an event handler —
//! e.g. after an upload succeeds).

use std::future::Future;

use dioxus::prelude::*;

use crate::result::AtomResult;

/// A `Copy` handle to a refreshable async value.
pub struct Async<T: 'static, E: 'static = String> {
    state: Signal<AtomResult<T, E>>,
    revision: Signal<u64>,
}

impl<T: 'static, E: 'static> Clone for Async<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static, E: 'static> Copy for Async<T, E> {}

impl<T: Clone + 'static, E: Clone + 'static> Async<T, E> {
    /// The current async state (stale-while-revalidate). Read in render and
    /// `match` over it.
    pub fn state(&self) -> AtomResult<T, E> {
        self.state.read().clone()
    }

    /// Re-run the loader, keeping the current value visible until the new
    /// one settles. The Dioxus analog of effect-atom's `useAtomRefresh`.
    pub fn refresh(&self) {
        let mut revision = self.revision;
        let next = *revision.read() + 1;
        revision.set(next);
    }
}

/// Drive an async value into a stale-while-revalidate [`AtomResult`].
///
/// `loader` is called to produce the future on first render, whenever a
/// signal it reads changes, and whenever [`Async::refresh`] is invoked.
/// Read a key signal inside `loader` to get the "atom family by key"
/// behavior (the resource re-runs when the key changes):
///
/// ```ignore
/// let session = use_async(move || {
///     let token = token();              // depend on the route param
///     async move { client.upload_session(token).await.map_err(|e| format!("{e:?}")) }
/// });
/// // in rsx! — a plain, exhaustive match on the phase (with
/// // `use architect::AtomResult::*` the arms are unqualified):
/// match session.state() {
///     Initial | Loading => rsx! { p { "Opening your upload link." } },
///     Reloading(prev) => rsx! { /* keep `prev` visible while refreshing */ },
///     Success(s) => rsx! { UploadForm { session: s, on_uploaded: move |_| session.refresh() } },
///     Error { error, .. } => rsx! { p { class: "error", "{error}" } },
///     Defect { defect, .. } => rsx! { p { class: "error", "{defect}" } },
/// }
/// ```
pub fn use_async<T, E, F, Fut>(loader: F) -> Async<T, E>
where
    T: Clone + 'static,
    E: Clone + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
{
    let mut state = use_signal(|| AtomResult::<T, E>::Initial);
    let revision = use_signal(|| 0u64);

    let resource = use_resource(move || {
        // Subscribe to refresh bumps; the loader's own reads add their deps.
        let _ = revision();
        loader()
    });

    // Fold resource transitions into the AtomResult — in an effect, never
    // during render. `peek` avoids subscribing to `state` inside its own
    // writer (which would loop).
    use_effect(move || {
        let next = match &*resource.read() {
            None => state.peek().clone().begin_reload(),
            Some(outcome) => state.peek().clone().settle(outcome.clone()),
        };
        state.set(next);
    });

    Async { state, revision }
}
