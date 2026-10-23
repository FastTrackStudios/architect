# RPC, services, layers, hosts, transports

Entity/derive, storage, the permissions gate, crdt/atom and the Action List have their own references; only the seams are named here. Guides: `docs/content/architecture/http.md`, `docs/content/architecture/streams.md`, `docs/content/architecture/construction.md`.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `#[architect::rpc]` / `service` / `http` | one trait → vox service, client, host, HTTP router, HTTP client, SSE streams, optionally Action List projection | `crates/architect-rpc-derive/src/lib.rs` `rpc`, `service`, `http` |
| `#[subscribe]` streams | sync marker → `<T>Stream` sibling + `<T>StreamSource` backend contract | `crates/architect-rpc-derive/src/lib.rs` `subscribe` |
| Options | `sync_client`, `context`, `scopes`, `ops`, `path`, `actions` on the attribute | `crates/architect-rpc-derive/src/lib.rs` |
| `Layer` / `Bind` / `Services` / `layers!` / `router!` | deferred bundle of service tokens, bound at `.provide(B)` | `crates/architect/src/layer.rs` `Layer`, `Services`, `layers`, `router` |
| `LayerRouter` | method-id dispatch, last-merge-wins, mount coverage, `gated(&gate)` | `crates/architect/src/layer.rs` `LayerRouter`, `unmounted_from` |
| Instance scopes | same trait mounted per `svc-scope` | `crates/architect/src/scoped.rs` |
| `LocalServer` | serve a router over a vox memory link, in-process | `crates/architect/src/local.rs` `LocalServer` |
| `EngineHost` | axum host: `/vox` WS, `/health`, `/version.json`, plugins, SPA, optional iroh | `crates/architect/src/host.rs` `EngineHost`, `BuildInfo` |
| `host::boot` / `block_on` / tracing | tokio runtime, fmt tracing, panic logger | `crates/architect/src/host.rs` `boot`, `block_on`, `init_tracing`, `install_panic_logger` |
| `connect::pool` | shared, single-flight, liveness-checked connections keyed by (target, bearer) | `crates/architect/src/connect/mod.rs` `pool`, `caller_with`, `evict_all` |
| `connect::ws` | WebSocket dial; native header / browser subprotocol bearer | `crates/architect/src/connect/ws.rs` |
| `axum_ws` | server half of vox over an axum WebSocket | `crates/architect/src/axum_ws.rs` |
| `iroh_link` | vox over QUIC, bare `EndpointId` dial, ALPN `architect/vox/1`, LAN lookup | `crates/architect/src/iroh_link.rs` `connect`, `PeerLookup`, `bind_endpoint_with` |
| HTTP face (server/client) | `POST /<prefix>/<method>` JSON, SSE; `<Trait>HttpClient` (reqwest, wasm fetch) | `crates/architect/src/http.rs` `HttpError`, `respond`, `sse`, `HttpClient` |
| Error envelope | `TransportError`, `ClientError<E>`, `vox_into_app` | `crates/architect/src/lib.rs` `TransportError`, `ClientError`, `vox_into_app` |
| `PubSub` / `EventSink` | server fan-out with overflow strategies, replay, buffered attach | `crates/architect/src/pubsub.rs` `PubSub` |
| `schedule` | `Schedule` policies, `retry`, `repeat` | `crates/architect/src/schedule.rs` `Schedule`, `retry`, `repeat` |
| `reconnect` | `Backoff` (full jitter), `use_connect_supervised` | `crates/architect/src/reconnect.rs` `Backoff`, `use_connect_supervised` |
| `ops` / `sync_client` | reified sync calls; blocking facade | `crates/architect/src/ops.rs`, `crates/architect/src/sync_client.rs` `BlockingCaller` |

## 1. Define the service

```rust
// examples/greeter/src/lib.rs (abridged)
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]
    Unknown(String),
}

#[architect::service(actions)]
pub trait Greeter {
    /// Greets a known name.
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    #[action(effect = read_only)]
    async fn count(&self) -> Result<u32, GreetError>;
    #[subscribe]
    fn greeted(&self) -> String;
}
```

- `#[architect::service]` = rpc + HTTP; `#[architect::rpc]` is vox only; `(actions)` adds the Action List projection (`references/commands-mcp.md`).
- All-async traits need no dispatcher; sync or mixed ones need `HasDispatcher` (`#[derive(architect::HasDispatcher)]`).
- Subscriptions are sync markers; `async fn` is rejected.
- Generated tokens come through `prelude` with the trait name baked in: `GreeterService`, `greeter_layer`, `greeter_serve`, `GreeterStreamService`, `greeter_http`, `greeter_actions`.

## 2. Implement the backend, then bundle it

