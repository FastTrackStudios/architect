+++
title = "The HTTP face"
description = "#[architect::service] — one trait, vox and HTTP+JSON, clients for both; what the derive emits and how a host mounts it."
weight = 36
+++

vox is the native wire for the Rust apps. A browser `fetch`, a `curl`,
a webhook, a language with no vox binding — those need the same service
as plain HTTP with JSON bodies. Nobody writes that mount by hand: the
derive emits it from the trait, exactly as it emits the vox client and
dispatcher.

## One attribute, every face

```rust,ignore
#[architect::service]
pub trait Greeter {
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    async fn whoami(&self, token: String) -> Result<String, GreetError>;
}
```

The consumer crate's cargo features decide which faces compile:

| feature on the proto crate | what materialises |
|---|---|
| (none) | the trait, the direct view, the in-process host |
| `vox` | `GreeterClient`, `GreeterDispatcher`, the descriptor, the `Service` token, `layer` / `serve` |
| `http` | `greeter::http::router(backend)` (axum), `http::<Method>Args`, `impl BindHttp for Service` |
| `http-client` | `GreeterHttpClient` |

A proto crate forwards **nothing** for HTTP: `architect::http` is a
facade that exists in every build — axum and reqwest under the
features, inert stubs otherwise — so the emitted routers and clients
compile wherever the trait does, and the *app* decides by turning
`http` / `http-client` on in `architect`. `#[architect::rpc]` is still
the vox-only spelling and `#[architect::http]` the explicit opt-in next
to it; `service` is both.

## Clients implement the trait

When every method of a service is fallible and each error type
implements `From<architect::TransportError>`, the derive also emits
`impl Greeter for GreeterHttpClient` and `impl Greeter for GreeterClient`
(vox). A screen, a CLI or a test then takes `impl Greeter` and never
learns which wire it runs on:

```rust,ignore
async fn scenario(client: &impl Greeter) { … }
scenario(&Memory::default()).await;            // in process
scenario(&local.establish::<GreeterClient>().await?).await;   // vox
scenario(&GreeterHttpClient::at(base)).await;  // HTTP
```

A transport failure arrives through the error type's `From` — for
`RepoError` that is `Internal`. For your own errors, `#[architect::error]`
writes all of it: the wire derives, the `HttpError` impl (status by
variant name or `#[architect(http_status = …)]`, code from the variant name
or `code = "…"`), and a `Transport(String)` variant with the `From`.

## The wire

