//! [`Store`] — a keyed, optimistic, client-side entity cache.
//!
//! The collection counterpart to effect-atom's `Atom.family` + writable
//! optimistic atoms. A `Store<T>` holds the canonical merged set of an
//! entity keyed by [`Id`], plus the backing list-fetch phase
//! ([`AtomResult`]) and a registry of rollback snapshots. Mutations patch
//! it optimistically and either *reconcile* (swap the optimistic value for
//! the server's authoritative one) or *roll back* (restore the prior
//! state) — see [`crate::Mutation`].
//!
//! All mutating logic lives on the inner [`StoreData`] as pure `&mut self`
//! methods (unit-testable without a Dioxus runtime); [`Store`] is the thin
//! `Signal` wrapper that makes it reactive and `Copy`.

use std::collections::HashMap;
use std::hash::Hash;

use dioxus::prelude::*;

use crate::id::Id;
use crate::result::AtomResult;

/// An opaque ticket identifying one in-flight mutation, so its rollback
/// snapshot can be found again when the server responds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MutationId(u64);

/// An entity that can live in a [`struct@Store`]: cloneable, comparable, and able
/// to report its stable key.
pub trait StoreEntity: Clone + PartialEq + 'static {
    /// The server-assigned identity type (e.g. `Uuid`).
    type Key: Clone + Eq + Hash + 'static;
    /// This value's stable key.
    fn key(&self) -> Self::Key;
}

/// What to undo if a mutation fails. Stored inside [`StoreData`] (not in a
/// component) so a rollback survives the component unmounting mid-flight —
/// e.g. a delete that navigates away before the server responds.
enum Snapshot<T: StoreEntity> {
    /// Insert: undo = remove this temp row.
    Inserted(Id<T::Key>),
    /// Update: undo = restore the prior value at this id.
    Updated(Id<T::Key>, T),
    /// Remove: undo = reinsert the value at its prior order position.
    Removed {
        id: Id<T::Key>,
        value: T,
        index: usize,
    },
}

/// The inner store value: the merged entity map, a stable ordering, the
/// backing list-fetch phase, the rollback registry, and a monotonic
/// sequence used for both temp ids and mutation tickets.
pub struct StoreData<T: StoreEntity, E = String> {
    items: HashMap<Id<T::Key>, T>,
    order: Vec<Id<T::Key>>,
    status: AtomResult<(), E>,
    rollback: HashMap<MutationId, Snapshot<T>>,
    seq: u64,
    /// Re-runs the backing list/entity fetch — registered by
    /// [`use_store_list`] / [`use_store_entry`] and invoked by
    /// [`Store::reload`]. `None` until the first fetch hook mounts.
    reloader: Option<Callback<()>>,
}

impl<T: StoreEntity, E> Default for StoreData<T, E> {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
            order: Vec::new(),
            status: AtomResult::Initial,
            rollback: HashMap::new(),
            seq: 0,
            reloader: None,
        }
    }
}

impl<T: StoreEntity, E: Clone> StoreData<T, E> {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Replace the canonical set from a fresh server list, **preserving any
    /// still-in-flight optimistic temp rows** so a concurrent refetch can't
    /// clobber an optimistic insert that hasn't reconciled yet.
    fn hydrate(&mut self, server: impl IntoIterator<Item = T>) {
        let temp_rows: Vec<(Id<T::Key>, T)> = self
            .order
            .iter()
            .filter(|id| id.is_temp())
            .filter_map(|id| self.items.get(id).map(|v| (id.clone(), v.clone())))
            .collect();

        self.items.clear();
        self.order.clear();
        for v in server {
            let id = Id::Real(v.key());
            if self.items.insert(id.clone(), v).is_none() {
                self.order.push(id);
            }
        }
        for (id, v) in temp_rows {
            self.items.insert(id.clone(), v);
            self.order.push(id);
        }
        self.status = AtomResult::Success(());
    }

