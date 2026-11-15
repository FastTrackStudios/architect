# Testing extensions and the showcase

What to run to prove an extension works on each runtime and platform, how CI finds every test, and how to add evidence to the showcase matrix.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/guide/showcase.md` (the matrix, #710), `docs/content/guide/testing.md` ("What CI runs"), `docs/content/architecture/extensions.md`, `docs/content/guide/cli-and-mcp.md` (the parity rule behind `cargo xtask surfaces`).

## Rules first

- **Evidence is a test plus screenshots.** A showcase cell turns ✅ only with an example, an automated test, and screenshots at 390 px and 1280 px, light and dark, on that platform.
- **The PR check runs no tests.** `.github/workflows/pr.yml` runs the fast checks (and clippy only with the `full-checks` label); tests run on main after the merge (`.github/workflows/checks.yml`). Run the tests of what you changed yourself before opening a PR (`docs/content/guide/testing.md`, "What CI runs").
- **Every feature-gated test must run somewhere**, or `cargo xtask feature-tests` fails the PR's fast checks (below).
- **Share-page and extension UI is verified on the local stack** (`just dev`, `just demo`) with 390 px screenshots before a PR (`AGENTS.md`).

## Recipes

All from `justfile`:

| Recipe | What it does |
|---|---|
| `just showcase` | the app (the standard distribution) on a fresh scratch copy of `apps/ui/showcase-vault`, every showcase extension loaded as a development copy (`scripts/showcase.sh`: `examples/showcase`, the sandbox's Luau, process and WASI extensions, the example counter and the web view board, built first); `just showcase plain` runs Architect's own app. Without it, the app's first run offers "Explore the extension showcase" (`apps/ui/src/vaults.rs` `showcase`), whose **Install all** takes the Luau, TypeScript and data ones from the app itself (`apps/ui/src/showcase.rs`) |
| `just showcase-shots` | desktop screenshots: an isolated Loft session per width, light and dark, into `target/showcase/desktop/` (`scripts/showcase-shots.sh`); first arg `plain` or `standard`, then vault paths or actions |
| `just showcase-web-shots` | the release web bundle in headless Firefox (`apps/web/test/app-showcase.test.mjs`), shots into `target/showcase/web/` |
| `just showcase-publish` | copies `target/showcase/<platform>` onto the `media` branch under `710-showcase/`; `PUSH=1` pushes |
| `just test` | `cargo nextest run --workspace`, doctests, then every `feature-tests.toml` pass |
| `just sandbox` | the dev sandbox (`apps/sandbox`) with the Board, Clicker and WASI Counter |
| `just sandbox-web` / `just sandbox-web-test` | the sandbox's browser demo (WASI Counter transpiled by jco), then its page and TypeScript-action tests |
| `just ext-pane-browser` | the Rust browser applier's input events in headless Firefox (`features/extensions/web/test/rust-pane.test.mjs`) |
| `just app-linked-extensions-test` | a distribution's linked extensions in the web app (W1): ⌘K, panes, settings |
| `just app-documents-test` | the document API, events and kept data in the web app (H1, H2, H4) |
| `just app-index-test` | the vault query service in the browser (H3) |
| `just app-ext-host-test` | the web app's extension frames against hostile bundles |
| `just app-share-guest-test`, `just app-guest-content-test` | a share link opened as its guest |
| `just webview-example` | builds, packs and signs `examples/extension/webview-board` |
| `just docs-check` | `cargo xtask docs`: every path, recipe and command in docs and skills exists |

How to run the showcase and capture each platform (including iOS on the Simulator) is the "Run it and capture it" section of `docs/content/guide/showcase.md`, which is being rewritten: read it there, do not copy it. `scripts/showcase.sh` and `examples/showcase/README.md` are the other sources.

## Tests by runtime

Rust tests run with `cargo nextest run -p <package> --test <file>`; one process per test matters for extensions that keep state in statics (`docs/content/extensions/calendar.md`, its tests).

### The host and the install lifecycle (`-p architect-extensions`)

| File | Covers |
|---|---|
| `features/extensions/tests/linked.rs` | linked (compiled-in) extensions (R1) |
| `features/extensions/tests/registries.rs`, `features/extensions/tests/distribution.rs` | registries, publisher keys, every archive refusal |
| `features/extensions/tests/showcase_registry.rs` | the showcase registry: consent, disable/enable, pin, update, remove, vault plan, macOS-only (L1–L4) |
| `features/extensions/tests/vault_needs.rs`, `features/extensions/tests/share_needs.rs`, `features/extensions/tests/person_sync.rs` | install on open, share links, a person's devices |
| `features/extensions/tests/platforms.rs` | `platforms`, "cannot run here" |
| `features/extensions/tests/check.rs` | `EXT_CHECK` |
| `features/extensions/tests/showcase.rs`, `features/extensions/tests/showcase_ui.rs`, `features/extensions/tests/showcase_content.rs`, `features/extensions/tests/showcase_services.rs` | the `examples/showcase` folders, loaded as development copies |
| `features/extensions/tests/host_service.rs`, `features/extensions/tests/host_ui.rs`, `features/extensions/tests/host_device.rs`, `features/extensions/tests/host_gate.rs` | the host service, interface calls, device APIs, capability gates |
| `features/extensions/tests/contributions.rs`, `features/extensions/tests/toolbars.rs`, `features/extensions/tests/interop.rs`, `features/extensions/tests/events.rs`, `features/extensions/tests/workers.rs` | contributions, services between extensions, events, workers |
| `features/extensions/tests/net.rs`, `features/extensions/tests/node.rs` | `fetch`, secrets, node actions |
| `features/extensions/tests/remote_view.rs`, `features/extensions/tests/remote_input.rs`, `features/extensions/tests/remote_embedded.rs`, `features/extensions/tests/remote_overlay.rs`, `features/extensions/tests/process_views.rs` | process extensions' remote views (all `#![cfg(unix)]`) |
| `features/extensions/tests/linked_paint.rs` | linked paint views |
| `features/extensions/tests/webview_bridge.rs` | the web view kind's bridge |
| `features/extensions/tests/wasi.rs` | WASI components (needs `wasm32-wasip2`; skips and passes without it unless `ARCHITECT_WASI_REQUIRED=1`) |
| `features/extensions/tests/wasi_web.rs` | the same components in the browser's shape under node, then the npm package's tests in `features/extensions/web/test` (skips without node/jco unless `ARCHITECT_WASI_WEB_REQUIRED=1`) |
| `features/extensions/tests/language_servers.rs` | `[[test]]` with `harness = false` and `required-features = ["lsp"]` |

