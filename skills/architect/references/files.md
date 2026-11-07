# Files: an org's roots, versions, uploads and access

`architect-files` (`features/files`) is the file layer: roots of files and folders an app keeps its things in, a version per save (a delete is one too), labels on versions, resumable uploads and ranged reads over vox channels, an org quota, and access by the caller's role in the org plus grants on folders. Design: `docs/content/architecture/documents.md` ("Files"). Ported from Task's `task:features/files`; what has not moved yet is listed at the end.

## Pieces

| Flag | What | Entry points |
|---|---|---|
| default (wasm-clean) | ids, paths, the model, the fault, capabilities and their resolution | `features/files/src/path.rs` `RootPath` (`parse` refuses `..`; `validate` what the wire decoded); `features/files/src/model.rs` `Root`, `Entry`, `Version`, `VersionLabel`, `Usage`, `OnConflict`, `Expect`, `FilesEvent`; `features/files/src/error.rs` `FilesFault`; `features/files/src/access.rs` `Capability`, `Grant`, `role_capabilities`, `stewards`, `resolve` |
| `vox` | the service, its gate tables, the app's client | `features/files/src/service.rs` `FilesService` (`#[subscribe] events`), `PERMITS`, `STREAM_PERMITS`; `features/files/src/client.rs` `FilesClient` (`ensure_root`, `put` + `Save`, `get`, `read_range`, `resolve`, `get_ref`, `events`), `content_of` |
| default (wasm-clean) | renditions' model, the byte face's routes and URLs | `features/files/src/rendition.rs` `RenditionKind` (`tag`, `mime`, `ladder`), `MediaClass`, `RenditionSet` (`state`, `settled`), `RenditionState`, `Ticket`, `Peaks` (`audiowaveform` JSON v2), `PeakReducer`, `RECIPE`; `features/files/src/web.rs` `FilesWeb` (an HTTP-only service, prefix `files/{org}`), `content_url`, `rendition_url` |
| `server` (native) | every org's files on a server, and their renditions | `features/files/src/server/mod.rs` `FilesServer` (`new`, `default_org`, `with_grants`, `acting`, `set_quota`, `with_renditions`, `with_rendition_engine`, `rendition_engine`, `ticket_key`), `GrantSource` (grants from elsewhere — a space's members — asked per call); `features/files/src/server/ticket.rs` `TicketKey` (`seal`/`unseal` any claims under a prefix: another feature's tickets on the same kind of key), `who` and `principal_of` (exported as `ticket_subject`, `ticket_principal`); `features/files/src/server/acting.rs` `Acting` (in process: `locate`, `rendition`, `receive`); `features/files/src/server/renditions.rs` `Transcoder`, `TranscodeError`, `Renditions` (`new`, `concurrency`, `set`, `ready`, `schedule`, `build`) |
| `http` (native) | the byte face a browser reads by range | `features/files/src/server/web.rs` `WebFiles` (`new(files, identity)`) |
| `ffmpeg` (native) | renditions by the `ffmpeg`/`ffprobe` on `PATH` | `features/files/src/server/ffmpeg.rs` `FfmpegTranscoder` (`new`, `with_binaries`, `available`) |
| `sync` (native) | synced roots: projects and asset libraries held in place, pinned per machine | `features/files/src/sync/manifest.rs` `Manifest` (`meta`, `push`, `versions`, `checkpoints`), `Tree` (`heads`, `settled`, `current`, `live`, `history`, `descends`, `ever_held`), `FileVersion`, `Checkpoint`, `RootMeta` (`place`, `doc_id`), `Section`, `doc_id_for`, `lane_name`, `place_of`, `conflict_path`; `features/files/src/sync/layout.rs` `discover`, `root_id`, `ignored`, `is_session_file`; `features/files/src/sync/scan.rs` `Scanner`, `Index`, `SETTLE`, `hash_settled`; `features/files/src/sync/store.rs` `TreeStore` (a `BlobStore` and a `Remote` that also answers from working files); `features/files/src/sync/engine.rs` `Engine` (`pass`, `pass_all`, `run`, `pin`, `unpin`, `restore`, `restore_checkpoint`, `make_checkpoint`, `save_bytes`, `status`), `EngineConfig`; `features/files/src/sync/lane.rs` `BLOB_PERMITS`, `Engine::blob_host`, `Engine::held`; `features/files/src/sync/dav.rs` `Dav` (`handle`: the WebDAV face), `mime_of`; `Engine::relocate`, `Engine::save_reader`; `features/files/src/sync/assets.rs` `frontmatter`, `links`, `assets_of`, `wav_peaks`, `Engine::assets`, `Engine::search`, `Engine::peaks`, `Engine::with_transcoder` |
| `drive` (Linux, macOS) | every root a device knows mounted as a real folder (FUSE via `fuser`) | `features/files/src/drive/mod.rs` `mount`, `Mounted`, `guard_main`, `clear_stale`, `is_mounted`, `XATTR_PINNED`, `XATTR_STATE`; `features/files/src/drive/fs.rs` `DriveFs`; `features/files/src/drive/cache.rs` `Cache` (all fetching and evicting); `Engine::set_drive`, `Engine::set_online`, `Engine::drive_info` |
| `s3` (native) | synced roots over the S3 API, `SigV4` access keys | `features/files/src/sync/s3/mod.rs` `S3Face` (`new`, `service`, `serve`); `features/files/src/sync/s3/keys.rs` `S3Keys` (`issue`, `list`, `revoke`, `get`), `S3Key`, `KeyHolder`, `KeyRoles`, `AsIssued`; `features/files/src/sync/s3/map.rs` `split_key`, `key_of`, `page`; `Engine::save_content` |
| `agent` | the device's files agent as a service | `features/files/src/agent.rs` `Files` (`roots`, `browse`, `pin`, `unpin`, `history`, `restore`, `checkpoint`, `checkpoints`, `restore_checkpoint`, `assets`, `search`, `peaks`; `FILES_*` actions, `FilesHttpClient`), `AgentError`; `features/files/src/roots.rs` `RootStatus`, `FileEntry`, `FileVersion`, `Checkpoint`, `RootMeta`, `Section`, `Asset`, `Media`, `Hit` (wasm-clean) |

## Rules it keeps

- **The org is the lane's**, never an argument: `architect_orgs::server::current_org()` (or `FilesServer::default_org` on a one-org server). Data lives under `<data>/org/<id>/files/` (`OrgId::path`), bytes in the `architect::blob::LocalFs` the server shares with documents.
- **The caller is the gate's** (`architect::permissions_gate::caller()`). No principal is `FilesFault::Unauthenticated`, never the server's own; in-process jobs use `FilesServer::acting(org, Principal::Service { .. })`.
- **Role, then grant.** Owner/admin/member hold every capability everywhere in the org (owner/admin also rename and release roots); viewer reads and reads history. A `Grant` adds capabilities over one folder for one user id. Outside what a person holds a path is `PathNotFound`, never `Denied`; folders above a grant are visible and hold nothing.
- **The gate checks membership, the feature checks the path.** `PERMITS` marks every method `read` on `files/**`; pair it with `ConfinedEngine` (membership) so a viewer granted `Write` on a folder still reaches `complete_upload`.
- **History replaces trash.** Delete appends a delete version; `restore` brings it back. A move keeps a file's history; a copy starts its own.
- **Saves are safe.** `Save::replacing(etag)` refuses with `Stale` when the file moved on; a taken destination is the caller's choice (`OnConflict`), never the server's.
- **Dedup is per org.** An upload naming content the org already references sends nothing; content only another org holds is not proof of having the bytes.
- **The byte face is not behind the vox gate.** `WebFiles` resolves the caller itself: a session through the gate's own `IdentityResolver` (pass the `MembersResolver`, so orgs are learnt), from `Authorization: Bearer` or `?token=`; then membership of the URL's org (`403` otherwise, as `ConfinedEngine` refuses), then role and grant per path as on vox. `FilesFault` implements `architect::http::HttpError` (`401`/`403`/`404`/`409`/`412`/`503`/`507`, codes like `path_not_found`, `rendition_pending`).
- **Media elements use tickets.** `<video>`/`<audio>`/`<img>` cannot send a header: mint `FilesService::ticket(root, path)` over the lane (needs `Read`), put `token` in `content_url`/`rendition_url`. A ticket (`fft1.…`, HMAC-SHA256, `TICKET_TTL_US` = 6 h) reads its path and below in one root, never writes, and is re-checked against the minter's access on every use. Its key is random per process unless `FilesServer::ticket_key` sets one (share it across replicas).
- **Renditions are off the request path.** Every `Changed` entry with content schedules its source (`Renditions::schedule`, `concurrency` builds at a time, default 1); a request reports `RenditionState::Pending` (`503 rendition_pending` on the byte route) and schedules what is missing. Keyed by the version's content (same bytes share them), addressed by path + version. Video yields `Proxy` (H.264/AAC MP4, ≤720p, faststart), `Poster` (JPEG), `Peaks`; audio `Peaks`, `Audio` (AAC MP4); other nothing. Records live at `<data>/renditions/<hh>/<hex>.json` (atomic, per rendition): they survive a restart, a record of another `RECIPE` or one whose blob is gone is made again. Failures are remembered until restart (`RenditionState::Failed`, `RenditionSet::problem` for a failed probe), then retried.
- **Quota** counts live files' bytes (`State::used`); every change that grows it past `Usage::quota` fails with `QuotaExceeded`. The deployment sets it (`FilesServer::set_quota`), not a member.

## Synced roots (`sync`)

The spec: `docs/content/architecture/project-files.md`. A root is `<org>/Projects/<project>` or `<org>/Assets/<kind>`; its manifest is one Loro document (an append-only `versions` list, `checkpoints`, `meta`), id `RootMeta::doc_id` derived from the place so two devices that find one tree seed one document. The node (`apps/desktop/src/node/files.rs`) runs the engine with `files_dir` (`files_hold` on the always-on node), opens manifests through `VaultCollab::with_plain_docs`, lists them in its catalogue as `files/<place>` and serves bytes as `BlobTransfer` on the peer lane (`crdt::p2p::node_router_with`).

- **Working files are the truth.** The node serves the tree in place; Task's `.fts-files` and `.fts-root.json` are never synced (`layout::ignored`), the marker's id is read (`root_id`). A `#fts-stub v1` file is never content.
- **Scan, then apply.** A change is committed once settled (same stat twice, mtime `settle` old, same stat around the hash). Incoming bytes go to `<file>.architect-part` and are renamed into place; a file whose stat differs from the index is never overwritten.
- **Forks keep both.** `Tree::settled`: the newest content (`at_us` = the file's mtime) wins; the others are written as `conflict_path(path, by)`; one version replacing every head closes it; heads with the same content are settled.
- **Versions are kept by size.** Up to `keep_inline` (`KEEP_INLINE`, 64 MiB) a committed file is copied into the store; a larger one stays in place, and is moved into the store when sync replaces it.
- **Unpin only when synced.** `Engine::unpin` refuses with `EngineError::NotSynced(paths)`.
- **Assets are library children.** A library is `<org>/Assets/<kind>`; an asset is a folder with a note of its own name, or a note alone. Its frontmatter is its properties, `stems` (else the folder's sound files) its media, `[[links]]` what it references. An unpinned library's notes are fetched anyway, so assets browse offline. Peaks: WAV read here (`wav_peaks`), anything else through the engine's `Transcoder`; kept at `<data>/peaks/<content>.json`.
- **The window reaches the agent through `FILES_*`.** `apps/ui/src/files.rs` registers the `Files` actions over `FilesHttpClient` at `ARCHITECT_FILES_AGENT`, each call on a tokio runtime of its own (the window's executor is not tokio). The Files extension (`features/files/extension`, id `files`) draws on those: its navigator provider (`FILES_NAV_*`: org → Projects, Assets → libraries; items a project's top entries or a library's assets) and the `root` and `asset` pages, opened with `FILES_OPEN {root, asset?, path?}`. Ids come over the wire as `["uuid"]` (`architect_files_extension::id` reads both forms).
- **The drive is a real folder.** The node mounts it with `drive` in its config (`ARCHITECT_DRIVE`; docs: `docs/content/features/drive.md`). Pinned roots pass through to their working copy; unpinned ones list from the manifest, fetch on open (reads stream while the bytes arrive) and stay cached, and writes are temp copies saved as versions on `flush`/`fsync`/`release` (`close(2)` waits for flush, not release). `setfattr -n user.architect.pinned -v 1|0` pins or unpins; `user.architect.state` reads `pinned|cached|fetching|online`. A guard process (`drive-guard <mount>`, the same binary) unmounts after a crash; a stale `ENOTCONN` mount is cleared at the next start. `architect-desktop agent install|uninstall|start|status` (`features/files/src/drive/agent.rs` `Defaults`, `install`, `systemd_unit`, `launchd_plist`, `set_key`, `get_key`) installs the agent as a systemd user unit or launchd agent with `agent.toml` defaults (drive `~/Architect`, store `<data>/roots`, bind `127.0.0.1:18471`); the window joins it (`apps/ui/src/files.rs` `join_agent`), starting it when it is down. The Files extension's `drive` view (also a settings section, and `FILES_DRIVE_OPEN`) shows `Files::drive` and edits the agent's `drive`/`files_dir` through the window's `DRIVE_CONFIG`/`DRIVE_CONFIGURE` (`ARCHITECT_AGENT_CONFIG` overrides which config), restarting the installed agent. Offline, an uncached file fails `ENETUNREACH`.
- **The agent faces the device only.** A node bound to loopback (a laptop's agent) serves `Files` over HTTP (`POST /files/<method>`) and WebDAV at `/dav/<org>/<section>/<name>/…`; a node bound wider serves `FILES_*` over `/mcp` only, behind its bearer. WebDAV lists from the manifest (offline), streams a missing file from a peer while it arrives (`Fetcher::open_at`) and keeps it, and a `PUT` is a version.

- **The S3 face** (`s3`, the node's `s3_bind` / `ARCHITECT_S3_BIND`): `S3Face` over the engine, on its own port, path-style. A bucket per org (its folder under `files_dir`), keys `Projects/<project>/<path>` and `Assets/<kind>/<path>`. `PutObject` and a completed multipart upload save a version (its id is the S3 version id); `ListObjectVersions` is a path's history, a delete a delete marker; deleting one version is refused (history is append-only). The `ETag` is the content's sha-256, a part's its MD5. `SigV4` (via `s3s`, Apache-2.0) against `S3Keys` (`<data_dir>/files/s3-keys.json`, 0600; `architect-desktop serve s3-key issue --org <org> --for user:<id>|device:<endpoint> --role <role>`). Each request: the key's role lowered to its holder's now (`KeyRoles`; the node's is the device's `Admission` grant), then `role_capabilities` and `access::resolve`, as `Acting`. A path the key may not see is `NoSuchKey`; another org is `NoSuchBucket`. `CopyObject` within the bucket moves no bytes (`Engine::save_content`).

Tests: `features/files/tests/s3.rs` (a real socket and `object_store`'s S3 client: list, range, multipart, copy, versions, roles, revoke), `features/files/tests/sync.rs` (two engines, documents exchanged by hand), `apps/desktop/tests/files.rs` (two nodes over iroh: a pinned project both ways; an unpinned one over WebDAV, opened on demand, written, pinned and unpinned over HTTP).

## Recipe: mount it

```rust
let files = FilesServer::new(&data, LocalFs::new(data.join("blobs")), members.clone());
let router = LayerRouter::new()
    .with(files_service_rpc_service_descriptor(), architect_files::serve(files.clone()))
    .merge(architect_files::stream_layer(files.clone()));
let gate = PermissionsGate::new(Arc::new(ConfinedEngine::new(members.clone())), Arc::new(identity))
    .permit(files_service_rpc_service_descriptor(), &architect_files::PERMITS)
    .permit(files_service_stream_service_descriptor(), &architect_files::STREAM_PERMITS);
// an EngineHost lane at /org/{slug}/vox (the host gates inside the org):
host.vox_at("/org/{slug}/vox", move |head| { let router = router.clone(); async move { Ok::<_, StatusCode>(Lane::new(router).scoped(org_of(&head)?)) } })
// or per connection on a transport of your own:
confine(org, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer))
```

`features/files/tests/files.rs` is the whole loop over a real vox link: roles, grants, absence, quota, resumed and deduplicated uploads, per-subscriber events, restart.

## Recipe: the byte face and renditions

```rust
let files = FilesServer::new(&data, LocalFs::new(data.join("blobs")), members.clone())
    .with_renditions(FfmpegTranscoder::new())   // feature `ffmpeg`; any `Transcoder` works
    .ticket_key(key);                            // shared by every replica
let web = WebFiles::new(files.clone(), gate.identity_resolver());
let app = architect::layers![architect_files::web::Service].provide_http(&web);
// routes: GET files/{org}/{root}/content/{*path}, …/rendition/{kind}/{*path},
// …/renditions/{*path} (?version=&token=); POST files/{org}/uploads,
// PUT/GET/DELETE …/uploads/{upload} (?offset=), POST …/uploads/{upload}/complete
```

Page side: `let t = files.ticket(root, "Cuts").await?;` then `web::rendition_url("", &org, root, RenditionKind::Proxy, &path, None, Some(&t.token))` as the `<video src>`; poll `files.renditions(root, path, None)` until `settled()`. Tests: `features/files/tests/web.rs`, `features/files/tests/renditions.rs` (a fake `Transcoder`; the ignored `ffmpeg_makes_a_proxy_a_poster_and_peaks` runs real `ffmpeg`).

## Recipe: an app's store

```rust
let files = FilesClient::new(caller); // the org lane's vox Caller
let irs = files.ensure_root("signal/impulse-responses", "Impulse responses").await?;
let saved = files.put(irs.id, "Cab/4x12.wav", &wav, Save::create_only()).await?;
let pin = saved.content; // the etag: Save::replacing(pin) next time
```

## Attachments: files on a document or a record

`architect-attachments` (`features/attachments`) hangs a blob off a `Parent` (`Parent::doc(id)`, `Parent::record(kind, id)`), named by its sha256. Design: `docs/content/architecture/documents.md` ("Attachments"); page: `docs/content/features/attachments.md`.

| Flag | What | Entry points |
|---|---|---|
| default (wasm-clean) | the model, a document's own list, the byte face's URLs | `features/attachments/src/model.rs` `Parent` (`resource`, `parse`, `validate`), `Attachment`, `Upload`, `ReadRequest`, `AttachUsage`, `AttachFault`; `features/attachments/src/doc.rs` `attach`, `detach`, `list`, `get`, `refs` (a `crdt::BlobRefs` for `DocKind::blobs`); `features/attachments/src/web.rs` `AttachmentsWeb`, `content_url`, `rendition_url` |
| `vox` | the service, its gate table, the client | `features/attachments/src/service.rs` `AttachmentsService` (`begin`, `send`, `attach`, `detach`, `list`, `size`, `read`, `renditions`, `ticket`, `usage`), `PERMITS` (`read` on `**`: the parent decides); `features/attachments/src/client.rs` `AttachmentsClient` (`push`, `read`), `attachment_of`, `hash_of` |
| `blob` (native) | a device's side | `features/attachments/src/blob.rs` `attach_local` (store + pin + doc), `AttachmentsClient::push_from`, `AttachmentsClient::push_doc`, `AttachmentsRemote` (a `Remote` for `Fetcher`, with the parent's permissions) |
| `server` (native) | every org's attachments | `features/attachments/src/server/mod.rs` `AttachmentsServer` (`new(data, blobs, engine)`, `with_docs`, `with_renditions`, `ticket_key`, `default_org`, `acting`, `set_quota`, `held`, `release_unreferenced`), `DocSource` |
| `http` (native) | the byte face | `features/attachments/src/server/web.rs` `WebAttachments` (`new(server, identity)`) |

Rules: access is the parent's, by the gate's engine on `org/<org>/doc/<id>` or `org/<org>/<kind>/<id>` (`read` to list and read, `write` to send and attach), so roles and spaces hold; a parent reads only bytes it points at; a document's list lives in the document (offline, synced), a record's on the server; the quota counts distinct blobs. Test: `features/attachments/tests/attachments.rs`.

## Not in architect yet

The browser integration (service worker answering `Range` from offline copies), the macOS File Provider (FUSE on Linux is the drive), filmstrips and a rendition-ready event (poll `renditions`), collecting rendition blobs (nothing yet hands their hashes to `LocalFs::collect`), search, review/share links, federation, storage locations. Task and Studio still run Task's own Files.
