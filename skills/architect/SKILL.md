---
name: architect
description: "Use when building or reviewing Task, Signal, Session or any FTS Rust app: contracts, commands (MCP/menu/UI), offline + P2P sync, auth, boundaries. Rules, API map, recipes."
version: 0.4.0
author: Commander (STA-216 v0.1; v0.2 from Researcher's STA-372 audit, reviewed in STA-371; moved into architect (v0.4))
license: MIT
metadata:
  hermes:
    tags: [architect, rust, framework, commands, mcp, crdt, local-first, p2p, boundaries, fts-auth, dioxus, telemetry]
    related_skills: [perfect-functions, rust-build-efficiency, release-workflow, project-drives, paperclip-workflow]
---

# The architect way

[FastTrackStudios/architect](https://github.com/FastTrackStudios/architect) is the fleet's Rust application framework: entities, vox RPC, HTTP/SSE faces, auth, permissions, CRDT sync, offline queues, commands + MCP, keymaps, telemetry and `architect-ui`. The Captain's goal: Task, Signal and Session get local-first, offline, peer-to-peer collaboration and enforced app boundaries **by building on architect**, not by hand-rolling plumbing per app. Reach for it first, and check the map below before writing any generic plumbing.

Every architect path in this skill is checked by `cargo xtask docs` against the architect repo; `task:` and `studio:` paths name the Task and studio repos.

Paperclip projects: **Architect** and **fts-auth**. Wiki: `architect` (Task org fasttrackstudio).

## When to Use

- Designing a feature, service, entity, command, keymap or synced document in Task, Signal, Session (studio) or architect.
- Replacing hand-written DTOs, axum routes, SeaORM repos, MCP catalogues, menu handlers, Loro sync glue, env parsing, token stores, self-updaters.
- Writing or reviewing an issue/PR in those repos.
- Sign-in or accounts: also read `references/fts-auth.md` (deployment) and `references/auth-permissions.md` (API).

## Before you rely on a version

- Tags land several times a day. Run `gh release list -R FastTrackStudios/architect -L 3` for the latest, and read the consumer's root `Cargo.toml` (`[workspace.dependencies]`, plus any `[patch]` of architect) for what it pins. `CHANGELOG.md` says which tag shipped what.
- When the consumer pins an older tag than the feature you want, repin first (recipe below); don't copy the code across.
- Vox handshakes fail across mismatched vox revisions: server and every client pin one architect tag.
- When the docs (`docs/content/`) and the code disagree, trust the code. `examples/greeter` is the reference feature.

## Where to look (API map)

| Need | Read | Core entry points |
|---|---|---|
| Entity, repo, storage, config from env, error enums, `Resource`/`Scope`, wire types | `references/entities-storage.md` | `crates/architect-derive/src/lib.rs` `entity`, `error`, `derive_config` |
| Service trait, layers, hosts, WS/iroh/HTTP, client pool, pubsub, schedule, reconnect | `references/rpc-services-hosts.md` | `crates/architect-rpc-derive/src/lib.rs` `service`; `crates/architect/src/host.rs` `EngineHost`; `crates/architect/src/connect/mod.rs` `pool` |
| Commands, Action List, async actions, a service as actions, MCP server | `references/commands-mcp.md` | `crates/architect/src/action_list.rs` `ActionList`; `features/mcp/src/lib.rs` `Server` |
| Keymaps, key sequences, modes, which-key bound to Action List ids | `docs/content/architecture/input.md` | `features/input/src/processor.rs` `InputProcessor`; `features/input/src/config.rs` `KeymapConfig` |
| Co-edited docs, Loro, `crdt(text)`, schema migration, pairwise peer exchange, presence, doc admission | `references/crdt-peer-sync.md` | `features/crdt/crdt/src/registry.rs` `DocRegistry`; `features/crdt/crdt/src/peer.rs` `exchange`; `features/crdt/crdt/src/schema.rs` `migrate` |
| Optimistic store, offline call queue, device rows, connection state | `references/offline-atom.md` | `features/atom/src/mutation.rs` `run_queued`; `features/atom/src/queue.rs` `CallQueue` |
| Principal/permits/gate, sign-in, OIDC, device flow, auth server | `references/auth-permissions.md` | `crates/architect/src/permissions_gate.rs` `PermissionsGate`; `features/auth/auth/src/lib.rs` `ArchitectAuth` |
| Sentry, tracing init, `wide::set`, stage timings, OTLP | `references/telemetry.md` | `features/telemetry/src/lib.rs` `init_tracing_full` |
| Dioxus component kit, theme tokens, snapshots, stories | `references/architect-ui.md` | `features/ui/architect-ui/src/prelude.rs`, `ThemeProvider` |
| `cargo xtask`, `boundaries.toml`, fleet-bump, CI, docs check | `references/xtask-tooling.md` | `xtask/src/main.rs` `Command` |

### App plumbing modules

Use these before writing your own. Each is behind a cargo feature and has a docs page under `docs/content/architecture/`.

| Need | Module / feature | Entry points | Docs page |
|---|---|---|---|
| Serve files/audio with Range/206 | `architect::http::ranged` (feature `http`) | `crates/architect/src/http/ranged.rs` `parse_range`, `serve_bytes`, `serve_body`, `serve_file` | `http.md` |
| Typed settings file / device KV | `architect::settings` (feature `settings`) | `crates/architect/src/settings.rs` `Store`, `FileKv` (implements atom `Kv`) | `settings.md` |
| Crash log + last-run flight log | `architect_telemetry::flight` (feature `flight`, native only) | `features/telemetry/src/flight.rs` `install_panic_recorder`, `RingLayer`, `restore_previous`; one hook per process | `flight-recorder.md` |
| Let agents find an app's MCP server | `architect_mcp::discovery` + `Server::serve_loopback` | `features/mcp/src/discovery.rs` `publish`, `project_mcp_json`; `features/mcp/src/http.rs` `serve_loopback_or` (honours `FTS_MCP=0`) | `commands.md` |
| Expiring signed links / grants | `architect::capability` (feature `capability`) | `crates/architect/src/capability.rs` `Capability`, `ServerKeypair` (0600, refuses a loose key file). Revocation is by expiry only | `capability.md` |
| Resumable, hash-verified download; content-addressed store | `architect::blob` (feature `blob`, native first) | `crates/architect/src/blob.rs` `ResumableFetch`, `BlobStore`, `LocalFs` | `blob.md` |
| `/version.json` | `EngineHost::build_info(architect::build_info!())` | `crates/architect/src/host.rs` `BuildInfo`, `build_info`; env `ARCHITECT_BUILD_REV`, `ARCHITECT_BUILT_AT` | `http.md` |
| Test sandbox server, allocation counting | `architect::testing` (features `testing`, `testing-alloc`; dev-dependency only) | `crates/architect/src/testing/sandbox.rs` `sandbox_server` (never reads the real HOME) | `testing.md` |
| Consistent snapshot of a live SQLite store | `architect::snapshot` (feature `snapshot`) | `crates/architect/src/snapshot.rs` `WriteGate`, `GatedLayer`, `quiesce`, `wal_checkpoint` | `snapshot.md` |
| Signed self-update from GitHub Releases | `architect::release` (feature `release`, native only) | `crates/architect/src/release.rs` `Updater`, `Target`, `Options`, `Outcome`; refuses `/nix/store`, a `None` key checks and refuses to install | `release.md` |
| Keymap engine (chords, trie, modes, which-key) | `architect-input` (`features/input`; feature `dioxus` for DOM events and `use_input_processor`) | `features/input/src/processor.rs` `InputProcessor`, `SequenceTimeout` (timer ticket for any host), `MacroKeys` (turn vim `q`/`@` off); `features/input/src/which_key.rs` `WhichKey`; `features/input/src/dioxus/hook.rs` `InputHandle` (`notify_key_release`; callable from a `web_sys` listener outside the runtime); keys bind to Action List ids | `input.md` |

Still not in architect: remote token introspection (I34). Until it lands, Task's `task:apps/server/src/central_auth.rs` `CentralAuth` is the only implementation.

## Rules

1. **Contract first, once.** Every capability is a Rust type or trait: `#[architect::entity]`, `#[architect::error]`, `#[architect::service]` (vox + HTTP, optionally `actions`) or `#[architect::rpc]` (vox only). Never write a second DTO, a hand axum route or a Dioxus server function for something a trait already describes. Wire types are facet-only (no serde next to them; the Entity `json` option and an action's `RawJson` reply are the documented exceptions). One error enum per service. Keep a method to at most 4 params (a convention, not compiler-enforced); more go in one facet struct. Subscriptions are sync `#[subscribe]` markers, not `async fn`.
2. **One command per capability.** Anything a person or an agent can do is an action in the app's `ActionList`: id `AREA_VERB`, facet `Args`/`Out`, an `Effect` (ReadOnly/Mutating/Destructive). Keys (`architect-input`), menus, palette, MCP, CLI and REAPER named commands all run it by id. A service's methods become actions with `#[architect::service(actions)]`. No bespoke MCP tool, menu handler or palette entry. UI-only effects use the queued-window pattern (`studio:apps/session/daw/src/actions.rs` `take_window`); a programmatic `run` never opens a dialog. Register at startup and call `list.validate()` in a test.
3. **Pick the state model per datum.**

   | Data | Model |
   |---|---|
   | Co-edited document (chart, setlist, note) | CRDT doc (`crdt`, Loro) |
   | Long text edited concurrently | `#[architect(crdt(text))]` on the `String` field, or an explicit `LoroText` child (`apply_text_ops`); never a plain derived `String` (that is last-writer-wins) |
   | Computed record / call made offline (timer start) | `atom` `CallQueue` (`Mutation::run_queued`) |
   | Server-owned rows | optimistic `Store` + persisted `LocalRows` |
   | Live signal (meters, playhead) | `PubSub`, never CRDT |
   | Bytes / files | `architect::blob` (`ResumableFetch`, `BlobStore`) for verified transfer; Task's Files lane (blake3 chunks, iroh-blobs) stays app-specific |

   Build and test against `LocalServer` first; every feature must work with the network off.
4. **Every node is a peer** (ADR 0006, Proposed). No code path only a server has. Dial by iroh endpoint id; identify peers as `Principal::Host`; check access per document in the library (`DocRegistry::with_authorizer` sees the caller). Test with three nodes and the always-on host stopped. A directory (fts-auth, n0 relays) must never be the only way to find a peer; LAN lookup exists behind `iroh-lan`.
5. **Boundaries are dependency directions, enforced.** L0 contract (wasm-clean: no tokio runtime, sea-orm, dioxus, axum, iroh) → L1 engine → L2 storage → L3 host/transport (only under `apps/*`) → L4 UI (L0 + `architect[client]` only). Run `cargo xtask boundaries` (rows in `boundaries.toml`, baseline counts ratchet down only). Every mounted service has a permit table (`PermissionsGate::coverage` empty). One architect tag per `Cargo.lock`; a branch `[patch]` only as a labelled experiment. Every crate has `[lints] workspace = true`.
6. **Prove it on every wire.** The same scenario test runs direct, `LocalServer`, WebSocket, iroh, HTTP and, for a service with `actions`, named MCP (copy `apps/auth-server/tests/surfaces.rs`, `examples/greeter/tests/round_trip.rs` and `examples/greeter/tests/mcp.rs`). A bug fix lands with a test that failed before it. Build discipline: `rust-build-efficiency`; function design: `perfect-functions`.

## Recipes

- **New feature:** `cargo xtask feature new <name>` writes ONE crate `features/<name>/` (contract as the default feature set, `memory`/`vox`/`ui` flags, a `spec/`, a passing test), registers it in the workspace and adds a boundaries row. Shape: entity, error, service, backend struct, `impl architect::Services { layers![...] }` (`examples/greeter/src/lib.rs`) → in-memory backend → scenario test across wires → real storage backend last. Layers are cargo features, not sibling crates (Captain decision, STA-257).
- **Mount on a host:** `EngineHost::new(backend.clone().into_router(), bind).plugin(backend.into_http_router()).serve()` (`examples/greeter/src/main.rs`); `/health` and `/vox` are always mounted, `/version.json` with `.build_info(architect::build_info!())`; `.iroh(key_path, id_path)` plus `.iroh_lookup(PeerLookup::N0AndLan)` for peers; protocol endpoints are builder methods, not `.plugin(Router)`: `.vox_at("/org/{slug}/vox", resolver)`, `.document(path, f)`, `.dav(mount, f)`, `.otlp_forward(at, upstream, authorize)`, MCP via `architect_mcp::host::McpHost`; wrap with `PermissionsGate` (start `observe_only`); `architect_telemetry::init_tracing_full` + `wide::set`. Clients share one connection through `architect::connect::pool()`. Details and the HTTP-face limits: `references/rpc-services-hosts.md`.
- **Add a command:** facet `Args`/`Out` → `list.register(&META, Effect::..., |args| ...)?` in the area module's `register(list)`; `register_async` for a handler that awaits; `register_with` / `register_async_with` when it needs the caller (`ActionCaller`: principal + one typed context, e.g. the org). Add it to the registry test (`validate()`, `probe_hostile_args()`) and check `describe` shows the schema. A whole service: `#[architect::service(actions)]` + `<snake>_actions::register(backend, &list)`, marking readers `#[action(effect = read_only)]`. Details: `references/commands-mcp.md`.
- **Serve to a model:** `architect_mcp::Server::new(name, version, list)`; `with_tools(Tools::Named)` (+ `with_tool_name`) keeps an agent's existing tool names; `serve_loopback_or(port, port_env, token_env)` then `serving.publish(name)`. An app with its own MCP endpoint merges tools via `Server::tools_for(&principal)` / `call_tool(&caller, name, args)`. Authorization: `set_authorizer` on the list + `run_json_as` / `handle_as`; the trusted `run_json` skips it.
- **Bind keys:** register the actions, write a `KeymapConfig` JSON whose bindings name Action List ids, build an `InputProcessor`, and run each `InputCommand` it returns through the list (`docs/content/architecture/input.md`). Draw which-key from the `WhichKey` model in the app's own components. A host that sees keyups reports them (`notify_key_release`), or `space space` reads as auto-repeat; without the hook, arm the timeout from `sequence_timeout()` and expire with `expire_sequence(ticket)`; a non-vim app sets `MacroKeys::Off`.
- **Offline write:** call → `Mutation::run_queued` with a `CallTransport`; document → `use_synced_doc_keyed` with File/Turso/IndexedDB persistence; rows → `use_persisted_store_list` over `LocalRows`.
- **Peer-synced document:** stable doc id (UUID v5 from the shared key), `DocRegistry` with `with_authorizer`, pairwise `crdt::peer::exchange` (`DocPeerHost`, `SelfLane`, `pull`). Version the doc with `crdt::schema` (`SchemaStamp`, `migrate`). Loro gotchas: subscription closures must not own a strong `CrdtDoc`; imports don't fire local-update subscriptions; create the text container once and sync before forking edits.
- **Self-update a binary:** `architect::release::Updater::new(app, repo, platform_asset(app)).run(Target { current, exe, key }, &opts)`; sign `SHA256SUMS` with the org release key in the release workflow (`docs/content/architecture/release.md`).
- **Bump architect:** `cargo xtask fleet-bump <tag> <manifests-or-dirs...> [-n]`, then refresh `Cargo.lock` (bump leaves it old, and `boundaries` runs `--locked`), confirm one architect source in the lock, run the wasm check, keep the vox revision identical across server and clients. Releases follow `release-workflow`.
- **UI:** depend on `architect-ui`, `@import` its `fts-theme.css` rather than vendoring a copy; a committed Tailwind sheet must pass `just ui-css-check`. See `references/architect-ui.md`.

## Pitfalls

- `run_json` (keys, menus) is the trusted door and **refuses an async action** rather than block: spawn `run_json_async` there. Only the `_as` / `_with` doors consult the authorizer. `infos()` is unfiltered; use `infos_for(&principal)` for a model or peer. Derived `#[architect::actions]` land as `Mutating` with no schema.
- The router `PermissionsGate` does **not** cover in-process `ActionList` runs: two authorization planes until the app wires `set_authorizer` with the gate's engine.
- `#[architect::service(actions)]` handlers do not see the caller (they register through `register_async`); a per-org service registers by hand with `register_async_with`. A projected action's failure is the error's `Display`, not the HTTP face's `{code, message}`. Not available for `context = T` traits.
- The MCP HTTP face runs every request as the server's one principal; a per-request caller needs `handle_as` / `handle_with` from your own transport. The bearer token is a gate, not an identity.
- Menu and palette adapters from `ActionInfo` are not in architect yet (I13): architect-ui's `Command*` and `Menubar*` take their own items, and Session builds its menu from `infos()` itself.
- `#[architect(crdt)]` needs `repo` and a `Uuid` pk; a plain `String` field is whole-string last-writer-wins, so mark prose `crdt(text)`. `crdt::sync` (`DocSync`) is hub-shaped; `crdt::peer` is the pairwise path. `DocRegistry` admission sees only the doc id; use the authorizer.
- LAN discovery needs feature `iroh-lan` (native, multicast UDP; no-op on wasm).
- `architect::host::init_tracing` (fmt only) is not `architect_telemetry::init_tracing` (Sentry); `wide::set` is a silent no-op without the `otel` feature; telemetry is native only.
- Generated HTTP face is JSON `POST /<prefix>/<method>` plus SSE; subscriptions carry no typed errors. Share pages, media, OAuth and webhooks are HTTP by nature and stay hand-written.
- Boundaries ratchets go one way: when a count drops, lower its baseline in the same PR. Do not grandfather tags to pass the tag-order check; re-pin to a correctly numbered tag.
- Crate layout (STA-257): one crate per feature, layers as cargo features. Don't create new `-proto`/`-<backend>`/`-ui` siblings; don't split or merge existing crates as a side effect.
- Session's `studio:features/session/CLAUDE.md` may still say `#[roam::service]`; it means `#[architect::rpc]`.

## Where changes go

- Framework gaps go **upstream into architect** as Architect-project issues, then a tag, then a repin. Don't fork the fix into an app.
- A PR to architect that adds, renames or removes public API or a feature flag updates this skill (`skills/architect/`) in the same PR.
- Auth engine changes go to `features/auth` / `apps/auth-server`; fts-auth is only the deployment shell.
- The adoption backlog is in the Architect project; check it before starting overlapping work.
