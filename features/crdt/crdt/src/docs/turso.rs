//! [`TursoDocsIndex`]: a device's [`DocsIndex`] in a local Turso database.
//!
//! ```text
//! crdt_doc_index   (doc TEXT PRIMARY KEY, kind, name, parent, created_by,
//!                   origin, created_us, updated_us, rev, deleted, pending,
//!                   lineage, seq UNIQUE)
//! crdt_doc_cursors (lane TEXT PRIMARY KEY, rev INTEGER)
//! ```
//!
//! Beside [`TursoPersistence`](crate::TursoPersistence)'s tables when the
//! app keeps both in one device database. (0.9.28's `crdt_docs_index` /
//! `crdt_docs_cursor`, which carried no lineage, seq or lanes, are not
//! read.)

use async_trait::async_trait;
use tokio::sync::Mutex;
use turso::Value;
use uuid::Uuid;

use super::{DocMeta, DocsError, DocsIndex, Lineage, LocalDoc};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS crdt_doc_index (doc TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, parent TEXT, created_by TEXT NOT NULL, origin TEXT, created_us INTEGER NOT NULL, updated_us INTEGER NOT NULL, rev INTEGER NOT NULL, deleted INTEGER NOT NULL, pending INTEGER NOT NULL, lineage TEXT NOT NULL, seq INTEGER NOT NULL UNIQUE)",
    "CREATE TABLE IF NOT EXISTS crdt_doc_cursors (lane TEXT PRIMARY KEY, rev INTEGER NOT NULL)",
];

const COLUMNS: &str = "doc, kind, name, parent, created_by, origin, created_us, updated_us, rev, deleted, pending, lineage, seq";

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

fn uint(v: &Value) -> u64 {
    u64::try_from(int(v)).unwrap_or(0)
}

fn uuid(v: &Value) -> Option<Uuid> {
    text(v).and_then(|s| s.parse().ok())
}

fn opt(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::Text)
}

fn big(n: u64) -> Value {
    Value::Integer(i64::try_from(n).unwrap_or(i64::MAX))
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
        lineage,
        seq,
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
            rev: uint(rev),
            deleted: int(deleted) != 0,
            lineage: Lineage::parse(&text(lineage).unwrap_or_default()),
        },
        pending: int(pending) != 0,
        seq: uint(seq),
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

    async fn rows_on(
        conn: &turso::Connection,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<Vec<Vec<Value>>, DocsError> {
        let mut rows = conn.query(sql, params).await.map_err(|e| backend(&e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| backend(&e))? {
            let mut values = Vec::with_capacity(row.column_count());
            for i in 0..row.column_count() {
                values.push(row.get_value(i).map_err(|e| backend(&e))?);
            }
            out.push(values);
        }
        Ok(out)
    }

    async fn rows(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, DocsError> {
        let conn = self.conn.lock().await;
        let out = Self::rows_on(&conn, sql, params).await;
        drop(conn);
        out
    }
}

#[async_trait]
impl DocsIndex for TursoDocsIndex {
    async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError> {
        let rows = self
            .rows(&format!("SELECT {COLUMNS} FROM crdt_doc_index"), vec![])
            .await?;
        Ok(rows.iter().filter_map(|r| row_of(r)).collect())
    }

    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError> {
        let rows = self
            .rows(
                &format!("SELECT {COLUMNS} FROM crdt_doc_index WHERE doc = ?1"),
                vec![Value::Text(doc_id.to_string())],
            )
            .await?;
        Ok(rows.first().and_then(|r| row_of(r)))
    }

    // The connection's lock is the transaction: the next seq is read and
    // the row written under it.
    #[allow(clippy::significant_drop_tightening)]
    async fn upsert(&self, mut doc: LocalDoc) -> Result<LocalDoc, DocsError> {
        let conn = self.conn.lock().await;
        let head = Self::rows_on(
            &conn,
            "SELECT COALESCE(MAX(seq), 0) FROM crdt_doc_index",
            vec![],
        )
        .await?
        .first()
        .and_then(|r| r.first())
        .map_or(0, uint);
        doc.seq = head.saturating_add(1);
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
            big(m.rev),
            Value::Integer(i64::from(m.deleted)),
            Value::Integer(i64::from(doc.pending)),
            Value::Text(m.lineage.as_str().to_owned()),
            big(doc.seq),
        ];
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO crdt_doc_index ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
            ),
            params,
        )
        .await
        .map_err(|e| backend(&e))?;
        Ok(doc)
    }

    async fn cursor(&self, lane: &str) -> Result<u64, DocsError> {
        let rows = self
            .rows(
                "SELECT rev FROM crdt_doc_cursors WHERE lane = ?1",
                vec![Value::Text(lane.to_owned())],
            )
            .await?;
        Ok(rows.first().and_then(|r| r.first()).map_or(0, uint))
    }

    async fn set_cursor(&self, lane: &str, rev: u64) -> Result<(), DocsError> {
        let conn = self.conn.lock().await;
        let done = conn
            .execute(
                "INSERT OR REPLACE INTO crdt_doc_cursors (lane, rev) VALUES (?1, ?2)",
                vec![Value::Text(lane.to_owned()), big(rev)],
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
    async fn rows_seqs_and_cursors_survive_reopening() {
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
                lineage: Lineage::Merge,
            },
            pending: true,
            seq: 0,
        };
        let stored = {
            let index = TursoDocsIndex::open_path(path).await.unwrap();
            let first = index.upsert(doc.clone()).await.unwrap();
            assert_eq!(first.seq, 1);
            let second = index.upsert(first).await.unwrap();
            assert_eq!(second.seq, 2, "every write takes the next seq");
            index.set_cursor("hub", 41).await.unwrap();
            index.set_cursor("peer-a", 3).await.unwrap();
            second
        };
        let index = TursoDocsIndex::open_path(path).await.unwrap();
        assert_eq!(
            index.get(doc.meta.doc_id).await.unwrap(),
            Some(stored.clone())
        );
        assert_eq!(index.docs().await.unwrap(), vec![stored]);
        assert_eq!(index.cursor("hub").await.unwrap(), 41);
        assert_eq!(index.cursor("peer-a").await.unwrap(), 3);
        assert_eq!(index.cursor("never").await.unwrap(), 0);
    }
}
