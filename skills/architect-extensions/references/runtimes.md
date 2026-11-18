# Runtimes: every way an extension's code runs

Which runner runs an extension's code, what selects it in the manifest, where it runs today, what it cannot do, a working example and the test that proves it.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` ("The layers", "WASI components", "On the web", "Linked extensions in the browser", "Processes over vox", "Responsive by design: views and workers", "One trait, three ways to run it", "What each runner costs", "Server-side parts on the node", "On each platform"), `docs/content/architecture/pages.md`, `docs/content/architecture/embedded-views.md` ("Web views"), `docs/content/getting-started/extension.md`, `docs/content/guide/extending.md`, `docs/content/guide/deploy.md` ("Extensions through Nix"), `docs/content/guide/showcase.md` (rows R1–R7, H16).

## Owner decisions

- **Powerful by design, gated by consent.** An extension may do a lot: documents, the network, secrets, the device, MIDI. Every runner's calls go through the same host service and the same check: what the manifest declares under `[capabilities]`, narrowed by what the person granted (`features/extensions/src/hostcall.rs` `check`). The Luau, WASI, page, web view and linked runners all reach that check. Nothing gets ambient access.
- **One implementation per service.** A service is one `#[architect::rpc]` trait with one server. Callers hold the generated `<T>Client`. The edge picks the transport:
  - `architect::serve_local` on desktop and iOS (`crates/architect/src/local.rs` `serve_local`);
  - vox over the org lane in the signed-in browser;
  - wasm `serve_local` in the web demo.
  
  Don't write a second copy for the web or the node. Examples: `features/extensions/src/docs_service.rs` (the documents service, `DocumentsClient`), `apps/web/src/app/index_sync.rs` (the vault query engine in the browser) and `features/extensions/src/node.rs` `NodeActions` (one server, `NodeHost`; every client holds `NodeActionsClient`).
- **WASI is on in iOS release builds.** `cargo xtask dist ios` builds with `wasi`, and Wasmtime's Pulley interpreter runs the components (no JIT). `--no-wasi` is the fallback if App Review objects.
- **HealthKit stays first-party.** Only compiled-in extensions may read or write health data. An installed or development copy is refused (`features/extensions/src/device.rs` `HEALTH_LINKED_ONLY`).
- **The production node gets its extensions through Nix.** Each is pinned by version, hash and signature and checked at system build time. Nothing is downloaded at run time (`nix/node/extension.nix`, `nix/node/module.nix`).

## At a glance

`kind` is the manifest's top-level key (`features/extensions/src/manifest.rs` `Kind`: `data`, `luau`, `typescript`, `wasm`, `process`). Pages, web views and process views are view-level keys, so any kind can have them.

