# architect

Declare your data model and your service once. Every wire, both clients,
the storage, the migration, the tests that run over all of them — the
derives write the rest.

```rust
use uuid::Uuid;

// An entity. From this one struct: `Note`, `NoteCreate`, `NoteUpdate`,
// `NoteList`, the `NoteRepo` trait (get / list / create / update / delete),
// its vox client + dispatcher, its HTTP routes + client, the `NoteEvent`
// feed, the SeaORM storage and the `NoteMigration`.
#[architect::entity(table_name = "notes", repo, events)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
}

// An error. The attribute writes the wire derives, the HTTP status and
// code per variant, and a `Transport(String)` variant — the one that lets
// the generated clients implement the trait.
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]
    Unknown(String),
}

// A service. From this one trait: the vox service, `POST /greeter/greet`,
// `POST /greeter/count`, `/greeter/greeted` as Server-Sent Events,
// `GreeterClient` (vox) and `GreeterHttpClient` — and both clients
// implement `Greeter`.
#[architect::service]
pub trait Greeter {
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    async fn count(&self) -> Result<u32, GreetError>;
    #[subscribe]
    fn greeted(&self) -> String;
}
pub use prelude::*; // `GreeterService`, `GreeterStreamService`, `greeter_layer`, …
```

A backend implements the traits and names its bundle:

```rust
#[derive(Clone)]
pub struct Memory { /* … */ }

impl Greeter for Memory { /* greet, count */ }
impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &architect::PubSub<String> { &self.greeted }
}
impl NoteRepo for Memory { /* get, list, create, update, delete */ }

impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}
```

A server mounts it — vox over WebSocket, the same router over iroh, and
HTTP+JSON, from one backend:

```rust
#[derive(architect::Config)]
#[architect(prefix = "GREETER")]
struct Config {
    #[architect(default = "127.0.0.1:4040")]
    bind: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    architect::host::boot("info");
    let config = Config::from_env()?;
    let backend = Memory::default();
    architect::host::EngineHost::new(backend.clone().into_router(), config.bind)
        .plugin(backend.into_http_router())
        .serve()
        .await?;
    Ok(())
}
```

```sh
curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'          # "hello cody"
curl -N     localhost:4040/greeter/greeted                              # data: "hello cody"
curl -XPOST localhost:4040/note/create -d '{"input":{"title":"a","body":"b"}}'
```

And because the clients implement the trait, one scenario covers every
wire, and a mock merged over the bundle replaces the real service on all
of them:

```rust
async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
}

scenario(&Memory::default()).await;                                   // in process
scenario(&local.establish::<GreeterClient>().await?).await;           // vox
scenario(&GreeterHttpClient::at("http://127.0.0.1:4040")).await;      // HTTP

let router = Memory::layers().merge(greeter_layer(Mock)).provide(Memory::default());
```

Storage is the same declaration under the `server` feature:

```rust
architect::migrator!(Migrator: [NoteMigration]);
let db = architect::storage::connect("sqlite::memory:").await?;
architect::storage::migrate::<Migrator>(&db).await?;
let repo = NoteRepoStorage::new(db);   // implements `NoteRepo`
```

That is the whole surface a feature writes. `examples/greeter` is this
example, running, with the tests; `cargo xtask feature new <name>`
scaffolds a new one; the docs under `docs/content` cover each piece.

## Also in the box

Optimistic client state for Dioxus (`store`), typed forms (`form`), CRDT
backends (`crdt`), a REAPER-style action registry, real-time-safe event
publishing, reconnecting connection pools, retry and supervision
policies, and a permissions gate — each behind a feature, each generated
from the same declarations.
