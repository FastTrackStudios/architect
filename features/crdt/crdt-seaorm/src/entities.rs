//! SeaORM entities for the two CRDT-persistence tables. Generic
//! across every feature in the workspace.

pub mod crdt_doc {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
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

pub mod crdt_update {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
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
