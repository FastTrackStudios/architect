+++
title = "EngineHost and protocol mounts"
description = "One server builder: /health, /vox and /version.json always; plugins, protocol mounts, iroh and the permissions gate by builder method."
weight = 12
+++

## What it is and when to use it

`EngineHost` turns a `LayerRouter` into a native server. It always mounts
`/health` and `/vox` (the WebSocket, `vox.v1` echoed), and `/version.json`
once given build info. Everything else is a builder method: generated HTTP
faces as plugins, an SPA bundle, tower layers, an iroh endpoint, and the
protocol endpoints an app would otherwise hand-route: `vox_at` (a vox lane per
path, chosen per upgrade), `document` (a JSON document at a path), `dav`
(WebDAV verbs) and `otlp_forward` (the browser's telemetry to a collector).
MCP mounts through `architect_mcp::host::McpHost`.

Use it for every server binary. Reach for `.plugin(Router)` only for what no
service and no builder covers (OAuth redirects, hosted pages); `cargo xtask
boundaries` counts those hand routes. Design:
[The HTTP face](@/architecture/http.md), sections on mounting and protocol mounts.

## A minimal example

From `examples/greeter/src/main.rs`, features `host` (implies `server-axum`,
`telemetry`, `vox`, `http`):

```rust
architect::host::boot("info");
let backend = Memory::default();
EngineHost::new(backend.clone().into_router(), config.bind)
    .plugin(backend.into_http_router())
    .build_info(architect::build_info!())   // ARCHITECT_BUILD_REV, ARCHITECT_BUILT_AT
    .serve()
    .await?;
```

A per-org vox lane behind the permissions gate, and the other mounts (after
`crates/architect/tests/host_mounts.rs`; `otlp_forward` needs feature
`otlp-forward`, `iroh` needs `iroh`):

```rust
EngineHost::new(router.clone(), bind)
    .vox_at("/org/{slug}/vox", move |head: RequestHead| {
        let (gate, router) = (gate.clone(), router.clone());
        async move {
            if head.param("slug") != Some("acme") { return Err(StatusCode::NOT_FOUND); }
            Ok(PermissionsGate::wrap_shared_with_bearer(gate, router, head.bearer().map(str::to_owned)))
        }
    })
    .document("/.well-known/app.json", |head| async move { Ok::<_, StatusCode>(head.path().to_owned()) })
    .dav("/org/{slug}/dav", |dav: DavRequest| async move { /* a Response */ })
    .otlp_forward("/otlp", "http://otel-collector:4318", |head| async move {
        head.bearer().map(|_| ()).ok_or(StatusCode::UNAUTHORIZED)
    })
    .iroh(key_path, Some(id_path))
    .serve()
    .await?;
```

## Code tour

1. `crates/architect/src/host.rs` `EngineHost`: `new`, `plugin`, `finish`,
   `build_info`, `iroh`, `iroh_lookup`, `web`, `into_app` (for tower tests),
   `serve`.
2. `crates/architect/src/host.rs` `assemble_app`: mounts `/health` and
   `/version.json`, then `/vox` (unless `vox_at("/vox", ..)` replaced it), the
   extra mounts, the SPA fallback and the `finish` layers.
3. `crates/architect/src/host/mount.rs` `vox_at`, `document`, `dav`,
   `DavRequest`: each hook is handed a `RequestHead`.
4. `crates/architect/src/http/body.rs` `RequestHead`: `param`, `query_param`,
   `bearer` (the header, then the `vox.bearer.` subprotocol).
5. `crates/architect/src/host/otlp.rs` `otlp_forward`, `OTLP_SIGNALS`: traces,
   logs and metrics pass through once authorized; an unknown signal is 404, a
   collector that is down 502.
6. `crates/architect/src/host.rs` `BuildInfo`, `build_info!`, `serve_iroh`.
7. `crates/architect/src/permissions_gate.rs` `PermissionsGate`,
   `wrap_shared_with_bearer`, `coverage`: the gate a resolver wraps around the
   router (see [Permissions](@/features/permissions.md)).

An upgrade to `/org/acme/vox` builds a `RequestHead` from the path parameters
and headers and awaits the resolver. `Err(status)` answers with that status.
`Ok(handler)` can be any vox handler, here a gated router, and is served over
the WebSocket with `vox.v1` echoed. On `serve()`, `serve_iroh` serves the
router given to `new` on the iroh endpoint.

## Tests to read

- `crates/architect/tests/host_mounts.rs`: `vox_at` resolving each upgrade from
  its parameters and bearer; `vox_at("/vox")` replacing the built-in; `dav`
  answering every verb outside the `finish` layers; `document` beating the SPA.
- `examples/greeter/tests/host_mounts.rs`: `otlp_forward` passing the three
  signals once authorized; MCP mounted per path and on loopback.
- `crates/architect/tests/version_json.rs`: the fields, `null` when unset.
- `apps/auth-server/src/server.rs`: a real host.

## Status

Done in architect. One thing to know: the built-in `/vox` and the iroh lane
serve the router passed to `new` as it is. A gate applies where the app wraps
it, through a `vox_at` resolver or by wrapping the router before `new`; the
router gate also does not cover in-process Action List runs (see
[Permissions](@/features/permissions.md)).

- **Task** does not use `EngineHost`. It hand-rolls what the builders now
  cover: `/vox` upgrades and `/.well-known/task-server.json`
  (`task:apps/server/src/lib.rs`), the OTLP forward
  (`task:apps/server/src/otlp.rs`) and WebDAV (`task:apps/server/src/webdav.rs`).
  It already wraps with `PermissionsGate::wrap_shared_with_bearer`.
- **Studio** uses `EngineHost` with `.plugin`, `.web` and `.iroh`
  (`studio:apps/signal/desktop/src/engine_main.rs`); it pins a tag older than
  the protocol mounts and `build_info`.
