//! [`TursoDocsIndex`]: a device's [`DocsIndex`] in a local Turso database.
//!
//! ```text
//! crdt_docs_index  (doc TEXT PRIMARY KEY, kind, name, parent, created_by,
//!                   origin, created_us, updated_us, rev, deleted, pending)
//! crdt_docs_cursor (id INTEGER PRIMARY KEY CHECK (id = 0), rev INTEGER)
//! ```
//!
//! Beside [`TursoPersistence`](crate::TursoPersistence)'s tables when the
//! app keeps both in one device database.

use async_trait::async_trait;
use tokio::sync::Mutex;
use turso::Value;
use uuid::Uuid;

use super::{DocMeta, DocsError, DocsIndex, LocalDoc};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS crdt_docs_index (doc TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, parent TEXT, created_by TEXT NOT NULL, origin TEXT, created_us INTEGER NOT NULL, updated_us INTEGER NOT NULL, rev INTEGER NOT NULL, deleted INTEGER NOT NULL, pending INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS crdt_docs_cursor (id INTEGER PRIMARY KEY CHECK (id = 0), rev INTEGER NOT NULL)",
];

const COLUMNS: &str =
    "doc, kind, name, parent, created_by, origin, created_us, updated_us, rev, deleted, pending";

/// A device's catalog of docs in a Turso database.
pub struct TursoDocsIndex {
    conn: Mutex<turso::Connection>,
}

fn backend(e: &turso::Error) -> DocsError {
    DocsError::Internal(format!("turso: {e}"))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

const fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(n) => *n,
        _ => 0,
    }
}

fn uuid(v: &Value) -> Option<Uuid> {
    text(v).and_then(|s| s.parse().ok())
}

fn opt(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::Text)
}

fn row_of(values: &[Value]) -> Option<LocalDoc> {
    let [
        doc,
        kind,
        name,
        parent,
        by,
        origin,
        created,
        updated,
        rev,
        deleted,
        pending,
    ] = values
    else {
        return None;
    };
    Some(LocalDoc {
        meta: DocMeta {
            doc_id: uuid(doc)?,
            kind: text(kind)?,
            name: text(name).unwrap_or_default(),
            parent: uuid(parent),
            created_by: text(by).unwrap_or_default(),
            origin: text(origin),
            created_us: int(created),
            updated_us: int(updated),
            rev: u64::try_from(int(rev)).unwrap_or(0),
            deleted: int(deleted) != 0,
        },
        pending: int(pending) != 0,
    })
}

impl TursoDocsIndex {
    /// Keep the index in `db`, creating its tables if they are missing.
    pub async fn open(db: &turso::Database) -> Result<Self, DocsError> {
        let conn = db.connect().map_err(|e| backend(&e))?;
        for stmt in SCHEMA {
            conn.execute(*stmt, ()).await.map_err(|e| backend(&e))?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open (or create) the database file at `path` and keep the index in it.
    pub async fn open_path(path: &str) -> Result<Self, DocsError> {
        let db = turso::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| backend(&e))?;
        Self::open(&db).await
    }

    async fn rows(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, DocsError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(sql, params).await.map_err(|e| backend(&e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| backend(&e))? {
            let mut values = Vec::with_capacity(row.column_count());
            for i in 0..row.column_count() {
                values.push(row.get_value(i).map_err(|e| backend(&e))?);
            }
            out.push(values);
        }
        drop(conn);
        Ok(out)
    }
}

#[async_trait]
impl DocsIndex for TursoDocsIndex {
    async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError> {
        let rows = self
            .rows(&format!("SELECT {COLUMNS} FROM crdt_docs_index"), vec![])
            .await?;
        Ok(rows.iter().filter_map(|r| row_of(r)).collect())
    }

    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError> {
        let rows = self
            .rows(
                &format!("SELECT {COLUMNS} FROM crdt_docs_index WHERE doc = ?1"),
                vec![Value::Text(doc_id.to_string())],
            )
            .await?;
        Ok(rows.first().and_then(|r| row_of(r)))
    }

    async fn upsert(&self, doc: &LocalDoc) -> Result<(), DocsError> {
        let m = &doc.meta;
        let params = vec![
            Value::Text(m.doc_id.to_string()),
            Value::Text(m.kind.clone()),
            Value::Text(m.name.clone()),
            opt(m.parent.map(|p| p.to_string())),
            Value::Text(m.created_by.clone()),
            opt(m.origin.clone()),
            Value::Integer(m.created_us),
            Value::Integer(m.updated_us),
            Value::Integer(i64::try_from(m.rev).unwrap_or(i64::MAX)),
            Value::Integer(i64::from(m.deleted)),
            Value::Integer(i64::from(doc.pending)),
        ];
        let conn = self.conn.lock().await;
        let done = conn
            .execute(
                &format!(
                    "INSERT OR REPLACE INTO crdt_docs_index ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                ),
                params,
            )
            .await;
        drop(conn);
        done.map(|_| ()).map_err(|e| backend(&e))
    }

    async fn cursor(&self) -> Result<u64, DocsError> {
        let rows = self
            .rows("SELECT rev FROM crdt_docs_cursor WHERE id = 0", vec![])
            .await?;
        Ok(rows
            .first()
            .and_then(|r| r.first())
            .map_or(0, |v| u64::try_from(int(v)).unwrap_or(0)))
    }

    async fn set_cursor(&self, rev: u64) -> Result<(), DocsError> {
        let conn = self.conn.lock().await;
        let done = conn
            .execute(
                "INSERT OR REPLACE INTO crdt_docs_cursor (id, rev) VALUES (0, ?1)",
                (i64::try_from(rev).unwrap_or(i64::MAX),),
            )
            .await;
        drop(conn);
        done.map(|_| ()).map_err(|e| backend(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rows_and_cursor_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.db");
        let path = path.to_str().unwrap();
        let doc = LocalDoc {
            meta: DocMeta {
                doc_id: Uuid::new_v4(),
                kind: "session".into(),
                name: "Gig".into(),
                parent: Some(Uuid::new_v4()),
                created_by: String::new(),
                origin: None,
                created_us: 7,
                updated_us: 8,
                rev: 0,
                deleted: false,
            },
            pending: true,
        };
        {
            let index = TursoDocsIndex::open_path(path).await.unwrap();
            index.upsert(&doc).await.unwrap();
            index.set_cursor(41).await.unwrap();
        }
        let index = TursoDocsIndex::open_path(path).await.unwrap();
        assert_eq!(index.get(doc.meta.doc_id).await.unwrap(), Some(doc.clone()));
        assert_eq!(index.docs().await.unwrap(), vec![doc]);
        assert_eq!(index.cursor().await.unwrap(), 41);
    }
}
