+++
title = "Documents"
description = "Co-edited state as Loro documents: CrdtDoc, kinds of document with schema stamps and migrations, named undo, DocRegistry, crdt entities and the sync hooks."
weight = 20
+++

## What it is and when to use it

Co-edited, offline-first state lives in a Loro document (`CrdtDoc`), one per
collaboration boundary, with typed repos over it and a pluggable persistence.
A `DocKind` says what a document is: its id, display name, file extension,
schema version with the migrations up to it, and which blobs it points at.
`History` gives named undo over transactions: each step has a name, and undo
takes back only this peer's edits. On a server, `DocRegistry` serves many
documents through one mounted sync and presence pair; on a client, the hooks
keep a replica synced and turn its changes into renders.

Use a document for anything people edit together or offline (a chart, a
setlist, a note). Server-owned rows stay on the optimistic store
([Atom and forms](@/features/atom-and-forms.md)). Design:
[Documents](@/architecture/documents.md) and
[Collaboration](@/architecture/collaboration.md).

## A minimal example

The `crdt` crate (`features/crdt/crdt`): kinds, history and `CrdtDoc` are in
the default build; `vox` adds sync and the registry; `dioxus` the hooks.

```rust
use crdt::{DocKind, History, Migration};
use loro::LoroDoc;

let kind = DocKind::new("studio.song", "Song")
    .extension("session")
    .schema(2, vec![Migration { from: 1, name: "add a title", run: add_title }]);
let doc = LoroDoc::new();
kind.stamp(&doc);            // meta.kind and meta.schema
kind.upgrade(&doc)?;         // Ok(2); refuses a document newer than the kind

let mut history = History::new(&doc, 100);
history.transact(&doc, "Type hello", |d| d.get_text("body").insert(0, "hello").unwrap());
assert_eq!(history.undo().as_deref(), Some("Type hello"));
```

An entity stored in a document (the crate's `crdt` feature turns on `dep:crdt`):

```rust
#[architect::entity(table_name = "notes", repo, crdt)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub title: String,
    #[architect(crdt(text))]
    pub body: String,
}

let repo = NoteRepoLoro::new(&CrdtDoc::ephemeral());
repo.create(NoteCreate { title: "plan".into(), body: "hi".into() }).await?;
```

Served, and followed from a client (from `features/crdt/crdt/tests/registry.rs`):

```rust
let registry = DocRegistry::new(|_id| Box::pin(async { Ok(CrdtDoc::ephemeral()) }));
let router = LayerRouter::new()
    .with(doc_sync_service_descriptor(), DocSyncDispatcher::new(registry.clone()))
    .with(doc_presence_service_descriptor(), DocPresenceDispatcher::new(registry));

// Dioxus: let doc = crdt::use_synced_doc_keyed(doc_id);  doc.status(), doc.repo::<Note>()
```

## Code tour

1. `features/crdt/crdt/src/kinds.rs` `DocKind` (`stamp`, `upgrade`, `blobs`),
   `DocKinds` (`register`, `for_extension`), `stamp_of`, `kind_of`.
2. `features/crdt/crdt/src/history.rs` `History`: `transact` is one named
   step; `begin`, `commit` and `abort` hold one step open across many
   transactions; `undo` and `redo` return the step's name.
3. `features/crdt/crdt/src/lib.rs` `CrdtDoc`: `open` loads snapshot and
   updates and forwards each local commit to persistence; `ephemeral`, `repo`,
   `compact`.
4. `features/crdt/crdt/src/lib.rs` `EntityCrdt`, `LoroRepo`: the per-entity
   codec and typed CRUD, which `crates/architect-derive/src/lib.rs` emits for a
   `crdt` entity.
5. `features/crdt/crdt/src/registry.rs` `DocRegistry` (`with_admission`,
   `with_permissions`, `with_authorizer`, `with_idle_eviction`) and
   `DocPermissions`: which documents a caller may read or write.
6. `features/crdt/crdt/src/sync.rs` `DocSyncHost`, `SyncedDoc`: the hub
   protocol; `SyncedDoc::durable` persists what it receives.
7. `features/crdt/crdt/src/hooks.rs` `use_synced_doc_keyed`, `use_doc_slot`,
   `use_crdt_list`: the replica as Dioxus state.

A derived create calls `LoroRepo::create`, which inserts a map and commits.
The commit fires the document's local-update subscription: `CrdtDoc` appends
the bytes to its persistence and `SyncedDoc` sends them up. On the server,
`DocRegistry` finds or opens that document's host and fans the update out;
on the other client, the hook's root subscription bumps a revision signal and
`use_crdt_list` reads again. Wrap edits in `History::transact` for named undo;
the [document file](@/features/document-file.md) does this for you.

## Tests to read

- `features/crdt/crdt/tests/history.rs`: named steps, held steps, undo of
  one's own edits only, a kind stamping and upgrading a document one migration
  at a time.
- `features/crdt/tests/derive/tests/crdt_text.rs`: a derived repo; `crdt(text)`
  merges concurrent edits, a plain `String` is last-writer-wins.
- `features/crdt/crdt/tests/sync_convergence.rs`: replicas converge, a late
  joiner catches up, an offline restart pushes back.
- `features/crdt/crdt/tests/registry.rs`: two documents through one
  dispatcher pair, the admission gate, evict and reopen.
- `features/crdt/crdt/tests/backlog_chunks.rs`: a long backlog goes in frames
  under the link's limit.

## Status

Done in architect. Not yet: undo that survives reopening a document (Loro's
undo manager cannot be seeded). The hooks have no tests of their own. Never
hold a strong `CrdtDoc` inside a Loro subscription closure (it deadlocks on
drop); `skills/architect/references/crdt-peer-sync.md` lists the pitfalls.

- **Task** serves documents with `DocRegistry`
  (`task:apps/server/src/app_docs.rs`) without `with_permissions`, and uses the
  presence hooks; it does not use `DocKind`, `History` or `crdt` entities.
- **Studio** uses `DocRegistry` (`studio:features/session/src/sync/net.rs`)
  and its own undo manager; `DocKind`, `History` and the hooks are still to adopt.
