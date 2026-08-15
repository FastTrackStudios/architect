//! Client-side stream consumption — **live reads**.
//!
//! The counterpart to [`PubSub`](crate::PubSub): a component
//! subscribes to a server event stream for its lifetime, and each event
//! folds into client state. With [`use_store_stream`] the target is the
//! optimistic [`struct@Store`] — every page already
//! rendering from the store goes **live** (rows appear/update/vanish as
//! other clients or the host change them) with zero page changes, and
//! optimistic local writes overlay incoming server truth exactly as they
//! do over fetches.
//!
//! Both hooks ride `use_resource`, so the subscription **re-establishes
//! itself** whenever a signal the `subscribe` future reads changes — read
//! the [`Connection`](architect_atom::Connection) inside it and a
//! reconnect resubscribes automatically.
//!
//! ```ignore
//! // feature hook: live entity events feed the derived store
//! pub fn use_example_events() {
//!     let conn = use_connection::<ExampleEventsClient>();
//!     let store = use_example_store();
//!     use_store_stream(store, move |sink| async move {
//!         match conn.state() {
//!             ConnectionState::Ready(c) => c.subscribe(sink).await.is_ok(),
//!             _ => false,                       // not up yet — retried on Ready
//!         }
//!     }, |store, ev: ExampleEvent| match ev {
//!         ExampleEvent::Upserted(row) => store.put(row),
//!         ExampleEvent::Deleted(id) => store.remove(id),
//!     });
//! }
//! ```

use std::future::Future;

use architect_atom::dioxus::prelude::*;
use architect_atom::{Store, StoreEntity};

/// Subscribe to a server stream for the lifetime of the calling
/// component, handing each received event to `on_event`.
///
/// **The subscription heals itself.** A stream ends for reasons that are not
/// the caller's fault and are not permanent: the server restarts, a hub drops
/// a subscriber that fell behind a burst, a socket blips. Without a retry the
/// component keeps rendering its last-known state — a UI that looks alive and
/// is deaf, which is the worst way for a live rig to fail, because you find
/// out when a control does nothing mid-song. So an ended subscription is
/// re-established after a backoff: ~400 ms when the stream had been running
/// (a restart, most likely) and 1s doubling to 8s when it never established
/// (nothing is listening yet), so a dead engine is waited for rather than
/// hammered, and the moment it comes back the UI is live again.
///
/// `subscribe` is given the fresh [`vox::Tx`] to pass into the service's
/// subscribe method. vox scopes channels to their request, so subscribe
/// hosts hold the call **in flight** for the life of the subscription —
/// the returned future staying pending *is* the healthy state; events pump
/// concurrently while it runs. It resolving means the subscription ended:
/// return `false` for "couldn't establish" (connection not up — the hook
/// re-runs when whatever signals the future read change, so it retries on
/// `Ready`); a server-side subscribe error resolves it likewise.
/// Unsubscribe is automatic: unmounting (or a hook re-run) drops the
/// in-flight call, which cancels the request and closes the channel.
pub fn use_stream<Ev, F, Fut, A>(subscribe: F, on_event: A)
where
    Ev: Clone + facet::Facet<'static> + 'static,
    F: Fn(vox::Tx<Ev>) -> Fut + 'static,
    Fut: Future<Output = bool> + 'static,
    A: Fn(Ev) + Clone + 'static,
{
    // Bumped when a subscription ends: the resource reads it, so writing it
    // re-runs this hook and subscribes again.
    let mut generation = use_signal(|| 0u32);
    // Consecutive failures to establish — the backoff's only state.
    let mut misses = use_signal(|| 0u32);
    use_resource(move || {
        let _ = generation();
        let (tx, mut rx) = vox::channel::<Ev>();
        let call = subscribe(tx);
        let on_event = on_event.clone();
        async move {
            // Pump until the stream ends (server gone, channel closed) or
            // the component unmounts / the subscription restarts (this
            // future is dropped either way).
            let pump = async move {
                while let Ok(Some(event)) = rx.recv().await {
                    // `SelfRef` has no owned extraction (its value may borrow
                    // from the receive buffer); `map` lends us the value by
                    // move while the buffer is alive, so cloning out is sound
                    // — our `Ev: Clone + 'static` events own their data.
                    let mut owned: Option<Ev> = None;
                    let _ = event.map(|ev| owned = Some(ev.clone()));
                    if let Some(ev) = owned {
                        on_event(ev);
                    }
                }
            };
            // Race the in-flight subscribe call against the pump: the call
            // resolving means the subscription is over (couldn't establish,
            // server error, or server-side stream end) — stop either way.
            let mut call = core::pin::pin!(call);
            let mut pump = core::pin::pin!(pump);
            // `Some(established)` when the call resolved, `None` when the pump
            // ended first — either way the subscription is over, and whether
            // it ever established decides how long to wait before retrying.
            let outcome = core::future::poll_fn(move |cx| {
                use core::future::Future;
                use core::task::Poll;
                if let Poll::Ready(established) = call.as_mut().poll(cx) {
                    return Poll::Ready(Some(established));
                }
                pump.as_mut().poll(cx).map(|()| None)
            })
            .await;

            // A pump that ended had events flowing, so treat it as a live
            // stream that stopped rather than one that never started.
            let was_live = !matches!(outcome, Some(false));
            let wait_ms = if was_live {
                misses.set(0);
                400
            } else {
                let n = *misses.peek();
                misses.set((n + 1).min(4));
                1000u64 << n.min(3)
            };
            crate::platform::sleep(core::time::Duration::from_millis(wait_ms)).await;
            generation += 1;
        }
    });
}

/// [`use_stream`] aimed at the optimistic [`struct@Store`]: each event folds into
/// the shared cache via `apply`, so every store-rendered page is live.
///
/// `apply` is a plain `fn` — typically a `match` mapping the feature's
/// event enum onto [`Store::put`] / [`Store::remove_real`] /
/// [`Store::hydrate`].
pub fn use_store_stream<T, E, Ev, F, Fut>(
    store: Store<T, E>,
    subscribe: F,
    apply: fn(&Store<T, E>, Ev),
) where
    T: StoreEntity,
    E: Clone + PartialEq + 'static,
    Ev: Clone + facet::Facet<'static> + 'static,
    F: Fn(vox::Tx<Ev>) -> Fut + 'static,
    Fut: Future<Output = bool> + 'static,
{
    use_stream(subscribe, move |event| apply(&store, event));
}
