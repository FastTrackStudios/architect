+++
title = "Extensions in your own web app"
description = "@fasttrackstudio/architect on npm: load an Architect extension (a WASI component) in any TypeScript app, run its actions with types, mount its views, inside a sandbox that reaches only what you grant."
weight = 131
+++

An Architect extension built as a WASI component runs in four places: compiled into the app, as a
separate program, sandboxed inside Architect, and sandboxed inside **any web app**. This page covers
the fourth. The npm package `@fasttrackstudio/architect` loads the same `.wasm` file the desktop runs.
It runs the extension in a Web Worker with memory and time limits. It calls the extension's actions
with types generated from its manifest and draws its views into your page. Your page decides what the
extension may reach.

The package is the browser host Architect's own web app uses
(`features/extensions/web/runner.js` and `features/extensions/web/instance.js`; see
[extensions, "On the web"](@/architecture/extensions.md)), packaged with a host you implement
instead of Architect's. The source is `features/extensions/web`. A working page is
`examples/web-counter`: Vite and TypeScript, the sandbox's WASI Counter, nothing but the public API.

## Install and build

```sh
npm install @fasttrackstudio/architect
npx architect build-web my_extension.wasm public/my-extension --manifest extension.toml --types src/my-extension
```

`build-web` does four things (`features/extensions/web/cli.mjs`):

1. It transpiles the component with [jco](https://github.com/bytecodealliance/jco) (pinned by the
   package): core wasm modules and a JS module that instantiates them (`transpile.mjs`).
2. It writes `architect.json`: the manifest (`extension.toml` as JSON) and the `architect:extension`
   world version the component was built for, read from the component itself.
3. It writes `actions.d.ts` and `actions.js`, the extension's typed actions (below), to `--types`, or
   beside the output.
4. It prints what a page downloads: each file raw, gzip and brotli, the extension's total and the
   package's own code. Over 1 MiB brotli (`--budget` changes it) it prints a warning.

Serve the output folder as static files. Your bundler needs no plugin: the package starts each
instance's worker from a string, not from a worker file it would have to find.

## Load it and run its actions

From `examples/web-counter/src/main.ts`:

```ts
import { loadExtension, memoryHost, mountView } from '@fasttrackstudio/architect';
import { bind, type Actions } from './counter/actions.js';

const host = memoryHost({
  // An app action the page offers extensions: they run it only if granted.
  actions: { THEME_LIST: { effect: 'read_only', run: () => ['light', 'dark', 'sepia'] } },
});
const ext = await loadExtension<Actions>('/counter/', {
  host,
  // Its manifest asks for THEME_LIST; the page grants it. Nothing else.
  grant: { actions: ['THEME_LIST'] },
});

await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 }); // checked at compile time
await bind(ext).ask();                            // the same, as a method
await mountView(ext, 'counter', document.getElementById('counter')!).rendered;
```

An `Extension` has its `manifest`, `world`, `capabilities` (what it may reach), `actions` (`id`,
`name`, `description`, `effect`, `schema`), `views`, `run(id, args)`, `report()` (calls, traps,
generation, workers) and `stop()`. A failed call rejects with an `ArchitectError`. Its `kind` and its
words match the native host's `hostcall::HostError` (`refused`, `confirmation_required`,
`not_loaded`, `not_found`, `failed`, `unsupported`).

Under node (24 or later), pass `spawn` from `@fasttrackstudio/architect/node`. It runs each instance
in a worker thread. A server-side render or a test can load the same output folder by its `file:` URL.

## The host: what it may ask your page for

Everything an extension asks of its app reaches your page as a host-service request
(`features/extensions/src/hostcall.rs` `HostRequest`; the TypeScript union is `HostRequests` in
`index.d.ts`). Each request goes through the same steps:

1. **It is checked first.** The check is `features/extensions/web/check.js`, a port of
   `hostcall::check` with the same capability names and wording. The extension reaches what its
   manifest's `[capabilities]` ask for, within what you `grant`. **Nothing is granted by default.** A
   wider grant gives it no more than it asked for, and its own actions, data and log need no grant.
   A refusal looks like the native one: `refused (actions): THEME_LIST is not in a namespace this
   extension may run`.
2. **Then it reaches your handler for its `op`.** Your `Host` has a `handle` object with one optional
   handler per `op` (`read_document`, `fetch`, `notify`, …). Each handler gets the typed request and
   returns the answer, or throws an `ArchitectError` (any other throw becomes `failed`).
3. **An `op` you do not handle answers `unsupported`:** "… is not available in this host". The
   extension gets an error it can show. It never sees a throw and never hangs, and the miss goes to
   its log.

App actions an extension may run (`run-action`) are `host.actions`, each with its `effect`
(`mutating` if you leave it out). That effect is what the grant's `effect` is checked against.

**Destructive actions** need a yes. When your page runs one of the extension's actions declared
`effect = "destructive"`, it runs only if `host.confirm(action, args)` resolves `true`. The same
applies when the extension runs a destructive app action. Without `confirm`, none of them runs.

`memoryHost(options)` is a host for demos. It keeps documents and files in `Map`s seeded from what you
hand it (an extension can change only files you handed it). It answers `org` and `selection` with what
you pass, collects notifications, and runs the app actions you give it. It persists nothing and has no
network or clipboard. Settings (with their manifest defaults) and an extension's own data are in
memory for every host.

## Typed actions

`actions.d.ts` maps each action id to its arguments, from the manifest's `params` JSON schema, so
`ext.run(id, args)` fails to compile on a wrong id or a wrong argument. `bind(ext)` gives the same
actions as methods named after the id without the extension's namespace (`LAB_WASI_COUNTER_ADD` →
`add`). The generator is `features/extensions/web/typegen.mjs`. It reads this JSON schema subset:

