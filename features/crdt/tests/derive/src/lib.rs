//! A minimal entity crate wired the way a feature `-proto` crate is:
//! `#[architect::entity(repo, crdt)]` plus the `crdt = ["dep:crdt"]`
//! feature, so the emitted `EntityCrdt` impl and `<E>RepoLoro` compile
//! here and the tests in `tests/` can drive them across replicas.

use uuid::Uuid;

/// A note whose `body` is collaborative text (`crdt(text)` → `LoroText`)
/// and whose `title` stays a last-writer-wins string.
#[architect::entity(table_name = "notes", repo, crdt)]
#[derive(Eq)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(sortable)]
    pub title: String,
    #[architect(crdt(text))]
    pub body: String,
}
