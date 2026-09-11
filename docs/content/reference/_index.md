+++
title = "Reference"
description = "Crate map, derive attributes, and macro outputs."
weight = 30
+++

## Crate map

architect itself lives in `crates/architect` (facade) +
`crates/` (its proc-macros); `examples/greeter` is the reference
feature a real project would copy, and `cargo xtask feature new <name>`
scaffolds one.

| Crate | Path | Role |
|-------|------|------|
| `architect` | `crates/architect` | User-facing — re-exports the derive + runtime (layer / `Resource` / `local`). |
| `architect-derive` | `crates/architect-derive` | The `#[derive(Entity)]` / `#[derive(JsonField)]` proc-macro. |
| `architect-rpc-derive` | `crates/architect-rpc-derive` | `#[architect::service]` / `rpc` / `http` — sync/async trait → vox service + `Layer` token + HTTP router + HTTP client. |
| `architect-action-derive` | `crates/architect-action-derive` | `#[architect::actions]` — named-command traits with menu/CLI metadata. |
| `crdt`, `crdt-seaorm`, `crdt-derive` | `features/crdt/` | Loro CRDT layer + its SeaORM persistence — the local-first feature every `#[architect(crdt)]` entity builds on. |
| `example-greeter` | `examples/greeter` | The reference feature + server: entity, service, backend, every wire, the tests. Copy this. |
| `auth-proto`, `auth`, `auth-db`, `auth-client`, `auth-ui`, `architect-auth` | `features/auth/` | The identity feature — see [the auth feature](@/architecture/auth.md). |
| `auth-server` | `apps/auth-server` | The identity server: the generated faces plus the OAuth/OIDC and page plugins. |
| `xtask` | `xtask` | `cargo xtask feature new <name>` scaffolds a feature; tags, mirror, fleet tooling. |

## Cargo features

| Crate | Feature | Effect |
|-------|---------|--------|
| `architect` | `vox` | Enable the `#[vox::service]` surface + the `layer` module. |
| `architect` | `server-seaorm` (alias `server`) | SeaORM storage helpers (`storage::DbConn`); derive emits `<T>RepoStorage`. |
| `architect` | `server-axum` | axum adapter — `architect::axum_ws` (Link + `serve`). |
| `architect` | `http` | The HTTP+JSON face over axum. Off, `architect::http` is an inert facade with the same names, so derive-emitted routers still compile (a wasm client, a proto crate on its own). Implied by `server` and `host`. |
| `architect` | `http-client` | `architect::http::HttpClient` over reqwest (native + wasm). Off, the generated `<Trait>HttpClient`s exist but answer every call with a `Transport` error. |
| `architect` | `host` | `architect::host::EngineHost` — `/vox` + `/health` + plugins + optional iroh + SPA bundle from one router. |
| `architect` | `local` | In-process transport — `architect::{LocalServer, serve_local}` (native). |
| `architect` | `platform` | Portable clock/sleep/spawn — `architect::platform` (native tokio ↔ wasm browser timers). |
| `architect` | `schedule` | Retry/repeat policies — `architect::{Schedule, retry, repeat}` (implies `platform`). |
| `architect` | `fake` | `#[derive(fake::Dummy)]` on emitted structs for seeding. |
| any `*-proto` | `vox` (default) / `server` | **The whole proto-crate profile.** `vox` is the RPC face; `server` adds the SeaORM storage + migration. HTTP, the HTTP client and `fake` need no forwarding. |

## Vox dependency convention

`vox = { default-features = false, features = ["runtime"] }` is the
wasm-clean baseline used in every `*-proto` and shared library crate.
Native servers add `transport-websocket` if they go through
`vox::serve` directly (architect's bundled `axum_ws` adapter doesn't
need it). Wasm test/client crates depend on `vox-core` and
`vox-websocket` directly via `[target.'cfg(target_arch = "wasm32")']`.

## Layer / construction / transport surface

The runtime DI engine (see [idioms §6–7](@/architecture/idioms.md)):