- `type` (one or a list), `enum`, `const`;
- `properties` and `required`;
- `additionalProperties`: left out or `false` gives a closed object, so a misspelt key fails to
  compile;
- `items`, `anyOf`/`oneOf` (a union), `allOf` (an intersection), and `description` (a doc comment).

It does not read `$ref`, `patternProperties`, tuples, `not`, formats or bounds; those become
`unknown`. A manifest has no result schema, so answers are `unknown`.

## Views

`mountView(extension, viewId, element)` draws a view into `element`, inside a box the view cannot
style its way out of. It returns `{ rendered, input, unmount }`; `input` sends an embed view its input
(`{ claim, source, props }`). React has `ArchitectView` (`@fasttrackstudio/architect/react`):
`<ArchitectView extension={ext} view="counter" />`.

The view's edits are the remote-view wire's (see [embedded views](@/architecture/embedded-views.md)).
`features/extensions/web/view.js` applies them: it is a port of `features/extensions/src/web.rs`
`Pane`, the applier Architect's web app draws views with. What a view may put into your page follows
the share pages' rules (`features/sites/src/sanitize.rs`, ported to
`features/extensions/web/sanitize.js` with its test cases):

- no `<script>`, frames, `<object>`, `<foreignObject>` or SMIL animation, each with everything inside
  it;
- no `on…` handler or `dangerous_inner_html` (the frame fails);
- no `javascript:` or other-site URLs, and no image from another site;
- no CSS that loads, binds or positions itself over the page (`position` only `static` or
  `relative`);
- no ids or `data-*` attributes your page reserves;
- no edit that reaches outside its box.

Events on its elements go back to it.

There is one difference from share pages, because a mounted view is live. Its form fields (`input`,
`textarea`, `select`, `option`, `label`) are kept, with only the attributes they need. No `<form>`
comes through, so nothing submits, and a `<button>` is always `type="button"`. Because `var(…)` is
refused, a view's theme variables fall back to their defaults: a view does not follow your page's
theme yet.

## Limits

Each instance runs in a Web Worker under its limits (`limits`, `workerLimits` in `loadExtension`):
64 MiB of memory and 5 s per call by default, and 64 MiB and 60 s for a worker's whole run. The
memory limit is written into each core module, so `memory.grow` fails past it. A call past its time
has its Web Worker terminated, and the extension starts again fresh. `report().traps` and
`generation` count these restarts. A stopped extension can be loaded again. The details and how they
differ from the native runner are in [extensions, "On the web"](@/architecture/extensions.md).

It needs JS Promise Integration: Chromium 137 or later, Firefox (checked on 159), node 24 or later.
Safari does not have it yet; there the extension does not start, and `loadExtension` rejects with
the reason.

## Security and your CSP

The extension runs in a Worker and reaches nothing but what your host answers. WASI is linked with
no files, environment or network, and a component that imports `wasi:sockets` or `wasi:http` is
refused. Its network is `fetch` through your host, checked against its `capabilities.hosts` first.
Your page needs this CSP, which `examples/web-counter/index.html` uses and its test runs under:

```text
script-src 'self' 'wasm-unsafe-eval'; worker-src blob:; connect-src 'self'
```

- `'wasm-unsafe-eval'` lets the page compile the extension's wasm.
- `blob:` is for each instance's worker, started from a string. Pass `instance` (a URL to the
  package's `instance.js` that you serve) to avoid `blob:`.
- `connect-src` must cover where its files are served from.
- Views need no `'unsafe-inline'`: their styles are set through the CSSOM.

## Sizes

What a page downloads for the sandbox's WASI Counter, a release build (`architect build-web`'s
report):

| File | Raw | gzip | brotli |
| --- | ---: | ---: | ---: |
| `architect_sandbox_wasi_counter.core.wasm` (the component's code) | 1035.6 KiB | 310.6 KiB | 229.8 KiB |
| `architect_sandbox_wasi_counter.js` (jco's glue) | 405.4 KiB | 54.2 KiB | 42.2 KiB |
| `architect.json` | 2.2 KiB | 0.8 KiB | 0.6 KiB |
| five small adapter modules | 0.9 KiB | 0.7 KiB | 0.6 KiB |
| **the extension, in all** | **1444.1 KiB** | **366.2 KiB** | **273.3 KiB** |

A debug build of the same component is 990 KiB brotli.

The package's own code adds about 27 KiB brotli before your bundler minifies it. Most of an
extension's size is its core module. Build it with `--release`, and keep heavy work in a worker
component that loads only when it runs.

## Versions

The package's version is Architect's release version. A `v*` tag publishes it to npm
(`.github/workflows/release.yml`, job `publish-npm`), with pre-releases under the `next` dist-tag,
never `latest`. The package runs one `architect:extension` world (`WORLD`, the version in
`features/extensions/wit/extension.wit`). `loadExtension` refuses a component built for a newer world,
or a manifest written for a newer `api`, with a message that says to update the package. The native
hosts do not check the world's version yet (#507).

## What is not there yet

- Keyflow's chart engraver as a component, and the TanStack Start demo (#535).
- A view following your page's theme (`var(…)` is refused, above), and a theme passed in with
  `host.theme` reaching views as tokens they read (the `theme` import works; nothing maps it to CSS).
- Answers typed from a result schema (manifests have none).
- In `memoryHost`: the network, clipboard, pickers, prompts, menus, the editor, ports, secrets and
  services. Implement the handlers you need.
- Native hosts checking the world's version (#507).