    /// Optimistically append a row with a forged temp id; returns the
    /// rollback ticket and the temp id.
    fn insert_optimistic(&mut self, value: T) -> (MutationId, Id<T::Key>) {
        let ticket = MutationId(self.next_seq());
        let id = Id::Temp(self.next_seq());
        self.items.insert(id.clone(), value);
        self.order.push(id.clone());
        self.rollback.insert(ticket, Snapshot::Inserted(id.clone()));
        (ticket, id)
    }

    /// Optimistically patch an existing row, snapshotting the prior value.
    /// No-op (but still returns a ticket) if the id isn't present.
    fn update_optimistic(&mut self, id: Id<T::Key>, patch: impl FnOnce(&mut T)) -> MutationId {
        let ticket = MutationId(self.next_seq());
        if let Some(prior) = self.items.get(&id).cloned() {
            let mut next = prior.clone();
            patch(&mut next);
            self.items.insert(id.clone(), next);
            self.rollback.insert(ticket, Snapshot::Updated(id, prior));
        }
        ticket
    }

    /// Optimistically remove a row, snapshotting it + its order position.
    fn remove_optimistic(&mut self, id: Id<T::Key>) -> MutationId {
        let ticket = MutationId(self.next_seq());
        if let Some(value) = self.items.remove(&id) {
            let index = self.order.iter().position(|x| *x == id).unwrap_or(0);
            if index < self.order.len() {
                self.order.remove(index);
            }
            self.rollback
                .insert(ticket, Snapshot::Removed { id, value, index });
        }
        ticket
    }

    /// Drop the snapshot and fold the server's authoritative value in: for
    /// an insert, swap the temp id for the real one; for an update, store
    /// the server copy; for a remove, the row stays gone.
    fn reconcile(&mut self, ticket: MutationId, server_value: Option<T>) {
        let Some(snapshot) = self.rollback.remove(&ticket) else {
            return;
        };
        match (snapshot, server_value) {
            (Snapshot::Inserted(temp_id), Some(v)) => {
                let real = Id::Real(v.key());
                if let Some(pos) = self.order.iter().position(|x| *x == temp_id) {
                    self.order[pos] = real.clone();
                }
                self.items.remove(&temp_id);
                self.items.insert(real, v);
            }
            (Snapshot::Inserted(temp_id), None) => {
                self.items.remove(&temp_id);
                self.order.retain(|x| *x != temp_id);
            }
            (Snapshot::Updated(id, _prior), Some(v)) => {
                self.items.insert(id, v);
            }
            // Server confirmed but returned no body — keep the optimistic
            // value (it's our best guess) / leave the removal in place.
            (Snapshot::Updated(_, _), None) => {}
            (Snapshot::Removed { .. }, _) => {}
        }
    }

    /// Restore the snapshot for a failed mutation.
    fn rollback(&mut self, ticket: MutationId) {
        let Some(snapshot) = self.rollback.remove(&ticket) else {
            return;
        };
        match snapshot {
            Snapshot::Inserted(id) => {
                self.items.remove(&id);
                self.order.retain(|x| *x != id);
            }
            Snapshot::Updated(id, prior) => {
                self.items.insert(id, prior);
            }
            Snapshot::Removed { id, value, index } => {
                let idx = index.min(self.order.len());
                self.order.insert(idx, id.clone());
                self.items.insert(id, value);
            }
        }
    }

    fn list(&self) -> Vec<T> {
        self.order
            .iter()
            .filter_map(|id| self.items.get(id).cloned())
            .collect()
    }

    fn get(&self, id: &Id<T::Key>) -> Option<T> {
        self.items.get(id).cloned()
    }

    fn entries(&self) -> Vec<(Id<T::Key>, T)> {
        self.order
            .iter()
            .filter_map(|id| self.items.get(id).map(|v| (id.clone(), v.clone())))
            .collect()
    }

