+++
title = "Idioms & enforcement"
description = "The bearcove/vox + Dioxus conventions architect follows, and what keeps them honest."
weight = 50
+++

architect's client/server story is opinionated. These are the rules the
example app (`examples/app/`) follows, why they exist, and how each one
is kept from drifting. Copy them into your own project.

## 1. All client↔server data flows through vox services

Every read and write goes through a `#[vox::service]` trait — the
architect-emitted `<Entity>Repo` (CRUD) or a hand-written `<Entity>Service`
(domain ops like search/duplicate). The client establishes a typed
`<Trait>Client` over a WebSocket; the server mounts the matching
`Dispatcher`. There is **one** wire schema, generated from the trait.

**Never use Dioxus server functions** (`#[server]`, `use_server_future`,
`dioxus-fullstack`). They'd introduce a second, parallel RPC mechanism
with its own serialization, defeating the facet-only wire.

> Enforced: `cargo xtask idioms` greps `examples/app/` for `#[server]`,
> `use_server_future`, and `dioxus(_|::)fullstack` and fails CI on a hit.

## 2. Wire types are facet-only

`#[derive(architect::Entity)]` emits `facet::Facet` — no `serde` derives
on the wire struct or its `Create`/`Update`/`List` payloads. vox speaks
facet natively, so there's zero per-type glue and no
`#[serde(rename)]` ↔ `#[architect(...)]` drift.

