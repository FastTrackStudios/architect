+++
title = "Extensibility"
description = "Two ways to add features: in-tree (monorepo) and external (third-party crate)."
weight = 25
+++

A project built on architect can be extended along two axes, and both
are first-class. Understanding the difference is mostly about
understanding **who depends on what**.

## The two axes

```text
       ┌──────────────────┐                  ┌──────────────────┐
       │  <feature>-proto │ ◄──── contract ──┤  <feature>-proto │
       │  (trait + types) │                  │  (trait + types) │
       └──────────────────┘                  └──────────────────┘
                ▲                                     ▲
                │                                     │
   ┌────────────┴───────────┐         ┌───────────────┴────────────┐
   │  in-tree implementations│         │  external implementations  │
   │                         │         │                            │
   │  features/<feature>/    │         │  someone-elses-crate       │
   │    <feature>-<backend>/ │         │  on crates.io              │
   │                         │         │                            │
   │  shipped with the app   │         │  brought in à la carte by  │
   │  binary by default      │         │  consumers who want it     │
   └─────────────────────────┘         └────────────────────────────┘
```

Both implement the same `#[vox::service]` trait that `<feature>-proto`
declares. From the trait's perspective, in-tree and external impls are
indistinguishable.

## Path 1 — In-tree (monorepo)

Crate sits inside the project's `features/<feature>/` tree alongside
the contract. Lives in the same workspace, ships from the same
`Cargo.lock`, gets pulled into the project's app binary directly.

This is what `example-db` and `example-memory` already are in this
repo:

```text
features/example/
  example-proto/        contract
  example-db/           in-tree impl  ←
  example-memory/       in-tree impl  ←
  example/              facade — pick which in-tree impls to enable
```

The facade crate (`example`) exposes cargo features:

```toml
[features]
backend-db = ["dep:example-db", "example-proto/server"]
backend-memory = ["dep:example-memory"]
```

The app binary (`app-cli`, `app-server`) opts in:

```toml
# apps/app/server/Cargo.toml
example = { workspace = true, features = ["backend-db"] }
```

**Use this when**:

- You own the implementation and it's specific to this project
- You want zero coordination cost (PR straight into the repo)
- The impl needs to track contract changes in lockstep
- Versioning is handled by `git tag` on the monorepo, not crates.io

**DAW analogue**: `daw-reaper`, `daw-protools`, `daw-ableton` —
written by the team that owns the `daw` repo, all live at
`features/<feature>/<feature>-reaper/` etc. The `daw` binary builds
with whichever combination of cargo features that project's release
needs.

## Path 2 — External (third-party crate)

Crate lives in someone else's repo, publishes to crates.io, depends
only on `<feature>-proto`. A consuming project pulls it in like any
other dependency.

**The contract crate is the only required surface.** Everything else
(facade, in-tree backends, app binary) is convenience — third parties
can replace any of it.

```toml
# Cargo.toml of a project consuming an external impl
[dependencies]
example-proto = "0.2"
daw-logic-backend = "0.1"   # third-party — implements example_proto::ExampleRepo
```

Then in the consumer's binary:

```rust
use example_proto::ExampleRepoDispatcher;
use daw_logic_backend::LogicBackend;

let backend = LogicBackend::new(/* … */);
let dispatcher = ExampleRepoDispatcher::new(backend);
// mount on vox WebSocket exactly like the in-tree backends
```

**Use this when**:

