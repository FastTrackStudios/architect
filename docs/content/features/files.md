+++
title = "Files"
description = "architect-files: an org's files in roots, a version per save, labels, resumable uploads and ranged reads over vox and HTTP, renditions, quota, access by role and per-folder grants."
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

### In a browser

Feature `http` mounts the byte face, `WebFiles`, beside the vox lane. It
answers `Range` with `206`, so a `<video>` seeks against it, and it takes
uploads as plain resumable `PUT`s. It resolves each request's caller with the
gate's own identity resolver. A media element cannot send a header, so it
loads with a read ticket, minted over the org's lane. With
`FilesServer::with_renditions` every media version gets its renditions, made
off the request path: a proxy, a poster and peaks for a video, peaks and an
AAC transcode for audio. From `features/files/tests/web.rs`:

```rust
let files = FilesServer::new(&data, LocalFs::new(data.join("blobs")), members.clone())
    .with_renditions(FfmpegTranscoder::new());          // feature `ffmpeg`
let web = WebFiles::new(files.clone(), identity);       // the gate's resolver
let app = architect::layers![architect_files::web::Service].provide_http(&web);

// The page: a ticket for the folder, then plain URLs.
let ticket = files_client.ticket(root, "Cuts").await?;
let src = web::rendition_url("", &org, root, RenditionKind::Proxy, &path, None, Some(&ticket.token));
```

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
8. `features/files/src/web.rs` `FilesWeb`: the HTTP routes, and
   `content_url` / `rendition_url` for a page to build them (wasm-clean).
   `features/files/src/server/web.rs` `WebFiles` serves them.
9. `features/files/src/rendition.rs` `RenditionKind`, `RenditionSet`,
   `Peaks`; `features/files/src/server/renditions.rs` `Transcoder`,
   `Renditions`: what is made, how, and where it is kept.
   `features/files/src/server/ffmpeg.rs` `FfmpegTranscoder` is the real one.
10. `features/files/src/server/ticket.rs`: read tickets, signed with the
    server's key (`FilesServer::ticket_key`), checked again on every use.

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

`features/files/tests/web.rs` (feature `http`, a real socket): a browser reads
by range as its session, a viewer reads, nobody, a stranger and another org's
member do not, and past versions read by id. A ticket reads its folder only,
writes nothing, does not open when forged, and stops when its minter leaves
the org. An upload resumes over `PUT`s. A video's renditions are served
ranged. `features/files/tests/renditions.rs`: what each class yields; the
same bytes share renditions; renditions survive a restart and one whose blob
is gone is made again, alone; a failed kind is reported and not asked again.
The ignored `ffmpeg_makes_a_proxy_a_poster_and_peaks` runs the real `ffmpeg`.

## Synced roots: projects and asset libraries

Feature `sync` holds an org's projects (`<org>/Projects/<project>`) and
asset libraries (`<org>/Assets/<kind>`) in place on the headless node and
syncs them to the devices that pin them; the rest browse on demand over
WebDAV. Each root's manifest is a Loro document on the peer lanes, its
bytes travel `BlobTransfer`. The spec and its six decisions:
[Project files](@/architecture/project-files.md). Code:
`features/files/src/sync/engine.rs` `Engine`, `features/files/src/agent.rs`
`Files` (the `FILES_*` actions), `features/files/src/sync/dav.rs` `Dav`,
`apps/desktop/src/node/files.rs` (the node), and the Files extension
`features/files/extension/src/lib.rs` (navigator, pages). Tests:
`features/files/tests/sync.rs`, `apps/desktop/tests/files.rs`.

### Over S3

Feature `s3` serves the same roots over the S3 API, so `rclone`, `aws s3`
and `restic` read and write them. Set `s3_bind` (`ARCHITECT_S3_BIND`) on
the node, then issue a key on it:

```sh
architect-desktop serve --config node.toml s3-key issue \
  --org fts --for user:cody --role member --label laptop
# access_key_id = AKAR…   secret_access_key = …   (shown once)
aws --endpoint-url http://127.0.0.1:9000 s3 cp take.wav \
  "s3://fts/Projects/Right Here/Audio Files/take.wav"
```

- **A bucket per org, keys as on disk:** `Projects/<project>/<path>` and
  `Assets/<kind>/<path>`. A project's name is not a valid bucket name,
  the org is what a key and a role belong to, and one bucket is the
  org's tree as the node and the WebDAV mount show it, so a copy between
  projects moves no bytes.
- **Versions are the manifest's.** A `PutObject` or a completed multipart
  upload saves a version, whose id is the S3 version id;
  `ListObjectVersions` lists a path's history and a delete is a delete
  marker. History is append-only, so deleting one version is refused.
- **The `ETag` is the content's sha-256.** Clients that compare MD5s
  (`rclone check`) fall back to size.
- **Keys** are per person (`user:<id>`) or per device
  (`device:<endpoint>`), in one org, with a role. On every request the
  role is lowered to what the holder holds now (a revoked device's key
  opens nothing), then checked by `role_capabilities` and
  `access::resolve`, as `FilesServer`'s `Acting` checks it. `SigV4` is
  `s3s`'s (Apache-2.0).

Code: `features/files/src/sync/s3/mod.rs` `S3Face`,
`features/files/src/sync/s3/keys.rs` `S3Keys`, `apps/desktop/src/node/s3.rs`.
Test: `features/files/tests/s3.rs`.

## Status

The core is done in architect, and so are the HTTP byte face, the browser's
ranged reads and renditions. Synced roots (`sync`) are done: client sync with
pins per machine and offline copies, the node as the device's agent, WebDAV
with fetch-on-open, versions and checkpoints, assets, search, waveforms, and
the Files extension, and the S3 face. `FilesServer` still refuses device (`Host`) and link
(`Guest`) callers and does not read synced roots. Not yet: FUSE and the
macOS File Provider (WebDAV covers both systems meanwhile); a service worker
that answers `Range` from offline copies; filmstrips; review and share links
of files; federation.

- **Task** runs its own Files crates (`task:features/files`); moving onto
  `architect-files` comes after the gaps above.
- **Studio** uses Task's files client over git; it moves when Task does.