(The CRDT layer's `JsonField` companions *do* use serde — that's a
storage-column concern, not the wire, so it's fine.)

## 3. The client is established once and shared via context

The app root (`examples/app/ui/src/lib.rs`, with the connection machinery
in `transport.rs`) establishes the typed clients **once**, stores them in a
`Signal<ConnState>`, and provides it with `use_context_provider`. The
context *shapes* (`ConnState`, `ExampleClients`, `use_conn`,
`use_example_store`) live with the feature in `example-ui`'s `context.rs`,
so the feature's view-blocks consume them while the shell owns the
transport. Views pull the context with `use_context` (via `use_conn`) and
read data with `use_resource`, which re-runs when the connection flips to
`Ready` or when its inputs change. Don't reconnect per-view; don't block
the first render on the socket.

```rust
let conn = use_context_provider(|| Signal::new(ConnState::Connecting));
use_future(move || async move { /* connect, then conn.set(Ready) */ });
// in a screen:
let results = use_resource(move || async move {
    let clients = match &*conn.read() { ConnState::Ready(c) => c.clone(), _ => return None };
    Some(clients.repo.list(/* … */).await)
});
```

## 4. Navigation is typed; UI primitives come from the catalog

- Navigate with the `Route` enum + `Link { to: Route::… }` / `use_navigator`,
  never a raw `<a href>` — the compiler then catches dead links.
- Reach for the Dioxus component catalog (`dx components add <name>`)
  before hand-rolling a widget (dialog, combobox, drag-to-reorder, …).

## 5. One coherent vox revision across server and clients

The server, CLI, web, and desktop must build against the **same** vox
revision — a mismatch yields a wire-schema translation error at the first
call, or (worse) a silent desync. The example pins one rev in the root
`Cargo.toml` and the client crates reference it verbatim.

> Lesson baked into the e2e suite: upstream vox v0.8.2 couldn't encode
> `Uuid` (no branch for opaque `Def::Scalar` shapes), which compiled fine
> but panicked the server on the first RPC. The
> `app-tests-e2e` suite — real client ↔ real server over a socket — is
> what catches this class of bug. Keep it green.

## What keeps these honest

| Rule | Gate |
| --- | --- |
| No Dioxus server fns (§1) | `cargo xtask idioms` (CI) |
| facet-only wire (§2) | the `Entity` derive emits facet; serde derive would be redundant |
| Excluded crates still build (§3–4) | `cargo xtask ci` checks `ui` / `web` (wasm) / `desktop` |
| Real wire round-trips (§5) | `app-tests-e2e` + `just test-e2e` (browser) |
| Hand-rolled widgets, signal/prop misuse | dioxus-MCP audits — run manually |

The dioxus-MCP audits aren't CLI-runnable, so they're a manual companion
to the automated gates. Run them against `examples/app/ui` when touching
the UI:

- `lint_project` — the full sweep (`check_rsx`, `dead_components`,
  `prop_drill`, `signal_lint`, `props_lint`, `reinvented_widget`,
  `components_audit`, …) in one call.
- `route_map` — sanity-check the route table after editing `Route`.

CI runs `cargo xtask ci`: `fmt --check` → `clippy -D warnings` →
`check --workspace` → target-cfg crate checks → `nextest` →
doctests → `app-ui` smoke test → `idioms` → `tracey validate`. The
browser e2e runs in a separate job (`just test-e2e` / `test-e2e-memory`).

## 6. Composing & building backends — the layer system

Services compose through the `Layer` system; backends are built through
the `Resource` construction layer. Keep the two straight:

- **`Layer<B>` (service binding).** A `#[derive(Entity)]` repo and any
  `#[architect::rpc]` trait emit a `Service` token. Compose tokens with
  `layers![…]` / `.merge(…)`, then `.provide(backend)` (or
  `backend.into_router()`) to get a `LayerRouter`. Backends are
  interchangeable at the `.provide` site — swap the backend, change no
  consumer code (see `examples/layered-services`).

- **`Resource<T, E = eyre::Report>` (backend construction).** A lazy
  builder for the backends a layer binds — `config → pool → repo`:
  - `.and_then(|a| build_b(a))` — dependent build (B from A's output).
  - `.zip(other)` — independent pair. `.map` / `.map_err` — transform.
  - `Resource::acquire_release(acquire, release)` — register cleanup on a
    `Scope`; finalizers run **LIFO** on `scope.close().await` (graceful
    shutdown — see `examples/app/server`, which closes its db pool this
    way on Ctrl-C).
  - `.memoize()` — build once, share across dependents (a pool built once
    for several repos).
  - `.into_router(&scope)` — build a `Services` backend and mount it.

- **`LayerGraph` (planner).** Declare nodes' `requires`/`provides`, call
  `.plan()` for a topological build order or a precise diagnostic
  (`missing-provider` / `conflicting-provider` / `cycle` / `duplicate`).

Full `Resource`/`Scope` walkthrough:
[construction](@/architecture/construction.md).

### Same-trait multi-backend: use a routing backend, not tags

Need two backends of the *same* trait (write→primary, reads→replica;
shard by key)? Don't reach for service tags — vox addresses a service by
name + method-id, identical for two backends sharing a descriptor, so the
router can't route to a specific instance over the wire. Instead, write a
**single backend that holds both and routes internally**:

```rust
#[derive(Clone)]
struct RoutingRepo { primary: PgRepo, replica: PgRepo }
impl ExampleRepo for RoutingRepo {
    async fn get(&self, id: Uuid) -> Result<Example, RepoError> { self.replica.get(id).await }   // reads → replica
    async fn create(&self, input: ExampleCreate) -> Result<Example, RepoError> { self.primary.create(input).await } // writes → primary
    // …
}
```

It impls `ExampleRepo`, so it's `into_router()`-able and serve-able like
any other backend — the routing policy lives in one place instead of
leaking to every caller, and there's no new wire machinery.

These are architect's own DI engine — borrowed from Effect/`id_effect`
(layers, scopes, memoization, a planner) **without** the `Effect<A,E,R>`
monad or typed-requirement tracking. architect stays plain `async fn` +
vox; `Resource` is just value-level builder combinators.

## 7. Inject the transport, not the backend

Screens (and the CLI, and tests) program against the generated
`<Entity>RepoClient` / `<Service>Client` and never hard-code *where* the
backend lives. That's the `Transport`, chosen once at the app root:

- **`Transport::Remote(url)`** — talk to a server over a vox WebSocket
  (`WsLink` + `establish`).
- **`Transport::Local(server)`** — serve a backend **in-process** over a
  vox in-memory link via `architect::serve_local(backend, &scope)` /
  `LocalServer::serve(router, scope)`. No server, no socket.

Same screens, same client types — only the root differs:

| App / mode | Transport | Result |
| --- | --- | --- |
| web (browser) | `Remote(url)` | talks to `app-server` |
| desktop | `Local(service_router(ExampleRepoMemory))` | **whole stack in-process, no server** |
| e2e `local_transport_round_trip` | `Local(…)` | identical asserts to the WS path, no server spawned |

The desktop app (`examples/app/desktop`) is the headline: it serves the
exact `app_server::service_router` (repo CRUD + `ExampleService`) the axum
server mounts, just over an in-memory link — proving remote and local are
behaviour-identical from the UI's point of view. `LocalServer` is native
only (vox's in-memory link isn't compiled for wasm), so the browser stays
remote; `examples/app/ui` cfg-gates the `Transport::Local` arm accordingly.

This is the consumer-facing tip of the layer/construction system: build
the backend with `Resource`/`Scope`, compose its services with `Layer`,
and expose them over whichever transport the deployment needs — all from
one set of screens. Full `LocalServer` walkthrough:
[the in-process transport](@/architecture/local.md).

## 8. Wrap fallible remote connects in `schedule::retry`

A vox connect can fail transiently — the server is still booting, a
WebSocket blips. Don't let that fail the first frame: wrap the
connect+handshake in `architect::schedule::retry` under an
exponential-backoff-with-jitter policy. The example client does exactly
this (`examples/app/ui/src/transport.rs`):

```rust
architect::schedule::retry(
    || establish_ws_once::<C>(url),
    architect::Schedule::exponential(Duration::from_millis(200))
        .max_delay(Duration::from_secs(5))
        .jittered()
        .take(5),
)
.await
```

The policy's clock is platform-portable (`tokio::time` natively, browser
timers on wasm), so the same resilient connect works on web and desktop.
See [scheduling & resilience](@/architecture/scheduling.md) for the full
`Schedule` surface and the `TestClock` testing story.

## 9. Client UI: features own state + presentation, the shell composes

The Dioxus client splits in two: a **feature crate** exposes state hooks
(returning an `AtomResult` phase) + dumb components; the **shell** owns the
transport + routes and composes features by `match`ing the phase. The hard
rule: **Dioxus primitives (`Link`/`nav`) for navigation; the `AtomResult`
phase-match for state** — nothing else. Full guide:
[composing the UI](@/architecture/composing-the-ui.md).

Reads are stale-while-revalidate and writes are optimistic: a
create/update/delete patches a shared `Store<Example>` instantly and only
*reconciles* against the server in the background, rolling back on failure.
**Don't hand-write the CRUD state layer at all** — add `store` to the
entity derive (`#[architect(table_name = …, repo, store)]`) and the typed
hooks, the store, and the optimistic mutations are emitted. Hand-written
hooks (search, live reads) bind the same primitives (`AtomResult`,
`Store` + `use_mutation`, `use_async`, `use_store_entry`,
`use_store_list`). Errors stay **typed** end-to-end (`ClientError<E>`,
never `format!("{e:?}")`); temp rows use the typed `Id::Temp`, never a
magic `"tmp-"` string prefix.

This is the client-side twin of the server-side Layer/Resource DI (§6),
and the same "borrow Effect's ideas without the monad" stance — here
following [effect-atom](https://github.com/tim-smart/effect-atom). Full
walkthrough: [optimistic state](@/architecture/optimistic.md).

> Enforced: unit tests in `features/atom`; the dioxus-MCP
> `magic_id_prefix_for_optimistic` + `optimistic_lock_gate` audits flag the
> anti-patterns the typed `Id<K>` + `use_mutation` lifecycle avoid.

Forms follow the same stance: validate with `architect-form` (typed
`Field`s + a `Form` controller whose submit result is an `AtomResult`),
then submit through the vox client — never a Dioxus server function. Full
walkthrough: [forms](@/architecture/forms.md).
