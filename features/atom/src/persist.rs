//! Local rows: a store's last-seen list kept on the device, so a list the
//! person saw is a list they can see again with no connection.
//!
//! Three pieces, all behind the `persist` feature:
//!
//! - [`LocalRows`] — where rows live: generic tables keyed by
//!   `(kind, scope, key)`, each row optionally *owned* by something (a
//!   document) so a projection can replace everything one owner derived
//!   ([`LocalRows::project`]). Implementations: [`MemoryRows`] (tests, and
//!   the fallback when nothing durable exists), `LocalStorageRows` (the
//!   browser, size-guarded), and `TursoRows` (native, the `turso`
//!   feature). The same object also holds the durable call queue
//!   ([`crate::CallQueue`]).
//! - [`provide_local_rows`] / [`try_use_local_rows`] — the app-root
//!   context the hooks read. Not provided = online-only, exactly as before.
//! - [`use_persisted_store_list`] — [`use_store_list`](crate::use_store_list)
//!   with the device copy in front of it: the local rows are loaded (and
//!   shown, [`Store::is_stale`]) independently of the first fetch; a fetch
//!   that succeeds replaces them; a fetch that fails leaves them in place
//!   and marks the store stale rather than empty or errored; and every
//!   change to the store's real rows is written back.
//!
//! **Scope** is whose rows these are: the hook takes a reactive closure
//! (an account and org selection, typically), so pointing the app at a
//! different account or org never shows the previous one's rows.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::id::Id;
use crate::result::AtomResult;
use crate::store::{Store, StoreEntity};

/// A local-storage failure, as a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalError(pub String);

impl std::fmt::Display for LocalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LocalError {}

/// One stored row: its key within `(kind, scope)`, the row as JSON, and
/// what owns it (`None` for a row cached from the server; a document id
/// for a row projected from a document).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct LocalRow {
    pub key: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub json: String,
}

/// `Send + Sync` on native, nothing on wasm (browser storage handles are
/// single-threaded and `!Send`).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSendSync for T {}
/// `Send + Sync` on native, nothing on wasm.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSendSync for T {}

/// Where a device keeps rows and queued calls. Object-safe, async so the
/// browser's storage can sit behind the same calls as a database.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait LocalRows: MaybeSendSync + 'static {
    /// The rows of `(kind, scope)`, or `None` when nothing was ever stored
    /// there (as opposed to a stored empty list).
    async fn load(&self, kind: &str, scope: &str) -> Result<Option<Vec<LocalRow>>, LocalError>;
    /// Replace every row of `(kind, scope)`.
    async fn replace(&self, kind: &str, scope: &str, rows: Vec<LocalRow>)
    -> Result<(), LocalError>;
    /// Insert or replace one row.
    async fn upsert(&self, kind: &str, scope: &str, row: LocalRow) -> Result<(), LocalError>;
    /// Remove one row.
    async fn remove(&self, kind: &str, scope: &str, key: &str) -> Result<(), LocalError>;
    /// Replace everything `owner` projected, in any kind and scope, with
    /// `rows` (each `(kind, scope, row)`; the row's owner is set to
    /// `owner`). The one write a document change makes into the index —
    /// a block index is another kind fed through the same call.
    async fn project(
        &self,
        owner: &str,
        rows: Vec<(String, String, LocalRow)>,
    ) -> Result<(), LocalError>;

    /// Append a queued call body; returns its sequence number (monotonic,
    /// never reused even once the queue drains).
    async fn queue_push(&self, body: String) -> Result<u64, LocalError>;
    /// Every queued body, oldest first.
    async fn queue_list(&self) -> Result<Vec<(u64, String)>, LocalError>;
    /// Replace a queued body in place.
    async fn queue_replace(&self, seq: u64, body: String) -> Result<(), LocalError>;
    /// Remove a queued body.
    async fn queue_remove(&self, seq: u64) -> Result<(), LocalError>;
}

// ── a key/value implementation, shared by memory and localStorage ───

