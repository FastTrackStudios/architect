+++
title = "Monorepo layout"
description = "apps/<app>/<role> + features/<feature>/<feature>-<role>."
weight = 30
+++

The template's directory shape is built so a project can host multiple
apps and an unlimited number of features without naming collisions or
ambiguity about what each crate is.

> In **this** repo (the framework itself), the reference project lives
> under `examples/app/` so the framework crates and the demo that
> consumes them stay clearly separated. A real consumer applies the shape
> below at its own repo root — `apps/` and `features/` directly.

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

crates/                        publishable libraries. architect's own
                                user-facing facade crate (`crates/architect`)
                                lives here too — it isn't a proc-macro
                                itself, just a regular lib that re-exports
                                the derives + runtime traits.

features/
  <feature>/                   one feature = one capability.
    <feature>/                 facade — selects backend via cargo features.
    <feature>-proto/           wire contract.
    <feature>-<backend>/       one impl per backend (db, memory, ...).
    <feature>-ui/              feature-scoped Dioxus components.
    spec/                      tracey-tracked rules for this feature.
    tests/
      native/                  cargo test against in-memory backend.
      web/                     wasm-bindgen browser tests against a server.
```

There is no top-level `macros/` or `libs/` directory. Proc-macro crates
(`architect-derive`, `architect-rpc-derive`, `architect-action-derive`)
live under `features/macros/` — they're cross-cutting rather than
scoped to one feature, but they're still consumable capabilities other
crates pull in, so they get a `features/` home rather than an
exception. A proc-macro scoped to a *single* feature nests inside that
feature instead — `crdt-derive` lives at `features/crdt/crdt-derive`,
next to `crdt` and `crdt-seaorm`.

The architect repo itself follows the same convention for its **built-in
features**: `features/atom` and `features/form` (the Dioxus client-state
primitives, re-exported on the `architect` crate behind the `atom` /
`form` features), `features/crdt/` (the local-first CRDT layer —
`crdt` + `crdt-seaorm` + `crdt-derive`), and `features/auth/` — the full
auth feature (`auth-proto` / `auth` / `auth-db` / the `architect-auth`
facade + `spec/` + `tests/`), folded in from the former architect-auth
repo. Auth is built *on* architect (its proto uses the Entity derive),
so it's consumed as the `architect-auth` crate rather than re-exported —
re-exporting would be a dependency cycle.

## Naming rules

- **Path prefix matches package name prefix.** A crate at
  `apps/<app>/<role>/` is named `<app>-<role>`; a crate at
  `features/<feature>/<feature>-<role>/` is named `<feature>-<role>`.
- **Cargo names use only `[a-z0-9_-]`.** The `<>` notation in this
  template is documentation shorthand for "fill in the blank" — Cargo
  itself rejects literal angle brackets in names.
- **App names and feature names live in different namespaces.** It's
  legal for an app to share a name with a feature, but the prefix split
  (`<app>-` vs `<feature>-`) keeps packages disambiguated.

## Why duplicate the prefix in the path *and* the name

Glanceability. Reading `features/timeline/timeline-reaper/Cargo.toml`
tells you (a) this is the `timeline` feature and (b) the package is
named `timeline-reaper` without opening the file. The slight redundancy
pays back in PR diffs, `cargo tree` output, and crate-graph thinking.

## Scaffolding a new feature

The mechanical churn of dropping in a new feature is automated:

```sh
just scaffold-feature mixing
# or directly:
cargo run -p architect-cli -- feature new mixing
```

That command writes the canonical layout into `features/mixing/`:

```
features/mixing/
  mixing-proto/        Cargo.toml + src/lib.rs with a placeholder Mixing entity
  mixing-memory/       in-tree HashMap impl
  mixing/              facade (vox + server-* + backend-memory features)
  spec/mixing.md       tracey rule stub
  tests/native/        sample round-trip test
```

It also updates the workspace `Cargo.toml` (members, default-members,
workspace.dependencies) and appends a spec block to
`.config/tracey/config.styx`. Rename the placeholder `Mixing` entity
to the real one for that feature, flesh out the memory backend, and
you're ready to consume it from an app via
`features = ["full"]` on the facade.

## Adding a second app

Same shape, different prefix:

```
apps/daw-reaper/
  server/                      package: daw-reaper-server
  ui/                          package: daw-reaper-ui
  ...
```

The `daw-reaper-ui` shell composes whichever `<feature>-ui` crates the
Reaper build needs; `daw-ableton-ui` composes the same feature-ui
crates (or a different set, if Ableton exposes a different surface).

## Workspace `members` vs `exclude`

The root `Cargo.toml`'s workspace `members` list everything that
compiles for the host target. Crates with target-cfg deps (wasm-only
test crates, Dioxus apps that pull `dioxus-web`/`dioxus-desktop`) sit
in `exclude` and are invoked with `cargo` directly from inside their
own directory. The `just check` recipe wraps this so a single command
verifies both.
