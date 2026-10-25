//! `SeaORM` entities for the two CRDT-persistence tables. Generic
//! across every feature in the workspace.

pub mod crdt_doc {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "crdt_doc")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub doc_id: Uuid,
        pub snapshot: Vec<u8>,
        pub updated_at: chrono::DateTime<chrono::Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The `crdt::docs` catalog: one row per document (`DocMeta`).
pub mod crdt_doc_meta {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "crdt_doc_meta")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub doc_id: Uuid,
        pub kind: String,
        pub name: String,
        pub parent: Option<Uuid>,
        pub created_by: String,
        pub origin: Option<String>,
        pub created_us: i64,
        pub updated_us: i64,
        #[sea_orm(unique)]
        pub rev: i64,
        pub deleted: bool,
        /// `strict` or `merge` (`crdt::docs::Lineage::as_str`).
        pub lineage: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod crdt_update {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "crdt_update")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub doc_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub seq: i64,
        pub bytes: Vec<u8>,
        pub created_at: chrono::DateTime<chrono::Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
