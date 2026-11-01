+++
title = "Clients"
description = "The generated vox and HTTP clients, connecting and the connection lifecycle, the offline queue and optimistic state, forms."
weight = 50
+++

## What you'll build

The consuming side: code that calls the service from a CLI, another
server, a desktop app or a browser. First the plain clients and how to
connect them; then, for a Dioxus app, one supervised connection at the
app root that survives server restarts, an optimistic store over the
entity, an offline queue for writes made with the network down, and a
form derived from the entity. The UI itself is part 2.

## 1. Program against the trait

The generated clients implement the service trait, so the code that
uses the service takes the trait and never learns the wire:

```rust
// examples/greeter/tests/round_trip.rs
async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
    assert_eq!(client.count().await.unwrap(), 1);
}
```

`Memory` itself, a `GreeterClient` (vox) and a `GreeterHttpClient`
(HTTP) all pass. Write screens, CLIs and jobs the same way: against
`impl Greeter` or `impl NoteRepo`, with the transport chosen once at
the root ([idioms §7](@/architecture/idioms.md)).

## 2. The HTTP client

```rust
let client = GreeterHttpClient::at("https://api.example.com").with_bearer(token);
let hi = client.greet("cody".into()).await?;
let mut greeted = client.greeted().await?;          // SSE, an EventStream<String>
while let Some(next) = greeted.next().await { /* … */ }
```

It needs architect's `http-client` feature (reqwest natively, `fetch`
in the browser). The trait's own methods return
`Result<T, GreetError>`, a transport failure arriving as
`GreetError::Transport`. Several clients can share one
`architect::http::HttpClient` (base URL, bearer, connection pool).
The entity has one too: `NoteRepoHttpClient`.

## 3. The vox client

Natively and in the browser, connect through the shared pool
(architect feature `connect`):

```rust
use architect::connect::{Endpoint, pool};

let endpoint = Endpoint::url("ws://127.0.0.1:4040/vox").with_bearer(token);
let greeter: GreeterClient = pool().client(&endpoint).await?;
let notes: NoteRepoClient = pool().client(&endpoint).await?;   // same socket
```

The pool (`crates/architect/src/connect/mod.rs` `Pool`) keeps one
connection per (endpoint, bearer), dials once when several callers ask
together, and drops a dead connection on the next request. Typed
clients are cheap views over one `vox::Caller`, so adding a feature
adds no sockets. After sign-in or sign-out, `pool().evict_all()`; the
bearer is part of the key, so a signed-in socket is never reused
anonymously.

Other dials:

- In-process: `LocalServer::serve(router, scope).establish::<GreeterClient>()`
  (native and wasm; feature `local`). A desktop app can run its whole
  backend this way with no server.
- iroh: `iroh_link::connect(&endpoint, remote_id)`, or through the pool
  with `Pool::caller_with(endpoint, dial)`. The dial is bounded and
  retries once after a peer restart; branch on `DialError::of(&err)`
  kinds, not on message text (`crates/architect/src/iroh_link.rs`).
- A test with several servers uses its own `Pool::default()` rather
  than the global one (`apps/auth-server/tests/surfaces.rs`
  `websocket_vox_stack`).

Wrap a one-off connect that may meet a server still booting in
`architect::schedule::retry` with a `Schedule`
(exponential, `max_delay`, jitter, `take(n)`); see
[scheduling](@/architecture/scheduling.md).

A stream over vox without Dioxus is a call held open for the
subscription's life. Poll the call and the channel together, the call
first:

```rust
let (tx, mut rx) = vox::channel::<String>();
let mut call = std::pin::pin!(stream_client.greeted(tx));
loop {
    tokio::select! {
        biased;
        res = &mut call => break res,            // the stream ended
        frame = rx.recv() => match frame {
            Ok(Some(frame)) => { let _ = frame.map(|greeting| handle(greeting)); }
            _ => break call.await,
        },
    }
}
```

Dropping `call` unsubscribes. `features/crdt/crdt/src/sync.rs`
`SyncedDoc::run` is a real one.

## 4. One supervised connection at the app root

A Dioxus app holds one connection, provided as context, and every
feature builds its clients from it. Under architect's `atom` +
`connect` + `platform` features, the one-liner is `use_app_pooled`:

```rust
// app root
let conn = use_app_pooled(move || {
    Endpoint::url(vox_url_for(&active_org())).with_bearer(session_token())
});
```

It provides the app-wide notifications and reactivity registries, then
supervises the connection (`crates/architect/src/lib.rs`
`use_app_pooled`, built on `use_app_supervised`):

- The endpoint closure is reactive: when a signal it reads changes (the
  org switcher, a server URL, a token arriving after sign-in), the
  connection is re-established.
- Death is watched with `Caller::closed()`, which resolves when the vox
  session ends (EOF, error, shutdown). The connection flips from
  `Ready` to `Connecting` at once and reconnects.
- Reconnects run under `architect::reconnect::Backoff`: exponential
  with full jitter, from 250 ms up to 10 s, reset on success. Each
  instance has its own seed, so clients that lost the same server do
  not retry in lockstep. One `warn` per outage, not per attempt.
- `Connection::generation()` bumps on every successful (re-)establish,
  so caches keyed on a caller can tell they belong to a dead one.

With a connect that is not the pool, `use_app_supervised(connect,
watch_death)` takes both halves:

```rust
use_app_supervised(
    move || { let slug = active_org(); async move { caller_for(&slug).await } },
    |caller: vox::Caller| async move { caller.closed().await },
);
```

