# Entities, storage, config, errors, wire, resources, platform

Derives live in `crates/architect-derive/src/lib.rs` (`entity`, `wire`, `error`, `derive_config`); runtime in `crates/architect/src/lib.rs`. Guides: `docs/content/architecture/pattern.md`, `docs/content/architecture/errors.md`, `docs/content/architecture/backends.md`. Emitted behaviour is covered by `crates/architect/tests/entity_storage.rs`; the derive's own tests are unit tests of `expand()`.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `#[architect::entity]` | writes the derive line (Entity, Facet, Clone, Debug, PartialEq, fake) | `crates/architect-derive/src/lib.rs` `entity` |
| `#[architect::wire]` | derives for plain wire types; enums get `#[repr(u8)]` | `crates/architect-derive/src/lib.rs` `wire` |
| `#[architect::error]` | wire derives + `HttpError` + `Transport` variant + `From<TransportError>` | `crates/architect-derive/src/lib.rs` `error` |
| Payloads + repo trait | `<E>Create/Update/List`; `<E>Repo` (a vox service under `vox`) | generated |
| Repo layer token | `<E>RepoLayer`, `<repo_snake>_layer()` | generated |
| SeaORM storage | `<E>RepoStorage<C>`, `<E>Migration` (create-if-not-exists + indexes) | generated; `crates/architect/src/lib.rs` `migrator`, `migrate` |
| `store` / `form` / `events` / `crdt` options | Dioxus store hooks, form bindings, event feed, Loro codec | generated (see below) |
| `JsonField` | serde-json round-trip into a SeaORM JSON column | `crates/architect-derive/src/lib.rs` `JsonField` |
| Config | `#[derive(architect::Config)]` → `from_env()` | `crates/architect-derive/src/lib.rs` `derive_config` |
| Error envelope | `RepoError`, `TransportError`, `ClientError<E>`, `vox_into_app` | `crates/architect/src/lib.rs` `RepoError`, `ClientError` |
| Wire primitives | `Page`, `SortOrder`, `Sort`, `Filter` | `crates/architect/src/lib.rs` `Page` |
| Storage helpers | `storage::connect`, `is_in_memory_sqlite`, `migrate`, `DbConn` | `crates/architect/src/lib.rs` `is_in_memory_sqlite` |
| Resources / Scope | `Scope` (LIFO finalizers), `Resource<T,E>`, memoized `SharedResource` | `crates/architect/src/resource.rs` `Scope`, `SharedResource` |
| Handles | `RawHandle` (wire), `Handle<T>`, `HandleRegistry<T>` | `crates/architect/src/handle.rs` `Handle`, `HandleRegistry` |
| Platform | `sleep`, `now`, `Clock`/`TestClock`, `spawn`, `timeout`, `Semaphore`, `CancellationToken` | `crates/architect/src/platform.rs` |
| Lock helpers | poison-recovering `lock` / `read` / `write` | `crates/architect/src/lock.rs` |

## Minimal recipe

Cargo features, as greeter wires them (`examples/greeter/Cargo.toml`):
```toml
[features]
default = ["vox"]
vox = ["architect/vox"]
server = ["architect/server", "dep:sea-orm"]
```
The consumer also depends directly on `vox`, `thiserror`, `uuid` and `facet`: emitted code names `::vox::service`, `::thiserror::Error` and `::sea_orm` by path.

Entity + storage (`crates/architect/tests/entity_storage.rs` `migrate_then_crud_over_sqlite`):
```rust
#[architect::entity(table_name = "gadgets", repo)]
#[derive(Eq)]
pub struct Gadget {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub name: String,
    pub weight: i32,
    pub note: Option<String>,
}
architect::migrator!(Migrator: [GadgetMigration]);
architect::storage::migrate::<Migrator>(&db).await?;
let repo = GadgetRepoStorage::new(db);
```

Config (`examples/greeter/src/main.rs`):
```rust
#[derive(architect::Config)]
#[architect(prefix = "GREETER")]
struct Config {
    #[architect(default = "127.0.0.1:4040")]
    bind: String,
}
let config = Config::from_env()?;
```

## Attribute reference

Container `#[architect(...)]` on an entity:
- `table_name = "…"` (default snake_case of the struct name); `repo` (repo trait, storage, layer); `store` (needs `repo`; consumer features `atom` + `vox`); `form` (consumer feature `form`); `events` (needs `repo`); `crdt` (needs `repo` and a `Uuid` pk); `page_size = N` (store list hook, default 100); `path = "seg"` (one segment, default kebab-case entity name). Any other key is a derive error.

