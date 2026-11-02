+++
title = "Files"
description = "architect-files: an org's files in roots, a version per save, labels, resumable uploads and ranged reads over vox, quota, access by role and per-folder grants."
weight = 26
+++

## What it is and when to use it

`architect-files` is an organization's file layer. An app finds its root by a
directory name (`signal/impulse-responses`) and keeps files and folders in
it. Every save is a version, and a delete is a version too, so history
replaces a trash folder; versions can carry labels. Uploads are resumable and
deduplicated (bytes the org already holds are not sent), reads can be ranged,
and each org has a quota. Access comes from the caller's role in the org plus
per-folder grants; a path the caller may not see is reported absent, never
"denied".

Use it for files a person sees by name (impulse responses, song audio,
attachments). Bytes a document points at by hash are
[Blobs](@/features/blobs.md). There is no architecture page of its own; the
design is in [Documents](@/architecture/documents.md), section on files, and
the API in `skills/architect/references/files.md`.

## A minimal example

Feature `vox` for the client, `server` for `FilesServer` (native). From
`features/files/tests/files.rs`:

```rust
use architect::blob::LocalFs;
use architect_files::{FilesClient, PERMITS, STREAM_PERMITS, Save, serve, stream_layer,
    files_service_rpc_service_descriptor, files_service_stream_service_descriptor,
    server::FilesServer};
use architect_orgs::server::{ConfinedEngine, MembersResolver, confine};

// Server: every org's files, behind the gate, confined to the lane's org
let files = FilesServer::new(&data, LocalFs::new(data.join("blobs")), members.clone());
let router = LayerRouter::new()
    .with(files_service_rpc_service_descriptor(), serve(files.clone()))
    .merge(stream_layer(files.clone()));
let gate = Arc::new(
    PermissionsGate::new(Arc::new(ConfinedEngine::new(members.clone())),
                         Arc::new(MembersResolver::new(sessions, directory, members.clone())))
        .permit(files_service_rpc_service_descriptor(), &PERMITS)
        .permit(files_service_stream_service_descriptor(), &STREAM_PERMITS),
);
// On a transport of your own; an EngineHost lane returns
// `Lane::new(router).scoped(org)` from `vox_at` and the host does the same.
let handler = confine(OrgId::new("fts"), PermissionsGate::wrap_shared_with_bearer(gate, router, bearer));

// Client, on the org's lane
let files = FilesClient::new(caller);
let root = files.ensure_root("signal/impulse-responses", "Impulse responses").await?;
let saved = files.put(root.id, "Cab/4x12.wav", &wav, Save::create_only()).await?;
let bytes = files.read_range(root.id, "Cab/4x12.wav", 1_000, 1_010).await?;
```

`Save::replacing(etag)` saves over a known version and fails on a conflict.

## Code tour

1. `features/files/src/service.rs` `FilesService`: the wire: roots, browse,
   rename, move, copy, delete, the upload calls, `read`, history and restore,
   labels, grants, `events`. `PERMITS` and `STREAM_PERMITS` are its permit tables.
2. `features/files/src/client.rs` `FilesClient` (`ensure_root`, `put`, `get`,
   `read_range`, `resolve`, `events`), `Save`, `content_of`.
3. `features/files/src/server/mod.rs` `FilesServer` (`new`, `acting`,
   `set_quota`): the org comes from `current_org()`, the caller from the gate.
4. `features/files/src/server/acting.rs` `Acting`: the service for one caller
   in one org; changes go to a copy of the state, are checked against the
   quota, saved, then published as events.
5. `features/files/src/access.rs` `resolve`, `role_capabilities`, `Grant`:
   what a person may do at a path.
6. `features/files/src/server/state.rs` `State`: an org's tree, versions,
   labels, grants and uploads.
7. `features/files/src/path.rs` `RootPath`: paths inside a root, `..` refused.

`FilesClient::put` hashes the bytes and calls `begin_upload`; the server
answers with only the ranges it still needs (none if the org holds that
hash). The client streams those chunks on a vox channel, then calls
`complete_upload`. On the server, `FilesServer` builds an `Acting` for the
caller and org, checks the right through `access::resolve`, applies the change,
checks the quota, writes the org's state under `org/<id>/files/` in the data
directory, and publishes `FilesEvent`s, filtered per subscriber.

## Tests to read

`features/files/tests/files.rs` (feature `server`; a real vox link with
`confine`, the gate and `MembersResolver`): a member saves, reads, lists
versions and follows a pin; moves and copies keep history; a role decides
everywhere and a grant in its folder; outside what a person holds a path is
absent; an org sees only its own files and non-members are kept out; a quota
bounds an org's bytes; an upload resumes and known bytes are not sent; events
reach only who may see them; files survive a restart.

## Status

The core is done in architect. `FilesServer` refuses device (`Host`) and link
(`Guest`) callers, so files do not reach peers yet. Not yet: client sync
(selective sync, offline copies); the browser's ranged reads and an HTTP byte
face; desktop integrations (daemon, FUSE, File Provider, WebDAV); renditions,
search, review and share links; federation.

- **Task** runs its own Files crates (`task:features/files`); moving onto
  `architect-files` comes after the gaps above.
- **Studio** uses Task's files client over git; it moves when Task does.
