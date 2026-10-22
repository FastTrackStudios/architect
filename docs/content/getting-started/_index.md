+++
title = "Getting started"
description = "Get a dev shell up, build the workspace and run the reference feature's tests."
weight = 10
+++

This page assumes a working Nix install with flakes enabled.

## Enter a dev shell

```sh
nix develop          # native libraries only, over the rust you already have
nix develop .#ci     # the same plus nixpkgs' cargo, rustc, rustfmt and clippy
```

The repo's `flake.nix` carries the native libraries the workspace links
against (the Dioxus desktop crates need the GTK/WebKit stack even in unit
tests). `rust-toolchain.toml` pins the toolchain. CI uses Task's dev
shell instead (`nix develop --accept-flake-config github:FastTrackStudios/task#ci`),
which pins the same toolchain and adds the `wasm32-unknown-unknown`
target; see `.github/workflows/checks.yml`.

## Build everything

```sh
just check           # cargo check --workspace --all-targets
```

The framework half must stay wasm-clean; CI checks it with:

```sh
cargo check --target wasm32-unknown-unknown \
  -p architect -p architect-atom -p architect-form \
  -p architect-permissions -p architect-permissions-proto \
  -p auth-proto -p auth-client -p crdt -p architect-input
```

## Run the tests

```sh
# The reference feature: one scenario in process, over vox and over HTTP.
cargo test -p example-greeter

# The facade: layers, HTTP face, iroh loopback, reconnect, RPC spans.
cargo test -p architect

# Everything (what CI runs).
just test
```

If `cargo test -p example-greeter` passes on a fresh checkout, your
environment is wired up correctly.

## Run a server

```sh
GREETER_BIND=127.0.0.1:4040 cargo run -p example-greeter
curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'
```

`EngineHost` serves vox over WebSocket at `/vox`, the HTTP routes and SSE
streams, and `/health`. The identity server has its own recipes
(`just auth-dev`, `just auth-demo`, `just auth-e2e`).

## Next

Read [**Build a feature, end to end**](@/getting-started/walkthrough.md):
it walks the whole flow (define an entity and a service, write a
backend, serve it, consume it remote or in-process) against
`examples/greeter`.

## A new feature in one command

<!-- doc-check: off (the crate the command creates does not exist yet) -->
```sh
cargo xtask feature new inventory
```

writes one crate, `features/inventory/` ([the layout](@/architecture/layout.md)):
the contract in `src/lib.rs` (an entity, a service, its error; the
default feature set, wasm-clean), an in-memory backend behind the
`memory` flag, empty `vox` / `ui` flags, a `spec/` and a scenario test
that already passes in process and over vox. It adds the crate to the
workspace `members` and to `[workspace.dependencies]` (with
`default-features = false`). Then `cargo test -p inventory`.
<!-- doc-check: on -->
`examples/greeter` is the same contract with a server on top: copy it
when you need one.

## Repo tooling

`cargo xtask` has six commands: `feature new`, `boundaries` (the
layer table and ratcheted idioms in `boundaries.toml`), `docs` (every
repo path, page link, `just` recipe, `cargo xtask` command and `-p`
package these docs name must exist), `tags` and `fleet-bump`, plus
`auth-mirror` for the identity server. `just --list` shows the recipes.