By default every method is `POST /<prefix>/<method-kebab>` with a JSON
object of named arguments (other verbs and path parameters: see
[Routes](#routes-verbs-path-parameters-body-limits)). The prefix is the trait name in kebab-case minus a
trailing `Service` — `AuthService` → `/auth`, `OrganizationService` →
`/organization`, `Greeter` → `/greeter` — or whatever
`#[architect::service(path = "…")]` says:

```text
POST /greeter/greet
content-type: application/json

{"name": "cody"}

200 OK
"hello cody"
```

- A method with no arguments takes `{}` or no body at all.
- A missing `Option<T>` argument reads as `None`.
- An argument named `token` may arrive as `Authorization: Bearer …`
  instead of the body — the body wins when it says something, the header
  fills in when it does not. A browser keeps the credential in a header,
  as a vox client keeps it in metadata.
- `Result<T, E>` replies `200` + `T`, or the error's status and

  ```json
  {"code": "unknown", "message": "nobody is called x", "error": {"Unknown": "x"}}
  ```

  `E` says how it maps by implementing `architect::http::HttpError`
  (`status()`, `code()`; the defaults are `400` / `"error"`). `error` is
  the typed `E` itself, so the generated client hands back the real enum.
- A body that does not decode is `422` with the same envelope shape.

`http::PATHS` lists every `(method, path)` pair the router mounts, for
tests and docs.

## Routes: verbs, path parameters, body limits

`POST` with a JSON body is the default, not the only shape. A method's
`#[http(…)]` names its verb, its path and its body cap; the service's
`path` may nest and hold parameters too:

```rust,ignore
#[architect::service(path = "org/{slug}")]
pub trait Notes {
    #[http(get, path = "notes/{id}")]                 // GET    /org/{slug}/notes/{id}
    async fn note(&self, slug: String, id: u32) -> Result<String, NoteError>;
    #[http(put, path = "notes/{id}")]                 // PUT    /org/{slug}/notes/{id}  {"text": …}
    async fn set_note(&self, slug: String, id: u32, text: String) -> Result<(), NoteError>;
    #[http(delete, path = "notes/{id}")]              // DELETE /org/{slug}/notes/{id}
    async fn drop_note(&self, slug: String, id: u32) -> Result<(), NoteError>;
    #[http(get, path = "files/{*path}")]              // GET    /org/{slug}/files/a/b.md
    async fn file(&self, slug: String, path: String) -> Result<String, NoteError>;
    #[http(post, path = "bulk", limit = "4MiB")]      // a JSON body past the 2 MiB default
    async fn bulk(&self, slug: String, text: String) -> Result<u64, NoteError>;
}
```

- Verbs: `get`, `put`, `post` (the default), `delete`, `any`. Several
  verbs may share a path; each reaches its own method.
- `{name}` is one segment and `{*name}` the rest of the path (last
  segment only, slashes kept). Each binds the argument of the same name:
  parsed with `FromStr` (a `400` with code `invalid_path` when it does not
  parse) and written by the client with `Display`, percent-encoded
  (`architect::http::encode_segment` / `encode_rest`). A `{param}` in the
  service `path` binds that argument on every method, so every method
  must take it. The derive refuses a parameter that names no argument.
- The other arguments are the JSON body, as before. A `get` route has no
  body (a browser's `fetch` will not send one), so all its arguments come
  from the path, or the bearer header for `token`; the derive says so at
  compile time otherwise.
- `limit = "1GiB"` raises the route's body cap: axum's `DefaultBodyLimit`
  on that route (units `B`, `KB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`).
  Without it the cap is axum's 2 MiB (`architect::http::DEFAULT_BODY_LIMIT`),
  and a bigger body is a `413`.
- These methods keep their vox face and their typed clients: the HTTP
  client fills in the path and sends the verb, and `http::PATHS` lists
  the templates (`/org/{slug}/notes/{id}`).
- A method with none of this keeps the exact `POST` route it always had.
  `crates/architect/tests/http_routes.rs` drives all of it over a socket.

## Bytes: uploads and ranged downloads

Media, blob downloads and uploads are bytes, not JSON. A method declares
that with three types from `architect::http`
(`crates/architect/src/http/body.rs`):

| type | where | what the handler does |
|---|---|---|
| `BodyStream` | argument | hands over the request body as a stream of chunks, capped at the route's `limit` |
| `RequestHead` | argument | hands over the method, URI, headers and path params (`bearer()`, `header(..)`, `query()`, `param(..)`) |
| `ByteBody` | `Ok` value (or the return) | serves it through `ranged::serve_body`: `200`, `206` + `Content-Range`, or `416` |

```rust,ignore
#[architect::service(path = "org/{slug}")]
pub trait Media {
    #[http(get, path = "blobs/{hash}")]
    async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError>;

    #[http(get, path = "media/{*path}")]
    async fn media(&self, slug: String, path: String, head: RequestHead)
        -> Result<ByteBody, MediaError>;

    #[http(put, path = "blobs/upload", limit = "1GiB")]
    async fn upload(&self, slug: String, body: BodyStream) -> Result<BlobRef, MediaError>;
}

impl Media for Store {
    async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError> {
        let (total, mime) = self.stat(&slug, &hash).await?;
        let store = self.clone();
        // Called once, with exactly the window the `Range` selects.
        Ok(ByteBody::new(total, mime, move |offset, len| async move {
            store.read_range(&slug, &hash, offset, len).await.map(Body::from)
        }))
    }
    …
}
```

- `ByteBody::new(total, mime, read)` takes a windowed reader;
  `ByteBody::from_bytes(bytes, mime)` serves memory (a range shares the
  buffer); natively `ByteBody::file(path, mime)` streams a file. A failed
  read is a `500`; an `Err(E)` from the method is the usual envelope.
- `BodyStream` is a `futures::Stream` of `io::Result<Bytes>` (or call
  `chunk()` / `to_vec()`). A declared `Content-Length` over the route's
  cap is a `413` before the method runs; a chunked body that grows past
  it ends the stream with `io::ErrorKind::FileTooLarge`, which the method
  maps to its own error. Its other arguments come from the path.
- Path parameters, verbs and `limit` work exactly as for JSON methods.

**These methods are HTTP-only.** vox has no byte-stream argument to carry
them, and a mapping onto vox channels would be a second wire protocol to
keep in step for every route. So a trait with any such method is an
*HTTP-only service*: no vox client, dispatcher, descriptor or `serve`; no
typed client method for the byte methods (any HTTP client calls them;
its JSON methods keep theirs); and none of `context`, `ops`,
`sync_client`, `actions` or `scopes` (the derive says which). Keep the
methods a vox client needs in their own trait. Every method is
`async fn`, and the HTTP face calls the backend directly. Its `Service`
token is still a `Layer`, so it mounts like any other:

```rust,ignore
let http = layers![MediaService, NotesService].provide_http(&backend);
EngineHost::new(vox, bind).plugin(http).serve().await?;
```

On vox the token binds nothing (an empty descriptor, which
`LayerRouter::unmounted` ignores). `crates/architect/tests/http_bytes.rs`
uploads 3 MiB through `limit = "1GiB"`, refuses one past the default cap
(declared and chunked), downloads by range, and reads the bearer through
`RequestHead` on a `{*path}` route, all over a socket.

## Streams

A `#[subscribe]` declaration gets a route too — as **Server-Sent
Events**. `POST /<prefix>/<name-kebab>` (GET works for filter-less
streams) with the filters as the JSON body answers `text/event-stream`,
one `data: <json>` frame per event, until the client drops the
connection:

```rust,ignore
#[architect::service]
pub trait Greeter {
    #[subscribe] fn ticks(&self) -> u32;                 // POST /greeter/ticks
    #[subscribe] fn ticks_over(&self, min: u32) -> u32;  // POST /greeter/ticks-over  {"min": 5}
}

let mut ticks = client.ticks().await?;                   // EventStream<u32>
while let Some(tick) = ticks.next().await { … }          // drop it to unsubscribe
```

The backend contract is the same one vox uses: `<name>_hub()` returns
the `PubSub`, or `<name>_attach(filters…, sink)` hands a sink to
whatever filters. The sink is an `architect::EventSink` — a vox `Tx`
or a local channel — so one backend serves both wires; the hub sees the
HTTP subscriber close exactly as it sees a vox one.

`http::stream_router(backend)` mounts the streams; `StreamService`
binds them, so `layers![Service, StreamService].provide_http(&backend)`
mounts request/reply and streams together.

## Entities

`#[derive(architect::Entity)]` with `repo` emits the same face for the
repository — `<entity>_http::router(backend)`, `impl BindHttp for
<Entity>RepoLayer`, and `<Entity>RepoHttpClient`:

```text
POST /widget/get      {"id": …}
POST /widget/list     {"page": {"index": 0, "size": 50}, "sort": null, "filter": null}
POST /widget/create   {"input": {…}}
POST /widget/update   {"id": …, "input": {…}}
POST /widget/delete   {"id": …}
```

With `events`, `POST /widget/events` streams the feed the vox
`<Entity>Events::subscribe` serves — a `Snapshot` of the current rows
first, then every `Upserted` / `Deleted` — through
`<entity>_http::events_router(evented)`, `BindHttp for <Entity>EventsLayer`,
and `<Entity>EventsHttpClient::subscribe()`. The prefix is the entity
name in kebab-case, or `#[architect(path = "widgets")]`.

## Mounting

A service's HTTP face is one router; a bundle of services binds the
same way it binds for vox — and a mock merged over the bundle replaces
its routes exactly as it replaces its vox handlers:

```rust,ignore

let svc = AuthVoxService::new(auth);
let vox  = layers![AuthServiceService, OrganizationServiceService].provide(svc.clone());
let http = layers![AuthServiceService, OrganizationServiceService].provide_http(&svc);

EngineHost::new(vox, "0.0.0.0:8080")
    .plugin(http)                  // the generated face
    .plugin(oauth::router(state))  // the escape hatch: a hand-built router
    .plugin(auth_ui::router(ui))   // hosted pages
    .finish(|app| app.layer(cors)) // outermost layers
    .serve()
    .await?;
```

`EngineHost` serves `/vox` (echoing the `vox.v1` subprotocol), `/health`,
`/version.json`, every plugin, an optional SPA bundle, and — with
`.iroh(key, id)` — the same vox router peer to peer. `into_app()` hands
back the axum `Router` for in-process tests.

### Protocol mounts

Some endpoints are adapters, not capabilities: a vox WebSocket per org,
a WebDAV bridge, a discovery document. They belong to the host the way
`/vox` does, so each has a builder method instead of a hand-written
route (`crates/architect/src/host/mount.rs`):

```rust,ignore
EngineHost::new(vox, bind)
    // A vox WebSocket at a parameterised path. The resolver picks the
    // handler (and so the identity) per connection, or refuses it.
    .vox_at("/org/{slug}/vox", move |head: RequestHead| {
        let orgs = orgs.clone();
        async move {
            let org = orgs.get(head.param("slug").unwrap_or_default())
                .ok_or(StatusCode::NOT_FOUND)?;
            Ok(org.router_for(head.bearer()))
        }
    })
    // A GET document: discovery, a per-org API description, a probe.
    .document("/.well-known/app.json", move |head| discovery(head.bearer()))
    // WebDAV: every verb, outside the `finish` layers.
    .dav("/org/{slug}/dav", move |dav: DavRequest| files.handle(dav))
    .finish(|app| app.layer(cors))
```

| Method | Serves | The hook gets | The hook answers |
|---|---|---|---|
| `vox_at(path, resolve)` | a vox WebSocket at `path` (`any` verb; `vox.v1` echoed) | `RequestHead` | `Ok(handler)` (a `LayerRouter` or a gated wrapper), or `Err(impl IntoResponse)` to refuse the upgrade |
| `document(path, describe)` | `GET path`, before the SPA fallback | `RequestHead` | `Ok(impl IntoResponse)` or `Err(impl IntoResponse)` |
| `dav(mount, handle)` | every verb at `mount`, `mount/` and below it, **outside** `finish` | `DavRequest`: `head`, `mount` as addressed (`/org/acme/dav`), `rel` (decoded, no leading `/`), the whole `request` | a `Response` |

- `RequestHead` (`crates/architect/src/http/body.rs` `RequestHead`, also
  exported as `architect::host::RequestHead`) is the same type a byte
  method takes as an argument. It carries the matched route's path params (`param("slug")`, decoded),
  the headers and the path. `bearer()` reads `Authorization: Bearer …`,
  then the `vox.bearer.<token>` subprotocol a browser dial sends.
- The resolver runs before the upgrade is accepted, so a refusal is a
  plain HTTP status the client sees, not a dropped socket.
- `vox_at("/vox", …)` replaces the built-in `/vox`, for a host whose root
  lane is resolved per connection too (a single-org client's legacy URL).
  The router given to `EngineHost::new` then serves only iroh peers.
- `dav` sits outside the `finish` layers on purpose. A CORS layer answers
  every `OPTIONS` itself, and a file manager opens a mount with a bare
  `OPTIONS` and reads `DAV:` and `Allow:` off the reply. WebDAV clients
  are not browsers, so the handler authenticates them itself.
- Two more mounts live behind features. `.otlp_forward(at, upstream,
  authorize)` (architect feature `otlp-forward`,
  `crates/architect/src/host/otlp.rs`) accepts
  `POST {at}/v1/{traces,logs,metrics}` and forwards each body, with its
  content type, to the collector at `upstream`, once `authorize(head)`
  says `Ok`. Clients reuse the host's auth and TLS instead of reaching a
  public collector. Any other signal is `404`, and an unreachable
  collector is `502`. `.mcp(…)` / `.mcp_at(path, resolve)` come from
  `architect_mcp::host::McpHost` (feature `host`), an extension trait,
  because architect cannot depend on its MCP crate. See
  [commands](@/architecture/commands.md).
- `.plugin(Router)` stays as the escape hatch, for what neither a service
  trait nor a builder method covers yet.

`crates/architect/tests/host_mounts.rs` drives all three: a real
upgrade per bearer and org, WebDAV verbs past a CORS-like layer, and
documents ahead of the SPA. `examples/greeter/tests/host_mounts.rs`
drives MCP and OTLP forwarding against a fake collector.

### `/version.json`

`GET /version.json` says which build is running:

```json
{"rev": "32997aa…", "version": "0.4.2", "built_at": "2026-10-22T08:00:00Z"}
```

It is a real route mounted beside `/health`, so a SPA bundle's
`index.html` fallback can never answer it, and it is sent with
`Cache-Control: no-store`. The values come from the binary's build-time
env through `architect::build_info!()`, expanded in the **binary** crate:

| Field | Source (read at compile time) |
|---|---|
| `version` | the binary crate's `CARGO_PKG_VERSION` |
| `rev` | `ARCHITECT_BUILD_REV`, e.g. `git rev-parse HEAD` |
| `built_at` | `ARCHITECT_BUILT_AT`, RFC 3339 UTC, e.g. `date -u +%Y-%m-%dT%H:%M:%SZ` |

```rust,ignore
EngineHost::new(vox, bind)
    .build_info(architect::build_info!())
    .serve()
    .await?;
```

```sh
ARCHITECT_BUILD_REV=$(git rev-parse HEAD) \
ARCHITECT_BUILT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ) \
cargo build --release -p example-greeter
```

A variable that is unset or empty is `null` in the JSON, never a guess;
without `.build_info(..)` every field is `null`. Cargo tracks both
variables, so changing one rebuilds the crate that expands the macro.
Don't mount a plugin at `/version.json`: axum refuses the duplicate route
at startup. `crates/architect/tests/version_json.rs` asserts the JSON and
that an unknown path still reaches the SPA.

By default the iroh endpoint finds peers through n0's relays and DNS,
which need the internet. With the `iroh-lan` feature,
`.iroh_lookup(PeerLookup::N0AndLan)` adds mDNS-style lookup on the local
network beside n0, so two machines on one LAN dial each other by bare
endpoint id with the internet down; `PeerLookup::Lan` stays on the LAN
only (relays off, nothing published to n0). Code that builds its own
endpoint takes the same choice through `iroh_link::endpoint_builder` /
`bind_endpoint_with`. `crates/architect/tests/iroh_lan.rs` proves the
bare-id dial on one host; `crates/architect/examples/iroh_lan.rs` is the
two-machine check.

## The client

```rust,ignore
let client = GreeterHttpClient::at("https://api.example.com").with_bearer(token);
let hi: Result<String, ClientError<GreetError>> = client.greet("cody".into()).await;
```

Every method returns `Result<T, ClientError<E>>` — the same envelope the
vox client's `VoxError<E>` folds into — so a screen written against one
transport reads errors identically on the other. Several clients share
one `architect::http::HttpClient` (base URL, bearer, connection pool),
the way typed vox clients share one `Caller`. Native and wasm (`fetch`).

## Testing every face at once

`apps/auth-server/tests/surfaces.rs` is the pattern: a small adapter
over the generated clients, and each scenario run over HTTP, vox
in-process (`LocalServer`), vox over WebSocket (`architect::connect`),
and vox over iroh (loopback). A behaviour that holds on one transport
and not another is a bug in the framework's face, not in the engine —
which is exactly what a per-transport hand-written test never notices.

## Byte ranges (`Range` → 206)

A method that returns a `ByteBody` gets all of this without asking (see
above). The rules live once in `architect::http::ranged`
(`crates/architect/src/http/ranged.rs`), and a route built outside the
derive uses the same helpers:

| `Range` | answer |
|---|---|
| absent, malformed, another unit, several ranges | `200`, whole body |
| `bytes=a-b`, `bytes=a-`, `bytes=-n` that selects bytes | `206` + `Content-Range: bytes a-b/total` |
| a well-formed `bytes=` spec that selects nothing | `416` + `Content-Range: bytes */total` |

- `parse_range(raw, total) -> ByteRange` is pure and builds in every
  configuration, wasm included. `ByteRange::window(total)` is the
  `(offset, len)` the body must carry.
- Under `http`: `serve_body(&headers, total, mime, |offset, len| …)` calls
  your reader once with exactly the selected window (never for a `416`),
  so a blob or object store reads only those bytes; `serve_bytes` slices an
  in-memory `Bytes`; `respond(range, total, mime, body)` is the response
  assembly on its own.
- Natively: `serve_file(&headers, &path, mime)` streams a file from disk,
  and `reader_body(reader)` turns any `tokio::io::AsyncRead` into a
  streamed body (`.take(len)` to stop at the range's end).

```rust,ignore
async fn download(headers: HeaderMap, Path(id): Path<String>) -> Result<Response, StoreError> {
    let (total, mime) = store.stat(&id).await?;
    serve_body(&headers, total, &mime, |offset, len| async move {
        store.read_range(&id, offset, len).await.map(Body::from)
    })
    .await
}
```

**Against `tower_http::services::ServeFile` / `ServeDir`.** For a file
on disk mounted at a path with nothing of yours in front of it (the SPA
bundle `EngineHost` serves, a static asset dir), use `tower-http`: it
also answers `HEAD`, conditional requests (`If-Range`, `If-None-Match`,
`If-Modified-Since` with `ETag` / `Last-Modified`), guesses the content
type and serves precompressed variants, none of which `ranged` does.
Reach for `ranged` when the bytes do not come from a path `ServeFile` can
open (a content-addressed blob store, S3, a database, a CRDT attachment),
when the route has to authorize or look the resource up first and wants
to set the content type itself, or when the total length is known before
the bytes are. `serve_file` is for the in-between case: a file whose path
the handler computes, served with the same response shape as the
store-backed routes.

## Not yet

- Traits with an ambient `context = T` have no HTTP face: there is no
  transport-neutral way to pull `T` out of a request yet.
- Arguments come from the path, the bearer header or a JSON body, never
  a query string; a `#[subscribe]` stream cannot sit under a service
  `path` with parameters.
- `ranged` serves one range per request (no `multipart/byteranges`) and
  checks no validators: an `If-Range` that no longer matches still gets
  the range.
- Streams flow server → client only, as vox channels do; there is no
  client-push channel over HTTP.
