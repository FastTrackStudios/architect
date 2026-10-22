+++
title = "Multi-backend features"
description = "One contract, swappable storage."
weight = 20
+++

A feature defines its contract once, in the default feature set of its
one crate ([layout](@/architecture/layout.md)), and ships each
implementation as a cargo feature of that same crate. A binary picks
which implementation it compiles by enabling the flag.

## Concrete example

`cargo xtask feature new inventory` writes one backend; a second one is
another flag and another module beside it:

<!-- doc-check: off (illustrative names, not paths in this repo) -->
| Flag | Module | Implementation | Why |
|------|--------|----------------|-----|
| `memory` | `inventory::memory` | `Mutex<Vec<Item>>` | Tests, demos, ephemeral processes. No external service needed. |
| `db` | `inventory::db` | SeaORM/SQLite | Production storage. |

Both implement the same `ItemRepo` trait that `lib.rs` declares. The
wasm browser tests don't care which backend the server is running:
they call `ItemRepoClient::create / get / list / delete` and the
contract holds.

## The flags

```toml
[features]
default = []                                  # the contract, wasm-clean
vox = ["dep:vox", "architect/vox"]
memory = []
db = ["architect/server-seaorm", "dep:sea-orm"]
```

Both backends can be enabled in the same compile (e.g. tests). They
live in different modules, so their names never collide:

```rust
#[cfg(feature = "memory")]
pub mod memory;   // inventory::memory::Memory

#[cfg(feature = "db")]
pub mod db;       // inventory::db::{ItemRepoStorage, Migrator}
```
<!-- doc-check: on -->

There is no facade crate: the feature crate is the facade, and an app
depends on it with the flags it needs.

## In-tree vs external implementations

Everything on this page describes the **in-tree** pattern: backends
live inside the feature's crate, behind flags. Third parties can
implement the same trait surface from their own crates without forking
the monorepo; see [Extensibility](@/architecture/extensibility.md) for
that path. The contract (the crate with no features) is what makes
both paths interchangeable from the running app's perspective.

## Pattern for the DAW analogue

Each DAW backend (Reaper, Ableton, Pro Tools) is a flag of the feature
crate (a sketch; this crate is not in this repo):

<!-- doc-check: off (illustrative names, not paths in this repo) -->
```
features/timeline/
  Cargo.toml             package `timeline`: flags reaper / ableton / protools / memory
  src/
    lib.rs               contract: `TimelineRepo`, `ClipRepo`, ...
    reaper.rs            #[cfg(feature = "reaper")]   clip + track storage
    ableton.rs           #[cfg(feature = "ableton")]  Live Object Model adapter
    protools.rs          #[cfg(feature = "protools")] control surface adapter
    memory.rs            #[cfg(feature = "memory")]   in-memory for tests
  timeline-reaper/       only if REAPER needs its own cdylib (layout: "When a second crate is allowed")
```

A REAPER extension binary pulls `timeline` with `reaper`, `mixing` with
`reaper`, etc. An Ableton host does the same with `ableton`. The same
UI shell composes both because both expose the same `timeline` contract
types and traits.
<!-- doc-check: on -->

## Large payloads

A vox return value serializes whole. For megabyte-scale results — an FX
state chunk, a rendered audio block — that's megabytes of encoding per
call even when the client only wants a slice. `architect::handle` keeps
the allocation in the backend instead:

| Crate side | Type | Role |
|------------|------|------|
| contract (wire) | `RawHandle` | Opaque id, facet-serializable (a bare UUID). What service signatures carry. |
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