    /// Insert or replace an authoritative row by its real key (no rollback
    /// tracking) — used to fold a single-row fetch (a deep link to a detail
    /// page) into the cache without disturbing the rest.
    fn put(&mut self, value: T) {
        let id = Id::Real(value.key());
        if self.items.insert(id.clone(), value).is_none() {
            self.order.push(id);
        }
    }

    /// Remove an authoritative row by its real key (no rollback tracking)
    /// — used to fold a server-pushed deletion into the cache.
    fn remove_real(&mut self, key: &T::Key) {
        let id = Id::Real(key.clone());
        self.items.remove(&id);
        self.order.retain(|x| *x != id);
    }
}

/// A cheap `Copy` handle over a reactive [`StoreData`]. Safe to capture in
/// event handlers and spawned futures with no ceremony, exactly like the
/// vox client handles the example already passes around.
pub struct Store<T: StoreEntity, E = String> {
    inner: Signal<StoreData<T, E>>,
}

impl<T: StoreEntity, E: 'static> Clone for Store<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: StoreEntity, E: 'static> Copy for Store<T, E> {}

/// Create a fresh, empty store. Provide it once at the app root via
/// `use_context_provider`, alongside the connection signal, and pull it in
/// screens with `use_context`.
///
/// The backing signal is owned by [`ScopeId::ROOT`], not the calling
/// component's scope. A store is read and written by detached tasks that
/// outlive any screen — the `spawn_forever` reconcile/rollback in
/// [`Mutation::run`](crate::Mutation::run) runs in the root context, so a
/// store owned by a transient provider scope would trip the dioxus
/// "value used outside its owning scope" warning (and risk a
/// use-after-drop) once that provider unmounts. Rooting it makes the
/// "the store lives at the app root" contract real.
pub fn use_store<T: StoreEntity, E: 'static>() -> Store<T, E> {
    let inner = use_hook(|| Signal::new_in_scope(StoreData::default(), ScopeId::ROOT));
    Store { inner }
}