Archive format: `features/extensions/archive/tests/archive.rs` (`-p architect-extension-archive`).

### Pages and TypeScript (`-p architect-pages`)

`features/pages/tests/showcase_pages.rs`, `features/pages/tests/showcase_recipes.rs`, `features/pages/tests/ts_actions.rs`, `features/pages/tests/js.rs`.

### The sandbox's examples

| File | Covers |
|---|---|
| `apps/sandbox/board/tests/both_ways.rs` | the Board linked in and as a process add the same things |
| `apps/sandbox/board/tests/dev_loop.rs` | `EXT_CLONE` from source, rebuild on save (ignored: builds a crate) |
| `apps/sandbox/board/tests/bench.rs` | runner costs (ignored; `--ignored --nocapture`) |
| `apps/sandbox/clicker/tests/check.rs`, `apps/sandbox/clicker/tests/host.rs`, `apps/sandbox/clicker/tests/manager_page.rs`, `apps/sandbox/clicker/tests/themed.rs` | a process extension, the manager page, theming |
| `apps/sandbox/kanban/tests/drag.rs`, `apps/sandbox/sketch/tests/sketch.rs` | rich input and scenes in remote views |
| `apps/sandbox/kanban/tests/shots.rs`, `apps/sandbox/sketch/tests/shots.rs` | headless screenshots of those views |
| `apps/sandbox/media/tests/builtins.rs` and the rest of `apps/sandbox/media/tests` | golden films of the sandbox (no pass turns on `architect-app/wasi` because of these) |

