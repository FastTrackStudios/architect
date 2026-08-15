+++
title = "Build a feature, end to end"
description = "Define an entity, pick a backend, serve it, consume it — the whole architect flow."
weight = 15
+++

This walks the entire architect flow against the reference example
(`examples/app/`). Each step points at the real file, so you can read the
working code alongside.

## The mental model

```
#[derive(Entity)]  ──▶  wire struct + Create/Update/List + <Repo> vox trait
                        (+ SeaORM storage under `server-seaorm`)

a backend          ──▶  impl <Repo> for it  +  impl Services (its Layer bundle)

Layer / service_router  ──▶  one LayerRouter that dispatches by method id

Transport          ──▶  serve the router remotely (axum WS) or in-process
                        (vox in-memory link); clients are identical either way
```

You write the struct and the backends; architect emits the wire types,
the repo trait, the storage, and the client. You choose, at the edge, how
the client reaches the backend.

## 1. Define the entity

One plain struct with `#[derive(architect::Entity)]`
([`examples/app/features/example/example-proto/src/lib.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/features/example/example-proto/src/lib.rs)):

```rust
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "examples", repo)]
pub struct Example {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable, fulltext)]
    pub name: String,
    #[architect(filterable, fulltext)]
    pub description: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}
```

The derive emits `Example` + `ExampleCreate`/`ExampleUpdate`/`ExampleList`,
the `ExampleRepo` `#[vox::service]` trait (→ `ExampleRepoClient` +
`ExampleRepoDispatcher`), and — because the repo is a vox service — an
`ExampleRepoLayer` token so the repo composes in the layer system. Under
`server-seaorm` it also emits the SeaORM `Model`/`Entity`/… +
`ExampleRepoStorage<C>`. See [the architect pattern](@/architecture/pattern.md)
for the full field/attribute table.

## 2. (Optional) a domain service

Operations that don't fit CRUD live on a hand-written `#[vox::service]`
trait next to the entity — `ExampleService` with `search` / `duplicate`
in the same file. It gets its own client/dispatcher just like the repo.

## 3. Pick or write a backend

A backend is any type that `impl ExampleRepo` **and** declares its
`Services` bundle. The example ships several, all interchangeable:

| Backend | Crate | Notes |
| --- | --- | --- |
| `ExampleRepoMemory` | `example-memory` | `RwLock<Vec<_>>`; tests, demos, offline |
| `ExampleRepoStorage<C>` | `example-db` (derive-emitted) | SeaORM/SQLite; `Services` impl emitted for free |
| `ExampleRepoLoro` | `example-crdt` | Loro CRDT; local-first |
| `StubBackend` | `examples/external-stub` | a third-party / out-of-tree impl |

A backend's bundle is one line ([`example-memory/src/lib.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/features/example/example-memory/src/lib.rs)):

```rust
impl architect::Services for ExampleRepoMemory {
    fn layers() -> impl architect::Layer<Self> { architect::layers![ExampleRepoLayer] }
}
```

Need primary+replica or sharding (two backends of the same trait)? Don't
reach for tags — write one backend that holds both and routes internally
(`impl ExampleRepo for RoutingRepo`); see [idioms §7](@/architecture/idioms.md).

## 4. Serve it — one router, any transport

Compose the whole vox surface (repo CRUD + `ExampleService`) into one
`LayerRouter` once, then serve it. The example factors this into
`app_server::service_router(repo)`
([`examples/app/server/src/lib.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/server/src/lib.rs)):

```rust
pub fn service_router<R: ExampleRepo + Services + Clone + Send + Sync + 'static>(repo: R) -> LayerRouter {
    repo.clone().into_router()                      // the repo's Services bundle
        .with(example::example_service_service_descriptor(),
              ExampleServiceDispatcher::new(ExampleServiceImpl::new(repo)))   // + the domain service
}
```

- **Remotely (axum WebSocket):** `vox_router(repo)` mounts that router on
  `/vox`. The server `main.rs` builds the backend with a `Resource` under
  a `Scope` and closes it on Ctrl-C (graceful shutdown).
- **In-process (no server):** `architect::LocalServer::serve(service_router(repo), scope)`
  serves the same router over a vox in-memory link. Native only.

## 5. Consume it — program against the client, inject the transport

Screens/CLI program against `ExampleRepoClient` and never hard-code where
it lives. The `Transport` is chosen once at the app root
([`examples/app/ui/src/transport.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/ui/src/transport.rs)):

```rust
match transport {
    Transport::Remote(url) => /* WsLink::connect + establish::<…>() */,
    Transport::Local(server) => /* server.establish::<…>() — in-process */,
}
```

- **web** → `Transport::Remote("ws://…/vox")` (the browser stays remote).
- **desktop** → `Transport::Local(service_router(ExampleRepoMemory::new()))`
  — the whole stack runs in-process, **no server to launch**.

Same screens, same client types. The `CLI` (`examples/app/cli`) is the raw
client form: `WsLink::connect` + `establish::<ExampleRepoClient>()`.

## 6. Compose & build backends (the DI engine)

For real wiring (`config → pool → repo`), use `architect::Resource` — a
lazy builder with dependent composition, memoization, and scoped teardown
([idioms §6](@/architecture/idioms.md)):

```rust
let scope = Scope::new();
let repo = Resource::from_fn(|_| async { load_config() })
    .and_then(|cfg| Resource::acquire_release(connect_pool(cfg), |p| async move { p.close().await }))
    .memoize()                       // built once, shared
    .map(ExampleRepoStorage::new)
    .build(&scope).await?;
// … serve service_router(repo) …
scope.close().await;                 // finalizers run LIFO (pool closes)
```

`LayerGraph` plans a multi-node backend graph (topological order +
cycle / missing-provider / conflict diagnostics) — see
`examples/layered-services`.

## 7. Test it

| Layer | Where | What |
| --- | --- | --- |
| Native repo | `features/example/tests/native` | trait behaviour against memory + CRDT + stub |
| Native e2e | `app/tests/e2e` | real clients over a WebSocket **and** over the in-process link, no server for the local one |
| Browser | `features/example/tests/web` | the same contract over a real WS from wasm |
| Component | `example-ui/tests`, `app/ui/tests` | SSR-render the Dioxus components + app shell |

See [testing strata](@/architecture/testing.md). Run `cargo nextest run
--workspace` for the native layers and `just test-e2e` for the browser one.

## Scaffolding your own feature

`just scaffold-feature <name>` (or `cargo run -p architect-cli -- feature
new <name>`) drops the canonical proto + memory backend + facade + spec +
native tests into `features/<name>/` and wires the workspace. Rename the
placeholder entity, flesh out the backend, and you have the same surface
this walkthrough used.
