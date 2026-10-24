//! `SeaOrmDocsStore` — the `crdt::docs` catalog in the `crdt_doc_meta`
//! table, beside the persistence tables in the same database.

use async_trait::async_trait;
use crdt::docs::{DocMeta, DocPage, DocQuery, DocsError, DocsStore};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, TransactionTrait,
};
use uuid::Uuid;

use crate::entities::crdt_doc_meta::{ActiveModel, Column, Entity, Model};

/// A `DocsStore` over a `SeaORM` database. Run [`crate::Migrator`] first.
#[derive(Clone)]
pub struct SeaOrmDocsStore {
    db: DatabaseConnection,
}

impl SeaOrmDocsStore {
    #[must_use]
    pub const fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

// By value: an error-mapping adapter for `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn internal(e: sea_orm::DbErr) -> DocsError {
    DocsError::Internal(e.to_string())
}

fn rev_of(rev: i64) -> u64 {
    u64::try_from(rev).unwrap_or(0)
}

fn meta(m: Model) -> DocMeta {
    DocMeta {
        doc_id: m.doc_id,
        kind: m.kind,
        name: m.name,
        parent: m.parent,
        created_by: m.created_by,
        origin: m.origin,
        created_us: m.created_us,
        updated_us: m.updated_us,
        rev: rev_of(m.rev),
        deleted: m.deleted,
    }
}

fn active(row: &DocMeta, rev: i64) -> ActiveModel {
    ActiveModel {
        doc_id: Set(row.doc_id),
        kind: Set(row.kind.clone()),
        name: Set(row.name.clone()),
        parent: Set(row.parent),
        created_by: Set(row.created_by.clone()),
        origin: Set(row.origin.clone()),
        created_us: Set(row.created_us),
        updated_us: Set(row.updated_us),
        rev: Set(rev),
        deleted: Set(row.deleted),
    }
}

async fn head_of(db: &impl sea_orm::ConnectionTrait) -> Result<i64, sea_orm::DbErr> {
    Ok(Entity::find()
        .order_by_desc(Column::Rev)
        .one(db)
        .await?
        .map_or(0, |m| m.rev))
}

#[async_trait]
impl DocsStore for SeaOrmDocsStore {
    async fn get(&self, doc_id: Uuid) -> Result<Option<DocMeta>, DocsError> {
        Ok(Entity::find_by_id(doc_id)
            .one(&self.db)
            .await
            .map_err(internal)?
            .map(meta))
    }

    async fn list(&self, query: &DocQuery) -> Result<DocPage, DocsError> {
        // The head first, and rows only up to it: a row committed between
        // the two reads must not be skipped by a cursor already past it.
        let head = head_of(&self.db).await.map_err(internal)?;
        let since = i64::try_from(query.since_rev).unwrap_or(i64::MAX);
        let mut find = Entity::find()
            .filter(Column::Rev.gt(since))
            .filter(Column::Rev.lte(head))
            .order_by_asc(Column::Rev);
        if let Some(kind) = &query.kind {
            find = find.filter(Column::Kind.eq(kind.clone()));
        }
        if !query.include_deleted {
            find = find.filter(Column::Deleted.eq(false));
        }
        if query.limit > 0 {
            // One past the page says whether there is more.
            find = find.limit(u64::from(query.limit) + 1);
        }
        let mut docs: Vec<DocMeta> = find
            .all(&self.db)
            .await
            .map_err(internal)?
            .into_iter()
            .map(meta)
            .collect();
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        if query.limit > 0 && docs.len() > limit {
            docs.truncate(limit);
            let cursor = docs.last().map_or(query.since_rev, |d| d.rev);
            return Ok(DocPage {
                docs,
                cursor,
                more: true,
            });
        }
        Ok(DocPage {
            docs,
            cursor: rev_of(head).max(query.since_rev),
            more: false,
        })
    }

    async fn put(&self, row: DocMeta, expect: Option<u64>) -> Result<DocMeta, DocsError> {
        self.db
            .transaction::<_, Result<DocMeta, DocsError>, sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    let current = Entity::find_by_id(row.doc_id).one(txn).await?;
                    let current_rev = current.as_ref().map(|m| rev_of(m.rev));
                    if current_rev != expect {
                        return Ok(Err(DocsError::Stale(current_rev.unwrap_or(0))));
                    }
                    let rev = head_of(txn).await?.saturating_add(1);
                    let model = active(&row, rev);
                    if current.is_some() {
                        model.update(txn).await?;
                    } else {
                        model.insert(txn).await?;
                    }
                    Ok(Ok(DocMeta {
                        rev: rev_of(rev),
                        ..row
                    }))
                })
            })
            .await
            .map_err(|e| match e {
                // Another writer took the same next revision first: a
                // revision race, which the host retries.
                sea_orm::TransactionError::Transaction(ref db)
                    if matches!(
                        db.sql_err(),
                        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
                    ) =>
                {
                    DocsError::Stale(0)
                }
                other => DocsError::Internal(other.to_string()),
            })?
    }

    async fn head(&self) -> Result<u64, DocsError> {
        Ok(rev_of(head_of(&self.db).await.map_err(internal)?))
    }
}

#[cfg(test)]
mod tests {
    use sea_orm_migration::MigratorTrait as _;

    use super::*;

    fn row(kind: &str) -> DocMeta {
        DocMeta {
            doc_id: Uuid::new_v4(),
            kind: kind.into(),
            name: "n".into(),
            parent: Some(Uuid::new_v4()),
            created_by: "user-1".into(),
            origin: None,
            created_us: 1,
            updated_us: 2,
            rev: 0,
            deleted: false,
        }
    }

    #[tokio::test]
    async fn revisions_pages_and_staleness() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        crate::Migrator::up(&db, None).await.unwrap();
        let store = SeaOrmDocsStore::new(db);
        assert_eq!(store.head().await.unwrap(), 0);

        let a = store.put(row("session"), None).await.unwrap();
        let b = store.put(row("setlist"), None).await.unwrap();
        assert_eq!((a.rev, b.rev), (1, 2));
        assert_eq!(store.get(a.doc_id).await.unwrap(), Some(a.clone()));
        assert_eq!(
            store.put(a.clone(), None).await.unwrap_err(),
            DocsError::Stale(1)
        );
        let gone = store
            .put(
                DocMeta {
                    deleted: true,
                    ..a.clone()
                },
                Some(1),
            )
            .await
            .unwrap();
        assert_eq!(gone.rev, 3);

        let live = store.list(&DocQuery::default()).await.unwrap();
        assert_eq!(live.docs, vec![b.clone()]);
        assert_eq!((live.cursor, live.more), (3, false));
        let first = store
            .list(&DocQuery {
                include_deleted: true,
                limit: 1,
                ..DocQuery::default()
            })
            .await
            .unwrap();
        assert_eq!((first.docs, first.cursor, first.more), (vec![b], 2, true));
        let rest = store
            .list(&DocQuery {
                include_deleted: true,
                since_rev: 2,
                kind: Some("session".into()),
                ..DocQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(rest.docs, vec![gone]);
    }
}