### Browsers (Node test runner, headless Firefox over WebDriver BiDi)

- `apps/web/test/*.mjs`: the web app. Each has a `just app-*` recipe above; run them with `env -u LD_LIBRARY_PATH` (the recipes do).
- `apps/sandbox/web/test/pages.test.mjs`, `apps/sandbox/web/test/actions.test.mjs`: `just sandbox-web-test`.
- `features/extensions/web/test/*.mjs`: the npm package and browser runner. `npm test`, `npm run test:unit` and `npm run test:browser` in `features/extensions/web` (scripts in its `package.json`); `features/extensions/tests/wasi_web.rs` runs the main ones from cargo.
- `.github/workflows/browser.yml` runs the share-guest, ext-host, favourites and `features/extensions/web/test/webview.test.mjs` suites after a merge and nightly (`ARCHITECT_BROWSER_REQUIRED` makes a missing browser a failure).

## `cargo xtask feature-tests`: registering a gated test

Main's suite is `cargo nextest run --workspace`, which builds each member with the features the workspace unifies. A test that needs more never builds there, silently. `xtask/src/feature_tests.rs` resolves that build with guppy (resolver 2, dev-deps on, `x86_64-unknown-linux-gnu`) and finds every test it leaves out:

- a `[[test]]` whose `required-features` the build lacks;
- a test behind a false `cfg(…feature…)`: a file's `#![cfg(…)]`, a `mod x;` under `#[cfg(…)]`, an inline `mod tests`, or a `#[test]` fn with its own `#[cfg(…)]`.

Each must be covered in `feature-tests.toml`:

```toml
[[pass]]                       # one extra `cargo nextest run`
name = "my-feature"
packages = ["architect-extensions"]
features = ["architect-extensions/lsp"]
# env = { ARCHITECT_WASI_REQUIRED = "1" }   # set for this pass
# workspace = true    # build --workspace with the features, run only `packages`

[[skip]]                       # or say why it never runs in CI
package = "architect-extensions"
file = "tests/some_test.rs"    # relative to the package
reason = "needs real hardware"
```

The check fails on a gated test no pass runs and no skip names, on a pass that runs nothing the workspace run does not, and on a stale skip. Use `workspace = true` for a member high in the graph, where `-p` alone would rebuild everything under it with other features.