impl<T: StoreEntity, E: Clone + 'static> Store<T, E> {
    /// Wrap an existing signal (e.g. one created with `use_context_provider`).
    pub fn from_signal(inner: Signal<StoreData<T, E>>) -> Self {
        Self { inner }
    }

    /// Replace the canonical set from a server list (keeps in-flight temp
    /// rows). Sets the backing phase to `Success`.
    pub fn hydrate(&self, server: impl IntoIterator<Item = T>) {
        let mut inner = self.inner;
        inner.write().hydrate(server);
    }

    /// Mark the backing list fetch as in flight (keeps any current value).
    pub fn set_loading(&self) {
        let mut inner = self.inner;
        let mut data = inner.write();
        let status = std::mem::take(&mut data.status);
        data.status = status.begin_reload();
    }

    /// Mark the backing list fetch as failed (keeps any current value).
    pub fn set_failure(&self, error: E) {
        let mut inner = self.inner;
        let mut data = inner.write();
        let status = std::mem::take(&mut data.status);
        data.status = status.settle(Err(error));
    }

    /// Register the restart for the fetch backing this store. Called by
    /// [`use_store_list`] / [`use_store_entry`]; the latest registration
    /// wins (a store backs one live fetch at a time).
    pub fn set_reloader(&self, reloader: Callback<()>) {
        let mut inner = self.inner;
        inner.write().reloader = Some(reloader);
    }

    /// Re-run the backing fetch — a manual retry/refresh. Drives the
    /// phase through `Reloading` (the last good value stays visible) to
    /// a fresh `Success`/`Error`. No-op before any fetch hook has
    /// mounted. Wire it to an error-state **Try again** button.
    pub fn reload(&self) {
        // Copy the callback out before calling so we don't hold a read
        // borrow across the restart.
        let reloader = self.inner.read().reloader;
        if let Some(reload) = reloader {
            reload.call(());
        }
    }

    /// Optimistically insert a row; returns `(ticket, temp_id)`.
    pub fn insert_optimistic(&self, value: T) -> (MutationId, Id<T::Key>) {
        let mut inner = self.inner;
        inner.write().insert_optimistic(value)
    }

    /// Optimistically patch a row; returns the rollback ticket.
    pub fn update_optimistic(&self, id: Id<T::Key>, patch: impl FnOnce(&mut T)) -> MutationId {
        let mut inner = self.inner;
        inner.write().update_optimistic(id, patch)
    }

    /// Optimistically remove a row; returns the rollback ticket.
    pub fn remove_optimistic(&self, id: Id<T::Key>) -> MutationId {
        let mut inner = self.inner;
        inner.write().remove_optimistic(id)
    }

    /// Server confirmed: drop the snapshot and store the authoritative
    /// value (swapping a temp id for the real one on insert).
    pub fn reconcile(&self, ticket: MutationId, server_value: Option<T>) {
        let mut inner = self.inner;
        inner.write().reconcile(ticket, server_value);
    }

    /// Server failed: restore the snapshot for this mutation.
    pub fn rollback(&self, ticket: MutationId) {
        let mut inner = self.inner;
        inner.write().rollback(ticket);
    }

    /// The ordered snapshot of rows. Call inside a `use_memo`.
    pub fn list(&self) -> Vec<T> {
        self.inner.read().list()
    }

    /// Insert or replace an authoritative row by its real key, without
    /// rollback tracking. Use to fold a single-row fetch (e.g. a deep link
    /// to a detail page that wasn't in the list yet) or a server-pushed
    /// upsert event into the cache.
    pub fn put(&self, value: T) {
        let mut inner = self.inner;
        inner.write().put(value);
    }

    /// Remove an authoritative row by its real key, without rollback
    /// tracking. Use to fold a server-pushed deletion into the cache.
    pub fn remove_real(&self, key: &T::Key) {
        let mut inner = self.inner;
        inner.write().remove_real(key);
    }

    /// The ordered `(id, row)` pairs — use when the view needs the [`Id`]
    /// itself (a stable list `key`, or to tell a temp row from a real one).
    /// Call inside a `use_memo`.
    pub fn entries(&self) -> Vec<(Id<T::Key>, T)> {
        self.inner.read().entries()
    }

    /// The list as a single [`AtomResult`]: the rows are the value, the
    /// backing fetch is the phase. Lets a view render the whole list with
    /// one plain `match` (loading / refreshing-with-stale-rows / success /
    /// error-keeping-stale-rows), the same idiom [`use_async`] uses for a
    /// single resource. Call inside a `use_memo`.
    ///
    /// [`use_async`]: crate::use_async
    #[allow(clippy::type_complexity)] // `AtomResult<Vec<(Id, T)>, E>` reads fine.
    pub fn entries_result(&self) -> AtomResult<Vec<(Id<T::Key>, T)>, E> {
        let data = self.inner.read();
        let entries: Vec<(Id<T::Key>, T)> = data
            .order
            .iter()
            .filter_map(|id| data.items.get(id).map(|v| (id.clone(), v.clone())))
            .collect();
        // `status` is `AtomResult<(), E>`; substitute the current rows as
        // the value so `Reloading`/`Failure{last}` carry the stale rows.
        data.status.clone().map(move |()| entries)
    }

    /// One row by id (real or temp). Call inside a `use_memo`.
    pub fn get(&self, id: &Id<T::Key>) -> Option<T> {
        self.inner.read().get(id)
    }

    /// One row by its real server key.
    pub fn get_real(&self, key: T::Key) -> Option<T> {
        self.inner.read().get(&Id::Real(key))
    }

    /// The backing list-fetch phase (for a stale-while-revalidate banner).
    pub fn status(&self) -> AtomResult<(), E> {
        self.inner.read().status.clone()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.inner.read().order.is_empty()
    }
}

