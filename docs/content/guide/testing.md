+++
title = "Testing"
description = "One scenario on every wire, in-process vox, mocks and fakes, sandboxed boots, property tests, and what CI runs."
weight = 60
+++

## What you'll build

Tests that prove a feature behaves the same however it is reached:
called directly, over vox in-process, over a WebSocket, over iroh and
over HTTP. Then mocks, fake data, a sandboxed boot for whole-app tests,
property tests for convergence-style rules, and the local run that
matches CI before you open a PR.

## 1. One scenario, every wire

Write the behaviour once as a generic `async fn` over the trait, then
call it with each client. Greeter's version
(`examples/greeter/tests/round_trip.rs`):

```rust
async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
    assert_eq!(
        client.greet("nobody".into()).await.unwrap_err(),
        GreetError::Unknown("nobody".into())
    );
    assert_eq!(client.count().await.unwrap(), 1);
}

#[tokio::test]
async fn in_process() {
    scenario(&Memory::default()).await;
}

#[tokio::test]
async fn over_vox() {
    let scope = Scope::new();
    let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
    let client: GreeterClient = local.establish().await.unwrap();
    scenario(&client).await;
    scope.close().await;
}

#[tokio::test]
async fn over_http() {
    let backend = Memory::default();
    let base = serve_http(backend.clone().into_http_router()).await;
    scenario(&GreeterHttpClient::at(base)).await;
}
```

`serve_http` binds `127.0.0.1:0` and spawns `architect::http::axum::serve`
on it; the port is the OS's choice, so tests run in parallel. The error
assertion matters as much as the success: a typed error that survives
every wire is what the generated `HttpError` and transport variant are
for.

For a service set that must hold on every transport, copy
`apps/auth-server/tests/surfaces.rs`. Each scenario is a generic
`async fn` over the service traits, and `on_every_transport` runs it
over four stacks, each a fresh in-memory database:

| Stack | How it is built |
|---|---|
| HTTP | the generated router on a real socket, `…HttpClient` clients |
| vox in-process | `LocalServer::serve(server::vox_router(auth), scope)` |
| vox over WebSocket | the full `EngineHost` on a socket, dialled through its own `Pool::default()` |
| vox over iroh | the router on an iroh endpoint, dialled over loopback with relays off |

A scenario that passes on one stack and fails on another is a bug in a
generated face, not in the engine. A service with `actions` adds one
more wire: the Action List and MCP, as in `examples/greeter/tests/mcp.rs`
`greet_over_mcp_is_greet_over_http`.

Streams get the same treatment. Subscribe, publish, assert the frame
(`examples/greeter/tests/round_trip.rs` `streams_over_http`):

```rust
let mut greeted = client.greeted().await.unwrap();
tokio::time::sleep(std::time::Duration::from_millis(30)).await;
Greeter::greet(&client, "ada".into()).await.unwrap();
assert_eq!(greeted.next().await.unwrap().unwrap(), "hello ada");
```

The sleep covers the attach gap; a source with an `<name>_intro` frame
lets the test wait for the intro instead.

## 2. In-process vox is the default

`LocalServer` runs the real `LayerRouter` over a vox memory link: every
byte is facet-encoded and dispatched, middleware and gates included,
with no socket. It is fast enough for the default test run, so most
wire tests belong there. Pick the lowest rung that covers the question:

1. Direct trait calls on the backend: behaviour, validation, ordering.
2. `LocalServer`: the wire, the gate, streams.
3. A real server on a port (`apps/<app>/tests/`): the axum upgrade
   path, several connections, the whole host.
4. A browser: the wasm build talking to a real server (Task carries
   these for its features).

## 3. Mocks and fakes

Replace one service in a bundle by merging a mock over it. The last
merge wins, on vox and on HTTP:

```rust
// examples/greeter/tests/round_trip.rs, a_mock_replaces_the_real_greeter
let bundle = Memory::layers().merge(greeter_layer(Mock));
let local = LocalServer::serve(bundle.provide(Memory::default()), scope.clone());
let client: GreeterClient = local.establish().await.unwrap();
assert_eq!(client.greet("cody".into()).await.unwrap(), "mocked");
```

