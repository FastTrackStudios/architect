+++
title = "The architect pattern"
description = "One source struct, every surface — wire, repo trait, storage."
weight = 10
+++

The canonical shape for any new entity: a plain Rust struct decorated
with `#[derive(architect::Entity)]`. The derive emits everything else.

## What you write per entity

```rust
// features/<feature>/<feature>-proto/src/lib.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "examples", repo)]
pub struct Example {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,

    #[architect(filterable, sortable, fulltext)]
    pub name: String,

    #[architect(filterable, fulltext)]
    pub description: String,

    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,

    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}
```

Nothing else. No parallel DTO struct, no manual `From<Model> for Example`,
no `cfg_attr` gymnastics on the fields.

## What the derive emits

Always (wasm-clean):

- `Example` — the wire struct (left as you wrote it; you derive
  `facet::Facet` yourself so it travels on vox transports).
- `ExampleCreate`, `ExampleUpdate`, `ExampleList` — payload structs
  with the right field exclusions.
- `ExampleRepo` — the `#[vox::service]` trait with `get` / `list` /
  `create` / `update` / `delete`. Vox generates `ExampleRepoClient` for
  consumers and `ExampleRepoDispatcher` for the server.
- `ExampleRepoLayer` — a `Layer` service token (+ a `<snake>_repo_layer`
  helper) so the repo composes through architect's layer system:
  `layers![ExampleRepoLayer]`, `.provide(backend)` / `backend.into_router()`.
  See [idioms §6](@/architecture/idioms.md).

Under `--features server-seaorm`:

- The SeaORM `Model` + `Entity` + `Column` + `Relation` + `ActiveModel`
  with the right `#[sea_orm(...)]` attributes synthesized from your
  architect attributes.
- `impl From<Model> for Example` — the bridge between storage and wire.
- `ExampleRepoStorage<C: ConnectionTrait + ...>` — concrete `ExampleRepo`
  implementation against SeaORM. Sort handling is wired against
  `Column` variants for every `#[architect(sortable)]` field.

With the `store` flag, under the consumer's `atom` + `vox` features (the
Dioxus client side):

- `ExampleStore` (+ `provide_example_store` / `use_example_store`) — the
  shared optimistic cache, and `ExampleClientError` — the typed error.
- `use_example(id)` / `use_example_list()` — data hooks returning an
  `AtomResult` phase for pages to `match`.
- `ExampleMutations` (`use_example_mutations`) — optimistic
  `create`/`update`/`delete` with rollback, failure notification, and
  reactivity-key invalidation. See
  [composing the UI](@/architecture/composing-the-ui.md).

## How a client consumes it

Identical from wasm and desktop — only the link construction differs:

```rust
let link = WsLink::connect("ws://localhost:4040/vox").await?;
let client = initiator_on(link, TransportMode::Bare)
    .establish::<ExampleRepoClient>()
    .await?;

let created = client.create(ExampleCreate {
    name: "alpha".into(),
    description: "first row".into(),
}).await?;

let listed = client.list(Page { index: 0, size: 50 }, None, None).await?;
```

This is the raw form. The example apps don't hard-code a URL — they
program against `ExampleRepoClient` and inject a `Transport` (remote
WebSocket *or* in-process) at the app root, so the same screens run
against a server or an embedded backend. See
[idioms §7 — inject the transport](@/architecture/idioms.md) and the
[end-to-end walkthrough](@/getting-started/walkthrough.md).

## Field attributes

| Attribute | Effect |
|-----------|--------|
| `primary_key` | Marks the SeaORM primary key. Required. |
| `auto_increment = false` | Disables SeaORM's auto-increment (use for UUIDs). |
| `on_create = <expr>` | Server-side default for inserts. Field drops out of `<T>Create`. |
| `on_update = <expr>` | Server-side default for updates. |
| `exclude(create)` / `exclude(update)` | Drop the field from the corresponding payload struct. |
| `filterable` | Reserved for the structured filter AST. |
| `sortable` | Adds a match arm in `<T>RepoStorage::list` mapping the wire-side field name to `Column::<PascalCase>`. |
| `fulltext` | Reserved for FTS5 emission. |

## Container attributes

| Attribute | Effect |
|-----------|--------|
| `table_name = "..."` | SeaORM table name. Defaults to snake_case of the struct name. |
| `repo` | Emit the `<T>Repo` `#[vox::service]` trait + the server-side `<T>RepoStorage<C>`. |
| `store` | Also emit the Dioxus client state layer (store, typed hooks, optimistic mutations). Requires `repo`; gated on the consumer's `atom` + `vox` features; pk must be `Clone + Eq + Hash + FromStr` (`Uuid`, `String`, integer ids — `Copy` not required). |
| `form` | Also emit typed form bindings (`<T>CreateFields` / `<T>UpdateFields`, `submit()` → the wire payload). Gated on the consumer's `form` feature. Field attrs: `form(optional)`, `form(label = "…")`. |
| `events` | Also emit the live-event story: `<T>Event` enum, `<T>Events` subscribe trait, the `<T>Evented<R>` publish-through wrapper (its `Services` bundle mounts CRUD + the event feed), and — with `store` — the `use_<t>_events()` client hook that makes every store-rendered page live. Requires `repo`. Full walkthrough: [server push](@/architecture/streams.md). |
