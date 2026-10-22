+++
title = "Testing strata"
description = "Native unit, native integration, browser e2e — which lives where."
weight = 40
+++

architect has three test layers that exercise the same contract from
progressively heavier setups.

## Layer 1: native, in-process

`features/<feature>/tests/native/`

Drives the auto-generated `<T>Repo` trait against an in-memory backend.
No socket, no server, no async runtime beyond `tokio::test`. Sub-second
test runs. Use this for everything that's about the *contract* — sort
ordering, validation errors, payload-shape exclusions, etc.

```rust
#[tokio::test]
async fn list_sorted_by_name_ascending() {
    let r = ExampleRepoMemory::new();
    for n in ["charlie", "alpha", "bravo"] {
        r.create(ExampleCreate { name: n.into(), description: String::new() })
            .await.unwrap();
    }
    let page = r.list(
        Page { index: 0, size: 100 },
        Some(Sort { field: "name".into(), order: SortOrder::Asc }),
        None,
    ).await.unwrap();
    let names: Vec<_> = page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
}
```

Run with `cargo test -p example-greeter` (the reference feature's
`tests/round_trip.rs`).

### One scenario, every wire

The generated clients implement the service trait, so write the
scenario once as `async fn scenario(s: &impl Greeter)` and call it with
the backend itself, a `LocalServer` client, a WebSocket client, an iroh
client and the HTTP client. `examples/greeter/tests/round_trip.rs` and
`apps/auth-server/tests/surfaces.rs` are the pattern. Mocks: merge one
over the bundle — `Memory::layers().merge(greeter::layer(Mock))` — and
it replaces the real service on vox **and** HTTP.

### Testing a backend without any server

Layer 1 has three rungs of its own, in increasing wire-fidelity. Pick
the lowest one that exercises what you're testing.

**Direct trait calls.** An `#[architect::rpc]` trait stays a plain
trait — sync methods stay sync. No router, no dispatcher, no future,
often no `tokio::test`:

```rust
let backend = AllSyncBackend::default();
backend.write(7, "direct".into()).unwrap();
assert_eq!(backend.read(7).as_deref(), Some("direct"));
```

**The `__<T>Bridge`.** When the question is "does the sync→async
marshaling work" — borrowed-arg rewrites (`&str` → `String`),
dispatcher round-trips — construct the hidden bridge the macro emits
and call the async mirror directly. It's `#[doc(hidden)]` and not for
production use, but it's exactly how `architect-rpc-derive`'s own smoke
tests (`crates/architect-rpc-derive/tests/smoke.rs`) exercise the
bridge body without vox in the build:

```rust,ignore
let bridge = __AllSyncBridge::new(AllSyncBackend::default(), CurrentThreadDispatcher);
futures_lite::future::block_on(async {
    AllSyncRpc::write(&bridge, 1, "hello".into()).await.unwrap();
    assert_eq!(AllSyncRpc::read(&bridge, 1).await.as_deref(), Some("hello"));
});
```

**`LocalServer` — the full wire, no socket.** Serve the real
`LayerRouter` over a vox in-memory link and drive the *generated typed
clients*: every byte goes through facet encoding, vox dispatch, and
middleware, at unit-test speed. This is how the e2e suite's
`local_transport_round_trip` covers the whole service surface, and how
the auth feature pins its token wire-format
(client `TokenStoreMiddleware` ↔ server `AuthServerMiddleware`)
without spawning anything:

```rust,ignore
let scope = Scope::new();
let local = LocalServer::serve(backend.into_router(), scope.clone());
let client: ExampleRepoClient = local.establish().await?;
// … same asserts as over a WebSocket …
scope.close().await;
```

See [the in-process transport](@/architecture/local.md). Reach for
layer 2 only when the thing under test *is* the network stack (axum
upgrade path, TCP behavior, real concurrency across connections).

## Layer 2: native, real server

`apps/<app>/tests/e2e/`

Spawns the full axum + vox stack on an OS-assigned port, then drives
it from a vox-core/vox-websocket client over native TCP. Validates the
whole transport-and-dispatcher pipeline without involving a browser.