/// Store-backed list read — the generic behind a feature's
/// `use_<entity>_list()` hook.
///
/// Drives `fetch` through `use_resource` (it re-runs when any signal the
/// closure reads changes — connection state, a search query, a
/// [`Reactivity`](crate::Reactivity) key), folds the outcome into the
/// store (`hydrate` preserves in-flight optimistic rows), and returns the
/// rows as one [`AtomResult`] for the page to `match`. `fetch` returns
/// `None` while the backend isn't reachable yet, which keeps the phase at
/// `Loading` instead of an error.
#[allow(clippy::type_complexity)] // `AtomResult<Vec<(Id, T)>, E>` reads fine.
pub fn use_store_list<T, E, F, Fut>(
    store: Store<T, E>,
    fetch: F,
) -> AtomResult<Vec<(Id<T::Key>, T)>, E>
where
    T: StoreEntity,
    E: Clone + PartialEq + 'static,
    F: Fn() -> Fut + 'static,
    Fut: std::future::Future<Output = Option<Result<Vec<T>, E>>> + 'static,
{
    let mut fetched = use_resource(fetch);
    // Expose the resource's restart through the store so consumers can
    // trigger a manual retry (`store.reload()`). Registered once on
    // mount; the callback is render-stable, so no write churn.
    let reload = use_callback(move |()| fetched.restart());
    use_effect(move || store.set_reloader(reload));
    use_effect(move || match &*fetched.read() {
        Some(Some(Ok(items))) => store.hydrate(items.clone()),
        Some(Some(Err(e))) => store.set_failure(e.clone()),
        _ => store.set_loading(),
    });
    store.entries_result()
}

