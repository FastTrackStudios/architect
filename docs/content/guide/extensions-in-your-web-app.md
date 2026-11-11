+++
title = "Extensions in your own web app"
description = "@fasttrackstudio/architect on npm: load an Architect extension (a WASI component) in any TypeScript app, pinned by its lock, in an opaque-origin sandbox; run its actions with types; mount its views in a shadow root; it reaches only what you grant."
weight = 131
+++

An Architect extension built as a WASI component runs in four places: compiled into the app, as a
separate program, sandboxed inside Architect, and sandboxed inside **any web app**. This page covers
the fourth.

Two npm packages cover it:

- `@fasttrackstudio/architect` is the runtime. It has no dependencies. It loads the same `.wasm` file
  the desktop runs into a sandbox in your page, calls its actions with types, and draws its views.
- `@fasttrackstudio/architect-cli` is the build tool (`architect-ext build-web`). It depends on jco.

The runtime is the browser host Architect's own web app uses (`features/extensions/web/runner.js` and
`features/extensions/web/instance.js`; see [extensions, "On the web"](@/architecture/extensions.md)),
with a host you implement instead of Architect's. A working page is `examples/web-counter`: Vite and
TypeScript, the sandbox's WASI Counter, nothing but the public API, tested in headless Firefox under
the CSP below.

## Install and build

```sh
npm install @fasttrackstudio/architect
npm install -D @fasttrackstudio/architect-cli
npx architect-ext build-web my_extension.wasm public/my-extension --manifest extension.toml --types src/my-extension
```

`build-web` (`features/extensions/web/cli/cli.mjs`) does five things:

1. **It checks the manifest** (`features/extensions/web/cli/manifest.mjs` `checkManifest`, the rules
   of the native `Manifest::check` that the tool relies on). The id, version and api must match their
   grammars. Action ids must be upper case and in the extension's namespace, effects must be one of
   the three, and `params` must be a JSON object. A bad manifest is refused, so nothing in it can
   become code.
