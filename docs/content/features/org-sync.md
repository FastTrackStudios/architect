+++
title = "Org sync driver"
description = "OrgSyncs, OrgDocs and Placement: a device keeps every document of each of its orgs synced, offline-safe, in the files the person opens."
weight = 22
+++

## What it is and when to use it

A signed-in device keeps one `OrgDocs` per organization. Each holds a
`DocsDriver` (the device's index of the org's documents and a sync per
document) and a `Placement` (which file each document lives in). `OrgDocs`
stays connected on its own: dial the org's lane, sync until the connection
closes, wait a jittered backoff (250 ms to 10 s, reset on each connect), dial
again. `OrgSyncs` makes one `OrgDocs` per org on first ask, follows the
person's current org, and stops the orgs they leave (their files stay).
Offline, every document still edits locally; the next session is a delta
both ways.

Use it on any device that syncs an org's documents, instead of a hand-written
reconnect loop. Design: [Documents](@/architecture/documents.md), section on
the org driver; for org scope see [Organizations](@/features/organizations.md).

## A minimal example

`crdt` with `vox`, native only. From `features/crdt/crdt/tests/org_sync.rs`:

```rust
use crdt::docs::{Dialed, DocsLane, InMemoryDocsIndex, OrgDocs, OrgSyncs, Placement};
use crdt::{DocKind, DocKinds};

let mut kinds = DocKinds::new();
kinds.register(DocKind::new("test.note", "Note").extension("note"));

let syncs = OrgSyncs::new(
    move |org: Org| {
        let placement = Placement::new(dir.join(&org.slug), kinds.clone());
        async move { Ok(OrgDocs::new(org, InMemoryDocsIndex::new(), placement)) }
    },
    move |org: Org| async move {
        match lane_for(&org).await {           // the app's dialer, a fresh token each try
            Ok(caller) => Dialed::Lane(DocsLane::over(&caller)),
            Err(e) => Dialed::Unreachable(e.to_string()),   // Dialed::Stop once signed out
        }
    },
);

let docs = syncs.current(&orgs).await?;          // None while the window shows All
// docs.placement().place(id, project_path);     // optional, before create or open
// docs.driver().create(CreateDoc { .. }).await?;
// docs.watch_link(): Connecting, Live, Offline, Stopped
syncs.follow(&fresh_orgs).await?;                // stop the orgs left
```

The server side is a `DocsHost` over a `DocRegistry`, both with
`DocPermissions::new(engine).in_org(&org.id)`. An app that already supervises
its connection calls `docs.run_on(&lane)` once per connection generation
instead of letting `OrgDocs` dial.

## Code tour

1. `features/crdt/crdt/src/docs/org.rs` `OrgSyncs`: `new(open, dial)`,
   `ensure` (one driver even when two callers ask first at once), `current`,
   `follow`, `stop`.
2. `features/crdt/crdt/src/docs/org.rs` `OrgDocs`: `driver`, `placement`,
   `link`, `watch_link`, `run_on`, `stay_connected`; `Dialed` and `OrgLink`
   beside it.
3. `crates/architect/src/reconnect.rs` `Backoff`: the floor, cap, full jitter
   and reset.
4. `features/crdt/crdt/src/docs/driver.rs` `DocsDriver` (`create`, `open`,
   `run`), `DocsLane`, `DocsIndex`, `InMemoryDocsIndex`.
5. `features/crdt/crdt/src/docs/placement.rs` `Placement`: `place`,
   `file_for` (the placed path, else `<dir>/<id>.<ext>`), and the
   `DocStorage` impl that opens a `DocFile` and hands out its persistence.
6. `features/crdt/crdt/src/docs/host.rs` `DocsHost` and
   `features/crdt/crdt/src/registry.rs` `DocPermissions`: the server half.

`OrgSyncs::ensure` calls the app's `open` and spawns `stay_connected`. Each
pass sets the link to `Connecting` and calls `dial(org)`. On a lane it resets
the backoff and runs `run_on`, which sets `Live` and runs
`DocsDriver::run`: register pending creates, catch up the catalogue, run a
durable `SyncedDoc` per document, until a lane closes. Then `Offline` and a
backoff sleep. Each document's bytes go through `Placement` into a
[document file](@/features/document-file.md), so every step that arrives is
in the file the person opens.

## Tests to read

- `features/crdt/crdt/tests/org_sync.rs`: a member's document syncing through a
  dropped connection and merging offline edits into `<org>/<id>.note`, then
  `follow` stopping the org; a document made on another device landing in a
  file the app placed; a non-member refused on the server.
- `features/crdt/crdt/tests/docs.rs`: the driver and host underneath: devices
  converge, an offline device learns of a delete, an edit survives an offline
  restart, lineage rules, devices syncing while the hub is down.

## Status

Done in architect. Two processes on one device must not share an index.

- **Task** serves the hub side (`DocsHost` and `DocRegistry` in
  `task:apps/server/src/app_docs.rs`, without `with_permissions`) and uses
  `DocsDriver` from its CLI (`task:apps/cli/src/project.rs`); its apps do not
  run `OrgSyncs` yet.
- **Studio** does not use it yet; moving its sync onto it is Studio's work.
