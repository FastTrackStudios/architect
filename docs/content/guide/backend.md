+++
title = "The backend"
description = "Implement a service and its repo, add storage and migrations, compose layers, read the caller, and record each call as one wide event."
weight = 30
+++

## What you'll build

The code that answers the contract: a backend struct that implements
the service and repo traits, owns the stream hubs, and names its bundle
of services once. Greeter's is `Memory` in `examples/greeter/src/lib.rs`.
Then real storage behind a cargo feature, the construction of that
storage with cleanup, the identity of whoever is calling, and the one
wide event each call leaves behind.

Order matters: write the in-memory backend first, get the scenario test
passing on every wire (the testing chapter), and add real storage last.
Everything above the backend stays the same when you swap it.

## 1. The backend struct

A backend is a `Clone` struct whose clones share state. Greeter's holds
its data behind `Arc<Mutex<…>>` and its stream hub as a `PubSub`, which
is cheap to clone:

```rust
// examples/greeter/src/lib.rs
#[derive(Clone)]
pub struct Memory {
    known: Arc<Vec<String>>,
    greetings: Arc<Mutex<u32>>,
    greeted: PubSub<String>,
    notes: Arc<Mutex<Vec<Note>>>,
}
```

## 2. Implement the service

The trait's methods are plain `async fn`s. Return the typed error;
never panic (the workspace lints deny `unwrap` and friends in shipped
code). Lock with `architect::lock`, which recovers a poisoned mutex
instead of panicking (`crates/architect/src/lock.rs`):

```rust
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
```

Drop a guard before the next `await`. A `std::sync::Mutex` guard held
across an await makes the future `!Send`, and the server will not
accept it.

## 3. Feed the streams

For each argless `#[subscribe]`, the backend implements
`<Trait>StreamSource` by handing over its hub. The generated host
attaches every subscriber (vox or SSE) to it:

```rust
impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &PubSub<String> {
        &self.greeted
    }
}
```

The hub's constructor picks what happens when a subscriber falls
behind: `PubSub::sliding(cap)` drops the oldest queued event (greeter
uses `sliding(64)`), `PubSub::dropping(cap)` drops the newest,
`PubSub::unbounded()` never drops. `publish` never waits, so a slow
client never slows the backend. `.with_replay(n)` hands a new
subscriber the last `n` events.

Two more hooks on the source:

- `<name>_intro(&self) -> Option<Ev>`: a first frame for each new
  subscriber (the current revision, say), so the client knows it is
  attached before it reads the current state.
- A subscription with arguments uses `<name>_attach(&self, filter…,
  sink)` instead of a hub. Snapshot-then-changes is
  `begin_attach(sink)`, read the state, then
  `complete_attach(pending, Some(snapshot))`.

`PubSub::publish` takes a mutex: never call it from an audio callback.
`architect::rt` (feature `rt`) is the wait-free bridge for that.

## 4. Implement the repo

The entity's `NoteRepo` is five methods. Greeter's in-memory version is
in the same file; the shape is the one the scaffold writes:

```rust
impl NoteRepo for Memory {
    async fn get(&self, id: Uuid) -> Result<Note, RepoError> {
        architect::lock(&self.notes)
            .iter()
            .find(|n| n.id == id)
            .cloned()
            .ok_or(RepoError::NotFound)
    }
    async fn create(&self, input: NoteCreate) -> Result<Note, RepoError> {
        let note = Note { id: Uuid::new_v4(), title: input.title, body: input.body };
        architect::lock(&self.notes).push(note.clone());
        Ok(note)
    }
    // list, update, delete …
}
```

## 5. Name the bundle

`architect::Services` is the one impl per backend the framework cannot
infer: the list of service tokens this backend serves.

```rust
impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}
```

With it, `backend.into_router()` is the vox `LayerRouter` and
`backend.into_http_router()` the axum router, both mounting the same
list. The `Layer` value is also composable by hand:

- `Memory::layers().merge(greeter_layer(Mock))` puts a mock in front of
  one service; a later merge wins for the methods it covers, on vox and
  HTTP alike (`examples/greeter/tests/round_trip.rs`
  `a_mock_replaces_the_real_greeter`).
