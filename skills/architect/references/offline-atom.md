# atom: optimistic Store, LocalRows, CallQueue, connection

Crate `architect-atom` (`features/atom`); guides `docs/content/architecture/optimistic.md` and `docs/content/architecture/local.md`. Gates: `persist` (LocalRows, CallQueue), `turso` (native `TursoRows`); Dioxus is always on.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `Store` / `StoreData` | keyed optimistic cache; temp and real ids; rollback snapshots | `features/atom/src/store.rs` `Store`, `insert_optimistic`, `is_stale`, `hydrate` |
| List and entry hooks | fetch folded into the store; `AtomResult` out | `features/atom/src/store.rs` `use_store`, `use_store_list`, `use_store_entry` |
| Mutation | optimistic `run`; queued `run_queued` | `features/atom/src/mutation.rs` `use_mutation`, `run`, `run_queued` |
| Id | `Real(K)` or `Temp(u64)` | `features/atom/src/id.rs` `Id` |
| LocalRows | device rows per (kind, scope), owner projection, queue storage | `features/atom/src/persist.rs` `LocalRows`, `KvRows`, `MemoryRows`, `LocalStorageKv` |
| Persisted list hook | device copy first; stale marking; write-back | `features/atom/src/persist.rs` `use_persisted_store_list` |
| `TursoRows` | transactional LocalRows over Turso | `features/atom/src/persist_turso.rs` `TursoRows` |
| `CallQueue` | durable ordered replay; refusal vs unreachable; temp-key remap | `features/atom/src/queue.rs` `CallQueue`, `QueuedCall`, `CallTransport`, `classify` |
| Connection | state, generation, connect hooks | `features/atom/src/connection.rs` `use_connect`, `use_connection`, `generation_now`; supervised: `crates/architect/src/reconnect.rs` `use_connect_supervised` |
| `ActionLike` | display grouping and fire-and-forget invoke; metadata only | `features/atom/src/actions.rs` `ActionLike`, `group_actions` |

`ActionLike` is not the command system: `architect::action::ActionList` is the registry (`references/commands-mcp.md`), and atom does not dispatch through it.

## Offline write model per datum

| Datum or write | Model |
|---|---|
| Server owns the row; client edits optimistically; a refusal rolls back | `Mutation::run` on `Store` |
| Same, but the call must survive being offline | `Mutation::run_queued` + `CallQueue` |
| Computed record the server numbers (timer start) | `run_queued` with `QueuedCall::creating(temp_key)` so later calls remap |
| Show the last-seen list offline | `use_persisted_store_list`; offline rows show `Store::is_stale` |
| Cache-first single entity | `use_store_entry` |
| Co-edited document | not here: CRDT (`references/crdt-peer-sync.md`) |

## Recipe 1: optimistic create with rollback

```rust
let create = use_mutation::<String>();
create.run(
    store,
    |s| s.insert_optimistic(draft).0,   // the patch returns the MutationId, not the pair
    move || async move {
        repo.create(input).await.map(Some).map_err(|e| format!("create: {e:?}"))
    },
);
```

`insert_optimistic` returns `(MutationId, Id)`: take `.0`. The crate-root doc example in `features/atom/src/lib.rs` passes the pair and would not compile. The call returns `Result<Option<T>, E>`: `Some` folds the server row in, `None` is a body-less confirm. `E: Clone + Display`.

## Recipe 2: queued call, replayed on reconnect

1. Describe the call: `QueuedCall::new("timer.start", route, payload, "start timer").creating("tmp-1")`.
2. `create.run_queued(store, |s| s.insert_optimistic(draft).0, move || live_call(), call)`. With no queue provided it falls back to `run`.
3. App root: `provide_local_rows(Arc::new(TursoRows::open_path(p).await?))` (or `MemoryRows::in_memory()`), then `provide_call_queue(CallQueue::new(rows), Rc::new(transport))`. It replays on mount and on `kick()`.
4. Transport: implement `CallTransport::send`; a creating call returns `Ok(Some(real_key))`.

Test oracles: `transport_errors_queue_and_refusals_do_not` (`features/atom/src/queue.rs`), `rows_and_the_queue_survive_reopening_the_file` (`features/atom/src/persist_turso.rs`).

## Recipe 3: persisted list

`use_persisted_store_list(store, "tasks", move || Some(format!("{org}/{acct}")), move || async move { Some(repo.list().await.map_err(..)) })`. The scope closure returns `Option<String>`; `None` reads and writes nothing. Task uses it in `task:crates/stores/src/lib.rs`.

## Connection

- App root: `use_connect(|| async { connect().await })`, or `use_connect_reactive` to reconnect on a signal change (it does not detect death). For death detection use `architect::use_connect_supervised` (`Caller::closed()` watch, `Backoff`).
- Feature hooks: `use_connection::<C>()` then `.ready()`.
- Drive loops read `generation_now()`, not `generation()`: dioxus `use_resource` tracks signal reads inside the future, so reading `generation()` restarts the loop.

## Pitfalls

- **`run` invalidates, `run_queued` doesn't.** `run` invalidates `invalidating` keys after every settle; the queued path does not, so derived reads stay stale until something refreshes them.
- **Unknown errors roll back.** `classify` defaults to `Refused`: a transport error whose text lacks an unreachable marker rolls back an offline edit. Add markers when adding a transport. Refusal markers win (`User(` beats `ConnectionClosed`).
- **Replay stops on unreachable, drops on refusal.** Refused calls are removed and reported `Lost`; the app names them in a notification and the user redoes them.
- **Temp-key remap is exact string match** on JSON string values; a key embedded in a longer string is not rewritten, and a refused creating call does not remap later calls that name its temp key.
- **`KvRows` is not transactional**, and a corrupt queue body is overwritten on the next push (queued calls lost). `TursoRows` wraps `project` and `queue_push` in a transaction.
- **`localStorage` drops oversize writes silently** (`LocalStorageKv` `max_bytes` default 1,500,000; write-back ignores the error).
- **Temp rows never reach the device copy;** an unconfirmed insert survives a restart only through the queue. Rows the device copy cannot decode are skipped; the next fetch restores them.
- **Overlapping optimistic updates on one row:** rollback restores the value captured when that mutation patched, which can overwrite a later success.
- **`hydrate` drops real rows missing from the list;** in-flight temp rows survive.
- **`put` and `remove_real` skip rollback:** use them for server events only.
- **Create the store at the app root.** `use_store` owns its signal in `ScopeId::ROOT` so detached tasks can write it.
- **`is_live() == false` means cache;** prefer a live stream.
- **`Id::Temp` numbers are process-local** (`temp-{n}` list keys): never persist them.
- **wasm:** `LocalRows` and `CallTransport` are `?Send`; `TursoRows` is native only.