/// Cache-first single-entity read backed by a [`struct@Store`] — the generic
/// behind a feature's `use_<entity>(id)` hook.
///
/// Returns `Success` straight from the store if the keyed row is cached
/// (instant after a list visit, and it reflects optimistic edits), else
/// drives a fallback fetch and folds the result into the store. `parse`
/// turns the route `:id` string into the key (re-runs reactively when `id`
/// changes); `fetch` performs the request for a key and returns `None`
/// while the backend isn't reachable yet (so the phase stays `Loading`, not
/// `Error`).
///
/// ```ignore
/// pub fn use_example(id: String) -> AtomResult<Example> {
///     let conn = use_conn();
///     let store = use_example_store();
///     use_store_entry(store, id, parse_id, move |uuid| async move {
///         let clients = match &*conn.read() {
///             ConnState::Ready(c) => c.clone(),
///             ConnState::Connecting => return None,           // -> Loading
///             ConnState::Failed(e) => return Some(Err(e.clone())),
///         };
///         Some(clients.repo.get(uuid).await.map_err(|e| format!("{e:?}")))
///     })
/// }
/// ```
pub fn use_store_entry<T, E, Fut>(
    store: Store<T, E>,
    id: String,
    parse: fn(&str) -> Result<T::Key, E>,
    fetch: impl Fn(T::Key) -> Fut + 'static,
) -> AtomResult<T, E>
where
    T: StoreEntity,
    E: Clone + PartialEq + 'static,
    Fut: std::future::Future<Output = Option<Result<T, E>>> + 'static,
{
    let parsed = use_memo(use_reactive!(|id| parse(&id)));

    let mut fetched = use_resource(move || {
        // Only fetch on a cache miss. (`T::Key` is only `Clone`, not
        // necessarily `Copy` — String keys — so the guard probes the
        // cache with a clone and the arm moves the key into `fetch`.)
        let pending = match parsed() {
            Ok(key) if store.get_real(key.clone()).is_none() => Some(fetch(key)),
            _ => None,
        };
        async move {
            match pending {
                Some(fut) => fut.await,
                None => None,
            }
        }
    });

    // Expose the resource's restart through the store (manual retry).
    let reload = use_callback(move |()| fetched.restart());
    use_effect(move || store.set_reloader(reload));

    use_effect(move || {
        if let Some(Some(Ok(value))) = &*fetched.read() {
            store.put(value.clone());
        }
    });

    match parsed() {
        Err(error) => AtomResult::Error { error, last: None },
        Ok(key) => {
            if let Some(value) = store.get_real(key) {
                return AtomResult::Success(value);
            }
            match &*fetched.read() {
                Some(Some(Ok(value))) => AtomResult::Success(value.clone()),
                Some(Some(Err(e))) => AtomResult::Error {
                    error: e.clone(),
                    last: None,
                },
                _ => AtomResult::Loading,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct Row {
        id: u32,
        name: String,
    }
    impl StoreEntity for Row {
        type Key = u32;
        fn key(&self) -> u32 {
            self.id
        }
    }

    fn data() -> StoreData<Row, String> {
        StoreData::default()
    }

    #[test]
    fn hydrate_orders_and_keys_by_real_id() {
        let mut d = data();
        d.hydrate([
            Row {
                id: 1,
                name: "a".into(),
            },
            Row {
                id: 2,
                name: "b".into(),
            },
        ]);
        assert_eq!(d.list().iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2]);
        assert!(d.status.is_success());
    }

    #[test]
    fn insert_then_reconcile_swaps_temp_for_real_id() {
        let mut d = data();
        let (ticket, temp) = d.insert_optimistic(Row {
            id: 0,
            name: "draft".into(),
        });
        assert!(temp.is_temp());
        assert_eq!(d.list().len(), 1);

        d.reconcile(
            ticket,
            Some(Row {
                id: 99,
                name: "draft".into(),
            }),
        );
        // temp row replaced by the server row, same position, real id.
        assert_eq!(d.get(&temp), None);
        assert_eq!(d.get(&Id::Real(99)).unwrap().id, 99);
        assert_eq!(d.list().len(), 1);
    }

    #[test]
    fn insert_then_rollback_removes_the_temp_row() {
        let mut d = data();
        let (ticket, temp) = d.insert_optimistic(Row {
            id: 0,
            name: "draft".into(),
        });
        d.rollback(ticket);
        assert_eq!(d.get(&temp), None);
        assert!(d.list().is_empty());
    }

    #[test]
    fn update_rollback_restores_prior_value() {
        let mut d = data();
        d.hydrate([Row {
            id: 1,
            name: "before".into(),
        }]);
        let ticket = d.update_optimistic(Id::Real(1), |r| r.name = "after".into());
        assert_eq!(d.get_real_name(1), "after");
        d.rollback(ticket);
        assert_eq!(d.get_real_name(1), "before");
    }

    #[test]
    fn remove_rollback_restores_value_and_position() {
        let mut d = data();
        d.hydrate([
            Row {
                id: 1,
                name: "a".into(),
            },
            Row {
                id: 2,
                name: "b".into(),
            },
            Row {
                id: 3,
                name: "c".into(),
            },
        ]);
        let ticket = d.remove_optimistic(Id::Real(2));
        assert_eq!(d.list().iter().map(|r| r.id).collect::<Vec<_>>(), [1, 3]);
        d.rollback(ticket);
        // restored to its original middle position.
        assert_eq!(d.list().iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn hydrate_preserves_in_flight_temp_rows() {
        let mut d = data();
        d.hydrate([Row {
            id: 1,
            name: "a".into(),
        }]);
        let (_ticket, temp) = d.insert_optimistic(Row {
            id: 0,
            name: "draft".into(),
        });
        // a concurrent refetch lands while the insert is still in flight.
        d.hydrate([Row {
            id: 1,
            name: "a".into(),
        }]);
        assert!(
            d.get(&temp).is_some(),
            "optimistic row must survive a refetch"
        );
        assert_eq!(d.list().len(), 2);
    }

    // test-only helper
    impl StoreData<Row, String> {
        fn get_real_name(&self, id: u32) -> String {
            self.get(&Id::Real(id)).unwrap().name
        }
    }
}
