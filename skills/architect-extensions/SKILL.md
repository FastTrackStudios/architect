---
name: architect-extensions
description: "Use when writing, installing, debugging or reviewing an Architect extension (linked Rust, Luau, TypeScript actions, WASI, process, process views, web views, data, node actions): the manifest, the host API and capabilities, ⌘K/CLI/MCP surfaces, install and sync, and the tests. Rules, decision tables, exact keys."
version: 0.2.0
author: architect
license: MIT
metadata:
  hermes:
    tags: [architect, extensions, manifest, wasi, luau, typescript, webview, process-view, capabilities]
    related_skills: [architect, perfect-functions, rust-build-efficiency]
---

# Architect extensions

An Architect extension adds to an app what the app's own features add (actions, views, settings, openers, toolbars, themes, hooks, services) without the app being rebuilt, and without doing more than the person allowed. In an app built on Architect every user-facing feature is itself one (#272). This skill is the how-to; the `architect` skill (`skills/architect/SKILL.md`) is the framework's API map.

Architect is the runtime: manifests, the host and its capability check, every runner, install and sync (`features/extensions`); the Dioxus side extensions link (`features/extensions-dioxus`). The chrome that shows extensions in a window (panes, remote views, toolbars, the settings sheet, the manager page, the palette) is app-shell's: `app-shell:crates/app-shell-extensions`.

