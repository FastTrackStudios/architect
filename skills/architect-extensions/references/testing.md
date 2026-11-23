# Testing extensions

What to run to prove an extension works on each runtime, and how CI finds every test.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/guide/testing.md` ("What CI runs"), `docs/content/architecture/extensions.md`, `docs/content/guide/cli-and-mcp.md` (the parity rule behind `cargo xtask surfaces`).

## Rules first

- **Evidence is a test.** An extension, a runtime or a host call is done when an automated test runs it, plus screenshots for anything it draws (390 px and 1280 px, light and dark), taken in the app that hosts it.
- **The PR check runs no tests.** `.github/workflows/pr.yml` runs the fast checks (and clippy only with the `full-checks` label); tests run on main after the merge (`.github/workflows/checks.yml`). Run the tests of what you changed yourself before opening a PR (`docs/content/guide/testing.md`, "What CI runs").
- **Every feature-gated test must run somewhere**, or `cargo xtask feature-tests` fails the PR's fast checks (below).
- **The GUI side is tested where it lives.** Panes, remote views, toolbars and the settings sheet are drawn by `app-shell:crates/app-shell-extensions`; their tests are in `app-shell:crates/app-shell-extensions/tests`.

## Recipes

All from `justfile`:

| Recipe | What it does |
|---|---|
| `just test` | `cargo nextest run --workspace`, doctests, then every `feature-tests.toml` pass |
| `just process-view-example` | the process-view demo: a GPU renderer in its own process, drawn into a pane |
| `just webview-example` | builds, packs and signs the web view board (`features/extensions/tests/fixtures/examples/extension/webview-board`) |
| `just docs-check` | `cargo xtask docs`: every path, recipe and command in docs and skills exists |

## Tests by runtime

Rust tests run with `cargo nextest run -p <package> --test <file>`; one process per test matters for extensions that keep state in statics.

### The host and the install lifecycle (`-p architect-extensions`)

| File | Covers |
|---|---|
| `features/extensions/tests/linked.rs` | linked (compiled-in) extensions |
| `features/extensions/tests/registries.rs`, `features/extensions/tests/distribution.rs` | registries, publisher keys, every archive refusal |
| `features/extensions/tests/showcase_registry.rs` | a registry end to end: consent, disable/enable, pin, update, remove, vault plan, macOS-only |
| `features/extensions/tests/vault_needs.rs`, `features/extensions/tests/share_needs.rs`, `features/extensions/tests/person_sync.rs` | install on open, share links, a person's devices |
| `features/extensions/tests/platforms.rs` | `platforms`, "cannot run here" |
| `features/extensions/tests/check.rs` | `EXT_CHECK` |
| `features/extensions/tests/showcase.rs`, `features/extensions/tests/showcase_ui.rs`, `features/extensions/tests/showcase_content.rs`, `features/extensions/tests/showcase_services.rs` | the example extensions in `features/extensions/tests/fixtures/examples/showcase`, each loaded from its folder as a development copy and run through its actions |
| `features/extensions/tests/host_service.rs`, `features/extensions/tests/host_ui.rs`, `features/extensions/tests/host_device.rs`, `features/extensions/tests/host_gate.rs` | the host service, interface calls, device APIs, capability gates |
| `features/extensions/tests/contributions.rs`, `features/extensions/tests/toolbars.rs`, `features/extensions/tests/interop.rs`, `features/extensions/tests/events.rs`, `features/extensions/tests/workers.rs` | contributions, services between extensions, events, workers |
| `features/extensions/tests/net.rs`, `features/extensions/tests/node.rs` | `fetch`, secrets, node actions |
| `features/extensions/tests/process_views.rs`, `features/extensions/tests/view_kinds.rs` | process views and the view kinds |
| `features/extensions/tests/webview_bridge.rs` | the web view kind's bridge |
| `features/extensions/tests/wasi.rs`, `features/extensions/tests/wasi_wit.rs` | WASI components (needs `wasm32-wasip2`; skips and passes without it unless `ARCHITECT_WASI_REQUIRED=1`) |
| `features/extensions/tests/wasi_web.rs` | the same components in the browser's shape under node, then the npm package's tests in `features/extensions/web/test` (skips without node/jco unless `ARCHITECT_WASI_WEB_REQUIRED=1`) |

Archive format: `features/extensions/archive/tests/archive.rs` and `features/extensions/archive/tests/npm_pack.rs` (`-p architect-extension-archive`). Luau scripting: `features/scripting/tests` (`-p architect-scripting`). Process views: `features/process-view/tests` (`-p architect-process-view`).

### The GUI side (app-shell)

| File | Covers |
|---|---|
| `app-shell:crates/app-shell-extensions/tests/remote_view.rs`, `app-shell:crates/app-shell-extensions/tests/remote_input.rs`, `app-shell:crates/app-shell-extensions/tests/remote_embedded.rs`, `app-shell:crates/app-shell-extensions/tests/remote_overlay.rs`, `app-shell:crates/app-shell-extensions/tests/remote_barrier.rs` | process extensions' remote views (all `#![cfg(unix)]`) |
| `app-shell:crates/app-shell-extensions/tests/linked_paint.rs`, `app-shell:crates/app-shell-extensions/tests/scene_bytes.rs` | linked paint views and scenes |
| `app-shell:crates/app-shell-extensions/tests/panes.rs`, `app-shell:crates/app-shell-extensions/tests/windows.rs`, `app-shell:crates/app-shell-extensions/tests/settings_sheet.rs`, `app-shell:crates/app-shell-extensions/tests/keys.rs` | panes, pop-out windows, the settings sheet, keys |

### Browsers (Node test runner, headless Firefox over WebDriver BiDi)

- `features/extensions/web/test/*.mjs`: the npm package and browser runner. `npm test`, `npm run test:unit` and `npm run test:browser` in `features/extensions/web` (scripts in its `package.json`); `features/extensions/tests/wasi_web.rs` runs the main ones from cargo. Run them with `env -u LD_LIBRARY_PATH`.
- `.github/workflows/browser.yml` runs the browser suites (`features/extensions/web/test/webview.test.mjs` among them) after a merge and nightly (`ARCHITECT_BROWSER_REQUIRED` makes a missing browser a failure).

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

## Can I do X on web or iOS?

| Capability | Desktop | Web | iOS |
|---|---|---|---|
| Linked (compiled-in) extension | yes | yes, the app's linked ones | yes |
| Luau installed at run time | yes | actions and hooks in Web Workers on luau-web, when the app gives a runner | yes |
| TypeScript actions | when the app registers a runner for `typescript` | Web Workers, when the app registers a runner | when the app registers a runner |
| WASI component | built with `wasi` | no (#495; needs JSPI) | built with Pulley |
| Process extension | yes | no, by nature | no, by nature |
| Web view in a pane | Linux X11/XWayland, `--features webview`; macOS | browser frame host | `WKWebView` |
| Install from a registry, consent | yes | no (#495) | Luau, data |
| Process views (remote views, GPU process views) | Linux and macOS | no | no (no processes) |
| Device APIs: camera, photos, HealthKit | `unsupported` (#709) | `unsupported` (#709) | photos yes; camera built; HealthKit compiled-in only |

Rule of thumb: everything interpreted (Luau, WASM on Pulley, linked code) runs on iOS but no processes; the browser runs linked extensions and what a Web Worker can run.