Field `#[architect(...)]`:
- `primary_key` (required); `auto_increment = bool` (SeaORM default non-auto); `on_create = <expr>` (field leaves `<E>Create`); `on_update = <expr>`.
- `filterable` emits an index only (the list filter argument is ignored); `sortable` emits a sort arm (unknown sort key → `InvalidInput`); `fulltext` is parsed and emits nothing.
- `json`: column becomes JSON; required for `Vec<T>` (not `Vec<u8>`), maps and sets; needs serde on the type (the documented exception to facet-only).
- `crdt(text)`: on a `String` field of a `crdt` entity, stores a `LoroText` child written by minimal diff, so concurrent edits merge (see `references/crdt-peer-sync.md`). On a non-`String` field, or without the container's `crdt`, it is a derive error.
- `form(optional)` (String fields only), `form(label = "…")`, `exclude(create)` / `exclude(update)`.

Config container: `prefix = "P"` (trailing `_` trimmed). Config field: `env = "NAME"`; `default = expr` (via `Into::into`); `secret` (reads `NAME_FILE` first); `nested` (calls `<T>::from_env()`); `skip` (`Default`). Default variable name: `PREFIX_FIELD` upper-cased.

Error variant (`#[architect::error]`): `http_status = N`; `code = "…"`; `transport` (holds one `String`, at most one). Status defaults: `Transport` 503, `NotFound` 404, `Conflict` 409, else 400.

## Generated items (entity `Note`, repo `NoteRepo`)

- Always: `NoteCreate`, `NoteUpdate` (all fields `Option`), `NoteList { items, total, page }`; trait `NoteRepo` with `get/list/create/update/delete`; `NoteRepoLayer`, `note_repo_layer()`, `note_repo_service_descriptor()`; `seed_fake_note` under `fake`.
- Under `server`: `NoteRepoStorage<C>`, `NoteMigration`; a private `__note_storage` module holds `Model`/`Entity`/`Column`/`ActiveModel`.
- `store`: `NoteStore`, `NoteMutations`, `NoteClientError`, `use_note_store`, `provide_note_store`, `use_note`, `use_note_list`, `use_note_mutations`, `provide_note`.
- `form`: `NoteCreateFields`, `use_note_create_fields`, `NoteUpdateFields`, `use_note_update_fields`.
- `events`: `NoteEvent`, `NoteEvents`, `NoteEvented<R>`, `NoteEventsLayer`, `use_note_events`.
- HTTP: module `note_http` with `GetArgs ListArgs CreateArgs UpdateArgs DeleteArgs` and `PATHS`.
- `crdt`: `impl crdt::EntityCrdt for Note`, `NoteRepoLoro`, `NoteCrdtActions`, `use_note_crdt_list`, `use_note_crdt`, `use_note_crdt_actions`.
- Payloads derive `Clone, Debug, PartialEq, facet::Facet`; `Eq` is not derived.

## Pitfalls

- **Feature names belong to the consumer crate.** The derive emits `cfg(feature = "server")`, `"vox"`, `"atom"` + `"vox"` (store/events), `"form"`, `"crdt"`, `"fake"`. A crate whose feature is named `server-seaorm` silently gets no storage. Greeter maps `server` → `architect/server`; architect's `server` implies `server-seaorm`, which gates the `storage` module.
- **`store`, `events`, `crdt` require `repo`**; container-shaped fields need `json` (`Vec<u8>` is native); `form` misuse is a derive error.
- **Generics are not handled by the entity derive**; facet's generic-bound problem is documented in `crates/architect/src/handle.rs`.
- **Every `DbErr` maps to `RepoError::Internal`**, so a unique-constraint violation surfaces as 500, not 409.
- **Sort/filter/paging are thin:** `filter` is ignored; `page.size` is only floored to 1.
- **Migration is create-only.** Column changes later need hand-written migrations. For CRDT docs, version with `crdt::schema` instead.
- **In-memory SQLite is a pool of one**; `cache=shared` is the alternative.
- **No relations:** the generated `Relation` enum is empty.
- **Error envelope:** `ClientError` has `App`, `Connect`, `Transport`; `into_app` folds transport failures into `E: From<TransportError>`. `RepoError` is `#[repr(u8)]`: adding a variant is a wire change.
- **Config types:** `FromEnvValue` exists for scalars, `bool`, `Option<T>`, `Vec<T>`; other types (`PathBuf`) don't compile. A blank variable counts as unset, so `default` applies. `architect::Config` (derive) and `architect::config` (module) both exist.
- **Stale doc comments:** `crates/architect/src/resource.rs` names `Resource::memoize`, which does not exist (use `SharedResource::from_fn`); `crates/architect/src/handle.rs` says "facet 0.46" while the workspace pins facet 0.50 rc.
- **Wasm:** storage, migration and store-http are gated off wasm; platform `sleep` is millisecond-grained there.
