# Files: an org's roots, versions, uploads and access

`architect-files` (`features/files`) is the file layer: roots of files and folders an app keeps its things in, a version per save (a delete is one too), labels on versions, resumable uploads and ranged reads over vox channels, an org quota, and access by the caller's role in the org plus grants on folders. Design: `docs/content/architecture/documents.md` ("Files"). Ported from Task's `task:features/files`; what has not moved yet is listed at the end.

## Pieces

| Flag | What | Entry points |
|---|---|---|
| default (wasm-clean) | ids, paths, the model, the fault, capabilities and their resolution | `features/files/src/path.rs` `RootPath` (`parse` refuses `..`; `validate` what the wire decoded); `features/files/src/model.rs` `Root`, `Entry`, `Version`, `VersionLabel`, `Usage`, `OnConflict`, `Expect`, `FilesEvent`; `features/files/src/error.rs` `FilesFault`; `features/files/src/access.rs` `Capability`, `Grant`, `role_capabilities`, `stewards`, `resolve` |
| `vox` | the service, its gate tables, the app's client | `features/files/src/service.rs` `FilesService` (`#[subscribe] events`), `PERMITS`, `STREAM_PERMITS`; `features/files/src/client.rs` `FilesClient` (`ensure_root`, `put` + `Save`, `get`, `read_range`, `resolve`, `get_ref`, `events`), `content_of` |
| `server` (native) | every org's files on a server | `features/files/src/server/mod.rs` `FilesServer` (`new`, `default_org`, `with_grants`, `acting`, `set_quota`), `GrantSource` (grants from elsewhere — a space's members — asked per call); `features/files/src/server/acting.rs` `Acting` |

## Rules it keeps

- **The org is the lane's**, never an argument: `architect_orgs::server::current_org()` (or `FilesServer::default_org` on a one-org server). Data lives under `<data>/org/<id>/files/` (`OrgId::path`), bytes in the `architect::blob::LocalFs` the server shares with documents.
- **The caller is the gate's** (`architect::permissions_gate::caller()`). No principal is `FilesFault::Unauthenticated`, never the server's own; in-process jobs use `FilesServer::acting(org, Principal::Service { .. })`.
- **Role, then grant.** Owner/admin/member hold every capability everywhere in the org (owner/admin also rename and release roots); viewer reads and reads history. A `Grant` adds capabilities over one folder for one user id. Outside what a person holds a path is `PathNotFound`, never `Denied`; folders above a grant are visible and hold nothing.
- **The gate checks membership, the feature checks the path.** `PERMITS` marks every method `read` on `files/**`; pair it with `ConfinedEngine` (membership) so a viewer granted `Write` on a folder still reaches `complete_upload`.
- **History replaces trash.** Delete appends a delete version; `restore` brings it back. A move keeps a file's history; a copy starts its own.
- **Saves are safe.** `Save::replacing(etag)` refuses with `Stale` when the file moved on; a taken destination is the caller's choice (`OnConflict`), never the server's.
- **Dedup is per org.** An upload naming content the org already references sends nothing; content only another org holds is not proof of having the bytes.
- **Quota** counts live files' bytes (`State::used`); every change that grows it past `Usage::quota` fails with `QuotaExceeded`. The deployment sets it (`FilesServer::set_quota`), not a member.

## Recipe: mount it

```rust
let files = FilesServer::new(&data, LocalFs::new(data.join("blobs")), members.clone());
let router = LayerRouter::new()
    .with(files_service_rpc_service_descriptor(), architect_files::serve(files.clone()))
    .merge(architect_files::stream_layer(files.clone()));
let gate = PermissionsGate::new(Arc::new(ConfinedEngine::new(members.clone())), Arc::new(identity))
    .permit(files_service_rpc_service_descriptor(), &architect_files::PERMITS)
    .permit(files_service_stream_service_descriptor(), &architect_files::STREAM_PERMITS);
// per connection at /org/{slug}/vox:
confine(org, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer))
```

`features/files/tests/files.rs` is the whole loop over a real vox link: roles, grants, absence, quota, resumed and deduplicated uploads, per-subscriber events, restart.

## Recipe: an app's store

```rust
let files = FilesClient::new(caller); // the org lane's vox Caller
let irs = files.ensure_root("signal/impulse-responses", "Impulse responses").await?;
let saved = files.put(irs.id, "Cab/4x12.wav", &wav, Save::create_only()).await?;
let pin = saved.content; // the etag: Save::replacing(pin) next time
```

## Not in architect yet

Client sync (selective sync, offline copies, the device catalogue), the browser integration (service worker answering `Range`), the HTTP byte face, desktop integrations (daemon, FUSE, File Provider, WebDAV), adopting on-disk folders in place, renditions (`files-transcode`), search, review/share links, federation and peering, project versions, divergent versions, storage locations. Task and Studio still run Task's own Files.
