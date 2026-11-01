+++
title = "The shape of an architect app"
description = "Workspace layout, layers and boundaries, cargo features, the dev shell, the toolchain, the build cache and the feature scaffold."
weight = 10
+++

## What you'll set up

A Cargo workspace laid out the way every FastTrackStudio Rust repo is
(architect itself, Task, studio): one crate per feature under
`features/`, binaries under `apps/`, the layers of each feature as cargo
features of its one crate, and a check that keeps those layers apart.
At the end of this chapter you have a dev shell that builds the
workspace, and a first feature crate with a passing test.

## 1. The workspace layout

<!-- doc-check: off (a layout sketch: names are placeholders) -->
```text
apps/
  <app>/
    server/          package <app>-server   the binary that serves
    db/              package <app>-db       migration CLI, if any
    ui/              package <app>-ui       the shell's components
    web/             package <app>-web      wasm entry point
    desktop/         package <app>-desktop  native entry point
crates/              shared libraries that are not a feature
features/
  <feature>/         package <feature>      ONE crate per capability
    src/lib.rs       the contract: entities, wire types, services, errors
    src/memory.rs    #[cfg(feature = "memory")]  a backend
    src/db.rs        #[cfg(feature = "db")]      another backend
    src/ui/          #[cfg(feature = "ui")]      Dioxus components
    spec/            the feature's rules, for tracey
    tests/           scenarios against the memory backend
```
<!-- doc-check: on -->

In this repo that is `crates/architect` (the facade crate and its
proc-macro crates beside it), `features/atom`, `features/crdt`,
`features/auth` and the rest, `apps/auth-server` (the identity server, a
full app built this way) and `examples/greeter`, the reference feature
with a server on top.

A capability is a feature crate. Its contract is the crate's default
feature set; its backends, its UI and its engine are cargo features of
the same crate, not `-proto` / `-ui` / `-memory` sibling crates. A
second crate is allowed only where cargo forces one: a proc-macro, a
separate binary or process, a cdylib, or a crate for a target that
cannot share the graph. The full rule and its history are in
[the layout ADR](@/architecture/layout.md).

Package names are directory names (`features/crdt/crdt` is `crdt`), app
crates are prefixed with the app's name (`auth-server`), and feature
crates are not.

## 2. The layers, enforced

Each crate (or cargo feature of a crate) sits at one layer, and
dependencies point one way:

| Layer | What | May not reach |
|---|---|---|
| L0 contract | entities, wire types, service traits, errors (default features, `vox`) | sea-orm, axum, iroh, a tokio runtime, dioxus, a transport host |
| L1 engine, L2 storage | backends (`memory`, `db`, …) | — |
| L3 host / transport | `EngineHost`, axum, iroh serving | only under `apps/` |
| L4 UI | Dioxus components (`ui`) | engine and storage packages, transport hosts |

`boundaries.toml` at the repo root maps each package and feature set to
its layer. One `[[row]]` per role:

```toml
[[row]]
package = "architect"
features = ["vox"]
layer = "L0"
```

`cargo run -q -p xtask -- boundaries` resolves every row with
`cargo tree` (L0 rows for `wasm32-unknown-unknown`) and fails when an L0
row reaches a `[forbidden]` package, when a crate outside `apps/`
enables a transport host by default, when a crate lacks
`[lints] workspace = true`, or when `Cargo.lock` holds two sources of
an architect crate. Known violations are counted in `[baseline]`; a
count may only go down, and the PR that lowers it lowers the baseline
too. The checker is `xtask/src/boundaries.rs` `run_cli`.

## 3. Cargo features

The contract compiles for the browser, so the default feature set must
stay wasm-clean: no tokio runtime, sea-orm, axum, iroh, dioxus or
`reaper-*` without a flag. The usual flags of a feature crate:

| Flag | Turns on | Typical contents |
|---|---|---|
| none (default) | the contract | `#[architect::entity]`, `#[architect::service]`, `#[architect::error]`, plain wire types |
| `vox` | `architect/vox` | the vox clients, dispatchers and `Layer` tokens the attributes emit |
| `memory`, `db`, … | one backend each | an in-memory store, SeaORM storage, a CRDT doc |
| `server` | `architect/server` | the SeaORM storage and migration the entity derive emits |
| `ui` | `architect/client`, `dioxus` | the feature's components |
| `native` | platform code | desktop-only pieces |

`examples/greeter/Cargo.toml` is the smallest real set:

```toml
[features]
default = ["vox"]
vox = ["architect/vox"]
server = ["architect/server", "dep:sea-orm"]
```

Two rules make features compose:

- **The names belong to your crate.** The derives emit
  `#[cfg(feature = "vox")]`, `"server"`, `"atom"`, `"form"`, `"fake"`
  and `"crdt"` into the crate that uses them. A crate that calls its
  storage flag `server-seaorm` instead of `server` gets no storage, and
  no error. Forward `server` to `architect/server` as greeter does.
- **`default-features = false` goes on the `[workspace.dependencies]`
  entry**, since cargo cannot turn defaults off on a workspace-inherited
  dependency. Consumers add flags at the use site:
  `x = { workspace = true, features = ["vox"] }`.

