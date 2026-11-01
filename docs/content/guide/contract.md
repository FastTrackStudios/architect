+++
title = "The contract"
description = "Entities, errors and services: what you write, what each attribute generates, and how to choose wire types."
weight = 20
+++

## What you'll build

The contract of a feature: the types and traits every other part of
the app (backends, servers, clients, UI, tests, a model over MCP)
programs against. In greeter it is the top half of
`examples/greeter/src/lib.rs`: one entity (`Note`), one error
(`GreetError`) and one service (`Greeter`). You write those three
declarations; architect generates the wire types, the repository trait,
the vox and HTTP faces, both clients and, under `server`, the storage
and its migration.

The contract lives in the feature crate's default feature set, so it
compiles for `wasm32-unknown-unknown`. Nothing in this chapter may pull
in a runtime, a database or a UI library.

## 1. An entity

An entity is a row the app stores and lists. Write the struct and its
options:

```rust
// examples/greeter/src/lib.rs
#[architect::entity(table_name = "notes", repo, events)]
#[derive(Eq)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
}
```

`#[architect::entity]` writes the derive line (`architect::Entity`,
`facet::Facet`, `Clone`, `Debug`, `PartialEq`, and the `fake` hook). Add
`Eq`, `Hash` or `Default` with your own `#[derive(…)]` under it.

What comes out, for `Note` with `repo`:

| Item | What it is | When |
|---|---|---|
| `Note` | the wire struct, as written | always |
| `NoteCreate`, `NoteUpdate`, `NoteList` | payloads: `Create` without `on_create` / `exclude(create)` fields, `Update` with every field `Option`, `List { items, total, page }` | always |
| `NoteRepo` | trait with `get`, `list`, `create`, `update`, `delete`, each `Result<_, RepoError>` | `repo` |
| `NoteRepoClient`, `NoteRepoHttpClient` | vox and HTTP clients; both implement `NoteRepo` | `vox`; architect `http-client` |
| `NoteRepoLayer` | the bundle token for `layers![…]` | `vox` |
| `note_http` | the HTTP router (`POST /note/get`, `/note/list`, …) and `PATHS` | always (inert without architect `http`) |
| `NoteEvent`, `NoteEvents`, `NoteEvented<R>` | a live feed: `Snapshot`, `Upserted`, `Deleted` | `events` |
| `NoteRepoStorage<C>`, `NoteMigration` | SeaORM storage and a create-table migration | your crate's `server` feature |
| `seed_fake_note` | create N fake rows through any `NoteRepo` | your crate's `fake` feature |

More container options: `store` (Dioxus store hooks and optimistic
mutations, for the client chapter), `form` (typed form fields), `crdt`
(a Loro-backed entity), `path = "…"` (the HTTP prefix) and
`page_size = N`. Field options:

| Field option | Effect |
|---|---|
| `primary_key` | required, once |
| `auto_increment = false` | for UUID and string keys |
| `on_create = <expr>` / `on_update = <expr>` | set server-side; the field leaves `<T>Create` |
| `exclude(create)` / `exclude(update)` | drop the field from that payload |
| `sortable` | `list` accepts a `Sort` on it; an unknown sort field is `InvalidInput` |
| `filterable` | an index in the migration (the filter argument itself is not applied yet) |
| `json` | store as a JSON column; required for `Vec<T>` (other than `Vec<u8>`), maps and sets |
| `crdt(text)` | on a `String` of a `crdt` entity: a mergeable text field |

The emitted code names `::vox`, `::facet`, `::thiserror` and `::uuid`
by path, so the feature crate depends on each directly, as
`examples/greeter/Cargo.toml` does. Generics on an entity are not
supported.

A plain wire type that is not a row (a request body, a nested record)
takes `#[architect::wire]`, which writes the same derive line and adds
`#[repr(u8)]` to enums:

```rust
// features/auth/auth-proto/src/device.rs
#[architect::wire]
#[derive(Eq)]
pub struct AccountOrg {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
}
```

## 2. An error

One error enum per service. `#[architect::error]` makes it a wire type
and maps it onto HTTP:

```rust
// examples/greeter/src/lib.rs
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]
    Unknown(String),
}
```

It generates (`crates/architect-derive/src/lib.rs` `error`):

- the derives `Facet`, `Clone`, `Debug`, `PartialEq`, `thiserror::Error`
  and `#[repr(u8)]`;
- an `architect::http::HttpError` impl: the code is the variant name in
  `snake_case` (`"unknown"`); the status follows the variant name
  (`NotFound` 404, `Unauthenticated` / `InvalidCredentials` /
  `SessionExpired` 401, `Forbidden` / `PermissionDenied` 403, `Conflict`
  409, `Internal` 500, anything else 400), unless
  `#[architect(http_status = N, code = "…")]` says otherwise;
- a `Transport(String)` variant (503) and `From<architect::TransportError>`.
  Mark one of your own single-`String` variants `#[architect(transport)]`
  to use it instead.

The transport variant is what lets the generated clients implement your
service trait: a dropped connection arrives as `GreetError::Transport`,
not as a different type. Repos use `architect::RepoError` (`NotFound`,
`InvalidInput`, `Conflict`, `Internal`), and that enum stays closed:
adding a variant changes the wire of every repo. A service that calls a
repo maps `RepoError` into its own enum at the boundary.

## 3. A service

Operations that are not CRUD are methods of a service trait:

