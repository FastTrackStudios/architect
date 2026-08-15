+++
title = "Optimistic state (architect-atom)"
description = "Client-side optimistic, stale-while-revalidate state for Dioxus — the client twin of architect's server-side Layer/Resource DI, borrowed from effect-atom without the monad."
weight = 54
+++

architect's server side borrows [Effect](https://effect.website/)'s
**Layer / Resource / Scope** DI *without the `Effect<A,E,R>` monad* (see
[idioms §6](@/architecture/idioms.md)). `architect-atom` is the
**client-side twin**: it borrows
[effect-atom](https://github.com/tim-smart/effect-atom)'s `Result` +
writable optimistic atoms — also without the monad — as plain Dioxus
`Signal`s. The result: a UI that feels instant (optimistic
create/update/delete, stale-while-revalidate reads) on top of the same
vox clients, with no second RPC mechanism.

The crate (`features/atom`) is **entity-agnostic and wasm-safe** — its only
dependency is Dioxus core. It knows nothing about vox or any concrete
entity; you bind your own type by implementing `StoreEntity`. It's
re-exported flat at the architect root under the `atom` feature, so
consumers write `architect::Store` / `architect::AtomResult` /
`architect::use_mutation` (the `architect-atom` crate is an implementation
detail).

## Why

The pre-`architect-atom` screens fetched with `use_resource` and mutated
with a raw `spawn { client.create(..).await; nav.push(..) }`. That has
three problems this pattern fixes:

1. **Empty flashes.** `use_resource`'s `Option<Result<T,E>>` drops to
   `None` on every refetch, blanking the list. `AtomResult` keeps the
   previous value while reloading.
2. **No optimism.** A create/delete blocked on the round-trip before the
   UI changed. Now the row appears/vanishes instantly.
3. **No rollback.** A failed mutation left a stale error string and no way
   to revert. Now a failure restores the prior state automatically.

## The three primitives

### 1. `AtomResult<T, E = String>` — stale-while-revalidate state

effect-atom's `Result`, as a Rust enum that **retains the previous value
across a refetch**:

```rust
pub enum AtomResult<T, E = String> {
    Initial,
    Loading,
    Reloading(T),                       // refetch in flight, prev value kept
    Success(T),
    Error  { error: E,      last: Option<T> },   // expected failure (vox/app error)
    Defect { defect: String, last: Option<T> },  // unexpected failure (panic/bug)
}
```

The phases are flat variants — an expected `Error` vs an unexpected
`Defect` (effect's error/defect distinction) — so a page can
`use architect::AtomResult::*` and `match` them unqualified. This is the
idiomatic Rust equivalent of effect's `AsyncResult.matchWithWaiting({
onWaiting, onError, onDefect, onSuccess })` (effect needs an object-matcher
because JS lacks pattern matching; Rust has `match`):

```rust
use architect::AtomResult::*;
match session.state() {
    Initial | Loading => rsx! { p { class: "status", "Opening your upload link." } },
    Reloading(_prev)  => rsx! { p { class: "status", "Refreshing…" } },
    Success(s)        => rsx! { UploadForm { session: s } },
    Error { error, .. }   => rsx! { p { class: "status error", "{error}" } },
    Defect { defect, .. } => rsx! { p { class: "status error", "{defect}" } },
}
```

> A `Defect` is the in-band complement to Dioxus's `ErrorBoundary`: an
> `ErrorBoundary` catches a panic that unwinds out of render; a `Defect`
> is a failure you chose to *carry as data* (`AtomResult::Defect { .. }`)
> so the page can render it inline without tearing down the subtree.

### 2. `use_async` / `Async` — a refreshable async value

The single-resource counterpart (effect-atom's async atom +
`useAtomRefresh`). It wraps `use_resource` and folds it into an
`AtomResult`, retaining the previous value across a refresh. Read a key
signal inside the loader for the "atom family by key" behaviour:

```rust
let session = use_async(move || {
    let token = token();                 // depends on the route param
    async move { client.upload_session(token).await.map_err(|e| format!("{e:?}")) }
});
// session.state() → AtomResult ;  session.refresh() → re-run the loader
```

### 3. `Store` + `Id` + `use_mutation` — the optimistic keyed cache

A `Store<T>` is the collection counterpart (effect-atom's `Atom.family` +
writable optimistic atoms): a keyed, ordered cache of an entity, plus the
backing list-fetch phase and a registry of rollback snapshots. Provide one
at the app root next to the connection:

```rust
// app root
let store = use_store::<Example, String>();
use_context_provider(|| store);
```

Bind your entity once (it lives in the proto crate, behind the client-only
`atom` feature so the server never pulls Dioxus):

```rust
impl StoreEntity for Example {
    type Key = Uuid;
    fn key(&self) -> Uuid { self.id }
}
```

`Id<K>` identifies a row as `Real(K)` or `Temp(u64)` — a **typed** temp id,
never a magic `"tmp-"` string prefix (which the dioxus-MCP
`magic_id_prefix_for_optimistic` audit flags). On reconcile the temp id is
swapped for the server's real one.

`use_mutation` ties the whole optimistic lifecycle into one call, so a call
site can't forget the rollback arm:

```rust
let create = use_mutation::<ExampleClientError>().invalidating(&["examples"]);
let draft = Example::draft(&input);          // client-side placeholder row
create.run(
    store,
    // 1. optimistic patch — runs synchronously, in the handler
    move |s| s.insert_optimistic(draft).0,
    // 2. the server call — Ok(Some) folds the row in, Ok(None) = body-less
    move || async move {
        client.create(input).await.map(Some).map_err(ClientError::from)
    },
);
// 3. reconcile on Ok (temp→real) / rollback + notification + key
//    invalidation on Err — automatic
```

`Store` and `Mutation` are both `Copy` (they wrap `Signal`s), so they move
into `spawn` closures with no `clone()` ceremony — like the existing
`clients_of(conn)` pattern.

## Live data: streams into the store

Reads don't have to poll. The streaming layer is modelled on effect's
`PubSub` + `SubscriptionRef`, adapted to wire subscribers:

- **`architect::PubSub<T>`** (server) — a synchronous, non-blocking
  fan-out hub: `attach` the `vox::Tx` sinks that subscribe RPCs receive,
  `publish` events to all of them. Per-subscriber mailboxes with
  effect-named **overflow strategies** — `sliding(cap)` (drop oldest;
  the default for state-shaped events), `dropping(cap)` (drop incoming),
  `unbounded()` — plus `.with_replay(n)` to hand late subscribers the
  last `n` events. A slow subscriber **never blocks the writer** (the
  back-pressure strategy is deliberately absent — a host's main thread
  can't suspend), and the **buffered attach** protocol
  (`begin_attach` → snapshot read → `complete_attach`) implements
  effect-`SubscriptionRef`'s *current-state-then-changes* without holding
  a lock across the snapshot.
- **`use_stream` / `use_store_stream`** (client) — subscribe a component
  to a server stream for its lifetime; with `use_store_stream` each event
  folds into the optimistic store, so **every store-rendered page is
  live** with zero page changes. Rides `use_resource`, so reading the
  `Connection` inside the subscribe future makes reconnects resubscribe
  automatically.
- **`#[architect(events)]`** on the entity derives the whole CRUD case:
  the `<E>Event` enum (`Snapshot(Vec<E>)` / `Upserted(E)` /
  `Deleted(Key)`), the subscribe trait, the `<E>Evented<R>`
  publish-through wrapper (mount it once; `into_router()` serves CRUD +
  the feed; subscribing delivers the snapshot first, so **the
  subscription alone fully hydrates a client store**), and the client
  hook. Custom streams (positions, meters, presence) hand-write a sibling
  `#[vox::service]` trait over the same `PubSub` + `use_stream`
  primitives.