Apps choose the flags: a server binary turns on a backend and
`server`; a wasm shell turns on `ui` and `vox`. On `architect` itself,
`server` and `client` are the two profiles a binary usually wants
(`server` = vox + SeaORM bridge + axum WebSocket + tokio dispatcher +
HTTP face; `client` = vox + optimistic store + forms), and the à la
carte flags (`host`, `http-client`, `local`, `iroh`, `connect`,
`telemetry`, …) are listed in [feature flags](@/architecture/features.md).
Greeter, a single binary that is also its own test client, uses
`features = ["host", "http-client", "local", "action-list"]`.

The workspace's own manifest declares every intra-repo crate once as a
path dependency and pins external crates (facet, vox, dioxus) for the
whole tree, so every crate builds against one vox revision. A server
and its clients on two vox revisions fail the handshake at the first
call ([idioms §5](@/architecture/idioms.md)).

## 4. The dev shell and the toolchain

`flake.nix` carries the native libraries the workspace links against.
The Dioxus desktop crates need the GTK/WebKit stack even in unit tests,
so without the shell `cargo test --workspace` fails at link time.

```sh
nix develop          # the libraries, over the rust you already have
nix develop .#ci     # the same plus nixpkgs' cargo, rustc, rustfmt, clippy
```

`rust-toolchain.toml` pins the toolchain (rust 1.99.1, with `rustfmt`,
`clippy`, `rust-src` and the `wasm32-unknown-unknown` target); rustup
reads it. The consumers (Task, studio) pin the same version, and the
pins move together. CI reads the same pin through `RUSTUP_TOOLCHAIN` in
`.github/workflows/checks.yml`.

Lints are workspace-wide (`[workspace.lints]` in the root
`Cargo.toml`): clippy's pedantic and nursery groups are denied, and so
are the panic-shaped lints (`unwrap_used`, `expect_used`, indexing,
arithmetic overflow). Shipped code returns errors instead of
panicking; tests opt out at the top of the file, as
`examples/greeter/tests/round_trip.rs` does. Every crate opts in with
`[lints] workspace = true`, and `boundaries` checks that it did.

## 5. The build cache

On the fleet's build hosts, every cargo invocation goes through kache,
a `rustc` wrapper that keys each compile by its inputs and restores a
stored result instead of compiling. The wrapper is set per machine
(`build.rustc-wrapper` in the user's cargo config), not by this repo.
A cache hit needs the same key, so keep builds deterministic:

- No `env!("CARGO_MANIFEST_DIR")` outside tests. It bakes the
  checkout's absolute path into the crate, and every checkout then
  misses. Find files at run time instead.
- No absolute paths, timestamps or git hashes in build-script output.
- Name the target directory `target`.
- Build one configuration: the commands CI runs
  (`cargo clippy --workspace --all-targets`, `cargo test --workspace`,
  the wasm `cargo check` lines in `.github/workflows/checks.yml`), with
  the same `RUSTFLAGS`. A `-p` subset resolves dependency features
  differently, so it compiles every dependency under a new key. Filter
  tests by name after `--` instead.

## 6. Scaffold a feature

```sh
cargo run -p xtask -- feature new inventory
```

(The docs write this `cargo xtask feature new`; architect defines no
cargo alias, so in this repo it is `cargo run -p xtask --`.) The
scaffold, `xtask/src/scaffold.rs` `feature_new`, writes one crate:

<!-- doc-check: off (the crate the command creates does not exist yet) -->
| File | What is in it |
|---|---|
| `features/inventory/Cargo.toml` | flags `vox`, `memory`, `ui`; a dev-dependency on itself with `memory` + `vox` so tests run the backend |
| `features/inventory/src/lib.rs` | an entity (`Inventory`, with `repo, events`), an error (`InventoryError`), a service (`InventoryService`) |
| `features/inventory/src/memory.rs` | the in-memory backend and its `Services` bundle |
| `features/inventory/src/ui.rs` | an empty module behind `ui` |
| `features/inventory/spec/service.md` | the service's rules, for tracey |
| `features/inventory/tests/memory.rs` | the scenario, in process and over vox; passes as written |
<!-- doc-check: on -->

It also adds the crate to the workspace `members`, adds
`inventory = { path = "features/<name>", default-features = false }`
(with the name filled in) to `[workspace.dependencies]`, writes the crate's `boundaries.toml`
rows (L0 default, L0 `vox`, L1 `memory`, L4 `ui`) and runs rustfmt. It
refuses a name that exists. `xtask/tests/feature_new.rs` builds and
tests a scaffolded crate.

Rename the placeholder entity, write the contract (next chapter), then
the backend. When the feature needs a server, copy
`examples/greeter/src/main.rs`.

## Where to read more

- [Monorepo layout](@/architecture/layout.md): the one-crate ADR, when
  a second crate is allowed, naming, `members` vs `exclude`.
- [Feature flags](@/architecture/features.md): every architect flag and
  what it implies.
- [Multi-backend features](@/architecture/backends.md): several
  backends as flags of one crate.
- [Idioms and enforcement](@/architecture/idioms.md): the conventions
  and the checks behind them.
- [Specs](@/architecture/specs.md): the `spec/` directory and tracey.
- [Getting started](@/getting-started/_index.md): the dev shell and the
  first test run.