- The implementation is reusable across projects ("a Stripe backend
  for any architect-shaped billing feature")
- Different teams own the contract and the impl
- The impl wants its own release cadence
- The contract is stable enough that third parties can rely on it
  without coordinating

**DAW analogue**: someone outside the daw team writes `daw-logic` —
a Logic Pro backend that implements the same `timeline_proto::TimelineRepo`
+ `mixing_proto::MixingRepo` traits as `daw-reaper` does in-tree.
They publish to crates.io. A user can then build their own `my-daw`
binary that pulls `daw-cli` (in-tree first-party backends) AND
`daw-logic` (external) — both implement the same contracts, both
mount on the same vox dispatcher.

## What changes between the two paths

| Concern | In-tree | External |
|---------|---------|----------|
| Where does the source live? | `features/<feature>/<feature>-<x>/` | Wherever the author wants |
| What does it depend on? | `<feature>-proto` (path) | `<feature>-proto` (crates.io) |
| How is it enabled? | cargo feature on the facade | direct dep + manual wiring in the binary |
| Who maintains the version? | This project's git history | The author, via crates.io semver |
| Can it be added without a fork? | No — needs a PR | Yes — `cargo add` and wire it in |

The contract crate (`<feature>-proto`) is the boundary that makes the
difference invisible to the running code. Both paths produce a struct
that implements the same `#[vox::service]` trait; the vox dispatcher
takes either one and serves it on the same WebSocket.

## How to design a contract that supports both

Two rules that have to hold for external impls to be possible at all:

**1. `<feature>-proto` must publish without `server` features active.**

If the trait declaration depends on SeaORM or any other server-side
machinery, external impls inherit that constraint. The proto crate
should be wasm-clean by default (which architect's `Entity` derive
already enforces) so a third party can implement the trait against
a Redis backend or a remote HTTP API without pulling in a SQL
dependency they don't want.

**2. The facade is optional, not required.**

`<feature>` (the facade) is a convenience for first-party in-tree
selection. Third parties should be able to skip it entirely and
depend only on `<feature>-proto`. The example app's facade is fine
for first-party builds; consumers who pull in external impls just
won't use it.

## Dependency injection: repo *and* service

Both halves of the architect surface are traits. They swap
independently:

```text
┌──────────────────────────────────────────────────────────────────────┐
│  <feature>-proto declares two traits + the wire types               │
│                                                                      │
│    pub trait <T>Repo     { async fn get/list/create/update/delete } │
│    pub trait <T>Service  { async fn search/duplicate/...           }│
└──────────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
   pick a repo:          pick a service:        binary assembles:
   - SeaORM (architect    - architect doesn't    let repo  = ...;
     emits this when        emit one — write     let svc   = MyService::new(repo.clone());
     `server-seaorm`)       it by hand           mount(<T>RepoDispatcher::new(repo));
   - in-memory                                   mount(<T>ServiceDispatcher::new(svc));
   - sqlx/postgres
   - HTTP proxy
   - mock for tests
```

Concrete consequences:

- **A third party can replace the architect-emitted SeaORM repo.**
  Disable `architect/server-seaorm` on the proto crate and the
  derive never compiles in the SeaORM bridge. Write your own struct
  implementing `<T>Repo` against sqlx/postgres/redis/whatever.
- **A third party can replace the service.** `<T>Service` is a
  hand-written `#[vox::service]` trait — anyone can write a
  different impl with different business rules, mount it on the
  same dispatcher.
- **A binary picks one of each.** No fixed coupling between
  "architect-generated repo" and "architect-generated service"
  because the second doesn't exist — services are always yours.

`architect`'s feature flags reflect the split:

| Feature | Pulls | What you get |
|---------|-------|--------------|
| `vox`           | `vox` (+ moire + facet-cbor + …) | Makes the architect-emitted `<T>Repo` trait a `#[vox::service]` — generates dispatcher + client. Without this, the trait is plain Rust 2024 async-in-trait, RPC types don't exist. |
| `server-seaorm` | `sea-orm` + `async-trait` | The SeaORM bridge → `<T>RepoStorage<C>` impl. Only enable if you want the architect-emitted SeaORM repo. |
| `server-axum`   | `axum` + `tokio` + `futures` + `tracing` + `vox-core` + `vox-types` | The `architect::axum_ws` adapter for mounting any dispatcher on an axum WS route. |
| `full`          | all of the above | tokio-style convenience: turns everything on. |

They're all independent. Three useful combinations:

```toml
# In-process trait use only — drives the repo from Rust, no RPC, no
# storage opinion. Pulls only `architect` + `facet`.
architect = { ..., default-features = false }

# Client crate for a vox-served binary — wasm app, native CLI, etc.
# No storage bridge, no axum.
architect = { ..., features = ["vox"] }

# Custom server (your own repo impl) — picks transport but skips the
# SeaORM emission. No `sea-orm` in the dep tree.
architect = { ..., features = ["vox", "server-axum"] }

# Everything. Single-binary first-party app, dev experimentation.
architect = { ..., features = ["full"] }
```

Feature crates (e.g. `example-proto`, `example`) carry their own
`full` features that bundle whatever subset is meaningful for that
layer. `example-proto/full` = `vox + server`. `example/full` =
`backend-db + backend-memory + example-proto/full`.

## The working examples

Two demonstrators live in `examples/`:

### `examples/external-stub/` — third-party repo shape

```text
examples/external-stub/
  Cargo.toml      depends only on example-proto + architect
                  (no server-seaorm, no in-tree backend)
  src/lib.rs      pub struct StubBackend impls ExampleRepo
```

```text
examples/
  external-stub/
    Cargo.toml      depends only on example-proto + architect
    src/lib.rs      pub struct StubBackend impls ExampleRepo
```

It sits **outside** `features/` on purpose — that's the visual cue
this isn't a first-party impl. Its `Cargo.toml` would look identical
if it lived in a separate repository and pulled its deps from
crates.io; only the path-vs-version of two dep lines changes.

### `examples/custom-server/` — binary assembled by hand

```text
examples/custom-server/
  Cargo.toml      depends on example-proto + example-stub-backend +
                  architect[server-axum]
                  NOTHING from sea-orm, the `example` facade, or
                  any in-tree backend.
  src/main.rs     - pick repo (StubBackend::with_seed_data)
                  - write a service impl (CustomExampleService<R>,
                    generic over any ExampleRepo)
                  - mount both dispatchers on /vox via architect::axum_ws
```

Smoke test that confirms the wiring is genuinely backend-agnostic:

```sh
$ cargo run -p example-custom-server &
$ cargo run -q -p app-cli -- list --url ws://127.0.0.1:4041/vox
3 examples
  385ae81f-...  stub.alpha
  062e7e27-...  stub.bravo
  ed8bc20b-...  stub.charlie
```

`app-cli` was built against the first-party (SeaORM-aware) workspace,
but the server it's talking to has *zero* SeaORM in its dependency
graph. The contract is what holds it together.

### Contract test

`features/example/tests/native/` exercises both `ExampleRepoMemory`
and `StubBackend` through the same trait surface — and the test
bodies don't reference the concrete backend type:

```rust
async fn external_backend_round_trip() {
    let r = example_stub_backend::StubBackend::new();
    let created = r.create(ExampleCreate { /* ... */ }).await.unwrap();
    let got = r.get(created.id).await.unwrap();
    assert_eq!(got.id, created.id);
}
```

The test body never references which concrete backend it's running
against — proof that the contract is the only thing that matters
at the consumer's call site.

## When you want both at once

A real DAW build will mix paths. The same binary might use
`daw-reaper` (in-tree) for timeline, `daw-mock` (in-tree) for tests,
and `daw-cloud-sync` (external, on crates.io) for collaboration.
Architect doesn't care — every backend implements the same trait,
the vox dispatcher takes anything that does, and cargo features
gate compilation.