- `.provide(backend)` binds a layer to a backend and returns the
  `LayerRouter`; `.provide_http(&backend)` returns the axum router.
- `architect::router![…]` merges several backends' routers.
- `LayerRouter::unmounted_from::<B>()` lists services a backend
  declares but the router does not mount.

A backend that has to route one trait across two stores (writes to a
primary, reads from a replica, shards) is one struct holding both and
deciding inside each method. vox addresses a service by name, so two
backends of the same trait cannot both sit on one router
([idioms §6](@/architecture/idioms.md)).

## 6. Real storage: SeaORM

Turn on your crate's `server` feature (forwarding `architect/server`)
and the entity derive emits `NoteRepoStorage<C>`, a `NoteRepo` over any
SeaORM connection, and `NoteMigration`, which creates the table and an
index per `filterable` / `sortable` column. The whole path, from
`crates/architect/tests/entity_storage.rs`
`migrate_then_crud_over_sqlite`:

```rust
architect::migrator!(Migrator: [GadgetMigration]);

let db = architect::storage::connect("sqlite::memory:").await?;
architect::storage::migrate::<Migrator>(&db).await?;
let repo = GadgetRepoStorage::new(db);
let made = repo.create(GadgetCreate { name: "flux".into(), weight: 3, note: None }).await?;
```

- `architect::storage::connect` sizes the pool: an in-memory SQLite URL
  gets a pool of one, because each SQLite memory connection is its own
  empty database. `sqlite://./app.db?mode=rwc` is a file; Postgres URLs
  work as SeaORM accepts them (`apps/auth-server/tests/postgres.rs`).
- `architect::migrator!` collects migrations in order; list hand-written
  ones beside the derived ones.
- The derived migration only creates. A later column change is a
  hand-written migration in the list.
- Every `DbErr` becomes `RepoError::Internal`, so a unique violation is
  a 500 to the client. Check for the conflict yourself where it
  matters.

`apps/auth-server/src/server.rs` `build` is the production version:
connect, migrate when configured, wrap the storage in the engine,
assemble the host.

### Local stores: Turso

Data that lives on the device rather than a server uses Turso's local
engine, through two adapters:

- `crdt::turso` (`features/crdt/crdt/src/turso.rs`
  `TursoPersistence`): a CRDT document's snapshots and update log in a
  local database (feature `turso` of `crdt`).
- `architect::TursoRows` (architect feature `atom-turso`,
  `features/atom/src/persist_turso.rs` `TursoRows`): the client's
  persisted rows and offline call queue, covered in the client chapter.

Both take a database the app opens, so one device file can hold both.
Sync between devices is Loro over the document sync, never Turso
replication. Documents get their own chapter in part 2.

## 7. Building the backend: `Resource` and `Scope`

A real backend is built from config, then a pool, then the storage, and
the pool must close when the server stops. `architect::Resource` is the
recipe; `architect::Scope` runs the finalizers in reverse order on
`close()`:

```rust
let scope = Scope::new();
let db = Resource::acquire_release(
    Resource::from_fn(|_| async {
        let db = architect::storage::connect(&database_url()).await?;
        architect::storage::migrate::<Migrator>(&db).await?;
        Ok(db)
    }),
    |db: DatabaseConnection| async move {
        let _ = db.close().await;
    },
);
let router = db.map(NoteRepoStorage::new).into_router(&scope).await?;
// … serve the router …
scope.close().await;                 // the pool closes here
```

- `Resource::from_fn(f)` builds from an async closure (it receives the
  scope); `acquire_release(acquire, release)` registers `release` as a
  finalizer; `.and_then(f)` is a dependent step (`config → pool`),
  `.zip(other)` an independent pair, `.map` / `.map_err` transform.
- `Resource::into_router(&scope)` builds a `Services` backend and
  mounts it in one step.
- A pool that several repos share is a `SharedResource`:
  `SharedResource::from_fn(f)` builds once (a failed build is retried,
  not cached), and `.resource()` turns it back into a `Resource` for
  each repo's `.map(…)`.

`crates/architect/src/resource.rs` holds the combinators and their
tests (`scope_closes_finalizers_lifo`);
`crates/architect/src/plan.rs` `LayerGraph` plans a multi-node graph
and names a missing provider or a cycle.

