//! `SeaOrmPersistence` — server-side `Persistence` impl backed by
//! two generic tables (`crdt_doc`, `crdt_update`). The same impl
//! handles every feature in the workspace; each feature just picks a
//! doc_id namespace and goes.

use async_trait::async_trait;
use crdt::{PersistError, Persistence};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, TransactionTrait,
};
use uuid::Uuid;

pub mod entities;
pub mod migrations;

pub use migrations::Migrator;

#[derive(Clone)]
pub struct SeaOrmPersistence {
    db: DatabaseConnection,
}

impl SeaOrmPersistence {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }
}

fn map_err(e: sea_orm::DbErr) -> PersistError {
    PersistError::Backend(e.to_string())
}

#[async_trait]
impl Persistence for SeaOrmPersistence {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        Ok(entities::crdt_doc::Entity::find_by_id(doc_id)
            .one(&self.db)
            .await
            .map_err(map_err)?
            .map(|m| m.snapshot))
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let now = chrono::Utc::now();
        let bytes = bytes.to_vec();
        // Upsert: try update, fall back to insert.
        let existing = entities::crdt_doc::Entity::find_by_id(doc_id)
            .one(&self.db)
            .await
            .map_err(map_err)?;
        match existing {
            Some(m) => {
                let mut a: entities::crdt_doc::ActiveModel = m.into();
                a.snapshot = Set(bytes);
                a.updated_at = Set(now);
                a.update(&self.db).await.map_err(map_err)?;
            }
            None => {
                entities::crdt_doc::ActiveModel {
                    doc_id: Set(doc_id),
                    snapshot: Set(bytes),
                    updated_at: Set(now),
                }
                .insert(&self.db)
                .await
                .map_err(map_err)?;
            }
        }
        Ok(())
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        entities::crdt_update::ActiveModel {
            doc_id: Set(doc_id),
            seq: Set(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            bytes: Set(bytes.to_vec()),
            created_at: Set(chrono::Utc::now()),
        }
        .insert(&self.db)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        let rows = entities::crdt_update::Entity::find()
            .filter(entities::crdt_update::Column::DocId.eq(doc_id))
            .order_by_asc(entities::crdt_update::Column::Seq)
            .all(&self.db)
            .await
            .map_err(map_err)?;
        Ok(rows.into_iter().map(|r| r.bytes).collect())
    }

    /// Atomic compact: write the fresh snapshot and drop the update
    /// log in one transaction, so a crash mid-compact can't lose data
    /// (snapshot succeeded but log persisted) or lose history
    /// (log dropped but snapshot failed).
    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        let now = chrono::Utc::now();
        let bytes = snapshot.to_vec();
        self.db
            .transaction::<_, (), sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    let existing = entities::crdt_doc::Entity::find_by_id(doc_id)
                        .one(txn)
                        .await?;
                    match existing {
                        Some(m) => {
                            let mut a: entities::crdt_doc::ActiveModel = m.into();
                            a.snapshot = Set(bytes);
                            a.updated_at = Set(now);
                            a.update(txn).await?;
                        }
                        None => {
                            entities::crdt_doc::ActiveModel {
                                doc_id: Set(doc_id),
                                snapshot: Set(bytes),
                                updated_at: Set(now),
                            }
                            .insert(txn)
                            .await?;
                        }
                    }
                    entities::crdt_update::Entity::delete_many()
                        .filter(entities::crdt_update::Column::DocId.eq(doc_id))
                        .exec(txn)
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(|e| PersistError::Backend(e.to_string()))?;
        Ok(())
    }
}
