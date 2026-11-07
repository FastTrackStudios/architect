+++
title = "Pages as files"
description = "A view written as a file — TSX or JS on QuickJS (in a browser, a Web Worker), Luau elements, A2UI JSON — re-rendered live on save, on a thread of its own, drawn as the same edit stream a process view sends; one API defined once in Rust, its TypeScript and Luau types generated from it."
weight = 68
+++

> **Status: landed (2026-11, #273)** in `architect-pages`
> (`features/pages`): the API, the QuickJS and Luau runners, TSX by
> oxc with source maps, A2UI JSON pages, workers and timers in both
> script runners, reload on save with the last good page kept,
> overrides in the app's folder, and the sandbox's Board tab as a file (with
> an A2UI page and a Luau page beside it). In a browser (#325): TSX/JS
> pages in a Web Worker on the browser's own engine, the same file and
> types ("On the web" below). A framework's own DOM (#326):
> `architect/dom`, so Preact runs in a page's runtime, natively and in a
> browser ("A framework's own DOM" below).

## The model

A **page** is a view drawn from a file. Every user-facing feature is a
linked extension (#272): its Rust crate is the model — data, actions,
heavy work — and its pages can be files. A page shows instead of the
extension's compiled view when there is one for it, in this order
(`features/pages/src/runner.rs` `Pages::resolve`):

1. an **override** in the app's folder, `<app dir>/pages/<extension>/<view>.tsx`
   (or `.ts`, `.jsx`, `.js`, `.luau`, `.a2ui.json`; `pages/<view>.tsx` for short) —
   it replaces the view live, and deleting it brings the built-in back;
2. the extension's **own page**, `[[views]] page = "pages/board.tsx"` in
   its manifest (`features/extensions/src/manifest.rs` `ViewDecl`);
3. a **built-in's page**, written read-only under
   `<app dir>/pages/.builtin/<extension>/` (`features/pages/src/files.rs`
   `write_builtin`) and edited by cloning it to the override's place
   (`clone_to_edit`, the `PAGES_CLONE` action);
4. otherwise the compiled view.

The app provides `ViewPages` in its root
(`features/pages/src/ui.rs` `use_view_pages`); `ExtensionView` asks it
first (`features/extensions/src/interface.rs` `ViewPages`, `PageAsk`).

## One API, defined once

The page API is two `#[architect::rpc]` services over facet types in
`features/pages/src/api.rs`:

- `PageHost` — the host service's subset that exists today: `run` an
  action, `actions`, `setting` / `set_setting`, `theme`, `shared` /
  `set_shared` (state every page shares), `notify`; `#[subscribe]`
  streams `theme_changes`, `data` and `workers`. The full host service,
  with consent, is #178's.
- `PageView` — `render(Element)`, overlays, and the `events` stream.

`Element` is the tree a page renders: a tag (`#text` for text), a key,
attributes (`style` through the theme: `var(--foreground)`), handlers
(`Listener`: an event and the number its runtime keeps the function
under) and children. In process a runner binds these calls directly
(`features/pages/src/host.rs` `AppHost`); under `vox` they are vox
services, so a process or the web build speaks the same API.

**Generated types.** `features/pages/src/codegen.rs` writes
`architect.d.ts` — vox's TypeScript codegen over the services'
descriptors (named types and caller interfaces), plus the runtime's
surface — and `architect.d.luau` from the same shapes. The app writes
both into its folder with its agents' guide, which names them
(`features/control/src/guide.rs` `Guide::file`; the sandbox's
`apps/ui/src/lib.rs` `page_types`); `write_types` writes them on
their own. The copies in `features/pages/types/` are
checked by `features/pages/tests/types.rs`, which fails when they drift
(`BLESS=1` writes them again).

## A page

```tsx
import { h, useData, useState, actions, worker, theme } from "architect";

export default function Board() {
  const board = useData<{ pins: string[] }>("LAB_BOARD_STATE", {}, { every: 500 });
  const [title, setTitle] = useState("");
  return (
    <div style={{ padding: 24, color: "$foreground" }}>
      <input value={title} onInput={(e) => setTitle(e.value ?? "")} />
      <button onClick={() => actions.run("LAB_BOARD_PIN", { title })}>Pin</button>
      {(board.value?.pins ?? []).map((p) => <div key={p}>📌 {p}</div>)}
      <small>{theme.scheme}</small>
    </div>
  );
}
```

The sandbox's Board tab is this, in full: `apps/sandbox/board/pages/board.tsx`
with its worker `apps/sandbox/board/pages/primes.js`
(`apps/sandbox/board/tests/page.rs`). A Luau page builds the same tree:

```lua
local n = 0
render(function()
  return div():style({ padding = 12 }):child(span():child("clicked " .. n))
    :child(button():child("+1"):on_click(function() n += 1 end))
end)
```

And an A2UI page declares it, bound to its extension's data and actions
(`apps/sandbox/board/pages/summary.a2ui.json` is the Board's, in full):

```json
{
  "components": [
    { "id": "root", "component": "Column", "children": ["draft", "pin", "pins"] },
    { "id": "draft", "component": "TextField", "label": "Title", "value": { "path": "/draft" } },
    { "id": "pin", "component": "Button", "child": "pin-label",
      "action": { "event": { "name": "LAB_BOARD_PIN", "context": { "title": { "path": "/draft" } } } } },
    { "id": "pin-label", "component": "Text", "text": "Pin" },
    { "id": "pins", "component": "List", "children": { "componentId": "pin-row", "path": "/board/pins" } },
    { "id": "pin-row", "component": "Text", "text": { "path": "" } }
  ],
  "data": { "draft": "" },
  "bind": { "/board": { "action": "LAB_BOARD_STATE", "every": 1000 } }
}
```

## The runners

**One thread per page** (`features/pages/src/runner.rs` `Pages::open`,
`PageSession`). The page's runtime is made on its thread and lives
there; it waits on an inbox (the pane's events, host calls' answers,
shared state, workers' news), calls into the page, and when the page
asked to render, diffs the new tree against what it drew
(`features/pages/src/diff.rs` `Mirror`: keyed children keep their
nodes, a changed attribute or text is set in place, a handler whose
function changed makes no edit) and sends the difference as a `Frame`
of `wire::Edit`s. The window applies frames to a Blitz sub-document —
the `RemoteDocument` a process extension's view fills, with the host's
live theme CSS, events, focus and IME (`features/pages/src/ui.rs`
`PageView`). **QuickJS replaces the socket with a channel; nothing else
in that path changes**, and no API gives a script the window's
document.

**QuickJS** (`features/pages/src/js/mod.rs` `JsEngine`, `rquickjs`):
one runtime per page; ES module imports inside the page's folder only
(`./list.tsx`; no packages, nothing outside); `"architect"` is the
runtime (`features/pages/src/js/prelude.js`: `h`, `Fragment`, hooks by
component place, `useData`, `useShared`, timers, `actions`, `theme`,
`worker`, `Overlay`); `console.log` goes to the page's log
(`features/pages/src/runner.rs` `Log`, `PAGES_STATUS`). No Node APIs:
`require`, `process` and `fetch` do not exist; files and the network
only through actions the extension's capabilities reach (its gate,
`features/extensions/src/host.rs` `gate`).

**TSX on save** (`features/pages/src/tsx.rs` `transpile`): oxc parses
`.ts`/`.tsx`/`.jsx`, strips types and turns JSX into `h(...)` as each
module loads — about a millisecond for a page — and a mistake is a
`SourceError` with its file, line and column.

**Source maps.** oxc prints each module with its map
(`transpile_mapped`, `SourceMap`: the printed module's places back to
the file's), and the page's runtime keeps them by module. What a page
throws as it runs — a render, a handler, a timer, a promise job,
`console.error(e)`, a worker — is remapped through them (`remap`)
before it is written anywhere: its stack says `board.tsx:13:3`, not the
transpiled module's line, its first line leads with that place, and the
page's folder's paths are relative. `<file>.error`, `APP_STATUS`'s
problem, the banner and `EXT_CHECK` all say it so
(`features/pages/tests/js.rs`
`a_tsx_pages_runtime_error_says_its_own_line_and_column`).

**Luau** (`features/pages/src/luau.rs` `LuauEngine`): the element
builder on the same edit path, Luau's sandbox, the theme read through.
Its timers are a TSX page's: `set_timeout(fn, ms)`,
`set_interval(fn, ms)`, `clear_timeout(id)`, each run on the page's
thread with a frame after. `worker.spawn("count.luau", args)` runs a
`.luau` file of the page's folder as a Luau view's worker does
(`architect_scripting::worker`, on its pool, `Workers::install_owner`):
its handle has `w:post`, `w:cancel`, `w:status`, `w:progress`,
`w:result`, and callbacks `w:on_message`, `w:on_progress`, `w:on_done`,
which run as the worker's news wakes the page's thread (`deliver`), a
frame after (`features/pages/tests/luau.rs`; the sandbox's
`apps/sandbox/board/pages/luau-primes.luau` with its worker `count.luau`).

**A2UI** (`features/pages/src/a2ui.rs` `A2uiEngine`, feature `a2ui`,
`Pages::with_a2ui`): a `.a2ui.json` page is A2UI v1.0 — the page form
above (`components`, one of them `root`; `data`; `bind`), or the
messages an agent sends (`createSurface`, `updateComponents`,
`updateDataModel`, as `A2UI_RENDER` takes them). It is drawn through
architect's catalog (`features/a2ui/src/catalog.rs` `CATALOG`, the
security boundary: a component outside it is a labelled placeholder,
only its props are read) onto the same `Element` tree, values read
through `a2ui-base`'s data context (paths, templates, its functions).
Each `bind` path holds what its action answers, read as the page loads,
every `every` ms, and after each of its buttons' actions; a Button's
`action` runs its Action List action with its context read through the
data model (the page's capabilities its reach, as a TSX page's
`actions.run`), `wantResponse` / `responsePath` writing the answer
back; a TextField, CheckBox, Slider, ChoicePicker or Tabs bound to a
path writes it as the person changes it. A mistake in the file is said
with its line and column (`summary.a2ui.json:4:5: trailing comma`), and
a page with no `root` says so (`features/pages/tests/a2ui.rs`).

**Reload.** The page's thread looks at its folder every 200 ms
(`WATCH_EVERY`); a save loads the page in a fresh runtime and swaps it
in once it has rendered. One that fails keeps the last good page on
screen under a banner and writes `<file>.error`; the next good save
takes both away. A theme change renders the page again with the new
snapshot (keyed by its revision, as Luau's `ThemeModule`). Measured
in the sandbox: a save shows in about 150–300 ms, of which loading and
rendering the page is 2–3 ms.

## On the web

**Which runner runs where.** Natively (desktop, iOS) a TSX/JS page runs
on QuickJS, a Luau page on Luau, an A2UI page through the catalog, each
on a thread of its own. In a browser (`wasm32-unknown-unknown`) a TSX/JS
page runs in a **Web Worker on the browser's own engine** — no QuickJS —
from the same file, with the same runtime and the same `architect.d.ts`
(`features/pages/src/web/mod.rs` `WebPages`, feature `web`). Luau and
A2UI pages do not run in a browser yet (#187).

**The same runtime, its bridge messages.** The worker runs the prelude
every native page gets (`features/pages/src/js/prelude.js`), its
`__host` made of messages to the main thread
(`features/pages/src/web/page.js`): a host call is a message answered by
one, shared state and the embed's input are kept in the worker (each
change comes from the main thread), `worker.spawn("job.js")` starts a
nested Web Worker with the worker runtime
(`features/pages/src/web/job.js`). The page's folder is given as text
(`PageFolder`: the sandbox's are its files, `include_str!`'d); its scripts
are transpiled by oxc in the main thread's wasm and their imports
resolved by the native loader's rules (`features/pages/src/modules.rs`
`prepare`: `"architect"`, `"architect/dom"`, `"architect/worker"`,
files of its own folder, nothing else), and the worker links them as `blob:` URLs.

**The main thread only draws.** What the worker renders comes back as
the element tree; `Mirror` diffs it into `wire::Edit`s and the
extensions host's checked applier applies them under the pane's element
(`features/extensions/src/web.rs` `Pane`, the one WASI views use: no
`on*` attribute, no `dangerous_inner_html`, nothing outside the pane).
No API gives the page the document. Its events go back to its handlers
as natively (`features/pages/src/frame.rs` `page_event`).

**The same capabilities.** Every host call is answered by the page's
extension's `WebHost` (`features/extensions/src/web.rs`), after the
decision every host makes (`hostcall::check`): `actions.run` is its
`run` (its own actions, and what `capabilities.actions` and `.effect`
allow), `net.fetch` and the interface's calls its host-service calls
(`capabilities.hosts`: a host not named is refused before the browser
fetches), the actions a page lists only those it may run. What a
browser host does not have it says (documents, ports, secrets). The
worker has none of the browser's own network, storage or workers
(`fetch`, `XMLHttpRequest`, `WebSocket`, `importScripts`, `indexedDB`,
`Worker`… are taken away before the page loads), and the embedder's
Content-Security-Policy, which the worker inherits, keeps `import()` of
another host and eval out (the sandbox's: `apps/sandbox/web/index.html`).

**Limits.** A browser has no interrupt to stop one call, so the page's
work is timed in units: a message (an event, a host call's answer,
shared state) or a timer, with the promise jobs and renders it leads to.
The worker says `busy` before one and `idle` after; past `Limits::frame`
(500 ms) the main thread terminates it, the banner says it "ran past its
time of 500 ms: hand heavy work to a worker (worker.spawn)", and the
page starts again from its files (three times, then it stays stopped
until it is reloaded). Loading has `Limits::load` (5 s), a worker
`Limits::worker_time`. The main thread never waits on a page. A browser
cannot cap one worker's memory.

**Errors and reloads.** A page that does not load, or whose render
throws, keeps its last good frame under the banner, its error at its
source's line and column (the worker says its modules' `blob:` URLs as
`/page/<file>`, remapped through oxc's maps by `tsx::remap`).
`WebPage::reload` is a save: the folder loads again in a fresh worker,
swapped in once it has rendered.

**Not on the web yet:** a dynamic `import()` of the page's own files
(only static imports are resolved), inline embeds' input, documents,
ports and secrets (the browser host has none), and services
(`services.call`).

**The sandbox.** `just sandbox-web` builds and serves the web lab
(`apps/sandbox/web`): the Board's own `apps/sandbox/board/pages/board.tsx` (and
`primes.js`, its worker) with the three Board actions it runs answered in
the page (`apps/sandbox/web/src/board.rs`; the Board's Rust extension is not
built for a browser), beside the WASI Counter; `?page=preact` is the
Board's Preact page; `?page=runaway` and `?page=net` are checks. `just sandbox-web-test` drives them in a headless
Chrome (`apps/sandbox/web/test/pages.test.mjs`): the Board renders and its
handlers and worker run, the Preact page answers clicks and typing, a
runaway is stopped while the main thread
keeps drawing, a host outside the page's capabilities is refused, and a
broken save keeps the last good page.

## Safety and performance

- **Limits** (`features/pages/src/runner.rs` `PageLimits`): memory
  (64 MB), and each call into a page — a render, a handler, a timer —
  has `frame` (500 ms) before QuickJS's interrupt stops it with "hand
  heavy work to a worker"; its module's top level has `load` (5 s).
  The pane shows its last frame with the shell's busy mark once an
  event goes unanswered past a frame's budget (`ViewMeter`,
  `FRAME_BUDGET`), long before.
- **Workers**: `worker.spawn("job.js", args)` runs a script of the
  page's folder in a QuickJS runtime of its own on its own thread
  (`worker_memory`, `worker_time`), spoken to by JSON messages; it ends
  with its page (or `w.cancel()`). A Luau page's `worker.spawn("job.luau")`
  runs one in a Luau state on the scripting pool, under the same limits.
  Their names are the other runners' (`extensions.md`, "Responsive by
  design").
- **Actions** a page runs go to the app's Tokio runtime, never the
  page's thread (`features/scripting/src/vm.rs` `Dispatch`, as Luau
  scripts' do).
- **Pages are for panels, lists, forms, dialogs and dashboards.** Dense
  or animated views — the paint kind, Studio's arrangement, a mixer —
  stay compiled: a page's every change crosses as JSON and a diff.

## For agents

`PAGES_STATUS` lists every running page, whether it is the page as
saved (or why not), its render and reload times and its log;
`PAGES_CHECK {file}` loads a page off the window and renders it once;
`PAGES_CLONE {extension, view}` makes a built-in's page editable
(`features/pages/src/actions.rs` `install`). Pages' problems are in
`APP_STATUS`: each page that is not the page as saved — open in a pane,
or its `<file>.error` from its last run — with its file and view
(`features/pages/src/runner.rs` `Pages::status_problems`,
`Pages::report_problems`). `EXT_CHECK` renders an extension's
`[[views]] page` files once through the app's pages (feature `manager`:
`features/pages/src/manage.rs` `Pages::page_checker`, given to
`Host::set_page_checker`), a broken one a `page` problem with its file
and field.

In the extensions manager (feature `manager`) pages are a kind,
`pages` (`features/pages/src/manage.rs` `PagesKind`, over
`Pages::entries`): every view's page — a built-in's read-only copy, an
extension's, an override — with whether an override wins, how it runs,
its problems and its frame times, and the ops `check`, `clone` (a
built-in's, to edit) and `restore` (the override removed: its own page
shows again).

## A framework's own DOM

A page may render with a framework that renders to the DOM — Preact,
Solid — instead of `h`: `import { document } from "architect/dom"` gives
the page a DOM of its own, in the spirit of AMP's worker-dom, in the
page's runtime (`features/pages/src/js/dom.js`): QuickJS natively, the
page's Web Worker in a browser. It is never the window's document. The
framework builds and mutates it; each change marks the page dirty, and
once the unit of work is over (the event, its promise jobs, the
framework's own microtask) the runtime draws it as it draws a
component's tree — every node keyed by its own id, so a node the
framework moves stays itself — and `Mirror` diffs that into
`wire::Edit`s, applied by Blitz natively and by the extensions host's
applier on the web. Nothing on the window's side changes.

```tsx
import { h, render } from "./vendor/preact.js";
import { useState } from "./vendor/hooks.js";
import { document } from "architect/dom";

function Counter() {
  const [n, setN] = useState(0);
  return <button onClick={() => setN(n + 1)}>Clicked {n}</button>;
}

render(<Counter />, document.body);
export default document.body;
```

The page's default export is the body it rendered into (`mount` in
`features/pages/src/js/prelude.js` takes a component or that body). The
framework comes from the page's folder, as any file does — no Node, no
bundler: the sandbox's Board ships Preact 11 in
`apps/sandbox/board/pages/vendor/` (its hooks' `import "preact"` written as
`"./preact.js"`, since a page imports no packages) and its `preact` view
is `apps/sandbox/board/pages/preact.tsx`.

**What it has.** `createElement`/`createElementNS`, `createTextNode`,
`createComment`, `createDocumentFragment`; `appendChild`,
`insertBefore`, `removeChild`, `replaceChild`, `remove`, `append`,
`before`/`after`/`replaceWith`; siblings, `childNodes`, `parentNode`;
attributes (and `id`, `className`, `value`, `checked`, `disabled`… as
properties), `classList`, `dataset`, `style` (properties, `cssText`,
`setProperty`); `textContent`, a text node's `data`;
`addEventListener` (capture, `once`) and `dispatchEvent`;
`querySelector` for compound selectors; `innerHTML` serialized (setting
it only clears: there is no HTML parser). Nothing is laid out:
`getBoundingClientRect` is zero. The prelude gives every page
`requestAnimationFrame` (a 16 ms timer of the page's own) and
`queueMicrotask`, so what a framework schedules runs in the page's units
of work, timed as any.

**Events.** A node with listeners is drawn with handlers for their
events; the pane's event comes back as a `ViewEvent` to the nearest
element that has one (`Mirror::handler`), and the shim dispatches it as
a DOM event through its own tree: captured from the document down, at
the element, bubbled back up, with `stopPropagation`, `preventDefault`,
the pointer's place and the keys. An `input` sets the element's `value`
first and is followed by a `change`. Solid's delegated `$$click` handlers
are drawn as listeners too (Solid is not tested yet).

Tests: `features/pages/tests/dom.rs` (Preact's tree and the edits each
click makes; the DOM by hand: capture and bubbling, a move that keeps
its node), `apps/sandbox/board/tests/page.rs` (the Board's Preact view), and
the web sandbox's `apps/sandbox/web/test/pages.test.mjs` (`?page=preact`, in a
headless Chrome).
