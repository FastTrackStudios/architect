//! Migrator + the two table definitions. Plug into your feature's
//! migrator (or just run this Migrator directly if the only thing
//! you persist is CRDT bytes).

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m_initial::Migration)]
    }
}

mod m_initial {
    use sea_orm_migration::prelude::*;

    #[derive(DeriveMigrationName)]
    pub struct Migration;

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(CrdtDoc::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(CrdtDoc::DocId)
                                .uuid()
                                .not_null()
                                .primary_key(),
                        )
                        .col(ColumnDef::new(CrdtDoc::Snapshot).blob().not_null())
                        .col(
                            ColumnDef::new(CrdtDoc::UpdatedAt)
                                .timestamp_with_time_zone()
                                .not_null(),
                        )
                        .to_owned(),
                )
                .await?;

            manager
                .create_table(
                    Table::create()
                        .table(CrdtUpdate::Table)
                        .if_not_exists()
                        .col(ColumnDef::new(CrdtUpdate::DocId).uuid().not_null())
                        .col(ColumnDef::new(CrdtUpdate::Seq).big_integer().not_null())
                        .col(ColumnDef::new(CrdtUpdate::Bytes).blob().not_null())
                        .col(
                            ColumnDef::new(CrdtUpdate::CreatedAt)
                                .timestamp_with_time_zone()
                                .not_null(),
                        )
                        .primary_key(Index::create().col(CrdtUpdate::DocId).col(CrdtUpdate::Seq))
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_crdt_update_doc_id_seq")
                        .table(CrdtUpdate::Table)
                        .col(CrdtUpdate::DocId)
                        .col(CrdtUpdate::Seq)
                        .to_owned(),
                )
                .await?;

            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(Table::drop().table(CrdtUpdate::Table).to_owned())
                .await?;
            manager
                .drop_table(Table::drop().table(CrdtDoc::Table).to_owned())
                .await?;
            Ok(())
        }
    }

    #[derive(Iden)]
    enum CrdtDoc {
        Table,
        DocId,
        Snapshot,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum CrdtUpdate {
        Table,
        DocId,
        Seq,
        Bytes,
        CreatedAt,
    }
}
