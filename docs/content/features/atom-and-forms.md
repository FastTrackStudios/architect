+++
title = "Atom and forms"
description = "architect-atom: optimistic, stale-while-revalidate client state, a keyed store with temp ids, mutations that patch then reconcile, device rows and an offline call queue; architect-form: typed, validated fields."
weight = 45
+++

## What it is and when to use it

`architect-atom` is client-side state for Dioxus. `AtomResult` names the
phases a read goes through (initial, loading, reloading, success, error,
defect), so a page renders stale data while it revalidates. `Store<T>` is a
keyed store of rows; `insert_optimistic` adds a row with a typed temporary
id. `use_mutation().run` patches the store first, makes the call, then
reconciles the temporary row with the server's or rolls it back and reports
the error. With `persist`, rows live on the device (`LocalRows`) and calls
made offline wait in a durable `CallQueue` that replays them in order on
reconnect (`run_queued`).

`architect-form` gives `Field<T>`: the text a person typed, parsed and
validated into `T`, with components to draw it; an entity's `form` option
writes its create and update fields. Use both for server-owned rows edited
from a UI. Co-edited state is a [document](@/features/documents.md) instead.
Design: [Optimistic state](@/architecture/optimistic.md) and
[Forms](@/architecture/forms.md); the pitfalls are in
`skills/architect/references/offline-atom.md`.

## A minimal example

`architect` with `atom` (or `client`), `atom-persist` for the queue, `form`
for fields. After `skills/architect/references/offline-atom.md`:

```rust
impl StoreEntity for Note {               // or #[architect::entity(.., store)]
    type Key = Uuid;
    fn key(&self) -> Uuid { self.id }
}

let store = use_store::<Note, String>();  // at the app's root
use_context_provider(|| store);

let create = use_mutation::<String>().invalidating(&["notes"]);
create.run(
    store,
    move |s| s.insert_optimistic(draft).0,       // the ticket; .1 is the temp id
    move || async move {
        client.create(input).await.map(Some).map_err(|e| format!("{e:?}"))
    },
);
```

Offline-safe, with `provide_local_rows(..)` and `provide_call_queue(CallQueue::new(rows), transport)`
at the root:

```rust
create.run_queued(store, move |s| s.insert_optimistic(draft).0, live_call,
    QueuedCall::new("timer.start", route, payload, "Start timer").creating("tmp-1"));
```

A field:

```rust
let name = architect::form::use_field("", architect::form::validate::required("Name"));
rsx! { architect::form::TextField { field: name, label: "Name" } }
```

## Code tour

1. `features/atom/src/result.rs` `AtomResult`: the phases.
2. `features/atom/src/store.rs` `StoreEntity`, `Store` (`insert_optimistic`,
   `hydrate`, which keeps temporary rows), `use_store`, `use_store_list`,
   `use_store_entry`; `features/atom/src/id.rs` `Id`.
3. `features/atom/src/mutation.rs` `use_mutation`, `Mutation::invalidating`,
   `Mutation::run`, `Mutation::run_queued`.
4. `features/atom/src/queue.rs` `QueuedCall`, `classify`, `CallTransport`,
   `CallQueue`, `provide_call_queue`.
5. `features/atom/src/persist.rs` `LocalRows`, `KvRows`, `provide_local_rows`,
   `use_persisted_store_list`; `features/atom/src/persist_turso.rs` `TursoRows`.
6. `features/atom/src/connection.rs` `use_connection`, `generation_now`: the
   connection state a page shows and a queue replays on.
7. `features/form/src/field.rs` `Field`, `use_field`;
   `features/form/src/validate.rs` `required`, `parse`, `min_length`, `email`;
   `features/form/src/components.rs` `TextField`, `TextArea`, `SelectField`,
   `Checkbox`.

`run_queued` runs the patch first (a temporary row), then spawns the call.
On success the store swaps the temporary row for the real one. If `classify`
says the server could not be reached, the row stays and the `QueuedCall` is
written to `LocalRows`, to replay in order on reconnect (or `kick`) and then
reload the store. Any other error rolls the patch back and reports it.
Without a queue provided, `run_queued` behaves as `run`.

## Tests to read

- Unit tests in `features/atom/src/store.rs`: temporary to real ids, rollback,
  `hydrate` keeping temporary rows, a queued mutation keeping its optimistic state.
- Unit tests in `features/atom/src/queue.rs`: transport errors queue and
  refusals do not.
- Unit tests in `features/atom/src/persist_turso.rs`: rows and the queue survive
  reopening the file.
- `crates/architect/tests/settings_file_kv.rs`: `KvRows` over files.
- Unit tests in `features/form/src/validate.rs`.
- The end-to-end offline test is Task's: `task:apps/server/tests/it/offline_queue_e2e.rs`.

## Status

Done in architect. Pitfalls: `run_queued` does not invalidate; an unknown
error classifies as a refusal and rolls back; temporary ids are local to the
process.

- **Task** uses the store, mutations, persisted lists and the queue
  (`task:crates/ui/src/stores.rs`, `task:crates/ui/src/offline.rs`); it does
  not use `architect-form` yet.
- **Studio** depends on both crates but does not use them in its source yet.
