+++
title = "Getting started"
description = "Get the dev shell up and run the e2e tests."
weight = 10
+++

This page assumes a working Nix install with flakes enabled.

## Enter the dev shell

```sh
nix develop
```

That gives you `cargo`, `dx`, `wasm-bindgen-cli`, `geckodriver`, headless
`firefox`, `just`, `sqlite`, `sea-orm-cli`, and `wasm-pack` on PATH. The
shellHook auto-wires the env vars needed for `cargo test --target
wasm32-unknown-unknown`.

## Build everything

```sh
just check
```

That runs `cargo check --workspace` plus a wasm32 check on each
target-cfg-only crate (`examples/app/web`, `examples/app/desktop`,
`examples/app/ui`, `examples/app/features/example/tests/web`).

## Run the test layers

```sh
# Native, in-process — fastest, no server.
cargo test -p example-tests-native

# Native, real server — spawns app-server on 127.0.0.1:0.
cargo test -p app-tests-e2e

# Browser + real server — sqlite backend, headless Firefox.
just test-e2e

# Browser + real server — in-memory backend.
just test-e2e-memory
```

If all three pass on a fresh checkout, your environment is wired up
correctly.

## Run the server interactively

```sh
just server
```

Migrations apply automatically on boot. The server listens on
`0.0.0.0:4040` by default; the vox WebSocket is at `/vox` and a
health endpoint at `/api/health`.

## Next

Read [**Build a feature, end to end**](@/getting-started/walkthrough.md)
— it walks the whole flow (define an entity → pick a backend → serve it →
consume it remote *or* in-process) against the reference example.