| Runtime | Selected by | Desktop | Web (`/app/`) | iOS | Node |
|---|---|---|---|---|---|
| Linked (Rust) | code: `Extensions::with_linked` | yes | yes (demo tested) | yes | no |
| Luau | `kind = "luau"`, `script = "*.luau"` | yes | Web Workers on luau-web, in the web app's host (#812) | yes | node actions only |
| Pages (TSX/JS, Luau, A2UI) | `[[views]] page = …` | QuickJS | Web Workers, in the web app's host (TypeScript extensions' pages) | QuickJS | no |
| TypeScript actions | `kind = "typescript"`, `script = "*.ts"` | QuickJS | runner exists, no host yet (#723, #736) | QuickJS (not shown) | no |
| WASI component | `kind = "wasm"`, `[wasm] component` | Wasmtime + Cranelift | jco + Web Workers, no host yet (#495) | Wasmtime + Pulley | not yet |
| Process | `kind = "process"`, `[entry] path` | yes | never | never | no |
| Process view (GPU frames) | `[[views]] kind = "process"`, `program` | Linux (dma-buf) and macOS (`IOSurface`), shared memory as the fallback, in every build of the window; Windows says why | never: the pane says why | never: the pane says why | no |
| Web view app | `[[views]] kind = "web"`, `src` | wry child (`--features webview`) | sandboxed iframe, mounted once #495 lands | WKWebView (`--features webview`) | no |
| Node action | `[[actions]] runs = "node"` | forwards to the node | not wired yet (#495) | forwards to the node | runs it |

The desktop and iOS share one host (`apps/ui/src/extensions.rs`). It adds `LuauRunner`, `TsRunner`, `ProcessRunner` and, with the `wasi` feature, `WasiRunner`. The web app's host is `apps/web/src/app/extensions.rs` `use_web_extensions`. On the web, `Host::new` runs only data extensions and linked extensions (`features/extensions/src/host.rs` `default_runners`); the web app adds `WebTsRunner` and `WebLuauRunner` (`features/pages/src/web/runner.rs`, `features/pages/src/web/luau.rs`).

A `kind = "data"` extension has no code: settings, presets, themes, keymaps and frame views. It runs everywhere. It is also the usual kind for a web view app, whose code runs only inside its view.

## 1. Linked (Rust, compiled in)

**What.** A crate implements `features/extensions/src/linked.rs` `Extension`. `manifest` returns its `extension.toml` text, and `build` gives a `Builder` its actions (`action`, `action_in`), its Dioxus views (`view`), its contributions and its root context. The app registers it with `features/extensions/src/extensions.rs` `Extensions::with_linked` as a built-in (`Origin::Builtin`). It runs in the app's own process, and its views are components in the app's document.

**The rule.** Every new user-facing feature is a linked extension in its own crate, with an `extension.toml` and a `[source]` (#272). It may use only what a third party could. It reaches the app through the host service (`linked::Handle::host`), never through Rust handles to app internals.

**Manifest.** The linked copy is chosen by code, not by a key. The same crate also builds a process, so its manifest says `kind = "process"`, `[entry] path` (its binary) and `[source]` (where `EXT_CLONE` finds the crate). An installed or development copy of the same id overrides the built-in live.

**Where.** Desktop and iOS (`apps/ui`). Web: `architect_web::run` (`apps/web/src/browser.rs` `run`). The web app's linked extensions are tested in `/app/?demo`; the signed-in `/app/` runs the same host but isn't tested yet.

**In a browser.**
- Depend on `architect-extensions` with `linked-core` (the trait and `Builder`, no host). Add `guest` only off wasm, by target.
- Action futures are `MaybeSend`, which is `Send` natively and nothing on wasm (`features/extensions/src/linked.rs` `MaybeSend`, `HostFuture`).
- There is no Luau, no host clock or event threads, no install folder, and secrets stay in memory.

**Can't.**
- Its views are not metered, so it keeps to the frame budget itself.
- It can't be built as a WASI component yet. A `linked::Extension` as a component is #272.
- Studio's linked extensions need a `linked-core` build from Studio before they run on the web (#495).

**Example** (`extensions/tasks`, trimmed):

```toml
# extensions/tasks/Cargo.toml
[dependencies]
architect-extensions = { workspace = true, features = ["linked-core"] }

[target.'cfg(not(target_family = "wasm"))'.dependencies]
architect-extensions = { workspace = true, features = ["guest"] }
```

```rust
// extensions/tasks/src/lib.rs
impl Extension for Tasks {
    fn manifest(&self) -> &'static str { include_str!("../extension.toml") }
    fn build(&self, ext: &mut Builder) {
        ext.view("list", ListView)
            .action("TASKS_LIST", list)
            .action("TASKS_ADD", add);
    }
}
// extensions/tasks/src/main.rs: the same value as a process
fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_tasks::Tasks).serve()
}
```

**Tests.**
- `features/extensions/tests/linked.rs`: linked, turned off and on, overridden by a development copy.
- `apps/sandbox/board/tests/both_ways.rs`: the same value linked and as a process add the same things.
- `apps/web/test/app-linked-extensions.test.mjs`: in the browser.
- `cargo xtask builtins` builds every built-in crate both ways.

## 2. Luau scripts

**What.**
- Luau through mlua (`features/scripting/src/luau.rs` `Runtime`), run by `features/extensions/src/host.rs` `LuauRunner`. A run is one undo step when the runner has transactions.
- Scripts reach the host service as `host.*`: actions, settings, data, documents, the interface, `host.fetch` and secrets (`features/extensions/src/luau_host.rs`).
- Luau also drives `[[overlays]] script` (Canvas and ImGui, live), `[[workers]] script`, Luau pages and node actions.

**Manifest.** `kind = "luau"`. Each action has `script = "greet.luau"`. A `.ts` script in a `luau` extension is refused (`ManifestError::ScriptKind`).

**Where.**
- Desktop: yes.
- iOS: yes. Templater-lite ran on the Simulator.
- Web: yes, in the web app's host (#812). mlua's Luau does not build for `wasm32-unknown-unknown`, so the browser runs luau-web's emscripten build (`apps/web/assets/luau-web`, fetched only when a Luau extension runs; `scripts/vendor-luau-web.sh`) in a Web Worker per run: `features/pages/src/web/luau.rs` `WebLuauRunner`, its globals `features/pages/src/web/luau_runtime.luau`. Every global is a message the page answers with the one implementation the desk uses: `host.*` with `features/extensions/src/luau_calls.rs` `host_request` and the host service (`WebHost::through`), `ext.*` with `ext_call`, `actions.*` and `state.query` with `features/scripting/src/bindings.rs`. A call suspends the script (Asyncify), so no `Atomics.wait` and no cross-origin isolation. Limits are a TypeScript action's: 5 s to load, 500 ms of work between host calls, a worker's 300 s; past one the Web Worker ends. No step or memory cap. A worker runs when its owner first waits on it (`join`, `result`, `status`…), inside the run's Web Worker, and serves no messages. Without the bundle's luau-web the host has no runner and says why (`features/extensions/src/host.rs` `why_no_runner`).
- Node: only as node actions (section 8).

**Can't.**
- A frame past `Limits::frame` (500 ms) is stopped, and `Limits::steps` stops runaway loops. Heavy work goes to a `[[workers]]` script.
- It has no network beyond `host.fetch` to `capabilities.hosts`.

**Example** (`examples/extension/registry/hello`):

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

A worker: `examples/showcase/timers` (`[[workers]] id = "primes"`, `script = "workers/primes.luau"`, `worker.progress(i, n, note)`).

**Tests.**
- `features/extensions/tests/metronome.rs`: install from a registry, reload on save.
- `features/extensions/tests/showcase.rs` and `features/extensions/tests/host_service.rs`.
- `features/extensions/tests/workers.rs`.

## 3. Pages: TSX/JS (QuickJS; a Web Worker in browsers), Luau, A2UI

**What.** A view drawn from a file, rendered live on save. Each page gets a thread of its own. Its element tree is diffed into `wire::Edit`s and applied to a Blitz sub-document, the same path a process view takes (`features/pages/src/runner.rs` `Pages`).
- TSX/JS runs on QuickJS (`features/pages/src/js/mod.rs` `JsEngine`) after oxc transpiles it.
- Luau pages run on `features/pages/src/luau.rs` `LuauEngine`.
- A2UI JSON runs through the catalog (`features/pages/src/a2ui.rs`).

The page API is defined once in Rust (`features/pages/src/api.rs` `PageHost`, `PageView`). `architect.d.ts` and `architect.d.luau` are generated from it.

**Manifest.** `[[views]] page = "pages/x.tsx"` (also `.ts`, `.jsx`, `.js`, `.luau`, `.a2ui.json`) on a view of any kind. An app that runs pages (`architect-pages`) draws the page in place of the compiled view.

**Where.**
- Desktop and iOS: QuickJS. Templater-lite's page ran on the Simulator.
- Browser: `features/pages/src/web/mod.rs` `WebPages` runs TSX/JS in a Web Worker on the browser's engine, with the same prelude and types, and draws through `features/extensions/src/web.rs` `Pane`. In the web app an installed extension's page view is drawn by `apps/web/src/app/installs.rs` `view_pages` (its host calls through `WebHost::through`); test: `apps/web/test/app-showcase-install.test.mjs`. The web lab runs them too (`just sandbox-web`).
- Luau and A2UI pages don't run in a browser yet (#187).

**Can't.**
- Use Node APIs: no `require`, `process` or `fetch`. Imports come only from the page's own folder plus `"architect"`, `"architect/dom"` and `"architect/worker"`.
- Exceed `PageLimits`: 64 MB, and 500 ms per call before "hand heavy work to a worker".
- Draw dense or animated views. Those stay compiled.
- In a browser: dynamic `import()`, documents, ports, secrets and `services.call`.

**Example** (`examples/showcase/templater-lite`, a `luau` extension with a TSX tab):

```toml
[[views]]
id = "templates"
title = "Templates"
page = "pages/templates.tsx"
```

```tsx
import { h, useData, actions, notify } from "architect";
export default function Templates() {
  const list = useData<{ path: string; title: string }[]>("SHOWCASE_TEMPLATER_LIST", {});
  const make = async (path: string) => {
    const made = await actions.run("SHOWCASE_TEMPLATER_NEW", { template: path });
    if (made) notify(`Made ${made}`);
  };
  return <div>{(list.value ?? []).map((t) => <button key={t.path} onClick={() => make(t.path)}>{t.title}</button>)}</div>;
}
```

**Tests.**
- `features/pages/tests/showcase_pages.rs` and `features/pages/tests/js.rs`.
- `apps/sandbox/board/tests/page.rs`: the Board's tab as `board.tsx`.
- `apps/sandbox/web/test/pages.test.mjs` (`just sandbox-web-test`): the browser runner.

## 4. TypeScript actions

**What.** A `typescript` extension writes its actions and hooks as `.ts` modules beside its pages, on the pages' runtime. Light extensions need no Rust and no WASM toolchain. The runner is `features/pages/src/js/action.rs` `TsRunner`. Each run gets a fresh QuickJS runtime on its own thread and has the whole host service, typed (`defineAction<Args>`).

**Manifest.** `kind = "typescript"`, and each `[[actions]] script` or `[[hooks]] script` is a `.ts`, `.mts`, `.tsx`, `.js`, `.mjs` or `.jsx` module (`features/extensions/src/manifest.rs` `is_ts_script`).

**Where.**
- Desktop: QuickJS. The Recipe scaler is shown.
- iOS: the same runner on the shared `apps/ui` host, not shown.
- Browser: `features/pages/src/web/actions.rs` `WebPages::register_actions` runs each action in its own Web Worker (no JSPI, so Safari works); the web lab runs it. In the extensions host it is `features/pages/src/web/runner.rs` `WebTsRunner` (feature `web-host`): the host's runner for the kind, its actions and hooks in Web Workers, every host call the app's host service for that extension (`features/extensions/src/web.rs` `WebHost::through`), so no second host. Prompts and pickers aren't in the browser host yet.

**Can't.**
- Import packages; bundle one into a file. No `fetch`, `fs`, `process` or Node globals.
- Exceed `ActionLimits`: 64 MB, 1 s per unit of work, 20 s CPU, 10 min wall time, 8 concurrent runs.
- Do heavy computation or touch the audio path. Write that in Rust and call it as an action.

**Example** (`examples/showcase/recipe-scaler`):

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

```ts
import { actions, defineAction } from "architect";
type Args = { folder?: string } | null;
export default defineAction<Args>(async (args) =>
  actions.run("VAULT_LIST", { folder: args?.folder ?? "Recipes" }));
```

**Tooling.** `@fasttrackstudio/architect-cli` (`features/extensions/web/cli`): `architect-ext new --typescript`, `types`, `check` and `pack`.

**Tests.**
- `features/pages/tests/ts_actions.rs` and `features/pages/tests/showcase_recipes.rs`.
- `apps/sandbox/web/test/actions.test.mjs`: the browser.

## 5. WASI components (Wasmtime; Pulley on iOS)

**What.** A WebAssembly component against the `architect:extension` world (`features/extensions/wit/extension.wit`). It imports `host` and `worker` and exports `guest` and `work`.
- Natively, `features/extensions/src/wasi.rs` `WasiRunner` runs it: one instance per thread (an actor), epoch time limits and a memory limit. Compiled code is cached as `.cwasm`.
- Views are the remote-view wire's edits, so the panes that show a process view show a component's unchanged.
- The Rust guest crate is `architect-extension-wasi` (`features/extensions/wasi-guest`).

**Manifest.** `kind = "wasm"` and `[wasm] component = "x.wasm"` (`[entry] path` still works). Workers are `[[workers]] entry = "<name>"` (an entry of the same component) or a separate `*.wasm` in the package.

**Where.**
- Desktop: Cranelift (`wasi` feature), on in `cargo xtask dist desktop` (#707). Android compiles natively too.
- iOS: Pulley interpreted, on in `cargo xtask dist ios` (#707, #716). A call costs about 3× native (~37 µs p50 on the Simulator).
- Browser: `features/extensions/src/web.rs` `WebComponent`. The same file is transpiled by jco (`features/extensions/web/transpile.mjs`) and runs in Web Workers inside an opaque-origin iframe. It needs JSPI, so Safari doesn't run it yet. The web lab runs it (`just sandbox-web`), and so does the npm package `@fasttrackstudio/architect` in any web app. The web app doesn't host installed components yet (#495).
- Node: WASI node actions are a follow-up (no issue named).

**Can't.**
- Reach the network, files or environment. An import of `wasi:sockets` or `wasi:http` is refused before linking.
- Exceed 64 MiB and 5 s per call (`wasi::Limits`).
- Use threads; workers take the heavy work.
- Run async actions before WASI 0.3.
- Run its own actions through `run-action`.

**Example** (`apps/sandbox/wasi-counter`):

```toml
id = "lab.wasi-counter"
kind = "wasm"
[wasm]
component = "architect_sandbox_wasi_counter.wasm"
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

## 6. Process extensions (over vox)

**What.** A program the host starts and supervises: restarted with backoff, saying why it died. It speaks vox over a local socket, as a remote client does (`features/extensions/src/process.rs` `ProcessRunner`, protocol `features/extensions/src/proto.rs` `ProcessExtension`).
- Its `main` is `Guest::new()…serve()`, or `Guest::extension(&MyLinked).serve()` for a linked extension's process copy (`features/extensions/src/guest.rs` `Guest`).
- Each view is a `VirtualDom` on its own thread in the process. Its edits are drawn by `features/extensions/src/remote.rs` `RemoteView`.
- Use it for isolation, other languages and separate lifetimes, not for build speed.
- A linked extension's view code runs unchanged in its process copy when it uses the both-ways pieces: `linked::use_pane_state` (its pane's id and state; `architect_workspace::use_pane` panics in a process) and `linked::PaintPane` (feature `paint-pane`) for a `PaintView` among its own elements (`docs/content/architecture/embedded-views.md`, "One `PaintView`, unchanged"). `guest::in_process` says which side it is on.
- A program named by `[entry] path` is a file in the package: a development copy's build puts it there (Keyflow's `app-from` recipe does). Files `Extension::files` hands a linked copy (toolbar `image`s) must also be in the package for a process copy's host to read them.

**Manifest.** `kind = "process"` and `[entry] path = "<program>"` (plus `args`). Its hello must cover every declared action and view.

**Where.** Desktop only (`Kind::Process` is "desk only"). Never on iOS, Android or the web: a phone may not start programs and a browser has none. There's no "desk only" state in the UI for it yet (#578).

**Can't.**
- Stay inside the frame budget for free. A view has 33 ms (socket round trip included), and opening one costs about 25 ms to its first frame.
- Show more than `MAX_VIEWS` (256) views at once.
- Be called per sample or per frame. Each call costs tens of µs.

**Its frames are paced to the display (#840).** A batch, and every change its document makes, asks the window for a frame through `features/workspace/src/pacing.rs` `Pacer` (`remote::HostShell::pacer`): right after a frame that waited for its swapchain image, the requests until the next refresh share one frame started just before the next image comes free, instead of each starting a frame that waits most of a refresh with the window's thread blocked. `ARCHITECT_FRAME_PACING=0` turns it off.

**What moves every frame is the host's.** A paint view (`linked::Builder::paint`) that returns layers (`PaintView::layers`) sends them once (`architect-scene`, `features/scene/src/ops.rs`), and the host draws them every frame without it:
- Motion on a clock: a layer's `X`, `Y`, `ScaleX`, `ScaleY`, `Opacity` or `Visible` from `Clock::Wall` or `Clock::Playhead` (`features/scene/src/anim.rs` `Animation`, `Linear`, `Steps`). A playhead line, a scrolling arrangement, a lyric line. Don't return `animating` true for these: the view would be asked every frame.
- Hit regions and drags (`features/scene/src/region.rs` `Region`, `Drag`): hover, cursor, highlight and a fader or drag box following the pointer, all on the host; the view hears `RegionEvent`s by id (`PaintView::region`), a drag's value at most once a frame. A press on a region still gives the view the keyboard focus (`Focus(true)`), linked in or in a process.
- Named values a layer is bound to (`PaintView::values`, `view.` names); the engine's lock-free `engine.` values with `capabilities.ui = ["engine"]` (`features/scene/src/engine.rs`). An engine in a process of its own shares its table in shared memory (`features/scene/src/engine/shared.rs` `create`, `open`), found through the service the view's extension `[uses]` or `[requires]` (`engine::read_from`).
- A malformed animation, region or value fails the view, linked in or not. The example: `apps/sandbox/sketch` `Timeline`.

**Example** (`examples/extension/counter`):

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
- `features/extensions/tests/remote_view.rs`: clicks through a real child process, then kills it.
- `features/extensions/tests/process_views.rs`: the 256-view cap.
- `examples/extension/counter/tests/survives_kill.rs`.

## 7. Web view apps (the `web` view kind)

**What.** A full HTML/TS/npm app (DOM, canvas, WebGL, any library) shown in a pane. Its code is untrusted (`Capability::Untrusted`).
- It reaches the app only through `window.architect` (`features/extensions/src/webview/runtime.js`). Every call is a `HostRequest` checked by `hostcall::check` (`features/extensions/src/webview.rs` `Bridge`).
- Its files are served only from its package, under `webview::csp` with `connect-src 'none'`.
- On iOS it is the one way installed code runs JIT-compiled.

**Manifest.** `[[views]] kind = "web"` and `src = "<html in the package>"`. A web view must have `src`, and only a web view may (`Manifest::check`). The extension itself is usually `kind = "data"`.

**Where.**
- Desktop: a wry child view (`features/webview/src/desk.rs` `use_desk_web_views`) in `architect-desktop --features webview`. It runs on Linux under X11 or XWayland (`ARCHITECT_WEBVIEW_X11=1` under Wayland). It isn't in the release build yet, and macOS and Windows are untested.
- Browser: a sandboxed `<iframe>` with no same-origin (`features/extensions/web/webview-host.js` `mountWebView`). The web app mounts it once it hosts extensions (#495).
- iOS: the same desk host. wry's `build_as_child` makes a WKWebView child of the pane's `UIView` (`architect-mobile --features webview`, `apps/ui/src/extensions.rs`). It runs on the Simulator but isn't on TestFlight yet, and it covers the phone's floating bars.

**Can't.**
- Use its own `fetch`, XHR or WebSocket. Its network is the host's `fetch` to `capabilities.hosts`.
- Navigate away, open windows or use WebRTC.
- Exceed `webview::Budget`: 500 messages and 16 MiB a second.
- Sit under anything the app draws. It is hidden while covered.
- Hear events while its tab is parked.

**Example** (`examples/extension/webview-board`, built by `just webview-example`):

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
- The tests in `features/webview/src/desk.rs`.

## 7b. Process views (the `process` view kind, #838)

**What.** A view drawn by a program in the package, on its own GPU device, in its own process: Bevy, a game engine, a renderer in another language. The app's process-view pane (`features/process-view/src/host.rs` `ProcessViewPane`) shows its frames: dma-buf textures read in place, or shared memory uploaded. It forwards the pane's input, size, focus and theme. It is not a process extension's remote view, which is a Dioxus document the app draws. Any kind of extension may have one, and it is shown the same whichever copy is loaded: built in, installed or in development.

**Manifest.** `[[views]] kind = "process"`, `program = "<file in the package>"`, `args = [...]`. An argument may name one of its own settings, `{{setting:<key>}}` (the `{{secret:…}}` placeholder's form): its value is put in at start and a change restarts the program; a setting alone that comes out empty is left out, so write `--show={{setting:x.show}}` for a value that may be empty. A secret never goes in an argument. The program must also be named in `capabilities.programs`, and `Manifest::check` refuses a process view whose program is missing, unasked or outside the package. Only a process view may name a `program`.

**Consent.** `capabilities.programs` is a capability like any other: asked in words ("Run bin/room, a program in its package, on this computer"), granted by a person for an installed copy, taken back at once, and never a share-link guest's (`share_needs::guest_capabilities`). A built-in or development copy (`architect dev --extension …`) runs with what it asks and is never prompted. An installed copy is asked at install; if the grant is later missing (taken back, or an update asks for a program), its pane asks again with an **Allow** button that grants that one program on this device (`process_view::needs_grant`, `grant_program`). The pane re-checks every 500 ms (`features/extensions/src/process_view.rs` `launch`); a session stopped for a revoked grant or an unload is retired, and its Restart starts nothing.

**Where.**
- Linux and macOS desktop, in every build of the window: `architect-desktop`'s `gui` feature turns on `architect-app`'s `process-views` (`apps/ui/src/extensions.rs` calls `architect_process_view::extension::use_process_views`), so the release, `dist/standard` and `architect dev` from a checkout all have it. Frames are dma-buf textures on Linux and `IOSurface`s on macOS (`features/process-view/src/surface.rs`), shared memory where neither imports. Pop-out windows draw it too (the window bridge carries `ProcessViews`). `APP_SHOT` does not show its frames (a CPU repaint); capture it with the compositor. `architect-desktop --features process-views` adds only the lab's ⌘6 demo screenset.
- Windows: the extension loads; its pane says why (`process_view::not_here`). Designed (a DXGI shared handle), not built.
- Browser and iOS: never; the pane says why.

**Lifecycle.** One program per pane (`<extension>/<view>#<pane>`). It is started when the pane opens and keeps running when the pane moves to another group or a pop-out window. It is stopped when its pane leaves the workspace (a close, checked every 250 ms; a move keeps the pane, so never a restart), and at the same check on an unload or a revoked grant. A pane with no workspace to ask stops 2 s after it went. It is restarted like a process extension's program (three tries with backoff, saying why it died). A development copy rebuilt (`EXT_DEV`) starts it again in place.

**The program.** It serves a `PaintView` with `architect_process_view::guest::serve`, or speaks the wire itself (`features/process-view/src/wire.rs`; its socket is in `ARCHITECT_VIEW_FD`). It runs in the package's folder.

**Example** (`examples/extension/process-view`, built by `just process-view-example`):

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

## 8. Node actions (server-side, on the org's node)

**What.** The one piece of extension code that runs on the server. It holds a secret the person's devices must never have, such as a token exchange with a client secret (#583). Everything else runs in the app.
- The node loads each installed extension's node part (`Manifest::node_part`: its node actions, their secrets, and only `hosts` and `effect` of its capabilities).
- It serves them with `NodeHost` (feature `node`) on the org lane, behind the org's permission gate.
- Apps register each node action as a forwarder (`Host::with_node`, a `node::NodeLane`), so ⌘K, the CLI, MCP and scripts run it like any action.

**Manifest.** `[[actions]] runs = "node"` with `secrets = ["…"]` naming at least one `[[secrets]]` entry. The action must be a `luau` extension's action with a `script`, and never a preset, a toggle or a window opener (`Manifest::check`).

**Where.**
- Node: it runs here. Production gets it through Nix: `lib.architectExtension` (pinned id, version, SHA-256 and publisher keys, checked by `archext install`, `features/extensions/archive/src/bin/archext.rs`) and `services.architect-node.extensions`. Secrets come from `extensionSecrets`, as sops-nix files, or an org admin's `set_secret`. They are insert-only.
- Desktop and iOS: forwarded as the account (`apps/ui/src/accounts.rs` `OrgNode`).
- Web: the same `NodeActionsClient`, but the web host isn't wired yet (#495).

**Can't.**
- Have views, hooks, workers, a background part, settings, documents, the interface or ports.
- Read its secrets. The host puts them into `{{secret:id}}` headers on requests to their hosts.
- Keep data on the node, and WASI node actions aren't supported. Both are follow-ups with no issue named.

**Example** (`examples/showcase/bank-link`):

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
- `apps/desktop/tests/node.rs`.
- `nix/tests/extensions.nix`: a wrong hash, key or a changed archive fails the build.

## Which runtime do I pick

| You need | Pick | Why not the others |
|---|---|---|
| A first-party, user-facing feature | Linked (`linked::Extension`, plus a process copy via `[source]`) | It runs natively in every app with no boundary and can still be swapped for a dev copy (#272) |
| A setting, theme, keymap or action preset, no logic | `kind = "data"` | No code means it runs everywhere |
| A person's macro, conditional action, overlay or tool window | Luau | Interpreted, desktop and iOS; actions and hooks in the browser (#812), not overlays or pages |
| A panel, list, form or dashboard view that can be edited live | A page (`page = "*.tsx"`) | Desktop and iOS on QuickJS, the browser on a Web Worker; not for dense or animated views |
| Light logic with no Rust toolchain | `kind = "typescript"` | The same runtime as pages; interpreted, so not for heavy work |
| Third-party code that needs real compute, sandboxed | WASI component | It runs on desktop, iOS (Pulley) and the browser (JSPI); no network of its own |
| A native GPU renderer (Bevy, wgpu, another engine) in a pane | A process view (`[[views]] kind = "process"`) | Native speed and its own crash domain; Linux and macOS (Windows not built), never phones or browsers |
| A whole npm app, canvas or WebGL, or JIT speed on iOS | A web view (`kind = "web"`) | Full DOM and JIT, but untrusted, so it reaches the app only through the bridge |
| Another language, a crash-prone tool, its own lifetime | Process | Desktop only, never phones or browsers |
| A secret the devices must never hold (OAuth client secret) | A node action (`runs = "node"`) | The bar is high: everything else runs in the app |
| Realtime, per-sample or per-frame work | Rust inside a linked extension | Nothing realtime crosses a boundary |
