+++
title = "Construction: Resource & Scope"
description = "Build the backend — config → pool → repo — with dependent composition, build-once sharing, and LIFO teardown."
weight = 27
+++

`Layer<B>` answers "given a backend, mount its services"
([idioms §6](@/architecture/idioms.md)). `architect::resource` answers
the step *before* that: **build** the backend, with dependencies,
sharing, and ordered cleanup. It's the Effect-`Layer` idea (dependency
wiring + scoped resources) as plain value-level combinators — no
`Effect<A,E,R>` monad, no typed requirement tracking, no feature flag
(the `Scope`/`Resource` surface is transport-agnostic and wasm-clean).

## The two nouns

- **`Resource<T, E = eyre::Report>`** — a lazy, composable *recipe*
  that builds a `T`. Nothing runs until `.build(&scope)`. Name a
  concrete `E` for typed failures; the default is fine for binaries.
- **`Scope`** — collects async finalizers and runs them in **reverse
  registration order** (LIFO) on `scope.close().await`. The pool opened
  after the config is closed before it. A second `close()` is a no-op.

```rust,ignore
let scope = Scope::new();                    // Arc<Scope>
let value = recipe.build(&scope).await?;     // run the recipe
// … use value …
scope.close().await;                         // finalizers, LIFO
```

The scope is an `Arc` so the resource graph and the eventual owner —
a server's shutdown path — share the same one.

## Combinators

| Combinator | Effect |
| --- | --- |
| `Resource::succeed(v)` | already a value — no work, no failure |
| `Resource::from_fn(f)` | build from an async closure; `f` receives the `Scope` so it can `defer` cleanup or build sub-resources |
| `Resource::acquire_release(acquire, release)` | build via `acquire`, register `release` as a LIFO finalizer (needs `T: Clone`) |
| `.and_then(f)` | **dependent** composition — `f: T -> Resource<U>`, the `config → pool → repo` chain; both steps' finalizers stack on the same scope |
| `.zip(other)` | **independent** pair — build both, return the tuple |
| `.map(f)` / `.map_err(f)` | transform the value / the error |
| `.memoize()` | build **once**, share — see below |
| `.into_router(&scope)` | terminal step on the server: build a `Services` backend and mount its bundle → `LayerRouter` |

## The real thing: the example server

`examples/app/server/src/main.rs` builds its sqlite backend exactly
this way — the pool is acquired under the scope so Ctrl-C closes it
gracefully:

```rust,ignore
use example::architect::{Resource, Scope};

fn resource() -> Resource<ExampleRepoStorage<DatabaseConnection>> {
    // config → pool (acquire_release: closed on scope teardown) → repo
    Resource::from_fn(|_| async {
        Ok(std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://./example.db?mode=rwc".into()))
    })
    .and_then(|url| {
        Resource::acquire_release(
            Resource::from_fn(move |_| async move {
                let db = Database::connect(&url).await?;
                Migrator::up(&db, None).await?;
                Ok(db)
            }),
            |db: DatabaseConnection| async move {
                tracing::info!("closing database pool");
                let _ = db.close().await;
            },
        )
    })
    .map(ExampleRepoStorage::new)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let scope = Scope::new();
    let repo = resource().build(&scope).await?;

    axum::serve(listener, vox_router(repo, collab))
        .with_graceful_shutdown(shutdown_signal())   // Ctrl-C
        .await?;

    scope.close().await;   // ← the pool's finalizer runs here
    Ok(())
}
```

The in-memory backend is the degenerate case —
`Resource::succeed(ExampleRepoMemory::new())` — which is why backend
selection stays a one-line `cfg` swap in the bin while the rest of the
chain is identical.

## Memoization: one pool, many repos

Backends often share a resource. `.memoize()` turns a `Resource<T>`
into a cloneable `SharedResource<T>`: the first `.build()` (on any
clone) runs the recipe and caches the value; every later build returns
the same instance. Failed builds are **not** cached — they can be
retried. `.resource()` views the shared handle as a fresh `Resource`
so it feeds further `.map`/`.and_then` chains:

```rust,ignore
let pool = pool_resource().memoize();                  // builds at most once

let users  = pool.resource().map(UserRepoStorage::new);
let orders = pool.resource().map(OrderRepoStorage::new);

let (users, orders) = users.zip(orders).build(&scope).await?;  // one pool
```

## Feeding the service layer

A construction chain on the server terminates in
`Resource::into_router` (vox-gated): build the backend, then mount its
[`Services`](@/architecture/idioms.md) bundle —
`Resource<B>` → `LayerRouter`:

```rust,ignore
let router = resource().into_router(&scope).await?;   // build + into_router()
// serve router …
scope.close().await;
```

That's the whole pipeline: **`Resource` builds the backend, `Layer`
binds its services, the transport in front of the `LayerRouter` is
chosen last** — axum WebSocket for a server, the
[in-process transport](@/architecture/local.md) for desktop/CLI/tests.

For multi-node backend graphs there's also a pure-data planner —
`LayerGraph`/`LayerNode::plan()` returns a topological build order or a
precise diagnostic (missing provider, conflict, cycle, duplicate). See
the end of `examples/layered-services/src/main.rs`.

## Scope rules of thumb

- **One scope per lifetime.** A process gets one (closed after the
  server stops); a test gets its own (closed at the end — the e2e
  suite's `local_transport_round_trip` does exactly this).
- **`defer` for anything that must outlive an await.** Finalizers are
  async; `scope.defer(move || async move { … })` is the escape hatch
  when `acquire_release` doesn't fit the shape.
- **LIFO is the contract.** Registration order is teardown order,
  reversed — verified by `resource.rs`'s own unit tests
  (`scope_closes_finalizers_lifo`).