```rust
impl Greeter for Memory { /* async fns */ }
impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &PubSub<String> { &self.greeted }
}
impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}
```

- A `StreamSource` provides `<name>_hub` (no filter args) or `<name>_attach(filter, sink)` (with args).
- `layers![]` terminates in `Empty` and cons-chains via `Append`.

## 3. Mount on an EngineHost

```rust
// examples/greeter/src/main.rs
EngineHost::new(backend.clone().into_router(), config.bind)
    .plugin(backend.into_http_router())
    .build_info(architect::build_info!())
    .serve()
    .await?;
```

- `into_router()` = vox `LayerRouter`; `into_http_router()` = axum router.
- `/health` and `/vox` (WS, subprotocol `vox.v1`) are always mounted; `/version.json` (`{rev, version, built_at}`, `no-store`) is mounted before the SPA fallback.
- `plugin` is an alias of `extend`; plugins merge before the SPA fallback. `finish(wrap)` wraps the assembled app after plugins and the SPA fallback. `plugin` is the escape hatch: its routes still count as `hand-routes`.
- Protocol mounts (`crates/architect/src/host/mount.rs`), each hook seeing a `RequestHead` (the same `architect::http::RequestHead` a byte method takes: path params, headers, `bearer()` from `Authorization` or the `vox.bearer.` subprotocol):
  - `.vox_at("/org/{slug}/vox", |head| async { Ok(handler) / Err(impl IntoResponse) })`: vox WS at a parameterised path; the resolver picks the handler (identity) per connection before the upgrade. `vox_at("/vox", …)` replaces the built-in `/vox`.
  - `.document(path, |head| async { Ok(impl IntoResponse) / Err(impl IntoResponse) })`: a `GET` document (`/.well-known/…`, `/org/{slug}/api`, a per-org probe).
  - `.dav(mount, |dav: DavRequest| async { response })`: WebDAV, every verb at `mount`, `mount/` and below, mounted outside `finish` (outside CORS); `DavRequest { head, mount, rel, request }`.
  - Tests: `crates/architect/tests/host_mounts.rs`.
- `serve()` binds TCP and spawns iroh if configured: `.iroh(key_path, id_path)` + optional `.iroh_lookup(PeerLookup::N0AndLan)`. Real use: `apps/auth-server/src/server.rs`. The key file is created 0600 on unix.

## 4. Connect

- Native vox: `let caller = pool().caller(&Endpoint::url(&vox_url).with_bearer(token)).await?;` then `TaskServiceClient::new(caller.clone())`.
- In-process: `LocalServer::serve(router, scope).establish::<GreeterClient>()` (`examples/greeter/tests/round_trip.rs`).
- iroh: `iroh_link::connect(&endpoint, remote_id)` then `vox_core::initiator_on(link).establish::<C>()`; through the pool with `Pool::caller_with(endpoint, dial)` (Task keys iroh peers `iroh://<id>`).
- HTTP: `GreeterHttpClient::at(base)`; streams are `client.greeted().await`.
- wasm: the same `pool()`, the browser dial carries the bearer as subprotocol `vox.bearer.<token>`; the HTTP client's `EventStream` drops `Send`; `LocalServer` uses architect's own memory link; iroh is client-only (relay dial); `EngineHost` is native (`host` feature).
- For an all-async trait whose methods all return `Result`, both `<Trait>Client` and `<Trait>HttpClient` implement the trait: delete any hand-written `impl Trait for TraitClient`.

## 5. HTTP face: what it can and cannot express

Can:
- Route `POST /<prefix>/<method-kebab>` by default; prefix = trait minus `Service`, kebab-cased; `path = "org/{slug}"` overrides it (may nest, may hold `{param}`s); `#[http(path = "sign-in/email")]` sets the method tail.
- `#[http(get | put | post | delete | any, path = "notes/{id}", limit = "1GiB")]`: verb, path, body cap (axum `DefaultBodyLimit`; default 2 MiB). `{name}` / `{*name}` (last segment) bind the same-named argument via `FromStr`, `400 invalid_path` if it does not parse; a service-prefix `{param}` must be an argument of every method. Several verbs may share a path. The typed client percent-encodes params (`architect::http::encode_segment`, `encode_rest`) and sends the verb. Test: `crates/architect/tests/http_routes.rs`.
- Body: one JSON key per non-path argument (`http::<Method>Args`); empty body = `{}`; content type not checked. A `get` route has no body: every argument comes from the path (or `token` from the bearer), else a compile error.
- Bytes: a `BodyStream` argument (streamed upload, capped at `limit`; `413` on a declared length over it, `io::ErrorKind::FileTooLarge` mid-stream), a `RequestHead` argument (`bearer()`, `header()`, `query()`), a `ByteBody` result (`ByteBody::new(total, mime, |offset, len| …)`, `from_bytes`, `file`; served via `ranged::serve_body`, so `206`/`416` for free). Any of these makes the trait an **HTTP-only service**: no vox client/dispatcher/descriptor, no typed client method for byte methods, all `async fn`, no `context`/`ops`/`sync_client`/`actions`/`scopes`. Its `Service` token still mounts with `provide_http`. Test: `crates/architect/tests/http_bytes.rs`.
- `200` + JSON on success; errors via `HttpError::status()` (default 400, clamped ≥ 400), body `{code, message, error}`; the client decodes `ClientError::App(E)`.
- An argument named `token` may come from `Authorization: Bearer`.
- `#[subscribe]` → SSE `text/event-stream`, POST and GET. An argless subscription delivers its `{name}_intro` frame first over SSE (fixed in v0.9.24), as the vox stream host does.

