# Extension pitfalls

Traps that have cost real time building extensions, each with its cause, what you see, the fix and where it is written down. Check this list before writing a manifest, a view, a service or a new crate.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` ("Risks", "Hot reload", "Which operating systems: `platforms`"), `docs/content/architecture/embedded-views.md` ("Web views"), `docs/content/guide/contract.md`, `docs/content/guide/testing.md`, `skills/architect/references/blitz-quirks.md`, `skills/architect/references/xtask-tooling.md`.

## Contracts and services

### 1. At most four parameters on an RPC method

- **Cause:** vox's service macro sends a method's arguments as one tuple (`(A, B, C, D,)`), and facet implements `Facet` for tuples of at most four elements unless its `tuples-12` feature is on, which the workspace does not turn on. `#[architect::rpc]` (`crates/architect-rpc-derive/src/lib.rs`) passes the trait to `#[vox::service]`, so it does not check this itself.
- **Symptom:** a fifth parameter fails to compile with a trait-bound error about `Facet` on a 5-tuple, far from your trait.
- **Fix:** keep each method to four parameters besides `&self`; put the rest in one `#[architect::wire]` struct and pass that.
- **Source:** `docs/content/guide/contract.md` ("Keep a method to four parameters or fewer; put more in one wire struct"); `skills/architect/SKILL.md` calls it a convention. The tuple cause is in vox's `vox-macros-core` and facet's `tuples-12` feature, not in architect's docs.

### 2. Futures that are not `Send` in a browser

- **Cause:** on wasm, vox clients hold `Rc`s and the page has one thread, so a future that awaits a host call is not `Send`. The host's `Host` must stay `Send + Sync`.
- **Symptom:** the web build (`wasm32-unknown-unknown`) fails with "cannot be sent between threads safely", or a hook's work silently never runs in the browser.
- **Fix:**
  - Bound futures with `MaybeSend`, not `Send`: `features/extensions/src/linked.rs` `MaybeSend` is `Send` natively and nothing on wasm; `ActionFuture` and `HostFuture` follow the same split. The framework's own is `crates/architect/src/connect/mod.rs` `MaybeSend`.
  - Keep a wasm-only client in `send_wrapper::SendWrapper` behind a `cfg(target_family = "wasm")` type alias, as `features/extensions/src/hostapi.rs` (`HeldQuery`) and `features/extensions/src/node.rs` (`NodeLane` for `SendWrapper<NodeActionsClient>`) do. Touched from another thread it panics rather than racing.
  - Give a browser host a spawner: `Host::with_spawn` with `wasm_bindgen_futures::spawn_local`. Without one, `default_spawn` in `features/extensions/src/host.rs` drops the work and logs `ext.spawn: no spawner in this browser host`. `apps/web/src/app/extensions.rs` is the reference.