`crates/architect/tests/layer_override.rs` covers the rule.

For data, turn on your crate's `fake` feature (forwarding
`architect/fake`): every emitted payload derives `fake::Dummy`, and the
entity gets `seed_fake_<entity>(repo, count)`, which creates fake rows
through any repo (`count` is a number or a range).

Time-based code takes a `Clock` from `architect::platform` and is
tested with its `TestClock`, so a retry schedule or a debounce runs
without sleeping ([scheduling](@/architecture/scheduling.md)).

## 4. Whole-app boots in a sandbox

A test that boots an app as it ships boots it over the developer's real
home directory unless told otherwise.
`architect::testing::sandbox_server(seed)` (feature `testing`, a
dev-dependency) points `HOME`, the `XDG_*` dirs and the Windows
profile dirs into a tempdir under one process-wide lock, runs your seed,
and serves the result on `127.0.0.1:0`:

```rust
let server = architect::testing::sandbox_server(|sb| async move {
    plant_fixtures(&sb.data())?;
    let backend = Backend::open(&sb.data()).await?;
    Ok(backend.into_router())
})
.await?;
let client = connect(&server.url()).await?;
```

`testing::sandbox()` is the builder (`.clear`, `.data_env`, `.env`,
`.boot`, `.over` for a restart over the same directory).
`architect::testing::alloc::Counting` (feature `testing-alloc`) is a
counting allocator a test binary installs to assert a heap budget.
`crates/architect/tests/testing_sandbox.rs` is the reference.

## 5. Property tests

A rule that must hold for any input (convergence, ordering, a parser)
is a `proptest` test. The workspace pins `proptest`; add it as a
dev-dependency with `proptest.workspace = true`.
`features/crdt/crdt/tests/peer.rs` `any_edit_order_converges` drives
random edit and exchange orders across three nodes and asserts they
end equal. Keep the case count small (`ProptestConfig::with_cases(24)`
there) when each case runs a runtime, and label the rule it verifies
(`t[verify crdt.peer.convergence]`) so tracey ties it to the spec.

Every bug fix lands with a test that failed before it.

## 6. What CI runs

A PR runs `.github/workflows/pr.yml`: one job, `check`, about a minute
on a hosted runner.

- `cargo fmt --all -- --check`
- a lockfile check: `cargo metadata --format-version 1` must leave
  `Cargo.lock` unchanged.

After a merge, every push to `main` runs `.github/workflows/checks.yml`,
with every warning an error (`CARGO_BUILD_WARNINGS=deny`) and the
pinned toolchain:

| Job | Runs |
|---|---|
| `lint` | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo run -q -p xtask -- boundaries`, `cargo run -q -p xtask -- docs` |
| `test` | `cargo test --workspace` |
| `wasm` | `cargo check --target wasm32-unknown-unknown` over the framework crates (`architect`, atom, form, permissions, auth proto and client, crdt, input) and a few with features |
| `tags` | `cargo run -p xtask -- tags --verbose` on the full history |
| `check` | passes only when `lint`, `test` and `wasm` all succeeded |

Because the PR runs only the fast checks, run the full set locally
before opening one, with `CARGO_BUILD_WARNINGS=deny`: fmt, clippy, the
workspace tests, the wasm `cargo check` lines copied from
`checks.yml`, `boundaries` and `docs`. Whoever merges watches `main`
and reverts or fixes forward when it goes red.

`cargo run -q -p xtask -- docs` (also `just docs-check`) fails on any
repo path, page link, `just` recipe, xtask command or `-p` package the
docs and skills name that does not exist (`xtask/src/docs.rs`). Cite
code as a path plus a symbol, never a line number.

Not in CI: `just lint` (clippy with `--all-features`),
`just ui-css-check`, `just auth-e2e` and the UI snapshot tests.

## Where to read more

- [Testing strata](@/architecture/testing.md): the layers, the bridge,
  the sandbox and the allocator.
- [The in-process transport](@/architecture/local.md).
- [The HTTP face](@/architecture/http.md): "Testing every face at
  once".
- [Specs](@/architecture/specs.md): rules and their `verify` labels.
- [Scheduling](@/architecture/scheduling.md): `TestClock`.