Feature code reads the connection with `use_connection::<C>()` and
`.ready()`, and never reconnects on its own.

One trap: Dioxus polls a `use_resource` future in a reactive context,
so a signal read inside it subscribes the resource. A drive loop that
reads `conn.generation()` restarts itself on every reconnect. Loops
read `conn.generation_now()`, which peeks
(`features/atom/src/connection.rs` `Connection::generation_now`).
`crates/architect/tests/reconnect_supervisor.rs` covers the supervisor.

## 5. Optimistic state: the store

Add `store` to the entity (and `atom` + `vox` features to the crate
that compiles the client side) and the derive emits a keyed optimistic
cache with hooks:

```rust
#[architect::entity(table_name = "notes", repo, events, store)]
pub struct Note { /* … */ }
```

| Item | What it does |
|---|---|
| `NoteStore`, `provide_note_store`, `use_note_store` | the shared cache, created at the app root |
| `use_note(id)`, `use_note_list()` | reads returning an `AtomResult` |
| `NoteMutations` (`use_note_mutations`) | `create` / `update` / `delete`: the row changes at once, the call runs, a failure rolls back and notifies |
| `use_note_events()` | with `events`: fold the server's `Upserted` / `Deleted` into the store, so every page reading it is live |
| `provide_note()` | store and live events in one call at the root |

`AtomResult` keeps the previous value while a refetch runs, so a list
never blanks:

```rust
use architect::AtomResult::*;
match notes.state() {
    Initial | Loading => rsx! { p { "Loading…" } },
    Reloading(prev) | Success(prev) => rsx! { NoteList { notes: prev } },
    Error { error, .. } => rsx! { p { class: "error", "{error}" } },
    Defect { defect, .. } => rsx! { p { class: "error", "{defect}" } },
}
```

Errors in hooks are `ClientError<E>`: `App(E)` when the handler
returned the service's error (branch on `App(RepoError::NotFound)`),
`Connect` and `Transport` for infrastructure, which
`is_infrastructure()` tells apart for one shared "reconnecting" state.

For hand-written state, the primitives underneath are
`architect::use_store`, `use_mutation` (whose `run` takes the
optimistic patch and the server call together, so the rollback cannot
be forgotten) and `use_async` (`features/atom/src/store.rs`,
`features/atom/src/mutation.rs`).

## 6. Offline: device rows and the call queue

Two pieces, behind architect's `atom-persist` (and `atom-turso` for the
native database):

- **Rows on the device.** `use_persisted_store_list(store, kind, scope,
  fetch)` shows the last-seen list first and marks rows stale until the
  fetch lands. The rows live in a `LocalRows`: `TursoRows` natively,
  `LocalStorageKv`-backed rows in the browser, `MemoryRows` in tests.
- **Calls made offline.** `Mutation::run_queued` applies the
  optimistic patch, tries the call, and when the server is unreachable
  keeps it in a durable `CallQueue` that replays in order on reconnect
  (`kick()`). A refusal (the server said no) is dropped and reported as
  `Lost`; an unreachable server stops the replay until next time. A
  call that creates a row the server numbers carries
  `QueuedCall::creating(temp_key)`, so later queued calls naming the
  temporary key are rewritten with the real one.

```rust
// app root
provide_local_rows(Arc::new(TursoRows::open_path(path).await?));
provide_call_queue(CallQueue::new(rows), Rc::new(transport));

// a write
let call = QueuedCall::new("timer.start", route, payload, "start timer").creating("tmp-1");
create.run_queued(store, |s| s.insert_optimistic(draft).0, move || live_call(), call);
```

`features/atom/src/queue.rs` `CallQueue` and its test
`transport_errors_queue_and_refusals_do_not` pin the rules. Pick the
model per datum: server-owned rows go through the store and queue;
co-edited documents are CRDTs (part 2, `documents.md`); live signals
like meters are `PubSub` streams, never stored.

## 7. Forms

Add `form` to the entity (and a `form = ["architect/form"]` feature to
the crate) and the derive emits typed, validated fields over the
payloads: `NoteCreateFields` (`use_note_create_fields()`) and
`NoteUpdateFields` (`use_note_update_fields(&note)`, seeded from the
row). `submit()` validates every field, reveals the errors and returns
the wire payload the mutations take:

```rust
let fields = use_note_create_fields();
rsx! {
    form {
        onsubmit: move |evt| {
            evt.prevent_default();
            if let Some(input) = fields.submit() {     // -> NoteCreate
                mutations.create(input);
                fields.reset();
            }
        },
        TextField { field: fields.title, label: "Title" }
        TextField { field: fields.body, label: "Body" }
        button { r#type: "submit", "Add note" }
    }
}
```

`String` fields are required unless marked `form(optional)`; other
types parse with `FromStr`. A form that does not mirror a payload
(sign-in, search) builds fields by hand with
`architect::form::use_field(initial, validate::required("Name"))` and
the validators in `features/form`.

## Where to read more

- [Optimistic state](@/architecture/optimistic.md): `AtomResult`,
  `Store`, `Id`, `use_mutation`.
- [Forms](@/architecture/forms.md): fields, validators, components.
- [Scheduling](@/architecture/scheduling.md): `Schedule`, `retry`,
  supervision, the platform clock.
- [The in-process transport](@/architecture/local.md): a backend in
  the same process.
- [Errors](@/architecture/errors.md): `ClientError<E>`.
- [Server push](@/architecture/streams.md): `use_stream`,
  `use_store_stream`, entity events on the client.
- [Composing the UI](@/architecture/composing-the-ui.md): what the
  `store` derive gives a page.