/// String storage under a key — the one thing [`KvRows`] needs.
pub trait Kv: MaybeSendSync + 'static {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), LocalError>;
    fn remove(&self, key: &str);
}

/// [`LocalRows`] over any [`Kv`].
///
/// One JSON body per `(kind, scope)`, one
/// per owner (what it projected where), one for the queue. Not
/// transactional — each call is one or a few writes, and a torn write
/// reads back as the previous value or a miss.
pub struct KvRows<S: Kv> {
    kv: S,
    prefix: String,
}

#[derive(Serialize, serde::Deserialize, Default)]
struct QueueBody {
    next: u64,
    entries: Vec<(u64, String)>,
}

impl<S: Kv> KvRows<S> {
    /// Rows under keys starting `prefix` in `kv`.
    pub fn new(kv: S, prefix: impl Into<String>) -> Self {
        Self {
            kv,
            prefix: prefix.into(),
        }
    }

    fn set_key(&self, kind: &str, scope: &str) -> String {
        format!("{}rows.{kind}.{scope}", self.prefix)
    }

    fn owner_key(&self, owner: &str) -> String {
        format!("{}owned.{owner}", self.prefix)
    }

    fn queue_key(&self) -> String {
        format!("{}queue", self.prefix)
    }

    fn read<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.kv.get(key).and_then(|b| serde_json::from_str(&b).ok())
    }

    fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<(), LocalError> {
        let body = serde_json::to_string(value).map_err(|e| LocalError(e.to_string()))?;
        self.kv.set(key, &body)
    }

    fn upsert_now(&self, kind: &str, scope: &str, row: LocalRow) -> Result<(), LocalError> {
        let key = self.set_key(kind, scope);
        let mut rows: Vec<LocalRow> = self.read(&key).unwrap_or_default();
        match rows.iter_mut().find(|r| r.key == row.key) {
            Some(slot) => *slot = row,
            None => rows.push(row),
        }
        self.write(&key, &rows)
    }

    fn queue(&self) -> QueueBody {
        self.read(&self.queue_key()).unwrap_or_default()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<S: Kv> LocalRows for KvRows<S> {
    async fn load(&self, kind: &str, scope: &str) -> Result<Option<Vec<LocalRow>>, LocalError> {
        Ok(self.read(&self.set_key(kind, scope)))
    }

    async fn replace(
        &self,
        kind: &str,
        scope: &str,
        rows: Vec<LocalRow>,
    ) -> Result<(), LocalError> {
        self.write(&self.set_key(kind, scope), &rows)
    }

    async fn upsert(&self, kind: &str, scope: &str, row: LocalRow) -> Result<(), LocalError> {
        self.upsert_now(kind, scope, row)
    }

    async fn remove(&self, kind: &str, scope: &str, key: &str) -> Result<(), LocalError> {
        let set = self.set_key(kind, scope);
        let Some(mut rows) = self.read::<Vec<LocalRow>>(&set) else {
            return Ok(());
        };
        rows.retain(|r| r.key != key);
        self.write(&set, &rows)
    }

    async fn project(
        &self,
        owner: &str,
        rows: Vec<(String, String, LocalRow)>,
    ) -> Result<(), LocalError> {
        let owned_key = self.owner_key(owner);
        let before: Vec<(String, String)> = self.read(&owned_key).unwrap_or_default();
        for (kind, scope) in &before {
            let set = self.set_key(kind, scope);
            if let Some(mut existing) = self.read::<Vec<LocalRow>>(&set) {
                existing.retain(|r| r.owner.as_deref() != Some(owner));
                self.write(&set, &existing)?;
            }
        }
        let mut now: Vec<(String, String)> = Vec::new();
        for (kind, scope, mut row) in rows {
            row.owner = Some(owner.to_owned());
            self.upsert_now(&kind, &scope, row)?;
            if !now.iter().any(|(k, s)| *k == kind && *s == scope) {
                now.push((kind, scope));
            }
        }
        self.write(&owned_key, &now)
    }

    async fn queue_push(&self, body: String) -> Result<u64, LocalError> {
        let mut q = self.queue();
        let seq = q.next.max(1);
        q.next = seq.saturating_add(1);
        q.entries.push((seq, body));
        self.write(&self.queue_key(), &q)?;
        Ok(seq)
    }

    async fn queue_list(&self) -> Result<Vec<(u64, String)>, LocalError> {
        Ok(self.queue().entries)
    }

    async fn queue_replace(&self, seq: u64, body: String) -> Result<(), LocalError> {
        let mut q = self.queue();
        if let Some(e) = q.entries.iter_mut().find(|(s, _)| *s == seq) {
            e.1 = body;
        }
        self.write(&self.queue_key(), &q)
    }

    async fn queue_remove(&self, seq: u64) -> Result<(), LocalError> {
        let mut q = self.queue();
        q.entries.retain(|(s, _)| *s != seq);
        self.write(&self.queue_key(), &q)
    }
}

/// A [`Kv`] in memory — lives as long as the process.
#[derive(Default)]
pub struct MemoryKv {
    map: Mutex<BTreeMap<String, String>>,
}

impl Kv for MemoryKv {
    fn get(&self, key: &str) -> Option<String> {
        self.map.lock().ok()?.get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), LocalError> {
        self.map
            .lock()
            .map_err(|_| LocalError("memory rows: lock poisoned".into()))?
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }
    fn remove(&self, key: &str) {
        if let Ok(mut m) = self.map.lock() {
            m.remove(key);
        }
    }
}

/// Rows in memory: tests, and the stand-in where nothing durable exists.
pub type MemoryRows = KvRows<MemoryKv>;

impl MemoryRows {
    /// An empty in-memory store.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(MemoryKv::default(), "")
    }
}

