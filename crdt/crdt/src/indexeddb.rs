//! `IndexedDbPersistence` — browser-backed [`Persistence`], so a wasm
//! replica survives restarts entirely offline. Layout (one database,
//! two object stores):
//!
//! - `snapshots`: key = doc id string → `Uint8Array` (last compaction)
//! - `updates`: auto-increment key → `{ doc: <doc id string>, bytes:
//!   Uint8Array }`, with an index on `doc` — the per-doc update log in
//!   commit order (auto-increment keys preserve insertion order).
//!
//! IndexedDB handles are `!Send`; the [`Persistence`] trait drops its
//! `Send` bounds on wasm (single-threaded anyway), and `CrdtDoc::open`
//! drains its persistence writes on `spawn_local`.

use async_trait::async_trait;
use idb::event::DatabaseEvent;
use idb::{Database, Factory, IndexParams, KeyPath, ObjectStoreParams, Query, TransactionMode};
use uuid::Uuid;
use wasm_bindgen::JsValue;

use crate::persistence::{PersistError, Persistence};

const SNAPSHOTS: &str = "snapshots";
const UPDATES: &str = "updates";
const DOC_INDEX: &str = "doc";

pub struct IndexedDbPersistence {
    db: Database,
}

impl IndexedDbPersistence {
    /// Open (or create) the backing database. One database can hold any
    /// number of docs.
    pub async fn open(db_name: &str) -> Result<Self, PersistError> {
        let factory = Factory::new().map_err(idb_err)?;
        let mut request = factory.open(db_name, Some(1)).map_err(idb_err)?;
        request.on_upgrade_needed(|event| {
            let db = event.database().expect("upgrade event has a database");
            let _ = db.create_object_store(SNAPSHOTS, ObjectStoreParams::new());
            let mut params = ObjectStoreParams::new();
            params.auto_increment(true);
            if let Ok(updates) = db.create_object_store(UPDATES, params) {
                let index: Option<IndexParams> = None;
                let _ = updates.create_index(DOC_INDEX, KeyPath::new_single("doc"), index);
            }
        });
        let db = request.await.map_err(idb_err)?;
        Ok(Self { db })
    }

    fn doc_key(doc_id: Uuid) -> JsValue {
        JsValue::from_str(&doc_id.to_string())
    }
}

fn idb_err(e: impl std::fmt::Display) -> PersistError {
    PersistError::Backend(format!("indexeddb: {e}"))
}

fn js_err(e: JsValue) -> PersistError {
    PersistError::Backend(format!("indexeddb: {e:?}"))
}

fn bytes_to_js(bytes: &[u8]) -> JsValue {
    js_sys::Uint8Array::from(bytes).into()
}

fn js_to_bytes(value: &JsValue) -> Result<Vec<u8>, PersistError> {
    value
        .dyn_ref::<js_sys::Uint8Array>()
        .map(|a| a.to_vec())
        .ok_or_else(|| PersistError::Backend("indexeddb: stored value is not bytes".into()))
}

use wasm_bindgen::JsCast;

#[async_trait(?Send)]
impl Persistence for IndexedDbPersistence {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        let tx = self
            .db
            .transaction(&[SNAPSHOTS], TransactionMode::ReadOnly)
            .map_err(idb_err)?;
        let store = tx.object_store(SNAPSHOTS).map_err(idb_err)?;
        let value = store
            .get(Query::Key(Self::doc_key(doc_id)))
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;
        value.map(|v| js_to_bytes(&v)).transpose()
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let tx = self
            .db
            .transaction(&[SNAPSHOTS], TransactionMode::ReadWrite)
            .map_err(idb_err)?;
        let store = tx.object_store(SNAPSHOTS).map_err(idb_err)?;
        store
            .put(&bytes_to_js(bytes), Some(&Self::doc_key(doc_id)))
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;
        tx.commit().map_err(idb_err)?.await.map_err(idb_err)?;
        Ok(())
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let row = js_sys::Object::new();
        js_sys::Reflect::set(&row, &JsValue::from_str("doc"), &Self::doc_key(doc_id))
            .map_err(js_err)?;
        js_sys::Reflect::set(&row, &JsValue::from_str("bytes"), &bytes_to_js(bytes))
            .map_err(js_err)?;

        let tx = self
            .db
            .transaction(&[UPDATES], TransactionMode::ReadWrite)
            .map_err(idb_err)?;
        let store = tx.object_store(UPDATES).map_err(idb_err)?;
        store
            .add(&row, None)
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;
        tx.commit().map_err(idb_err)?.await.map_err(idb_err)?;
        Ok(())
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        let tx = self
            .db
            .transaction(&[UPDATES], TransactionMode::ReadOnly)
            .map_err(idb_err)?;
        let store = tx.object_store(UPDATES).map_err(idb_err)?;
        let index = store.index(DOC_INDEX).map_err(idb_err)?;
        let rows = index
            .get_all(Some(Query::Key(Self::doc_key(doc_id))), None)
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;
        rows.iter()
            .map(|row| {
                let bytes =
                    js_sys::Reflect::get(row, &JsValue::from_str("bytes")).map_err(js_err)?;
                js_to_bytes(&bytes)
            })
            .collect()
    }

    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        // One transaction over both stores: replace the snapshot and
        // prune the doc's update log atomically.
        let tx = self
            .db
            .transaction(&[SNAPSHOTS, UPDATES], TransactionMode::ReadWrite)
            .map_err(idb_err)?;
        let snaps = tx.object_store(SNAPSHOTS).map_err(idb_err)?;
        snaps
            .put(&bytes_to_js(snapshot), Some(&Self::doc_key(doc_id)))
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;

        let updates = tx.object_store(UPDATES).map_err(idb_err)?;
        let index = updates.index(DOC_INDEX).map_err(idb_err)?;
        let keys = index
            .get_all_keys(Some(Query::Key(Self::doc_key(doc_id))), None)
            .map_err(idb_err)?
            .await
            .map_err(idb_err)?;
        for key in keys {
            updates
                .delete(Query::Key(key))
                .map_err(idb_err)?
                .await
                .map_err(idb_err)?;
        }
        tx.commit().map_err(idb_err)?.await.map_err(idb_err)?;
        Ok(())
    }
}