Cannot:
- Query strings as arguments (read them via `RequestHead`); multipart; custom success status (always 200). `#[subscribe]` under a service `path` with params. A byte method on vox (split vox methods into their own trait).
- Auth beyond `Bearer` / `token`: no cookie, no per-route guard. Use `plugin` / `finish` tower layers.
- `context = T` traits: the HTTP face (and the action projection) is skipped.
- Generic methods, `&mut self`, borrowed or `Self` returns: compile errors.
- Typed errors on streams: a non-2xx is a transport error. Browser `EventSource` works only for zero-arg streams (GET has no body).

## 6. Layered services

- A service token is a one-element layer; `merge` conses onto the front; `.provide(B)` binds and builds the `LayerRouter`. Missing `Bind<B>` impls surface at `.provide(...)`.
- Duplicate method id: last merged wins. This is how mocks override (`crates/architect/tests/layer_override.rs`).
- `LayerRouter::unmounted_from::<B>()` lists services a backend declares but the router does not mount; `PermissionsGate::coverage` is the cross-check.
- `router![...]` merges several backends' `into_router()`.

## 7. Proof per wire (copy these tests)

| Wire | Test |
|---|---|
| in-process, vox memory link, HTTP, SSE, mock override | `examples/greeter/tests/round_trip.rs` `in_process`, `over_vox`, `over_http`, `streams_over_http`, `a_mock_replaces_the_real_greeter`; `crates/architect/tests/http_face.rs` |
| named MCP = HTTP | `examples/greeter/tests/mcp.rs` `greet_over_mcp_is_greet_over_http` |
| vox over WebSocket and iroh, one suite | `apps/auth-server/tests/surfaces.rs` `websocket_vox_stack`, `iroh_vox_stack` |
| vox over iroh | `crates/architect/tests/iroh_link.rs` `vox_connection_over_iroh` |
| iroh LAN, bare id | `crates/architect/tests/iroh_lan.rs` (needs a multicast interface) |
| reconnect | `crates/architect/tests/reconnect_supervisor.rs` |
| dispatch span | `crates/architect/tests/rpc_spans.rs` `dispatch_span_names_the_method` |
| scopes | `crates/architect/tests/scoped_router.rs` |
| pool single-flight | `crates/architect/src/connect/mod.rs` `concurrent_callers_share_one_dial` |

## Pitfalls

- **vox revision match.** Client and server pin the same vox rc or the handshake fails ("schema decode consumed N of M bytes"). Pin in the root manifest, not per crate.
- **One error enum per service.** Generated clients implement the trait only when it is all-async, every method is fallible, and the error absorbs `TransportError` via `From`. Transport failures are never application errors.
- **Pool keys include the bearer**, so a signed-in socket is never anonymous. Concurrent callers share one dial. Call `evict_all()` on sign-in/out.
- **Reconnect is lazy.** The pool drops a dead root on the next `caller()`; a held dead `Caller` fails. The supervised hook needs `atom` + `platform`; `use_app_pooled` evicts before redial. dioxus `use_resource` tracks signal reads inside the future: drive loops read `generation_now()`.
- **Backoff:** full jitter, floor 250 ms, cap 10 s, reset on success; one `warn` per outage, `debug` after.
- **Features.** Greeter needs `host`, `http-client`, `local`. `host` → `server-axum` → `telemetry` → `vox`. LAN needs `iroh-lan`. The `rpc` span needs `telemetry` (inert without a subscriber).
- **PubSub never blocks the publisher.** Overflow drops (sliding/dropping); no back-pressure. Never publish from an audio thread.
- **`BlockingCaller` deadlocks** if used inside an async context.
- `examples/greeter` is the only example; any doc that names another example directory is stale.
