+++
title = "Pages as files"
description = "A view written as a file — TSX or JS on QuickJS, Luau elements — re-rendered live on save, on a thread of its own, drawn as the same edit stream a process view sends; one API defined once in Rust, its TypeScript and Luau types generated from it."
weight = 68
+++

> **Status: landed (2026-11, #273)** in `architect-pages`
> (`features/pages`): the API, the QuickJS and Luau runners, TSX by
> oxc, reload on save with the last good page kept, overrides in the
> app's folder, and the lab's Board tab as a file. Not yet: A2UI JSON
> pages (`PageKind::A2ui` is named but not run), the web build's Web
> Worker runner, and phase two (a worker-side DOM for Preact/Solid).

## The model

A **page** is a view drawn from a file. Every user-facing feature is a
linked extension (#272): its Rust crate is the model — data, actions,
heavy work — and its pages can be files. A page shows instead of the
extension's compiled view when there is one for it, in this order
(`features/pages/src/runner.rs` `Pages::resolve`):

1. an **override** in the app's folder, `<app dir>/pages/<extension>/<view>.tsx`
   (or `.ts`, `.jsx`, `.js`, `.luau`; `pages/<view>.tsx` for short) —
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
both into its folder at start (`write_types`, the seam the generated
guide of #268/#270 calls); the copies in `features/pages/types/` are
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

The lab's Board tab is this, in full: `apps/lab/board/pages/board.tsx`
with its worker `apps/lab/board/pages/primes.js`
(`apps/lab/board/tests/page.rs`). A Luau page builds the same tree:

```lua
local n = 0
render(function()
  return div():style({ padding = 12 }):child(span():child("clicked " .. n))
    :child(button():child("+1"):on_click(function() n += 1 end))
end)
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

**Luau** (`features/pages/src/luau.rs` `LuauEngine`): the element
builder on the same edit path, Luau's sandbox, the theme read through.

**Reload.** The page's thread looks at its folder every 200 ms
(`WATCH_EVERY`); a save loads the page in a fresh runtime and swaps it
in once it has rendered. One that fails keeps the last good page on
screen under a banner and writes `<file>.error`; the next good save
takes both away. A theme change renders the page again with the new
snapshot (keyed by its revision, as Luau's `ThemeModule`). Measured
in the lab: a save shows in about 150–300 ms, of which loading and
rendering the page is 2–3 ms.

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
  with its page.
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
(`features/pages/src/actions.rs` `install`). Pages' problems are what
`APP_STATUS` (#270) lists for them (`Pages::problems`), and
`PAGES_CHECK` is `EXT_CHECK`'s page half (`Pages::check`).

## Phase two

A worker-side DOM (in the spirit of worker-dom) would let Preact or
Solid run unchanged in a page's runtime: their DOM calls would build
the same `Element` tree `h` does, diffed into edits by the same
`Mirror`, so nothing on the window's side changes.