| Item | Module | Role |
|------|--------|------|
| `Layer` / `Services` / `layers!` / `LayerRouter` | `architect::layer` | compose service tokens, bind a backend → router |
| `Resource<T, E>` / `Scope` / `SharedResource` | `architect::resource` | lazy backend builders — `and_then` / `zip` / `acquire_release` / `memoize`, LIFO teardown |
| `LayerGraph` / `LayerNode` / `LayerPlan` | `architect::plan` | declared-graph topological planner + diagnostics |
| `LocalServer` / `serve_local` | `architect::local` (feature `local`, native) | serve a router in-process over a vox in-memory link |
| `Schedule` / `retry` / `repeat` | `architect::schedule` (feature `schedule`) | composable retry/repeat policies — backoff, jitter, caps; see [scheduling](@/architecture/scheduling.md) |
| `Supervisor` / `Restart` / `Supervised` | `architect::supervisor` (feature `schedule`) | keep a service loop alive — restart-under-policy with `Schedule` backoff, cancellable |
| `Clock` / `SystemClock` / `TestClock` / `timeout` / `spawn` (`JoinHandle`) / `CancellationToken` / `Deferred` / `Semaphore` / `Queue` | `architect::platform` (feature `platform`) | the portable async-runtime seam — clock, tasks, timeouts, cancellation, concurrency primitives (native↔wasm) |

## What the derive emits

See [The architect pattern](@/architecture/pattern.md) for the full
list of fields and the input → output mapping.

## `#[architect::service]` and `#[architect::rpc]` mechanics

`#[architect::service]` is `#[architect::rpc]` plus the HTTP face
(see [the HTTP face](@/architecture/http.md)); the consumer's cargo
features (`vox`, `http`, `http-client`) decide which faces compile.
Both take the same options.

Given a trait, the macro classifies each method **sync** or **async**
and adapts what it emits: all-sync traits get a bridge that marshals
every call onto a `Dispatcher` plus an async mirror for the vox
client/host; all-async traits are already their own RPC face; mixed
traits bridge the sync methods and pass the async ones through
unchanged. `#[subscribe]` methods are stream declarations, not calls —
see [streams](@/architecture/streams.md).

**Object-safety requirements** (checked at macro-expansion time, with a
clear compile error on violation):

- Methods take `&self` — never `&mut self` or by-value `self`.
- No generic type or const parameters on the method.
- No borrowed return types, and no `Self` returns.

**Argument rewriting** (sync trait → owned async mirror, so the
bridge's closures can capture arguments across the dispatch boundary):

| Sync-trait argument | Mirror-trait argument |
|---|---|
| `&str` | `String` |
| `&[T]` | `Vec<T>` |
| other `&T` | `T` (the backend must impl `Clone`) |

**`Dispatcher`** (`architect::dispatch`) marshals a sync closure onto a
runtime-appropriate execution context; it's object-safe
(`Arc<dyn Dispatcher>`) so the choice composes at runtime.
`DispatchError` (`ShutDown` / `Panicked` / `Cancelled`) wraps
transport-level dispatch failures — application errors returned by the
closure flow through unchanged. Two dispatchers ship in `architect`:
`CurrentThreadDispatcher` (calls inline — tests, in-process callers)
and `TokioBlockingDispatcher` (`spawn_blocking`, feature
`dispatch-tokio`, the server default — see
[feature flags](@/architecture/features.md)). Runtime-specific
dispatchers (a UI main-thread queue, a hardware-API thread) live in
their own crates and implement the same trait.

## Glossary

- **Service** — a trait describing operations on a domain concept,
  annotated with `#[architect::rpc]` (or emitted by `#[architect(repo)]`).
  The trait *is* the service; the macro derives the network face.
- **Entity** — a struct describing data shape, annotated with
  `#[derive(architect::Entity)]`. Travels over the wire as `facet::Facet`.
- **Host** — the server-side wrapper (`<T>Host`) that mounts a backend
  impl on a vox router.
- **Client** — the caller-side proxy (`<T>Client`) that talks to a
  `<T>Host` over vox.
- **Bridge** — the adapter inside `<T>Host` that turns `Arc<dyn T>` +
  `Dispatcher` into the hidden async mirror trait vox serves. Not
  user-visible.
