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

Run with `cargo test -p example-tests-native`.

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
tests (`features/macros/architect-rpc-derive/tests/smoke.rs`) exercise the
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

Run with `cargo test -p app-tests-e2e`.

## Layer 3: real browser, real server

`features/<feature>/tests/web/`

`wasm-bindgen-test` crate. Spawned in headless Firefox via
`wasm-bindgen-test-runner`. Loads the wasm module, opens a real
WebSocket against a running `app-server`, and exercises
`<T>RepoClient::create / list / get / delete`.

This is the test that proves "wasm-clean" is more than aspirational —
every byte goes through facet encoding on a real transport, into a real
database, and back.

Run with `just test-e2e` (sqlite-backed server) or `just test-e2e-memory`
(in-memory backend). Both run the same three browser tests.

## What goes where

| Test concern | Layer |
|--------------|-------|
| Trait method behavior, edge cases | 1 — native, in-process |
| Validation errors, sort ordering | 1 |
| Wire round-trips, middleware, streams | 1 — `LocalServer` |
| rpc sync→async marshaling | 1 — `__<T>Bridge` |
| Cross-feature integration | 2 — native, real server |
| Auth + permission flows | 2 |
| Wire format compatibility | 3 — browser |
| Wasm32 build correctness | 3 |
| End-user latency / UX | dx serve + manual |
