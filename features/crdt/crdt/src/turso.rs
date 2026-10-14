//! `TursoPersistence` — [`Persistence`] over a local Turso database.
//!
//! Two tables, created on [`TursoPersistence::open`] if missing:
//!
//! ```text
//! crdt_snapshots (doc TEXT PRIMARY KEY, bytes BLOB)          last compacted snapshot
//! crdt_updates   (doc TEXT, seq INTEGER, bytes BLOB,          the update log since,
//!                 PRIMARY KEY (doc, seq))                    in commit order
//! ```
//!
//! The caller opens the [`turso::Database`] and hands it over, so one
//! device database can hold this doc store beside the app's other local
//! tables (architect-atom's `turso` rows, a host's own). Turso's local
//! engine only — sync between devices is Loro over `DocSync`, never
//! Turso's replication.
//!
//! [`compact`](Persistence::compact) is one transaction: the snapshot
//! replaces the old one and the updates it folds in are pruned together,
//! so a crash between the two cannot replay updates over a snapshot that
//! already contains them (harmless for Loro, but wasteful) or — worse —
//! leave a pruned log with no snapshot.

use async_trait::async_trait;
use tokio::sync::Mutex;
use turso::Value;
use uuid::Uuid;

use crate::persistence::{PersistError, Persistence};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS crdt_snapshots (doc TEXT PRIMARY KEY, bytes BLOB NOT NULL)",
    "CREATE TABLE IF NOT EXISTS crdt_updates (doc TEXT NOT NULL, seq INTEGER NOT NULL, bytes BLOB NOT NULL, PRIMARY KEY (doc, seq))",
];

/// Doc snapshots + update logs in a Turso database. Every method holds
/// the one connection's lock for its statements, so a multi-statement
/// write is a transaction no other caller interleaves with.
pub struct TursoPersistence {
    conn: Mutex<turso::Connection>,
}

fn backend(e: &turso::Error) -> PersistError {
    PersistError::Backend(format!("turso: {e}"))
}

fn blob(v: Value) -> Option<Vec<u8>> {
    match v {
        Value::Blob(b) => Some(b),
        _ => None,
    }
}

impl TursoPersistence {
    /// Persist docs in `db`, creating the two tables if they are missing.
    pub async fn open(db: &turso::Database) -> Result<Self, PersistError> {
        let conn = db.connect().map_err(|e| backend(&e))?;
        for stmt in SCHEMA {
            conn.execute(*stmt, ()).await.map_err(|e| backend(&e))?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open (or create) the database file at `path` and persist docs in it.
    pub async fn open_path(path: &str) -> Result<Self, PersistError> {
        let db = turso::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| backend(&e))?;
        Self::open(&db).await
    }

    async fn first_column(
        conn: &turso::Connection,
        sql: &str,
        doc: String,
    ) -> Result<Vec<Value>, PersistError> {
        let mut rows = conn.query(sql, (doc,)).await.map_err(|e| backend(&e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| backend(&e))? {
            out.push(row.get_value(0).map_err(|e| backend(&e))?);
        }
        Ok(out)
    }

    async fn in_transaction(
        conn: &turso::Connection,
        statements: Vec<(&'static str, Vec<Value>)>,
    ) -> Result<(), PersistError> {
        conn.execute("BEGIN", ()).await.map_err(|e| backend(&e))?;
        for (sql, params) in statements {
            if let Err(e) = conn.execute(sql, params).await {
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(backend(&e));
            }
        }
        conn.execute("COMMIT", ()).await.map_err(|e| backend(&e))?;
        Ok(())
    }
}

#[async_trait]
impl Persistence for TursoPersistence {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        let conn = self.conn.lock().await;
        let values = Self::first_column(
            &conn,
            "SELECT bytes FROM crdt_snapshots WHERE doc = ?1",
            doc_id.to_string(),
        )
        .await?;
        drop(conn);
        Ok(values.into_iter().next().and_then(blob))
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let conn = self.conn.lock().await;
        let done = conn
            .execute(
                "INSERT OR REPLACE INTO crdt_snapshots (doc, bytes) VALUES (?1, ?2)",
                (doc_id.to_string(), bytes.to_vec()),
            )
            .await;
        drop(conn);
        done.map(|_| ()).map_err(|e| backend(&e))
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let conn = self.conn.lock().await;
        let doc = doc_id.to_string();
        let next = Self::first_column(
            &conn,
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM crdt_updates WHERE doc = ?1",
            doc.clone(),
        )
        .await?
        .into_iter()
        .next()
        .and_then(|v| match v {
            Value::Integer(n) => Some(n),
            _ => None,
        })
        .unwrap_or(1);
        let done = conn
            .execute(
                "INSERT INTO crdt_updates (doc, seq, bytes) VALUES (?1, ?2, ?3)",
                (doc, next, bytes.to_vec()),
            )
            .await;
        drop(conn);
        done.map(|_| ()).map_err(|e| backend(&e))
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        let conn = self.conn.lock().await;
        let values = Self::first_column(
            &conn,
            "SELECT bytes FROM crdt_updates WHERE doc = ?1 ORDER BY seq",
            doc_id.to_string(),
        )
        .await?;
        drop(conn);
        Ok(values.into_iter().filter_map(blob).collect())
    }

    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        let conn = self.conn.lock().await;
        let doc = doc_id.to_string();
        let done = Self::in_transaction(
            &conn,
            vec![
                (
                    "INSERT OR REPLACE INTO crdt_snapshots (doc, bytes) VALUES (?1, ?2)",
                    vec![Value::Text(doc.clone()), Value::Blob(snapshot.to_vec())],
                ),
                (
                    "DELETE FROM crdt_updates WHERE doc = ?1",
                    vec![Value::Text(doc)],
                ),
            ],
        )
        .await;
        drop(conn);
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrdtDoc;

    #[tokio::test]
    async fn log_then_compact_round_trips() {
        let p = TursoPersistence::open_path(":memory:").await.unwrap();
        let doc = Uuid::new_v4();
        assert_eq!(p.load_snapshot(doc).await.unwrap(), None);
        p.append_update(doc, b"u1").await.unwrap();
        p.append_update(doc, b"u2").await.unwrap();
        assert_eq!(
            p.load_updates(doc).await.unwrap(),
            vec![b"u1".to_vec(), b"u2".to_vec()]
        );
        p.compact(doc, b"snap").await.unwrap();
        assert_eq!(p.load_snapshot(doc).await.unwrap(), Some(b"snap".to_vec()));
        assert!(p.load_updates(doc).await.unwrap().is_empty());
        // Another doc is untouched by this one's compaction.
        let other = Uuid::new_v4();
        p.append_update(other, b"x").await.unwrap();
        p.compact(doc, b"snap2").await.unwrap();
        assert_eq!(p.load_updates(other).await.unwrap(), vec![b"x".to_vec()]);
    }

    #[tokio::test]
    async fn a_doc_survives_reopening_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.db");
        let path = path.to_str().unwrap();
        let id = Uuid::new_v4();
        {
            let p = TursoPersistence::open_path(path).await.unwrap();
            let doc = CrdtDoc::open(id, p).await.unwrap();
            doc.loro()
                .get_text("body")
                .insert(0, "offline words")
                .unwrap();
            doc.loro().commit();
            // Compaction writes the snapshot directly (the update log is
            // fed by a fire-and-forget writer task).
            doc.compact(id).await.unwrap();
        }
        let p = TursoPersistence::open_path(path).await.unwrap();
        let doc = CrdtDoc::open(id, p).await.unwrap();
        assert_eq!(doc.loro().get_text("body").to_string(), "offline words");
    }
}