- **Source:** commits "a browser's host keeps the vault query client in a SendWrapper" and "the host builds for a browser again" (#495); the code comments above. Not in `docs/content`.

### 3. Action ids must start with the extension's prefix

- **Cause:** `Manifest::check` (`features/extensions/src/manifest.rs`) requires every action id to start with `Manifest::action_prefix`: the id upper-cased, each run of other characters one `_` (`fts.metronome` → `FTS_METRONOME_`).
- **Symptom:** the extension does not load: `ManifestError::ActionNamespace { action, expected }`. `EXT_CHECK` names it.
- **Fix:** prefix every `[[actions]] id`. A new prefix means a new extension id. An app's own ids (`APP_…`) are owned only through `reserved` plus `Host::allow_reserved`, and only by a built-in or its development copy (`LoadError::Reserved` for an installed copy).
- **Source:** `docs/content/architecture/extensions.md` ("Reserved ids", and "an action: one in its own namespace (`action_prefix`)").

### 4. Cross-extension reads inside `refresh`

- **Cause:** two extensions that read each other's services from their `refresh` (Calendar → Day Plans → Calendar) recurse.
- **Symptom:** a stack overflow at load or on the first refresh.
- **Fix:** read the other extension's service outside `refresh`, as Calendar's `refresh_plans` does.
- **Source:** `docs/content/extensions/calendar.md` (its views); `docs/content/extensions/day-plans.md`.

## Views

### 5. A view reading a cache needs a revision prop

- **Cause:** Dioxus re-renders a component when its props or the signals it reads change. A component that reads a process-wide cache (not a signal) with the same props is never drawn again.
- **Symptom:** the data changes (logs, `EXT_*` reads show it) but the view keeps its first picture.
- **Fix:** pass the cache's revision as a prop, and move the revision whenever the cache changes.
- **Source:** `docs/content/extensions/calendar.md` ("The views take the cache's revision as a prop, so they draw again when it moves").

### 6. refresh → bump → effect loops

- **Cause:** an effect keyed on a revision calls an action that refreshes the cache and bumps the revision, which fires the effect again.
- **Symptom:** the whole app freezes at full CPU with no log line.
- **Fix:** `refresh` moves the revision only on a real change; views read the cache and do not call refreshing actions from an effect.
- **Source:** `docs/content/extensions/day-plans.md` ("`refresh` moves the revision only when the vault changed, so a view that reads again when the revision moves never loops"). The freeze symptom is not written in `docs/content`.

### 7. Blitz draws `<select>` (and date inputs) blank

- **Cause:** linked and process views draw through Blitz, not a browser; `select` and `input type="date"` have no widget there.
- **Symptom:** an empty box or a dot where the picker should be, on desktop and iOS only (the web app is a browser).
- **Fix:** chips or a popover of buttons (`features/capture/src/ui/process.rs` `TaskShape`).
- **Source:** `skills/architect/references/blitz-quirks.md` ("Layout and text"). Read the whole table: `placeholder` draws nothing, `ch` is unsupported, `title` shows no tooltip, `translate` moves in device pixels, `onresize` never fires, `env(safe-area-inset-*)` is missing, `overflow: hidden` clips drawing but not hits.

### 8. No `style { … }` node in a linked view

- **Cause:** a `style` element mounted in the extensions banner beside the workspace broke Blitz's layout tree.
- **Symptom:** a panic at first layout: `layout_parent … does not point at its layout container`.
- **Fix:** load sheets once at app level with `document::Style`, and style the view with inline styles over the theme's variables (`var(--border)`, `var(--card)`). Remember `document::Style` is inserted once and never updated, so a sheet built from theme values goes stale (`docs/content/architecture/theming.md`).
- **Source:** the comment in `features/extensions/src/page.rs` `MissingExtensions`. Not in `docs/content`.

### 9. Raw HTML in a process view

- **Cause:** a remote view's edits are checked when applied (`features/extensions/src/remote.rs`): `dangerous_inner_html` is refused on any element but `<style>`.
- **Symptom:** the whole view fails: "a view may not set raw HTML".
- **Fix:** build markup with elements. `style { dangerous_inner_html: CSS }` is allowed and stays inside the view's own sub-document. Pages forbid `dangerous_inner_html` everywhere (`docs/content/architecture/pages.md`).
- **Source:** `features/extensions/src/remote.rs`; commit "a process view draws like a linked one in its pane" (#655).

### 10. Web views: a native child, its own network blocked

From `docs/content/architecture/embedded-views.md` ("Web views"):

- **Nothing draws over it.** A desk web view is a native child window; the host hides it while ⌘K, a menu, a tooltip or a dialog covers any of nine sample points, and shows it two clear frames later. Don't design a view that expects app chrome on top of it.
- **Its own `fetch`, XHR and WebSocket fail** (`connect-src 'none'`). Use the bridge's `fetch` within `capabilities.hosts`.
- **Keys stay in the view** while it has focus: app chords don't fire. An unhandled Escape gives focus back.
- **Linux under Wayland**: WebKitGTK is a child of X11 windows only. Run with `ARCHITECT_WEBVIEW_X11=1` (XWayland) in a build with `webview`; otherwise the pane says it cannot show.
- A view switched away is parked a minute and hears no events; messages past its budget are dropped.

## Building and CI

### 11. A new crate and the workspace-hack

- **Cause:** every member depends on `architect-workspace-hack` under a gate (`cfg(not(any(target_family = "wasm", no_workspace_hack)))`), generated by cargo-hakari (`.config/hakari.toml`, `workspace-hack/Cargo.toml`) plus the fleet union (`cargo xtask fleet-deps`).
- **Symptoms:**
  - PR fast checks fail on the workspace-hack step (`cargo hakari generate --diff`, `cargo hakari manage-deps --dry-run`) after you add a crate or dependency;
  - `cargo hakari manage-deps` writes the new member's hack line ungated, and its wasm build pulls the native union and fails;
  - a crate builds in the workspace but fails in the node or a distribution build (`--cfg no_workspace_hack`) because a dependency feature it needs came only from the hack.
- **Fix:**
  1. `cargo hakari generate`, then `cargo hakari manage-deps`;
  2. `cargo xtask fleet-deps` to move the new line under the gate (with `--repo` checkouts after a dependency change);
  3. name every feature your crate needs in its own `Cargo.toml`, never relying on the hack. `features/extensions/Cargo.toml` turns on `dep:dioxus-html` for `serialize` for this reason;
  4. when you remove a member, delete its gated line by hand.
- **Source:** `skills/architect/references/xtask-tooling.md` ("Fleet feature union", pitfall 10); `docs/content/guide/your-own-architect.md` (`node` builds with `--cfg no_workspace_hack`).

### 12. A feature-gated test that never runs

- **Cause:** main runs `cargo nextest run --workspace` with the unified features; a `[[test]]` with `required-features`, or a test under a `cfg(feature …)` that is off there, is never built.
- **Symptom:** `cargo xtask feature-tests` fails PR fast checks ("add a [[pass]] (or a [[skip]] with its reason) to feature-tests.toml"); before that check existed, such a test sat broken unnoticed (#299).
- **Fix:** add a `[[pass]]` to `feature-tests.toml` (packages, `package/feature` list, optional `env`, `workspace = true` for a member high in the graph) or a `[[skip]]` with `package`, `file` and `reason`. A pass that runs nothing new and a stale skip also fail. See `testing-showcase.md` in this skill.
- **Source:** `xtask/src/feature_tests.rs`; `docs/content/guide/testing.md` ("What CI runs").

### 13. Tests that skip quietly

- **Cause:** `features/extensions/tests/wasi.rs` and `features/extensions/tests/wasi_web.rs` pass with a message when `wasm32-wasip2`, node or jco are missing; browser tests skip without Firefox.
- **Symptom:** a green local run that tested nothing.
- **Fix:** read the output; run WASI tests in `devenv --profile wasi shell`, or set `ARCHITECT_WASI_REQUIRED=1` / `ARCHITECT_WASI_WEB_REQUIRED=1` (CI does), and `ARCHITECT_BROWSER_REQUIRED` for browser suites. Run browser tests with `env -u LD_LIBRARY_PATH`.
- **Source:** the module docs of those test files; `.github/workflows/browser.yml`.

### 14. Extensions that keep state in statics

- **Cause:** a linked extension's statics are shared by every test in one process.
- **Symptom:** tests pass alone and fail together.
- **Fix:** run with `cargo nextest` (one process per test), not `cargo test`.
- **Source:** `docs/content/extensions/calendar.md` (its tests).

## Install and platforms

### 15. A copy for another platform still wins

- **Cause:** an installed or development copy whose `platforms` exclude this OS still counts as the copy of its id.
- **Symptom:** a built-in that could run shows "Can't run here".
- **Fix:** remove the copy, or **Use the built-in**. "Not here" is decided at load and not retried while the copy is the same: a platform change shows after a restart.
- **Source:** `docs/content/architecture/extensions.md` ("Which operating systems: `platforms`").

### 16. Signer and vault rules

- An update signed by a different key than the install is refused (`InstallError::SignerChanged`): keep one key per extension.
- A vault's `.architect/extensions/*.archext` cannot introduce a new publisher; only the org key or the installed copy's signer.
- A share link refuses development copies and unsigned installs.
- `EXT_DEV` is refused unless development installs are on (or the build is a debug build).
- **Source:** `docs/content/architecture/extensions.md` ("Where a vault and a project name them", "A share link's extensions", "A third party's extension in the Architect app"); `install-lifecycle.md` in this skill.

### 17. Platform gaps you will hit

- **Web**: the web app runs linked extensions, installed TypeScript actions and pages, and Luau actions and hooks (luau-web in Web Workers, #812); WASI waits on #495. mlua's Luau never builds for the browser: Luau code there must use only what `features/pages/src/web/luau_runtime.luau` gives (no live overlays or Luau pages), and a `worker.spawn` runs when first waited on (`apps/web/src/app/extensions.rs`).
- **iOS**: no processes; WASM is interpreted by Pulley (lower-tested; a 64 MiB reservation with no guard); installed WASM rests on App Review guideline 2.5.2 (`cargo xtask dist ios --no-wasi` is the fallback build).
- **Tooling**: the component tooling is young (WASI 0.3, async components, needs Wasmtime 46+); old WIT versions are kept forever, so design WIT changes as additions.
- **Source:** `docs/content/architecture/extensions.md` ("On each platform", "Risks"); `docs/content/guide/showcase.md`.

### 18. `document::Style` in a process view (fixed)

- **Was:** a process copy's view unstyled while the linked copy was fine: its `VirtualDom` had no document for `document::Style` to reach.
- **Now:** every view a process serves is drawn under `guest`'s root, which gives it a document whose `document::Style` is a `style` element its host applies, the app's theme store (`architect_theme::try_use_theme`, `use_resolved`) holding the host's theme, and in a paint view's `ViewCtx` the actions it may run (`guest::app_actions`, the host call `actions`). A `style` element inline still works. Test: `apps/sandbox/sketch/tests/paint_parity.rs` `a_process_view_has_the_theme_its_styles_and_its_actions`.
- **Still:** a linked extension's `Extension::files` are not sent by its process copy: put files its toolbars read in its package.
- **Source:** #840, #844.

## Before calling an extension done

1. `architect-desktop extensions check <folder>` (or `EXT_CHECK`) passes: manifest, scripts, load, each view rendered once.
2. Its tests run under `cargo nextest`, and any gated test is registered in `feature-tests.toml`.
3. The web build compiles if the crate is linked into the web app.
4. Screenshots at 390 px, light and dark, on the local stack.
5. The showcase row it moves is updated with its evidence.