Two rules: wrap the backend in `<E>Evented` **once** per process (the hub
lives inside; per-connection routers must share it or broadcasts won't
cross sockets), and make event payloads carry **full state**, not diffs —
the changes collected while a snapshot is being read are delivered after
it, so re-application must be idempotent.

## The supporting cast

Four smaller primitives round the layer out:

- **`Connection<C>` / `use_connect` / `use_connection`** — generic
  connection state (Connecting / Ready / Failed) for a typed client
  bundle, provided once at the root. Replaces every feature's hand-rolled
  `ConnState` enum.
- **`ClientError<E>`** — the typed client error envelope: `App(E)` (the
  service's typed error, via `From<VoxError<E>>`) vs `Connect` /
  `Transport { retryable }` infrastructure failures. Hooks return
  `AtomResult<T, ClientError<E>>`, so pages can `match` on
  `App(RepoError::NotFound)`.
- **`Notifications` / `provide_notifications`** — an app-wide queue;
  `Mutation::run` reports rollback failures to it automatically, so an
  optimistic write that navigated away still surfaces its error.
- **`Reactivity` / `provide_reactivity`** — keyed invalidation
  (effect-atom's `Reactivity.invalidate`): loaders `track(key)`, settled
  mutations `invalidate(key)` (wired via
  `use_mutation().invalidating(&[…])`), and every tracking loader
  re-fetches. `AtomResult::zip` / `AtomResult::all` combine several
  resources into one all-or-nothing phase, and `use_interval` gives
  cadence-based re-fetch.

## How it composes with what's already there

The store **wraps** `use_resource`, it doesn't replace it — the
`use_store_list` / `use_store_entry` helpers package the pattern: drive a
fetch through `use_resource` (it re-runs when the connection flips to
`Ready`, a query changes, or a tracked key is invalidated), fold the
outcome into the store, return `entries_result()` for the page to `match`.

Detail/edit screens read cache-first (instant after a list visit) and fall
back to a per-key fetch only on a cache miss — that's `use_store_entry`.

Most apps never touch any of this directly: `#[architect(store)]` on the
entity derives the store binding, the typed hooks, and the optimistic
mutations (see [composing the UI](@/architecture/composing-the-ui.md)).
The primitives are the floor you drop to for feature-specific hooks.

## Three rules that keep it correct

1. **Patch in handlers, fold in effects — never in render.** Optimistic
   writes happen in the event handler; fetch results fold into the store in
   a `use_effect`. Writing signals during render risks an infinite loop
   ([Dioxus: avoid updating state during render](https://dioxuslabs.com/learn/0.7/#avoid-updating-state-during-render)).
2. **Mutations outlive the screen.** `Mutation::run` spawns on
   `spawn_forever`, so a create/delete that navigates away the instant it's
   issued still reconciles or rolls back the shared store. The store and
   its snapshots live at the app root; the per-screen `pending`/`error`
   signals may be gone, so those writes are guarded.
3. **`hydrate` preserves in-flight temp rows.** A list refetch that races
   an optimistic insert won't drop the optimistic row.

## Relationship to dioxus-fullstack

None. architect's UI is client-side-rendered and talks to the server only
through vox ([idioms §1](@/architecture/idioms.md)), so the Dioxus
fullstack hooks (`use_server_future`, `use_loader`, `#[server]`,
`dioxus::serve`, `Lazy<T>`) are deliberately unused — `cargo xtask idioms`
fails CI on them. `architect-atom` covers the same ground those hooks would
(data loading, refresh, optimistic mutation) without a second RPC stack:
isomorphic loading → `use_async`; server state → the vox `Resource`/`Layer`
graph; error pages → `match` on `AtomResult` + Dioxus `ErrorBoundary`.

> Enforced: the primitives are unit-tested in `features/atom`
> (`AtomResult` SWR transitions, store insert→reconcile temp→real swap,
> update/remove→rollback, hydrate-preserves-temp). The worked reference is
> the example feature's state hooks
> (`example-ui/src/data.rs` + `mutations.rs`), which the shell's route pages
> (`examples/app/ui/src/pages/`) `match` and compose.