```rust
// examples/greeter/src/lib.rs
#[architect::service(actions)]
pub trait Greeter {
    /// Greets a known name.
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    /// How many greetings so far.
    #[action(effect = read_only)]
    async fn count(&self) -> Result<u32, GreetError>;
    /// Every greeting, as it happens.
    #[subscribe]
    fn greeted(&self) -> String;
}

pub use prelude::*;
```

`#[architect::service]` is the vox face and the HTTP face together
(`#[architect::rpc]` is vox only). From `Greeter` you get
(`crates/architect-rpc-derive/src/lib.rs` `service`):

| Item | What it is |
|---|---|
| `GreeterClient` | vox client; implements `Greeter` |
| `GreeterHttpClient` | HTTP client (`::at(base)`, `.with_bearer(token)`); implements `Greeter` |
| `GreeterService` | the bundle token for `layers![…]` |
| `http::router(backend)` | axum routes: `POST /greeter/greet` with `{"name": …}`, `POST /greeter/count` |
| `GreeterStream`, `GreeterStreamSource`, `GreeterStreamService` | the `greeted` stream: vox sibling service, the backend's contract, its token; SSE at `/greeter/greeted` |
| `greeter_actions::register` | with `actions`: every method as an Action List action (`GREETER_GREET`, `GREETER_COUNT`) |
| `GreeterDirect` | the in-process view, no wire |

The generated names are module-level and unqualified (`Service`,
`layer`, `serve`, `http`), so put **one rpc trait per module**. The
`prelude` submodule re-exports them with the trait name baked in
(`GreeterService`, `greeter_layer`, `greeter_http`), and
`pub use prelude::*` at the crate root is how greeter exposes them.

The clients implement the trait only when every method is `async fn`
returning a literal `Result<T, E>` whose `E` has
`From<TransportError>`, which `#[architect::error]` provides.
`#[subscribe]` declarations don't count; they are stripped from the
trait. A sync or mixed trait, or a vox-only `#[architect::rpc]` trait,
gets clients with inherent methods returning `VoxError<E>` instead; fold
those with `architect::vox_into_app`.

### Streams

A `#[subscribe]` method is a declaration, not a method you call. It
names the event type (`String` for `greeted`). A declaration with no
arguments asks the backend for a hub; one with arguments
(`#[subscribe] fn events(&self, filter: F) -> E`) asks for an attach
function that receives the filter and a sink. The backend half is in
the next chapter.

### Routes other than `POST`

The default route is `POST /<prefix>/<method-kebab>`; the prefix is the
trait name in kebab-case minus a trailing `Service`. A method's
`#[http(…)]` names a verb, a path with parameters and a body limit, and
the service's `path` may hold parameters every method takes:

```rust
#[architect::service(path = "org/{slug}")]
pub trait Notes {
    #[http(get, path = "notes/{id}")]
    async fn note(&self, slug: String, id: u32) -> Result<String, NoteError>;
    #[http(put, path = "notes/{id}")]
    async fn set_note(&self, slug: String, id: u32, text: String) -> Result<(), NoteError>;
}
```

The details (query arguments on `get`, byte bodies, ranged downloads)
are in the server chapter and in [the HTTP face](@/architecture/http.md).
`crates/architect/tests/http_routes.rs` declares this trait for real.

### Other service options

- `actions`: project every method onto the Action List, so keys, menus,
  the palette and MCP run it by id. Mark readers
  `#[action(effect = read_only)]`; the default is `mutating`.
- `context = T`: an ambient per-call value every method receives
  without listing it, and a `<Trait>ScopedClient` that binds it once
  (`client.scoped(ctx)`). Such traits have no HTTP face and no actions
  yet.
- `sync_client`: a blocking `<Trait>SyncClient` for sync code calling a
  remote backend (native only).
- `scopes(…)`, `ops`: leveled call scopes and reified calls, for the
  DAW-shaped traits in studio.

## 4. Choosing types

Every type that crosses the wire derives `facet::Facet`; vox encodes it
natively and the HTTP face writes it as JSON through `facet-json`. No
`serde` on wire types (the entity `json` column option and an action's
`RawJson` reply are the documented exceptions).

- Scalars, `String`, `bool`, `Option<T>`, `Vec<T>`, `Uuid` and
  `chrono` dates travel as they are. Your own structs and enums take
  `#[architect::wire]`.
- Keys: `Uuid` with `on_create = Uuid::new_v4()` is the default. A
  `String` key (a slug the caller picks) works too; a duplicate is
  `RepoError::Conflict`.
- Keep a method to four parameters or fewer; put more in one wire
  struct.
- Event payloads carry full state (`Upserted(row)`), never diffs, so a
  late or repeated event re-applies cleanly.
- A large result (megabytes: an audio render, a state chunk) is better
  kept in the backend behind an `architect::handle::RawHandle`, with a
  method that reads ranges of it ([large payloads](@/architecture/backends.md)).
- Bytes for a browser or a download go through an HTTP-only method
  returning `architect::http::ByteBody` (server chapter).

## Where to read more

- [The architect pattern](@/architecture/pattern.md): every entity
  option and what it emits.
- [Errors](@/architecture/errors.md): `RepoError`, `ClientError<E>`,
  and the checklist for a service error.
- [The HTTP face](@/architecture/http.md): the wire, routes, query
  arguments, bytes and SSE.
- [Server push](@/architecture/streams.md): `PubSub`, entity events,
  `#[subscribe]`.
- [Commands](@/architecture/commands.md): what `actions` puts on the
  Action List.
- [Idioms](@/architecture/idioms.md): facet-only wire types, one vox
  revision.
