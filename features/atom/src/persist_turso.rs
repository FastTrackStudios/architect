//! `TursoRows` — [`LocalRows`] in a local Turso database (native only).
//!
//! Generic tables, created on open if missing:
//!
//! ```text
//! atom_rows  (kind, scope, key, owner, json)   PRIMARY KEY (kind, scope, key)
//! atom_sets  (kind, scope)                     "this list was stored" (empty ≠ never)
//! atom_queue (seq, body)                       the durable call queue
//! atom_meta  (name, value)                     the queue's next sequence number
//! ```
//!
//! `owner` is what a row was projected from (a document id), indexed, so
//! [`LocalRows::project`] replaces one owner's rows in one transaction. A
//! block index is more rows of another `kind` through the same call, or
//! a table of its own beside these in the same database — the caller
//! opens the [`turso::Database`] and can hand it to crdt's
//! `TursoPersistence` too, so documents and their index share one file.
//!
//! Turso's local engine only; its sync is never used.

use tokio::sync::Mutex;
use turso::Value;

use crate::persist::{LocalError, LocalRow, LocalRows};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS atom_rows (kind TEXT NOT NULL, scope TEXT NOT NULL, key TEXT NOT NULL, owner TEXT, json TEXT NOT NULL, PRIMARY KEY (kind, scope, key))",
    "CREATE INDEX IF NOT EXISTS atom_rows_owner ON atom_rows (owner)",
    "CREATE TABLE IF NOT EXISTS atom_sets (kind TEXT NOT NULL, scope TEXT NOT NULL, PRIMARY KEY (kind, scope))",
    "CREATE TABLE IF NOT EXISTS atom_queue (seq INTEGER PRIMARY KEY, body TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS atom_meta (name TEXT PRIMARY KEY, value INTEGER NOT NULL)",
];

/// Rows and the call queue in a Turso database. One connection, locked
/// per call, so each multi-statement write is a transaction nothing else
/// interleaves with.
pub struct TursoRows {
    conn: Mutex<turso::Connection>,
}

fn err(e: &turso::Error) -> LocalError {
    LocalError(format!("turso: {e}"))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(t) => Some(t.clone()),
        _ => None,
    }
}

const fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        _ => None,
    }
}

fn to_i64(n: u64) -> Result<i64, LocalError> {
    i64::try_from(n).map_err(|e| LocalError(e.to_string()))
}

async fn query(
    c: &turso::Connection,
    sql: &str,
    params: Vec<Value>,
    cols: usize,
) -> Result<Vec<Vec<Value>>, LocalError> {
    let mut rows = c.query(sql, params).await.map_err(|e| err(&e))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| err(&e))? {
        let mut vals = Vec::with_capacity(cols);
        for i in 0..cols {
            vals.push(row.get_value(i).map_err(|e| err(&e))?);
        }
        out.push(vals);
    }
    Ok(out)
}

async fn exec(c: &turso::Connection, sql: &str, params: Vec<Value>) -> Result<(), LocalError> {
    c.execute(sql, params).await.map_err(|e| err(&e))?;
    Ok(())
}

