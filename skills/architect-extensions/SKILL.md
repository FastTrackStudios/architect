---
name: architect-extensions
description: "Use when writing, installing, debugging or reviewing an Architect extension (linked Rust, Luau, TSX pages, TypeScript actions, WASI, process, web view, node actions): the manifest, the host API and capabilities, ⌘K/CLI/MCP surfaces, install and sync, the showcase and tests. Rules, decision tables, exact keys."
version: 0.1.0
author: architect (#710 extension showcase)
license: MIT
metadata:
  hermes:
    tags: [architect, extensions, manifest, wasi, luau, quickjs, typescript, webview, capabilities, showcase]
    related_skills: [architect, perfect-functions, rust-build-efficiency]
---

# Architect extensions

An Architect extension adds to the app what the app's own features add (actions, views, settings, openers, toolbars, themes, hooks, services) without the app being rebuilt, and without doing more than the person allowed. Every user-facing feature of Architect is itself one (#272). This skill is the how-to; the `architect` skill (`skills/architect/SKILL.md`) is the framework's API map.

Every architect path here is checked by `cargo xtask docs`. When a doc and the code disagree, trust the code (`features/extensions/src/manifest.rs` is the manifest's truth). The human walkthrough is `docs/content/extensions/authoring.md`.

## Owner decisions (they decide what is allowed)

- **Extensions are powerful by design.** They may draw over the whole app (`[[overlays]]`, `ui = ["overlay"]`), and share-link guests run extensions. Power is gated behind declared, consented capabilities, never removed: every call goes through `features/extensions/src/hostcall.rs` `check` against the manifest's `[capabilities]` narrowed by the person's grant.
- **A share link is the app, as a guest.** A link opens the Architect web app (or the native app) as `Principal::Guest` of that link: the same app, views and extensions. An extension never ships a guest-only view or renderer, and the host never loads a guest-only extension set; what a guest can't do is a capability it isn't granted (`features/extensions/src/share_needs.rs` `guest_capabilities`). Today's guest-only paths are gaps to remove (#805, `docs/content/guide/shares.md`).
- **One implementation per service.** A service is one `#[architect::rpc]` trait with one server; callers hold the generated `<T>Client` and the edge picks the transport: `serve_local` on desktop and iOS, vox over the org lane on the signed-in web, wasm `serve_local` in the demo. Never write a second copy for web or node. (Today the documents service is `serve_local` in the browser too, over `WebDocuments`: `references/host-api.md`.)
- **Browsers get open notes plus a synced index.** Vault queries run client-side (`apps/web/src/app/index_sync.rs`); full-text search runs on the node.
- **WASI extensions are on in iOS release builds**, interpreted by Wasmtime's Pulley (`cargo xtask dist ios`; `--no-wasi` is the fallback). **HealthKit stays first-party only** (`features/extensions/src/device.rs` `HEALTH_LINKED_ONLY`).
- **Installed extensions sync across a person's devices** (`.architect/`, `features/extensions/src/person_sync.rs`). **The production node gets its extensions through Nix**, pinned and verified (`nix/node/extension.nix`).

## Which runtime

| You need | Pick | Runs on |
|---|---|---|
| A first-party, user-facing feature | linked Rust (`linked::Extension`, `Extensions::with_linked`), with a process copy via `[source]` | desktop, web, iOS |
| Settings, a theme, keymap or preset, no logic | `kind = "data"` | everywhere |
| A person's macro, conditional action, overlay | `kind = "luau"` | desktop, iOS (web: #812) |
| A panel, list or form edited live | a page: `[[views]] page = "*.tsx"` | QuickJS on desktop and iOS; a Web Worker in the web app (installed TypeScript extensions' pages) |
| Light logic, no Rust toolchain | `kind = "typescript"` (`.ts` actions beside TSX pages) | QuickJS on desktop and iOS; Web Workers in the web app |
| Sandboxed third-party compute | `kind = "wasm"` (WASI component, `[wasm] component`) | Wasmtime on desktop, Pulley on iOS (web: #495, JSPI) |
| Another language, a crash-prone tool | `kind = "process"` (`[entry] path`, vox) | desktop only |
| A whole npm app, canvas/WebGL | a web view: `[[views]] kind = "web"`, `src` | desktop (`--features webview`), browser iframe, iOS WKWebView |
| A secret the devices must never hold | a node action: `[[actions]] runs = "node"` | the org's node; apps forward to it |
| Per-frame or per-sample work | Rust inside a linked extension | — |

Details, minimal examples and tests for each: `references/runtimes.md`.

## Where to look

| Need | Read |
|---|---|
| Every runtime: keys, where it runs, a minimal example, its tests | `references/runtimes.md` |
| `extension.toml` key by key: actions, views and kinds (remote, `paint`, scenes, overlays, held views, `web`), workers, hooks, settings, contributions and points, themes, `[[secrets]]`, `platforms`, `[provides]`/`[requires]`/`[uses]`, `[[share_layers]]` | `references/manifest.md` |
| Every host call by area, capability keys, `hostcall::check`, consent and grants, what a share guest gets | `references/host-api.md` |
| ⌘K, ⌘O, keys, the `architect-ext` CLI, the actions CLI and control socket, MCP, toolbars and ribbon, favourites, notifications, openers, fence views, Bases views, remote input | `references/surfaces.md` |
| Dev copies and hot reload, registries, `.archext` and signing, install-on-open, `.architect/` and sync, enable/disable/pin/update/remove, core extensions, Nix on the node | `references/install-lifecycle.md` |
| `just showcase`, tests per runtime, `cargo xtask feature-tests`, adding a showcase row, "can I do X on web or iOS" | `references/testing-showcase.md` |
| Traps that cost real time | `references/pitfalls.md` |
| The platform matrix with evidence | `docs/content/guide/showcase.md` (link it; don't copy it) |
| The design and the long-form reference | `docs/content/architecture/extensions.md`, `docs/content/architecture/pages.md`, `docs/content/architecture/embedded-views.md` |

## Rules

1. **Declare, then call.** Every host call needs its capability in `[capabilities]` (`reads`, `writes`, `hosts`, `ports`, `ui`, `device`, …) and the person's consent; an update that asks for more waits for a new yes. Never work around a refusal; ask for the capability.
2. **Action ids start with the extension's prefix** (`fts.metronome` → `FTS_METRONOME_`, `Manifest::action_prefix`), or the extension does not load.
3. **Reach the app through the host service** (`linked::Handle::host`, the SDKs' `host`), never through Rust handles into the app: then the process, page and web copies behave the same.
4. **A new user-facing feature is a linked extension** in a crate of its own with an `extension.toml` and a `[source]` (`docs/content/architecture/extensions.md`, "The rule"), so `EXT_CLONE` can swap a dev copy in.
5. **One service, one server, a client everywhere.** Use `[provides]` for your API and `[requires]`/`[uses]` for others'; call them through the host, never by linking their crate.
6. **Unknown manifest keys are mostly ignored, not refused** (only `[[themes]]`, `[source]` and `[[share_layers]]` refuse them). Run `EXT_CHECK` / `architect-ext check` / `architect-desktop extensions check <folder>` on every manifest change.
7. **Prove it per platform.** A showcase cell moves only with an example, a test and 390 px and 1280 px screenshots, light and dark (`references/testing-showcase.md`). Verify extension UI on `just dev` / `just demo` before a PR (`AGENTS.md`).
8. **Feature-gated tests need a pass** in `feature-tests.toml`, or `cargo xtask feature-tests` fails the PR.
9. **Say "not built yet (#N)"** for anything planned. Don't claim a platform the matrix doesn't show.
10. **A PR that changes the manifest, host API, capabilities, a runtime or the install flow updates this skill** and the docs page it cites, in the same PR.

## The loop

1. Scaffold: `architect-ext new --typescript <id> [dir]` (npm `@fasttrackstudio/architect-cli`, `features/extensions/web/cli/README.md`), or copy the nearest example in `examples/showcase/` or `examples/extension/`.
2. Run it as a development copy: `just showcase` loads every `examples/showcase/*` folder; for your own, `architect-desktop extensions dev <folder>` or `EXT_CLONE` for a built-in. It reloads on save (polled).
3. Check: `EXT_CHECK`, `EXT_LOGS`, `APP_STATUS` over the actions CLI (`references/surfaces.md`).
4. Test: the runtime's test file (`references/testing-showcase.md`), then `cargo xtask feature-tests` if you gated one.
5. Ship: `architect-ext pack --key …` or `cargo xtask ext-pack` / `ext-sign` into a signed `.archext`; a registry entry; for the standard release, `extensions/core-extensions.toml` (`docs/content/guide/core-extensions.md`).

## Pitfalls (short list; `references/pitfalls.md` has cause and fix)

- `#[architect::rpc]` methods take at most four parameters besides `&self` (vox sends a tuple; facet stops at four): put the rest in one wire struct.
- On wasm, futures aren't `Send`: bound with `MaybeSend`, hold wasm-only clients in `SendWrapper`, give a browser host `Host::with_spawn`.
- Linked UI runs on Blitz: no native `<select>`, no `style { … }` node beside the workspace (use `document::Style`); see `skills/architect/references/blitz-quirks.md`.
- A new crate must join the `workspace-hack` (hakari) correctly: `skills/architect/references/xtask-tooling.md`.
- A view that reads a cache needs a revision prop; a refresh → bump → effect loop freezes the app with no log.
