+++
title = "EngineHost and protocol mounts"
description = "One server builder: /health, /vox and /version.json always; plugins, protocol mounts and iroh by builder method; the permissions gate on every vox lane."
weight = 12
+++

## What it is and when to use it

`EngineHost` turns a `LayerRouter` and its `PermissionsGate` into a native
server. It always mounts `/health` and `/vox` (the WebSocket, `vox.v1`
echoed), and `/version.json` once given build info. Everything else is a builder method: generated HTTP
faces as plugins, an SPA bundle, tower layers, an iroh endpoint, and the
protocol endpoints an app would otherwise hand-route: `vox_at` (a vox lane per
path, chosen per upgrade), `document` (a JSON document at a path), `dav`
(WebDAV verbs) and `otlp_forward` (the browser's telemetry to a collector).
MCP mounts through `architect_mcp::host::McpHost`.

The gate is on every vox lane the host owns, by default:

| Lane | Who the gate sees |
| --- | --- |
| `/vox` | the upgrade's bearer (`Authorization: Bearer`, or the `vox.bearer.<token>` subprotocol a browser offers), per connection; per-call `authorization` metadata wins |
| the iroh lane (`.iroh`, `.iroh_endpoint`) | `Principal::Host { endpoint }`, the endpoint id the QUIC handshake proved; a call that presents a bearer is resolved from it instead |
| `vox_at(path, resolve)` | as `/vox`: the host wraps the handler of the `Lane` the resolver picks, inside the lane's scope (`Lane::new(router).scoped(org)`), so the gate checks each call in the lane's org |
| `vox_at_permissioned(path, resolve)` | whatever gate the resolver put in the `PermissionedRouter` it returns (a gate per org, a share link's guest), optionally `scoped(org)` outside it; not wrapped again |

`EngineHost::ungated(router, bind)` is the opt-out: no gate on any lane, no
caller in any handler, and a `warn` on every start. It is for a router whose
services check their own tokens (`apps/auth-server`), a demo with no accounts
(`examples/greeter`), or a loopback test.

Use it for every server binary. Reach for `.plugin(Router)` only for what no
service and no builder covers (OAuth redirects, hosted pages); `cargo xtask
boundaries` counts those hand routes. Design:
[The HTTP face](@/architecture/http.md), sections on mounting and protocol mounts.

## A minimal example

From `examples/greeter/src/main.rs`, features `host` (implies `server-axum`,
`telemetry`, `vox`, `http`). The greeter has no accounts, so it says so:

```rust
architect::host::boot("info");
let backend = Memory::default();
EngineHost::ungated(backend.clone().into_router(), config.bind)
    .plugin(backend.into_http_router())
    .build_info(architect::build_info!())   // ARCHITECT_BUILD_REV, ARCHITECT_BUILT_AT
    .serve()
    .await?;
```

A gated host with a per-org vox lane, and the other mounts (after
`crates/architect/tests/host_gate.rs` and `crates/architect/tests/host_mounts.rs`;
`otlp_forward` needs feature `otlp-forward`, `iroh` needs `iroh`):

```rust
let gate = PermissionsGate::new(engine, Arc::new(identity))
    .permit(notes_service_descriptor(), &NOTES_PERMITS);
EngineHost::new(router.clone(), gate, bind)
    // The resolver picks the lane and its org; the host gates the
    // router inside that org, so the engine checks every call there.
    .vox_at("/org/{slug}/vox", move |head: RequestHead| {
        let router = router.clone();
        async move {
            if head.param("slug") != Some("acme") { return Err(StatusCode::NOT_FOUND); }
            Ok(Lane::new(router).scoped(OrgId::new("acme")))
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

1. `crates/architect/src/host.rs` `EngineHost`: `new` (router and gate),
   `ungated`, `gate`, `plugin`, `finish`, `build_info`, `iroh`,
   `iroh_endpoint`, `iroh_lookup`, `web`, `into_app` (for tower tests),
   `serve`.
2. `crates/architect/src/host.rs` `assemble_app`: mounts `/health` and
   `/version.json`, then `/vox` (unless `vox_at("/vox", ..)` replaced it), the
   extra mounts, the SPA fallback and the `finish` layers. `VoxLane` wraps
   each `/vox` connection in the gate with its bearer.
3. `crates/architect/src/host/mount.rs` `vox_at`, `vox_at_permissioned`,
   `Lane`, `document`, `dav`, `DavRequest`: each hook is handed a
   `RequestHead`. `crates/architect/src/permissions_gate.rs` `LaneScope`,
   `Scoped`: a lane's scope, entered outside the gate.
4. `crates/architect/src/http/body.rs` `RequestHead`: `param`, `query_param`,
   `bearer` (the header, then the `vox.bearer.` subprotocol).
5. `crates/architect/src/host/otlp.rs` `otlp_forward`, `OTLP_SIGNALS`: traces,
   logs and metrics pass through once authorized; an unknown signal is 404, a
   collector that is down 502.
6. `crates/architect/src/host.rs` `BuildInfo`, `build_info!`, `serve_iroh`,
   `serve_iroh_lane`: one gated router per peer connection through
   `iroh_link::serve_peers`.
7. `crates/architect/src/permissions_gate.rs` `PermissionsGate`,
   `wrap_shared_with_bearer` (`/vox`, `vox_at`),
   `wrap_shared_with_proved_caller` (iroh), `coverage` (reported by `new`;
   see [Permissions](@/features/permissions.md)).

An upgrade to `/org/acme/vox` builds a `RequestHead` from the path parameters
and headers and awaits the resolver. `Err(status)` answers with that status.
`Ok(handler)` is wrapped in the host's gate with the upgrade's bearer and
served over the WebSocket with `vox.v1` echoed. On `serve()`, `serve_iroh`
serves the router given to `new` on the iroh endpoint, each peer connection
behind the gate as the `Principal::Host` it proved.

## Tests to read

- `crates/architect/tests/host_gate.rs`: a caller the gate does not permit is
  refused on `/vox`, on a `vox_at` lane and on the iroh lane (in-process
  loopback endpoints, relays off), and a permitted one is served as itself;
  `vox_at_permissioned` keeps the resolver's own gate; `ungated` serves with
  no caller; a lane's scope (a `LaneScope`) is entered outside the gate, on
  `vox_at` and on a `vox_at_permissioned` lane `scoped` outside its own.
- `features/orgs/tests/host_lane.rs`: an org's `vox_at` lane refuses a member
  of another org at the gate; `features/spaces/tests/wires.rs` the same for a
  space-scoped lane (the org engine beside `SpacesEngine`).
- `crates/architect/tests/host_mounts.rs`: `vox_at` resolving each upgrade from
  its parameters and bearer; `vox_at("/vox")` replacing the built-in; `dav`
  answering every verb outside the `finish` layers; `document` beating the SPA.
- `examples/greeter/tests/host_mounts.rs`: `otlp_forward` passing the three
  signals once authorized; MCP mounted per path and on loopback.
- `crates/architect/tests/version_json.rs`: the fields, `null` when unset.
- `apps/auth-server/src/server.rs`: a real host.

## Status

Done in architect. The router gate does not cover in-process Action List runs
(see [Permissions](@/features/permissions.md)), and the HTTP face is not behind
the vox gate: a generated route reads its bearer through `RequestHead`.

- **Task** does not use `EngineHost`. It hand-rolls what the builders now
  cover: `/vox` upgrades and `/.well-known/task-server.json`
  (`task:apps/server/src/lib.rs`), the OTLP forward
  (`task:apps/server/src/otlp.rs`) and WebDAV (`task:apps/server/src/webdav.rs`).
  Its org lanes, vox and iroh, already wrap with
  `PermissionsGate::wrap_shared_with_bearer`.
- **Studio** uses `EngineHost` with `.plugin`, `.web` and `.iroh`
  (`studio:apps/studio/desktop/src/engine_main.rs`) and gates none of its
  lanes; moving to this release means passing a gate to `new` or saying
  `ungated` at each host.
