//! The reference feature: everything architect generates, from the
//! smallest possible declarations.
//!
//! Read top to bottom. There are four things a developer writes — an
//! entity, a service, an error, a backend — and the rest of this crate
//! (`main.rs`, the tests) only *mounts* and *calls* what the framework
//! emitted from them.

use std::sync::{Arc, Mutex};

use architect::{Filter, Page, PubSub, RepoError, Sort};
use uuid::Uuid;

// ── 1. An entity ──────────────────────────────────────────────────────
//
// `#[architect::entity]` writes the derive line. From this struct you get
// `Note`, `NoteCreate`, `NoteUpdate`, `NoteList`, the `NoteRepo` trait,
// its vox client/dispatcher and HTTP router/client, the `NoteRepoLayer`
// bundle token, the `NoteEvent` feed with `NoteEvented`, the SeaORM
// `NoteRepoStorage` and the `NoteMigration` (under `server`).

#[architect::entity(table_name = "notes", repo, events)]
#[derive(Eq)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
}

// ── 2. A service and its error ────────────────────────────────────────
//
// `#[architect::error]` writes the wire derives, the `HttpError` impl
// (status per variant: named or `status = …`, code from the variant
// name) and a `Transport(String)` variant with `From<TransportError>` —
// which is what lets the generated clients implement `Greeter`.

#[architect::error]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(status = 404)]
    Unknown(String),
}

/// One `#[architect::service]` and you have the vox service, the axum
/// routes at `/greeter/…`, both clients — and both clients implement
/// this very trait.
#[architect::service]
pub trait Greeter {
    /// Greets a known name.
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    /// How many greetings so far.
    async fn count(&self) -> Result<u32, GreetError>;
    /// Every greeting, as it happens.
    #[subscribe]
    fn greeted(&self) -> String;
}

// ── 3. A backend ──────────────────────────────────────────────────────
//
// Implement the trait, own the hub, list the bundle. `Services` is the
// one impl per backend the framework cannot infer.

#[derive(Clone)]
pub struct Memory {
    known: Arc<Vec<String>>,
    greetings: Arc<Mutex<u32>>,
    greeted: PubSub<String>,
    notes: Arc<Mutex<Vec<Note>>>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            known: Arc::new(vec!["cody".into(), "ada".into()]),
            greetings: Arc::new(Mutex::new(0)),
            greeted: PubSub::sliding(64),
            notes: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Greeter for Memory {
    async fn greet(&self, name: String) -> Result<String, GreetError> {
        if !self.known.contains(&name) {
            return Err(GreetError::Unknown(name));
        }
        {
            let mut n = architect::lock(&self.greetings);
            *n = n.saturating_add(1);
        }
        let greeting = format!("hello {name}");
        self.greeted.publish(greeting.clone());
        Ok(greeting)
    }
    async fn count(&self) -> Result<u32, GreetError> {
        Ok(*architect::lock(&self.greetings))
    }
}

impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &PubSub<String> {
        &self.greeted
    }
}

impl NoteRepo for Memory {
    async fn get(&self, id: Uuid) -> Result<Note, RepoError> {
        architect::lock(&self.notes)
            .iter()
            .find(|n| n.id == id)
            .cloned()
            .ok_or(RepoError::NotFound)
    }
    async fn list(
        &self,
        page: Page,
        _sort: Option<Sort>,
        _filter: Option<Filter>,
    ) -> Result<NoteList, RepoError> {
        let items = architect::lock(&self.notes).clone();
        let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
        Ok(NoteList { items, total, page })
    }
    async fn create(&self, input: NoteCreate) -> Result<Note, RepoError> {
        let note = Note {
            id: Uuid::new_v4(),
            title: input.title,
            body: input.body,
        };
        architect::lock(&self.notes).push(note.clone());
        Ok(note)
    }
    async fn update(&self, id: Uuid, input: NoteUpdate) -> Result<Note, RepoError> {
        let mut notes = architect::lock(&self.notes);
        let note = notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(RepoError::NotFound)?;
        if let Some(title) = input.title {
            note.title = title;
        }
        if let Some(body) = input.body {
            note.body = body;
        }
        let updated = note.clone();
        drop(notes);
        Ok(updated)
    }
    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
        let mut notes = architect::lock(&self.notes);
        let before = notes.len();
        notes.retain(|n| n.id != id);
        let removed = notes.len() != before;
        drop(notes);
        if removed {
            Ok(())
        } else {
            Err(RepoError::NotFound)
        }
    }
}

impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}

// The trait-prefixed names (`GreeterService`, `greeter_layer`, …) so a
// crate root with several services never collides on `Service`.
pub use prelude::*;
