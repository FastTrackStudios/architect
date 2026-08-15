# architect

> **Canonical location.** This repository is canonical. Architect was
> extracted from the FastTrackStudio monorepo (`libs/architect/`) in
> August 2026; history before the extraction lives there.
>
> Consumers pin a tag:
>
> ```toml
> architect = { git = "https://github.com/FastTrackStudios/architect", tag = "v0.1.0" }
> ```
>
> When co-developing architect alongside a consumer, override the tag
> with a local checkout instead of bumping:
>
> ```toml
> [patch."https://github.com/FastTrackStudios/architect"]
> architect = { path = "../architect/architect" }
> ```

## What is in here

| path | crates |
|---|---|
| `architect/`, `atom/`, `form/`, `macros/` | the framework core and its derives |
| `auth/` | identity, sessions, organizations |
| `permissions/` | `Principal × Resource × Action → Decision`, the gate |
| `crdt/` | collaborative document sync |
| `telemetry/` | `architect-telemetry` — wide events; the span *is* the event |
| `ui/` | `architect-ui`, the foundation component library |
| `story/` | `architect-story-*`, the story/snapshot framework |

A facet-native, vox-friendly entity framework for Rust. One
`#[derive(Entity)]` on a plain struct, and you get:

- A wasm-safe wire struct (with `facet::Facet`) usable from any
  client crate (Dioxus web, Dioxus desktop, future iOS via FFI).
- `<Entity>Create` / `<Entity>Update` / `<Entity>List` payload types.
- An auto-generated `<Entity>Repo` `#[vox::service]` trait — typed
  RPC over WebSocket, no JSON hand-rolling.
- Under `--features server`: the full SeaORM `Model` + `Entity` +
  `Column` + `Relation` + `ActiveModel`, plus a
  `<Entity>RepoStorage<C>` that implements the repo trait against a
  SeaORM connection.

```rust
#[derive(architect::Entity)]
#[architect(table_name = "examples", repo)]
pub struct Example {
    #[architect(primary_key, on_create = Uuid::new_v4())]
    pub id: Uuid,

    #[architect(filterable, sortable, fulltext)]
    pub name: String,

    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}
```

That's the whole entity. No `cfg_attr`. No parallel struct in another
crate. No manual `From<Model> for Example` glue.

## Layout

```
architect/  the user-facing crate (re-exports the derive + runtime traits)
macros/     architect-derive / architect-rpc-derive / architect-action-derive — the proc-macros
atom/       architect-atom — optimistic client state for Dioxus
form/       architect-form — typed, validated form state for Dioxus
auth/       architect-auth / auth / auth-proto / auth-db / auth-client — the auth feature
crdt/       crdt / crdt-seaorm / crdt-derive — the local-first layer
docs/       architecture + getting-started docs
```

All of these are ordinary members of the monorepo's root workspace and
are consumed as `architect.workspace = true` etc. The standalone repo's
`examples/app/` reference demo, CLI scaffolder, and xtask were not
imported — they remain in the standalone repo (and in this subtree's
git history).

The standalone repo's `examples/app/` is the **reference example** — a full Dioxus web +
desktop app (list/detail/create/edit/delete/search/duplicate) talking to
the server entirely over vox. Read it to learn the pattern, then template
it when spinning up a new project. The conventions it follows — and the
CI gates that enforce them — are written up in
[docs/architecture/idioms](docs/content/architecture/idioms.md).

## Why facet-only

vox uses facet for its wire encoding. By dropping serde derives
entirely, the wire format is one cohesive system — every architect
type is automatically Facet-able, which means vox can transport it
without any per-type glue. No parallel `serde` derives, no
`#[serde(rename = …)]` mismatches with `#[architect(...)]`.

## Learn

New here? Read **[Build a feature, end to end](docs/content/getting-started/walkthrough.md)**
— define an entity → pick a backend → serve it → consume it remote *or*
in-process, against the reference example. Then:

- [The architect pattern](docs/content/architecture/pattern.md) — what the
  `Entity` derive emits, field by field.
- [Idioms & enforcement](docs/content/architecture/idioms.md) — the
  conventions (vox-only RPC, the `Layer`/`Resource` DI engine, transport
  injection) and the CI gates behind them.
- [Reference](docs/content/reference/_index.md) — crate map, features, the
  runtime surface.

## Status

Working end to end. The `Entity` derive emits the full surface (wire types
+ `Create`/`Update`/`List` + the `<Entity>Repo` vox trait + a `Layer`
token, and the SeaORM storage under `server-seaorm`). The reference
example (`examples/app/`) runs as a Dioxus web app against an axum+vox
server, as a fully in-process desktop app, and as a CLI — all from the one
contract, covered by native + browser + in-process tests.
