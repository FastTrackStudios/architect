+++
title = "Build a feature, end to end"
description = "Define an entity and a service, write a backend, serve it, consume it — the whole architect flow."
weight = 15
+++

This walks the entire architect flow against the reference feature,
`examples/greeter` (`examples/greeter/src/lib.rs`,
`examples/greeter/src/main.rs`, `examples/greeter/tests/round_trip.rs`).
Each step points at the real file, so you can read the working code
alongside.

## The mental model

```
#[architect::entity]    ──▶  wire struct + Create/Update/List + <Entity>Repo trait
                             (+ SeaORM storage and migration under `server`)

#[architect::service]   ──▶  vox service + HTTP routes + both clients,
#[architect::error]          which implement the trait themselves

a backend               ──▶  impl the traits + impl Services (its Layer bundle)

EngineHost / LocalServer ──▶ serve that one router over WebSocket, iroh,
                             HTTP+JSON, SSE, or in-process
```

You write the struct, the trait, the error and the backend; architect
emits the wire types, the repo trait, the clients and the routes. You
choose, at the edge, how a client reaches the backend.

## 1. Define the entity

One plain struct (`examples/greeter/src/lib.rs`, section 1):

```rust
#[architect::entity(table_name = "notes", repo, events)]
#[derive(Eq)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
}
```

`#[architect::entity]` writes the derive line. From this struct you get
`Note`, `NoteCreate`, `NoteUpdate`, `NoteList`, the `NoteRepo` trait, its
vox client/dispatcher and HTTP router/client, the `NoteRepoLayer` bundle
token, the `NoteEvent` feed, and under `server` the SeaORM
`NoteRepoStorage` and `NoteMigration`. See
[the architect pattern](@/architecture/pattern.md) for the full
field/attribute table.

## 2. A service and its error

Operations that are not CRUD live on a `#[architect::service]` trait, with
one `#[architect::error]` enum for it (section 2 of the same file):

```rust
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]
    Unknown(String),
}

#[architect::service]
pub trait Greeter {
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    async fn count(&self) -> Result<u32, GreetError>;
    #[subscribe]
    fn greeted(&self) -> String;
}
```

The error gets the wire derives, an HTTP status per variant and a
`Transport(String)` variant with `From<TransportError>`, which is what
lets the generated `GreeterClient` (vox) and `GreeterHttpClient` (HTTP)
implement `Greeter` themselves. `#[subscribe]` declares a stream
([streams](@/architecture/streams.md)). Errors in depth:
[errors](@/architecture/errors.md).

## 3. Write a backend

A backend implements the traits and lists its bundle once (section 3):

```rust
impl Greeter for Memory { /* greet, count */ }
impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &PubSub<String> { &self.greeted }
}
impl NoteRepo for Memory { /* get, list, create, update, delete */ }

impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}
```

`Services` is the one impl per backend the framework cannot infer. Swap
the backend (in-memory, SeaORM, a CRDT doc) and no consumer changes; see
[backends](@/architecture/backends.md). Need primary+replica or sharding?
Write one backend that holds both and routes internally
([idioms §7](@/architecture/idioms.md)).

## 4. Serve it: one backend, every wire

`examples/greeter/src/main.rs` is the whole server:

```rust
#[derive(architect::Config)]
#[architect(prefix = "GREETER")]
struct Config {
    #[architect(default = "127.0.0.1:4040")]
    bind: String,
}

let backend = Memory::default();
EngineHost::new(backend.clone().into_router(), config.bind)
    .plugin(backend.into_http_router())
    .serve()
    .await?;
```

`EngineHost` serves vox over WebSocket at `/vox`, the HTTP+JSON routes and
SSE streams from the plugin, `/health`, and with `.iroh(key, id)` the same
router peer to peer ([HTTP and iroh](@/architecture/http.md)). Try it:

```sh
GREETER_BIND=127.0.0.1:4040 cargo run -p example-greeter
curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'
curl -N   localhost:4040/greeter/greeted            # Server-Sent Events
```

In-process, with no socket, `LocalServer::serve(backend.into_router(), scope)`
serves the same router over a vox memory link (native only; see
[the in-process transport](@/architecture/local.md)).

## 5. Consume it: program against the trait

The generated clients implement the service trait, so a screen, a CLI or
a test takes `impl Greeter` and cannot tell which wire it is on.
`examples/greeter/tests/round_trip.rs` runs one scenario on each:

```rust
async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
    assert_eq!(client.count().await.unwrap(), 1);
}

#[tokio::test]
async fn in_process() { scenario(&Memory::default()).await; }

#[tokio::test]
async fn over_vox() {
    let scope = Scope::new();
    let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
    let client: GreeterClient = local.establish().await.unwrap();
    scenario(&client).await;
    scope.close().await;
}

#[tokio::test]
async fn over_http() {
    let backend = Memory::default();
    let base = serve_http(backend.clone().into_http_router()).await;
    scenario(&GreeterHttpClient::at(base)).await;
}
```

Clients in a real app share one connection per (url, identity) through
`architect::connect::pool()`, and wrap the connect in
`architect::schedule::retry` ([scheduling](@/architecture/scheduling.md)).

## 6. Compose and build backends

For real wiring (`config → pool → repo`), use `architect::Resource`: a lazy
builder with dependent composition, memoization and scoped teardown
([construction](@/architecture/construction.md)):

```rust
let scope = Scope::new();
let repo = Resource::from_fn(|_| async { load_config() })
    .and_then(|cfg| Resource::acquire_release(connect_pool(cfg), |p| async move { p.close().await }))
    .memoize()                       // built once, shared
    .map(make_repo)                  // e.g. the derive's SeaORM storage over the pool
    .build(&scope).await?;
// … serve repo.into_router() …
scope.close().await;                 // finalizers run LIFO (pool closes)
```

`LayerGraph` plans a multi-node backend graph (topological order plus
cycle / missing-provider / conflict diagnostics); see
`crates/architect/src/plan.rs`.

## 7. Test it

```sh
cargo test -p example-greeter
```

runs the scenario in process, over vox and over HTTP, plus the SSE
stream. For a feature that has to hold on every wire, copy the shape of
`apps/auth-server/tests/surfaces.rs` (one scenario over direct,
`LocalServer`, WebSocket, iroh and HTTP). See
[testing strata](@/architecture/testing.md).

## Scaffolding your own feature

`cargo xtask feature new <name>` writes one crate at `features/<name>/`
([the layout](@/architecture/layout.md)): the contract in `src/lib.rs`, an
in-memory backend behind the `memory` flag, `vox` / `ui` flags, a `spec/`
and a passing scenario test, and wires it into the workspace. Rename the
placeholder entity, flesh out the backend, and copy
`examples/greeter/src/main.rs` when you need a server.
