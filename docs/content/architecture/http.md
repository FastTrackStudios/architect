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
`RepoError` that is `Internal`; a feature's own error usually adds an
`Unreachable(String)` variant.

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
every plugin, an optional SPA bundle, and — with `.iroh(key, id)` — the
same vox router peer to peer. `into_app()` hands back the axum
`Router` for in-process tests.

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

## Not yet

- Traits with an ambient `context = T` have no HTTP face: there is no
  transport-neutral way to pull `T` out of a request yet.
- Streams flow server → client only, as vox channels do; there is no
  client-push channel over HTTP.
