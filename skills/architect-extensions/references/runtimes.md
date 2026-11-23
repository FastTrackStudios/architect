# Runtimes: every way an extension's code runs

Which runner runs an extension's code, what selects it in the manifest, where it runs, what it cannot do, a working example and the test that proves it.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` ("The layers", "WASI components", "On the web", "Linked extensions in the browser", "Processes over vox", "Responsive by design: views and workers", "One trait, three ways to run it", "What each runner costs", "Server-side parts on the node", "On each platform"), `docs/content/architecture/embedded-views.md` ("Web views"), `docs/content/getting-started/extension.md`, `docs/content/guide/extending.md`.

## Owner decisions

- **Every kind stays.** Compiled-in Rust, process, Luau, TypeScript, WASM and data: the set of runtimes is what makes the system powerful. Don't propose dropping one.
- **Powerful by design, gated by consent.** An extension may do a lot: documents, the network, secrets, the device, MIDI. Every runner's calls go through the same host service and the same check: what the manifest declares under `[capabilities]`, narrowed by what the person granted (`features/extensions/src/hostcall.rs` `check`). Nothing gets ambient access.
- **One implementation per service.** A service is one `#[architect::rpc]` trait with one server. Callers hold the generated `<T>Client`. The edge picks the transport: `architect::serve_local` in process (`crates/architect/src/local.rs` `serve_local`), vox over a lane to a server. Don't write a second copy for the web or the node. Examples: `features/extensions/src/docs_service.rs` (the documents service, `DocumentsClient`) and `features/extensions/src/node.rs` `NodeActions` (one server, `NodeHost`; every client holds `NodeActionsClient`).
- **HealthKit stays first-party.** Only compiled-in extensions may read or write health data. An installed or development copy is refused (`features/extensions/src/device.rs` `HEALTH_LINKED_ONLY`).
- **A production node gets its extensions through Nix.** Each is pinned by version, hash and signature and checked at system build time. Nothing is downloaded at run time (`nix/node/extension.nix`, `nix/node/module.nix`).

## At a glance

`kind` is the manifest's top-level key (`features/extensions/src/manifest.rs` `Kind`: `data`, `luau`, `typescript`, `wasm`, `process`). Web views and process views are view-level keys, so any kind can have them.

A host runs a kind through a `Runner` (`features/extensions/src/host.rs` `Runner`). `Host::new` has the data and linked runners (`default_runners`); the app adds the rest with `Host::with_runner`: `LuauRunner` (`features/extensions/src/host.rs`), `ProcessRunner` (`features/extensions/src/process.rs`), `WasiRunner` (`features/extensions/src/wasi.rs`, feature `wasi`), and whatever runner it has for `typescript`. A kind with no runner is "Can't run here" with why, before anything installs (`why_no_runner`, `kind_cannot_run`).