Every architect path here is checked by `cargo xtask docs`. When a doc and the code disagree, trust the code (`features/extensions/src/manifest.rs` is the manifest's truth). The human walkthrough is `docs/content/extensions/authoring.md`.

## Owner decisions (they decide what is allowed)

- **Every kind stays.** Compiled-in Rust, process, Luau, TypeScript, WASM and data: the set of runtimes is what makes the system powerful.
- **Extensions are powerful by design.** They may draw over the whole app (`[[overlays]]`, `ui = ["overlay"]`), and share-link guests run extensions. Power is gated behind declared, consented capabilities, never removed: every call goes through `features/extensions/src/hostcall.rs` `check` against the manifest's `[capabilities]` narrowed by the person's grant.
- **A share link is the app, as a guest.** A link opens the app as `Principal::Guest` of that link: the same app, views and extensions. An extension never ships a guest-only view or renderer, and the host never loads a guest-only extension set; what a guest can't do is a capability it isn't granted (`features/extensions/src/share_needs.rs` `guest_capabilities`, `Host::for_guest`, `hostcall::guest_check`).
- **One implementation per service.** A service is one `#[architect::rpc]` trait with one server; callers hold the generated `<T>Client` and the edge picks the transport (`serve_local` in process, vox over a lane). Never write a second copy for web or node.
- **HealthKit stays first-party only** (`features/extensions/src/device.rs` `HEALTH_LINKED_ONLY`).
- **Installed extensions sync across a person's devices** (`.architect/`, `features/extensions/src/person_sync.rs`). **A production node gets its extensions through Nix**, pinned and verified (`nix/node/extension.nix`).

## Which runtime

| You need | Pick | Runs on |
|---|---|---|
| A first-party, user-facing feature | linked Rust (`linked::Extension`, `Extensions::with_linked`), with a process copy via `[source]` | desktop, web, iOS |
| Settings, a theme, keymap or preset, no logic | `kind = "data"` | everywhere |
| A person's macro, conditional action, overlay | `kind = "luau"` | desktop, iOS; actions and hooks in a browser (luau-web in Web Workers) |
| Light logic, no Rust toolchain | `kind = "typescript"` (`.ts` actions) | where the app gives the host a runner for it |
| Sandboxed third-party compute | `kind = "wasm"` (WASI component, `[wasm] component`) | Wasmtime on desktop, Pulley on iOS (web: #495, JSPI) |
| Another language, a crash-prone tool | `kind = "process"` (`[entry] path`, vox) | desktop only |
| A native GPU renderer (Bevy, wgpu) in a pane | a process view: `[[views]] kind = "process"`, `program`, plus `capabilities.programs`; any extension kind, any copy | Linux and macOS desktop; Windows, iOS and the browser: the pane says why |
| A whole npm app, canvas/WebGL | a web view: `[[views]] kind = "web"`, `src` | desktop, browser iframe, iOS WKWebView |
| A secret the devices must never hold | a node action: `[[actions]] runs = "node"` | the org's node; apps forward to it |
| Per-frame or per-sample work | Rust inside a linked extension | — |

Details, minimal examples and tests for each: `references/runtimes.md`.

## Where to look

| Need | Read |
|---|---|
| Every runtime: keys, where it runs, a minimal example, its tests | `references/runtimes.md` |
| `extension.toml` key by key: actions, views and kinds (remote, `paint`, scenes, overlays, held views, `web`, `process`), workers, hooks, settings, contributions and points, themes, `[[secrets]]`, `platforms`, `[provides]`/`[requires]`/`[uses]`, `[[share_layers]]` | `references/manifest.md` |
| Every host call by area, capability keys, `hostcall::check`, consent and grants, what a share guest gets | `references/host-api.md` |
| ⌘K, ⌘O, keys, the `architect-ext` CLI, the actions CLI and control socket, MCP, toolbars, favourites, notifications, openers, fence views, remote input | `references/surfaces.md` |
| Dev copies and hot reload, registries, `.archext` and signing, install-on-open, `.architect/` and sync, enable/disable/pin/update/remove, Nix on the node | `references/install-lifecycle.md` |
| Tests per runtime, `cargo xtask feature-tests`, "can I do X on web or iOS" | `references/testing.md` |
| Traps that cost real time | `references/pitfalls.md` |
| The design and the long-form reference | `docs/content/architecture/extensions.md`, `docs/content/architecture/embedded-views.md`, `docs/content/architecture/process-views.md` |

## Rules

1. **Declare, then call.** Every host call needs its capability in `[capabilities]` (`reads`, `writes`, `hosts`, `ports`, `ui`, `device`, …) and the person's consent; an update that asks for more waits for a new yes. Never work around a refusal; ask for the capability.
2. **Action ids start with the extension's prefix** (`fts.metronome` → `FTS_METRONOME_`, `Manifest::action_prefix`), or the extension does not load.
3. **Reach the app through the host service** (`linked::Handle::host`, the SDKs' `host`), never through Rust handles into the app: then the process and web copies behave the same.
4. **A new user-facing feature is a linked extension** in a crate of its own with an `extension.toml` and a `[source]` (`docs/content/architecture/extensions.md`, "The rule"), so `EXT_CLONE` can swap a dev copy in.
5. **One service, one server, a client everywhere.** Use `[provides]` for your API and `[requires]`/`[uses]` for others'; call them through the host, never by linking their crate.
6. **Unknown manifest keys are mostly ignored, not refused** (only `[[themes]]`, `[source]` and `[[share_layers]]` refuse them). Run `EXT_CHECK` / `architect-ext check` on every manifest change.
7. **Prove it with a test**, and with 390 px and 1280 px screenshots, light and dark, for anything it draws (`references/testing.md`).
8. **Feature-gated tests need a pass** in `feature-tests.toml`, or `cargo xtask feature-tests` fails the PR.
9. **Say "not built yet (#N)"** for anything planned. Don't claim a platform you haven't seen it run on.
10. **A PR that changes the manifest, host API, capabilities, a runtime or the install flow updates this skill** and the docs page it cites, in the same PR.

## The loop

1. Scaffold: `architect-ext new --typescript <id> [dir]` (npm `@fasttrackstudio/architect-cli`, `features/extensions/web/cli/README.md`), or copy the nearest example in `features/extensions/tests/fixtures/examples`.
2. Run it as a development copy in the app that hosts it: `EXT_DEV {folder}` over its actions CLI or control socket (`architect-actions --app <app> run EXT_DEV --arg folder=<dir>`). It reloads on save (polled); a process extension's program is its own build's to write. `EXT_CLONE` copies a built-in. Details: `references/install-lifecycle.md`; the person's page: `docs/content/extensions/authoring.md`, "The development loop".
3. Check: `EXT_CHECK`, `EXT_LOGS`, `APP_STATUS` over the actions CLI (`references/surfaces.md`).
4. Test: the runtime's test file (`references/testing.md`), then `cargo xtask feature-tests` if you gated one.
5. Ship: `architect-ext pack --key …` or `cargo xtask ext-pack` / `ext-sign` into a signed `.archext`, and a registry entry.

## Pitfalls (short list; `references/pitfalls.md` has cause and fix)

- `#[architect::rpc]` methods take at most four parameters besides `&self` (vox sends a tuple; facet stops at four): put the rest in one wire struct.
- On wasm, futures aren't `Send`: bound with `MaybeSend`, hold wasm-only clients in `SendWrapper`, give a browser host `Host::with_spawn`.
- Linked UI runs on Blitz: no native `<select>`, no `style { … }` node beside the workspace (use `document::Style`); see `skills/architect/references/blitz-quirks.md`.
- A new crate names every feature it needs in its own `Cargo.toml`; nothing unifies them for it.
- A view that reads a cache needs a revision prop; a refresh → bump → effect loop freezes the app with no log.
