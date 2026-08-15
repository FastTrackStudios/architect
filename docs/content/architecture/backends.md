+++
title = "Multi-backend features"
description = "One contract, swappable storage."
weight = 20
+++

A feature defines its contract once (in `<feature>-proto`) and ships
one or more implementations as sibling crates. A facade picks which
implementation a binary uses via cargo features.

## Concrete example

The reference template ships two implementations of `ExampleRepo`:

| Crate | Implementation | Why |
|-------|----------------|-----|
| `example-db` | SeaORM/SQLite | Production storage. |
| `example-memory` | `RwLock<Vec<Example>>` | Tests, demos, ephemeral processes. No external service needed. |

Both implement the same `ExampleRepo` trait that `example-proto` exposes.
The wasm browser tests don't care which backend the server is running —
they call `ExampleRepoClient::create / get / list / delete` and the
contract holds.

## The facade

`examples/app/features/example/example/src/lib.rs` chooses between them
with cargo features:

```toml
[features]
default = []
backend-db = ["dep:example-db", "example-proto/server"]
backend-memory = ["dep:example-memory"]
```

Both backends can be enabled in the same compile (e.g. tests). They
live in different modules to avoid name collisions:

```rust
#[cfg(feature = "backend-db")]
pub mod backend_db {
    pub use example_db::{ExampleRepoStorage, Migrator};
}

#[cfg(feature = "backend-memory")]
pub mod backend_memory {
    pub use example_memory::ExampleRepoMemory;
}
```

## In-tree vs external implementations

Everything on this page describes the **in-tree** pattern: backends
live inside the project's `features/<feature>/` tree alongside the
contract. Third parties can implement the same trait surface from
their own crates without forking the monorepo — see
[Extensibility](@/architecture/extensibility.md) for that path. The
contract (`<feature>-proto`) is what makes both paths interchangeable
from the running app's perspective.

## Pattern for the DAW analogue

Each DAW backend (Reaper, Ableton, Pro Tools) lives as a sibling
implementation of the relevant feature contracts:

```
features/timeline/
  timeline-proto/        contract — `TimelineRepo`, `ClipRepo`, ...
  timeline-reaper/       Reaper-specific clip + track storage
  timeline-ableton/      Ableton Live Object Model adapter
  timeline-protools/     Pro Tools control surface adapter
  timeline-mock/         in-memory for tests
  timeline/              facade with backend-reaper / backend-ableton / ...
```

A `crates/daw-reaper` binary then pulls `timeline` with `backend-reaper`,
`mixing` with `backend-reaper`, etc. `crates/daw-ableton` does the same
with `-ableton` features. The same `apps/daw-ui` shell composes both
because both expose the same `<feature>_proto` types and traits.

## Large payloads

A vox return value serializes whole. For megabyte-scale results — an FX
state chunk, a rendered audio block — that's megabytes of encoding per
call even when the client only wants a slice. `architect::handle` keeps
the allocation in the backend instead:

| Crate side | Type | Role |
|------------|------|------|
| proto (wire) | `RawHandle` | Opaque id, facet-serializable (a bare UUID). What service signatures carry. |
| backend | `HandleRegistry<T>` | Thread-safe table owning the values; mints `Handle<T>`s. |
| backend | `Handle<T>` | Typed wrapper around `RawHandle` — a registry of one type can't deref a handle of another. |

**Send the bytes** when the payload is small (tens of KB), consumed
whole, or needed offline — a handle that outlives its server entry is
just a miss. **Send a handle** when the payload is large, when the
client only ever derefs ranges of it (the DAW's audio accessors: render
once server-side, `get_samples(handle, start, len)` per window), or when
the resource isn't a wire type at all (FFI pointers, mmaps).

Eviction is the **caller's policy**: pair every handle-returning method
with a `release` method, and sweep with `HandleRegistry::retain` for
sessions that leak. Chunked / streaming responses — moving the bytes
themselves incrementally rather than parking them behind an id — are a
separate planned facility, not what handles are for.