2. **It transpiles** the component with [jco](https://github.com/bytecodealliance/jco), pinned by the
   CLI.
3. **It writes `architect.json`** (the manifest, the transpiled module's name, and the
   `architect:extension` world version read from the component) **and `architect.lock.json`** (a
   SHA-256 of every file a page loads). It prints the lock's own digest: the `integrity` you pin.
4. **It writes `actions.d.ts` and `actions.js`**, the typed actions (below), with `integrity`
   exported beside them.
5. **It prints what a page downloads**: each file raw, gzip and brotli, the extension's total and the
   runtime's own code, with a warning over 1 MiB brotli (`--budget`).

Serve the output folder as static files. Your bundler needs no plugin.

## Load it, pinned

From `examples/web-counter/src/main.ts`:

```ts
import { isSupported, loadExtension, memoryHost, mountView } from '@fasttrackstudio/architect';
import { bind, integrity, type Actions } from './counter/actions.js';

if (!isSupported()) showMessage('This browser cannot run extensions yet.');
const ext = await loadExtension<Actions>('/counter/', {
  integrity, // its files pinned: the lock build-web wrote, by its digest
  host: memoryHost({ actions: { THEME_LIST: { effect: 'read_only', run: () => ['light', 'dark'] } } }),
  grant: { actions: ['THEME_LIST'] }, // nothing is granted unless you grant it
});
await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 }); // checked at compile time
await bind(ext).ask();
await mountView(ext, 'counter', document.getElementById('counter')!).rendered;
```

**Pin what you load.** With `integrity`, the page fetches the lock, checks it against the digest
you pinned, and checks every file's bytes against the lock before anything uses it. A file that
differs, is missing from the lock, or lies outside the folder is refused. Without `integrity`,
nothing is checked: whoever serves the folder decides what runs in the sandbox.

The module named in `architect.json` must be a relative path inside the folder: no scheme, no `..`,
no leading `/`.

An `Extension` has its `manifest`, `world`, `capabilities` (frozen), `actions` (`id`, `name`,
`description`, `effect`, `schema`), `views`, `run(id, args)`, `report()` and `stop()`, and nothing
else. A failed call rejects with an `ArchitectError`. Its `kind` and its words match the native host's
`hostcall::HostError`.

`isSupported()` says whether the browser has JS Promise Integration, which the runtime needs. It does
in Chromium 137 and later, Firefox (checked on 159) and node 24 and later. Safari does not have it
yet. There, `loadExtension` rejects with that reason, and your page should say the feature needs
another browser.

## The security model

**What the sandbox protects: your page from the extension.**

- **Its JS runs in an opaque origin.** An extension ships its own JS (jco's glue) beside its
  `.wasm`, so the runtime treats that JS as hostile. Each instance's Web Worker is started inside a
  hidden `<iframe sandbox="allow-scripts">` (no `allow-same-origin`) that the runtime makes
  (`runner.js` `Sandbox`). The frame has its own CSP (`SANDBOX_CSP`): `default-src 'none'`,
  `connect-src 'none'`, scripts only its own (by hash) and blob modules.
- **The page hands it everything.** The page fetches the files (and checks them, above) and hands in
  the glue as text and the core modules as bytes. Everything else crosses on a `MessageChannel` port
  the frame takes only from its parent.
- **It reaches no cookie, storage, channel or network of yours.** It has no cookie-bearing request,
  no IndexedDB, Cache Storage or BroadcastChannel of your origin, no `fetch`, XHR, WebSocket or
  beacon, and no script import from your origin. `features/extensions/web/test/glue.test.mjs` runs a
  fake glue that tries each of these under the CSP below and checks that your server hears none of
  them.
- **It reaches only what you grant.** Everything it asks of the app is checked against its
  capabilities within your `grant` before your handler sees it (below).
- **Its wasm is limited too**: memory, time and floods (see Limits).
- **Its views can't escape into your page.** They draw in a shadow root with the share pages'
  sanitiser rules, made stricter (see Views).

**What it does not protect:**

- **The extension's data from your page.** Your page is its host. It answers its requests and can read
  its views (the shadow root is open).
- **Anything under node.** Node has no iframes. There the extension's JS runs in a worker thread with
  what node gives every worker: files, network and environment. **A node host trusts the extension's
  JS**; its wasm is still limited to its imports and capabilities. Run an extension you don't trust
  in a browser, or in a process of its own (`node --permission`).
- **Files you didn't pin.** See "Load it, pinned".
- **Anything, through the world version.** The check that refuses a component built for a newer
  `architect:extension` world (see Versions) is about compatibility, not security.

## The host: what it may ask your page for

Everything an extension asks of its app reaches your page as a host-service request
(`features/extensions/src/hostcall.rs` `HostRequest`; the TypeScript union is `HostRequests` in
`index.d.ts`). Each request goes through the same steps:

1. **It is checked first** (`features/extensions/web/check.js`, a port of `hostcall::check` with the
   same capability names and words). The extension reaches what its manifest's `[capabilities]` asks
   for, within what you `grant`. **Nothing is granted by default.** A wider grant gives it no more
   than it asked for.
2. **Then it reaches your handler for its `op`.** `host.handle` has one optional handler per op
   (`read_document`, `fetch`, `notify`, …). Each gets the typed request and a `HostContext` (the
   extension's id, manifest and frozen capabilities), and returns the answer or throws an
   `ArchitectError`.
3. **An op you don't handle answers `unsupported`** ("… is not available in this host"). The
   extension gets an error it can show, never a throw and never a hang.

**App actions** (`run-action`) are `host.actions`, each with its `effect` (`mutating` if you leave it
out), and the grant's `effect` is checked against it. An action you offer is never the extension's
own, whatever its name. An extension `vault` granted nothing cannot run your `VAULT_EMPTY_TRASH`.
The native hosts still treat the namespace as the extension's own (#545).

**Destructive actions need a yes.** An action declared `effect = "destructive"`, whether the
extension's own run by your page or an app action it runs, runs only when `host.confirm(action,
args)` resolves to exactly `true`. This applies even when `handle.run` answers the request. Without
`confirm`, none of them runs.

**What your handler must still check itself**, because only it knows:

- a path against `ctx.capabilities.folders` (`read_file`, `write_file`; `memoryHost` does this);
- that a stream, port or hook is this extension's own (`read_body`, `close_body`, `port_send`,
  `port_receive`, `close_port`, `unsubscribe`, `raise`);
- a service's, point's or window's own rules (`service`, `point`, `window`).

**The network.** Use `fetchHandler({ allow })` and spread it into `handle`. It never sends your
credentials (`credentials: "omit"`), never reaches your page's own origin, and goes only to `allow`
(host names, `*.domain`, `host:port`), on top of the extension's `capabilities.hosts`. It follows no
redirect (a page cannot see where one goes, so it could not check it), and has a time and a size
limit. If you write your own `fetch` handler, keep all of those.

`memoryHost(options)` is for demos. It keeps documents and files in `Map`s seeded from what you
hand it (files only under granted `folders`, and only those you handed it). It answers `org` and
`selection` with what you pass, collects notifications, and runs the app actions you give it. It
persists nothing and has no network or clipboard.

## Typed actions

`actions.d.ts` maps each action id to its arguments, from the manifest's `params` JSON schema, so
`ext.run(id, args)` fails to compile on a wrong id or a wrong argument. `bind(ext)` gives the same
actions as methods named after the id without the extension's namespace (`LAB_WASI_COUNTER_ADD` →
`add`).

The generator (`features/extensions/web/cli/typegen.mjs`) writes ids, keys and literals with
`JSON.stringify`. It takes method names only from identifiers that match a strict pattern, and puts
no manifest text in a comment. A test feeds it hostile manifests and checks two things: the output
has the same syntax tree, and running it in a VM context trips nothing.

It reads this JSON schema subset:

- `type` (one or a list), `enum`, `const`;
- `properties` and `required`;
- `additionalProperties`: left out or `false` gives a closed object, so a misspelt key fails to
  compile;
- `items`, `anyOf`/`oneOf`, and `allOf`.

Anything else becomes `unknown`. Answers are `unknown`, because manifests have no result schema.

## Views

`mountView(extension, viewId, element, { urls })` draws a view into `element` inside a **shadow
root** of its own. It returns `{ rendered, input, unmount }`. React has `ArchitectView`
(`@fasttrackstudio/architect/react`).

The view's edits are the remote-view wire's (see [embedded views](@/architecture/embedded-views.md)).
`features/extensions/web/view.js` applies them as a port of `features/extensions/src/web.rs` `Pane`.
It follows the share pages' rules (`features/sites/src/sanitize.rs`, ported to
`features/extensions/web/sanitize.js`), made stricter for a live view in someone else's page:

- **No code.** No `<script>`, frames, `<object>`, `<foreignObject>`, SMIL animation or `<form>`. No
  `on…` handler or `dangerous_inner_html` (the frame fails).
- **No network it wasn't allowed.**
  - An image or media URL loads only if it starts with one of the `urls` you pass (absolute `https:`
    or `blob:` prefixes; none by default), or if it is a `data:` raster image.
  - A relative URL is always refused: it would be a request to your origin with your cookies.
  - CSS that loads anything (`url(…)`, `image-set`, …) is refused.
  - A link goes only to `https:`, `mailto:` or one of the view's own fragments, and opens in a new tab
    with `noopener noreferrer`.
- **Fields that read like nothing.** `input`, `textarea`, `select` and `label` are kept, since the
  view is interactive.
  - They get no `name`, `form` or `autocomplete` (forced `off`).
  - Input types come only from a short list (text, search, number, range, checkbox, radio, color,
    date, time): never `hidden`, `password`, `file` or `email`.
  - A `<button>` is always `type="button"`.
  - In a shadow root, its fields belong to no form of yours: your `FormData` is unchanged.
- **Ids that are its own.** Every id, and every reference to one (`for`, the ARIA id lists,
  `href="#…"`, `url(#…)`), gets the pane's prefix. Inside a shadow root, its ids neither resolve in
  `document.getElementById` nor create named properties on `window`.
- **A box it stays in.** No `position` but `static` or `relative`, CSS containment, and a capped
  height (`LIMITS.viewHeight`, 4096 px). Its CSS stays out of your page and yours stays out of it.
- **Limits on frames.** At most `LIMITS.editsPerFrame` edits per frame and `LIMITS.nodesPerPane`
  nodes per pane. Mouse events don't say where the screen is.

Because `var(…)` is refused, a view's theme variables fall back to their defaults. A view does not
follow your page's theme yet.

## Limits

Each instance has limits (`limits`, `workerLimits`; defaults in `LIMITS`):

- **Memory and time.** 64 MiB and 5 s per call by default, and 64 MiB and 60 s for a worker's whole
  run. The memory limit is written into each core module. A call past its time has its worker
  terminated, and the extension starts again.
- **Floods.**
  - At most 200 log lines a second, each up to 4096 characters; the rest are counted and reported.
  - A view's timers are at least 16 ms apart.
  - At most 5 restarts a minute, each after a longer wait (up to 10 s). After that it stays stopped,
    and `report().lastTrap` says why.

Under node, an idle extension doesn't keep the process alive, and `stop()` ends everything at once.
The details and how they differ from the native runner are in
[extensions, "On the web"](@/architecture/extensions.md).

## Your CSP

Start from `default-src 'none'`. `examples/web-counter/index.html` uses this and its test runs under
it:

```text
default-src 'none';
script-src 'self' 'wasm-unsafe-eval' blob: 'SANDBOX_HASH';
worker-src blob:;
frame-src 'none';
connect-src 'self';
img-src 'self' data:;
media-src 'none';
style-src 'self';
base-uri 'none'; form-action 'none'; object-src 'none'
```

- **`'SANDBOX_HASH'`** is the runtime's `SANDBOX_HASH` export, the hash of the sandbox frame's
  script. It changes with the runtime's version, so put it in from the package: the example's Vite
  config does. The frame is a `srcdoc` document, which inherits your policy.
- **`blob:` and `'wasm-unsafe-eval'`** in `script-src`, and **`worker-src blob:`**, are for the
  sandbox frame's workers, its blob modules and its wasm. These run in the frame's opaque origin, and
  each inherits your policy as well as the frame's own.
- **`frame-src 'none'`** still allows the sandbox's `srcdoc` frame (Firefox checked).
- **`connect-src`** must cover where the extension's files are served from: the page fetches them,
  never the extension.
- **`img-src` and `media-src`** must cover the `urls` you pass to `mountView`, if any.
- Views need no `'unsafe-inline'`: their styles are set through the CSSOM.

**Trusted Types.** With `require-trusted-types-for 'script'`, add
`trusted-types fasttrackstudio-architect`. The runtime makes a policy of that name for the frame's
`srcdoc`, which passes only its own document, and the frame makes one for its workers, which passes
only `blob:` URLs. The example's test runs under both.

## Sizes

What a page downloads for the sandbox's WASI Counter, a release build (`build-web`'s report):

| File | Raw | gzip | brotli |
| --- | ---: | ---: | ---: |
| `architect_sandbox_wasi_counter.core.wasm` (the component's code) | 1035.6 KiB | 310.6 KiB | 229.8 KiB |
| `architect_sandbox_wasi_counter.js` (jco's glue) | 405.4 KiB | 54.2 KiB | 42.2 KiB |
| `architect.json` | 2.2 KiB | 0.8 KiB | 0.6 KiB |
| five small adapter modules | 0.9 KiB | 0.7 KiB | 0.6 KiB |
| **the extension, in all** | **1444.1 KiB** | **366.2 KiB** | **273.3 KiB** |

A debug build of the same component is 990 KiB brotli. The runtime's own code adds about 35 KiB
brotli before your bundler minifies it. Most of an extension's size is its core module: build it
with `--release`.

## Versions

The packages' version is Architect's release version. A `v*` tag publishes both
(`.github/workflows/release.yml`, jobs `npm-pack` and `npm-publish`), with pre-releases under the
`next` dist-tag, never `latest`.

The runtime runs one `architect:extension` world (`WORLD`, the version in
`features/extensions/wit/extension.wit`). `loadExtension` refuses a component built for a newer
world, or a manifest written for a newer `api`, and says to update the package. This is a
compatibility check, not a security check. The native hosts don't check the world version yet
(#507).

## What is not there yet

- Keyflow's chart engraver as a component, and the TanStack Start demo (#535).
- A view following your page's theme (`var(…)` is refused).
- Answers typed from a result schema (manifests have none).
- A sandbox under node (see above).
- The native hosts' own-namespace rule (#545) and world check (#507).