Configuration comes from the environment through
`#[derive(architect::Config)]`:

```rust
// examples/greeter/src/main.rs
#[derive(architect::Config)]
#[architect(prefix = "GREETER")]
struct Config {
    #[architect(default = "127.0.0.1:4040")]
    bind: String,
}
let config = Config::from_env()?;     // reads GREETER_BIND
```

Field options: `env = "NAME"`, `default = expr`, `secret` (reads
`NAME_FILE` first, for mounted secrets), `nested`, `skip`.

## 8. Who is calling

A method that answers differently per person reads the caller, it does
not take it as an argument. Behind a `PermissionsGate`, the principal
the gate resolved for the call is available for the whole of the
method:

```rust
let Some(principal) = architect::permissions_gate::caller() else {
    return Err(NoteError::Unauthenticated);
};
```

`caller()` is `None` when nothing resolved one (no gate in front, or
code running off the request task). Treat that as no identity and
refuse. The gate itself (`crates/architect/src/permissions_gate.rs`
`PermissionsGate`: a permit table per method, `wrap(router)`,
`coverage(&router)` for unmapped methods) is mounted by the server and
covered in [auth](@/architecture/auth.md). For a per-org server, the
org comes from the lane too, never from an argument
([organizations](@/architecture/organizations.md)).

For a value the client sets once and every call carries (a project, a
document), declare `context = T` on the trait instead
(`crates/architect-rpc-derive/src/lib.rs` `rpc`): each method receives
it, and the client binds it with `.scoped(ctx)`. Those traits have no
HTTP face yet.

## 9. Telemetry: one wide event per call

Each dispatched call is a `tracing` span named `rpc`, with
`otel.name = "<Service>/<method>"`, `rpc.service`, `rpc.method` and
`rpc.scope` (`crates/architect/src/layer.rs` `LayerRouter::call_span`,
architect feature `telemetry`, which `server-axum` and so `host` turn
on). Its duration is also recorded as the metric
`rpc.server.duration`. A caller's trace context rides along as
`traceparent` metadata, so a caller and its calls are one trace
(`crates/architect/tests/trace_context.rs`).

That span is the event. Don't log lines from a handler; put the facts
of the call on the span with `architect_telemetry::wide::set`
(`features/telemetry/src/lib.rs` `wide`):

```rust
// apps/auth-server/src/oauth.rs, linked_token
use architect_telemetry::wide;

let outcome = |name: &'static str| wide::set("auth.linked_token.outcome", name);
// …
wide::set("auth.linked_token.provider", provider.id());
wide::set_display("auth.linked_token.user_id", user_id);
```

- Keys are dotted, namespaced and `&'static str`: `note.count`,
  `org.slug`. Counts are `i64`; `set_display` takes anything `Display`.
- Never put a token, password or secret in a field. Fields leave the
  machine.
- `wide::set` does nothing without the `otel` feature of
  `architect-telemetry`. A library may call it freely; the binary that
  enables `otel` makes it live.
- Time the stages of a call with
  `architect_telemetry::timing::Stages`: `Stages::start()`, then
  `stages.mark("note.load_ms")` after each stage. The subscriber needs
  `timing::DIRECTIVE` in its filter and a `timing::Recorder` layer;
  `timing::recent(n)` reads the last ones back in a test
  (`features/telemetry/src/timing.rs`).

The subscriber is installed once in the binary (the server chapter).
In code shared with wasm, keep `wide` and `timing` calls under
`cfg(not(target_arch = "wasm32"))`.

## Where to read more

- [Multi-backend features](@/architecture/backends.md): backends as
  flags, large payloads behind handles.
- [Construction](@/architecture/construction.md): `Resource`, `Scope`,
  memoization, `into_router`.
- [Idioms §6](@/architecture/idioms.md): the layer system and routing
  backends.
- [Server push](@/architecture/streams.md): hubs, attach, intro frames,
  real-time safety.
- [Auth](@/architecture/auth.md) and
  [organizations](@/architecture/organizations.md): the gate, the
  caller, org confinement.
- [Documents](@/architecture/documents.md): CRDT documents and their
  persistence backends.
- [Snapshot quiesce](@/architecture/snapshot.md): a consistent copy of
  a live SQLite store.
