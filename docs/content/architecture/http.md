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

Every method is `POST /<prefix>/<method-kebab>` with a JSON object of
named arguments. The prefix is the trait name in kebab-case minus a
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
    .plugin(oauth::router(state))  // anything that is HTTP by nature
    .plugin(auth_ui::router(ui))   // hosted pages
    .finish(|app| app.layer(cors)) // outermost layers
    .serve()
    .await?;
```

`EngineHost` serves `/vox` (echoing the `vox.v1` subprotocol), `/health`,
`/version.json`, every plugin, an optional SPA bundle, and — with
`.iroh(key, id)` — the same vox router peer to peer. `into_app()` hands
back the axum `Router` for in-process tests.

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

Media and downloads are HTTP by nature and stay hand-written routes, but
the byte-range rules under them are not app code. `architect::http::ranged`
(`crates/architect/src/http/ranged.rs`) has them once:

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
- `ranged` serves one range per request (no `multipart/byteranges`) and
  checks no validators: an `If-Range` that no longer matches still gets
  the range.
- Streams flow server → client only, as vox channels do; there is no
  client-push channel over HTTP.