/// `window.localStorage` as a [`Kv`], refusing bodies over `max_bytes` so
/// one large list cannot evict every small preference the origin keeps.
/// A first step for the browser; `IndexedDB` is the target.
#[cfg(target_arch = "wasm32")]
pub struct LocalStorageKv {
    pub max_bytes: usize,
}

#[cfg(target_arch = "wasm32")]
impl Default for LocalStorageKv {
    fn default() -> Self {
        Self {
            max_bytes: 1_500_000,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl LocalStorageKv {
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window().and_then(|w| w.local_storage().ok().flatten())
    }
}

#[cfg(target_arch = "wasm32")]
impl Kv for LocalStorageKv {
    fn get(&self, key: &str) -> Option<String> {
        Self::storage()?.get_item(key).ok().flatten()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), LocalError> {
        if value.len() > self.max_bytes {
            return Err(LocalError(format!(
                "{key}: {} bytes is over the {} byte localStorage guard",
                value.len(),
                self.max_bytes
            )));
        }
        Self::storage()
            .ok_or_else(|| LocalError("no localStorage".into()))?
            .set_item(key, value)
            .map_err(|e| LocalError(format!("localStorage: {e:?}")))
    }
    fn remove(&self, key: &str) {
        if let Some(s) = Self::storage() {
            let _ = s.remove_item(key);
        }
    }
}

/// Rows in the browser's `localStorage`, under `architect.` keys.
#[cfg(target_arch = "wasm32")]
pub type LocalStorageRows = KvRows<LocalStorageKv>;

// ── the app-root context ────────────────────────────────────────────

/// The device's rows, as the hooks find them.
#[derive(Clone)]
pub struct LocalRowsCtx(pub Arc<dyn LocalRows>);

/// Provide the device's rows at the app root. Call before the stores'
/// list hooks mount; without it every persisted hook is online-only.
pub fn provide_local_rows(rows: Arc<dyn LocalRows>) -> LocalRowsCtx {
    use_context_provider(|| LocalRowsCtx(rows))
}

/// The device's rows, if the app provided them.
#[must_use]
pub fn try_use_local_rows() -> Option<Arc<dyn LocalRows>> {
    try_consume_context::<LocalRowsCtx>().map(|c| c.0)
}

// ── the persisted list hook ─────────────────────────────────────────

fn encode_rows<T: StoreEntity + Serialize>(entries: &[(Id<T::Key>, T)]) -> Vec<LocalRow>
where
    T::Key: Serialize,
{
    entries
        .iter()
        // A temp row is an insert the server has not confirmed; its
        // durable form is the queued call (or document), not this list.
        .filter(|(id, _)| id.is_real())
        .filter_map(|(_, row)| {
            Some(LocalRow {
                key: serde_json::to_string(&row.key()).ok()?,
                owner: None,
                json: serde_json::to_string(row).ok()?,
            })
        })
        .collect()
}

fn decode_rows<T: DeserializeOwned>(rows: &[LocalRow]) -> Vec<T> {
    rows.iter()
        // A row an older build wrote in another shape is skipped, not an
        // error: the fetch will bring it back.
        .filter_map(|r| serde_json::from_str(&r.json).ok())
        .collect()
}

/// [`use_store_list`](crate::use_store_list) backed by the device's rows.
///
/// `kind` names the list (`"tasks"`); `scope` is a reactive closure for
/// whose rows they are (`None` while that is not known yet — nothing is
/// read or written). `fetch` is exactly `use_store_list`'s.
///
/// - The device copy for `(kind, scope)` is loaded on mount and whenever
///   the scope changes, and shown at once ([`Store::is_stale`]) unless a
///   fetch for that scope has already landed.
/// - A fetch that succeeds replaces the rows (and so the copy).
/// - A fetch that fails while rows are showing keeps them, marked stale,
///   phase `Success`; with nothing to show it fails as before.
/// - Every change to the store's real rows (fetch, live event, confirmed
///   mutation) is written back.
///
/// Without [`provide_local_rows`] this is `use_store_list`.
pub fn use_persisted_store_list<T, E, S, F, Fut>(
    store: Store<T, E>,
    kind: &'static str,
    scope: S,
    fetch: F,
) -> AtomResult<Vec<(Id<T::Key>, T)>, E>
where
    T: StoreEntity + Serialize + DeserializeOwned,
    T::Key: Serialize,
    E: Clone + PartialEq + 'static,
    S: Fn() -> Option<String> + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Option<Result<Vec<T>, E>>> + 'static,
{
    let local = use_hook(try_use_local_rows);
    let scope = use_memo(scope);
    // The scope whose rows the store holds — from the device copy or a
    // fetch. Write-back waits for it, so an empty store at mount never
    // overwrites a good copy.
    let mut held = use_signal(|| None::<String>);
    // The scope of the last fetch that succeeded: a slow local read must
    // not replace fresher rows.
    let mut fetched_for = use_signal(|| None::<String>);

    // 1. The device copy, independent of the fetch.
    let cached = use_resource({
        let local = local.clone();
        move || {
            let local = local.clone();
            let at = scope();
            async move {
                let (Some(local), Some(at)) = (local, at) else {
                    return None;
                };
                let rows = local.load(kind, &at).await.ok().flatten()?;
                Some((at, decode_rows::<T>(&rows)))
            }
        }
    });
    use_effect(move || {
        if let Some(Some((at, rows))) = &*cached.read() {
            if fetched_for.peek().as_deref() != Some(at.as_str()) {
                store.hydrate_stale(rows.clone());
                held.set(Some(at.clone()));
            }
        }
    });

    // 2. The fetch.
    let mut fetched = use_resource(fetch);
    let reload = use_callback(move |()| fetched.restart());
    use_effect(move || store.set_reloader(reload));
    use_effect(move || match &*fetched.read() {
        Some(Some(Ok(items))) => {
            store.hydrate(items.clone());
            let at = scope.peek().clone();
            fetched_for.set(at.clone());
            held.set(at);
        }
        Some(Some(Err(e))) => {
            if store.peek_is_empty() {
                store.set_failure(e.clone());
            } else {
                store.mark_stale();
            }
        }
        _ => {
            if !store.peek_is_stale() {
                store.set_loading();
            }
        }
    });

    // 3. Write back every change to the real rows.
    use_effect(move || {
        let entries = store.entries();
        let Some(at) = held.read().clone() else {
            return;
        };
        if scope.peek().as_deref() != Some(at.as_str()) {
            return;
        }
        let Some(local) = local.clone() else {
            return;
        };
        let rows = encode_rows(&entries);
        spawn_forever(async move {
            // Best-effort: a failed write leaves the previous copy, which
            // is still a correct (older) answer.
            let _ = local.replace(kind, &at, rows).await;
        });
    });

    store.entries_result()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn row(key: &str, json: &str) -> LocalRow {
        LocalRow {
            key: key.into(),
            owner: None,
            json: json.into(),
        }
    }

    pub async fn contract(s: &dyn LocalRows) {
        assert_eq!(
            s.load("tasks", "a/acme").await.unwrap(),
            None,
            "never stored"
        );
        s.replace("tasks", "a/acme", vec![]).await.unwrap();
        assert_eq!(
            s.load("tasks", "a/acme").await.unwrap(),
            Some(vec![]),
            "stored empty"
        );
        s.replace("tasks", "a/acme", vec![row("1", "{}"), row("2", "{}")])
            .await
            .unwrap();
        s.upsert("tasks", "a/acme", row("2", r#"{"v":2}"#))
            .await
            .unwrap();
        s.remove("tasks", "a/acme", "1").await.unwrap();
        assert_eq!(
            s.load("tasks", "a/acme").await.unwrap(),
            Some(vec![row("2", r#"{"v":2}"#)])
        );
        assert_eq!(
            s.load("tasks", "a/vnt").await.unwrap(),
            None,
            "scopes are apart"
        );

        // Projection: what an owner derived is replaced whole.
        s.project(
            "doc-1",
            vec![("tasks".into(), "a/acme".into(), row("t1", "{}"))],
        )
        .await
        .unwrap();
        s.project(
            "doc-1",
            vec![("tasks".into(), "a/vnt".into(), row("t1", "{}"))],
        )
        .await
        .unwrap();
        let acme = s.load("tasks", "a/acme").await.unwrap().unwrap();
        assert!(
            acme.iter().all(|r| r.key != "t1"),
            "the old projection is gone"
        );
        let vnt = s.load("tasks", "a/vnt").await.unwrap().unwrap();
        assert_eq!(vnt.len(), 1);
        assert_eq!(vnt.first().unwrap().owner.as_deref(), Some("doc-1"));

        // Queue: monotonic, never reused.
        let a = s.queue_push("a".into()).await.unwrap();
        let b = s.queue_push("b".into()).await.unwrap();
        assert!(b > a);
        s.queue_replace(a, "a2".into()).await.unwrap();
        assert_eq!(s.queue_list().await.unwrap().first().unwrap().1, "a2");
        s.queue_remove(a).await.unwrap();
        s.queue_remove(b).await.unwrap();
        let c = s.queue_push("c".into()).await.unwrap();
        assert!(c > b, "a drained queue never reuses a number");
        assert_eq!(s.queue_list().await.unwrap(), vec![(c, "c".to_owned())]);
    }

    #[test]
    fn memory_rows_keep_the_contract() {
        futures_executor_block_on(contract(&MemoryRows::in_memory()));
    }

    #[derive(Clone, PartialEq, Debug, Serialize, serde::Deserialize)]
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

    #[test]
    fn temp_rows_are_not_written_back_and_old_shapes_are_skipped() {
        let entries = vec![
            (
                Id::Real(1),
                Row {
                    id: 1,
                    name: "a".into(),
                },
            ),
            (
                Id::Temp(7),
                Row {
                    id: 0,
                    name: "draft".into(),
                },
            ),
        ];
        let rows = encode_rows(&entries);
        assert_eq!(rows.len(), 1);
        let mut rows = rows;
        rows.push(row("9", r#"{"old":"shape"}"#));
        let back: Vec<Row> = decode_rows(&rows);
        assert_eq!(
            back,
            vec![Row {
                id: 1,
                name: "a".into()
            }]
        );
    }

    /// A tiny single-threaded executor: the contract's futures never
    /// actually suspend on the memory backend.
    pub fn futures_executor_block_on<F: Future>(f: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, Waker};
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
}
