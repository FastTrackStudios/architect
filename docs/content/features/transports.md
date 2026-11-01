+++
title = "Transports"
description = "One router, any wire: vox in process, over WebSocket and over iroh; the HTTP face with query arguments, ranged bytes and SSE."
weight = 11
+++

## What it is and when to use it

A backend becomes one `LayerRouter`, and the transport is picked last. vox
runs it in process (`LocalServer`, for desktop apps, CLIs and tests), over a
WebSocket (`/vox` on a server, `architect::connect` on the client) or over
iroh QUIC, dialled by endpoint id. The same trait also has an HTTP face: JSON
by default, arguments in the query string on a `get`, byte methods with
`Range` and 206, and each `#[subscribe]` method as Server-Sent Events.

Pick vox for an app's own clients (typed, streaming, one connection) and HTTP
for browsers without a vox client, curl and other tools. Design pages:
[The in-process transport](@/architecture/local.md),
[The HTTP face](@/architecture/http.md) and
[Peer to peer](@/architecture/peers.md) for iroh.

## A minimal example

Cargo features on `architect`: `local` (native and wasm), `connect` (the
WebSocket client and iroh's client side), `server-axum`, `iroh` (plus
`iroh-lan` for LAN lookup), `http`, `http-client`.

```rust
// In process
let scope = Scope::new();
let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
let client: GreeterClient = local.establish().await?;

// WebSocket: EngineHost serves /vox; the client shares one connection per endpoint
let endpoint = architect::connect::Endpoint::url(format!("ws://{addr}/vox")).with_bearer(token);
let client: GreeterClient = architect::connect::pool().client(&endpoint).await?;

// HTTP: the same trait
let http = GreeterHttpClient::at(base);
let mut greeted = http.greeted().await?;        // an SSE EventStream
```

On the HTTP face, a `get` reads its arguments from the query, and a method
returning `ByteBody` answers ranged requests (from
`crates/architect/tests/http_routes.rs` and `crates/architect/tests/http_bytes.rs`):

```rust
#[http(get, path = "search")]
async fn search(&self, slug: String, q: String, limit: u32, after: Option<u32>)
    -> Result<Vec<u32>, NoteError>;

#[http(get, path = "blobs/{hash}")]
async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError>;
// ByteBody::new(total, mime, |offset, len| async move { .. }), ByteBody::from_bytes, ByteBody::file
```

## Code tour

1. `crates/architect/src/local.rs` `LocalServer`, `establish`: an in-process
   link plus an acceptor task registered on the `Scope`.
2. `crates/architect/src/memory_link.rs` `memory_link_pair`: the link
   architect uses on wasm.
3. `crates/architect/src/axum_ws.rs` `serve_router`, `AxumWsLink`: the server
   half of vox over a WebSocket.
4. `crates/architect/src/connect/mod.rs` `Pool`, `Endpoint`, `pool`: one
   connection per URL and bearer, dialled once for concurrent callers and
   checked for liveness.
5. `crates/architect/src/connect/ws.rs` `dial`: the bearer is an
   `Authorization` header natively and a `vox.bearer.<token>` subprotocol in a
   browser.
6. `crates/architect/src/iroh_link.rs` `bind_endpoint_with`, `serve_router`,
   `connect`: vox over iroh on ALPN `architect/vox/1`, a bounded dial with
   one retry.
7. `crates/architect/src/http.rs` `QueryString`, `query_value`, `sse`,
   `HttpClient`: query arguments, SSE and the typed client.
8. `crates/architect/src/http/body.rs` `ByteBody`, `BodyStream`, `RequestHead`
   and `crates/architect/src/http/ranged.rs` `serve_body`, `serve_file`,
   `parse_range`: byte methods.

A vox call goes out over a link (memory, WebSocket or iroh stream). The far
side hands each connection to `LayerRouter::acceptor`, which dispatches by
method id: the three transports run the same router. On HTTP, a `get` handler
parses path parameters (`400 invalid_path`) and query arguments
(`400 invalid_query`; an absent `Option` is `None`), calls the method, and a
`ByteBody` result goes through `serve_body` as 200, 206 or 416.

## Tests to read

- `apps/auth-server/tests/surfaces.rs`: one scenario over HTTP, `LocalServer`,
  pooled WebSocket and iroh.
- `examples/greeter/tests/round_trip.rs`: in process, HTTP and SSE.
- `crates/architect/tests/http_routes.rs`: every face runs one scenario; a
  `get` reads its arguments from the query.
- `crates/architect/tests/http_bytes.rs`: an upload past its cap, a ranged
  download, query arguments on both.
- `crates/architect/tests/http_ranged.rs`: `serve_file` and `serve_body`
  answering 206 and 416.
- `crates/architect/tests/http_face.rs`: subscriptions as SSE; the typed client
  over a real socket.
- `crates/architect/tests/iroh_link.rs`: vox over iroh, redial after a
  restart, an unreachable peer timing out.
- `crates/architect/tests/iroh_lan.rs`: dial by bare id on a LAN (needs multicast).
- `crates/architect/tests/reconnect_supervisor.rs`: re-establishing after the
  server dies.

## Status

Done in architect; query arguments are newer than the latest tag. Not yet
(from the HTTP design page): an HTTP face for `context = T` traits; query
arguments on JSON-body routes; `#[subscribe]` under a service `path` with
parameters; more than one range per request or `If-Range`; typed errors on
SSE; byte methods over vox.

- **Task** uses `LocalServer`, mounts `axum_ws::serve_router` on its own routes
  (`task:apps/server/src/lib.rs`), uses `iroh_link`
  (`task:apps/server/src/iroh_host.rs`) and the ranged helpers on hand-written
  routes (`task:apps/server/src/attachments/routes.rs`). To adopt: the
  generated HTTP face and clients, `ByteBody` methods, `connect::pool`.
- **Studio** uses `LocalServer` and `iroh_link::serve_router`
  (`studio:apps/session/desktop/src/engine_server.rs`); no HTTP face yet.
