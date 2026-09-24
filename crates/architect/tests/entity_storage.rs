//! The storage story with nothing hand-written: an entity, the derived
//! `SeaORM` repo, the derived migration, one `migrator!` — CRUD over a
//! real `SQLite` database.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod gadget {
    use uuid::Uuid;

    #[architect::entity(table_name = "gadgets", repo)]
    #[derive(Eq)]
    pub struct Gadget {
        #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
        pub id: Uuid,
        #[architect(filterable, sortable)]
        pub name: String,
        pub weight: i32,
        pub note: Option<String>,
    }
}

use architect::{Page, RepoError};
use gadget::{
    Gadget, GadgetCreate, GadgetMigration, GadgetRepo as _, GadgetRepoStorage, GadgetUpdate,
};

architect::migrator!(Migrator: [GadgetMigration]);

#[tokio::test]
async fn migrate_then_crud_over_sqlite() {
    let db = architect::storage::connect("sqlite::memory:")
        .await
        .unwrap();
    architect::storage::migrate::<Migrator>(&db).await.unwrap();
    let repo = GadgetRepoStorage::new(db);

    let made = repo
        .create(GadgetCreate {
            name: "flux".into(),
            weight: 3,
            note: None,
        })
        .await
        .expect("create");
    assert_eq!(repo.get(made.id).await.expect("get").name, "flux");

    let renamed = repo
        .update(
            made.id,
            GadgetUpdate {
                name: Some("capacitor".into()),
                ..Default::default()
            },
        )
        .await
        .expect("update");
    assert_eq!(renamed.name, "capacitor");

    let all = repo
        .list(Page { index: 0, size: 10 }, None, None)
        .await
        .expect("list");
    assert_eq!(
        all.items,
        vec![Gadget {
            name: "capacitor".into(),
            ..made.clone()
        }]
    );

    repo.delete(made.id).await.expect("delete");
    assert_eq!(repo.get(made.id).await.unwrap_err(), RepoError::NotFound);
}