This is where you put tests that span features (auth + multi-row
updates + permission checks together).

Run with `cargo test -p auth-server` (`apps/auth-server/tests/`: real
clients against a real server).

## Layer 3: real browser, real server

`features/<feature>/tests/web/`

`wasm-bindgen-test` crate. Spawned in headless Firefox via
`wasm-bindgen-test-runner`. Loads the wasm module, opens a real
WebSocket against a running `app-server`, and exercises
`<T>RepoClient::create / list / get / delete`.

This is the test that proves "wasm-clean" is more than aspirational —
every byte goes through facet encoding on a real transport, into a real
database, and back.

architect itself has no browser suite since the example app left the
tree; `just auth-e2e` runs the identity server's end-to-end flow, and
Task carries the wasm tests of its features. Both run the same three browser tests.

## Booting an app in a sandbox (`architect::testing`)

A test that boots an app as-is boots it over whatever the machine's
environment says: on a developer's laptop that is the real home, the
real config dir and the real data. Task's e2e suite once booted every
real org that way and wrote markers into real vaults.
`architect::testing::sandbox_server(seed)` (feature `testing`, a
dev-dependency) closes that door:

```rust,ignore
let server = architect::testing::sandbox_server(|sb| async move {
    plant_fixtures(&sb.data())?;              // the seed: write the world
    let backend = Backend::open(&sb.data()).await?;
    Ok(backend.into_router())                 // a LayerRouter, EngineHost or axum Router
})
.await?;
let client = connect(&server.url()).await?;   // ws://127.0.0.1:<port>/vox
```

Under one process-wide env lock (`testing::env_lock`) it points `HOME`,
`USERPROFILE`, the `XDG_*` dirs, `APPDATA` and `LOCALAPPDATA` inside a
fresh tempdir, runs `seed`, then serves the result on `127.0.0.1:0`.
`testing::sandbox()` is the builder: `.clear("APP_VAR")` removes a
variable a developer's shell may set, `.data_env("APP_DATA_ROOT")` names
the sandbox's data dir to the app, `.env(k, v)` sets anything else,
`.boot(seed)` builds without serving, and `.over(&sandbox)` boots again
over a previous boot's directory (the restart shape). The variables stay
set after the boot, which is sound one test per process (nextest).
`crates/architect/tests/testing_sandbox.rs` sets `HOME` to a poisoned
directory first and asserts the boot never touches it.

### Holding a memory budget

`architect::testing::alloc::Counting` (feature `testing-alloc`, std only)
is a `GlobalAlloc` over `System` that counts live heap bytes and their
peak, so a test can assert a budget in bytes the program asked for
instead of RSS. architect never installs it; the test binary does:

```rust,ignore
#[global_allocator]
static ALLOC: architect::testing::alloc::Counting = architect::testing::alloc::Counting;

alloc::reset_peak();
let before = alloc::live();
run_the_thing().await;
assert!(alloc::peak() - before < 64 << 20, "{}", alloc::mib(alloc::peak() - before));
```

The counters are process-wide: guard a budget test with
`alloc::counters_are_this_tests()`, which is true under nextest and
skips (with a line saying why) under plain `cargo test`.
`crates/architect/tests/testing_alloc_uninstalled.rs` pins that linking
the feature installs nothing.

Run them with
`cargo test -p architect --features testing --test testing_sandbox --test testing_alloc --test testing_alloc_uninstalled`.

## What goes where

| Test concern | Layer |
|--------------|-------|
| Trait method behavior, edge cases | 1 — native, in-process |
| Validation errors, sort ordering | 1 |
| Wire round-trips, middleware, streams | 1 — `LocalServer` |
| rpc sync→async marshaling | 1 — `__<T>Bridge` |
| Cross-feature integration | 2 — native, real server |
| App boot that must not see the developer's home | 2 — `testing::sandbox_server` |
| Memory budget in heap bytes | 2 — `testing::alloc::Counting` |
| Auth + permission flows | 2 |
| Wire format compatibility | 3 — browser |
| Wasm32 build correctness | 3 |
| End-user latency / UX | dx serve + manual |