| Runtime | Selected by | Desktop | Browser | iOS | Node |
|---|---|---|---|---|---|
| Linked (Rust) | code: `Extensions::with_linked` | yes | yes | yes | no |
| Luau | `kind = "luau"`, `script = "*.luau"` | yes | luau-web in Web Workers, when the app gives the runner (#812) | yes | node actions only |
| TypeScript actions | `kind = "typescript"`, `script = "*.ts"` | the app's runner | the app's runner | the app's runner | no |
| WASI component | `kind = "wasm"`, `[wasm] component` | Wasmtime + Cranelift | jco + Web Workers (`WebComponent`; needs JSPI, #495) | Wasmtime + Pulley | not yet |
| Process | `kind = "process"`, `[entry] path` | yes | never | never | no |
| Process view (GPU frames) | `[[views]] kind = "process"`, `program` | Linux (dma-buf) and macOS (`IOSurface`), shared memory as the fallback; Windows says why | never: the pane says why | never: the pane says why | no |
| Web view app | `[[views]] kind = "web"`, `src` | a native child web view the app hosts | sandboxed iframe | WKWebView | no |
| Node action | `[[actions]] runs = "node"` | forwards to the node | forwards to the node | forwards to the node | runs it |

A `kind = "data"` extension has no code: settings, presets, themes, keymaps and frame views. It runs everywhere. It is also the usual kind for a web view app, whose code runs only inside its view.

The Dioxus side (views as components, a process's `VirtualDom`s, paint panes, embedded views) is `architect-extensions-dioxus` (`features/extensions-dioxus`); the chrome that shows them (panes, remote views, toolbars, the manager page) is `app-shell:crates/app-shell-extensions`.

## 1. Linked (Rust, compiled in)

**What.** A crate implements `features/extensions/src/linked.rs` `Extension`. `manifest` returns its `extension.toml` text, and `build` gives a `Builder` its actions (`action`, `action_in`), its views, its contributions and its root context (Dioxus views and paint views through `architect_extensions_dioxus::BuilderExt`). The app registers it with `features/extensions/src/extensions.rs` `Extensions::with_linked` as a built-in (`Origin::Builtin`). It runs in the app's own process, and its views are components in the app's document.

**The rule.** Every new user-facing feature is a linked extension in its own crate, with an `extension.toml` and a `[source]` (#272). It may use only what a third party could. It reaches the app through the host service (`linked::Handle::host`), never through Rust handles to app internals.

**Manifest.** The linked copy is chosen by code, not by a key. The same crate also builds a process, so its manifest says `kind = "process"`, `[entry] path` (its binary) and `[source]` (where `EXT_CLONE` finds the crate). An installed or development copy of the same id overrides the built-in live.

**In a browser.**
- Depend on `architect-extensions` with `linked-core` (the trait and `Builder`, no host). Add `guest` only off wasm, by target.
- Action futures are `MaybeSend`, which is `Send` natively and nothing on wasm (`features/extensions/src/linked.rs` `MaybeSend`, `HostFuture`).
- There is no Luau, no host clock or event threads, no install folder, and secrets stay in memory.
- `architect_extensions_dioxus::deferred` links one by its manifest alone and builds (in a browser, fetches) its code the first time it is used.

**Can't.**
- Its views are not metered, so it keeps to the frame budget itself.
- It can't be built as a WASI component yet. A `linked::Extension` as a component is #272.

**Example** (trimmed):

<!-- doc-check: off (a sketch of an extension crate's layout) -->
```toml
# my-ext/Cargo.toml
[dependencies]
architect-extensions = { version = "*", features = ["linked-core"] }
architect-extensions-dioxus = { version = "*" }

[target.'cfg(not(target_family = "wasm"))'.dependencies]
architect-extensions = { version = "*", features = ["guest"] }
```
<!-- doc-check: on -->

```rust
// src/lib.rs
impl Extension for Tasks {
    fn manifest(&self) -> &'static str { include_str!("../extension.toml") }
    fn build(&self, ext: &mut Builder) {
        ext.view("list", ListView)
            .action("TASKS_LIST", list)
            .action("TASKS_ADD", add);
    }
}
// src/main.rs: the same value as a process
fn main() -> Result<(), GuestError> {
    Guest::extension(&my_ext::Tasks).serve()
}
```

**Tests.**
- `features/extensions/tests/linked.rs`: linked, turned off and on, overridden by a development copy.
- `cargo xtask builtins` builds every built-in crate both ways.

## 2. Luau scripts

**What.**
- Luau through mlua (`features/scripting/src/luau.rs` `Runtime`), run by `features/extensions/src/host.rs` `LuauRunner`. A run is one undo step when the runner has transactions.
- Scripts reach the host service as `host.*`: actions, settings, data, documents, the interface, `host.fetch` and secrets (`features/extensions/src/luau_host.rs`).
- Luau also drives `[[overlays]] script` (Canvas and ImGui, live), `[[workers]] script` and node actions.

**Manifest.** `kind = "luau"`. Each action has `script = "greet.luau"`. A `.ts` script in a `luau` extension is refused (`ManifestError::ScriptKind`).

**Where.**
- Desktop and iOS: yes.
- Browser: mlua's Luau does not build for `wasm32-unknown-unknown`, so a browser app runs luau-web's emscripten build (`scripts/vendor-luau-web.sh`) in a Web Worker per run, as the runner it gives the host. Every global is a message the page answers with the one implementation the desk uses: `host.*` with `features/extensions/src/luau_calls.rs` `host_request` and the host service (`WebHost::through`), `ext.*` with `ext_call`, `actions.*` and `state.query` with `features/scripting/src/bindings.rs`. A worker runs when its owner first waits on it. No live overlays there. Without the runner the host says why (`why_no_runner`).
- Node: only as node actions (section 7).

**Can't.**
- A frame past `Limits::frame` (500 ms) is stopped, and `Limits::steps` stops runaway loops. Heavy work goes to a `[[workers]]` script.
- It has no network beyond `host.fetch` to `capabilities.hosts`.

**Example** (`features/extensions/tests/fixtures/examples/extension/registry/hello`):

```toml
id = "example.hello"
name = "Hello"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "EXAMPLE_HELLO_GREET"
name = "Hello: greet"
params = '{"type":"object","properties":{"name":{"type":"string"}}}'
script = "greet.luau"
```

```lua
-- greet.luau: `args` is what the caller passed; the return is the result.
local name = if args and args.name then args.name else "you"
return "Hello, " .. name .. "!"
```

A worker: `features/extensions/tests/fixtures/examples/showcase/timers` (`[[workers]] id = "primes"`, `script = "workers/primes.luau"`, `worker.progress(i, n, note)`).

**Tests.**
- `features/extensions/tests/metronome.rs`: install from a registry, reload on save.
- `features/extensions/tests/showcase.rs` and `features/extensions/tests/host_service.rs`.
- `features/extensions/tests/workers.rs`.
- `features/scripting/tests` (`-p architect-scripting`): the runtime itself.

## 3. TypeScript actions

**What.** A `typescript` extension writes its actions and hooks as `.ts` modules. Light extensions need no Rust and no WASM toolchain. Architect checks and installs them; the app gives the host its runner for the kind (`Host::with_runner`), and every host call that runner makes goes through the host service for that extension (`features/extensions/src/web.rs` `WebHost::through` in a browser), so there is no second host.

**Manifest.** `kind = "typescript"`, and each `[[actions]] script` or `[[hooks]] script` is a `.ts`, `.mts`, `.tsx`, `.js`, `.mjs` or `.jsx` module (`features/extensions/src/manifest.rs` `is_ts_script`).

**Where.** Wherever the app registers a runner for `typescript`; elsewhere "Can't run here" with why.

**Can't.**
- Import packages; bundle one into a file. No `fetch`, `fs`, `process` or Node globals.
- Do heavy computation or touch the audio path. Write that in Rust and call it as an action.

**Example** (`features/extensions/tests/fixtures/examples/showcase/recipe-scaler`):

```toml
id = "showcase.recipes"
kind = "typescript"
[capabilities]
actions = ["VAULT_LIST", "VAULT_READ", "VAULT_CREATE", "EXT_VIEW_OPEN"]
effect = "mutating"
ui = ["prompts", "notify"]
[[actions]]
id = "SHOWCASE_RECIPES_LIST"
name = "Recipe scaler: list recipes"
script = "actions/list.ts"
```

**Tooling.** `@fasttrackstudio/architect-cli` (`features/extensions/web/cli`): `architect-ext new --typescript`, `types`, `check` and `pack`.

**Tests.** `features/extensions/tests/browser_installs.rs` (which kinds a browser host runs, and why not), `features/extensions/web/cli/test/ts.test.mjs`.

## 4. WASI components (Wasmtime; Pulley on iOS)

**What.** A WebAssembly component against the `architect:extension` world (`features/extensions/wit/extension.wit`). It imports `host` and `worker` and exports `guest` and `work`.
- Natively, `features/extensions/src/wasi.rs` `WasiRunner` runs it: one instance per thread (an actor), epoch time limits and a memory limit. Compiled code is cached as `.cwasm`.
- Views are the remote-view wire's edits, so the panes that show a process view show a component's unchanged.
- The Rust guest crate is `architect-extension-wasi` (`features/extensions/wasi-guest`).

**Manifest.** `kind = "wasm"` and `[wasm] component = "x.wasm"` (`[entry] path` still works). Workers are `[[workers]] entry = "<name>"` (an entry of the same component) or a separate `*.wasm` in the package.

**Where.**
- Desktop: Cranelift (`wasi` feature). Android compiles natively too.
- iOS: Pulley interpreted (feature `pulley`). A call costs about 3× native (~37 µs p50 on the Simulator).
- Browser: `features/extensions/src/web.rs` `WebComponent`. The same file is transpiled by jco (`features/extensions/web/transpile.mjs`) and runs in Web Workers inside an opaque-origin iframe. It needs JSPI, so Safari doesn't run it yet. The npm package `@fasttrackstudio/architect` runs it in any web app.
- Node: WASI node actions are a follow-up (no issue named).

**Can't.**
- Reach the network, files or environment. An import of `wasi:sockets` or `wasi:http` is refused before linking.
- Exceed 64 MiB and 5 s per call (`wasi::Limits`).
- Use threads; workers take the heavy work.
- Run async actions before WASI 0.3.
- Run its own actions through `run-action`.

**Example** (`features/extensions/tests/wasi-counter`):

```toml
id = "lab.wasi-counter"
kind = "wasm"
[wasm]
component = "architect_extension_wasi_counter.wasm"
[capabilities]
actions = ["THEME_LIST"]
effect = "read_only"
[[workers]]
id = "primes"
entry = "count-primes"
```

```rust
fn build() -> Extension {
    Extension::new()
        .view("counter", Counter)
        .action("LAB_WASI_COUNTER_ASK", |_| ask())
        .worker("count-primes", |work, args: Value| { /* work.check()?, work.progress(…) */ })
}
architect_extension_wasi::export!(build);
```

Build it with `cargo build --target wasm32-wasip2` inside `devenv --profile wasi shell`.

**Tests.**
- `features/extensions/tests/wasi.rs`: view, refused call, both limits, workers, cache. On Pulley: `cargo test -p architect-extensions --features pulley --test wasi`.
- `features/extensions/tests/wasi_wit.rs`: the WIT matches the wire and the host service.
- `features/extensions/tests/wasi_web.rs`, which runs `features/extensions/web/test/web.test.mjs` under node 24.

## 5. Process extensions (over vox)

**What.** A program the host starts and supervises: restarted with backoff, saying why it died. It speaks vox over a local socket, as a remote client does (`features/extensions/src/process.rs` `ProcessRunner`, protocol `features/extensions/src/proto.rs` `ProcessExtension`).
- Its `main` is `Guest::new()…serve()`, or `Guest::extension(&MyLinked).serve()` for a linked extension's process copy (`features/extensions/src/guest.rs` `Guest`; the Dioxus views it serves are `architect_extensions_dioxus::guest`).
- Each view is a `VirtualDom` on its own thread in the process. Its edits are drawn by `app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteView`.
- Use it for isolation, other languages and separate lifetimes, not for build speed.
- A linked extension's view code runs unchanged in its process copy when it uses the both-ways pieces in `features/extensions-dioxus/src/pane.rs`: `use_pane_state` (its pane's id and state), `use_view_state` (where it is in what it shows, kept by its pane and given back when its program restarts on a rebuild, or its pane moves or pops out: keep zoom, scroll, selection and mode there, written when a gesture ends; in a process the pane's input arrives just after the view's first render, so apply it when it first returns `Some`) and `PaintPane` (feature `paint`) for a `PaintView` among its own elements (`docs/content/architecture/embedded-views.md`). `linked::in_process` says which side it is on.
- A program named by `[entry] path` is a file in the package: a development copy's build puts it there. Files `Extension::files` hands a linked copy (toolbar `image`s) must also be in the package for a process copy's host to read them.

**Manifest.** `kind = "process"` and `[entry] path = "<program>"` (plus `args`). Its hello must cover every declared action and view.

**Where.** Desktop only (`Kind::Process` is "desk only"). Never on iOS, Android or the web: a phone may not start programs and a browser has none.

**Can't.**
- Stay inside the frame budget for free. A view has 33 ms (socket round trip included), and opening one costs about 25 ms to its first frame.
- Show more than `MAX_VIEWS` (256) views at once.
- Be called per sample or per frame. Each call costs tens of µs.

**Panes shown together (#891).** Views whose frames must land in the same window frame (an arrangement and its mixer, each a process on one engine) set `data-architect-group` and `data-architect-epoch` on an element (`app-shell:crates/app-shell-extensions/src/barrier.rs` `GROUP_ATTR`, `EPOCH_ATTR`): the host holds a member's frame until every member has drawn that epoch, at most 50 ms. An engine's table holds up to `architect_scene::engine::MAX_SLOTS` (8,192) values.

**Its frames are paced to the display (#840)** by the shell (`app-shell:crates/app-shell-extensions/src/pacing.rs`): the requests until the next refresh share one frame started just before the next swapchain image comes free. `ARCHITECT_FRAME_PACING=0` turns it off.

**What moves every frame is the host's.** A paint view that returns layers (`PaintView::layers`) sends them once (`architect-scene`, `features/scene/src/ops.rs`), and the host draws them every frame without it:
- Motion on a clock: a layer's `X`, `Y`, `ScaleX`, `ScaleY`, `Opacity` or `Visible` from `Clock::Wall` or `Clock::Playhead` (`features/scene/src/anim.rs` `Animation`, `Linear`, `Steps`). A playhead line, a scrolling arrangement, a lyric line. Don't return `animating` true for these: the view would be asked every frame.
- Hit regions and drags (`features/scene/src/region.rs` `Region`, `Drag`): hover, cursor, highlight and a fader or drag box following the pointer, all on the host; a hover (or press) that redraws the element itself: `Region::hover_layer(key)` / `press_layer(key)` naming a layer you send once with `visible = false`; the view hears `RegionEvent`s by id (`PaintView::region`), a drag's value at most once a frame; a `Press` or `Release` with a key held comes just after `Modifiers(id, mods)` (`region::Mods`). A press on a region still gives the view the keyboard focus (`Focus(true)`).
- Named values a layer is bound to (`PaintView::values`, `view.` names); the engine's lock-free `engine.` values with `capabilities.ui = ["engine"]` (`features/scene/src/engine.rs`). An engine in a process of its own shares its table in shared memory (`features/scene/src/engine/shared.rs` `create`, `open`), found through the service the view's extension `[uses]` or `[requires]` (`engine::read_from`).
- The pointer leaving the pane: `ViewInput::PointerLeave`, to a linked paint view, a process copy's paint view, a browser's canvas and a `kind = "process"` program alike; clear a hover there.
- A malformed animation, region or value fails the view, linked in or not.

**Measure a view's frames** with `APP_FRAMES` (`features/workspace/src/frames.rs`: per frame its draw, render, refresh, the input and batches it shows). Numbers and what they show: `docs/content/features/remote-views.md`, "Frames".

**Example:**

```toml
id = "example.counter"
kind = "process"
[entry]
path = "example-extension-counter"
[[views]]
id = "counter"
title = "Counter"
```

```rust
fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)            // a plain Dioxus component
        .action("EXAMPLE_COUNTER_CLICKS", |_args| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .serve()
}
```

**Tests.**
- `app-shell:crates/app-shell-extensions/tests/remote_view.rs`: clicks through a real child process, then kills it.
- `features/extensions/tests/process_views.rs`: the 256-view cap.

## 6. Web view apps (the `web` view kind)

**What.** A full HTML/TS/npm app (DOM, canvas, WebGL, any library) shown in a pane. Its code is untrusted (`Capability::Untrusted`).
- It reaches the app only through `window.architect` (`features/extensions/src/webview/runtime.js`). Every call is a `HostRequest` checked by `hostcall::check` (`features/extensions/src/webview.rs` `Bridge`).
- Its files are served only from its package, under `webview::csp` with `connect-src 'none'`.
- On iOS it is the one way installed code runs JIT-compiled.

**Manifest.** `[[views]] kind = "web"` and `src = "<html in the package>"`. A web view must have `src`, and only a web view may (`Manifest::check`). The extension itself is usually `kind = "data"`.

**Where.**
- Desktop and iOS: a native child web view (wry; WKWebView on iOS) the app hosts over the bridge. On Linux it needs X11 or XWayland (`ARCHITECT_WEBVIEW_X11=1` under Wayland).
- Browser: a sandboxed `<iframe>` with no same-origin (`features/extensions/web/webview-host.js` `mountWebView`).

**Can't.**
- Use its own `fetch`, XHR or WebSocket. Its network is the host's `fetch` to `capabilities.hosts`.
- Navigate away, open windows or use WebRTC.
- Exceed `webview::Budget`: 500 messages and 16 MiB a second.
- Sit under anything the app draws. It is hidden while covered.
- Hear events while its tab is parked.

**Example** (`features/extensions/tests/fixtures/examples/extension/webview-board`, built by `just webview-example`):

<!-- doc-check: off (a path inside the package) -->
```toml
id = "demo.webview-board"
kind = "data"            # its code runs only in its view
[capabilities]
actions = ["EXT_VIEW_OPEN"]
effect = "mutating"
ui = ["notify"]
[[views]]
id = "board"
title = "Board (web view)"
kind = "web"
src = "dist/index.html"
```
<!-- doc-check: on -->

```ts
await architect.setData(KEY, cards);
await architect.notify(`The board has ${cards.length} cards`, { title: "Web view board" });
architect.theme.onChange(showScheme);
```

**Tests.**
- `features/extensions/tests/webview_bridge.rs`: refusals, an insert-only secret, the budget, events.
- `features/extensions/web/test/webview.test.mjs`: headless Firefox.

## 6b. Process views (the `process` view kind, #838)

**What.** A view drawn by a program in the package, on its own GPU device, in its own process: Bevy, a game engine, a renderer in another language. The process-view pane (`features/process-view/src/host.rs` `ProcessViewPane`) shows its frames: dma-buf textures read in place, or shared memory uploaded. It forwards the pane's input, size, focus and theme. It is not a process extension's remote view, which is a Dioxus document the app draws. Any kind of extension may have one, and it is shown the same whichever copy is loaded: built in, installed or in development.

**Manifest.** `[[views]] kind = "process"`, `program = "<file in the package>"`, `args = [...]`. An argument may name one of its own settings, `{{setting:<key>}}`: its value is put in at start and a change restarts the program; a setting alone that comes out empty is left out, so write `--show={{setting:x.show}}` for a value that may be empty. A secret never goes in an argument. The program must also be named in `capabilities.programs`, and `Manifest::check` refuses a process view whose program is missing, unasked or outside the package. Only a process view may name a `program`.

**Consent.** `capabilities.programs` is a capability like any other: asked in words ("Run bin/room, a program in its package, on this computer"), granted by a person for an installed copy, taken back at once, and never a share-link guest's (`share_needs::guest_capabilities`). A built-in or development copy runs with what it asks and is never prompted. An installed copy is asked at install; if the grant is later missing, its pane asks again with an **Allow** button that grants that one program on this device (`process_view::needs_grant`, `grant_program`). The pane re-checks every 500 ms (`features/extensions/src/process_view.rs` `launch`); a session stopped for a revoked grant or an unload is retired, and its Restart starts nothing.

**Where.**
- Linux and macOS desktop, in an app that mounts them (`architect_process_view::extension::use_process_views`, `features/process-view/src/extension.rs`). Frames are dma-buf textures on Linux and `IOSurface`s on macOS (`features/process-view/src/surface.rs`), shared memory where neither imports. Pop-out windows draw it too. `APP_SHOT` does not show its frames (a CPU repaint); capture it with the compositor.
- Windows: the extension loads; its pane says why (`process_view::not_here`). Designed (a DXGI shared handle), not built.
- Browser and iOS: never; the pane says why.

**Lifecycle.** One program per pane (`<extension>/<view>#<pane>`). It is started when the pane opens and keeps running when the pane moves to another group or a pop-out window. It is stopped when its pane leaves the workspace (checked every 250 ms; a move keeps the pane, so never a restart), and at the same check on an unload or a revoked grant. It is restarted like a process extension's program (three tries with backoff, saying why it died). A development copy rebuilt (`EXT_DEV`) starts it again in place.

**The program.** It serves a `PaintView` with `architect_process_view::guest::serve`, or speaks the wire itself (`features/process-view/src/wire.rs`; its socket is in `ARCHITECT_VIEW_FD`). It runs in the package's folder. `just process-view-example` runs the demo (`features/process-view/src/demo.rs`).

**Example:**

<!-- doc-check: off (a path inside the package) -->
```toml
id = "demo.process-view"
kind = "data"            # no code of its own in the app
[capabilities]
programs = ["bin/process-view-demo"]
[[views]]
id = "orbits"
title = "Orbits (process view)"
kind = "process"
program = "bin/process-view-demo"
```
<!-- doc-check: on -->

**Tests.**
- `features/process-view/tests/extension_kind.rs`: installed (refused, granted, frames), revoked, unloaded, as a development copy, rebuilt, killed and restarted.
- `features/process-view/tests/frames.rs`: the wire, dma-buf (Linux), `IOSurface` (macOS) and shared memory.
- `features/extensions/src/manifest.rs` `process_view_tests`, `features/extensions/src/process_view.rs` tests.

## 7. Node actions (server-side, on the org's node)

**What.** The one piece of extension code that runs on the server. It holds a secret the person's devices must never have, such as a token exchange with a client secret (#583). Everything else runs in the app.
- The node loads each installed extension's node part (`Manifest::node_part`: its node actions, their secrets, and only `hosts` and `effect` of its capabilities).
- It serves them with `NodeHost` (feature `node`) on the org lane, behind the org's permission gate.
- Apps register each node action as a forwarder (`Host::with_node`, a `node::NodeLane`), so ⌘K, the CLI, MCP and scripts run it like any action.

**Manifest.** `[[actions]] runs = "node"` with `secrets = ["…"]` naming at least one `[[secrets]]` entry. The action must be a `luau` extension's action with a `script`, and never a preset, a toggle or a window opener (`Manifest::check`).

**Where.**
- Node: it runs here. Production gets it through Nix: `lib.architectExtension` (pinned id, version, SHA-256 and publisher keys, checked by `archext install`, `features/extensions/archive/src/bin/archext.rs`) and `services.architect-node.extensions`. Secrets come from `extensionSecrets`, as sops-nix files, or an org admin's `set_secret`. They are insert-only.
- Apps: forwarded as the account, through the same `NodeActionsClient`.

**Can't.**
- Have views, hooks, workers, a background part, settings, documents, the interface or ports.
- Read its secrets. The host puts them into `{{secret:id}}` headers on requests to their hosts.
- Keep data on the node, and WASI node actions aren't supported. Both are follow-ups with no issue named.

**Example** (`features/extensions/tests/fixtures/examples/showcase/bank-link`):

```toml
kind = "luau"
[capabilities]
hosts = ["127.0.0.1"]
[[secrets]]
id = "client_secret"
title = "Bank API secret"
hosts = ["127.0.0.1"]
[[actions]]
id = "SHOWCASE_BANK_LINK_EXCHANGE"
name = "Bank link: exchange a public token"
script = "node/exchange.luau"
runs = "node"
secrets = ["client_id", "client_secret"]
```

```lua
local r = host.fetch(api .. "/item/public_token/exchange", {
  method = "POST",
  headers = { ["PLAID-SECRET"] = "{{secret:client_secret}}" },
  body = '{"public_token":"' .. token .. '"}',
})
```

**Tests.**
- `features/extensions/tests/node.rs`: over vox behind a gate, the app holds no secret, a viewer is refused, insert-only, undeclared hosts, the manifest's rules.
- `nix/tests/extensions.nix`: a wrong hash, key or a changed archive fails the build.

## Which runtime do I pick

| You need | Pick | Why not the others |
|---|---|---|
| A first-party, user-facing feature | Linked (`linked::Extension`, plus a process copy via `[source]`) | It runs natively in every app with no boundary and can still be swapped for a dev copy (#272) |
| A setting, theme, keymap or action preset, no logic | `kind = "data"` | No code means it runs everywhere |
| A person's macro, conditional action, overlay or tool window | Luau | Interpreted, desktop and iOS; actions and hooks in the browser (#812), not overlays |
| Light logic with no Rust toolchain | `kind = "typescript"` | Interpreted, so not for heavy work; runs where the app gives a runner |
| Third-party code that needs real compute, sandboxed | WASI component | It runs on desktop, iOS (Pulley) and the browser (JSPI); no network of its own |
| A native GPU renderer (Bevy, wgpu, another engine) in a pane | A process view (`[[views]] kind = "process"`) | Native speed and its own crash domain; Linux and macOS (Windows not built), never phones or browsers |
| A whole npm app, canvas or WebGL, or JIT speed on iOS | A web view (`kind = "web"`) | Full DOM and JIT, but untrusted, so it reaches the app only through the bridge |
| Another language, a crash-prone tool, its own lifetime | Process | Desktop only, never phones or browsers |
| A secret the devices must never hold (OAuth client secret) | A node action (`runs = "node"`) | The bar is high: everything else runs in the app |
| Realtime, per-sample or per-frame work | Rust inside a linked extension | Nothing realtime crosses a boundary |
