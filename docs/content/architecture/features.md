+++
title = "Feature flags"
description = "Every architect cargo feature, what it implies, and which profile to reach for."
weight = 55
+++

architect is wasm-clean with no default features. Everything optional is
behind a flag, and flags compose in one direction only (an implication
arrow never points back up). Two umbrella features cover the common
profiles; the à la carte flags below them exist for fine-grained builds.

## Profiles (start here)

| Feature  | Enables                                            | For |
|----------|----------------------------------------------------|-----|
| `client` | `vox`, `atom`, `form`                              | UI crates — wasm or desktop shells. Typed RPC clients, optimistic store hooks, validated forms. Wasm-clean. |
| `server` | `vox`, `server-seaorm`, `server-axum`, `dispatch-tokio`, `http` | Native server binaries. RPC, SeaORM storage bridge, axum WebSocket transport, tokio blocking dispatcher, the generated HTTP+JSON face. |
| `full`   | `vox`, `server-seaorm`, `server-axum`, `fake`, `platform`, `schedule`, `rt` | Development / single-binary builds. Deliberately excludes `atom`/`form` (server builds never compile Dioxus). |

A typical app:

<!-- doc-check: off (illustrative names, not paths in this repo) -->
```toml
# apps/web (wasm UI)
architect = { workspace = true, features = ["client"] }

# apps/server (axum binary)
architect = { workspace = true, features = ["server"] }

# features/foo (one crate per feature, see layout.md) — forward vox so
# each consumer decides whether the RPC stack compiles:
[features]
vox = ["dep:vox", "dep:vox-types", "architect/vox"]
```
<!-- doc-check: on -->

`server` is a strict superset of the old `server = ["server-seaorm"]`
back-compat alias — consumers that wrote `features = ["server"]` for
the SeaORM bridge keep working and additionally get the transport +
dispatcher.

## À la carte flags

