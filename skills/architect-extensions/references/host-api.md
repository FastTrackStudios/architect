# The host API: every call an extension makes, and the capability model

What an extension can ask the app for, how each call is spelled in each runtime, which capability it needs, where it works, and how a person's consent decides it.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` (sections "Capabilities, the host service and consent", "The app's interface", "The device", "Timers, events and background extensions", "Documents:", "The vault's query service", "The editor", "The network and secrets", "MIDI, OSC and control surfaces", "A share link's guests", "A share link's extensions"), `docs/content/architecture/documents.md`, `docs/content/architecture/pages.md`, `docs/content/guide/extensions-in-your-web-app.md`.

## Owner decisions

These are settled. Don't propose reversing them.

- **Extensions are powerful by design.** An extension may draw over the whole app (`ui = ["overlay"]`), draw over another extension's view (`over`), run app actions, edit the person's notes, reach the network and open MIDI ports. Share-link guests run extensions too. Power is **gated** behind capabilities the manifest declares and a person grants. It is never removed.
- **One implementation per service.** Callers hold the generated `<T>Client`. The edge picks the transport: `architect::serve_local` on the desk and iOS, vox over the org lane on the signed-in web, wasm `serve_local` in the demo. Never write a second copy for the web or the node.
- **Browsers hold open notes plus a synced index.** Queries (Bases, tasks, the calendar) run client-side over that index. Full-text search over note bodies runs on the node.
- **HealthKit is first-party only.** Only an extension compiled into the distribution may read or write health data, and only for the person using the app.

## How a call travels

Every call is one JSON value, `HostRequest` (`{"op": "...", ...}`), and every answer is a `HostAnswer` (`{"ok": value}` or `{"error": {kind, ...}}`). Both live in `features/extensions/src/hostcall.rs` `HostRequest`, `HostAnswer`. Each runtime carries the same JSON:

| runtime | how it calls | source |
|---|---|---|
| linked (Rust, in process) | `handle.host(HostRequest::...)`, plus `handle.service(..)` and `handle.window(..)` | `features/extensions/src/linked.rs` `Handle` |
| process (Rust over vox) | `guest::host(..)`; the host serves `ExtensionHost` | `features/extensions/src/guest.rs` `host`, `features/extensions/src/host_proto.rs` `ExtensionHost` |
| Luau script or page | the `host` global: `host.read_document(path)` and the rest; `host.call{op=...}` for any request (it answers `{ok}` or `{error}` and never raises) | `features/extensions/src/luau_host.rs` |
| WASI component | typed imports (`read-document`, `fetch`, `notify`, ...) plus `call: func(request: string) -> string` for any request | `features/extensions/wit/extension.wit` |
| TSX/JS page | globals `documents`, `ui`, `clipboard`, `events`, `editor`, `net`, `ports`, `settings`, `actions`, `services`, and `host.hostCall(request)` | `features/pages/src/js/prelude.js` |
| web view (`kind = "web"`) | `window.architect`: `call(request)`, `documents.*`, `fetch`, `secrets.*`, `notify`, `prompt`, `pick`, `menu`, `clipboard`, `on(event)` | `features/extensions/web/webview.d.ts` `Architect` |
| your own web app (npm `@fasttrackstudio/architect`) | the embedder implements `Host.handle[op]`; requests are checked before they reach a handler | `features/extensions/web/index.d.ts` `HostRequests`, `HostHandlers` |
| anyone, for testing | the action `EXT_HOST_CALL {id, request}` (what extension `id` would be answered); a page's own calls go through `EXT_PAGE_CALL` | `features/extensions/src/actions_manage.rs` |