/// Run `steps` between BEGIN and COMMIT; ROLLBACK on the first error.
async fn transaction(
    c: &turso::Connection,
    steps: Vec<(&'static str, Vec<Value>)>,
) -> Result<(), LocalError> {
    exec(c, "BEGIN", vec![]).await?;
    for (sql, params) in steps {
        if let Err(e) = exec(c, sql, params).await {
            let _ = exec(c, "ROLLBACK", vec![]).await;
            return Err(e);
        }
    }
    exec(c, "COMMIT", vec![]).await
}

const INSERT_ROW: &str =
    "INSERT OR REPLACE INTO atom_rows (kind, scope, key, owner, json) VALUES (?1, ?2, ?3, ?4, ?5)";
const MARK_SET: &str = "INSERT OR REPLACE INTO atom_sets (kind, scope) VALUES (?1, ?2)";

fn row_params(kind: &str, scope: &str, row: LocalRow) -> Vec<Value> {
    vec![
        Value::Text(kind.to_owned()),
        Value::Text(scope.to_owned()),
        Value::Text(row.key),
        row.owner.map_or(Value::Null, Value::Text),
        Value::Text(row.json),
    ]
}

fn set_params(kind: &str, scope: &str) -> Vec<Value> {
    vec![Value::Text(kind.to_owned()), Value::Text(scope.to_owned())]
}

impl TursoRows {
    /// Keep rows in `db`, creating the tables if they are missing.
    pub async fn open(db: &turso::Database) -> Result<Self, LocalError> {
        let conn = db.connect().map_err(|e| err(&e))?;
        for stmt in SCHEMA {
            exec(&conn, stmt, vec![]).await?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open (or create) the database file at `path` and keep rows in it.
    pub async fn open_path(path: &str) -> Result<Self, LocalError> {
        let db = turso::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| err(&e))?;
        Self::open(&db).await
    }
}

#[async_trait::async_trait]
impl LocalRows for TursoRows {
    async fn load(&self, kind: &str, scope: &str) -> Result<Option<Vec<LocalRow>>, LocalError> {
        let c = self.conn.lock().await;
        let known = query(
            &c,
            "SELECT 1 FROM atom_sets WHERE kind = ?1 AND scope = ?2",
            set_params(kind, scope),
            1,
        )
        .await?;
        if known.is_empty() {
            return Ok(None);
        }
        let rows = query(
            &c,
            "SELECT key, owner, json FROM atom_rows WHERE kind = ?1 AND scope = ?2 ORDER BY rowid",
            set_params(kind, scope),
            3,
        )
        .await?;
        drop(c);
        Ok(Some(
            rows.iter()
                .filter_map(|r| {
                    Some(LocalRow {
                        key: r.first().and_then(text)?,
                        owner: r.get(1).and_then(text),
                        json: r.get(2).and_then(text)?,
                    })
                })
                .collect(),
        ))
    }

    async fn replace(
        &self,
        kind: &str,
        scope: &str,
        rows: Vec<LocalRow>,
    ) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        let mut steps = vec![
            (
                "DELETE FROM atom_rows WHERE kind = ?1 AND scope = ?2",
                set_params(kind, scope),
            ),
            (MARK_SET, set_params(kind, scope)),
        ];
        steps.extend(
            rows.into_iter()
                .map(|r| (INSERT_ROW, row_params(kind, scope, r))),
        );
        transaction(&c, steps).await
    }

    async fn upsert(&self, kind: &str, scope: &str, row: LocalRow) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        transaction(
            &c,
            vec![
                (MARK_SET, set_params(kind, scope)),
                (INSERT_ROW, row_params(kind, scope, row)),
            ],
        )
        .await
    }

    async fn remove(&self, kind: &str, scope: &str, key: &str) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        let mut params = set_params(kind, scope);
        params.push(Value::Text(key.to_owned()));
        exec(
            &c,
            "DELETE FROM atom_rows WHERE kind = ?1 AND scope = ?2 AND key = ?3",
            params,
        )
        .await
    }

    async fn project(
        &self,
        owner: &str,
        rows: Vec<(String, String, LocalRow)>,
    ) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        let mut steps = vec![(
            "DELETE FROM atom_rows WHERE owner = ?1",
            vec![Value::Text(owner.to_owned())],
        )];
        for (kind, scope, mut row) in rows {
            row.owner = Some(owner.to_owned());
            steps.push((MARK_SET, set_params(&kind, &scope)));
            steps.push((INSERT_ROW, row_params(&kind, &scope, row)));
        }
        transaction(&c, steps).await
    }

    async fn queue_push(&self, body: String) -> Result<u64, LocalError> {
        let c = self.conn.lock().await;
        // The sequence lives in `atom_meta`, so a drained queue never
        // hands out a number it already used.
        let next = query(
            &c,
            "SELECT value FROM atom_meta WHERE name = 'queue_next'",
            vec![],
            1,
        )
        .await?
        .first()
        .and_then(|r| r.first())
        .and_then(int)
        .unwrap_or(1);
        transaction(
            &c,
            vec![
                (
                    "INSERT OR REPLACE INTO atom_meta (name, value) VALUES ('queue_next', ?1)",
                    vec![Value::Integer(next.saturating_add(1))],
                ),
                (
                    "INSERT INTO atom_queue (seq, body) VALUES (?1, ?2)",
                    vec![Value::Integer(next), Value::Text(body)],
                ),
            ],
        )
        .await?;
        drop(c);
        u64::try_from(next).map_err(|e| LocalError(e.to_string()))
    }

    async fn queue_list(&self) -> Result<Vec<(u64, String)>, LocalError> {
        let c = self.conn.lock().await;
        let rows = query(
            &c,
            "SELECT seq, body FROM atom_queue ORDER BY seq",
            vec![],
            2,
        )
        .await?;
        drop(c);
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    u64::try_from(r.first().and_then(int)?).ok()?,
                    r.get(1).and_then(text)?,
                ))
            })
            .collect())
    }

    async fn queue_replace(&self, seq: u64, body: String) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        exec(
            &c,
            "UPDATE atom_queue SET body = ?2 WHERE seq = ?1",
            vec![Value::Integer(to_i64(seq)?), Value::Text(body)],
        )
        .await
    }

    async fn queue_remove(&self, seq: u64) -> Result<(), LocalError> {
        let c = self.conn.lock().await;
        exec(
            &c,
            "DELETE FROM atom_queue WHERE seq = ?1",
            vec![Value::Integer(to_i64(seq)?)],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn turso_rows_keep_the_contract() {
        let rows = TursoRows::open_path(":memory:").await.unwrap();
        crate::persist::tests::contract(&rows).await;
    }

    #[tokio::test]
    async fn rows_and_the_queue_survive_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.db");
        let path = path.to_str().unwrap();
        {
            let rows = TursoRows::open_path(path).await.unwrap();
            rows.replace(
                "tasks",
                "alice/acme",
                vec![LocalRow {
                    key: "1".into(),
                    owner: None,
                    json: "{}".into(),
                }],
            )
            .await
            .unwrap();
            rows.queue_push("timer.start".into()).await.unwrap();
        }
        let rows = TursoRows::open_path(path).await.unwrap();
        assert_eq!(
            rows.load("tasks", "alice/acme")
                .await
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(rows.queue_list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn one_database_holds_crdt_docs_and_rows() {
        // The device opens ONE file and hands it to both stores.
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let rows = TursoRows::open(&db).await.unwrap();
        rows.queue_push("x".into()).await.unwrap();
        let c = db.connect().unwrap();
        drop(db);
        c.execute("CREATE TABLE IF NOT EXISTS other (k TEXT)", ())
            .await
            .unwrap();
        drop(c);
        assert_eq!(rows.queue_list().await.unwrap().len(), 1);
    }
}
