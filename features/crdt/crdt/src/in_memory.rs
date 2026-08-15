//! `InMemoryPersistence` — the test/demo backend. Backs every
//! method onto a per-doc `(snapshot, Vec<update>)` pair inside a
//! `Mutex<HashMap>`. Survives across clones (via `Arc`), does not
//! survive across processes.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use uuid::Uuid;

use crate::persistence::{PersistError, Persistence};

#[derive(Default)]
struct DocBytes {
    snapshot: Option<Vec<u8>>,
    updates: Vec<Vec<u8>>,
}

#[derive(Default, Clone)]
pub struct InMemoryPersistence {
    inner: std::sync::Arc<Mutex<HashMap<Uuid, DocBytes>>>,
}

impl InMemoryPersistence {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Persistence for InMemoryPersistence {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&doc_id)
            .and_then(|d| d.snapshot.clone()))
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        self.inner
            .lock()
            .unwrap()
            .entry(doc_id)
            .or_default()
            .snapshot = Some(bytes.to_vec());
        Ok(())
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        self.inner
            .lock()
            .unwrap()
            .entry(doc_id)
            .or_default()
            .updates
            .push(bytes.to_vec());
        Ok(())
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&doc_id)
            .map(|d| d.updates.clone())
            .unwrap_or_default())
    }

    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        let mut guard = self.inner.lock().unwrap();
        let entry = guard.entry(doc_id).or_default();
        entry.snapshot = Some(snapshot.to_vec());
        entry.updates.clear();
        Ok(())
    }
}
