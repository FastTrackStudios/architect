+++
title = "The server"
description = "EngineHost and its transports: vox over WebSocket, iroh and in-process; the HTTP face with verbs, parameters, streamed bodies, ranges and SSE; protocol mounts; health."
weight = 40
+++

## What you'll build

A binary that serves one backend on every wire: vox over a WebSocket
at `/vox`, the same router over iroh, the generated HTTP+JSON routes
and SSE streams, `/health` and `/version.json`, and whatever protocol
mounts the app needs (MCP, an OTLP forward, WebDAV). Greeter's whole
server is `examples/greeter/src/main.rs`. The server is an L3 crate, so
it lives under `apps/<app>/server` (greeter is the documented
exception, counted in the `boundaries.toml` baseline).

## 1. The binary

```rust
// examples/greeter/src/main.rs
use architect::Services as _;
use architect::host::EngineHost;
use example_greeter::Memory;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    architect::host::boot("info");
    let config = Config::from_env()?;
    let backend = Memory::default();
    // A demo with no accounts: every wire is public, the HTTP face too.
    // A real server passes its `PermissionsGate` to `EngineHost::new`.
    EngineHost::ungated(backend.clone().into_router(), config.bind)
        .plugin(backend.into_http_router())
        .build_info(architect::build_info!())
        .serve()
        .await?;
    Ok(())
}
```

- `architect::host::boot(filter)` installs a fmt `tracing` subscriber
  and a panic logger. It is the minimal setup. A production server
  installs the full one instead: `architect_telemetry::init` (Sentry)
  held for all of `main`, an `EnvFilter`, the fmt layer,
  `architect_telemetry::tracing_layer()`, then the layers from
  `architect_telemetry::otel::init` when OTLP is configured. Install
  one subscriber, once. (`task:apps/server/src/main.rs` is the
  reference; `features/telemetry/src/lib.rs` has the functions.)
