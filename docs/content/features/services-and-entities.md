+++
title = "Services and entities"
description = "The derives: one Rust declaration gives the wire types, the vox service, the HTTP face, the clients and the actions."
weight = 10
+++

## What it is and when to use it

An app declares each capability once, as Rust, and the derives write the rest.
`#[architect::entity]` on a struct gives its create and update payloads, a
`<Entity>Repo` trait with get, list, create, update and delete, a layer token,
an HTTP face and, by option, change events, a store binding, forms, a CRDT
codec and SeaORM storage. `#[architect::service]` on a trait gives the vox
service, its client and dispatcher, the HTTP router and HTTP client, SSE for
each `#[subscribe]` method and, with `(actions)`, one Action List action per
method. `#[architect::rpc]` is the vox-only spelling.

Use them for every capability instead of a hand-written DTO, axum route or
Dioxus server function. The design is in
[The architect pattern](@/architecture/pattern.md) and
[The HTTP face](@/architecture/http.md); `examples/greeter` is the reference
feature, and `cargo xtask feature new <name>` scaffolds a new one shaped like it.

## A minimal example

From `examples/greeter/src/lib.rs`. The crate depends on `architect` with
`host`, `http-client`, `local` and `action-list`, and on `vox`, `facet`,
`thiserror` and `uuid`. Its own `vox` feature forwards `architect/vox`,
because the derive emits `cfg(feature = "vox")` in the crate that uses it.

```rust
#[architect::entity(table_name = "notes", repo, events)]
#[derive(Eq)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
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

// impl Greeter for Memory { .. }
// impl GreeterStreamSource for Memory { fn greeted_hub(&self) -> &PubSub<String> { .. } }
// impl NoteRepo for Memory { get, list, create, update, delete }

impl architect::Services for Memory {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![GreeterService, GreeterStreamService, NoteRepoLayer]
    }
}
```

The backend is now a router for every transport:

```rust
let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
let client: GreeterClient = local.establish().await?;   // vox
let http = GreeterHttpClient::at(base);                 // HTTP; both implement Greeter
```

## Code tour

1. `examples/greeter/src/lib.rs` `Greeter`, `Memory`: the four things an app
   writes: an entity, an error, a service trait and a backend that implements it.
2. `crates/architect-derive/src/lib.rs` `entity`: adds the derive line
   (`Entity`, `Facet`, `Clone`, `Debug`, `PartialEq`).
3. `crates/architect-derive/src/lib.rs` `derive_entity` and `expand`: the
   payloads and the repo; `build_http_block`, `build_events_block` and
   `build_server_block` add the optional faces.
4. `crates/architect-rpc-derive/src/lib.rs` `service`: the `rpc` expansion with
   the HTTP face on; `emit_stream_block`, `emit_http_block` and `emit_prelude`
   write `GreeterService`, `GreeterStreamService`, `GreeterClient`,
   `GreeterHttpClient` and the rest.
5. `crates/architect/src/layer.rs` `Services`, `layers!`, `LayerRouter`: bind
   the layer tokens to one backend and dispatch by method id; `into_http_router`
   gives the axum face.
6. `crates/architect/src/local.rs` `LocalServer`: a typed client over an
   in-process link.
7. `crates/architect/src/http.rs` `respond`, `HttpClient`: the server's reply
   and the typed HTTP client.

A `greet` call over vox is a request carrying a method id. `LayerRouter` finds
the generated dispatcher for that id, which decodes the arguments and calls
`Memory::greet`. Over HTTP, `POST /greeter/greet {"name":"cody"}` reaches the
generated router, which decodes the same arguments, calls the same method and
answers through `respond`. Both clients implement `Greeter`, so one scenario
test runs on every wire.

## Tests to read

- `examples/greeter/tests/round_trip.rs`: in process, over vox, over HTTP,
  streams over HTTP, and a mock layer replacing the real greeter.
- `examples/greeter/tests/mcp.rs`: the `actions` face answers what HTTP answers.
- `crates/architect/tests/http_face.rs`: a service and an entity repo with its
  events over HTTP; sync traits through the bridge.
- `crates/architect/tests/entity_storage.rs`: migrate, then CRUD over SQLite
  (features `server` and `vox`).
- `crates/architect/tests/layer_override.rs`: a merged mock wins on vox and HTTP.
- `crates/architect-rpc-derive/tests/smoke.rs`: sync, async and mixed traits.

## Status

Done in architect. Known limits, from
`skills/architect/references/entities-storage.md` and
`skills/architect/references/rpc-services-hosts.md`: the list `filter` is
ignored and `fulltext` emits nothing; migrations only create; no relations;
every `DbErr` becomes `Internal`; no generics; one rpc trait per module
(the emitted names are unqualified); `context = T` traits get no HTTP face and
no actions; a generated client implements the trait only for an all-async,
all-`Result` `#[architect::service]`.

- **Task** uses `#[derive(architect::Entity)]` with `repo` only and
  `#[architect::rpc]` throughout (for example
  `task:features/agent/agent-proto/src/service/runs.rs`). It does not use
  `#[architect::entity]`, `#[architect::service]` or `#[architect::error]` yet,
  so it gets no generated HTTP face or actions.
- **Studio** uses `#[architect::rpc]` (for example
  `studio:features/midi/src/proto/service.rs`) and SeaORM entities of its own
  in `studio:features/signal/storage/src/entity`; no architect entities or
  `service` traits yet.