| Command | Use |
|---|---|
| `cargo xtask feature-tests` | the check (PR fast checks, main's lint) |
| `cargo xtask feature-tests --list` | every gated test and what runs it |
| `cargo xtask feature-tests --run` | run every pass (`just test`, main's Tests job) |
| `cargo xtask feature-tests --run --filter <expr> -- <nextest args>` | a filtered run |

The lint job also runs `cargo clippy` for each pass the PR reaches (on a PR only with the `full-checks` label, `.github/workflows/pr.yml`; always on main), so a pass's feature set must lint clean. Blind spots (by design): `#[cfg(feature)]` on an inline module other than `tests`, `#[cfg_attr(…, test)]`, tests made by macros. `#![cfg(unix)]` alone is not a feature gate.

## `cargo xtask surfaces`

`xtask/src/surfaces.rs`: any member whose `src/` serves MCP (`serve_stdio`, `serve_loopback`, `McpHost`, `http::router`, …) must also serve the `actions` CLI or control socket (and the reverse), and have an `architect_mcp::parity` test (`Parity::new`) showing both list the same actions and answer alike. An app with no stdio may write `// surfaces: mcp via <member dir>` in its `src/`. Extensions reach MCP through the host's Action List, so an extension crate rarely trips it; a new app or host crate that mounts MCP does.

## Adding a showcase row or cell

The matrix in `docs/content/guide/showcase.md` has one table per group (R runtimes, L install and lifecycle, C contributions, H host services, U drawing and input, W web). Each row:

```
| Id | Capability | Desktop | Web | iOS | Example | Evidence |
```

- **Id**: #710's ids (R1…, L1…, C1…, H1…, U1…, W1…, G1). Add a new id only for a capability #710 tracks.
- **Platform cells**: ✅ works, shown by a showcase example with a test and screenshots there; ⚠️ partial (say what is missing) or tested but "not shown"; ❌ does not run (give the reason and issue); `n/a` not about that platform.
- **Example**: the showcase example's name; it lives in `examples/showcase/<name>` (or the sandbox), and its manifest and note in `apps/ui/showcase-vault` say what it stands in for. Add it to "The example extensions" table too.
- **Evidence**: the test file paths first (code spans; they must exist), then screenshot links for each width and theme.

Steps to move a cell:

1. Add or extend the example in `examples/showcase/` and its test (usually `features/extensions/tests/showcase*.rs` or `features/pages/tests/showcase_*.rs`).
2. Run it: `just showcase` (or the web or iOS route in the guide's run section).
3. Capture: `just showcase-shots plain <vault path or action>` and/or `just showcase-web-shots`.
4. Publish: `just showcase-publish PUSH=1` puts them under `710-showcase/<platform>/` on the `media` branch; link the `raw.githubusercontent.com/…/media/710-showcase/…` URLs. One-off PR shots go on a `pr-media/<name>` branch, linked with `?raw=true`. Never commit screenshots to `main`.
5. Update the cell and its evidence in the same PR as the code.

## Platform matrix: can I do X on web or iOS?

A summary of `docs/content/guide/showcase.md` as of 2026-11-14; read the guide for the current cell and its evidence.

| Capability | Desktop | Web | iOS |
|---|---|---|---|
| Linked (compiled-in) extension (R1, W1) | yes | yes, the distribution's linked ones | yes (same host; not shown) |
| Luau installed at run time (R2) | yes | yes: actions and hooks in Web Workers on luau-web (#812) | yes (Simulator) |
| TSX page / TypeScript actions on QuickJS (R3, R7) | yes | sandbox lab only; the web app does not host installed ones (#495, #723, #736) | yes (QuickJS, interpreted) |
| WASI component (R4) | built with `wasi` (#716); not captured | no (#495; needs JSPI) | built with Pulley (#716); not captured |
| Process extension (R5) | yes | no, by nature | no, by nature |
| Web view app in a pane (R6) | Linux X11/XWayland, `--features webview`; not in the release build | browser frame host exists; web app not wired (#495) | `WKWebView` on the Simulator; not on TestFlight |
| Install from a registry, consent (L1) | yes | no (#495) | Luau, pages, data; not shown |
| Install on open (L2) | vault and project folders; share links not wired (G1) | no (#495) | same banner; not shown |
| Remote views from a process (U1, U2) | yes | no host (#495) | no (no processes) |
| Linked Dioxus views, settings sections (C4, C5) | yes | yes | yes |
| Document API on the vault (H1) | yes | demo vault yes; org vault create/edit only | same host; not shown |
| Device APIs: camera, photos, HealthKit (H15) | `unsupported` (#709) | `unsupported` (#709) | photos yes; camera built; HealthKit compiled-in only; location not started |
| Secrets (H5) and `fetch` (H6) | yes | no (#495) | not shown |
| Node actions (H16) | yes; not shown | client exists; host not wired (#495) | same code; not shown |
| Share-link guests run extensions (G1) | no | no | no |

Rule of thumb: on the web today, linked extensions, TypeScript actions and pages, and Luau actions and hooks run; on iOS, anything interpreted (Luau, QuickJS pages, WASM on Pulley, linked code) but no processes.
