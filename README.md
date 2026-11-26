# architect

An application framework for Rust apps: declare each service once and architect gives you every wire; put every command on one Action List that keys, menus, scripts, extensions and MCP all run; and build the app's features as extensions of every kind — compiled-in Rust, a process, Luau, TypeScript, WASM or plain data — compiled in for speed, loaded at run time while you develop.

Architect is the framework, not an app and not a UI kit. The app's shell, workspace (docks, splits, tabs), toolbars, palette and UI are [FastTrackStudio/app-shell](https://github.com/FastTrackStudios/app-shell), which draws Architect's extensions (`app-shell:crates/app-shell-extensions`); Studio is an app built on both.

[![CI on main](https://img.shields.io/github/check-runs/FastTrackStudios/architect/main?label=CI)](https://github.com/FastTrackStudios/architect/actions/workflows/checks.yml?query=branch%3Amain) [![Latest tag](https://img.shields.io/github/v/tag/FastTrackStudios/architect?sort=semver)](https://github.com/FastTrackStudios/architect/tags) [![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Pre-1.0: any release may break the API · Rust 1.99.1 (`rust-toolchain.toml`) · not on crates.io: depend on a release tag.

## The 30-second version

```rust
use uuid::Uuid;

// A service. From this one trait: the vox service, `POST /greeter/greet`,
// `POST /greeter/count`, `/greeter/greeted` as Server-Sent Events,
// `GreeterClient` (vox) and `GreeterHttpClient`, both implementing
// `Greeter`; and with `actions`, the Action List actions `GREETER_GREET`
// and `GREETER_COUNT` that keys, menus, scripts and MCP run.
#[architect::service(actions)]
pub trait Greeter {
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    #[action(effect = read_only)]
    async fn count(&self) -> Result<u32, GreetError>;
    #[subscribe]
    fn greeted(&self) -> String;
}

// An error: the wire derives, an HTTP status and code per variant, and
// the `Transport(String)` variant that lets the clients implement the trait.
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]
    Unknown(String),
}

// An entity: `NoteCreate`, `NoteUpdate`, `NoteList`, the `NoteRepo` trait
// (get / list / create / update / delete), its clients and routes, the
// `NoteEvent` feed, the SeaORM storage and the `NoteMigration`.
#[architect::entity(table_name = "notes", repo, events)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub title: String,
    pub body: String,
}
```

A backend implements the traits and lists its bundle; one server serves
vox over WebSocket, the HTTP face and `/health` from it:

```sh
curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'   # "hello cody"
curl -N     localhost:4040/greeter/greeted                       # data: "hello cody"
```

Because the clients implement the trait, one scenario test covers every
wire:

```rust
async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
}

scenario(&Memory::default()).await;                              // in process
scenario(&local.establish::<GreeterClient>().await?).await;      // vox
scenario(&GreeterHttpClient::at("http://127.0.0.1:4040")).await; // HTTP
```

## What it gives you

| Area | What you get | Start at |
|---|---|---|
| Contracts | Services and entities declared once: vox (in process, WebSocket, iroh) and HTTP+JSON with SSE from one trait, typed errors, SeaORM storage and migrations, one `EngineHost` server | `docs/content/features/services-and-entities.md` |
| Commands | The Action List: every capability an action by id with a JSON Schema, run by the ⌘K palette, ⌘O, menus, keys, the CLI and MCP | `docs/content/features/commands.md` |
| Input | Key bindings with chords, sequences, modes and which-key, the person's own keymap and a binding editor, MIDI and OSC | `docs/content/features/key-bindings.md` |
| Extensions | The runtime for every kind (linked Rust, process, Luau, TypeScript, WASI, data): manifests, capabilities and consent, registries and signed archives, development copies that hot-reload, process views drawn in the app's panes | `docs/content/features/extensions.md`, `docs/content/features/remote-views.md` |
| Scripting | Luau scripts over the Action List as one undo step, saved scripts as actions | `docs/content/features/scripting.md` |
| MCP | An app's Action List served to a model, one tool per action, or five generic tools over the whole list, discoverable on loopback | `features/mcp/src/lib.rs` |
| Documents | CRDT documents (Loro) with kinds and migrations, named undo, and a crash-safe file per document with branches and versions | `docs/content/features/documents.md`, `docs/content/features/document-file.md` |
| Sync and collaboration | Org sync, peer to peer over iroh with share links and LAN discovery, presence (pointers, selections) and collaborative text | `docs/content/features/org-sync.md`, `docs/content/features/peer-to-peer.md`, `docs/content/features/presence-and-text.md` |
| Blobs | Bytes by sha256, fetched on demand from a peer or a server, pinned and collected | `docs/content/features/blobs.md` |
| Identity | Sign-in against one auth server, sessions and devices, permissions on every service call, organizations as the scope of everything | `docs/content/features/auth.md`, `docs/content/features/permissions.md`, `docs/content/features/organizations.md` |
| Settings | `#[derive(Settings)]` on a struct, values by scope (device, the person's synced, the project's) under an org's defaults and locks, a settings page with search | `docs/content/features/settings.md` |
| Scenes and process views | Views painted by the GPU or drawn in another process, timed against a frame budget | `features/scene`, `features/process-view`, `features/views` |

## Install

Prerequisites: Nix and devenv for the dev shell (`devenv.nix`), or the
pinned Rust toolchain and the GTK/WebKit libraries the desktop crates
link against.

```toml
[dependencies]
architect = { git = "https://github.com/FastTrackStudios/architect", tag = "vX.Y.Z", features = ["client"] }
```

Use the newest tag from [Releases](https://github.com/FastTrackStudios/architect/releases),
and one tag for every crate in a `Cargo.lock`: vox handshakes fail across
mismatched revisions. The feature crates (`architect-workspace`,
`architect-commands`, `crdt`, …) come from the same repository and tag.
A consumer also carries the root `Cargo.toml`'s `[patch.crates-io]`
entries for the vendored forks.

| Feature | Turns on |
|---|---|
| `client` | Typed vox clients, the optimistic store and forms, for a UI crate (wasm or desktop) |
| `server` | vox, SeaORM storage, the axum WebSocket transport and the HTTP face, for a server |
| `host` | `EngineHost`: one binary serving `/vox`, `/health`, the HTTP face and an SPA |
| `iroh`, `iroh-lan` | vox over iroh QUIC, dialled by endpoint id; LAN discovery |
| `action-list` | The Action List registry |
| `local` | The in-process transport, native and wasm |

Every flag, what it implies and what it pulls: `docs/content/architecture/features.md`.

## Start a feature

```sh
devenv shell
just check                        # cargo check --workspace --all-targets
just test
cargo xtask feature new <name>    # a feature crate with a passing test
```

## Workspace

| Path | What it is |
|---|---|
| `crates/architect` | The crate you depend on: services, layers, hosts, transports, the Action List, behind features |
| `crates/architect-derive`, `crates/architect-rpc-derive`, `crates/architect-action-derive` | The entity, error, service and action macros |
| `features/extensions`, `features/extensions-dioxus` | The extension runtime of every kind, and its Dioxus side (the guest renderer, its scenes) |
| `features/control`, `features/mcp` | The Action List from the command line and a control socket, and to a model over MCP |
| `features/scene`, `features/process-view`, `features/views` | Painted scenes, process views, view kinds and their meters |
| `features/` | And the other features: `input`, `settings`, `scripting`, `auth`, `permissions`, `orgs`, `crdt` and more |
| `auth:apps/auth-server`, `auth:apps/mock-oauth` | The identity server, and a mock OAuth provider for its tests |
| `apps/site` | The public site and these docs |
| `skills/architect`, `skills/architect-extensions` | The agent skills: the framework, and writing an extension |
| `xtask` | `cargo xtask`: `feature new`, `boundaries`, `docs`, `tags`, `fleet-bump` |

## Documentation

- Getting started: `docs/content/getting-started/_index.md`, then the
  walkthroughs `docs/content/getting-started/walkthrough.md`,
  `docs/content/getting-started/scripting.md` and
  `docs/content/getting-started/extension.md`.
- The guide, chapter by chapter from the contract to extensions:
  `docs/content/guide/_index.md`.
- One page per feature, with the least code to use it and the tests to
  read: `docs/content/features/_index.md`.
- Design pages: `docs/content/architecture/_index.md`. The crate map and
  macro attributes: `docs/content/reference/_index.md`.
- What shipped in which tag: `CHANGELOG.md`.

## For coding agents

`skills/architect/` is the skill agents load to build on architect: the
rules, an API map and recipes, with every repo path checked by
`cargo xtask docs`. `skills/architect-extensions/` is its companion for
writing, installing, testing and reviewing an extension. The fleet's skill
sync vendors both from each release tag; in this repo `.claude/skills/`
links to them, and `just skill-install` links this checkout's copies into
`~/.claude/skills` and `~/.codex/skills`. `AGENTS.md` holds the repo's
rules for agents: what a PR updates and how docs cite code.

## When not to use it

- Your app is not Rust. architect is Rust on both ends; a web UI is
  Dioxus compiled to wasm, a desktop UI Dioxus on Blitz.
- You need a stable API. It is pre-1.0, tags land often, and breaking
  changes land in any of them.
- You build outside its pins. It tracks one toolchain, a pinned Dioxus
  revision and FastTrackStudios forks of vox, facet and dioxus-native.
- You need CI coverage on macOS, Windows or iOS. CI builds and tests on
  Linux (`ubuntu-latest`) and checks the wasm build; nothing else runs
  there.

## Contributing

`devenv shell` (or direnv, through `.envrc`), then `just check`, `just test` and `just docs-check`; CI
runs the full suite on every push to `main` (`.github/workflows/checks.yml`).
A PR that adds, renames or removes public API or a feature flag updates
`skills/architect/` (and `skills/architect-extensions/` when it touches
extensions) in the same PR.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT), at your option. Unless you explicitly
state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