In the tables below, **op** is the wire name (the same in `HostRequest`, Luau's `host.<op>`, `webview.d.ts` and `index.d.ts`). The Luau name is the op unless the table says otherwise. A WIT import is the op in kebab case where one exists; otherwise the component uses `call`.

The native host is `features/extensions/src/hostapi.rs` `ExtensionApi::call`. The browser's component host is `features/extensions/src/web.rs` `WebHost`. Both decide with the same function, `features/extensions/src/hostcall.rs` `check`.

## The check

`check(manifest, may, request, effect)` returns the capability family it tested (`"reads"`, `"ui"`, `"device"`, ...) and either `Ok` or `(capability, why)`. `may` is what the extension may reach **now**:

- built-in or development copy: its manifest's `[capabilities]`, as declared;
- installed: the manifest's capabilities **within** what a person granted (`features/extensions/src/capabilities.rs` `Capabilities::within`, each part the narrower of the two), from `features/extensions/src/hostapi.rs` `Host::capabilities_of`.

The host reads the grant at call time, so a revoke holds on the next call, hook or script.

Some calls pass `check` and are checked by the host that answers them: `read_file`/`write_file` (its own folder or `folders`), `service` (its `[requires]`/`[uses]` and the method's effect), `point` and `window` (its own), stream, port and surface ids (its own).

A refusal is `HostError::Refused { capability, why }`, where `capability` is dotted (`reads.documents`, `ui.pickers`, `device.camera`, `ports.midi:X-Touch`, `events.transport.changed`). The other errors (`features/extensions/src/hostcall.rs` `HostError`):

| kind | when |
|---|---|
| `refused` | outside what it may reach |
| `confirmation_required` | a person must approve this exact `run` (an extension acting for an agent ran something destructive); run it again with `ticket` |
| `not_loaded` | the extension was unloaded or replaced |
| `not_found` | no such action, document, file or provider |
| `failed` | allowed, and it did not work |
| `unsupported` | this platform or host can't (a headless host asked for a picker, a desk asked for the camera) |

Every call is a tracing span `ext.host_call` with `ext.id`, `ext.op`, `perm.principal`, `perm.capability`, `perm.decision`, `perm.reason` and `ext.outcome`, also set with `architect_telemetry::wide::set`. A refused or failed call is a line in the extension's log (`EXT_LOGS`).

### Who a call is made as

An action an extension runs is run as `Principal::Extension { id, on_behalf_of }` (`features/permissions/src/lib.rs` `Principal`; built by `features/extensions/src/hostapi.rs` `ExtensionApi::principal`). `on_behalf_of` is the person or agent whose run this is part of, or `None` for the app itself (its own hook, timer or view). `ExtensionApi::acting_for` sets it. An extension acting for another extension acts for whoever that one acted for, and the relaying extension is remembered (`via`). A list's authorizer or a server's gate can tell an extension's call from the person's; `RoleEngine` gives it at most the rules of `on_behalf_of`. Document and editor edits carry the same principal, so a peer can tell them from the person's typing.

## Capability keys

Declared in `extension.toml` `[capabilities]` (`features/extensions/src/manifest.rs` `Capabilities`). Exact spellings:

| key | values | consent line (from `features/extensions/src/capabilities.rs` `Capabilities::grows_from`) |
|---|---|---|
| `actions` | action id prefixes past its own namespace (`TRANSPORT_`, `VAULT_`), or `*` | "Run the app's TRANSPORT_… actions" |
| `effect` | `read_only` (default), `mutating`, `destructive`: the most an action it runs may change | "Through them, only read / change things / delete or replace things" |
| `reads` | `org`, `selection`, `documents`, `editor`, `files` (`READS`) | e.g. "Open, read, list and watch your documents" |
| `writes` | `documents`, `editor` (`WRITES`) | e.g. "Create, edit, rename, move and delete your documents" |
| `ui` | `notify`, `clipboard`, `pickers`, `prompts`, `menus`, `badges`, `overlay` (`UI`) | e.g. "Draw over the whole app, outside its own panes" |
| `events` | app-defined event names it may hear: `transport.changed`, `transport.*`, `*` | "Hear the app's transport.* events" |
| `ports` | `midi:<name>` (a trailing `*` matches a prefix, `midi:*` any), `osc:<host>:<port>`, `osc:<host>:*`, `osc:*`, `osc:listen:<port>`, `osc:listen:*` | "Use your MIDI port X-Touch" |
| `hosts` | `api.example.com`, `*.example.com` (the domain too), `127.0.0.1:8080`, `*` | "Reach api.example.com on the network" |
| `folders` | absolute folders past its own (`~` expanded) | "Read and write files in …" |
| `languages` | `servers` (`LANGUAGES`): run its language servers | "Run its languages' servers …" |
| `device` | `notifications`, `camera`, `photos`, `health:read:<type>`, `health:write:<type>` | `features/extensions/src/device.rs` `words` |
| `over` | view kinds not its own it may draw an overlay layer over (`lab.arrange`) | "Draw over the arrangement" |
| `programs` | paths in its package its `process` views run (`bin/room`), one by one; checked again each time a pane opens (`Capabilities::may_run_program`); never a share-link guest's | "Run bin/room, a program in its package, on this computer to draw its views" |
| `webrtc` | `peer-connections` (a share page's web view frame), granted by an operator | not in the prompt |

The extension's own actions, settings (`<id>.…`), data, log, points and windows need no capability. Hosts match through `capabilities::reaches_host`, ports through `capabilities::opens_port`, device entries are validated by `features/extensions/src/device.rs` `check_entry`, and port entries by `capabilities::check_port_entry`. A manifest with a bad entry does not load.

**The vault's and Bases' actions are document access.** `run` of a `VAULT_*` or `BASES_*` action needs `reads = ["documents"]` (read-only ones: `VAULT_READ`, `VAULT_LIST`, `VAULT_SEARCH`, `VAULT_METADATA`, `BASES_ROWS`) or `writes = ["documents"]` (the rest), **on top of** `actions`. See `features/extensions/src/capabilities.rs` `Capabilities::may_run_app`, `VAULT_ACTIONS`, `BASES_ACTIONS`. An `actions = ["VAULT_"]` grant alone reads no note.

## Consent

`features/extensions/src/consent.rs` `Grants` holds every grant on the device.

- **What runs with what.** A built-in or a development copy runs with what it asks. An installed one runs with what it asks *and* a person granted. Until they grant, it gets nothing past its own actions, settings and data.
- **Where a grant is kept.** A `Grant` is `{id, version, capabilities, scope, granted_ms, by}`. `Scope::Device` holds on this device in any org. `Scope::Org(id)` holds only while that org is current (`Grants::set_org`). `Grants::granted` is the union of the device grant and the current org's grant. `Grants::open(path)` persists to a file (`<app dir>/extensions/grants.json` in the lab). `Host::with_grants` gives a host its grants.
- **What the prompt shows.** `Capabilities::asks()` is the install prompt, one `Ask {kind, what, says}` per line (the `says` text is the sentence shown). `EXT_PREVIEW` returns them before install. `EXT_INSTALL` without `grant` answers `needs_consent`.
- **Updates that ask for more.** `Capabilities::grows_from(granted)` lists only what is new, and `Grants::needs(id, asks)` is that list for an installed id. `EXT_UPDATE` answers `needs_consent`. Until the person says yes, the new version runs with the old grant (`within`), so new calls are refused, not crashed.
- **Revoking.** `EXT_GRANT {id, scope}` records a yes and `EXT_REVOKE` takes it back at once (`Grants::revoke`). After a change, `Host::grants_changed` withdraws what the extension lost, such as its scheduled OS notifications. `EXT_GRANTS` lists what each extension may reach now.
- **Agents.** An agent asking to install with a grant, or to grant, is answered `confirmation_required` with a ticket at once (`features/extensions/src/actions_manage.rs` `Extensions::confirm_on`).

## Calls by area

### Actions, org and selection

| op | needs | platforms |
|---|---|---|
| `run {action, args?, ticket?}` | its own namespace, or `actions` plus `effect`; `VAULT_*`/`BASES_*` also need `reads`/`writes = ["documents"]` | all |
| `org` | `reads = ["org"]` | all (from the app's `AppContext`) |
| `selection` | `reads = ["selection"]` | all |

- **Spellings.** TSX: `actions.run(id, args)`. Web view: `architect.run`. WIT: `run-action`.
- **Where the answers come from.** `features/extensions/src/hostapi.rs` `AppContext` supplies org and selection. With no context, both answer `null`.

### A view's place

| op | what | needs |
|---|---|---|
| `set_view_state {pane, state}` | keeps where its view in pane `pane` is, given back when the view starts again; only a pane showing one of its own views takes it | nothing |
| `actions` | the app's actions it may run (its own, and those `capabilities.actions` reaches), as the Action List describes them: what a process view's `ViewCtx::actions` holds (`guest::app_actions`) | nothing |

Call it through `linked::use_view_state()` (`ViewState::set`), which writes the pane directly linked in and sends this from a process: the same code either way.

### Documents (the documents service)

| op | Luau / TSX `documents.` / web view `documents.` | needs |
|---|---|---|
| `list_documents {under?}` | `list_documents` / `list` / `list` | `reads = ["documents"]` |
| `read_document {path}` | `read_document` / `read` / `read` | `reads` |
| `open {path}` | `open` / (none) / `open` | `reads` |
| `create_document {path, text?}` | `create_document` / `create` / `create` | `writes = ["documents"]` |
| `edit_document {path, edits, name?}` | `edit_document` / `edit` / `edit` | `writes` |
| `rename_document {path, to}`, `move_document {path, folder}` | same / `rename`, `move` / `rename`, `move` | `writes` |
| `delete_document {path}` | `delete_document` / `delete` / `remove` | `writes` |
| `undo_document {path}`, `redo_document {path}` | `undo_document`, `redo_document` / (none) / (none) | `writes` |
| `watch_documents {action \| script, function?}` | `watch_documents` / `watch` / `on("doc.*")` | `reads` |

- **WIT.** The typed imports are `list-documents`, `read-document`, `create-document`, `edit-document`, `rename-document`, `move-document`, `delete-document` and `watch-documents`.
- **Undo.** `undo_document` takes back the document's last undo step and `redo_document` makes it again: `{step, text}` (the step's name and the text after), or `null` when there is none. A CRDT document (`features/extensions/src/documents/crdt_docs.rs` `CrdtDocuments`) walks its own `History`: every step, the person's and each extension's. A vault note (`features/navigator/src/documents.rs` `VaultDocuments`; a file keeps no undo) walks the edits made through the documents (`architect_documents::Journal`), each taken back by the three-way merge an open note uses, so edits made since stay. `DOC_UNDO {path?}` / `DOC_REDO {path?}` are the same as actions (⌘K, menus, MCP; without `path` the active pane's document), and ⌘Z / ⇧⌘Z run them in every view an opener shows a file in (`features/extensions/src/contrib_actions.rs`). A chart needs no undo stack of its own. Not yet a WIT import.
- **Edits.** `DocEdit` is `{insert: {at, text}}`, `{delete: {at, len}}`, `{append: {text}}` or `{replace: {text}}`, with positions in characters. One call is one CRDT transaction and one undo step named `name`, made as the extension. `replace` is diffed, so the person's concurrent typing survives. An edit that changes nothing writes nothing and raises no `doc.saved`.
- **Paths** are relative and `/`-separated, never `..`.

**One service, through its client.** The document calls and the extension's data are one `#[architect::rpc]` service: `features/extensions/documents/src/lib.rs` `Documents` (crate `architect-documents`, wasm-clean). Its methods are `list`, `read`, `create`, `edit`, `rename`, `delete`, `metadata`, `data` and `set_data`, and each names the extension it is for. `DocumentsServer` is its only implementation, and the gate is inside it (`Gate`: `reads`/`writes = ["documents"]`), so every transport gets the same check.

The host only ever holds a `DocumentsClient` (`features/extensions/src/docs_service.rs`). It serves `DocumentsServer` over three pieces with `architect::serve_local`:

- `AsStore`: the app's `Documents`;
- `FileData`: each extension's data;
- `HostGate`: what each extension may reach now.

Natively the server runs on its own one-thread runtime (`ext-documents`). In wasm it runs with `spawn_local`.

What sits under `Documents` on each platform:

| where | the documents | source |
|---|---|---|
| desk, iOS | the person's open vault: `VaultDocuments` over the one `VaultSource` the navigator uses; writes go compare-and-swap through the vault, merged into an open note's typing | `features/navigator/src/documents.rs`, `apps/ui/src/extensions.rs` |
| web, signed in | `WebDocuments` over the page's `MemorySource`. A create or edit is the page's copy at once and the node's through `Notes::write`; rename, move and delete are desktop-only (the org lane has no such call yet) | `apps/web/src/app/documents.rs` `WebDocuments` |
| web, `?demo` | the same, over the demo vault in memory | `apps/web/src/app/documents.rs` |
| sandbox, no vault | in-memory spaces `notes` and `archive` | `apps/ui/src/extensions.rs` |
| no `Documents` given | the calls answer `unsupported` | `features/extensions/src/docs_service.rs` `NoDocuments` |

- **Not yet.** An org's other vaults (`@org/…`) are not documents yet. A note's parsed metadata is the read-only action `VAULT_METADATA` (run it with `actions = ["VAULT_"]` plus `reads = ["documents"]`), not a host call.

### The vault's query service (Bases over it)

| op | Luau | needs | platforms |
|---|---|---|---|
| `query_vault {path? \| source?, view?, if_changed?, limit?}` | `host.query_vault{…}` | `reads = ["documents"]` | desk, iOS, web (signed in and demo); not a share guest |

- **What it runs.** One view of a base (a `.base` path, or base YAML in `source`) over the vault index. Rows come in pages: `hostapi::QUERY_LIMIT` (200) by default, at most `QUERY_LIMIT_MAX` (5000).
- **What it answers.** The views, the view, rows in groups (`{label, rows: [{path, name, values, formulas}]}`), `total`, columns, formulas, filters, the index `revision` and a `hash`. Pass that `hash` back as `if_changed` and you get `{unchanged: true}` with nothing run.
- **Keeping a view live.** Re-ask on `doc.*` events with `if_changed`.

The service is `features/vault/vault-proto/src/query.rs` `VaultQuery` (one method, `query(QueryRequest)`). Its one implementation is `vault_query::QueryEngine` (`features/vault/vault-query`). The host holds only the `VaultQueryClient`, given together with the documents and a scope: `features/extensions/src/hostapi.rs` `Host::set_vault(documents, client, VaultScope::Person | VaultScope::Guest)`. A `Guest` scope keeps no query client, so `query_vault` answers `unsupported`. Calling `Host::set_documents` alone drops the client.

| where | the index | served by |
|---|---|---|
| desk, iOS | built from the open vault (`vault::query::Built`) | `serve_local` on its own runtime: `apps/ui/src/notes.rs` `vault_query` |
| web, signed in | a `Replica` synced from the node's `VaultIndexSync` on the org lane (snapshot, then deltas every 2 s, filtered per caller); no note bodies | `serve_local` in the page: `apps/web/src/app/index_sync.rs` |
| web, demo | built in the page from the demo vault | `apps/web/src/app/index_sync.rs` `serve_demo` |
| node | answers no queries; serves the index sync (`features/vault/vault-proto/src/index_sync.rs`) | n/a |

`BASES_ROWS` runs on the same engine through the same client (`features/bases/src/actions.rs`). Bases is an extension over this core and uses only what a third party could. A guest's subset of the index is not built yet (#710 G1).

### Live documents of its kind (the node's copy)

| op | Rust (linked or process) | needs | platforms |
|---|---|---|---|
| `live_document {path}` | `Handle::document(path)`, `guest::document(path)` | `reads = ["documents"]`, and the file's kind in its own `[[doc_kinds]]`; edits reach the node only with `writes = ["documents"]` | desk (feature `doc-sync`; native only); not Luau, TSX, WASI or the web |

- **What it is for.** A vault file of a kind the extension declares (a Keyflow `.session`) is a document of the app's node, kept in the file (`docs/content/guide/documents.md`). The file has one writer, the node. An extension edits the node's copy through the host instead of opening the file: never `crdt::file::DocFile` on a vault file, never a sync host of its own.
- **The answer.** `features/extensions/src/live.rs` `LiveDoc` `{doc_id, kind, path, writable}`. A path that is no document of a registered kind is `not_found`. A kind the manifest does not declare is refused as `doc_kinds.<kind>`. No `reads` is refused as `reads.documents`. A host without node documents answers `unsupported`.
- **The session.** `Handle::document` and `guest::document` return a `features/extensions/src/live.rs` `DocLane`. `DocLane::replica()` keeps an in-memory `Replica` in step on a tokio task (`doc()`, `attached().await`, `ended()`). `DocLane::sync(&mut SyncedDoc)` runs one session of the extension's own `crdt::sync::SyncedDoc` for `doc_id` over its own `CrdtDoc`; run it again after it ends and it catches up by version vector. `DocLane::client()` is the raw `crdt::sync::DocSyncClient`.
- **The wire.** No new socket. A process's `DocSync` lane rides the connection it has to the host (`guest` opens it beside the host service, `features/extensions/src/guest.rs` `doc_sync`); a linked one gets an in-memory lane (`features/extensions/src/live.rs` `serve_in_memory`). The host's `features/extensions/src/live.rs` `ExtensionDocSync` relays each session to the node's registry through `crdt::peer::SelfLane` as `Principal::Extension` for whoever it acts for, so the registry's authorizer and the vault's write guard decide (`features/vault/vault-live/src/write_guard.rs` `account_of` holds an extension to the person it acts for). Only a document `live_document` answered that extension is reached, re-checked against its kinds and `reads` on every attach; anything else is `UnknownDoc`. Without `writes` the host drops its updates: it only follows. A share guest's `guest_capabilities` has no `writes`, so a guest only follows.
- **The app's part.** `features/extensions/src/hostapi.rs` `Host::set_kind_documents` with a `features/extensions/src/live.rs` `KindDocuments`: `resolve(path)` to `KindDoc {doc_id, kind}`, and `registry()`. `RegistryDocuments::new(registry, resolve)` builds one from a node's `VaultCollab` (`document_of`, `kind_of`, `registry`). The desktop app gives its window's host the node's (`apps/desktop/src/node/mod.rs` `Running::kind_documents`, through `apps/ui/src/extensions.rs` `on_host`): a path is in the node's one vault, or its first folder names one of several (`vault_path`); its node holds the vaults its configuration names (`ARCHITECT_VAULTS`), none by default.
- **Tests.** `features/extensions/tests/live_documents.rs`.

### A project's files on the org's node

| op | Rust (linked or process) / Luau | needs | platforms |
|---|---|---|---|
| `file_list {place, folder?}` | `Handle::host(HostRequest::FileList {..})` / `host.file_list(place, folder?)` | `reads = ["files"]` | the web app signed in; elsewhere `unsupported` |
| `file_url {place, path}` | `Handle::host(HostRequest::FileUrl {..})` / `host.file_url(place, path)` | `reads = ["files"]` | the same |

- **What it is for.** An engine in the app (Studio's, in a browser) lists a project folder and range-reads its files: the member's own project, not only a share link's. Generic: a place is `vault:<name>` (a vault of the org's) or `root:<id>` (a root the node holds, `architect_files::sync::Engine::hold_at`).
- **The answers.** `file_list`: `{place, folder, files: [{path, size, sha256, url}], expires_us}`, every file under the folder the person may read, in path order, each `url` absolute and read with `Range` and no header (a read ticket for the folder in its query) until `expires_us`. `file_url`: `{url, expires_us}`, a ticket for one file. Ask again once it runs out (an hour, `apps/desktop/src/node/held.rs` `TICKET_TTL_US`).
- **Who decides.** The host checks the capability (`features/extensions/src/hostcall.rs` `check`, and `features/extensions/web/check.js`), then hands the call to the app's `features/extensions/src/held.rs` `HeldFilesSource` (`Host::set_held_files`); the node decides what the person may read (`features/files/src/held.rs` `HeldFiles`, served by `apps/desktop/src/node/held.rs` `HeldFace` under the org lane's gate). A host with no source answers `unsupported`: a share link's guest, the demo and the desk app have none, so a guest reads nothing past what its link hands it.
- **The app's part.** The web app's source is `apps/web/src/app/held.rs` `WebHeldFiles`, over the signed-in org's lane (`apps/web/src/app/org.rs` `OrgVaults::held_list`, `held_url`, the lane's bearer in a header).
- **Tests.** `features/extensions/tests/held_files.rs`; the node's face `apps/desktop/tests/node.rs` `a_member_reads_a_project_folders_files_by_range_and_nobody_else_does`.

### Files, settings and data

| op | Luau / TSX | needs | notes |
|---|---|---|---|
| `read_file {path}`, `write_file {path, text}` | same / (none) | its own data folder (relative paths), `capabilities.folders`, or a path the person picked this run | desk, iOS. A browser's `WebHost` fails them. The secrets file is always refused |
| `setting {key}`, `set_setting {key, value}` | same / `settings.get`, `settings.set` | none, but only its own keys (`<id>.…`; another key is `refused (settings)`) | the settings page's store (`SettingValues`) |
| `data {key}`, `set_data {key, value?}` | same / web view `data`, `setData` | none | see below |
| `log {text, level?}` | same / `console.*` (the page's log), web view `log` | none | goes to `EXT_LOGS` |

Data goes through the documents service's `data`/`set_data`. Where it is kept depends on the host:

- **with a data root** (`Host::with_data_root`): one file per extension, written whole, `0600`, never wiped by a file that doesn't parse (`features/extensions/src/docs_service.rs` `FileData`);
- **in a browser**: `localStorage`, one entry per extension and org (`features/extensions/src/hostapi.rs` `DataStore`, `Host::with_data_store`);
- **otherwise**: in memory for the run.

### The editor

| op | Luau / TSX `editor.` | WIT | needs |
|---|---|---|---|
| `editor_state` | `host.editor()` / `state()` | `editor-state` | `reads` or `writes = ["editor"]` |
| `edit_editor {changes, selection?, name?, base?}` | `edit_editor` / `edit` | `edit-editor` | `writes = ["editor"]` |
| `decorate_editor {decorations}` | `decorate_editor` / `decorate` | `decorate-editor` | `writes = ["editor"]` |

- **Positions.** They are byte offsets into UTF-8 text.
- **Stale edits.** `base` is the `version` you read; an edit against a text that has moved on is refused.
- **Decorations.** Each call replaces the extension's previous set. Classes are prefixed `ext-`.
- **Manifest entries.** `[[keymaps]]` and `[[slash]]` need `writes = ["editor"]`, checked at load.
- **The app's side.** The app implements `features/extensions/src/editor.rs` `Editors` and passes it to `Host::with_editors`. `MemoryEditor` is a headless editor for tests. `apps/ui/src/note_editor.rs` wires the desk's note editor.
- **Platforms.** The browser's component host (`WebHost`) answers `unsupported`.

### Network fetch and secrets

| op | Luau / TSX `net.` / web view | needs |
|---|---|---|
| `fetch {url, method?, headers?, body?, timeout_ms?, max_bytes?, stream?, binary?}` | `fetch(url, opts?)` / `fetch` / `architect.fetch` | the URL's host, and every redirect's, in `hosts` |
| `read_body {stream, max?}`, `close_body {stream}` | same / `read`, `close` | its own stream |
| `secret {name}` | same / `secret` / `secrets.isSet` | one of its `[[secrets]]`; answers `{set, title}`, never the value |
| `ask_secret {name}` | same / `askSecret` / `secrets.ask` | declaring it |
| `store_secret {name, value}` | same / (none) / `secrets.store` | declaring it; insert-only, at most 8 KiB (`hostapi::MAX_SECRET`); refused if already set |

- **WIT.** The imports are `fetch`, `read-body`, `close-body`, `secret-set` and `ask-secret`.
- **Using a secret.** A header value names it: `"Bearer {{secret:api_key}}"`. The host fills it in only on requests to that secret's `hosts` and never hands the value back (`features/extensions/src/hostcall.rs` `with_secrets`, `may_reach_url`).
- **Limits and redirects** (`features/extensions/src/net.rs`):
  - `TIMEOUT` is 30 s, `MAX_TIMEOUT` 300 s;
  - `MAX_BODY` 16 MiB, `MAX_STREAM` 1 GiB, `MAX_UPLOAD` 16 MiB;
  - `MAX_REDIRECTS` is 10, and the host follows each redirect itself, checking it.

| platform | fetch | secrets |
|---|---|---|
| desk | `architect-extensions` feature `net` (reqwest), on in `apps/desktop/Cargo.toml` | kept in `net::Secrets` (a `0600` file) |
| build without `net` | `unsupported` | kept |
| browser | the browser's `fetch`, where CORS allows; a redirect answers `unsupported` | in memory only; `store_secret` is `unsupported` in `WebHost` |
| node | node actions only (`runs = "node"`); see `features/extensions/src/node.rs` | operator files or admin-set, insert-only |

### Ports: MIDI, OSC, control surfaces

| op | Luau / TSX `ports.` | WIT | needs |
|---|---|---|---|
| `ports {kind?}` | same / `list` | `ports` | any `ports` entry |
| `open_port {kind, name, listen?}` | `open_port(kind, name, listen?)` / `open` | `open-port` | a `ports` entry naming it (and `osc:listen:<listen>`) |
| `port_send`, `port_receive {port, max?}`, `close_port` | same / `send`, `receive`, `close` | `port-send`, `port-receive`, `close-port` | its own port |
| `targets`, `surfaces`, `surface_bank {surface, bank}` | same / `targets`, `surfaces`, `setBank` | `call` | none / its own surfaces |

- **What a port hears.** Incoming messages arrive as the event `port.message` (`features/extensions/src/ports.rs` `PORT_MESSAGE`), sent only to the extension that opened the port. Its detail is `{port, kind, name, message}`.
- **Where ports come from.** The app gives them (`Host::with_ports`, an `architect_input::PortBackend`; `Host::with_targets`).
- **Platforms.** Desk only today: the sandbox has UDP OSC and a virtual MIDI port. A system MIDI backend is the app's to add.

### The app's interface

All of these need `ui = [...]`. `pick`, `prompt` and `menu` wait for the person and answer `null` on cancel.

| op | Luau | TSX page | web view | WIT | needs |
|---|---|---|---|---|---|
| `notify {text, kind?, title?}` | `notify` | `ui.notify` | `notify` | `notify` | `notify` |
| `clipboard_read`, `clipboard_write {text}` | same | `clipboard.read`, `.write` | `clipboard.read`, `.write` | `clipboard-read`, `-write` | `clipboard` |
| `pick {kind: open\|open_many\|folder\|save\|photo\|photos, …}` | `pick` | `ui.pick` | `pick` | `pick` | `pickers` |
| `prompt {kind: text\|confirm\|pick_one, title, …}` | `prompt` | `ui.prompt`, `ui.input`, `ui.confirm`, `ui.pickOne` | `prompt` | `prompt` | `prompts` |
| `menu {items, x?, y?}` | `menu` | `ui.menu(items, at?)` | `menu` | `menu` | `menus` |
| `decorate {target: {kind: file\|tab, id}, decoration?}` | `decorate` | `ui.decorate` | via `call` | `decorate` | `badges` |
| (not a call) its views' scenes draw unclipped over the whole window | | | | | `overlay` |
| (not a call) an `[[overlays]]` layer over another's view kind | | | | | `over` |

- **A picked file is readable.** A path the person picked is one the extension may read (or, for a save or inside a picked folder, write) for the run, past `folders`.
- **Who answers** is the host's `HostUi` (`features/extensions/src/hostui.rs`; `Host::set_ui`):
  - `NoUi` is the default: notifications become log lines and everything else is `unsupported`;
  - `FakeUi` is for tests;
  - `WindowUi` (`features/extensions/src/host_ui_view.rs`) draws toasts, dialogs and menus in the window.
- **Pickers by platform:**
  - **desk:** rfd, the system's own panels (feature `pickers`);
  - **iOS:** UIKit (`features/extensions/src/pickers_ios.rs`). Files open as copies; a folder is opened with a security scope held for the run; a save is a picked folder plus the suggested name. Photos use `PHPickerViewController`, so there is no library permission prompt, and the images come back as metadata-stripped JPEG copies;
  - **browser `WebHost`:** every interface call is `unsupported`.
- **Overlay.** Drawing over another view and drawing over the whole app are separate grants: `capabilities.over` and `ui = ["overlay"]`. See `features/extensions/src/capabilities.rs` `Capabilities::may_draw_over`; its own views are always allowed.

### Device APIs

`device = [...]`, granted entry by entry. The host's `Device` answers (`features/extensions/src/device.rs` `Device`; `Host::set_device`).

| op | needs | answer |
|---|---|---|
| `os_notify {id?, title, body?, at_ms?, after_s?}` | `notifications` | `{id}`; ids are kept as `<extension>/<id>`, so one extension can't touch another's |
| `cancel_os_notify {id}` | `notifications` | whether it had one |
| `capture {kind?}` | `camera` | a JPEG path with EXIF and GPS stripped, or `null` |
| `photos {limit?, since_ms?}`, `photo_file {id}` | `photos` | `[{id, created_ms, width, height}]`; a copied file's path |
| `health_read {type, from_ms, to_ms, sum?}` | `health:read:<type>` | `{type, unit, sum}` or `{…, samples}` |
| `health_write {type, value, start_ms?, end_ms?}` | `health:write:<type>` | `null` |

- **Spellings.** Luau, WIT and TSX pages reach these through the generic call (`host.call`, `call`, `ui.call`). The web view types them (`webview.d.ts`).
- **Health types** (`HEALTH_TYPES`): `steps`, `active_energy`, `dietary_energy`, `protein`, `carbohydrates`, `fat`, `water`, `distance`, `body_mass`, `heart_rate`.
- **Platforms.**
  - **iOS:** `features/extensions/src/device_ios.rs` `IosDevice` (feature `device`, set in `apps/ui/src/extensions.rs`). The OS asks its own permission prompt after the manifest's consent. The camera is `unsupported` on the Simulator.
  - **desk and browser:** every device call is `unsupported`; there is no `Device` there yet.

**HealthKit is first-party only.** A health call passes `check` only with the entry, and then `features/extensions/src/hostapi.rs` `ExtensionApi::health_refused` refuses it in three cases:

- the extension is not `Origin::Builtin` (installed and development copies, even of a built-in's id): `HEALTH_LINKED_ONLY`;
- the call is made on anyone's behalf (an agent over MCP, a peer): `HEALTH_NOT_RELAYED`;
- the call is relayed through a non-built-in extension (a service call): `HEALTH_NOT_RELAYED`.

### Timers and events

| op | Luau / TSX `events.` | WIT | needs |
|---|---|---|---|
| `subscribe {on \| every \| at, action \| script, function?}` | `subscribe` / `subscribe` | `subscribe` | `events` for an `app:<name>` hook; `reads = ["documents"]` for a `doc.*` hook (`features/extensions/src/events.rs` `is_doc_event`) |
| `unsubscribe {key}` | same / `unsubscribe` | `unsubscribe` | none |
| `events` | same / `list` | `call` | none |
| `raise {name, detail?}` | `host.call{op="raise"}` | `call` | none: it raises its own `<id>.<name>`, which others hear as `app:<id>.<name>` under their `events` grant |

- **Schedules.** `every = "5m"`, `at = "09:30"` and five-field cron run on the host's clock (`Host::with_clock`, `Host::tick`).
- **Events the app defines.** The app defines them (`Host::define_event`) and raises them (`Host::raise_app`); hooks are refused unless granted (`Capabilities::may_hear`, matched by `events::covers`).
- **Background extensions.** `[background] idle = "5m"` starts the extension on its first event and stops it when idle (`features/extensions/src/host_events.rs`).
- **WASI timers.** A WASI view's timers use the `now` import and the `tick` export.
- **Platforms.** The browser's component host answers subscribe, events and raise with `unsupported`.

### Interop: other extensions' services, points, windows

| op | needs |
|---|---|
| `service {service, method, args?}` | `[requires]` or `[uses]` naming the service; the provider's action `<PREFIX><METHOD>` must exist; its effect must be within your `effect` |
| `point {point}` | owning the point (`[[points]]`) |
| `window {id, open, state?}` | its own `[[windows]]` |

- **Spellings.** Linked: `Handle::service`. TSX: `services.call(service, method, args)` (the action `EXT_SERVICE_CALL`). WIT: `call-service`. Luau: `host.service`.
- **Declaring a service.** A provider declares `[provides] "lab.tempo" = "1.2.0"`. See `features/extensions/src/interop.rs`; the host's check is `features/extensions/src/hostapi.rs` `ExtensionApi::service`.
- **Two providers of one service.** The host takes the newest version; at the same version, the first by id (`features/extensions/src/interop.rs` `choose_provider`). A person or a launcher picks another with `EXT_SERVICE_USE {service, provider}` (`null` gives the choice back), kept while that provider is loaded; `EXT_SERVICES` lists each service's providers, the chosen one and the one in use (`features/extensions/src/host.rs` `Host::use_provider`, `Host::service_providers`).
- **The call runs as the caller.** The provider's action runs as the calling extension, so a health call relayed this way is refused.

## Share-link guests

A share link is the Architect app opened as a guest of that link (`Principal::Guest`): the same app, views and extensions, with some permissions off (`docs/content/guide/shares.md`). There is no guest renderer or guest extension set; a guest's limits are the capabilities below.

A share link's guests can run extensions, scoped to the share (#495 G1).

**The profile** is one function every guest host uses: `features/extensions/src/share_needs.rs` `guest_capabilities(asks, hosts)`. It is the manifest's asks within:

- `reads`: `GUEST_READS` (`documents`, `selection`);
- `ui`: `GUEST_UI` (`notify`, `prompts`, `menus`, `badges`);
- `hosts`: only those the owner approved for that extension, and that it also asks for;
- `effect`: `read_only`.

It then clears `writes`, `actions`, `folders`, `ports`, `languages` and `webrtc`. A guest host refuses `[[secrets]]` outright.

**Which extensions a link runs** is decided by the owner, not the vault's list:

- the owner's choice is kept in `_architect/shares/<code>.extensions.toml` (`LinkNeeds`), written only by the owner;
- `EXT_SHARE_NEEDS_SET` writes it;
- `plan_link` plans what to fetch;
- `verify_package` checks each signed archive against the org key and the owner's publishers, never against keys named in the vault.

**Data.** A guest's documents are the share's, read-only, through `ShareGuest` (`features/spaces/src/guest.rs` `ShareGuest`: `link`, `read`, `files`, each call made as `Principal::Guest`). A guest's vault is set with `VaultScope::Guest`, so it gets no query service.

**A page drawing a guest's content** runs on `features/pages/src/host.rs` `AppHost::guest`. That host:

- runs no actions;
- makes no host-service calls;
- keeps its settings and shared state in memory.

Only a page embed draws guest content (`features/extensions/src/embeds.rs` `GuestEmbedView`). A linked or process view would run with the person's session, so it shows its fallback instead.

**What a guest's view is handed** is decided in the app, not by the node. The node gives a guest the note's text and the link's referenced files whenever the link's scope, audience and layer admit them (`apps/desktop/src/node/shares.rs` `GuestLane`). The web app then hands a view only what its island declares (`apps/web/src/ext_host.rs` `note_reads`, `note_props`; the same rule as `features/sites/src/island.rs` `note_reads`). The view gets the note's text only when its claim was made with `SiteIsland::embed_note` (properties declared) and its extension declares `reads = ["documents"]` (`READS_NOTE`). It also gets the files, which the app fetches only then, if it declares `reads = ["files"]` (`READS_FILES`). A view that declares no read gets no note.

**Consent is the guest's own**, per person. Nothing from the owner's grants carries over.

**What's built.** Hosts actually loading a link's fetched packages within `guest_capabilities` is #495 G1. Until a host does it, a guest runs no fetched extension. The model, the island path and `AppHost::guest` exist. The page-only `AppHost::guest` path and the web app's guest mode without extensions are known gaps, not the design (#805).

## Where each host lives

| host | file | covers |
|---|---|---|
| native, every runtime (desk, iOS, linked web build) | `features/extensions/src/hostapi.rs` `Host`, `ExtensionApi` | everything above, as the app wires it |
| a WASI component in a browser | `features/extensions/src/web.rs` `WebHost` | `run`, org, selection, settings and data (in memory), log, fetch. Documents and files fail; editor, interface, events, device, secrets storage and ports are `unsupported` |
| your own web app | `features/extensions/web/index.d.ts` `Host` (`memoryHost`, `fetchHandler`) | whatever handlers the embedder gives; deny by default |
| a web view | `features/extensions/web/webview.d.ts` `Architect` | the native host's calls, through the bridge |

## Traps

- A `doc.*` hook (manifest, `subscribe`, or `watch_documents`) without `reads = ["documents"]` is refused at load, and for an installed extension it is never fired.
- `actions = ["VAULT_"]` alone reads nothing; add `reads` or `writes = ["documents"]`.
- `set_setting` on another extension's key is refused, even for a built-in.
- The SDK types lag the wire. `features/extensions/web/index.d.ts` `HostRequests` has no `query_vault`, no device ops and no `store_secret`, and its `Capabilities` lacks `device`, `over`, `webrtc`, `files` and `overlay`. `webview.d.ts` has no `query_vault`. Use `call`/`hostCall` with the raw request.
- On the web, a rename, move or delete of an org-vault note through an extension does not reach the node yet (no lane call).