| Feature | Implies | Pulls | What you get |
|---------|---------|-------|--------------|
| `vox` | — | vox | `#[vox::service]` decoration on emitted repo traits, typed clients/dispatchers, `Layer`/`layers!` composition, `PubSub`. With `vox` off, `#[architect::rpc]` traits stay plain async traits — in-process use only. |
| `atom` | — | architect-atom (Dioxus) | Optimistic `Store`, `AtomResult`, `use_mutation`, `use_app`, connection state. Client-side only. |
| `form` | `atom` | architect-form | Typed validated form fields over Entity payloads (`architect::form::*`). |
| `server-seaorm` | — | sea-orm, async-trait | The SeaORM bridge: `architect::storage::DbConn`, derive-emitted `<T>RepoStorage<C>`. |
| `server-axum` | `telemetry`, `platform` | axum, tokio (rt-multi-thread, macros), tracing, vox-core/types | `architect::axum_ws::{AxumWsLink, serve, …}` — the WebSocket transport. Independent of `server-seaorm`: bring your own repo impl and skip sea-orm entirely. |
| `dispatch-tokio` | — | tokio (rt-multi-thread) | `TokioBlockingDispatcher`, and makes it the `dispatch::DefaultDispatcher` that `#[derive(HasDispatcher)]` points at. |
| `local` | `vox`, `platform` | vox-core, vox-types, tracing, tokio rt | In-process transport (`architect::local`): serve a `LayerRouter` over an in-memory link. Native and wasm (wasm over architect's own `memory_link`). See [in-process transport](@/architecture/local.md). |
| `connect` | `vox`, `platform` | vox-core, vox-websocket, tracing; tokio-tungstenite (native), wasm-bindgen/web-sys/js-sys (wasm) | `architect::connect` — client connection establishment, one connection per (endpoint, identity), single-flighted and liveness-checked. Native and wasm. |
| `iroh` | `vox` | iroh, n0-future, tokio rt, tracing, vox-core/types | `architect::iroh_link` — serve and consume a vox router over an iroh QUIC endpoint, dialled by `EndpointId`. The client side also compiles on wasm (relay-only); serving is native only. See [peers](@/architecture/peers.md). |
| `iroh-lan` | `iroh` | iroh-mdns-address-lookup | LAN discovery for `iroh_link` (`PeerLookup::Lan`, `PeerLookup::N0AndLan`), so endpoints on one network dial each other with the internet down. Native only; a no-op on wasm. |
| `http` | — | axum, facet-json, async-channel | `architect::http` — the HTTP+JSON face the rpc derive emits per service, the `Call<Args>` extractor, the `ErrorBody` envelope, `BindHttp` / `HttpLayer`. See [the HTTP face](@/architecture/http.md). |
| `http-client` | — | reqwest, facet-json | The typed `<Trait>HttpClient` the rpc derive emits per service, over `reqwest` (native, and `fetch` on wasm). |
| `host` | `server-axum`, `http` | tracing-subscriber, tower-http, include_dir | `architect::host` — `EngineHost` serving a `LayerRouter` over `/vox` plus `/health`, the HTTP face, optional iroh and a static SPA bundle; `host::boot` for tracing and the panic logger. Native only. |
| `otlp-forward` | `host` | reqwest | `EngineHost::otlp_forward` — authenticated OTLP/HTTP ingest forwarded to an in-cluster collector. Native only. |
| `telemetry` | `vox` | tracing, opentelemetry, tracing-opentelemetry, web-time | One span per dispatched RPC call (`LayerRouter::call_span`), its duration as the `rpc.server.duration` metric, and W3C trace context across calls. See [telemetry](@/features/telemetry.md). |
| `action-list` | — | facet-json, facet-value, tracing | `architect::action::ActionList` — the registry every key, menu, CLI and MCP server runs an app's commands through, described as JSON Schema. See [commands](@/architecture/commands.md). |
| `settings` | — | facet-json, dirs, web-sys | `architect::settings` — `Store<T>` (one facet value as JSON in the app's config dir, or `localStorage` on wasm) and `FileKv`. See [settings](@/architecture/settings.md). |
| `rig` | `vox` | tracing | `architect::rig` — the `RigBackend` trait and the shared meter/status pump instrument-rig backends serve through. Native only. |
| `atom-persist` | `atom` | architect-atom/persist | Device persistence for atom stores: `architect::LocalRows`, `use_persisted_store_list`, the durable `CallQueue`, with memory and `localStorage` backends. |
| `atom-turso` | `atom-persist` | architect-atom/turso | `architect::TursoRows`, over a local Turso database. Native only (compiles to nothing on wasm). |
| `platform` | — | web-time, async-channel, tokio time/rt, gloo-timers + wasm-bindgen-futures (wasm) | `architect::platform` — `Clock` (async sleep/now), `spawn`, deterministic `TestClock`. Required by `use_interval`. |
| `schedule` | `platform` | — | `architect::schedule` — composable retry/repeat policies (backoff, jitter, caps) + drivers. |
| `rt` | — | rtrb | `architect::rt` — wait-free SPSC ring (`rt_channel`) bridging real-time threads (audio callbacks) to `PubSub`; drop-on-full, drained at UI rate. See [server push](@/architecture/streams.md). |
| `sync-client` | `vox` | tokio (rt) | `architect::BlockingCaller` — drives the `#[architect::rpc(sync_client)]`-emitted `<Trait>SyncClient` blocking facades. Native only; for sync code calling a *remote* backend (in-process callers use the sync trait directly). |
| `fake` | — | fake | `#[derive(fake::Dummy)]` on every emitted struct — trivially seedable test data. |
| `capability` | — | ed25519-dalek, base64, getrandom, zeroize | `architect::capability` — Ed25519-signed, expiring `purpose\|expiry\|subject` tokens and the `0600` `ServerKeypair` key file. Native only. See [capability tokens](@/architecture/capability.md). |
| `blob` | — | sha2 | `architect::blob` — `ResumableFetch` (`.part` resume, sha256 verify, `<file>.sha256` sidecar skip, atomic rename, one transfer per file) and the content-addressed `BlobStore` trait with `LocalFs`. Native only. See [blob transfer and storage](@/architecture/blob.md). |
| `release` | `blob` | minisign-verify, reqwest, facet-json, tempfile, tokio (process, time) | `architect::release` — signed self-update from GitHub Releases: minisign signature over `SHA256SUMS` (trusted comment names the tag), the asset's sha256, a `--version` smoke test, an atomic swap and a `--then` hook. `Updater`, `Target { current, exe, key: Option<&str> }`, `Options`, `Outcome`. Native only. See [signed self-update](@/architecture/release.md). |
| `testing-alloc` | — | — | `architect::testing::alloc` — `Counting`, a counting `GlobalAlloc` over `System` (live bytes, peak) that a test binary installs with `#[global_allocator]`; architect never installs it. Dev-dependencies only. See [testing](@/architecture/testing.md). |
| `testing` | `testing-alloc`, `host` | tempfile | `architect::testing::sandbox_server(seed)` — boot an app with `HOME` and the XDG/AppData dirs inside a tempdir, under one process-wide `env_lock`, and serve it on `127.0.0.1:0`. Native only, dev-dependencies only. See [testing](@/architecture/testing.md). |
| `snapshot` | `vox` | tokio (time), tracing | `architect::snapshot` — `WriteGate` (bounded hold, default 30 s), `GatedLayer` / `LayerRouter::gated` parking vox calls at the gate, `quiesce` (pause → checkpoint → step) and, with `server-seaorm`, the sqlite `wal_checkpoint` callback. Native only. See [snapshot quiesce](@/architecture/snapshot.md). |

## Implication graph

```text
client ─┬─ vox
        ├─ atom ◄────── form
        └─ form

server ─┬─ vox
        ├─ server-seaorm
        ├─ server-axum ─┬─ telemetry ── vox
        │               └─ platform
        ├─ dispatch-tokio
        └─ http

host ─┬─ server-axum
      └─ http
otlp-forward ── host
testing ─┬─ testing-alloc
         └─ host

schedule ── platform
local ───┬─ vox
         └─ platform
connect ─┬─ vox
         └─ platform
iroh-lan ── iroh ── vox
release ── blob
atom-turso ── atom-persist ── atom
```

Nothing implies `atom` except `form` and `client` — server-side flags
never pull Dioxus, and `atom`/`form` never pull tokio's runtime, so the
wasm build stays clean no matter which side of the graph you're on.

## Gotchas the matrix resolves

- **`form` without `vox`** compiles — fields and validation work — but
  `submit()` has no transport to post through. If you're using forms
  against a server, you want `client`.
- **`vox` off on a proto crate** means no clients, no dispatchers, no
  RPC machinery at all; the emitted repo trait is a plain async trait
  you can call in-process. This is a feature (CLI/embedded use), not a
  broken build.
- **`dispatch-tokio` off** makes `dispatch::DefaultDispatcher` the
  inline `CurrentThreadDispatcher` — fine for tests, wrong for a server
  wrapping blocking work. `server` turns it on for you.
- **`architect-atom` direct dependency vs `architect/atom`**: the
  primitive crate is wasm-clean and Dioxus-only; the re-export through
  `architect` additionally lights up vox-aware hooks when `vox` is on.
