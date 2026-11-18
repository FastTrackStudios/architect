+++
title = "Writing an extension"
description = "Every way to build, run, install, use and test an Architect extension, on one page that links to the rest: runtimes, the manifest, the host API and consent, surfaces, install and sync, the showcase."
weight = 5
+++

An extension adds to Architect what the app's own features add (actions,
views, settings, openers, toolbars, themes, hooks, services) without the
app being rebuilt, and without doing more than the person allowed. Every
user-facing feature of Architect is itself one (#272), and Studio is a
set of them. This page walks through it all and links to the page that
goes deep on each part. Agents load the `architect-extensions` skill
(`skills/architect-extensions/SKILL.md`), which says the same with exact
keys and file paths.

## The principles

- **Powerful by design, gated by consent.** An extension may draw over
  the whole app, reach the vault, the network, secrets, the device and
  MIDI, and a share link's guests run extensions too. None of that is
  removed; each is a capability the manifest declares and the person
  grants. Every call from every runtime goes through one check
  (`features/extensions/src/hostcall.rs` `check`).
- **One implementation per service.** A service (the documents service,
  the vault's query service, node actions, an extension's own
  `[provides]`) is one `#[architect::rpc]` trait with one server. Callers
  hold its generated client, and the edge picks the transport:
  `architect::serve_local` on the desktop and iOS, vox over the org lane
  on the signed-in web, `serve_local` in wasm in the demo. There is never a
  second copy for the web or the node.
- **Browsers get open notes plus a synced index.** Queries run in the
  page over the synced index (`apps/web/src/app/index_sync.rs`);
  full-text search runs on the node.
- **WASI runs on iOS**, release builds included, interpreted by
  Wasmtime's Pulley. HealthKit stays first-party: only a compiled-in
  extension may use it.
- **Installs follow the person.** What a person installs syncs to their
  other devices through the vault's `.architect/`; the production node
  gets its extensions through Nix, pinned and verified.

## Pick a runtime

| You need | Pick | Runs on |
|---|---|---|
| A first-party feature | linked Rust: a `linked::Extension` the app links in, also runnable as a process | desktop, web, iOS |
| Settings, a theme, keymap, presets | `kind = "data"`: no code | everywhere |
| A macro, conditional action, overlay | `kind = "luau"` | desktop, iOS (web: #495) |
| A panel or form, edited live | a page (`[[views]] page = "x.tsx"`) | `QuickJS` on desktop and iOS; a Web Worker in browsers |
| Light logic with no Rust | `kind = "typescript"`: `.ts` actions beside the pages | `QuickJS` on desktop and iOS |
| Sandboxed compute from a third party | `kind = "wasm"`: a WASI component | Wasmtime (desktop), Pulley (iOS); browsers: #495 |
| Another language or a crash-prone tool | `kind = "process"`: a program over vox | desktop only |
| A whole npm app, a canvas | a web view (`[[views]] kind = "web"`) | desktop (`webview` feature), browsers (an iframe), iOS (`WKWebView`, #785) |
| A secret no device may hold | a node action (`[[actions]] runs = "node"`) | the org's node |

What each costs, where it runs and why:
[Extensions](@/architecture/extensions.md) ("The layers", "WASI
components", "Processes over vox", "One trait, three ways to run it",
"What each runner costs", "Server-side parts on the node"),
[Pages as files](@/architecture/pages.md) (pages and TypeScript actions)
and [Embedded views](@/architecture/embedded-views.md) (`paint`, `web`).

The desktop and iOS share one host (`apps/ui/src/extensions.rs`), which
adds the Luau, TypeScript, process and (with `wasi`) WASI runners to the
data and linked ones. The web app's host
(`apps/web/src/app/extensions.rs`) runs the distribution's linked
extensions today; installed Luau, pages and WASI in the browser are
#495's.

## Start one

- **TypeScript:** `architect-ext new --typescript <id> [dir]` (the npm
  package `@fasttrackstudio/architect-cli`,
  `features/extensions/web/cli/README.md`) writes a manifest, an action,
  a page and its types; `architect-ext types` regenerates
  `architect.d.ts`, `architect-ext check` loads it as the app would.
- **Anything else:** copy the nearest example. `examples/showcase/` has
  one per capability (the table in [The extension
  showcase](@/guide/showcase.md), "The example extensions");
  `examples/extension/` has a Luau and a process extension, a registry and
  a web view app; `apps/sandbox/` has the Board (linked and process), the
  Clicker (a remote view), the WASI Counter and Sketch (`paint`, scenes).
- **The walkthrough:** [Your first extension](@/getting-started/extension.md),
  then [Extending an app](@/guide/extending.md).

## The development loop

Develop against the app itself, in an instance of your own: your
extension loaded from its folder, reloaded on save, nothing else in the
way, and the app you use every day untouched.

1. **Have the app, with `architect` on your `PATH`.** From a checkout of
   Architect, `cargo run --release -p architect-desktop -- install`
   installs that build (on a Mac bundled, signed, notarized and stapled
   as a release is, into `/Applications`; on Linux under `~/.local`) and
   links `~/.local/bin/architect` to it (`apps/desktop/src/install.rs`).
   Your extension's repository needs no Architect crate to build.
2. **Run your extension in an isolated instance.** From its repository:

   ```sh
   architect dev --extension path/to/your-extension      # it, and the app's own extensions
   architect dev --extension ./ext --only your.id         # it alone
   architect dev --extension ./ext --open "Charts/A.kf" --hide all   # just its view, no sidebars, rail or status bar
   ```

   Each working folder gets its own instance (its own settings, vault,
   secrets and control socket), so agents in several worktrees each run
   theirs. `--fresh` starts it over; run it again with other flags and
   the running one takes them. The same with no launcher, for a
   `justfile` or devenv: `ARCHITECT_DEV=1` plus `ARCHITECT_DEV_EXTENSIONS`,
   `ARCHITECT_DEV_ONLY`, `ARCHITECT_DEV_OPEN`, `ARCHITECT_DEV_HIDE`, … on
   the app's environment.
3. **Save.** The folder is a development copy: every file is
   fingerprinted twice a second, and a change reloads it as a diff, its
   state kept and its panes where they were. Data, themes, Luau, pages
   and TypeScript take effect at once. A process extension's program (its
   `[entry] path`) is your build's to write, path dependencies and all;
   when it changes, the app restarts the process on it. A view keeps its
   place across that restart with `linked::use_view_state`: what it kept
   (zoom, scroll, selection, a mode, as a string) is its pane's, given
   back when the new program's view starts, and when the pane moves or
   pops out. A broken save keeps the last good copy and says why.
4. **See and drive it.** `dev` prints the instance's `socket`;
   `architect actions --app <socket> run APP_SHOT --json '{"path":"/tmp/a.png"}'`
   draws it, `EXT_LOGS {id}` and `APP_STATUS` say what went wrong, and
   every action your extension adds runs the same way. `architect dev
   path` prints this folder's instance; `dev list`, `dev stop [--all]`.
5. **Check it as CI would.** `architect extensions check <folder>` loads
   it once with no window and renders each view; nonzero when it fails.

More: "An instance of your own" in the `architect-extensions` skill
(`skills/architect-extensions/references/install-lifecycle.md`), and
[Command line and MCP](@/guide/cli-and-mcp.md).

## The manifest

`extension.toml` is read by `features/extensions/src/manifest.rs`
`Manifest`; its contributions by `features/extensions/src/contrib.rs`.
The design of each table is in [Extensions](@/architecture/extensions.md)
("Contributions", "Hooks and the invocation", "Windows, points and
services", "Inline embeds", "Base view types", "Share layers", "The
network and secrets", "Which operating systems"), [The settings
page](@/architecture/settings-page.md) and
[Toolbars](@/architecture/toolbars.md). A few rules no other page states:

- **Unknown keys are ignored, not refused**, in almost every table: a
  misspelt key loads as its default. Only `[[themes]]` (`path` alone),
  `[source]` and `[[share_layers]]` refuse one. Check a manifest after
  every change (below).
- **`api`**: a `0.x` manifest must name this build's minor exactly
  (`manifest::API`); from `1.0`, an older minor of the same major is
  taken.
- **`[entry] args`**: a process's arguments, after `path`.
- **`[[actions]] shortcut`**: a key that runs it (`Mod-Alt-D`), shown and
  bound in the keymap.
- **`[[views]] document`**: a frame document in the package (a view with
  no code); `src` is for `kind = "web"` only and refused on any other
  view.
- **`[[data]]`** `{name, scope}`: document data it keeps, namespaced by
  its id, one container per project (`scope = "project"`, the default) or
  one per object it annotates (`"object"`).
- **`[[toolbars]] hide_labels`** and its `[…style]` `size`, `spacing`,
  `rounding`, `preset` (`features/extensions/src/toolbar.rs`
  `ToolbarStyle`).
- **`[[sections]] view`** draws a settings section with one of its views
  (a page of its own); **`[[sections.settings]] block`** groups a setting
  under a titled block.
- **An opener's `prefix`** (`click:`) is what a ⌘O query starts with to
  ask only that opener.
- **Held views** (#764) have no key: a remote view holds another
  extension's view through a placeholder element
  (`features/extensions/src/embedded.rs`), and the held view runs with
  its own capabilities.

## The host API and consent

An extension reaches the app only through the host service:
`linked::Handle::host` in Rust, `host` in Luau, `architect` in a page or a
TypeScript action, `window.architect` in a web view, the
`architect:extension` WIT world in a component. The calls (actions, the
selection, documents, the vault's query service, files, settings and
data, the editor, `fetch` and secrets, MIDI and OSC ports, notifications,
the clipboard, pickers, prompts, menus, badges, overlays, the device,
timers and events, other extensions' services) and the capability each
needs are in [Extensions](@/architecture/extensions.md) ("Capabilities,
the host service and consent" and the sections after it) and
[Documents](@/architecture/documents.md).

- **A call answers** one of: its value; `refused` with the capability it
  needs and why; `confirmation_required` with a ticket (an extension
  acting for an agent ran something destructive: run it again with the
  ticket once the person says yes); `not_loaded`; `not_found`; `failed`;
  or `unsupported` (this app or platform can't: a browser asked for a
  context menu) (`features/extensions/src/hostcall.rs` `HostError`).
- **Grants** are kept in `<app dir>/extensions/grants.json`: per
  extension, the version and capabilities granted, where (this device or
  the org) and when. An update that asks for no more keeps its grant; one
  that asks for more waits for a new yes
  (`features/extensions/src/consent.rs` `Grants`).
- **Secrets** are stored insert-only, at most 8 KiB each
  (`hostapi::MAX_SECRET`); an extension can use one in a `fetch` to its
  declared hosts but never read it back.
- **The browser's component host** (`features/extensions/src/web.rs`)
  answers `unsupported` for the editor, the interface calls, events,
  device calls, secrets and ports today.
- **A share link's guest** gets a narrow, read-only profile scoped to the
  link (`features/extensions/src/share_needs.rs` `guest_capabilities`);
  running a link's extensions for its guests is #495's G1.

## Reach it

Every action an extension declares is on the app's Action List, so it is
in ⌘K, bindable to keys, runnable from the `actions` CLI and the control
socket, and a tool over MCP, with no extra code. Past that, its manifest
adds toolbar buttons, ribbon and status items, favourites, menus and
context menus, openers (a file type, a document kind, a link, a ⌘O
source), code-block fence views, Bases view types, tabs, panes, windows
and overlays. Remote views get pointer, wheel, key, IME and focus events.
See [Commands](@/guide/commands.md), [CLI and MCP](@/guide/cli-and-mcp.md),
[Toolbars](@/architecture/toolbars.md), [Input](@/architecture/input.md)
and [Extensions](@/architecture/extensions.md) ("Contributions",
"Favourites", "Inline embeds", "Base view types").

The command lines:

| Tool | Commands |
|---|---|
| `architect-ext` (npm, `features/extensions/web/cli`) | `new --typescript`, `types`, `check`, `pack [--key]`, `keygen`, `build-web` |
| `cargo xtask` (this repo) | `ext-pack`, `ext-sign`, `ext-verify`, `ext-keygen`, `ext-index` |
| `archext install` (`features/extensions/archive`) | unpacks a signed archive with no app running (what Nix uses) |
| `architect-desktop extensions` | `check <folder>`, `dev <folder>`, `undev <folder>`, `registries` |
| the app's actions (`EXT_*`) | `EXT_LIST`, `EXT_CHECK`, `EXT_LOGS`, `EXT_DEV`, `EXT_CLONE`, `EXT_RELOAD`, `EXT_ENABLE`, `EXT_DISABLE`, `EXT_GRANT` and the rest, from the `actions` CLI, MCP or ⌘K |

## Install and lifecycle

- **A development copy** loads a folder unsigned (`EXT_DEV`, or
  `architect-desktop extensions dev <folder>`) and reloads it on save: the
  app polls every 500 ms, fingerprinting every file
  (`features/extensions/src/watch.rs`); a reload is a diff, its state kept.
  `EXT_CLONE` copies a built-in or installed one into a development copy,
  and for a built-in with a `[source]` builds its crate and overrides it
  live. See "Hot reload" and "A built-in's development copy" in
  [Extensions](@/architecture/extensions.md).
- **Registries and archives**: a signed `.archext` (a content index of
  SHA-256s, a minisign signature, builds per target), refused unsigned,
  tampered or by an untrusted key; installed from a git or folder
  registry. An install records its link, pin, signer key and signer in
  `installed.toml` (`features/extensions/src/install.rs` `Record`) and
  keeps the archive beside it. See "Packaging", "Distribution", "The
  archive", "Signing and trust" and "The registry".
- **Install on open**: a vault, a project or a share link names the
  extensions it needs; opening it offers them once, with **Install all**.
  A vault carries them in its `.architect/`, and a person's installs sync
  across their devices. See "What a project needs", "Where a vault and a
  project name them", "A person's installs on all their devices" and "A
  share link's extensions".
- **Enable, disable, pin, update, remove**: the extensions page and the
  `EXT_*` actions; an update that asks for more waits for consent.
- **The standard release** ships the core extensions listed in
  `extensions/core-extensions.toml`: [Core
  extensions](@/guide/core-extensions.md).
- **The node** gets its extensions through Nix (`nix/node/extension.nix`),
  each pinned by version, hash and signature and checked when the system
  builds: [Deploy](@/guide/deploy.md).

## Test it and show it

- **Tests per runtime** live in `features/extensions/tests/` (the host,
  install, WASI, processes, remote views and input, node actions),
  `features/pages/tests/` (pages and TypeScript),
  `features/extensions/web/test/` (the browser runtimes, `npm test`) and
  `apps/web/test/` (the web app's host, `just app-linked-extensions-test`
  and its neighbours). See [Testing](@/guide/testing.md).
- **A feature-gated test needs a pass.** `cargo xtask feature-tests`
  fails when a test the workspace build leaves out is neither run by a
  `[[pass]]` nor named by a `[[skip]]` in `feature-tests.toml`
  ([Testing](@/guide/testing.md), "Feature-gated tests").
- **The showcase** is the checklist of what an extension can do on each
  platform, with evidence: [The extension showcase](@/guide/showcase.md).
  `just showcase` runs the app on a scratch vault with every example as a
  development copy; its "Run it and capture it" section covers each
  platform. A cell moves only with an example, a test and screenshots at
  390 px and 1280 px, light and dark.

## Traps

- **Four parameters.** An `#[architect::rpc]` method takes at most four
  besides `&self`: vox sends them as one tuple, and facet implements its
  trait for tuples of up to four. Put more in one wire struct.
- **wasm futures aren't `Send`.** Bound a linked extension's futures with
  `features/extensions/src/linked.rs` `MaybeSend`, keep a wasm-only
  client in a `SendWrapper`, and give a browser host a spawner
  (`Host::with_spawn` with `spawn_local`): without one, hook work is
  dropped with only a log line.
- **Linked UI is drawn by Blitz.** No native `<select>`; a `style { … }`
  node beside the workspace panics its layout (use `document::Style` and
  inline token styles). A process view may set raw HTML only on a
  `<style>`.
- **A new crate** joins the `workspace-hack` (hakari): `cargo hakari
  generate`, `cargo hakari manage-deps`, then `cargo xtask fleet-deps` to
  gate its line, or its wasm build pulls the native union and fails
  (`skills/architect/references/xtask-tooling.md`).
- **Action ids** start with the extension's prefix (`fts.metronome` →
  `FTS_METRONOME_`) or it does not load.
- **Views that read a cache** need a revision prop to redraw; a refresh
  that bumps a signal an effect turns into another refresh freezes the
  app with no log line.
