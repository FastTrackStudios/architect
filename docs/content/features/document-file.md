+++
title = "The document file"
description = "crdt::file: one append-only file per document, every finished step durable, Save as name and fold, crash recovery, branches, versions and blobs."
weight = 21
+++

## What it is and when to use it

`crdt::file` keeps a document in one file. Every `Branch::transact`, `undo`
and `redo` is on disk before it returns, so there is no unsaved state: Save
means "name this version and fold the steps into a snapshot". A file holds
the document's kind and schema, its branches (forked from `main`), every
named step, checkpoints, snapshots, and blobs by SHA-256. Opening after a
crash drops the torn tail and keeps every finished step. The format is plain
bytes, so a browser reads it with `DocFile::from_bytes`.

Use it for anything a person opens as a file. Give a `CrdtDoc` the file's
persistence to sync it while the steps stay in the file. Design:
[Documents](@/architecture/documents.md), sections on the file and on
versions and branches.

## A minimal example

In the default build of `crdt`, wasm-clean; `DiskStorage` is native only.
From `features/crdt/crdt/tests/file.rs`:

```rust
use crdt::file::{DiskStorage, DocFile, MAIN};
use crdt::DocKind;

let kind = DocKind::new("test.note", "Note").extension("note");
let file = DocFile::create(DiskStorage::open(&path)?, &kind).await?;
let mut main = file.open_branch(MAIN).await?;

main.transact("Type hello", |d| d.get_text("body").insert(0, "hello").unwrap()).await?;
main.save("Sent to Dan").await?;                 // a checkpoint, then a fold

let alt = main.fork("acoustic").await?;
main.merge(&alt).await?;                         // a step named "Merge acoustic"

let v = file.versions(MAIN).await?[1].clone();
let old = file.open_version(&v).await?;          // read only
let back = file.restore(&v, "back to one").await?;
let hash = file.put_blob("fx", &bytes).await?;   // sha256 hex

let replica = CrdtDoc::open_shared(id, Arc::new(file.persistence(MAIN))).await?;
```

Edit through the `Branch`, not straight on its `loro()`: a direct edit is
written only with the next step.

## Code tour

1. `features/crdt/crdt/src/file/mod.rs` `DocFile`: `create`, `create_as`,
   `adopt`, `open`, `open_any`, `from_bytes`, `to_bytes`; `open_branch`,
   `versions`, `open_version`, `restore`, `delete_branch`; `put_blob`, `blob`;
   `recovered`; `persistence`.
2. `features/crdt/crdt/src/file/branch.rs` `Branch`: `transact`, `undo`,
   `redo`, `flush`, `save`, `checkpoint`, `fork`, `merge`, `import`.
3. `features/crdt/crdt/src/file/format.rs` `Record`, `frame`, `parse`: a
   12-byte header (`ARCHDOC\0` and a format number), then records framed as
   length, payload, CRC32.
4. `features/crdt/crdt/src/file/index.rs` `Index`: the records folded in
   memory; rebuilds a branch from its snapshot and steps.
5. `features/crdt/crdt/src/file/storage.rs` `Storage` (`read`, `append`,
   `replace`, `truncate`), `MemoryStorage`, `DiskStorage`.
6. `features/crdt/crdt/src/file/persistence.rs` `BranchPersistence`: a
   branch as a `CrdtDoc`'s persistence; synced updates land as steps.

`Branch::transact(name, f)` runs `f` inside `History::transact`, one undo
group, then flushes: it exports the update and `DocFile` appends one framed
step record through `Storage::append`, which syncs to disk before returning.
`save` writes a checkpoint and folds: it rebuilds the branch from the file
(keeping another handle's steps), adds a snapshot, and replaces the file
atomically (write to a temp file, sync, rename, sync the directory).
Reopening parses the records and cuts off the first frame that runs past the
end, fails its CRC or doesn't decode; `recovered()` reports it. `open_branch`
upgrades an old branch to the kind's schema as a step of its own.

## Tests to read

- `features/crdt/crdt/tests/file.rs`: every finished step on disk when it
  returns; a torn write dropped; a tail cut anywhere reopening with the steps
  before it; `kill -9` mid-edit; save naming a version and folding; branches
  forked, opened side by side, merged and deleted; a version restored as a new
  branch; two handles on one branch; blobs kept once by hash; the kind checked
  and a branch upgraded; a `CrdtDoc` keeping its replica in a branch.
- Unit tests in `features/crdt/crdt/src/file/format.rs`: every record
  round-trips; a corrupt tail is cut at the last whole record.
- `features/crdt/crdt/tests/org_sync.rs`: synced documents ending up in placed files.

## Status

Done in architect. Not yet: blobs kept beside the file and their
reachability; an IndexedDB `Storage`; undo that survives reopening; one
writer per file is assumed (two processes appending would interleave).

- **Task** does not use it.
- **Studio** still keeps sessions in its own store
  (`studio:crates/daw/session-format/src/store/mod.rs`); moving that format
  onto `DocFile` is Studio's work.
