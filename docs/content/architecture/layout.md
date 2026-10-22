+++
title = "Monorepo layout"
description = "apps/<app>/<role> + features/<feature>: one crate per feature, roles as cargo features."
weight = 30
+++

> **Decision (ADR, accepted 2026-10-22 by the Captain, STA-257).** One
> layout for every FastTrackStudio Rust repo: architect, Task and studio
> (Session, Signal). **One crate per feature, with cargo features.** A
> feature's contract, backends, UI and engine are cargo features of its
> one crate, not `-proto` / `-ui` / `-<backend>` sibling crates. This
> replaces the sibling-crate shape this page used to prescribe and the
> Session "facade crate" pattern. Existing sibling crates migrate one
> feature at a time; new features start in the new shape.

## The shape

```
apps/
  <app>/                       runtime apps. one app = one binary suite.
    server/                    package: <app>-server
    db/                        package: <app>-db   (migration CLI)
    ui/                        package: <app>-ui   (shell)
    web/                       package: <app>-web
    desktop/                   package: <app>-desktop
    tests/
      e2e/                     package: <app>-tests-e2e

crates/                        shared libraries that are not a feature.
                               architect's own `crates/architect` facade and
                               its proc-macro crates (`architect-derive`,
                               `architect-rpc-derive`, ...) live here.

features/
  <feature>/                   one feature = one capability = ONE crate.
    Cargo.toml                 package: <feature>
    src/
      lib.rs                   the contract: entities, wire types, service
                               traits, error enum, action declarations.
                               Always compiled; wasm-clean.
      memory.rs                #[cfg(feature = "memory")]  in-memory backend
      db.rs                    #[cfg(feature = "db")]      SeaORM backend
      ui/                      #[cfg(feature = "ui")]      Dioxus components
    spec/                      tracey-tracked rules for this feature.
    tests/                     cargo test against the memory backend.
```

Studio already uses this shape: `studio:features/dsp/eq` (`native`, `ui`,
`profiles`, ...), `studio:features/patchbay` (`server`, `ui`, `vox`),
`studio:features/session` (one crate; no `session-proto` / `session-ui`).

## Roles are cargo features

| Feature | Layer | Contains | May pull in |
|---|---|---|---|
| none (default) | L0 contract | entities, wire types, service traits, error enum, action ids | `facet`, `uuid`, `chrono`, `thiserror`, `architect` without server flags |
| `vox` | L0 | RPC client/server stubs for the traits | `architect/vox` |
| `memory`, `db`, `<backend>` | L1 engine + L2 storage | one backend per flag | `architect/server-seaorm`, tokio, Loro, file IO |
| `server` | L3 glue the feature needs | router wiring for the backend | `architect/server` |
| `ui` | L4 | Dioxus components for this feature | `architect/client`, `dioxus` |
| `native`, `dsp`, ... | as named | platform or engine code | as needed |

Rules:

1. **The default feature set is the contract, and it is wasm-clean.** No
   `tokio` runtime, `sea-orm`, `axum`, `iroh`, `dioxus` or `reaper-*`
   without a flag. CI checks each feature crate with
   `--no-default-features --target wasm32-unknown-unknown`, and with
   `ui` / `vox` where the crate has them.
2. **Flags compose one way.** `ui` never implies a backend; a backend
   never implies `ui`. Apps choose: a server binary enables `db` +
   `server`; a wasm shell enables `ui` + `vox`.
3. **`default-features = false` goes on the `[workspace.dependencies]`
   entry**, because cargo cannot set it on a workspace-inherited dep.
   Consumers add flags with `x = { workspace = true, features = [...] }`.
4. **Apps depend on the feature crate.** There is no separate facade
   crate; the feature crate is the facade.
5. **Every crate has `[lints] workspace = true`.**

## When a second crate is allowed

Only where cargo or the build forces it. It is named `<feature>-<role>`
and lives inside the feature directory:

- a **proc-macro** (`features/crdt/crdt-derive`);
- a separate **binary or process** (a daemon, a REAPER/SHM extension, a
  FUSE or FileProvider host, a CLI);
- a **plugin / cdylib** target;
- a **different target** that cannot share the crate graph (a wasm-only
  browser test crate kept in workspace `exclude`).

"The UI is big" and "it keeps the contract wasm-clean" are not reasons:
the `ui` flag and rule 1 handle both. "It compiles faster" needs a
measured number in the PR.

## Boundary checks read features, not crate names

`cargo xtask boundaries` (STA-268) maps each layer above to a crate
**or a cargo feature of a crate** and checks the resolved graph from
`cargo metadata` per feature set: the default set and `ui` must not
reach L1-L3 dependencies, and nothing outside `apps/*` enables a
transport host. A sibling crate that has not been folded in yet gets a
layer entry in `boundaries.toml` until it is.

## Migration

- **New features:** one crate from day one. `cargo xtask feature new
  <name>` scaffolds the one-crate shape (contract in `lib.rs`, a
  `memory` flag, a passing test).
- **Existing sibling crates** (architect `auth-*`, `launcher-*`,
  `crdt-seaorm`, `permissions/proto`; Task `*-proto` / `*-ui` /
  `*-<backend>`; any studio leftovers) fold in one feature per PR: move
  the proto into `lib.rs`, put the backend and UI behind flags, update
  dependents, delete the old crate. A fold-in PR changes no behaviour.
  Proc-macro and binary crates stay separate.
- Repo `CLAUDE.md` files point here instead of restating a layout.

## Naming rules

- **Package name = directory name.** `features/<f>/` is the
  `<f>` crate; an allowed second crate at
  `features/<f>/<f>-reaper/` is `<f>-reaper`.
- **Cargo names use only `[a-z0-9_-]`.** The `<>` notation here is
  shorthand for "fill in the blank".
- **App names and feature names live in different namespaces.** App
  crates are prefixed (`<app>-server`); feature crates are not.

## Workspace `members` vs `exclude`

The root `Cargo.toml`'s workspace `members` list everything that
compiles for the host target. Crates with target-cfg deps (wasm-only
test crates, Dioxus apps that pull `dioxus-web`/`dioxus-desktop`) sit
in `exclude` and are invoked with `cargo` directly from inside their
own directory.

## Superseded

Before 2026-10-22 this page prescribed `features/<f>/<f>-proto`,
`<f>-<backend>`, `<f>-ui` and a `<f>` facade as sibling crates (and a
separate CLI's `feature new` command that never existed), and Session's
`CLAUDE.md` prescribed a facade crate over internal `session-proto`.
Both are replaced by the one-crate shape above, which studio's
`CLAUDE.md` ("Crate rules") already follows.