- `EngineHost::new(router, gate, bind)` takes the vox `LayerRouter` and
  the `PermissionsGate` that checks every vox lane the host owns: `/vox`
  (the caller from the upgrade's bearer), the iroh lane (the caller is the
  peer's proved endpoint id, `Principal::Host`) and every `vox_at` lane.
  `EngineHost::ungated(router, bind)` serves the router to everyone
  unchecked and logs a `warn` on every start; the greeter uses it because
  it has no accounts.
- `.plugin(router)` merges an axum router: here the generated HTTP
  face. A host may take several.
- `.build_info(architect::build_info!())` fills `/version.json`
  (below).
- `.serve()` binds TCP and serves until the process dies; a port in
  use is an `Err`, not a panic. `.into_app()` returns the assembled
  axum `Router` without binding, for tests.

The architect features this needs: `host` (which brings `server-axum`,
`http` and `telemetry`), and `iroh` for peers.

Try it:

```sh
GREETER_BIND=127.0.0.1:4040 cargo run -p example-greeter
curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'
curl -N localhost:4040/greeter/greeted
curl -XPOST localhost:4040/note/create -d '{"input":{"title":"hi","body":"there"}}'
```

## 2. vox transports

The same `LayerRouter` serves every vox transport. Clients cannot tell
them apart; only the dial differs.

| Transport | Server side | Notes |
|---|---|---|
| WebSocket | `EngineHost` mounts `/vox` (subprotocol `vox.v1`) | a browser sends its bearer as the `vox.bearer.<token>` subprotocol |
| iroh | `.iroh(key_path, id_path)` on the host | QUIC, dialled by bare endpoint id; `.iroh_lookup(PeerLookup::N0AndLan)` (feature `iroh-lan`) adds LAN lookup |
| in-process | `LocalServer::serve(router, scope)` | a vox memory link: desktop apps, CLIs, tests (feature `local`) |
| per-path | `.vox_at("/org/{slug}/vox", resolve)` | a resolver picks the `Lane` (handler and org) per connection; the gate checks inside its org |

The iroh key file is created `0600`; `id_path`, when given, receives
the endpoint id so other processes can find it.
`apps/auth-server/src/server.rs` `build` turns iroh on from config.

In-process serving is the same router with no socket:

```rust
let scope = Scope::new();
let local = LocalServer::serve(backend.into_router(), scope.clone());
let client: GreeterClient = local.establish().await?;
// …
scope.close().await;   // stops the acceptor tasks
```

## 3. Gate the router

Every mounted service needs a permit table before it is exposed. Wrap
the router in a `PermissionsGate` (`crates/architect/src/permissions_gate.rs`):
it resolves the caller from the bearer, checks each method against its
permit, and makes the principal available to the handler through
`permissions_gate::caller()`. `gate.coverage(&router)` lists methods
with no permit; keep it empty, and start a new gate in observe-only
mode while the table fills in. Details are in
[auth](@/architecture/auth.md).

## 4. The HTTP face

`backend.into_http_router()` (or `layers![…].provide_http(&backend)`)
is the axum router the service attribute generated. By default each
method is `POST /<prefix>/<method-kebab>` with a JSON object of its
named arguments:

```text
POST /greeter/greet
content-type: application/json

{"name": "cody"}

200 OK
"hello cody"
```

- An error replies with its `HttpError` status and
  `{"code": "unknown", "message": "nobody is called x", "error": {"Unknown": "x"}}`;
  the generated client decodes `error` back into the typed enum.
- A body that does not decode is `422`. A body over the route's limit
  (2 MiB by default) is `413`.
- An argument named `token` may come from `Authorization: Bearer …`.
- `http::PATHS` lists every `(method, path)` the router mounts.

### Verbs and path parameters

```rust
#[architect::service(path = "org/{slug}")]
pub trait Notes {
    #[http(get, path = "notes/{id}")]                 // GET    /org/{slug}/notes/{id}
    async fn note(&self, slug: String, id: u32) -> Result<String, NoteError>;
    #[http(delete, path = "notes/{id}")]              // DELETE /org/{slug}/notes/{id}
    async fn drop_note(&self, slug: String, id: u32) -> Result<(), NoteError>;
    #[http(get, path = "files/{*path}")]              // GET    /org/{slug}/files/a/b.md
    async fn file(&self, slug: String, path: String) -> Result<String, NoteError>;
    #[http(post, path = "bulk", limit = "4MiB")]      // a bigger JSON body
    async fn bulk(&self, slug: String, text: String) -> Result<u64, NoteError>;
}
```

Verbs are `get`, `put`, `post`, `delete` and `any`. `{name}` binds one
segment and `{*name}` the rest of the path to the argument of that
name, parsed with `FromStr` (`400 invalid_path` when it does not
parse). The methods keep their vox face and their typed clients.
`crates/architect/tests/http_routes.rs` runs this on every face.

### Query arguments

A route with no JSON body (a `get`, or a method taking a `BodyStream`)
reads its other arguments from the query string:

```rust
#[http(get, path = "search")]          // GET /org/{slug}/search?q=apple%20pie&limit=10
async fn search(&self, slug: String, q: String, limit: u32, after: Option<u32>)
    -> Result<Vec<u32>, NoteError>;
```

A missing `Option` is `None`; a missing or unparseable required
argument is `400 invalid_query` before the method runs; unknown names
are ignored. `?token=` works for URLs that cannot carry a header (an
`<img src>`).

### Streamed uploads and ranged downloads

Bytes are not JSON. Three types from `architect::http`
(`crates/architect/src/http/body.rs`) say so in the signature:

```rust
#[architect::service(path = "org/{slug}")]
pub trait Media {
    #[http(get, path = "blobs/{hash}")]
    async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError>;

    #[http(put, path = "blobs/upload", limit = "1GiB")]
    async fn upload(&self, slug: String, body: BodyStream) -> Result<BlobRef, MediaError>;
}

impl Media for Store {
    async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError> {
        let (total, mime) = self.stat(&slug, &hash).await?;
        let store = self.clone();
        Ok(ByteBody::new(total, mime, move |offset, len| async move {
            store.read_range(&slug, &hash, offset, len).await.map(Body::from)
        }))
    }
    // …
}
```

- `ByteBody` is served through `architect::http::ranged`: no `Range`
  is `200`, a satisfiable range `206` with `Content-Range`, an
  unsatisfiable one `416`. The reader is called once with exactly the
  window selected. `ByteBody::from_bytes` serves memory and
  `ByteBody::file` a file.
- `BodyStream` is the request body as a stream of chunks, capped at the
  route's `limit`: a declared length over it is `413` before the method
  runs, and a chunked body that grows past it ends the stream with
  `io::ErrorKind::FileTooLarge`.
- `RequestHead` hands over the method, headers, path params and query
  (`bearer()`, `header(..)`, `param(..)`, `query_param(..)`).

A trait with any of these is an HTTP-only service: vox cannot carry a
byte stream, so it gets no vox client or dispatcher. Keep the methods a
vox client needs in their own trait, and mount the HTTP-only one with
`layers![MediaService].provide_http(&backend)`.
`crates/architect/tests/http_bytes.rs` uploads, refuses and downloads
by range over a socket.

### SSE

Each `#[subscribe]` is an SSE route: `POST /<prefix>/<name-kebab>`
with the filters as the JSON body (or `GET` for one with no filters)
answers `text/event-stream`, one `data: <json>` frame per event, until
the client disconnects. `http::stream_router(backend)` mounts streams;
the `StreamService` token in the bundle does the same through
`provide_http`. An argless stream sends its `<name>_intro` frame first.
Streams carry no typed errors: a failed subscribe is a transport error.

## 5. Protocol mounts

Endpoints that adapt a protocol rather than expose a capability are
builder methods on the host, not hand-written routes
(`crates/architect/src/host/mount.rs`):

```rust
EngineHost::new(vox, gate, bind)
    // The resolver picks the lane: a router and the org its calls run in.
    // The host's gate wraps the router inside that org.
    .vox_at("/org/{slug}/vox", move |head: RequestHead| {
        let orgs = orgs.clone();
        async move {
            orgs.get(head.param("slug").unwrap_or_default())
                .map(|org| Lane::new(org.router()).scoped(org.id()))
                .ok_or(StatusCode::NOT_FOUND)
        }
    })
    .document("/.well-known/app.json", move |head| discovery(head.bearer()))
    .dav("/org/{slug}/dav", move |dav: DavRequest| files.handle(dav))
    .finish(|app| app.layer(cors))
```

| Mount | Serves | Feature |
|---|---|---|
| `.vox_at(path, resolve)` | a vox WebSocket at a parameterised path behind the host's gate, inside the `Lane`'s scope (its org) so the gate checks there; the resolver refuses with a status before the upgrade | `host` |
| `.vox_at_permissioned(path, resolve)` | the same, for a resolver that returns a `PermissionedRouter` it gated itself (a gate per org, a share link's guest), optionally `.scoped(org)` | `host` |
| `.document(path, describe)` | a `GET` document ahead of the SPA fallback | `host` |
| `.dav(mount, handle)` | every WebDAV verb at and below `mount`, outside the `finish` layers so CORS does not answer `OPTIONS` | `host` |
| `.otlp_forward(at, upstream, authorize)` | `POST {at}/v1/{traces,logs,metrics}` forwarded to a collector once `authorize(head)` is `Ok`; other signals `404`, a dead collector `502` | `otlp-forward` |
| `.mcp(server, token)`, `.mcp_at(path, resolve)` | an MCP endpoint over the app's Action List | `architect-mcp` feature `host` (`architect_mcp::host::McpHost`) |

`examples/greeter/tests/host_mounts.rs` mounts MCP per org and on
loopback, and forwards OTLP to a fake collector:

```rust
let app = EngineHost::ungated(LayerRouter::new(), "127.0.0.1:0")
    .otlp_forward("/otlp", &format!("http://{upstream}/"), ingest_only)
    .into_app();
```

`.finish(|app| …)` wraps the assembled app (CORS, a trace layer) after
plugins and the SPA fallback. `.plugin(router)` stays for what neither
a service nor a mount covers: OAuth redirects, server-rendered pages.
`apps/auth-server/src/server.rs` `host_with_senders` shows both.

## 6. Health, version and the web bundle

- `/health` answers `ok` on every host. Add deployment-named probes
  (`/healthz`, `/readyz`) as a plugin if the platform expects them, as
  the auth server does.
- `/version.json` is `{"rev", "version", "built_at"}`, sent
  `no-store`. `architect::build_info!()` is expanded in the binary
  crate and reads `CARGO_PKG_VERSION`, `ARCHITECT_BUILD_REV` and
  `ARCHITECT_BUILT_AT` at compile time; an unset variable is `null`:

  ```sh
  ARCHITECT_BUILD_REV=$(git rev-parse HEAD) \
  ARCHITECT_BUILT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ) \
  cargo build --release -p example-greeter
  ```

- `.web(Some(bundle))` serves a built SPA as the fallback;
  `.cross_origin_isolated(true)` adds the COOP/COEP headers a page needs
  for `SharedArrayBuffer`.

## Where to read more

- [The HTTP face](@/architecture/http.md): the wire in full, ranges,
  protocol mounts, `/version.json`, iroh LAN lookup, what is not
  supported yet.
- [The in-process transport](@/architecture/local.md): `LocalServer`.
- [Peers](@/architecture/peers.md): every node as a peer, over iroh.
- [Auth](@/architecture/auth.md): the gate and sign-in.
- [Commands](@/architecture/commands.md): the Action List behind the
  MCP mount.
- [Server push](@/architecture/streams.md): streams on every wire.
