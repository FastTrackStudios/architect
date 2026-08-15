+++
title = "The in-process transport"
description = "architect::local — serve a LayerRouter over a vox memory link: typed clients, no server, no socket."
weight = 35
+++

`architect::local` (the `local` feature, implies `vox`) serves a
`LayerRouter` over a vox **in-memory link**, so the generated typed
clients — `ExampleRepoClient`, `AuthServiceClient`, any
`<Trait>Client` — consume a backend running *in the same process*. No
HTTP server, no WebSocket, no port.

This is the consumer-facing half of
[idioms §7 — inject the transport, not the backend](@/architecture/idioms.md):
screens, CLIs, and tests program against the client types; *where* the
backend lives is decided once, at the root.

## The surface

Two items:

```rust,ignore
use architect::{LocalServer, Scope, serve_local};

// 1. Serve an already-built router…
let local = LocalServer::serve(router, scope.clone());

// 2. …or the shortcut for a `Services` backend (= backend.into_router()):
let local = serve_local(ExampleRepoMemory::new(), &scope);

// Either way: establish typed clients, same as over a WebSocket.
let repo: ExampleRepoClient = local.establish().await?;
```

`LocalServer` is `Clone` — hand it to whatever owns the app root.
Each `establish::<C>()` opens a fresh memory-link pair plus an acceptor
task (one client per session, mirroring the remote per-service
connection shape); those tasks are registered on the `Scope` and
aborted when it closes. Same acceptor dance as `axum_ws::serve`, just
over `vox_core::memory_link_pair` instead of a socket.

## Full round-trip, no server

From the e2e suite (`examples/app/tests/e2e`,
`local_transport_round_trip`) — the **same** `service_router` the axum
server mounts, identical asserts to the WebSocket path:

```rust,ignore
use app_server::service_router;
use architect::{LocalServer, Scope};

let scope = Scope::new();
let local = LocalServer::serve(
    service_router(ExampleRepoMemory::new(), &app_server::Collab::ephemeral()),
    scope.clone(),
);
let repo: ExampleRepoClient = local.establish().await?;
let service: ExampleServiceClient = local.establish().await?;

let created = repo.create(ExampleCreate {
    name: "local".into(),
    description: "in-process".into(),
}).await?;
assert_eq!(repo.get(created.id).await?.name, "local");

let hits = service.search("local".into(), 10).await?;   // hand-written rpc
assert_eq!(hits.len(), 1);

scope.close().await;   // aborts the acceptor tasks
```

Every byte still goes through facet encoding and vox dispatch — this
is a real wire test minus the network, which is exactly what makes it
fast enough to live in the default test run. (It's also how
streaming subscriptions are tested: establish a `<T>EventsClient` /
`<T>StreamClient` the same way — see
[server push](@/architecture/streams.md).)

## When to reach for it

| Situation | Why local |
| --- | --- |
| **Desktop app** | whole stack in-process; ship one binary, no server to launch |
| **CLI tools** | command runs against an embedded backend (or a remote one — same client code) |
| **Tests** | full-wire coverage at unit-test speed; see [testing strata](@/architecture/testing.md) |
| **Servers, internally** | a server-side consumer of its own services without loopback HTTP |

The desktop app (`examples/app/desktop`) is the headline use. Its root
is the entire difference from the web build:

```rust,ignore
#[component]
fn Root() -> Element {
    let transport = use_hook(|| {
        let scope = Scope::new();
        let router = service_router(ExampleRepoMemory::new(), &app_server::Collab::ephemeral());
        Transport::Local(LocalServer::serve(router, scope))
    });
    use_context_provider(|| transport.clone());
    rsx! { App {} }
}
```

Same `App`, same screens, same client types as the browser — only the
`Transport` differs, proving remote and local are behaviour-identical
from the UI's point of view.

## Native only

vox's `MemoryLink` isn't compiled for wasm, so `architect::local` is
cfg'd off on `wasm32` — browser clients stay remote
(`Transport::Remote(url)` over `WsLink`). The example UI cfg-gates its
`Transport::Local` arm accordingly; copy that pattern in shells that
build for both targets.
