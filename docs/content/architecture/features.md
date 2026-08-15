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
| `server` | `vox`, `server-seaorm`, `server-axum`, `dispatch-tokio` | Native server binaries. RPC, SeaORM storage bridge, axum WebSocket transport, tokio blocking dispatcher. |
| `full`   | `vox`, `server-seaorm`, `server-axum`, `fake`, `platform`, `schedule`, `rt` | Development / single-binary builds. Deliberately excludes `atom`/`form` (server builds never compile Dioxus) and `diagnostics` (needs a frame-pointer build, see below). |

A typical app:

```toml
# apps/web (wasm UI)
architect = { workspace = true, features = ["client"] }

# apps/server (axum binary)
architect = { workspace = true, features = ["server"] }

# features/foo/foo-proto (contract crate) — forward vox so each
# consumer decides whether the RPC stack compiles:
[features]
vox = ["dep:vox", "dep:vox-types", "architect/vox"]
```

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
| `server-axum` | — | axum, tokio (rt-multi-thread), futures, tracing, vox-core/types, moire | `architect::axum_ws::{AxumWsLink, serve, …}` — the WebSocket transport. Independent of `server-seaorm`: bring your own repo impl and skip sea-orm entirely. |
| `dispatch-tokio` | — | tokio (rt-multi-thread) | `TokioBlockingDispatcher`, and makes it the `dispatch::DefaultDispatcher` that `#[derive(HasDispatcher)]` points at. |
| `local` | `vox` | vox-core, tracing, tokio rt | In-process transport: serve a `LayerRouter` over a vox memory link. Native only. |
| `platform` | — | web-time, async-channel, tokio time/rt, gloo-timers + wasm-bindgen-futures (wasm) | `architect::platform` — `Clock` (async sleep/now), `spawn`, deterministic `TestClock`. Required by `use_interval`. |
| `schedule` | `platform` | — | `architect::schedule` — composable retry/repeat policies (backoff, jitter, caps) + drivers. |
| `rt` | — | rtrb | `architect::rt` — wait-free SPSC ring (`rt_channel`) bridging real-time threads (audio callbacks) to `PubSub`; drop-on-full, drained at UI rate. See [server push](@/architecture/streams.md). |
| `sync-client` | `vox` | tokio (rt) | `architect::BlockingCaller` — drives the `#[architect::rpc(sync_client)]`-emitted `<Trait>SyncClient` blocking facades. Native only; for sync code calling a *remote* backend (in-process callers use the sync trait directly). |
| `fake` | — | fake | `#[derive(fake::Dummy)]` on every emitted struct — trivially seedable test data. |
| `diagnostics` | — | moire/diagnostics | Moiré instrumentation in axum_ws (dashboard at `MOIRE_DASHBOARD`). Zero-cost passthrough when off. Enable only in builds compiled with `-C force-frame-pointers=yes` — moire's trace capture validates frame pointers at runtime and aborts otherwise (which is why it's not in `full`). |

## Implication graph

```text
client ─┬─ vox
        ├─ atom ◄────── form
        └─ form

server ─┬─ vox
        ├─ server-seaorm
        ├─ server-axum
        └─ dispatch-tokio

schedule ── platform
local ───── vox
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
