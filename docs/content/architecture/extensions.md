+++
title = "Extensions and scripts"
description = "Adding to an app at runtime, without rebuilding it: data-only extensions, WASM components over a versioned WIT world drawn from the Action List, Luau scripts for macros, out-of-process plugins on a desk — with capabilities, and views as frame documents."
weight = 67
+++

> **Status: scripts landed (2026-11-01)** in `architect-scripting`
> (`features/scripting`): the Luau runtime over the Action List
> (`features/scripting/src/luau.rs` `Runtime`), a run as one undo step
> (`Transactions`; over `crdt::History` with `HistoryTransactions`),
> saved scripts per person and per organization as actions
> (`features/scripting/src/actions.rs` `Scripting`), and the `SCRIPT_RUN`,
> `SCRIPT_SAVE` and `SCRIPT_DESCRIBE` actions a model reaches over MCP.
> Not yet: dry runs, the typed model services in the Luau library,
> streams, Luau on the web.
>
> **The runtime landed (2026-11-01)** in `architect-extensions`
> (`features/extensions`): the manifest (`features/extensions/src/manifest.rs`
> `Manifest`), the host that loads and reloads extensions as a diff onto
> the Action List and the settings page (`features/extensions/src/host.rs`
> `Host`, with data and Luau runners and `Runner` for the rest), git and
> folder registries and ReaPack indexes (`features/extensions/src/install.rs`),
> built-ins overridden by newer installs and development copies
> (`features/extensions/src/catalog.rs` `choose`), hot reload by polling
> (`features/extensions/src/watch.rs` `Watch`), a project's requirements
> and an organization's approved list (`features/extensions/src/project.rs`
> `plan`), and the `EXT_*` actions. Not yet: overlays, signed
> archives, `architect ext dev` as a command.
>
> **Processes and remote views landed (2026-11-01)** in the same crate:
> `process` runs process extensions (`features/extensions/src/process.rs`
> `ProcessRunner`: started with a local socket in their environment,
> spoken to over vox through one `#[architect::rpc]` trait,
> `features/extensions/src/proto.rs` `ProcessExtension`; a hello that
> must cover the manifest; supervised, restarted with backoff, saying why
> it died); `guest` is a process extension's `main`
> (`features/extensions/src/guest.rs` `Guest`: each view a `VirtualDom` on
> its own thread, its mutations written as the wire's edits by
> `WireWriter`); `remote` mounts a view in a pane
> (`app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteView`, a Blitz sub-document
> filled by `RemoteDocument`, which checks every edit, and a placeholder
> when the process is gone). The wire is architect's own, versioned
> (`features/extensions/src/wire.rs` `WIRE`).
> `app-shell:crates/app-shell-extensions/tests/remote_view.rs` clicks
> through a real child process and kills it. Not yet: stdio, lazy start
> and idle stop, state handed over on a reload, the host service an
> extension calls back into, pointer, wheel and IME events, the
> allowlist, Windows.
> **Contributions, hooks and in-process extensions landed (2026-11-03)**
> in the same crate. An extension adds to the interface past its actions,
> settings and themes — openers for files, document kinds and links,
> document kinds, tabs, status items, menu and context-menu entries, a
> phone's bar buttons, hooks, and the shell's slots it fills
> (`features/extensions/src/contrib.rs` `Contribution`, read into
> `Manifest::contributes`); the host keeps them by owner
> (`Contributions`), as a diff on a reload and withdrawn on unload, and
> runs the hooks (`features/extensions/src/host.rs` `Host::emit`,
> `Host::watch`). Actions are told where they were invoked from
> (`crates/architect/src/action_list.rs` `Invocation`). One Rust value
> runs linked in or as a process (`features/extensions/src/linked.rs`
> `Extension`). The window side is `app-shell:crates/app-shell-extensions/src/interface.rs`
> `use_extension_ui`. See
> "Contributions" and "One trait, three ways to run it" below. Not yet:
> WASM, hooks on a phone.
>
> The rest of this page is proposed (2026-10-31). Decided: one crate, `architect-extensions`, with four kinds — data,
> WebAssembly components, Luau scripts, and processes over vox — and a
> git-based registry. Proposed for UI: Dioxus views rendered remotely (the
> extension's `VirtualDom`, the host's Blitz), frame documents for faces.
> [Extensibility](@/architecture/extensibility.md) covers adding a feature
> at build time (a crate); this page is adding one at runtime.

A person, or anyone else, should be able to add to an app what the app's
own features add — actions, conditional macros, views, settings sections,
openers, themes, keymaps — without the app being rebuilt, and without
being able to do more than they were allowed. Rust can do it, and
shipping apps do: Zed's extensions are WASM components against a
versioned WIT API; Yazi's plugins and WezTerm's config are Lua; Nushell's
plugins are processes speaking a protocol. What none of them gives a
plugin is UI, and that is the part architect adds, through frame
documents.

## The layers

| Layer | What | Where it runs | For |
|---|---|---|---|
| 1. **Data** | a manifest and files: themes, keymaps, screensets, settings defaults, frame views with no logic, action presets | everywhere, iOS included | most customisation |
| 2. **WASM components** | code against `architect:extension`, a WIT world generated from the Action List and the services | Wasmtime: compiled on desk and Android, its interpreter (Pulley) on iOS; the browser's WebAssembly on the web | third-party features |
| 3. **Scripts** | Luau (through mlua): query, branch, loop, run actions | everywhere (interpreted) | a person's own macros and conditional actions |
| 3b. **TypeScript** | `kind = "typescript"`: actions and hooks as `.ts`/`.js` modules beside TSX pages, on the pages' runtime with the whole host service, typed by the generated `architect.d.ts`; its runtime went with the pages crate, so an app brings one | `QuickJS` on desk and iOS (interpreted); a Web Worker on the browser's engine on the web | light extensions with no Rust and no WASM toolchain |
| 4. **Processes** | a program speaking vox, registering actions as a client does | desk only | heavy or other-language tools (Python, a model, an MCP server) |
| 5. **Native** | a dynamic library | first-party only | what must be native and is ours (CLAP/VST3 already covers audio) |

Native libraries are not a plugin format: Rust has no stable ABI (Bevy
removed its dynamic plugins for that), they run unsandboxed, and iOS and
Google Play forbid downloading them.

## The API is the Action List

An extension sees the app the way a model over MCP does: through its
actions. The WIT world is generated from the
[Action List](@/architecture/commands.md) and the `#[architect::rpc]`
services, so the API is the app's, typed, and grows with it.

<!-- doc-check: off (proposed names) -->
```wit
package architect:extension@0.1.0;

world extension {
  // What the app gives it (each behind a capability).
  import actions;   // list, describe, run, enabled, checked, watch
  import settings;  // its own section, and reading the app's it may
  import state;     // read-only queries the app exposes as actions
  import http;      // to the hosts its manifest names
  import fs;        // its own folder, and what the person grants
  import log;

  // What it adds.
  export register: func() -> contributions;        // actions, views, openers, settings sections, keymap
  export run-action: func(id: string, args: string) -> result<string, string>;
  export is-enabled: func(id: string) -> result<_, string>;
  export open: func(opener: string, query: string) -> list<item>;
  export view-event: func(view: string, event: string) -> view-state;
}
```
<!-- doc-check: on -->

Old versions of the world stay loadable (Zed's `since_vN`), so an
extension keeps working across app updates; an action an extension
relies on is now public API, and breaking it breaks the ecosystem —
the Action List's ids are versioned like any API.

## Actions, the model, scripts

Three different things, which an extension and a script both use:

- **Actions** are the app's verbs, for a person: things to trigger with
  little or no context (Arm track, Recall screenset, Split at cursor).
  They are what keys, menus, ⌘K and the ribbon run
  ([actions everywhere](@/architecture/actions-everywhere.md)), and they
  stay coarse on purpose.
- **The model API** is everything underneath, fine-grained and typed:
  read and change tracks, items, takes, FX parameters, envelopes, MIDI,
  markers, the tempo map — REAPER's 878 getters and setters. It is the
  app's `#[architect::rpc]` services, generated into the WIT world and the
  Luau library, so a script or an extension never depends on how the
  actions happen to be cut.
- **Scripts** are programs over both: they query the model, change it,
  run actions, loop, branch, wait on streams. A script saved under a name
  becomes an action itself.

**Changes are transactions, on Loro.** The model's documents are Loro
documents ([collaboration](@/architecture/collaboration.md)), so a
script's or an extension's changes are a Loro transaction: committed
together as one undo step (a script is one undo step unless it says
otherwise), aborted together if it fails, and merged like anyone else's
edits when a collaborator is changing the same song.

**An extension's own data lives in the project, on Loro.** Each
extension gets a container in the project's document, namespaced by its
id — for the project, and for any object in it (a track, an item, a
section; REAPER's `ProjExtState` and `P_EXT`) — saved with the project,
part of undo, synced with it, and readable by no other extension unless
it grants it.

## An agent writes scripts

A model connected over MCP gets what a script gets — the model API, the
actions, the streams — through three tools:

| Tool | What it does |
|---|---|
| `script_run { source, args?, name? }` | runs a one-off Luau script; returns its result, its log, the actions it ran and the undo step it made (proposed: `dry_run`, making the changes in a scratch transaction and returning the diff instead) |
| `script_save { name, source, description? }` | saves it as a script action: in the action list, ⌘K, bindable, pinnable (`script_save_org` for the organization) |
| `script_describe` | the API it can call: every action it may run, with schemas — generated, so always current (proposed: the model services too) |

They are actions (`SCRIPT_RUN`, `SCRIPT_SAVE`, `SCRIPT_DESCRIBE`, with
`SCRIPT_LIST` and `SCRIPT_DELETE`), so a model reaches them as tools of
their own under `Tools::Named` or through the generic `run`, and a
person's keys and ⌘K reach them too.

The agent's script runs with the capabilities the person granted that
agent (read-only, or allowed to change things, or destructive too), as
one transaction, so "undo" undoes all of what it did, and the person can
see the script before saving it. Every action a script runs is
authorized for whoever ran the script, one by one; a script that fails
says the line it stopped on, and what it changed is taken back.

## Scripts and conditional actions

A script is an action: it has an id, a name, a binding, a place in ⌘K.
It is Luau, sandboxed, with the app's actions as its library:

<!-- doc-check: off -->
```lua
-- "Arm the selected tracks, or all if none are selected"
local selected = state.query("session.tracks.selected")
if #selected == 0 then
  actions.run("session.tracks.select_all")
end
actions.run("session.tracks.arm", { on = true })
```
<!-- doc-check: on -->

The macro editor of the [action list window](@/architecture/actions-everywhere.md)
writes the simple ones (steps, conditions, repeats); a script is for
the rest. Scripts run interpreted, so they work on iOS and the web.
(Rhai is the pure-Rust alternative, if a C toolchain on iOS or wasm is a
problem; Luau has the better sandbox and is a language people know.)

## Views: Dioxus, rendered remotely; frame documents for faces

An extension writes its views in Dioxus, like the app. It runs its own
`VirtualDom` (inside its WASM component, or its process); what that
renders crosses the boundary as Dioxus's own edit stream — since Dioxus
0.8's buffered writer, a stack machine (push a node by its element id,
create, clone, append, replace, set an attribute, register an id) — and
the host applies it into a document of its own in Blitz, the same
machine the app's own writer runs. Events go back the other way as
dioxus-html's serialised event data, to the extension's handlers.
Dioxus LiveView already works this way across a network; here the
boundary is a function call or a local socket, well under a millisecond.

- **Where it mounts:** a Blitz sub-document (Blitz's own iframe: it has
  its own ids, its own styles, is laid out, painted and given events by
  the host document) sized by the pane it is in; inside a document, as
  an inline embed, the same sub-document sized to its content ("Inline
  embeds" below).
- **What it may use:** an allowlist, as Shopify's remote UI keeps one —
  the elements and CSS properties Blitz draws, no raw HTML, no custom
  widgets; the shell's components (buttons, rows, menus) as named
  elements the host maps onto its own, so an extension's view looks like
  the app's.
- **What crosses the boundary per frame:** only edits and events.
  Pointer-move and input events are coalesced; `mounted` and size
  queries are asynchronous.
- **Why not native code:** subsecond (Dioxus hot-patching) only patches
  the binary it was built from; a dylib exporting components needs the
  identical compiler and Dioxus build, shares the runtime unsandboxed,
  and iOS forbids it.

**Frame documents** stay for what has no code: a face, a control
surface, a data-only extension's panel — bound to parameters and actions,
rendered by the app.

**Process views** are for what draws on a GPU of its own: a Bevy room, a
game engine, a renderer in another language (#838). A `[[views]] kind =
"process"` names a `program` in the package. The app's process-view pane
runs it, shows its frames (a shared dma-buf texture, or shared memory)
and forwards the pane's input and size, whichever copy of the extension
is loaded: built in, installed or in development. Any kind of extension
may declare one. Running the program is a capability
(`capabilities.programs`, below), and the view is Linux only for now;
elsewhere the extension loads and its pane says why
(`docs/content/architecture/process-views.md`,
`features/extensions/src/process_view.rs`).

### Spike

1. **In process:** a second Dioxus document as a Blitz sub-document of a
   host `<div>`, polled by the host — proves paint, layout and events.
   (About three days.)
2. **Through a channel:** the second `VirtualDom` writes a wire format;
   a host `RemoteDocument` applies it; events go back serialised.
   Measure bytes per frame and the round trip of typing into a field.
   (About a week.)
3. **In a WASM component** under Wasmtime on a worker thread: `init`,
   `tick(events) -> edits`, `resize`; host imports for time and the
   app's actions; an epoch budget per tick and a memory cap (Zed's
   `embedded_gpui` spike is the precedent).
4. **Across a process** (vox): the same wire, the same `tick`.

## WASI components

> **Landed (2026-11-03, #167 parts 1 and 2)** behind the `wasi` feature
> of `architect-extensions`, desktop and Android: Wasmtime with the
> component model and Cranelift, and workers (more instances of a
> component on a pool). **iOS (2026-11-04, #167 part 3)**: the same
> runner, workers too, on Wasmtime's Pulley interpreter. **The web
> (2026-11-04, the rest of part 3)** behind the `web` feature: the same
> components, transpiled by jco, in Web Workers ("On the web" below).
> **In the release builds (#707):** `cargo xtask dist desktop` and
> `dist ios` build the app with `wasi` (Cranelift on the desk, Pulley
> on iOS), so an installed component runs in the shipped app;
> `--no-wasi` leaves Wasmtime out.

A `kind = "wasm"` extension is a WebAssembly component against the
`architect:extension` world (`features/extensions/wit/extension.wit`),
run by `features/extensions/src/wasi.rs` `WasiRunner`. Its manifest
names the component:

```toml
kind = "wasm"
[wasm]
component = "my_extension.wasm"
```

**The world.** It imports `host` and `worker`, and exports `guest` and
`work`:

| | | |
|---|---|---|
| import | `run-action`, `setting`, `set-setting`, `log` | the host service's `run`, `setting`, `set_setting`, `log` |
| import | `call` | any `HostRequest` as JSON, its `HostAnswer` back (`org`, `selection`, files, data) |
| import | `theme` | the app's theme as data (`wire::ThemeData`) |
| export | `init`, `actions`, `views` | starts it (with what the copy it replaces saved); what it runs and renders, checked against its manifest |
| export | `invoke` | runs an action: JSON args, the invocation as JSON |
| export | `render`, `on-event`, `pending`, `close` | a view: its first edits, its handlers' edits, what changed after an action, unmount |
| export | `save-state` | its state for a reload |
| import | `spawn`, `post`, `cancel`, `progress`, `cancelled`, `receive` | its workers ("Workers" below) |
| export | `run`, `on-message`, `on-progress`, `on-done` | a worker's run; its owner's news |

A view's edits are the remote-view wire's (`wire::Edit`, the world's
`edit`), so the same `RemoteDocument` applies them: the runner puts each
component among the app's processes as a local one
(`WasiRunner::with_processes`, `process::Process::is_local`), and
`RemoteView`, `ExtensionView` and the panes that show a process's view
show a component's unchanged.

The WIT is written by hand, not generated from the vox descriptors: the
host service is one vox method carrying JSON (`host_proto::ExtensionHost`
`call`), so its descriptor says nothing a WIT could be made from, and the
calls themselves (`hostcall::HostRequest`) are serde types, not facet
ones. `features/extensions/tests/wasi_wit.rs` keeps it from drifting: the
world's types are the wire's (each case and field, from their facet
shapes), its typed imports are host-service ops, and every method of the
process protocol's descriptor (`proto::ProcessExtension`) has its place
in the world or is named as not yet a component's (`updates`).

**Capabilities.** Every import goes through the extension's own
`hostapi::ExtensionApi`, acting for whoever ran the action, so a
component is checked exactly as a process, a Luau script and a linked
extension are: its manifest's capabilities, narrowed by what a person
granted (`consent`), at the moment it asks. A refused call answers
`HostError::Refused` as JSON and is a line in its log. WASI 0.2 is
linked with nothing to reach: no preopened folders, no environment, no
network; its stdout and stderr are lines in its log (`EXT_LOGS`). A
component that imports `wasi:sockets` or `wasi:http` is refused before
it is linked: no capability grants the network. An extension's own
actions are refused through `run-action` (the call would wait for
itself).

**Limits** (`wasi::Limits`, per runner):

- **memory** (64 MiB by default): a `ResourceLimiter` refuses growth
  past it and the call traps, `out of memory: it asked for N MiB, past
  its limit of M MiB`;
- **time** (5 s a call by default): one engine ticks its epoch every
  10 ms; at each tick a running call yields to its thread's executor,
  and one past its limit of its own time (waiting for the host does not
  count) traps, `interrupted: it ran for N ms`.

There are no threads in a component: each instance is a `Store` on a
thread of its own, an actor its calls queue for, so an action, an event
and a render never meet. A trap ends the instance (a component is not
entered again after one): the call fails saying why, its views say it
stopped and mount again, and it is started again fresh; `Stats::traps`
and `last_trap` say so, the manager's `component` kind lists them.
Unloading it ends its thread and frees its store.

**The compile cache.** `WasiRunner::with_cache(dir)` keeps each compiled
component as `<sha256>.cwasm`, keyed by the component's bytes and the
engine's settings; the next start deserializes it instead of compiling.
The extension's log says which (`compiled in N ms`, `loaded from the
compile cache`).

**A Rust guest.** `architect-extension-wasi`
(`features/extensions/wasi-guest`) is the guest side, the shape of a
process's `Guest`: views are Dioxus components (a `VirtualDom` per open
view, its mutations the world's edits), actions take and give JSON, and
`host::{run, setting, set_setting, theme, log, call}` reach the app. An
action that changes what a view shows calls `refresh`. Actions are
synchronous (no async in a component before WASI 0.3).

```rust
fn build() -> architect_extension_wasi::Extension {
    architect_extension_wasi::Extension::new()
        .view("counter", Counter)
        .action("ACME_COUNTER_ASK", |_| {
            architect_extension_wasi::host::run("THEME_LIST", &serde_json::Value::Null)
                .map_err(|e| e.to_string())
        })
}

architect_extension_wasi::export!(build);
```

The crate is a `cdylib` built with
`cargo build --target wasm32-wasip2` in `devenv --profile wasi shell`
(the fleet toolchain plus that target: a profile, as an extra target
changes the rustc path kache shares). The tests' WASI Counter
(`features/extensions/tests/wasi-counter`) is one: a counter view and an
action that asks the app for its themes.
`features/extensions/tests/wasi.rs` builds it and a component that
misbehaves on purpose (`features/extensions/tests/wasi-hazards`) and
checks the view, a refused call, both limits, unload and the cache.

**Workers.** There are no threads in a component: its heavy work goes
to a worker, another instance of the same component
(`features/extensions/src/wasi/workers.rs`). A `[[workers]]` entry of a
`wasm` package names what it runs: an `entry` of the component itself
(the name its `work.run` is given), or a separate component in the
package (`entry = "workers/primes.wasm"`, given the worker's id):

```toml
[[workers]]
id = "primes"
entry = "count-primes"
```

Its owner (the extension's instance, from a handler or an action)
spawns it; the runner instantiates the compiled component again, its
code shared from the compile cache, on a thread of its `wasi::Pool`, with
a memory limit and an epoch deadline of its own
(`WasiRunner::with_worker_limits`; `Limits::worker`: 64 MiB and 60 s of
its own time for the whole run). The two share nothing but JSON
messages, in the same API the other runners have
(`features/extensions/tests/wasi_wit.rs` checks the names):

| the owner | the worker | Luau / process |
|---|---|---|
| `spawn(name, args)`: a handle | `run(entry, worker, args)`: its result | `worker.spawn`, `spawn_work` |
| `post(w, msg)` | `post(0, msg)` | `w:post`, `Work::post` |
| `cancel(w)` | `cancelled()`, `receive(ms)` | `w:cancel`, `worker.cancelled`, `worker.receive` |
| `on-message`, `on-progress`, `on-done` | `progress(done, total, note)` | `worker.on_message`, `on_progress`, `on_done`, `Work::progress` |

- **News** comes into the owner between its own calls, at most once
  every 16 ms (`wasi::NEWS_EVERY`): every message in order, progress
  coalesced to the latest, the end once; then its views' changes. A
  click on its view waits behind one round of news at most.
- **Back-pressure**: each way a queue holds 64 messages (`wasi::QUEUE`).
  An owner's `post` to a full one fails at once (a view never waits); a
  worker's waits (that time is the host's, not counted against it). An
  instance has at most 8 workers running or waiting for the pool
  (`wasi::MAX_WORKERS`).
- **Cancellation**: `cancel`, its owner trapping (it starts again
  fresh, its workers do not follow it) and unloading stop a worker at
  its next epoch tick, whether it looks at `cancelled()` or not.
- **Limits**: a worker past its memory or time traps alone; its owner
  hears `failed` and why, and runs on.
- **The pool** (`wasi::Pool`, `WasiRunner::with_pool`; one thread fewer
  than the machine has, at most four): threads start as workers are
  spawned and stop after 30 s idle. `Stats::workers` and
  `Stats::workers_alive` say what runs; the manager's `component` kind
  shows how many.
- A worker reaches its owner only: its `host` imports other than `log`
  are refused.

In the Rust guest, `Extension::worker(entry, |work, args| …)` is the
worker's function and `worker::spawn(name, args)` gives a `Worker`
(dropping it cancels it); `Work` is the worker's side (`progress`,
`post`, `receive`, `cancelled`, `check`) and `WorkState` the owner's
(`status`, `progress`, `messages`, `result`, `fraction`), the same as a
process's (`features/extensions/src/work.rs`). News updates the state
and calls `refresh`, so views read it as they render:

```rust
Extension::new()
    .action("ACME_PRIMES", |args| {
        let w = worker::spawn("primes", &args)?;   // keep `w`: dropping it cancels
        Ok(json!({ "worker": w.id() }))
    })
    .worker("count-primes", |work, args| {
        for i in 0..n { work.check()?; work.progress(i as f64, n as f64, ""); /* … */ }
        Ok(json!(found))
    })
```

The sandbox's WASI Counter counts primes in a worker (`LAB_WASI_COUNTER_PRIMES`,
`LAB_WASI_COUNTER_JOB`, its "Count primes" button) with a progress bar,
while the counter answers clicks. `features/extensions/tests/wasi.rs`
measures that (`a_worker_job_keeps_the_view_within_its_frame_budget`:
each click's time from its event to the frame that shows it, into a
`ViewMeter` with `REMOTE_BUDGET`; p99 about 4 ms in a debug build with
the job running), and checks cancellation on unload, both limits,
back-pressure both ways and the pool.

**Its actions on the app's own list.** A host puts an extension's
actions on its list (the extensions' own, which ⌘K reads among the
app's lists). An app whose control socket, CLI and MCP serve another
list forwards them there with `Host::forward_actions_to`
(`features/extensions/src/forward.rs`), as extensions load, reload and
unload; each runs on the host's list as whoever ran it. The sandbox does,
so `architect-sandbox actions run LAB_WASI_COUNTER_ADD` reaches the
counter.

**iOS: Pulley, no JIT.** iOS lets no app make memory executable, so
there `wasi::PULLEY` is on and the engine (`features/extensions/src/wasi.rs`
`config`) is Wasmtime's portable interpreter instead of native code:

- Cranelift still compiles each component, to Pulley bytecode
  (`Config::target("pulley64")`), which is data, not code; the compile
  cache keeps it as it keeps native code (the engine's settings are in
  the cache key, so a Pulley `.cwasm` and a native one never mix).
  Compiling on the phone, once, is what the cache is for; precompiling
  on the Mac (`Engine::precompile_component` with the same settings)
  would only save that first start;
- traps are found by the interpreter, not by signals
  (`signals_based_traps(false)`);
- each linear memory reserves `wasi::PULLEY_RESERVATION` (64 MiB, the
  default memory limit) with no guard, where native code reserves
  4 GiB and a guard so its loads need no bounds check: an iOS app has a
  few GiB of address space in all, and Pulley checks each access itself;
- the limits are the same code: the epoch still ticks every 10 ms and
  the interpreter checks it as native code does, and the
  `ResourceLimiter` is the store's, not the engine's. Every test in
  `features/extensions/tests/wasi.rs` passes on Pulley too
  (`cargo test -p architect-extensions --features pulley --test wasi`:
  the feature `pulley` makes the interpreter the engine anywhere), and
  `the_engine_is_the_platforms` checks which one ran. The same test
  binary built for `aarch64-apple-ios-sim` passes in the Simulator
  (`xcrun simctl spawn <udid> <binary>`, `ARCHITECT_WASI_GUESTS` passed
  as `SIMCTL_CHILD_ARCHITECT_WASI_GUESTS`).

A call costs more interpreted. `an_action_calls_latency` (ignored; run it
with `--ignored --nocapture`) times `LAB_WASI_COUNTER_ADD` through the
app's list into the component and back, 2000 times, on an M4 Mac mini
in the dev profile:

| Engine | p50 | p90 | p99 | compile + load |
|---|---|---|---|---|
| Cranelift, native (macOS) | 11.6 µs | 12.8 µs | 13.9 µs | 156 ms |
| Pulley, interpreted (macOS, `pulley`) | 32.5 µs | 34.5 µs | 44.5 µs | 161 ms |
| Pulley, interpreted (iOS 27 Simulator, the test binary under `xcrun simctl spawn`) | 36.8 µs | 39.8 µs | 44.7 µs | 168 ms |

About three times a native call, and still far inside a frame:
an extension's action or a view's event is not where the time goes. A
component doing real work in a loop pays more (an interpreter is
typically several times slower on hot code); that is what workers are
for, and they run on the same engine: on the Simulator, a
"Count primes" worker counts the 148933 primes below two million in
about 13 s while its view answers each click.

**Size and memory.** Wasmtime and Cranelift add about 19 MB to a
stripped `architect-desktop` built with the dev profile (opt-level 1).
In a stripped release build (#707, measured 2026-11-14 on x86-64 Linux
with a probe that links Wasmtime 50.0.1 with this crate's features, an
engine, WASI p2 on a linker and a component compile) they add 15.9 MB;
with Pulley as the target, as on iOS, 16.0 MB. Making the engine and
the linker costs about 6 MB resident either way. Each running
instance's linear memory then reserves address space: 4 GiB and a guard
under Cranelift (virtual, not resident), `PULLEY_RESERVATION` (64 MiB)
under Pulley; what it touches is what it uses, up to its limit. The
release builds have the feature on (`cargo xtask dist`); the crate
keeps it off by default.

### On the web

Wasmtime does not run in a browser (`wasm32-unknown-unknown` has no
JIT to embed), so the browser's own WebAssembly runs the component
(`features/extensions/src/web.rs`, feature `web`). The same component
file the desktop runs is transpiled ahead of time by
[jco](https://github.com/bytecodealliance/jco) into core wasm modules
and an `instantiate` function:

```sh
node features/extensions/web/transpile.mjs my_extension.wasm out/   # jco pinned in features/extensions/web/package.json
```

(`architect-ext build-web`, from the npm package
`@fasttrackstudio/architect-cli`, runs the same transpile and writes the
extension's metadata, a lock of every file's SHA-256 and typed actions
beside it.)

**Why jco.** It is the Bytecode Alliance's component-model
implementation for JS: it lifts and lowers every WIT type the world uses
(variants, records, lists, results), so the browser runs the component
the desktop runs, unchanged, with no second build. The alternatives were
worse: a component runtime compiled to wasm (Wasmtime has no
interpreter for `wasm32-unknown-unknown`; Pulley in a browser would be
an interpreter inside the browser's JIT, many times slower), or a
second guest build for `wasm32-unknown-unknown` with wasm-bindgen
(another ABI to keep beside the WIT, and not the same artifact). jco
runs at build time only: the page ships its output, no node. Its
`--async-mode jspi` makes the imports that wait for the host
(`run-action`, `setting`, `set-setting`, `call`, `spawn`, `post`,
`receive`, a sleep) suspend the component with JS Promise Integration
until the page answers, so the guest stays synchronous, as natively.
JSPI ships in Chromium browsers (137 on), in Firefox (checked here on
159) and in node 24; where it is missing (Safari, for now) the component
does not start, and `WebComponent::start` says why.

**Where it runs.** Each instance is a Web Worker
(`features/extensions/web/instance.js`), started inside a sandbox: a
hidden `<iframe sandbox="allow-scripts">` with no `allow-same-origin`,
an opaque origin with its own CSP (`runner.js` `Sandbox`,
`SANDBOX_CSP`). An extension's JS (jco's glue ships with it) never runs
with the page's origin, so it gets none of the page's cookies, storage,
channels or network. The page fetches the files and hands in the glue as
text and the core modules as bytes; a page with a CSP names the frame's
script by its hash (`SANDBOX_HASH`). Under
node there is no frame, so a node host trusts the extension's JS. The page's side
(`features/extensions/web/runner.js`, linked in by wasm-bindgen) is the
native actor: one call at a time, a trap restarting it fresh, its views'
panes told. `web::WebComponent` drives it from Rust: `start` (the
transpiled module's URL and a `web::WebHost`), `register_actions` (onto
the app's `ActionList`, run as whoever ran them), `mount` (a view under
a DOM element: the wire's `Edit`s applied to the page's DOM, its events
sent back as `ViewEvent`s, as `RemoteDocument` does in Blitz), `report`
and `stop`. That applier is `web::Pane`, public: a TSX page in a
browser is drawn by it too, its host calls answered by a `WebHost`
(`pages.md`, "On the web").

**Capabilities.** Every host import arrives at `web::WebHost` as a
host-service request (`hostcall::HostRequest`), decided by
`hostcall::check`, the same function `hostapi::ExtensionApi` decides
with natively: its manifest's capabilities within what a person granted
(`WebHost::with_granted`), its own actions refused through
`run-action`, a worker's host calls refused. App actions run on the
page's list as `Principal::Extension`. What a browser host lacks it says
so after the check: no documents or files (`open`, `read_document`,
`read_file`, `write_file` fail), settings and data in memory. WASI 0.2
is linked with nothing to reach (no folders, environment or network;
stdout and stderr lines in its log), and an import of `wasi:sockets` or
`wasi:http` is refused.

**Limits** (`web::Limits`, the native defaults), and how they differ:

| | native (`wasi`) | browser (`web`) |
|---|---|---|
| memory | a `ResourceLimiter` refuses growth past the limit | the limit is written into each core module's memory as its maximum (`runner.js` `limitMemory`), so `memory.grow` fails past it; the guest's allocator then aborts and the instance is ended, `out of memory: …` |
| time | an epoch tick every 10 ms; past its limit the call traps | no epoch interruption: the page times each call (its clock stops while the page answers a host import) and terminates the Web Worker past its limit, `interrupted: it ran for N ms, past its limit of M ms` |
| after either | the instance is dropped and started again | the Web Worker is terminated and a new one started (its compiled modules reused) |

A call over its time is stopped within a timer's precision of its
deadline, not at a 10 ms tick, and because a Web Worker is the sandbox
that is stopped, a stuck call costs the page nothing while it runs.

**Workers** are Web Workers too, each another instance of the
component (or of a separate component, `Options::worker_urls`), on a
pool of at most four running (one fewer than the machine's cores), the
rest waiting. The messages are the same:

- **Back-pressure** is counted by the page, since a browser's
  `postMessage` never blocks: each queue holds 64 messages (`QUEUE`, as
  natively). An owner's `post` to a full one fails at once; a worker's
  `post` to its owner's full queue suspends it (JSPI) until the owner
  hears one, which is the native semantics: it waits, and that time is
  not counted against it.
- **News** reaches the owner between its calls at most every 16 ms,
  messages in order, progress coalesced, the end once; at most 8 workers
  running or waiting.
- **Cancellation**: `cancel`, its owner trapping and unloading
  terminate the worker's Web Worker at once, whether it looks at
  `cancelled()` or not (natively: at its next epoch tick).

**Tests.** `features/extensions/web/test/web.test.mjs` runs the
transpiled components under node (24, JSPI on), each instance in a
worker thread standing in for a Web Worker: the view and a click, the
host service and a refused call, both limits, workers (a job beside a
responsive owner, cancellation, limits alone, both queues'
back-pressure, `MAX_WORKERS`). `features/extensions/tests/wasi_web.rs`
runs it from `cargo test` (node from `devenv --profile wasi shell`; it
says so and passes without node, `ARCHITECT_WASI_WEB_REQUIRED=1` makes
that a failure).

**In someone else's page.** The same runner is the npm package
`@fasttrackstudio/architect` (`features/extensions/web` is the package;
`features/extensions/web/cli` is `@fasttrackstudio/architect-cli`). Any
TypeScript app loads a component with it, pinned by its lock, runs its
actions with types generated from its manifest, and mounts its views in
a shadow root. Its host is the embedder's, deny by default. See
[Extensions in your own web app](@/guide/extensions-in-your-web-app.md).

**Next.** WASI 0.3's async (`component-model-async`, which also lets
jco drop JSPI) and what a component adds at run time (`updates`). Its
views' timers landed with #308 ("Timers, events and background
extensions").

### Linked extensions in the browser

An app's web build compiles in the extensions it links, as its desktop
and iOS builds do (#495), and runs the same host over the page's Action
List (`Host`, `Extensions::with_linked`, and the window side's
`use_extension_ui` in `app-shell:crates/app-shell-extensions/src/interface.rs`),
so a linked extension's actions are in ⌘K, its settings sections on the
settings page and its views among the workspace's kinds, drawn as Dioxus
components in the app's own document. A share link's guest runs the same
extensions under the guest profile ([below](#a-share-link-s-guests)).

What changes in a browser:

- **No Luau.** Luau is C++, which `wasm32-unknown-unknown` has no
  library for, so `host` builds there without its Luau runner
  (`features/extensions/src/host.rs` `default_runners`): data and linked
  extensions run, a `luau` one does not start.
- **`linked-core`.** A linked extension's crate depends on
  `architect-extensions` with `linked-core` (the trait and its `Builder`,
  no host) and adds `guest` (vox, tokio: its process copy) only off the
  browser, by target.
- **Not `Send`.** In a browser architect's `ActionFuture` is local, so
  the linked API's `HostFuture` and `ActionFuture` are too, and an
  action's future is `MaybeSend` (`features/extensions/src/linked.rs`).
  Work no one awaits runs on the page's thread (`Host::with_spawn`,
  `spawn_local`).
- **No threads or files.** The host's clock and event threads are not
  started, nothing is installed from a folder, and secrets stay in
  memory.

`cargo xtask dist web` builds a distribution crate's own wasm when it has
a `web` feature (`xtask/src/dist.rs` `build_web`).

### A share link's guests

A share link is the app opened as a guest of that link
(`Principal::Guest`): the same app, views and extensions, with some
permissions off. There is
no guest renderer and no guest extension set; what a guest can't do is a
capability it isn't granted.

A share link's guests run extensions, scoped to the share (#495 G1;
the model is #495's G1 comment). What an extension may do for a guest is
the install workstream's guest profile, one function every host uses:
`share_needs::guest_capabilities` (#755: the share's documents read-only,
no writes, no secrets, `fetch` only to hosts the owner approved, no app
actions past its own). Which extensions a link runs, and the packages
and signers it trusts, are the link's owner-signed or node-held needs
(`share_needs::LinkNeeds`, `plan_link`, `verify_package`), never a vault
file. The guest host scopes the documents and the vault query engine to
the share: `GUEST_READS`' `documents` is the generic capability.

**A guest runs the app's linked extensions** (#805):
the same compiled-in extensions as a member's, on a host built for a
guest (`features/extensions/src/hostapi.rs` `Host::for_guest`). That
host has no documents, no vault query service and no data store, and it
refuses every host call that reaches a vault, kept data, files, the
device or an app action, whatever the manifest says
(`features/extensions/src/hostcall.rs` `guest_check`, refusals named
`guest`). A note a linked view asks to open goes to the app's own opener
alone (`Host::open_with_app`), which opens only a note the link lists.
Its views draw a guest's note because the app says so
(`features/extensions/src/embeds.rs` `GuestEmbedPolicy::own_linked`):
they are the app's own code, and what each is handed is still what the
link admits and its manifest reads.

**Known gap.** A guest runs no fetched extension yet (#495 G1).

## Processes over vox

A process extension is a program the host starts and speaks
[vox](@/architecture/streams.md) with, exactly as a remote client does:
it registers actions, serves its own services, and calls the app's. What
dodeca learnt doing this (it ran its "cells" as processes over shared
memory, then as in-process libraries, then went back to one binary) is
what this layer copies and avoids.

**Copied:**

- **One protocol crate per extension**, its services `#[architect::rpc]`
  traits, so the app and the extension share types with no codegen.
- **A host service the extension calls back into**, over a reverse vox
  connection routed by service name: the app's actions, settings and
  state, behind the extension's capabilities.
- **A `serve_extension!` main** that does the plumbing: dies with its
  parent, connects over a local socket (or stdio), and says hello with
  its name, version, API version and contributions. vox exchanges
  schemas per connection and bridges old and new, so an extension built
  against an older app still talks to a newer one.
- **Lazy start:** started on first use, behind a non-generic
  `get_or_start(name)` (dodeca's generic client accessor alone cost 325k
  lines of LLVM IR).
- **One trait, three ways to run it:** linked in (a first-party
  extension called directly), on a thread, or as a process — chosen by
  the manifest or a cargo feature, never by the extension's code.

**Avoided:**

- **Shared-memory rings:** dodeca's leaked and raced, and vox has
  dropped them. Large payloads go as a file or a passed descriptor
  (`vox-fdpass`), not through the RPC.
- **Exiting when an extension dies:** the host supervises — restarts
  with backoff, says why it died, stops idle ones.
- **Tables of extensions in the host:** found from their manifests.
- **Long-lived work inside a request:** an action that starts something
  long returns at once and streams its progress on a channel (dodeca hit
  vox's request timeout tearing its channels down).

**Not for build speed:** splitting the app into processes to link it
faster was dodeca's reason, and it undid it. Processes are for isolation
(third-party code, crash-prone work) and separate lifetimes (Studio's
engine), not for builds.

## Hot reload

No extension ever needs the app restarted (REAPER's do): the host can
take an extension out and put a new one in while the app runs, and an
author's edit shows in the running app in the time it takes to build.

**What the host does on a reload**, the same for every kind:

1. Asks the old one for its state (`save-state`, opaque bytes, optional).
2. Stops it: its actions, views, openers, settings sections and keymap
   are withdrawn as a diff, not torn down.
3. Starts the new one, hands it the state (`restore-state`), and takes
   its contributions as a diff against the old: an action that is still
   there keeps its bindings and its place on the ribbon; a view that is
   still there remounts in the same panes with the same pane state; one
   that is gone leaves a placeholder that says so (and comes back if the
   next build has it).
4. Says so: a toast with the extension's name and how long it took, or
   its error (a failed build or a panic) without removing the last good
   one.

**How fast, by kind:**

| Kind | On save |
|---|---|
| Data (themes, keymaps, frame views) | re-read: instant |
| Luau scripts | re-read on the next run; a running one finishes on the old code |
| TypeScript actions | its folder transpiled again as it reloads (a mistake keeps the last good one, at its line); each run loads its module fresh, a running one finishes on the old code |
| WASM components | rebuilt (`cargo component build`), re-instantiated: seconds, state kept |
| WASM views' `rsx!` | Dioxus's own hot reload: the extension's `VirtualDom` takes template changes from the dev server, so markup and style edits apply without rebuilding the `.wasm` — instant |
| Processes | rebuilt and restarted by the supervisor, state handed over (a view's own place in what it shows: `linked::use_view_state`, kept by its pane, `Pane::view_state`); in development the extension can run under `dx serve` with Rust hot-patching (it is its own binary), so most code edits apply without a restart at all |

**The author's loop:** `architect ext dev <path>` builds the extension,
installs it into the running app as a development copy (marked as such,
its capabilities granted for the session), watches its sources, and
reloads on every save; its logs and panics stream to the terminal and to
the app's diagnostics. A development copy shadows an installed one with
the same id until the session ends.

## Extensions and themes

An extension, and every GUI it draws, follows the app's theme live, the
same as the app's own views
([theming](@/architecture/theming.md)). It can also add to the theme: a
theme of its own, and tokens of its own that default to the palette.

**What a manifest adds** (`features/extensions/src/manifest.rs`
`ThemeDecl`, `Manifest::tokens`, `Manifest::token_namespace`):

```toml
[[themes]]
path = "themes/clicker-neon.json"   # a theme file; its id is the file's name

[tokens]                             # its own tokens, in its namespace
"clicker.face" = "mix($card, $brand, 0.22)"
"clicker.count" = "$brand"
"clicker.radius" = "10"
```

A token's namespace is the last part of the extension's id (`lab.clicker`
gives `clicker.`). Its names and values are checked as a theme's
overrides are, when the manifest is read.

**Into the app's store.** A host given the app's store
(`features/extensions/src/host.rs` `Host::with_themes`) contributes each
extension's themes and tokens as it loads (`theme_contribution`,
`architect_theme::ThemeStore::contribute`). On a reload they go in as a
diff, so a development copy's saved theme file shows at once. They come
out as it unloads (`ThemeStore::withdraw`). `THEME_LIST` shows such a
theme with the source `extension:<id>`, and `THEME_SET` picks it like any
other. A file that does not read is a `THEME_PROBLEMS` entry, never a
failed load, and the extension's last good theme of that id stays. The
tokens are the store's `Extension` layer: defaults under the theme's own
overrides, so a theme file can restyle an extension, and under the app's
and the person's layers. The store's revision moves only when what shows
changes.

**Each kind of GUI follows it:**

| GUI | how it themes | live |
|---|---|---|
| A process extension's Dioxus view | the host's `:root` CSS variables in its sub-document (`app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteDocument::set_theme_css`); `var(--brand)`, and an extension's own namespace when the app opts it in (`ThemeStoreProvider { css_namespaces }`) | on every change |
| A process that paints itself or works out colours | the theme as data (wire 3, below): `Guest::theme()`, `Guest::on_theme`, `guest::use_theme()` give an `architect_theme::Resolved` | as it connects and on every change |
| A Luau script | `theme` (`features/scripting/src/theme.rs` `ThemeModule`): `theme.color(name)`, `theme.number`, `theme.value`, `theme.scheme`, `theme.revision`, `theme.snapshot()`, `theme.subscribe(fn)` | a frozen snapshot per revision, made again at the first frame after a change |

**The theme as data, on the wire.** The host sends a process the
resolved tokens, the scheme and the revision
(`features/extensions/src/wire.rs` `ThemeData`, the
`ProcessExtension::theme` method in `features/extensions/src/proto.rs`)
right after hello, before any view opens, and again on each real change
(`features/extensions/src/process.rs` `ProcessRunner::with_theme`). The
wire is version 3. A host still runs a wire-2 process (`WIRE_MIN`) and
sends it no theme. vox names a method by its service and its name, so
the new method moves no other. A host built before wire 3 refuses a
wire-3 process, since it checks the version exactly.

**A script's snapshot.** A script reads one theme for a whole run, and a
live script one per frame. The snapshot is keyed by the store's revision
and made again only when it moves (herdr's `__theme_revision`), so reading
`theme.color` in a loop costs a table lookup. `theme` is a mutable global
to Luau's compiler (`Module::live_fields`), so `theme.scheme` is read
each time, never folded when the chunk loads.

## Immediate-mode UI for scripts (ReaImGui's)

A script that wants a window draws it every frame, ReaImGui's way. We do
not build a toolkit for it, and we do not need C++:

- **egui draws it.** egui is pure Rust (MIT/Apache) and builds for every
  target we ship, the web included; `egui-wgpu` renders into a texture on
  the app's own wgpu device, and a Blitz custom widget shows it — so a
  script's window is a pane like any other: docked, tabbed, stacked,
  saved in screensets. The widget forwards the pointer and keys to egui.
  Its style comes from the app's theme, live ("Extensions and themes"
  above).
- **Scripts see ReaImGui's API.** A Luau table shaped like ReaImGui's
  (`local ImGui = require 'imgui' '0.10'`, `ImGui.Begin(ctx, …)`,
  `ImGui.Button(ctx, label)`), implemented on egui, so most ReaImGui
  scripts port by replacing their `reaper.*` calls. ReaImGui is LGPL, so
  the names are re-declared from Dear ImGui's own (MIT) API, not copied
  from its files. Tables, draw-list specifics and multi-viewport flags
  do not port exactly.
- **WASM extensions** call the same API through a WIT interface.
- **`gfx`:** a small drawing API (lines, rectangles, text, images) on the
  same canvas, for gfx-style scripts and JSFX's `@gfx`.
- It redraws when something changes (input, the state it reads), not
  every frame, and passes ids and flags as plain values: the one Lua
  binding to egui that existed died of the garbage a per-frame UI made.
- Where exact ReaImGui fidelity matters more than the web, Dear ImGui
  itself (`dear-imgui-rs`, its wgpu backend) can sit behind the same API
  on native platforms.

## JSFX, in Studio

JSFX — REAPER's scripted effects — went open source in June 2025
(`justinfrankel/jsfx`, LGPL-2.1); its language, EEL2, is under WDL's
zlib-style licence. Studio hosts them, so the thousands of existing JSFX
run as effects and instruments in its audio graph, **with their own
GUIs**.

The bar is a whole library working unchanged: chmaha's DSP-Ports
(a ReaPack repository of 54 effects and synths) is the first test.
Every one of them draws its own GUI (`@gfx`), and between them they use
nearly all of JSFX's drawing (`gfx_set`, `rect`, `drawstr`, `setfont`,
`measurestr`, `circle`, `line`, `gradrect`, `blit`, `arc`, `triangle`,
`roundrect`, images with `gfx_loadimg`, HiDPI with `gfx_ext_retina`),
keyboard input (`gfx_getchar`) and pop-up menus (`gfx_showmenu`); twenty
take or send MIDI (synths, drum kits); seven keep state with
`@serialize`; some take large memory (`maxmem`) or share it between
instances (`gmem`); and some ship files beside them — an include
(`.jsfx-inc`), a meter's skin (PNG), sample sets (FLAC and a text map),
presets.

- **ysfx** (Apache-2.0; the maintained fork, `JoepVanlier/ysfx`) is the
  host: load, compile, sliders, MIDI, `@init` `@slider` `@block`
  `@sample` `@serialize`, `@gfx` into a framebuffer, file access for a
  package's own files. Studio links it over FFI and wraps each JSFX as an
  audio-graph node: sliders as parameters (automatable, in the generic
  parameter panel when a JSFX has no `@gfx`, or the person hides it),
  MIDI in and out, latency, state saved with the project.
- **Its GUI** is a canvas pane ([below](#what-studio-hooks-into)): ysfx
  draws `@gfx` into its framebuffer at the pane's size and scale, the
  canvas shows it and hands it the pointer and the keys;
  `gfx_showmenu` opens the shell's own menu at the pointer; a JSFX's
  window docks, tabs and floats like any view.
- EEL2 compiles to machine code on desk and Android; on iOS, which
  forbids that, it runs interpreted (about an eighth of the speed —
  enough for most effects, not for every synth).
- The web needs EEL2 without C++: a Rust interpreter (none published yet)
  or compiling EEL2 to WebAssembly at runtime.

### What Studio hooks into

JSFX is Studio's (a DAW's format); what it needs from architect is
general, and each piece serves more than JSFX:

| architect gives | JSFX uses it for | also used by |
|---|---|---|
| **A canvas view**: a pane that shows what the app draws on the GPU (a texture each frame, or a display list), with the pointer, keys, size and scale given back | `@gfx` | egui (script UI), plugin editors' own drawing, meters, scopes |
| **Menus from code**: open a menu at a point, get the pick back | `gfx_showmenu` | context menus anywhere |
| **Package kinds in the registry**: an app registers a kind (`studio.jsfx`), where it installs, what it checks | installing JSFX and their files | presets, sample packs, themes |
| **ReaPack indexes as a registry source**: the registry reads ReaPack's `index.xml` (versions, categories, per-platform files, `@provides`) beside its own format | DSP-Ports and every other JSFX repository, unchanged | REAPER-world scripts and data |
| **A parameter panel**: generated controls for a list of typed parameters (the settings controls, in a pane) | sliders of a JSFX without a GUI | any effect, CLAP/VST3 plugins without an editor |

## Overlays: drawing over another view

REAPER scripts that draw over the arrangement (TK's track names, activity
glow) do it from outside: find the arrange window by its child id, read
its rectangle and scroll, float a click-through ImGui window on top, read
every track's Y and the visible time range every frame, and keep pulling
the window back above whatever covers it. Here a view offers it:

- **A view publishes its coordinate spaces:** the arrangement publishes
  time → x (the visible range, the zoom), track → y (each lane's top and
  height, pinned and hidden ones included), and its scroll — as a stream
  that changes when they do, not a value polled every frame.
- **An extension contributes a layer** to a view by kind
  (`session.arrange`): drawn inside the view, between named planes (under
  the items, over them, over everything), clipped to it, scrolled and
  zoomed with it, in the right order whatever floats elsewhere. A layer
  is a frame document or the extension's Dioxus view, positioned in the
  view's coordinates (`at: { time: 12.5, track: "Kick" }`).
- **Input:** a layer passes the pointer through by default; it may claim
  regions (a label it can be dragged by), which then take the pointer
  and context-menu there.
- **What it reads** — track levels for an activity glow, the playhead —
  are the app's streams, through the extension's capabilities.

The same mechanism lets a first-party feature draw on the timeline (a
lyrics lane, chord symbols over the sections) without the timeline
knowing about it.

**How to draw one.** An extension with views of its own (a process's,
a WASI component's, a linked one's) names one of them (`draws`, #175);
a Luau overlay script (`[[overlays]] script`) is parsed and checked, and
its canvas went with `architect-canvas`:

```toml
[capabilities]
over = ["session.arrange"]  # drawing over a view not its own: asked for in words

[[views]]
id = "marks"
title = "Marks"

[[overlays]]
id = "marks"
view = "session.arrange"   # the kind of view it draws over
plane = "over"             # under | over | top
draws = "marks"            # one of its [[views]]; or script = "…" (Luau), never both
```

Drawing over a view not its own (another extension's, or the app's own
arrangement) is a capability: `capabilities.over` names the view kinds,
the consent prompt says "Draw over the arrangement", the manifest is
refused without it, and the host doesn't mount the layer until it is
granted (`Host::view_overlays`, `Capabilities::may_draw_over`). An
overlay's spaces reach it coalesced: a view scrolled or zoomed sends one
update a frame, and only the latest waits while the overlay's view is
busy.

The view it draws over mounts `app-shell:crates/app-shell-extensions/src/interface.rs`
`ExtensionViewOverlays { view, plane, spaces }` at that plane, filling its
box, with its coordinate spaces now (`features/extensions/src/overlay.rs`
`OverlaySpaces`: size, time across, lanes down, scrolled). For each such
overlay the host mounts the extension's view over the box, clipped to it,
and passing the pointer through to the view
(`remote::RemoteOverlay`, or the linked view itself). The view is handed
the spaces as its embed input whenever they change: `overlay::spaces()`
reads them, so it draws in the view's terms (a `scene::RemoteScene`
filling its box, its coordinates the view's). A process that isn't
running yet, or died, is simply absent; nothing is said over the view.
Test: `app-shell:crates/app-shell-extensions/tests/remote_overlay.rs` (a mark on a
lane at a time, following the view as it scrolls, a click let through).
Not yet: a layer that claims regions to take the pointer there.

An extension that needs to draw outside any one view declares `overlay`
(`capabilities.ui`, above): its scenes then draw over the whole window,
in window coordinates.

## What an extension costs

Not everything should be one. Every extension pays for its boundary:

- **An API has to exist for everything it touches,** and that API is
  then public: the Action List, the streams, the views' coordinate
  spaces are versioned, and a rename is an ecosystem break.
- **Crossing is not free:** a WASM call copies its arguments, a process
  call is IPC, a remote view's frame is serialised and its events come
  back. Fine for a panel; wrong for anything per-sample, per-voice or
  per-frame at scale. Nothing realtime crosses a boundary.
- **Text input and focus** are the hardest part of remote views; a
  feature that is mostly editing text pays the most.
- **More to run and debug:** instances to start and supervise, memory per
  WASM instance, a second stack in a panic, versions to keep in step.
- **iOS runs WASM interpreted:** slower, still fine for UI logic.

dodeca split its own features into plugins to keep the core light, and
took it back: the core got lighter, the product got harder.

## Responsive by design: views and workers

The guarantee: **an extension's GUI never waits on its own heavy work,
and the app's UI never waits on any extension**, on every platform.
Every runner gives an extension two roles, and the host keeps them
apart:

- **A view** handles input and answers with edits or paint commands
  within a frame's budget (16 ms for a Luau window, 33 ms for a
  process's view, the socket's round trip included). It runs on a
  thread of its own, never the app's UI thread; the host awaits it.
- **A worker** does the heavy work: its own instance, its own state and
  memory, on a host thread pool, spoken to by messages only (Zellij's
  plugin workers, a browser's Web Workers). A view spawns it, posts it
  messages, and reads its progress and result; it is cancelled when the
  view or the extension goes, and runs within its own step or CPU limit.

An extension declares its workers in `extension.toml`:

```toml
[[workers]]
id = "primes"
script = "workers/primes.luau"   # luau and process packages
# entry = "primes"               # process and wasm: the program's own name for it
```

(`features/extensions/src/manifest.rs` `WorkerDecl`.)

### Per runner

| | the view runs | a worker is | cancelled when |
|---|---|---|---|
| Luau | its own VM thread, one Luau state per session; one-off runs (actions, hooks) on a pool | another Luau state of the declared script, on the pool | its owner's state goes, or the extension unloads (`WorkerGroup::cancel_all`) |
| Process | one thread per open view in the process, one event at a time | `guest::spawn_work`: a thread of the process | its `WorkHandle` drops (the view closed) |
| WASI | one instance of the component on a thread of its own, each call within an epoch deadline | another instance of the same component on a thread of `wasi::Pool` (on the web, next, a Web Worker) | its owner cancels it, traps or is unloaded |
| Linked | the app's Dioxus tree: a linked extension is first-party code and keeps to the rule itself | its own thread or task | as the code says |
| Page (TSX, Luau) | one thread per open page, each call within `PageLimits::frame` | TSX: a `.js`/`.ts` file of the page's folder in a `QuickJS` runtime of its own thread; Luau: a `.luau` file of it in a Luau state on the scripting pool | its page closes, or the handle's `cancel()` |

**Luau** (`features/scripting/src/vm.rs`, `features/scripting/src/worker.rs`):
no script runs on its caller's thread. `Runtime::run` takes a thread of
an elastic pool (one that grows when every thread is busy, so a script
that runs a script never waits for one) and the caller awaits a channel;
dropping the future stops the script at its next step. `Runtime::start`
gives a live script (an `ImGui` window, an overlay) a thread of its own
for its life; each `Live::frame` runs there. `Limits::frame` (500 ms by
default) stops a frame that runs past it, saying to use a worker;
`Limits::steps` still stops a runaway loop; `Limits::worker` gives
workers far more steps and five minutes of CPU. The theme reaches a
script on any thread: `ThemeCell` is shared, and each frame takes its
snapshot at the start (`ThemeModule`).

A script spawns a worker by name and reads it each frame, immediate
mode, with no callbacks needed:

```lua
local job = worker.spawn('primes', { n = 3000000 })
local function loop()
  local p = job:progress()          -- {done, total, fraction, note}
  ImGui.ProgressBar(ctx, p and p.fraction or 0)
  if job:status() == 'done' then ImGui.Text(ctx, job:result() .. ' primes') end
  wake_after(33)                    -- the next frame comes by itself
  defer(loop)
end
```

The worker sees `args`, `worker.progress(done, total, note)`,
`worker.post(msg)`, `worker.receive(ms)` / `worker.poll()`,
`worker.on_message(fn)` and `worker.cancelled()`; no actions, no
`ImGui`, no theme. Its news wakes the view's host for a frame
(`Live::woken`); `wake_after(ms)` asks for an animation frame. A
one-off run waits with `job:join()` or `job:next()`; a view must not.

**Process guests** (`features/extensions/src/work.rs`): a view's thread
renders one event at a time, so the rule is that a handler starts work
and returns. `Guest::spawn_work` (`spawn_work`) runs a closure on a
thread of its own with a `Work` to report through (`progress`, `post`,
`check` for cancellation); `use_work` gives a view its state as a
signal. Every event is answered with a frame, empty when nothing
changed (wire 5, `ANSWERED_WIRE`), so the host times each one.

### Budgets and degradation

The host measures every frame of a view it does not draw itself
(`features/views/src/meter.rs` `ViewMeter`): a Luau window's time on
its own thread (`Session::frame_metered`), a process view's time from
the event to its answering frame (`app-shell:crates/app-shell-extensions/src/remote.rs`
`RemoteView`). Past the budget, the pane keeps its last good frame and
shows a small busy mark in the shell's theme
(`features/shell/src/shell/busy.rs` `BusyBadge`); the rest of the app
answers as ever, since nothing on the UI thread waits. Each view's p50,
p99, slowest and slow-frame count are in the app's `Meters` (provided in
context, `<extension>/<view>` or the script's step name), read by
`EXT_VIEWS` (`features/extensions/src/views.rs` `install_view_actions`).
`Meters::with_report` takes a `ViewReport`, told when a view turns slow
or busy and when it recovers: the seam an app wires to its status board.
`features/extensions/src/views.rs` `ViewProblems` is that report for
`APP_STATUS`: a slow or busy view is a problem (source `view`, its
timings) until it draws within its budget again. The manager's page of
an extension has its views' p50, p99 and slow count ("Frame health",
`health_lines`; `ExtensionKind::with_meters`). Linked extensions' views
are Dioxus components in the app's own document and are not metered.

`features/scripting/tests/workers.rs` and
`features/extensions/tests/workers.rs` measure it.

## Contributions: what an extension adds to the interface

Past actions, settings, views and themes, an extension adds to the
interface the way the app's own features do, declared in its manifest or
sent at run time. The same tables are one type,
`features/extensions/src/contrib.rs` `Contribution`, whichever way they
arrive: a manifest's (`Manifest::contributes`), a process guest's
(`features/extensions/src/guest.rs` `contribute`), a Luau script's
(`ext.contribute{…}`), an in-process extension's builder
(`features/extensions/src/linked.rs` `Builder::contribute`). Each is
checked against its owner's namespace, capabilities and views
(`Contribution::check`): an action it names is its own or one its
`[capabilities] actions` reach, a view one of its `[[views]]`, an event,
a target and a slot ones there are, a URL pattern one that compiles.

```toml
[[openers]]                 # a claim: files by extension, kinds, links
id = "kf"
title = "Keyflow chart"
files = ["kf"]
view = "chart"              # a pane per file (its id kind:path); or action = "…": given ctx.path / ctx.url

[[openers]]
id = "issue"
title = "GitHub issue"
urls = ['^https://github\.com/[^/]+/[^/]+/issues/\d+$']
action = "LAB_BOARD_ISSUE"

[[openers]]                 # a ⌘O source: kind:… asks only it
kind = "theme"
title = "Themes"
action = "THEME_PICKER_FIND" # {query} in, {items: [{id, title, detail?, recency?}]} out
open = "THEME_PICKER_PICK"  # an item picked: {id} (and `in` when its params take it)

[[doc_kinds]]               # a document kind, registered while loaded
id = "keyflow.chart"
name = "Keyflow chart"
files = ["kf"]
view = "chart"

[[tabs]]                    # an app section showing one of its views
id = "board"
title = "Board"
icon = "layout-dashboard"   # a lucide name
view = "board"

[[status]]                  # a status item; its text changes at run time
id = "pins"
text = "Nothing pinned"
side = "left"

[[menus]]                   # a row in a named menu of the menu bar
menu = "View"
action = "LAB_BOARD_CLEAR"

[[context]]                 # a row in a target's context menu
target = "view:note"        # pane, tab, view:<kind>, file, file:<ext>, doc:<kind>, link, selection
action = "LAB_BOARD_PIN"
title = "Pin to the board"

[[bar]]                     # a button on a phone's action bar
action = "LAB_BOARD_PIN"
icon = "pin"

[[toolbars]]                # buttons, separators, groups, dropdowns, widgets
id = "transport"
title = "Transport"
where = "dock:right"        # dock:left|right|bottom, float, window, pane, view:<kind>

[[toolbars.items]]          # a button (the default kind)
action = "LAB_BOARD_LOOP"   # a toggle's button shows it on or off
icon = "timer"              # a lucide name; none: its label alone
label = "Loop"              # the action's name when unset
right_click = "arm"         # menu, arm, none, or another action's id

[[hooks]]                   # on an app event: one action, or a Luau script
on = "startup"              # workspace.changed, doc.opened, theme.changed, selection.changed, changed:<topic>
action = "LAB_BOARD_HELLO"

[[slots]]                   # a shell region it fills with one of its views
slot = "status"             # ribbon, left, right, tabs, status, title
view = "status-bar"
```

**The registry.** `features/extensions/src/host.rs` `Host::contributions`
is the app's `Contributions`: a load puts the manifest's set (with an
in-process builder's beside it), a reload puts it again as a diff (what
was added at run time stays until the new instance says otherwise), an
unload withdraws everything the extension added and its runtime state.
`Contributions` answers the window's questions: `tabs`, `status`,
`menus`, `context` on a target, `bar`, `toolbars` (and
`header_toolbars` of a view kind), `slots` (the latest claim wins),
`open_path` (by extension, then by document kind), `open_url` (by
pattern), `open_doc_kind`, `doc_kind_of`.

**⌘O's sources.** An `[[openers]]` with a `kind` (no `files`,
`doc_kinds` or `urls`) is one of ⌘O's sources, the same as the app's own
(`architect_commands::Opener`): `use_extension_ui` registers each one
while its extension is loaded (`app-shell:crates/app-shell-extensions/src/interface.rs`
`use_extension_ui`, `Contributions::quick_open`). A query runs its
`action` with `{query}`; a linked extension's answer is used at once, a
process's when it arrives (the palette looks again, the last answer
shown meanwhile). An item picked runs `open` with `{id}`. `COMMANDS_SHOW
{"overlay": "quick-open", "query": "theme:"}` opens ⌘O on one source
alone, under its title.

**The palette, its keys and its sources (#381).** ⌘K and ⌘O are an
extension's: the shell keeps the slot (`architect_commands::CommandLayer`)
and `COMMANDS_SHOW`, and the Command Palette (id `command-palette`; its
engine is `app-shell:crates/app-shell/src/palette`, its sources
`app-shell:crates/app-shell-extensions/src/palette.rs`) fills it. Three points, withdrawn when their
extension is off and read with `EXT_PALETTE`
(`app-shell:crates/app-shell-extensions/src/interface.rs` `use_palette` keeps the shell
in step):

```toml
[[palette_views]]            # which of its views draws an overlay
overlay = "palette"          # or "quick-open"
view = "palette"

[[keys]]                     # a key in the app's keymap, to its action
chord = "⌘K"
action = "COMMAND_PALETTE_OPEN"

[[keys]]                     # a plain letter, only while its view has the focus
chord = "A"
action = "KEYFLOW_NOTE_A"
when = "view:chart"          # the keymap's `view:<extension>/chart` context

[[palette_sources]]          # rows for ⌘K, ranked by the engine
id = "tours"                 # and its prefix: tours: asks only it
title = "Tours"
action = "TOURS_PALETTE_FIND"   # {query} in, {items: [{id, label, detail?, group?}]} out
run = "TOURS_START"             # a row picked, given {id}
ranking = "open"                # or "commands"
```

The first loaded extension that draws an overlay draws it; a
development copy of the palette (`EXT_CLONE {"id":"command-palette"}`)
draws it in the built-in's place. A view named in `[[palette_views]]` is
not offered as a pane. With no palette, ⌘K and ⌘O do nothing and
`COMMANDS_SHOW` fails. The engine is `architect-palette`
(`features/palette/src/source.rs` `Engine`), the one the launcher ranks
with.

**A settings section drawn by a view.** `[[sections]] view = "section"`
puts one of its views on the settings page as a section of its own (the
extensions manager's); `group = ""` puts it in the first, untitled group.
The page draws it through `architect_settings::SectionViews`, which
`use_extension_ui` provides (a linked view's component as it is). A
`[[sections]]` table is an item of the settings page's point,
`settings.sections` ("Windows, points and services" below).

**Context menus.** A `[[context]]` row is offered on its target: the
workspace asks `architect_workspace::ContextEntries` (which
`use_extension_ui` provides) each time a menu opens. A tab's menu (its
right-click, its header's ⋯) asks for `view:<kind>` and `tab`, with the
pane's id; a view's own targets open through
`architect_workspace::PaneHandle::context_menu` — the sandbox's file tree
calls it on a row's right-click with `file` and the file's path, and a
`file:<ext>` row is on files of that extension. The row runs with
`architect_workspace::context_invocation`: source `context_menu`, the
target and its id, the pane and its view, and for a file its path. A row
whose action is a toggle shows its tick. A target is a contribution
point its feature declares (`Host::declare_context_targets`; point
`context.<kind>`, `features/extensions/src/contrib.rs` `context_point`,
`context_target_point`): the workspace's `pane`, `tab` and `view`
(`architect_workspace::CONTEXT_TARGETS`), the toolbars' `toolbar` and
`toolbar-item` (`features/extensions/src/toolbar.rs` `CONTEXT_TARGETS`),
the sandbox's file tree its `file`. Each `[[context]]` row is an item of its
target's point, so `EXT_POINTS` lists them by target, and rows for a
target nobody declared wait there; a manifest's target need only be a
target's name (`contrib::known_target`).

**Toolbars.** A toolbar is a document (`features/extensions/src/toolbar.rs`
`ToolbarDecl`): an id, a title, `where` it goes, and its items, one flat
type (`ToolItem`) whose `kind` is a `button` (the default), a
`separator`, a `group` (its `items`, buttons drawn as one, with an
optional `label`) or a `widget` (an extension's `view` as a live control,
`width` × `height`, drawn by `ExtensionView` whatever runs it); an item
with `newline` starts another row. A button runs its `action` or, with
none, opens its `menu` (a dropdown: `MenuItem` rows of kind `action`,
`heading`, `separator` and `submenu`, nested up to `MAX_MENU_DEPTH`, a
toggle's row ticked); it has a `label` and/or an `icon`, `hide_label`,
`colors` (`fill`, `ink`, `on_*` while on, `armed_*` while armed: theme
tokens, drawn as their CSS variables, or CSS colours), and a
`right_click`: `menu`, `arm`, `none`, or another action's id. The format
is the same in a manifest (`[[toolbars]]`, each action its own or one its
capabilities reach, each widget one of its views), in a person's file,
and in `TOOLBAR_GET`'s answer, so a visual editor reads and writes it as
it is.

A person's toolbar files are `*.toml` in a folder the app names
(`features/extensions/src/toolbars.rs` `Toolbars::set_dir`; the sandbox's
`toolbars/`), read again as they change (`Toolbars::poll`, once a
second). A file whose `id` is an extension toolbar's key
(`fts.metronome/transport`) is the person's copy, used in its place until
`Toolbars::reset`; another id is a toolbar of their own
(`user/<id>`); a file that does not read or check is a problem
`APP_STATUS` lists, the rest load. `Host::toolbars` is the registry
(`Toolbars::all`: every toolbar in force, `Source` `extension`,
`override` or `person`).

Where it goes: `dock:<side>`, `float` and `window` are a pane of its own,
a headless workspace view of kind `toolbar:<key>`, opened there when it
appears (`window` a window of its own, as a popped-out pane is; once a
window: one the person closes stays closed) and closed when it goes;
`pane` (every pane's header) and `view:<kind>` (that kind's) draw it
between a pane's title and its menu, through
`architect_workspace::HeaderTools`. Both are
`app-shell:crates/app-shell-extensions/src/toolbar_ui.rs` `ToolbarStrip`: a press runs the
action told source `toolbar`, target its place, id its key, and the pane.
`typing` is a phone's Typing Toolbar over the keyboard, shown while its
`when` is in force (the page or view kind the focus is in,
`view:lab.board/chart`; an editor's context, `editor:markdown`), after
the focused editor's own items: `app-shell:crates/app-shell-extensions/src/typing_ui.rs`,
`app-shell.md` ("The action bar and the keyboard bar"). The app's own
toolbars at run time are `Toolbars::provide` (`Source` `provided`): a
focused editor's items as `typing/<context>`, which a person's file
overrides as it would an extension's.
A bad toolbar in a manifest (a place there is not, no items, an item not
whole for its kind, an action it may not run) refuses the load like any
bad contribution: a problem `APP_STATUS` lists, field and all, never a
crash.

**Armed.** One action at a time may be armed (`Toolbars::arm`, a
button's `right_click = "arm"`, `TOOLBAR_ARM`): its buttons show it
(`data-armed`, the `armed_*` colours) until it is disarmed. What an armed
action does next, to what is clicked, is the app's.

| action | what it does |
|---|---|
| `TOOLBAR_LIST` | every toolbar in force, the folder of a person's files, the armed action |
| `TOOLBAR_GET` | toolbar `{id}` whole, as its file has it |
| `TOOLBAR_SAVE` | `{id, toolbar}` (JSON or TOML text) as the person's file |
| `TOOLBAR_RESET` | takes away the person's file for `{id}` |
| `TOOLBAR_ARM` / `TOOLBAR_DISARM` | arms `{action}`, or disarms |

`Extensions::install_contribution_actions` puts them on the host's list
(`features/extensions/src/toolbar_actions.rs`), with the editor's
fine-grained edits (`TOOLBAR_INSERT`, `TOOLBAR_MOVE`, `TOOLBAR_SET` and
the rest). The visual editor — edit mode, + chips, drag and drop, the
action and widget pickers, item and toolbar settings, debounced writes
with backups — is `toolbars.md`.

A toolbar may also have a `style` (`ToolbarStyle`: `size`, `spacing`,
`rounding`, a colour `preset`), a `when` (the context it is for), and a
button an `image` file in place of its `icon`; `where = "hidden"` keeps
one without showing it.

**Toggles.** An action can say whether it is on: the Action List's
`ActionList::set_checked`, read into `ActionInfo::checked`. An
extension's action declares `toggle = true` and says it with runtime
state of kind `checked`, keyed by its id, `text = "on"` (or `"true"`)
for on, anything else for off; it is off until it writes one
(`Contributions::checked`; the host registers the check as it loads the
action). Every kind writes it the same way: `ext.state{kind = "checked",
key = "…", text = "on"}` in Luau, `guest::set_state`, a WASI guest's
`state` update, a linked `Handle`. The tick shows in ⌘K, the menu bar's
contributed rows, a context menu's row, and a toolbar's button
(`aria-pressed`, the accent), and follows the state live: a toolbar
redraws when the contributions change and when anything happens on the
host's list. A metronome's `FTS_METRONOME_CLICK`, say, can sit on a
toolbar in the right dock, in a pane's header, and in a tab's context
menu, ticked in all of them.

**Opening.** `Host::open` takes a file's path, a link or a document
kind to the first opener that claims it: its view opens through the
app's view opener (`Host::set_view_opener`), or its action runs with
`ctx.path` / `ctx.url`. A file opened raises `doc.opened`. In the
window, a link's Ctrl- or Cmd-click goes there
(`app-shell:crates/app-shell-extensions/src/interface.rs` `ExtensionUi::open_link`,
`link_press`); `EXT_OPEN` does it from the Action List.

### File types

An extension teaches the vault a file type without core code naming it:
`[[file_types]]` declares an extension whose files are notes like `.md`
ones.

```toml
[[file_types]]
extension = "cook"     # without the dot, lower-case; not md, markdown, base or canvas
name = "Recipe"
icon = "chef-hat"
note_type = "recipe"   # the `type` such a file reads as when its frontmatter has none
body = "text"          # markdown (the default) or text: Cooklang's `#pan{}` is no tag
preview = "description" # its navigator row's preview: this property, not its body
```

A note, to the vault, is optional YAML `---` frontmatter and then a body
of text, so a declared file's frontmatter is its properties and the rest
its text, and everything that handles notes handles it unchanged: the
walkers, the open vault (`OpenVault`), its Loro document and sync, the
vault index (where it reads as `type: recipe` unless its frontmatter
says otherwise: `features/vault/vault-query/src/index.rs` `Entry::of_note`),
Bases (`filters: type == "recipe"` finds it), the documents API
(`read_document`, `edit_document`), the navigator and the web's notes.

They all ask one registry, `features/vault/vault-proto/src/note_types.rs`
(`is_note_path`, `is_note_extension`, `note_type_of`, `note_stem`;
re-exported as `vault::note_types`), process-wide and wasm-clean. `md` is
always in it. The extension host registers each declared type as it
loads the manifest (`Host::load`, on a desk, a phone and the web);
a node registers those of the extensions it loads
(`features/extensions/src/node.rs` `NodeHost::load`). A type stays registered for the life of the process:
its files stay notes when the extension is turned off. `EXT_FILE_TYPES`
lists `md` and every declared type, with who declared it.

**Its text in the editor.** `[[renderers]]` gives a note type its own
live preview: the text stays what is edited, and the extension says how
its ranges read.

```toml
[[renderers]]
note_type = "recipe"
action = "COOKBOOK_RENDER"   # given {text}, answers {items}
```

While such a note is open the app runs the action with its body each time
the text changes, and draws its answer over every line but the caret's:
each item a `conceal`ed range, a range `replace`d by other words, or a
range or a `line` in a look (`features/extensions/src/render.rs`
`RenderItem`, `Look`: heading, strong, muted, accent, chip, chip-accent,
code). The looks are the app's classes, styled by its theme, never CSS
the extension sends.

**Its body.** A declared type's body is markdown unless it says `body =
"text"`: then the vault reads only its frontmatter and its `[[links]]`,
not its inline `#tags`, task lines or headings, because the `#` and `-`
of another language mean something else (Cooklang's `#pan{}` is
cookware). One function parses a note by its path for everyone who reads
one: `features/vault/vault-query/src/moved/obsidian_parse.rs` `parse_note`
(over `vault_proto::note_types::body_is_markdown`).

An app registers its linked extensions' types before it reads a vault,
as a node does, so a vault opened at start sees their files as notes
before the extension host loads.

Opening such a file opens a note. An extension that draws its own view
for it claims its `note_type` as it would any note type's: `[[embeds]]
note_types`, or an `[[openers]]` entry's `files`. The manifest check
(`features/extensions/src/contrib.rs` `FileTypeDecl::check`) refuses an
extension with a dot or a capital, one the vault reads as its own
(`RESERVED_FILE_TYPES`), and an entry with no `name` or `note_type`.

### Hooks and the invocation

A hook runs on an app event (`contrib::EVENTS`, or `changed:<topic>`
for any Action List topic): its action through the list, inside the
extension's gate, or a Luau script — the whole file, or `function` of
the table it returns. `startup` runs once per extension as it first
loads (not on a reload); `extension.loaded` and `extension.unloaded`
for every load; `theme.changed` from the theme store; the app's list
events through `Host::watch` (`workspace` → `workspace.changed`,
`selection` → `selection.changed`, `doc` → `doc.opened`). One run of a
hook at a time: an event while it runs does not start it again.
`EXT_EMIT` raises one by hand. A hook can be on a schedule instead
(`every`, `at`), on a document's or the window's event, or on an event
the app defines: "Timers, events and background extensions" below.

Every run carries where it came from
(`crates/architect/src/action_list.rs` `Invocation`,
`ActionCaller::with_invocation`): the source (palette, menu,
context menu, key, hook, link, open, agent, script, toolbar) and what it knew —
the target and its id, window, pane, view, selection, url, path, event.
A Luau action reads it as `ctx`; a process at wire 4 is sent it
(`ProcessExtension::run_in`, `Guest::action_in`); an in-process action
gets it as its second argument. So an action needs no arguments to act
on what it was invoked on: "Pin to the board" on a note's tab pins that
note.

### Views: placement and ownership

A `[[views]]` entry says where the view lives (`home`: `main`, `left`,
`right`, `bottom`) and how it opens (`placement`: `tab`, `split`,
`overlay`, `popup`, `zoomed`, `float`, with `width` and `height`). In
the window each is a workspace view of kind `<extension>/<view>`
(`ExtensionUi::views`); `EXT_VIEW_OPEN` and a view opener place it
through `WORKSPACE_OPEN` and `WORKSPACE_ZOOM`, or as a layer over the
window (`ExtensionOverlays`; a second popup answers `busy`). What its
panes do when the extension unloads is its `on_unload`: `placeholder`
(the pane says the extension is not loaded, and comes back with it) or
`close`.

### Owned runtime state

Anything set at run time — a status item's text, a badge, metadata on a
target, whether a toggle is on — is owned by its source (`ext:<id>`, `agent:<name>`, `app`),
sequenced (a `seq` at or below the last kept is dropped as stale),
time-limited (`ttl_ms`), capped (64 keys a source, 80 characters of one
line), and withdrawn with an extension when it unloads
(`contrib::StateWrite`, `Contributions::set_state`). An extension writes
its own through its guest (`guest::set_state`), `ext.state{…}` in Luau,
or its `linked::Handle`; an agent through `EXT_STATE_SET`, which refuses
an `ext:` source.

| action | what it does |
|---|---|
| `EXT_CONTRIBUTIONS` | every extension's contributions (declared and at run time), the slots, the runtime state |
| `EXT_OPEN` | `{path}`, `{url}` or `{doc_kind, path?}` to its opener |
| `EXT_VIEW_OPEN` | `{extension, view, state?, placement?}` |
| `EXT_STATE_SET` / `EXT_STATE_CLEAR` | runtime state, by source and key |
| `EXT_EMIT` | raises `{event, detail?}` |
| `EXT_ENABLE` / `EXT_DISABLE` | turns `{id}` on or off at once |
| `EXT_EMBEDS` | every inline embed and its claims, or `{claim}`'s (`fence:kf`) |

`Extensions::install_contribution_actions` puts them on the host's list.

## Windows, points and services

Extensions own whole windows, and work with each other through the
host, declared in the manifest (`features/extensions/src/interop.rs`
reads and checks each table) and kept by the host while they are loaded.

```toml
reserved = ["APP_SETTINGS"]          # app ids it owns, when the app allows it

[[windows]]
id = "settings"
view = "sheet"                       # one of its [[views]]
title = "Settings"
kind = "sheet"                       # over the main window; "window": an OS window
opens = "APP_SETTINGS"               # one of its [[actions]]; the host runs it
# width = 720, height = 560          # points (unset: the sheet's own size)

[[points]]
id = "settings.sections"             # in its namespace: `<id>.…`
deliver = "SETTINGS_SECTIONS"        # told {point, revision, items: [{from, value}]}
schema = '{"type": "object", "required": ["id", "title"]}'   # or a TOML table

[[contribute."settings.sections"]]   # an item for another's point
id = "metronome"
title = "Metronome"

[provides]
"lab.tempo" = "1.2.0"                # method `get` is its action LAB_TEMPO_GET

[requires]
"lab.tempo" = "^1"                   # loaded after a provider; refused without one
```

**Windows.** A `[[windows]]` is one of its views in a window of its own.
The action it `opens` is the host's to run (its code has no handler for
it): run, it opens the window with its arguments as the window's state
(`features/extensions/src/windows.rs` `Windows`); the host service's
`window {id, open, state?}` opens or closes one of its own, and an unload
closes them. The core keeps only the machinery
(`app-shell:crates/app-shell-extensions/src/window_ui.rs` `ExtensionWindows`, mounted by
`ExtensionOverlays`): a `sheet` is a scrim and a card over the main
window (the whole screen on a phone), closed by a click outside or its
own close; a `window` is an OS window of its own through the workspace's
pop-out windows (`features/workspace/src/host/windows.rs`
`Popout::page`, where the platform gives one, else a sheet), closed by
the person or the host. Either way the view is drawn as every view is
(`ExtensionView`: a linked component, a process's or a WASI component's
remote document, a page) and follows the theme live: a sheet is in the
app's document; an OS window carries the shell and the theme's `:root`
sheet into its own. `EXT_WINDOWS` lists them and which are open,
`EXT_WINDOW_CLOSE` closes one.

**Reserved ids.** An app's own action ids (`APP_…`) are no extension's
namespace. An extension owns one when its manifest lists it in
`reserved` and the app allows that extension that id
(`Host::allow_reserved`): its built-in or a development copy of it (an
`EXT_CLONE`), never an installed copy (`LoadError::Reserved`). So
`APP_SETTINGS` (⌘,) is the settings extension's, and a person's
development copy of it still answers ⌘,.

**Contribution points.** An extension declares a point and others add
items to it; the app adds its own as `app`
(`Host::contribute_app`). The host (`features/extensions/src/points.rs`
`Points`) checks each item against the point's schema (a subset of JSON
Schema: `type`, `properties`, `required`, `items`, `enum`,
`additionalProperties`, `crate::interop::validate`): one that does not
meet it is left out and is a problem in `APP_STATUS` (`lab.giver: its
item 1 for lab.owner.cards was left out: title: is integer, not
string`), never a failed load. Items for a point nobody declares wait for
its owner. The owner is told the set through its `deliver` action as
extensions load, reload and unload, the app's items first, then each
contributor's in the order it came; it keeps the highest `revision`, and
reads them any time with the host service's `point {point}`.
`EXT_POINTS` shows every point, its items, the ones left out and why.
`[[sections]]` is sugar: each is an item of `settings.sections`
(`Manifest::point_items`), so every manifest that had one keeps working.

**Services.** `[provides] <service> = "<version>"` offers a service; a
method `m` of it is the provider's action `<PREFIX><M>`
(`interop::method_action`: `lab.tempo`'s `get` is `LAB_TEMPO_GET`).
`[requires] <service> = "<req>"` asks for one: `Extensions::sync` loads
providers first, and one whose service has no loaded provider meeting
the requirement is not loaded (`LoadError::Requires`, in `APP_STATUS`:
`it requires the service lab.tempo ^2: the loaded provider is lab.tempo
1.2.0`); when a provider goes, what requires it goes too, until it is
back. `[uses] <service> = "<req>"` is the optional form, for an integration
that lights up when the other extension is there (#367): no provider is
no problem, the extension loads and stays when its provider goes, and a
call answers `no provider of <service> <req>` (`HostError::NotFound`)
until one is loaded. A service is in one table or the other, never both.
A call is the host service's `service {service, method, args}`,
the same from every runner: `linked::Handle::service` in Rust (linked or
process), `host.service(name, method, args)` in Luau, `call-service` in
the WIT world (`features/extensions/wit/extension.wit`; the WASI guest's
`host::service`), and `services.call(service, method, args)` on a
`QuickJS` page (`EXT_SERVICE_CALL`, run as the page's extension). It is
checked as every host call is: the caller must require or use the service, the
method's effect must be within its `capabilities.effect`, and it runs
as `Principal::Extension` for whoever the caller acts for.

| action | what it does |
|---|---|
| `EXT_WINDOWS` | every extension window: its view, kind and opening action, whether it is open, its state |
| `EXT_WINDOW_CLOSE` | closes `{extension, id}` |
| `EXT_POINTS` | every point: owner, schema, items (who gave each), the items left out and why, items waiting for an owner |
| `EXT_SERVICE_CALL` | `{service, method, args?}` as the extension running it (a page's way to call a service) |

`features/extensions/tests/interop.rs` checks the manifest, a point's
items checked and delivered live on load and unload, a service across a
linked and a Luau extension (a missing and a too-old provider), and a
window opened as a sheet and as an OS window.

## Inline embeds

An extension draws a block of a document, inside it, where the block
is: a fenced block by its language, a note by its frontmatter `type:`,
an embed (`![[…]]`) by the type of what it points at. It is how Keyflow
shows a chart in a note and Studio a song card in the Vault
(`task:crates/widgets/src/lib.rs` `WidgetRegistry` is the shape it
backs). An embed is a contribution like the others: declared, or sent at
run time, and withdrawn when its extension unloads, the plain block
coming back.

```toml
[[embeds]]
id = "kf"
title = "Keyflow chart"
page = "pages/kf.tsx"         # or view = "chart": one of its [[views]]
claims = { fence = ["kf"] }   # also note_type = ["song"], embed_type = ["setlist"]
max_height = 420              # points; past it, it scrolls (480 when unset)
fill = false                  # true: a note it claims by type is the view
```

(`features/extensions/src/contrib.rs` `EmbedDecl`, `EmbedClaims`,
`Claim`; checked by `Contribution::check`: one view or one page in its
package, at least one claim.)

`fill` matters for a note claimed by its `type:`. Unset, the view is a
card at the note's head, over its text. Set, the view is the note's
page: the note's own heading, properties and text are out of its way,
as for a setlist or a song a view draws whole.

**The host's side.** An app asks
`features/extensions/src/embeds.rs` `Embeds::resolve` (`Host::embeds`)
whether a block is claimed — the first loaded extension that claims it
wins — and mounts `EmbedView` where it is:

```rust
let claim = Claim::fence(&info).unwrap();      // the info string's first word
rsx! {
    EmbedView { claim, source: body, props: json!({ "info": info }),
        fallback: rsx! { PlainFence { info, body } } }
}
```

`source` is the block's text, `props` the host's own properties (a
note's frontmatter, the fence's info string); the view is drawn again
when either changes. With nothing claiming it, or once its extension
goes, `fallback` shows. `EXT_EMBEDS` lists the claims, or answers which
extension draws one.

**How it is drawn** follows the view:

| the view is | it mounts | its size | its input |
|---|---|---|---|
| linked in | a component in the host's own document; its events are the document's | laid out by the host's document, up to `max_height` | `linked::use_embed` |
| a process's | a `RemoteDocument` sub-document (`app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteEmbed`) | its content's, measured | `linked::use_embed`, sent as `wire::EMBED_EVENT` to its root |
| a WASI component's | as a process's | its content's, measured | `architect_extension_wasi::embed()` (`features/extensions/wasi-guest/src/lib.rs` `embed`), the same `wire::EMBED_EVENT` |

**Content-sized.** A pane's sub-document is as tall as its pane; an
embed's is laid out at the host's width with its own height left to its
content (`RemoteDocument::fit_content`: the page as tall as what is in
it, not the viewport). The host reads that height
(`RemoteDocument::content_height`, every `remote::MEASURE_EVERY`) and
sets the embed's box to it, between `embeds::MIN_HEIGHT` and
`max_height` (`remote::embed_height`); past `max_height` the
sub-document scrolls itself. So the host lays out the height: the text
below an embed moves when it grows, as below an image.

**Events, focus and keys** go through the host document as Blitz gives
them to any sub-document: a click in the embed reaches the view's
handlers, focus follows the pointer into it, and a focused field inside
it takes the keys. A linked view is in the host's document, so it is
simply part of it.

**Theme.** A sub-document gets the host's `:root` CSS
(`RemoteDocument::set_theme_css`), live; a linked view reads
`var(--…)` from the host's own sheets. Either way a theme switch redraws
the embed in the new theme.

**Responsive.** An embed keeps to "Responsive by design": a page or
process embed runs on its own thread, its frames timed into the same
`Meters` (`<extension>/embed.<id>`, or its view's name) and marked busy
with `BusyBadge` past the budget, while the document around it answers
as ever. Nothing on the host's UI thread waits for an embed.

`app-shell:crates/app-shell-extensions/tests/remote_view.rs` checks that a
claimed fence resolves, mounts, takes its content's height and goes when
unloaded.

**The FastTrackStudios editor** renders fences through a registry of
`FencePlugin`s (`editor_state::plugin`) that return an HTML string,
synchronously, cached per pass. An embed is a live view with a size, so
backing that registry needs an editor-side hook it does not have: a
widget decoration that mounts a Dioxus element (or names a node the
host attaches a sub-document to) at the fence, a height it is told back
for its line layout, and the fence's text as it is edited. With that,
the editor's fence languages are `Embeds::fences`, and each widget an
`EmbedView`. Until then an app draws its notes itself.

**The web host** has no Blitz sub-documents: there an embed's view
would be drawn by the extension's own Dioxus tree into an element of the
page (linked views work as they are), or a page's tree rendered into
a shadow root, its height read with a `ResizeObserver`.

### Favourites

A note type's **favourites** are the few places a shell keeps one tap
away while a note of that type is open, and its one primary action: a
setlist's Setlist, Charts and Lyrics, and a round Play. The extension
that owns the type declares them, as data:

```toml
[[favourites]]
note_type = "setlist"

[[favourites.tabs]]
id = "setlist"
label = "Setlist"
icon = "list-music"            # a Lucide icon's name
target = { view = "page" }     # or { action = "ID" }, or { place = "#heading" }

[favourites.primary]
id = "play"
label = "Play"
icon = "play"
target = { action = "PLAYER_TOGGLE" }
state = "playing"              # while on, the button shows `on`
on = { label = "Pause", icon = "pause" }
```

(`features/favourites/src/lib.rs` `TypeFavourites`, checked by
`TypeFavourites::check` when the manifest is read: a note type, at most
five tabs with unique ids, labels of at most 24 characters, Lucide icon
names, a `place` relative to the note and never another site.) Nothing in
it belongs to one shell: a target is a view id, an action id or a place
relative to the open note, and whoever draws the shell resolves it (a
share link's phone shell its own views; the app's shell its views and its
Action List). A tab whose target the shell does not know is left out; a
`state` it does not know shows the first face only. A type that declares
none keeps the shell's own tabs. An app's shell opens `page` (the open
note), `search` (quick open), a view of the declaring extension's own
(`<view id>`, or `<its id>/<view id>`) showing the open note, an action it
has, and a `place` that names a note beside the open one
([the app shell](@/architecture/app-shell.md)).

**Who may name what.** A declaration names only what its own extension
may reach, as every other contribution that names an action does
(`features/extensions/src/contrib.rs` `Rules::reaches`,
`features/extensions/src/host.rs` `gate`,
`features/extensions/src/capabilities.rs` `Capabilities::may_run`):

- a view: its own extension's only. Another extension's view, loaded or
  not, is left out;
- an action: one in its own namespace (`action_prefix`: `acme.charts` →
  `ACME_CHARTS_`), or one its `capabilities.actions` names whose effect is
  at most its `capabilities.effect`. A destructive action is never a
  favourite's, its own neither: a tab's press asks no one;
- anything else is left out, as an unknown target is: no tab, never a
  dead one.

A node can list every declaration of its extensions, each with the
extension that declared it and what its capabilities let it run
(`features/favourites/src/lib.rs` `Listed`, `Declarer`), so a browser
applies the same rule (`Declarer::may_target`, `may_open_view_of`). The
rule is one function, `architect_favourites::may_run`, which
`Capabilities::may_run` calls too, and `manifest::action_prefix` is
`architect_favourites::action_prefix`: `architect-favourites` is serde
only, so the browser bundle takes it, and nothing is copied to drift. The browser reads at most 256 KiB of the
listing and 64 declarations. A note type is at most 64 characters. A
`place` takes no spaces, so a note whose name has one is out of a place's
reach.

## Project types

Architect owns the `project` kind (#344): a folder whose `project.md` is
the project, with the kind's properties (`state`, `start`, `due`, `up`,
`members`, `icon`; `features/vault/vault-live/src/property_schema.rs`
`PropertySchemaRegistry`). An extension adds **types** of it, each with
properties on top of the kind's, never in place of them:

```toml
[[project_types]]
id = "software"                     # written as the project's `projectType:`
name = "Software"
icon = "code"
description = "A codebase: its repository, its stack, its docs and releases."
properties = [
    { key = "repo", name = "Repository", kind = "link" },
    { key = "billable", name = "Billable", kind = "toggle", default = "true" },
]
template = "templates/software.md"  # in its package: a new project.md's body
folders = ["Docs", "Releases"]      # made inside a new project
views = ["page"]                    # its own [[views]] it opens with
```

A type is a contribution like the rest (`features/extensions/src/contrib.rs`
`ProjectTypeDecl`, `ProjectPropertyDecl`): checked at load (an id of
lowercase letters, digits, `-` and `_`; its template inside its package;
each property once; its views its own), its template's text read from the
package when it loads (`ProjectTypeDecl::template_text`, so a linked
package with no folder hands it out too), and withdrawn when the
extension unloads. A type two extensions add is the first loaded's. The
registry is `Contributions::project_types` / `Contributions::project_type`;
the app and other extensions read it with `EXT_PROJECT_TYPES {id?}`
(`features/extensions/src/contrib_actions.rs`), which answers each type
with the extension it is `from`. An in-process extension adds types with
`Builder::contribute(Contribution { project_types, .. })`.

## One trait, three ways to run it

`features/extensions/src/linked.rs` `Extension` is a crate's extension
in Rust: its manifest's text, and `build`, which gives a `Builder` its
actions (`action`, `action_in` with the invocation), its views (Dioxus
components), what it adds past its manifest (`contribute`), and its root
context (`context`). Its `Builder::handle` is how its code adds at run
time, the same linked in or not.

- **Linked in.** `Extensions::with_linked` registers it as a built-in
  (`Origin::Builtin`): when that copy runs, the host runs it in process
  (`Host::linked_view` gives its views' components to
  `ExtensionView`). It is turned off and on like any extension
  (`EXT_DISABLE`, `EXT_ENABLE`), and an installed or development copy of
  its id overrides it.
- **As a process.** Its crate's `main` is
  `Guest::extension(&Board).serve()`; its manifest's `[entry]` names
  that program. A development copy of the same id runs it out of
  process, its views drawn remotely, and the app cannot tell the
  difference in what it adds.
- **As WASM**: not yet against the same trait. A WASI component has a
  guest crate of its own of the same shape (`architect-extension-wasi`,
  "WASI components" below); a `linked::Extension` built to
  `wasm32-wasip2` is a later step.

`cargo xtask builtins` (CI) builds every built-in
extension crate (an `extension.toml` with a `[source]` beside a
`Cargo.toml`) both ways: its `--lib`, linked in, and its `--bin`, the
process. WASI is not a third way for these crates yet: the WASI runner
runs components built against `architect-extension-wasi`, and a
`linked::Extension` exported as a component is the step still to come
(#272).

### A built-in's development copy, from its source

Every user-facing feature is a linked extension (#272), and any of them
can be swapped for a development copy in a running app, and back. A
built-in says where its crate is:

<!-- doc-check: off (a path in FastTrackStudios/app-shell) -->
```toml
[source]
git = "https://github.com/FastTrackStudios/app-shell"
rev = "main"
path = "crates/app-shell-extensions/theme-picker"
# bin = "theme-picker"   # its [entry] path's file name unless given
```
<!-- doc-check: on -->

`EXT_CLONE {"id": "theme.picker"}` (no `as`) then does what
`features/extensions/src/source.rs` describes:

1. **Writes its crate** to `<dev folder>/<id>`: from the app's own copy
   of the repository when it has one (`Extensions::with_vendored`: the
   lab, built from a checkout, names that checkout), else a clone of
   `git` at `rev` under `<dev folder>/.sources`. Its `Cargo.toml` becomes
   a workspace of its own: what it inherits from its workspace resolved,
   every path dependency absolute, the workspace's `[patch]`, `[profile]`,
   `Cargo.lock` and `.cargo/config.toml` with it, so it builds against the
   same crates the app was built with.
2. **Builds its process** (`cargo build --bin …`, in one target folder for
   all copies, `Extensions::with_build_target`, `<dev folder>/.target`
   unless given) on a thread of its own. The program goes to its
   `[entry] path`, swapped whole. It is the app's `cargo` (`$CARGO`, else
   the one on its `PATH`): run the app from the repository's dev shell, so
   it is the toolchain the crate was written for.
3. **Overrides the built-in live**, under the built-in's own id: it
   replaces it, so its program answers the ids the built-in had (a
   renamed copy of a process extension is still refused). Until the first
   build is good the built-in runs; the extensions page says "built in
   (0.1.0), overridden by 0.1.0 (in development): building… (the built-in
   runs until it builds)", with **Use the built-in** (`end_dev`).
4. **Rebuilds on save.** Every poll (`Extensions::reload_changed`)
   fingerprints the crate's sources (not `target/`, its program or its
   lockfile); a change rebuilds it, and a good build restarts its process
   on the new program. A build that fails keeps the last good program
   running and says why: a problem in `APP_STATUS` (`theme.picker: its build
   failed: src/lib.rs:12:5: error[E0425]: … (the last good build runs)`,
   its file), its error lines in `EXT_LOGS` (source `build`), and
   "failed: …" on the page. `EXT_INFO` and `EXT_CLONE` answer the build
   (`build`: `building`, `built` with `took_ms`, `failed` with `error`).

The removed Architect app's Board example ran the whole loop in a test:
clone, first build, an edit swapped in, a break kept on the last good
build, back to the built-in.

### What each runner costs

An example extension's state action and its view's frame, and the
nearest equivalents on the scripted runners (measured on the removed
Architect app's Board example):

| runner | what | n | p50 | p99 |
|---|---|---:|---:|---:|
| linked | action | 5000 | 2.1 µs | 2.9 µs |
| linked | view frame | 2000 | 13.9 µs | 17.1 µs |
| process | action | 2000 | 29.5 µs | 105.5 µs |
| process | view frame (open to first frame) | 225 | 24.9 ms | 33.2 ms |
| Luau | action (the same JSON) | 5000 | 252.7 µs | 547.1 µs |
| `QuickJS` | view frame (the Board's TSX page, open to first frame) | 30 | 25.1 ms | 26.1 ms |

Measured 2026-11-03 on THEBATTLESHIP (the dev profile, which is
optimized). Luau has no equivalent of the Board's view, and a page calls
the Board's own action (the linked row).

A linked view's frame is its component rendered in the app's own
document; a process's is the same component rendered in its process,
sent over the socket and applied here; the `QuickJS` row is the Board's
tab as its TSX page, opened and run until its first frame. The process
boundary costs tens of microseconds per action call: fine for anything a
person does, wrong per frame or per sample, which is why hot paths stay
native inside the linked extension. Opening a remote view costs about
25 ms to its first frame, a mount, not a redraw (both remote rows land on
it, which says it is the mount's fixed cost, not the view's work).

### How many views a process shows

Up to `MAX_VIEWS` (256) at once (`features/extensions/src/wire.rs`
`MAX_VIEWS`). Each open view is a request in flight on the process's
stream lane, held while the view lives, and vox caps those per lane (64
by default): the host opens the lane with room for `MAX_VIEWS` and the
call lane (a view's events and its close) with four times that, so the
lanes are never what stops a view. One more than `MAX_VIEWS` is refused
with an error that says so, shown in the pane's placeholder; closing a
view makes room once its request ends in the process
(`Process::open_views` counts them).

A process at wire 6 or later (`VIEWS_WIRE`) ends a view's request when
the view closes, and the host awaits that answer instead of dropping the
call (a dropped vox call sends no cancel, and its answer arriving later
would close the connection). An older process holds every view's request
for as long as it runs, so its host opens at most `MAX_VIEWS` of its views
over its connection's life, closed ones counted, then says to restart or
rebuild it. Before this, the 65th view at once waited forever, and a
process stopped showing views after 64 opens. `features/extensions/tests/process_views.rs` shows 100 views at once,
400 more opened and closed, 256 at once, the 257th refused, and the
place a closed one leaves taken again.

### The rule: a new user-facing feature is a linked extension

A feature a person sees (a page, a picker, a palette source, a settings
section of its own, a tab) is written as a `linked::Extension` in a crate
of its own with an `extension.toml` and a `[source]`, uses only what a
third party could (its manifest's tables, its builder, the host
service), and is linked into the app with `Extensions::with_linked`. So
it can be turned off, cloned to a development copy (`EXT_CLONE`) that
overrides it live, and put back. Reach the app's state through the host
service (`linked::Handle::host`: the app's actions), not through Rust
handles to the app's internals: then its process copy does the same.

**The core** stays compiled in, not extensions: workspace and view kinds
(the `paint` kind included), the shell's slots (`Chrome`), the window
machinery (sheets and OS windows, "Windows, points and services"), the
theme store and its `THEME_*`, the Action List and the control socket,
CRDT documents, settings storage, the settings page's frame
(`architect_settings::SettingsSheet`) and the palette's `#`
(`SettingsPage`), input, the extension host and its host service, native
host kernels.

**Built in now:**

| built-in | crate | adds | its dev copy |
|---|---|---|---|
| `theme.picker` | `app-shell:crates/app-shell-extensions/theme-picker` | ⌘K's "Choose a theme…" and "Light or dark", ⌘O's `theme:` | both ways in its `tests/both_ways.rs` |
| `manager` (the extensions page) | `features/extensions/src/manager` | `MANAGER_*`, its views, its settings section, ⌘O's `ext:` | none yet: its kinds are the app's Rust values (below) |
| `settings` (the settings page) | `app-shell:crates/app-shell-extensions` (`settings_sheet`) | the window ⌘, opens (`APP_SETTINGS`, reserved), the point `settings.sections` | both ways in app-shell-extensions' `tests/settings_sheet.rs` |


**The settings page** is the settings extension's window over the
core's frame. Every section arrives as an item of `settings.sections`:
the app's own (Workspace, Editor, Keyboard, and Appearance from the
theme store, which the app draws with its own page) through
`use_extension_ui` (`window_ui::use_settings_window`, which also opens
the window on `SettingsHandle::show` and closes the page with it), and
each extension's `[[sections]]` (the extensions manager's, a
metronome's). Linked in, it draws them over the app's values and
section pages; its development copy draws them in its own process over
the values the host service gives it (`APP_SETTINGS_VALUES`,
`APP_SETTINGS_SET`), a section drawn by another extension's view named,
not drawn (a remote document cannot hold another view yet).

**Not moved yet, each waiting on a decision:**

- **The settings page** moved (2026-11-04): extensions own windows
  (`[[windows]]`), an app lets one own an `APP_…` id (`reserved`), and
  its sections are a contribution point. The palette's `#` stays the
  core's (it searches the app's settings registry).
- **The manager's dev copy.** Its kinds (extensions, scripts, workers,
  themes, layouts, pages) are Rust values the app registers, so a process
  copy has nothing to list. Its `MANAGER_*` would move to the host (the
  core already owns what they read) and its page would read them through
  the host service; the page is then the built-in.
- **The sandbox's pages** (Inbox, Tasks, Schedule, Vault and their item,
  capture and help routes). Their state (`Lab`: the route, the lists,
  undo and redo, the selection) is the sandbox's root's, and every lab
  command reads it. Linked, a page could read it from context; a dev copy
  could not, so it needs that state to move into the extension (the lists
  as a document of its own, the route as the workspace's), which is the
  sandbox's rewrite, not a move.
- **The Appearance section** stays the app's: its rows are the theme
  store's layers kept as settings (`appearance.*`, in no extension's
  namespace), contributed to `settings.sections` as `app` and drawn by
  the app's page.

## First-party extensions: the API, linked in

So the rule is: **write auxiliary features against the extension API,
ship them linked in.** A first-party extension is a crate that uses only
what a third party could (actions, streams, views, layers, settings,
openers) and is compiled into the app by a cargo feature, called
directly: no boundary, no serialisation, native text input. The same
crate can be built as a WASM component or a process when that is worth
it (to load it later, to sandbox it, to hot-reload it in development).

That gives what a "light core" was for, without the costs:

- the core knows nothing about the feature: an app without the feature is
  a build without the crate, and the feature can be turned off at runtime
  like any extension;
- the extension API is proven by the app's own features (VS Code builds
  its own features as extensions for the same reason): if a first-party
  feature needs something the API lacks, the API grows, for everyone.

**A built-in can be overridden.** A linked-in extension is an extension
like any other to the app: it has an id and a version. Installing one
with the same id — a newer version from the registry, or a development
copy (`architect ext dev`) — shadows the built-in until it is removed;
the extensions page shows "built in, overridden by 1.4.0-dev" with a
button back. So the team ships a feature in the app and still tries the
next version of it in a running build, and a person can take a fix
before the next app release.

Good first-party extensions are auxiliary and not realtime: chart and
lyric views, importers and exporters, publishing, overlays, analytics.
The engine, the timeline, transport and DSP stay core.

## Capabilities, the host service and consent

An extension's manifest names what it needs in `[capabilities]`: the
action namespaces it may run past its own (`actions`) and the most they
may change (`effect`: read-only, mutating, destructive), what of the
app's state it may read (`reads`: `org`, `selection`, `documents`,
`editor`, `files`: the images and lyrics of a note its view draws, #669)
and change (`writes`: `documents`, `editor`, below), the
app's interface it may use (`ui`: below), the folders past its own,
the network hosts, the MIDI ports and OSC endpoints (`ports`: "MIDI,
OSC and control surfaces", below), and what the host may do with its
languages (`languages = ["servers"]`: run their servers, "Languages for
the editor", below), and the programs in its package its `process` views
run (`programs = ["bin/room"]`, one by one: "Run bin/room, a program in
its package, on this computer"; checked each time a pane opens, never a
share-link guest's; `docs/content/architecture/process-views.md`)
(`features/extensions/src/manifest.rs` `Capabilities`;
`features/extensions/src/capabilities.rs` turns them into the lines a
person reads, `Capabilities::grows_from`, and into what may run,
`Capabilities::within`).

**The host service** is what an extension asks the app for, the same
over every way it runs (`features/extensions/src/hostcall.rs`
`HostRequest`, `HostAnswer`, JSON): run an app action, read the current
organization or the selection, open, read and change documents (below),
read and write files (its own folder, or `capabilities.folders`), read and write its
own settings and data, log a line. `Host::api` gives one extension's
session (`features/extensions/src/hostapi.rs` `ExtensionApi::call`),
checked against what it may reach now. A process calls it over its own
connection (the host serves `features/extensions/src/host_proto.rs`
`ExtensionHost`; the guest calls `guest::host`), a Luau script through
its `host` global (`features/extensions/src/luau_host.rs`), an
in-process extension through `linked::Handle::host`; the WASM runner
maps one JSON-in, JSON-out import onto the same call. What the app knows
comes from its `hostapi::AppContext` (organization, selection,
documents) and `SettingValues`; data and files live under
`Host::with_data_root`; `Host::set_app_list` names the app's list when
extensions have one of their own.

An action an extension runs is run as `Principal::Extension { id,
on_behalf_of }` (`features/permissions/src/lib.rs`): the person or agent
whose run it is part of, or `None` for the app itself. A list's
authorizer or a server's gate tells it apart from the person's;
`RoleEngine` gives it at most the rules of `on_behalf_of`. Acting for
the app itself it is not asked to confirm (the person granted it on
install); acting for an agent it asks as the agent would. Each call is
a span, `ext.host_call`, with `perm.capability`, `perm.decision`,
`perm.reason` and `perm.principal` (set with
`architect_telemetry::wide::set` too); a refusal is `HostError::Refused
{capability, why}` and a line in the extension's log.

**Consent** (`features/extensions/src/consent.rs` `Grants`): an
installed extension runs with what it asks *and* a person granted, kept
per device or per organization (only while it is the current one) in a
file the app names (`Host::with_grants`), revocable at once
(`Grants::revoke`): the next call, hook or script is refused what it
lost. A built-in or a development copy runs with what it asks. Install
shows the capabilities (`EXT_PREVIEW`, the consent prompt); an update
whose capabilities grow waits for a yes (`EXT_UPDATE`'s
`needs_consent`). An agent asking to install with a grant, or to grant,
is answered `confirmation_required` with a ticket at once, never left
waiting (`Extensions::confirm_on`).

| action | what it does |
|---|---|
| `EXT_PREVIEW` | what a version would be and do, before it is installed: kind, runner, capabilities and which need a yes, contributions, hooks, network and files, its git ref |
| `EXT_INSTALL` | `{id, version?, grant?, pin?}`; without a grant one that asks for more answers `needs_consent` |
| `EXT_UPDATE` | to the newest each pin allows; unchanged ones skipped |
| `EXT_PIN` | keeps an install at a version range, or `unpin` |
| `EXT_GRANT` / `EXT_REVOKE` / `EXT_GRANTS` | a person's yes, taking it back, and what each may reach now |
| `EXT_LOGS` | `{id, since?, limit?}`: a process's stdout and stderr, Luau prints and errors, failed hooks, refused calls |
| `EXT_HOST_CALL` | one host-service call as an extension, for the caller |
| `EXT_PAGE_CALL` | one host-service call as the extension running it (a page's way to the interface below) |
| `EXT_DECORATIONS` | the icons and badges on rows and tabs, and who set each |
| `EXT_INFO` | which copy runs, versions, pin, grants, what it may reach |

## The app's interface: notifications, the clipboard, pickers, prompts, menus, badges, overlay

What Templater, QuickAdd, Importer, Iconize and js_ReaScriptAPI need of
the app's interface, as host-service calls
(`features/extensions/src/hostcall.rs` `HostRequest`), each refused
unless the manifest asks for it in `capabilities.ui` and a person
granted it (`features/extensions/src/capabilities.rs` `UI`; the consent
prompt says each in words):

```toml
[capabilities]
ui = ["notify", "clipboard", "pickers", "prompts", "menus", "badges", "overlay"]
```

| call | what it does | answer | needs |
|---|---|---|---|
| `notify {text, kind?, title?}` | a toast (`info`, `success`, `warning`, `error`), and a line in its log | `null` | `notify` |
| `clipboard_read` / `clipboard_write {text}` | the clipboard's text | the text, or `null` | `clipboard` |
| `pick {kind, title?, filters?, folder?, name?}` | a picker (`open`, `open_many`, `folder`), a save dialog (`save`), or the person's photos (`photo`, `photos`: a copy of each image made for it) | a path, a list (`open_many`, `photos`), `null` when cancelled | `pickers` |
| `prompt {kind, title, message?, default?, placeholder?, choices?}` | asks for text (`text`), a yes or no (`confirm`), one of a list (`pick_one`) | the text, `true`/`false`, the choice's id; `null` when cancelled | `prompts` |
| `menu {items, x?, y?}` | a context menu of its own items (`{id, label, icon?, shortcut?, disabled?, separator?}`) at a point in the window (none: its middle) | the id chosen, `null` when dismissed | `menus` |
| `decorate {target, decoration?}` | an icon or badge (`{icon?, badge?, color?, tooltip?}`) on a file-tree row (`{kind: "file", id: <path>}`) or a tab (`{kind: "tab", id: <view kind or pane id>}`); none takes it off | `null` | `badges` |

**`overlay`: drawing over the whole app.** It is not a call. It changes
how the extension's views draw: the scenes its remote views paint
(`features/extensions/src/scene.rs`) are not clipped to their panes, and
draw over the whole window, above the other panes, its toolbars and its
dialogs. The consent prompt says "Draw over the whole app, outside its
own panes". The owner accepts the risk that comes with it, so an
extension granted it could paint something that looks like the app's
own prompt. Without it, a scene is clipped to its pane, so a view that
draws past its box can't smear over the app by mistake (`scene::framed`;
a linked paint view is clipped the same way, `PaintWidget`).

How it's drawn: the host adds a widget filling the window, above the
panes and taking no input (`scene::OverlayLayer`, `OverlayWidget`, put
there by `remote::RemoteView` given `overlay`, which `interface`'s
extension view reads from `Host::capabilities`). It draws each of the
view's scenes at its element's place in the window. The element in the
pane keeps its size and its input.

`pick`, `prompt` and `menu` wait for the person: the call answers when
they do. A path picked is one the extension may then read for the run (a
save, or inside a folder picked: write), past `capabilities.folders`.
On a desk the pickers are the platform's own (rfd: the portal on Linux,
the system's panels on macOS and Windows), and a photo pick is a file
picker for images. On iOS (#708, `app-shell:crates/app-shell-extensions/src/pickers_ios.rs`)
they are UIKit's: `UIDocumentPickerViewController` opens a file as the
app's own copy and a folder through its security scope (kept for the
run); a save is a folder picked and the suggested `name` in it (iOS saves
by exporting a file that exists); `photo` and `photos` are
`PHPickerViewController`, which runs outside the app, so no photo-library
permission is asked, and each image picked is written to the app's
temporary folder as a plain JPEG, without its metadata (EXIF, the camera,
GPS), and that copy's path answered. A save's `name` is one plain file
name in the folder picked (`features/extensions/src/hostcall.rs`
`save_path`: an absolute name, a slash, `.` or `..` is `Untitled`), and
the folder's security scope is closed when the extension unloads.
Every runtime makes the same calls: Luau's `host.notify`,
`host.clipboard_read`, `host.pick`, `host.prompt`, `host.menu`,
`host.decorate` (`features/extensions/src/luau_host.rs`; a cancel is
`nil`); the WIT world's typed imports `notify`, `clipboard-read`,
`clipboard-write`, `pick`, `prompt`, `menu`, `decorate`
(`features/extensions/wit/extension.wit`; the Rust guest's
`features/extensions/wasi-guest/src/lib.rs` `host::notify` and the
rest); a process's `guest::host` and a linked extension's
`linked::Handle::host` with the requests themselves.

**The app answers** through its `HostUi`
(`features/extensions/src/hostui.rs`; `Host::set_ui`):

- `hostui::NoUi`, the default: a notification is a log line, the rest
  `HostError::Unsupported` (a headless host has no one to ask);
- `hostui::FakeUi`, for tests and headless runs: it keeps the
  notifications and a clipboard of its own and answers at once — the
  answers it was given (`FakeUi::answer`), or a text prompt's default, a
  confirm's yes, a list's first choice, a menu's first item that may be
  chosen, a picker cancelled;
- the window's, `app-shell:crates/app-shell-extensions/src/host_ui_view.rs` `WindowUi`:
  `HostUiLayer` (mounted by `ExtensionOverlays`) draws toasts at the
  window's corner, a prompt as a dialog, a menu at its point, and lends
  the host its `WindowUi` unless the app gave a `HostUi` of its own
  (`WindowUi::lend`). The clipboard is the system's
  (`hostui::system_clipboard_read`: `wl-paste`, `xclip`, `pbpaste`); the
  pickers are rfd's (feature `pickers`, on in the sandbox), the same the
  shell's file inputs use — the portal on Linux, the system's panels on
  macOS and Windows. Without the feature, on a phone, or in a browser
  (`features/extensions/src/web.rs` `WebHost`), a pick is
  `HostError::Unsupported`.

**Icons and badges** are a contribution point other features consult:
`app.decorations` (`hostui::DECORATIONS_POINT`), which a manifest
contributes to statically (`[[contribute."app.decorations"]] target = {
kind = "file", id = "README.md" }, icon = "star"`), and what extensions
set at run time with `decorate` (`Host::decorations`,
`hostui::Decorations`: one decoration an extension a target, gone when
it unloads). `Host::decorations_of(target)` is both, in order; a file
tree draws them after a row's name with `host_ui_view::Decorations
{host, target}` (`app-shell:crates/app-shell-extensions/tests/host_ui_view.rs`, its golden
`badged-file-row.html`), and the workspace's tabs through
`architect_workspace::TabBadges`, which `use_extension_ui` provides.
`EXT_DECORATIONS` lists them all, with who set each.

A context menu here is imperative: an extension shows one when it
decides to (after its own action ran, say), with items it makes then.
The rows an extension adds to the app's own context menus are
declarative (`[[context]]`, "Contributions").

Tests: `features/extensions/tests/host_ui.rs` (Luau, refusals, a picked
file, `EXT_PAGE_CALL`, a headless host), `features/extensions/tests/wasi.rs`
(`a_component_notifies_copies_asks_and_gets_the_answers_back`).

## The device: OS notifications, the camera, photos, health (#709)

An extension asks for the device's own services in `capabilities.device`,
one entry each, shown one line each in the consent prompt and granted
entry by entry:

```toml
[capabilities]
device = ["notifications", "camera", "photos", "health:read:steps", "health:write:dietary_energy"]
```

Health data is an allow-list: each type is named, read and write apart,
from `features/extensions/src/device.rs` `HEALTH_TYPES` (`steps`,
`active_energy`, `dietary_energy`, `protein`, `carbohydrates`, `fat`,
`water`, `distance`, `body_mass`, `heart_rate`); a manifest naming any
other entry does not load.

| call | what it does | answer | needs |
|---|---|---|---|
| `os_notify {id?, title, body?, at_ms?, after_s?}` | an OS notification: now, at a time (ms since the epoch) or after some seconds; the same `id` replaces it | `{id}` | `notifications` |
| `cancel_os_notify {id}` | takes one it sent or scheduled back | whether it had one | `notifications` |
| `capture {kind?}` | the camera: the person takes a photo | the image's path, `null` when cancelled | `camera` |
| `photos {limit?, since_ms?}` | the newest photos in the library (20, at most 200) | `[{id, created_ms, width, height}]` | `photos` |
| `photo_file {id}` | one of them as a file copied for it | its path, `null` when there is none | `photos` |
| `health_read {type, from_ms, to_ms, sum?}` | one type over a span: its sum (a summed type's default) or its samples | `{type, unit, sum}` or `{type, unit, samples: [{value, start_ms, end_ms}]}` | `health:read:<type>` |
| `health_write {type, value, start_ms?, end_ms?}` | one sample, in the type's unit | `null` | `health:write:<type>` |

Every runtime reaches them through its generic call (Luau `host.call`,
WIT `call`, a page's `EXT_PAGE_CALL`, `EXT_HOST_CALL`). A notification is
kept under the extension's id (`<extension>/<id>`), so one extension never
replaces or cancels another's; its name is the banner's subtitle; and
every one it scheduled or showed is taken back when it unloads, is
removed, or loses `notifications` (`Device::forget`,
`Host::grants_changed`). A capture or a photo's copy is a file the
extension may read for the run, as one a person picked, and a plain JPEG
drawn again from the pixels: no EXIF camera data, no GPS.

The host answers them with its `Device`
(`features/extensions/src/device.rs` `Device`; `Host::set_device`). On iOS
it is `features/extensions/src/device_ios.rs` `IosDevice` (feature
`device`): `UNUserNotificationCenter` (shown as a banner while the app is in
front), `UIImagePickerController` with the camera (none on the Simulator:
unsupported there), `PHAsset` and `PHImageManager`, and `HKHealthStore`.
The OS asks the person too, the first time each is needed: its own
prompts, after the manifest's consent. The app's Info.plist carries the
usage strings (`NSCameraUsageDescription`, `NSPhotoLibraryUsageDescription`,
`NSHealthShareUsageDescription`, `NSHealthUpdateUsageDescription`).

**Health data is the app's own extensions' alone.** Only an extension
compiled into the distribution (the Health bundle, #371) may read or
write it; an installed or development copy is refused (`device.health`,
`features/extensions/src/device.rs` `HEALTH_LINKED_ONLY`) even with the
entry in its manifest, and so is a development copy of a built-in's id.
Nor is it read or written on anyone else's behalf: a built-in's health
call made for an outside caller (an agent over MCP, a peer) or relayed
for an extension that is not built in (a service call) is refused
(`HEALTH_NOT_RELAYED`). It stays on the device unless the person saves it
to a note.

**Signing.** A phone's build is signed with the app's own entitlements
(HealthKit with an empty
`healthkit.access`, so no clinical health records, and no background
delivery), each only where the App Store profile allows it, with the
profile's identity; never the profile's whole set, which for HealthKit
offers clinical records and background delivery; the app's release
tooling checks the signed app carries exactly that set. A Simulator build is
signed ad hoc with no entitlements, and HealthKit there answers
`Missing com.apple.developer.healthkit entitlement`. A desk and a browser answer every device
call `unsupported` until they have a `Device` of their own: the shape is
the platform's business only inside it. Tests: `features/extensions/tests/host_device.rs`
(`FakeDevice`).

## Timers, events and background extensions

What Templater, Git's auto-commit, TaskNotes' pomodoro, Linter's format
on save and SWS's project triggers need (#308): time, the app's events,
and work that runs only while it is wanted. The model is
`features/extensions/src/events.rs` (wasm-clean), the host's side
`features/extensions/src/host_events.rs`.

**The clock.** The host reads the time on one `events::Clock`
(`Host::with_clock`, `Host::set_clock`): `events::SystemClock` (its
times of day `utc_offset_min` east of UTC, which the app sets), or in a
test `events::FakeClock`, moved by hand (`advance`, `set`).
`Host::tick` runs what is due on it, and the app calls it from a thread
(`Host::run_clock`, every second in the sandbox), so a timer, a schedule
and an idle timeout are tested by moving the clock, never by waiting.

**Scheduled hooks.** A hook with no `on` runs on a schedule:

```toml
[[hooks]]
every = "5m"            # 30s, 5m, 1h30m, 1d; a second at least
action = "ACME_SYNC"

[[hooks]]
at = "09:30"            # every day, the clock's time of day
script = "daily.luau"

[[hooks]]
at = "0 9 * * 1-5"      # five cron fields: minute hour day month weekday
action = "ACME_STANDUP"
```

`events::Schedule` reads them and says when each is next due; an `every`
hook first runs one interval after its extension loads. A run is a
hook's run as any other's (one at a time, inside its capabilities), its
invocation's event `schedule` and its detail `{every | at, due_ms}`. A
host asleep past several runs runs a hook once, then on from then.
`Host::next_runs` says when each will run.

**Document and app events.** Past `startup`, `workspace.changed`,
`theme.changed` and the rest (`contrib::EVENTS`):

| event | detail | raised by |
|---|---|---|
| `doc.created`, `doc.saved`, `doc.deleted` | `{path}` | the layer that did it: `Host::raise_doc`, or `events::DocEvent::raise_on` the app's Action List (a `Changed` event the host hears through `Host::watch`; the document store raises them there without depending on extensions) |
| `doc.renamed` | `{from, to}` | the same |
| `app.focused`, `app.blurred` | `{window}` | `Host::focus`, or the list topic of the same name (an app's window raises it) |

The detail is JSON, so a Luau hook reads `event.detail.path`; an
action's invocation has it as text (`ctx.detail`). `EXT_EMIT {event,
detail}` raises any of them by hand (a detail that is a JSON object
stays one).

**Events the app defines.** Studio's transport and project events are
the app's own: it defines each once (`Host::define_event` with an
`events::AppEvent`, or `Host::define::<E>()` for a type implementing
`events::TypedEvent`) and raises it typed (`Host::raise(&event)`, its
value the hooks' detail; `Host::raise_app` with JSON). An extension
hears one with a hook on `app:<name>`, and only when its manifest asks
for it and a person granted it:

```toml
[capabilities]
events = ["transport.*"]          # a name, <prefix>.*, or *

[[hooks]]
on = "app:transport.changed"
script = "transport.luau"
```

A hook on an `app:` event the manifest does not ask for is refused as it
loads (and at run time); one an installed extension was not granted is
not run, and says so in its log. Raising an event that was not defined
is an error. The consent prompt says it in words ("Hear the app's
transport.* events").

**Events an extension raises.** An extension raises its own events with
`HostRequest::Raise { name, detail }`: the host defines `<its id>.<name>`
the first time (`Host::raise_own`) and raises it, so another extension
hears it as `app:<its id>.<name>` under the same `capabilities.events`
grant. A navigator, for one, raises `navigator.selection-changed`.
Raising needs no grant: an extension's events are its own.

**Hooks made at run time.** The host service's `subscribe {on | every |
at, action | script, function?}` adds a hook, checked as a manifest's
are, and answers its key; `unsubscribe {key}` takes it off; `events`
says what it can hear (`Host::subscribe`, `Host::events_for`). Every
runtime makes them: Luau's `host.subscribe{ on = "doc.saved", action =
… }`, `host.unsubscribe(key)`, `host.events()`
(`features/extensions/src/luau_host.rs`); the WIT world's `subscribe`
and `unsubscribe` imports (`features/extensions/wit/extension.wit`; the
Rust guest's `host::subscribe`); a process's and a linked extension's
host requests. They go when it unloads.

**Timers in a WASI view.** The world has the host's clock as an import
(`now`) and a `tick` export: the guest keeps its timers
(`features/extensions/wasi-guest/src/lib.rs` `timers::set_interval`,
`set_timeout`, `clear`, and the hook `timers::use_interval`), the host's
actor calls `tick` when the next is due (it answers how many ms until
the one after) and sends what the timers changed as the view's edits
(`pending`), as #302's Luau page workers' `page.tick` does. A timer set
while a view renders or handles an event is that view's: it runs inside
its `VirtualDom`, so it writes the view's signals, and goes when the
view closes. `wasi::Stats::tick` runs what is due at once, for a test
that moved the clock. In a browser the page's clock and `setTimeout`
drive the same `tick` (`features/extensions/web/runner.js`).

```rust
fn Ticker() -> Element {
    let mut seconds = use_signal(|| 0);
    architect_extension_wasi::timers::use_interval(1000, move || seconds += 1);
    rsx! { "{seconds} s" }
}
```

**Background extensions.** `[background]` makes a code extension start
on its first event, not as it loads, and stop once it has been idle:

```toml
[background]
idle = "5m"     # the default
```

Its actions are on the list and its hooks registered from the start;
the first action run or hook to fire starts its instance
(`features/extensions/src/host_events.rs` `Lazy`), with the state its last one saved. Once no call
has run in it for `idle`, `Host::tick` stops it (saving its state), and
the next event starts it again. Each start and stop is a line in its
log (`EXT_LOGS`: `started (hook doc.saved)`, `stopped: idle for 300 s`),
and what each is doing is `Host::background` (`events::BackgroundStatus`:
`stopped`, `running` or `failed`, its starts, what started it, when it
stops) and `APP_STATUS`'s `background` section. It works for every
runner the same way: a process is not spawned, a component not
instantiated, a Luau runtime not made until it is needed.

Tests: `features/extensions/tests/events.rs` (on a fake clock: an
`every` and an `at` hook, one made with `host.subscribe`; a save raised
on the host and on the list, the window's focus, a typed app event and
its refusals; a background extension started by a hook, kept by an
action, stopped idle, started again), `features/extensions/tests/wasi.rs`
(`a_views_interval_runs_on_the_hosts_clock_and_a_hook_it_made_runs`).

## Documents: list, create, edit, rename, move, delete, watch

What Templater, Dataview, Tasks, Kanban, QuickAdd, Linter and an importer
need (#309): the app's documents, not raw files. The model is
`features/extensions/src/documents.rs`; the calls are the host
service's, so every way an extension runs has them: a Luau script's
`host`, a page's `documents`, a component's typed imports, a process
over vox, `EXT_HOST_CALL`.

| call | Luau `host.` / a page's `documents.` | answer | needs |
|---|---|---|---|
| `list_documents {under?}` | `list_documents(under?)` / `list(under?)` | `[{path, name, space}]` | `reads = ["documents"]` |
| `read_document {path}` | `read_document(path)` / `read(path)` | its text | `reads` |
| `create_document {path, text?}` | `create_document(path, text?)` / `create` | its info | `writes = ["documents"]` |
| `edit_document {path, edits, name?}` | `edit_document(path, edits, name?)` / `edit` | its text after | `writes` |
| `rename_document {path, to}` | `rename_document(path, to)` / `rename` | its info | `writes` |
| `move_document {path, folder}` | `move_document(path, folder)` / `move` | its info | `writes` |
| `delete_document {path}` | `delete_document(path)` / `delete` | nothing | `writes` |
| `undo_document {path}` / `redo_document {path}` | `undo_document(path)` / `redo_document(path)` | `{step, text}`, or `null` with none | `writes` |
| `watch_documents {action \| script, function?}` | `watch_documents(hook)` / `watch(hook)` | the hooks' keys | `reads` |

A path is relative and `/`-separated (`notes/Inbox.md`), never `..`; its
first folder is the space the app keeps it in. **An edit is a
transaction**: `edits` (`documents::DocEdit`: `{insert: {at, text}}`,
`{delete: {at, len}}`, `{append: {text}}`, `{replace: {text}}`, positions
in characters) apply in order as one CRDT commit and one undo step named
`name` (`<extension>: edit` unless given); one that falls outside the
text changes nothing. It is made as the extension's
`Principal::Extension` (the commit's message, so a peer tells it from
the person's), and it merges with the person's concurrent typing
character by character. A `replace` is diffed into edits, so a
concurrent edit elsewhere in the text survives it.

```lua
host.create_document("notes/Inbox.md", "# Inbox\n")
host.edit_document("notes/Inbox.md", {
    { insert = { at = 0, text = "---\ntidy: true\n---\n" } },
    { append = { text = "- [ ] call Sam\n" } },
}, "Add a task")                                -- one undo step
host.move_document("notes/Inbox.md", "archive")  -- archive/Inbox.md, its history kept
host.watch_documents({ action = "ACME_REINDEX" })
```

**Watching** is the document events (#308): the host raises
`doc.created`, `doc.saved`, `doc.renamed` (a rename or a move, `{from,
to}`) and `doc.deleted` for each call that changed something
(`Host::raise_doc`), and the app raises its own for the person's
changes, as it always did (`DocEvent::raise_on`). `watch_documents`
hooks one of its actions on all four (`Host::subscribe`); a manifest's
`[[hooks]] on = "doc.saved"` hears them too. A hook that edits the
document it heard is heard again: it checks what it already did.
Hearing a document's events is reading the documents, so every hook on
one needs `reads = ["documents"]`: a manifest that declares one without
it is refused as it loads (`contrib::Rules::reads`), a `subscribe` for
one is refused (`hostcall::check`), and an installed extension a person
did not grant `documents` is not given them (`Host::fire`; a line in its
log). `doc.opened`, the app's own event for a tab it opened, is not one
of them.

**The app's side** (`features/extensions/src/documents.rs` `Documents`):
what an app implements to give extensions its documents, set with
`Host::with_documents` (or `Host::set_documents`): `list`, `read`,
`create`, `edit` (the transaction, its step's name, who made it),
`rename`, `delete`. The host has checked the capability and the path
before it calls, and raises the events after; a move is a rename. An app
whose documents are `crdt` documents uses
`documents::CrdtDocuments`: each document a `CrdtDoc` with its own
`History`, in named spaces (`with_space`), its text the root text
`CrdtDocuments::TEXT`, its document there for the app's editor and sync
(`CrdtDocuments::doc`), its undo (`undo`, `redo`, `steps`) and who
changed it (`authors`). With no documents given, the calls answer
`unsupported`; `read_document` falls back to `AppContext::document`.

**A note's parsed metadata** is the read-only action `VAULT_METADATA`
(`features/vault/vault-obsidian/src/metadata.rs` `metadata`): `{path}`
answers one note's headings `{level, text, line}`, list items `{line,
text, depth, parent, ordered, task, done, block, section}`, its tasks
(indices into the list items), block ids `{id, line}` and inline fields
`{key, value, line}` (`key:: value`, `[key:: value]`, `(key:: value)`);
`{folder, limit?, cursor?}` answers a page of the notes' under `folder`
(`""`: the whole vault) in path order, `{notes, next}`, at most `limit`
(100 by default, 1000 at most) after `cursor`. Neither is refused: a
caller never parses the whole vault without asking. Each note is parsed
outside the vault's lock and kept by its modified time and length, so a
note is parsed once until it changes. The parser follows CommonMark's
fences (a closing run at least the opening's, of its character, nothing
after it; an inline ```` ``` ```` opens none), counts frontmatter only
when it closes, and leaves out `%%` comments, `$$` math and indented
code. It is an action, like `VAULT_READ` and `BASES_ROWS`, not a host
call: an extension runs it with `capabilities.actions = ["VAULT_"]`,
and so do the CLI, MCP and the node, with no runtime-specific binding;
the document calls stay what every app's document set has, and metadata
is the vault's.

**The vault's actions are the documents.** An extension runs a
`VAULT_*` action only with `reads = ["documents"]` (a read-only one:
`VAULT_READ`, `VAULT_LIST`, `VAULT_SEARCH`, `VAULT_METADATA`) or
`writes = ["documents"]` (the rest), past its `actions` grant
(`capabilities::VAULT_ACTIONS`, `Capabilities::may_run`), so the consent
prompt says "Read your documents" or "Change your documents" for it and
an `actions = ["VAULT_"]` grant alone reads no note.

**One service, through its client.** The document calls and an
extension's data are one `#[architect::rpc]` service,
`architect_documents::Documents` (`features/extensions/documents`,
wasm-clean): `list`, `read`, `create`, `edit`, `rename`, `delete`,
`metadata` (`VAULT_METADATA`'s answer), `data`, `set_data`, each naming
the extension it is for. Its one implementation, `DocumentsServer`, has
the gate in it (`Gate`: `reads` / `writes = ["documents"]`), so every
transport gets the same check, and calls a `DocumentStore` and a
`DataStore` below it. The host reaches it only through the generated
`DocumentsClient` (`features/extensions/src/docs_service.rs`): it serves
`DocumentsServer` over the app's `Documents` (`AsStore`), each
extension's data (`FileData`) and its own gate (`HostGate`) with
`architect::serve_local` on a runtime of its own, the same on the desk
and iOS. A browser's host does the same in the page (`serve_local` in
wasm, on `spawn_local`) over the app's own `Documents`. The rule stays
one implementation: when the node serves `DocumentsServer` on the org
lane, the browser's host swaps the transport under the same
`DocumentsClient`, not the code. What is not wasm-clean is only below the
stores: the vault on disk and the data files.

**Extension data and settings persist.** `host.data`/`set_data` are a
file per extension under the host's data root (the app's
`<app dir>/extension-data`), written whole (a temporary file, synced,
renamed over it), readable by its owner only (0600), one change at a
time per extension; a file that does not parse is an error, never read
as empty, so a write never wipes it. Settings are the settings page's,
kept in the app's device values. Both survive a restart
(`features/extensions/tests/documents.rs`
`an_extensions_data_is_written_whole_privately_and_never_wiped`).

Tests: `features/vault/vault/tests/actions.rs` (`VAULT_METADATA`),
`features/extensions/tests/documents.rs` (an extension in in-memory
spaces: creates, tidies as one undo step, renames, files and hears each;
the person's concurrent typing merges with its transaction; refused
without `reads` / `writes`), `features/extensions/tests/wasi_wit.rs` (the typed imports are the
calls).

## A project's files on the org's node

An engine an extension runs in the app (Studio's, in a browser) streams a
project: it lists a folder and range-reads its files. The member's own
project, not only a share link's: two host calls, each needing
`reads = ["files"]` alone (`features/extensions/src/held.rs`).

| call | Luau `host.` | answer |
|---|---|---|
| `file_list {place, folder?}` | `file_list(place, folder?)` | `{place, folder, files: [{path, size, sha256, url}], expires_us}` |
| `file_url {place, path}` | `file_url(place, path)` | `{url, expires_us}` |

A place is `vault:<name>` (one of the org's vaults) or `root:<id>` (a
root the node holds). Every `url` is absolute, read with `Range` and no
header until `expires_us` (a short-lived read ticket in its query); then
ask again. The host checks the capability (`hostcall::check`, and
`features/extensions/web/check.js` in a page) and hands the call to the
app's `HeldFilesSource` (`Host::set_held_files`). The node decides what
the person may read: `features/files/src/held.rs` `HeldFiles`, served
under the org lane's gate (a member reads what their role lets them;
another org's person, a share link's guest and no bearer read nothing; a
hidden file and a symlink are nowhere). A host whose app gives no source,
a guest's included, answers `unsupported`, so a guest reads only what its link lists, through
the link (`<link>_files.json`).

## The vault's query service

The vault keeps an index of its notes, as Obsidian's metadata cache is
what Dataview, Tasks and Bases read (`features/vault/vault-query`,
wasm-clean).

**The index** (`vault_query::index`). One compact `Entry` per note: its
title, frontmatter properties, inline tags, body links, headings and task
lines, never its prose; and one per `.base` file, its YAML. An entry is
capped (`MAX_PROPS`, `MAX_LIST`, `MAX_TEXT`, `MAX_VALUE`, `MAX_BASE`) and
says when it was (`truncated`). Property keys, tags and link targets are
shared strings, one per distinct text (`Interner`).

`index::Builder` keeps it from a vault that changes (`NoteSource`), a
note at a time. A note whose stamp moved and whose text changed gets a
new entry, and each sync is an `IndexDelta` (entries put, paths gone).
It reads `index::CHUNK` notes per hold of the vault's lock. When a write
lands between two chunks, it reads again, waiting a little longer each
time (`index::TRIES`), and never holds the lock for more than a chunk.

- **Where it runs:** wherever the vault lives. A desk and a phone build
  it over their own copy (`vault::actions::OpenVault` is a `NoteSource`,
  its stamps hashes of each note's text; `vault_query::Built` keeps one).
- **On the node:** the index is built from the vault's folder
  (`vault::actions::query::DiskSource`). It holds no note's text, reads
  only the files whose time or size moved, and is moved on by a watcher.
  Each vault is built once, however many ask at once, and let go after
  `INDEX_IDLE` with no one asking.
- **In a browser:** a browser holds no vault, only the notes it opens and
  a copy of the index. It gets the copy from the node's
  `vault_proto::index_sync::VaultIndexSync` on the org lane: a
  `snapshot`, then everything `since` a revision as one delta.
- **What each reader gets:** the node filters every answer per caller
  (`OrgLane::reads`: a `read` of `org/<id>/docs/<note>`, as
  `OrgLane::opens` checks a write). A delta never names a path the caller
  may not read. When nothing the caller may read changed, the answer
  stays at its revision, so it cannot count edits to notes it cannot
  read. Its `access` is a fingerprint of its rights (`OrgLane::read_key`,
  its role); when that moves, `since` answers "take a snapshot".
- **Histories:** each builder has a random `epoch`. A node started again
  is another history, and `since` from an old one answers "take a
  snapshot", as does a revision further back than the deltas kept
  (`index::KEPT`). The first full build is no delta.
- **On the wire:** JSON with a version (`v`, `index::WIRE`), the epoch,
  short keys and empty fields left out (`VaultIndex::to_bytes`,
  `IndexDelta::to_bytes`), at most `index::MAX_WIRE` bytes.
- **Share guests** get no index yet; a guest's subset of it is #710 G1.

**The engine** (`vault_query::QueryEngine`). It runs a base's view over
the index: filters, formulas, sort and group-by (`vault_query::bases`,
over `vault_proto::grouping`), never over a note's body. It is the one
implementation of `vault_proto::query::VaultQuery`. Every client serves
it in process (`architect::serve_local`) over its own index: a desk and a
phone over their vault (`vault_query::Built`), a browser over its synced
copy (`engine::Replica`). The node answers no queries, and full-text
search over bodies is a separate service. `BASES_ROWS` is the same
engine, through the same client (`architect_bases::actions::register`).

- **It keeps no rows.** A query makes one per note from the index's
  entries (sharing their properties), runs its view, builds the table
  only for the rows its `limit` keeps (`bases::view_table_at_most`), and
  lets the rest go. What a client holds between queries is its index.
- **The answer is written once** (`QueryEngine::run_text`). Its hash is
  that text's, and `revision` and `hash` go in front.
- **Live:** a view hears the document events (`doc.saved`,
  `doc.created`, `doc.renamed`, `doc.deleted`) and asks again with
  `if_changed: <hash>`. When the index has not moved, the answer is
  `{unchanged: true, revision, hash}`, with nothing run. After any edit,
  even to a note the view does not show, the view runs again in full;
  following each view's own notes is later work.

**From an extension.** The host holds only the service's client, given
with its documents in one call for one scope (`Host::set_vault`,
`hostapi::VaultScope`): a share guest's host gets none, and
`Host::set_documents` alone lets any client go. An extension asks with
the host call `query_vault` (`HostRequest::QueryVault`: `{path? |
source?, view?, if_changed?, limit?}`; `host.query_vault{…}` in Luau),
which needs `reads = ["documents"]`, as the document calls and every
`BASES_*` action do (`capabilities::BASES_ACTIONS`). Its answers are
pages: `hostapi::QUERY_LIMIT` rows unless it asks for more, at most
`hostapi::QUERY_LIMIT_MAX`. The answer has:

- the base's views;
- the view (`type`, `name`, `order`, `sort`, `groupBy`, `filters`,
  `limit`, `options`);
- its rows in groups (`{label, rows: [{path, name, values, formulas}]}`)
  and `total`;
- every column it could show, and its formulas and filters;
- the index's `revision` and a `hash` of the answer.

A row's `file.mtime` is its note's (RFC 3339). `file.ctime`, `file.size`
and `file.backlinks` are not in the index yet.

**The boundary.** The index, its builder, its sync and the engine are the
core. Bases is an extension over them, using only what a third party
could. It opens `.base` files, draws a base with its toolbar and view
picker, adds table, cards and list as `[[base_views]]`, keeps the
`BASES_*` actions, draws the fence, and provides `architect.bases` to
other extensions. Reading and parsing a `.base` file, and writing one
back, go through the document calls.

**What it costs** on a generated vault of 10 000 notes, through the
service's client, so each answer is serialized and read back as an
extension gets it (`features/vault/vault/tests/query_bench.rs`, ignored:
run it with `--release --ignored --nocapture`). The base has a filter, a
formula, a sort and a group-by. The even vault is about a kilobyte a
note; the skewed one has a note in a hundred with 120 properties and a
long body, every note linking to one note, and most notes sharing one
status.

| | even | skewed |
|---|---:|---:|
| index built from an open vault | 66 ms | 96 ms |
| a snapshot on the wire (JSON) | 257 B a note | 303 B a note |
| a delta for one property set | 315 B | 320 B |
| the index in memory, natively (entries and stamps) | 1059 B a note | 1190 B a note |
| a browser's copy in memory, in V8 (wasm) | 611 B a note | |

| query, through the client, p50 / p99 | even | skewed |
|---|---:|---:|
| every matching row (7 500 / 9 496 rows answered) | 97 / 129 ms | 159 / 428 ms |
| `limit: 50`, an extension's page | 37 / 62 ms | 75 / 312 ms |
| `if_changed`, nothing moved | 0.02 / 0.10 ms | 0.03 / 0.07 ms |
| index sync after one edit | 10 / 13 ms | 13 / 17 ms |
| `limit: 50` after one edit | 45 / 66 ms | 73 / 94 ms |
| a save, with syncs and queries on two other threads | 0.02 / 0.04 ms | 0.02 / 0.45 ms |

Measured 2026-11-14 on THEBATTLESHIP in release, beside other builds (the
spread is theirs). A browser's copy is measured in V8 by
`features/vault/vault-query/examples/index_memory.mjs` (built to wasm
with `wasm-bindgen`). There a query over 10 000 notes peaks at about
5 MiB while it runs and keeps nothing, at 41–57 ms. Before this, a query
made every note a row under the vault's lock, every time; now a save
waits on a sync for at most a chunk of notes, never on a query.

## The editor: commands, keys, decorations, slash rows

What Advanced Tables, Outliner and Editing Toolbar need (#312, gap
`G-editor`): the live editor the person is typing in, not the document
behind it. The editor's own surface is data, in the editor repository
(`editor:crates/editor-ext`, `editor_ext`, re-exported as
`architect_editor::ext`): a `Snapshot` of the editor,
an `EditCommand`, `KeymapEntry`s, `DecorationSpec`s and `SlashEntry`s, so
an extension in another process, in Luau, in a WASI component or in a
page sends and receives the same values a compiled-in one would.
`editor:crates/editor-state/src/ext.rs` joins them to the editor's state
(`snapshot`, `transaction`, `resolve_key`, `decorations`, `mapped`). The
host side is `features/extensions/src/editor.rs`.

| call | Luau `host.` / a page's `editor.` | answer | needs |
|---|---|---|---|
| `editor_state` | `editor()` / `state()` | the focused editor: `{editor, context, text, selection, primary, cursor, node, version}` | `reads` or `writes = ["editor"]` |
| `edit_editor {changes, selection?, name?, base?}` | `edit_editor(cmd)` / `edit(cmd)` | the editor after | `writes = ["editor"]` |
| `decorate_editor {decorations}` | `decorate_editor(list)` / `decorate(list)` | nothing | `writes` |

The WIT world has them as `editor-state`, `edit-editor` and
`decorate-editor` (the wasi guest's `host::editor_state`,
`host::edit_editor`, `host::decorate_editor`), and `call` reaches them
too.

- **Positions are byte offsets** into the UTF-8 text, from 0, the
  editor's own unit (a Luau string position is that plus one).
- **The node at the cursor** (`node`, `editor_ext::NodeAt`) is what a
  keymap scopes on and what an extension reads instead of parsing the
  markdown again: its `kind` (`paragraph`, `blank`, `heading`,
  `list-item`, `table`, `fence`, `quote`), its line (`line_from`,
  `line_to`, `line`), `indent`, a list item's or heading's `marker`,
  `content_from`; in a table, the `cell` the cursor is in and every
  `cells` of the table, row by row, the delimiter row left out.
- **An edit is a transaction**: `changes` (`{from, to, insert}`, all
  against the text read, in any order, none overlapping) and then
  `selection` (in the text after; the one there was, moved, when not
  given) are one step of the editor's undo, named `name`, made as the
  extension (`Principal::Extension`). `base` is the `version` read: an
  edit made against a text that has since moved on is refused, as is a
  position outside the text or inside a character, so a sandboxed
  extension cannot break the editor with a bad offset. In the editor a
  step never merges with the person's typing (its `user_event` is
  `extension`), and undo takes it back whole.
- **Decorations** are an extension's own set, each call's in place of
  the last (`[]`: none), moved through later edits and taken away when
  it unloads: a `mark` (`{from, to, class, title?}`), a `line` class
  (`{at, class}`), a `gutter` marker (`{at, text, class?}`) and a
  `widget` (`{at, view, props?, block?}`) that mounts one of the
  extension's own `[[embeds]]` (by id, or the view one draws) or
  `[[views]]`, given `props` as its `EmbedInput.props`: the embed half of
  #180, drawn as an inline embed is (`embeds::WidgetView`,
  `Embeds::of`). Classes are prefixed `ext-` (`todo` is drawn
  `ext-todo`), so an extension's never reach the editor's own.

From the manifest, or contributed at run time:

```toml
[capabilities]
writes = ["editor"]

[[keymaps]]
key = "Tab"                 # Shift-Tab, Mod-Enter, Ctrl-Alt-k
when = "node:list-item"     # "editor" (the default), "node:table", …
action = "ACME_INDENT"

[[slash]]
label = "Table"
action = "ACME_TABLE"
group = "Insert"            # the extension's name when empty
icon = "|-|"
```

A key runs its action while the editor has the focus and the cursor is
in the scope's node; a node's entry is tried before an editor-wide one
(`editor::key_action`, `editor_ext::resolve`). A key or a slash row
needs `writes = ["editor"]`, checked at load. A slash row's action runs
once the slash and query typed are taken out. **Toolbar entries** are
toolbars (`[[toolbars]]`, "Toolbars" above) with `when =
"editor:<context>"`: on the Typing Toolbar while that editor is typed
in, and wherever else their `where` puts them.

**The app side.** An app gives the host its editors by implementing
`editor::Editors` (`focused`, `apply` as one step of its history,
`decorate`, `withdraw`) and passing it to `Host::with_editors`;
`editor::MemoryEditor` is one over a string with its own undo, for a
headless host and for tests. With the FastTrackStudios editor
(`editor:features/ui`), the host:

- gives the `Editor` a `history` it keeps, and applies an extension's
  transaction with `dispatch_with_history`, so it undoes in that one stack;
- answers `on_key` with `editor_state::ext::resolve_key` over
  `Host::contributions().keymaps()`, running the action it finds;
- draws `editor_state::ext::decorations` as a `DecorationSource` and
  mounts widgets through `mounts` (`WidgetMounts`, the native renderer's;
  `ext::mount_index` names the spec), with `WidgetView` for the embed;
- registers the slash rows with `palette::register_actions` and runs
  them from `on_action`.

A Blitz document may have no HTML parser (a headless one has
none), so the native renderer draws gutter markers and mounted widgets
itself, never from HTML.

Tests: `features/extensions/tests/editor.rs` (an outliner extension on a
`MemoryEditor`: the keys by node, one undo step leaving the person's
typing, decorations that follow edits and go when it unloads, a widget
naming its own embed, refusals without `reads` / `writes`, of a stale or
out-of-bounds edit, of a key without `writes`),
`features/extensions/tests/wasi_wit.rs` (the typed imports are the
calls); in the editor repository, `editor:features/ui/tests/native_ext.rs`
and `editor:crates/editor-state/src/ext.rs`'s tests.

## Languages for the editor

Languages are installed by extensions while they are on; their grammars
are linked into the app (#512). The editor asks a registry by
file type or fence name (`features/editor/src/languages.rs` `Languages`);
the app puts its own languages
in it and gives it to the host (`Host::with_languages`), and an extension
adds to it:

```toml
[[languages]]
id = "keyflow"                            # lowercase letters, digits, - and _
name = "Keyflow chart"
extensions = ["kf"]                       # file extensions, no dot
fences = ["kf"]                           # fence names; its id is always one
grammar = "linked:keyflow"               # a grammar linked into the app; "linked": the editor's own
highlights = "queries/highlights.scm"    # its highlights query, in its package
comment = "//"
[languages.server]                        # optional LSP
command = "kf-lsp"
args = ["--stdio"]
```

A language is a contribution like the rest
(`features/extensions/src/contrib.rs` `LanguageDecl`, which is
architect-editor's `Language`): checked at load (its id; no empty
extension or fence; no grammar file; its query inside its package; a server has
a command; an id once per extension), its query's text read from the
package when it loads (`Language::highlights_text`), in the registry while
the extension is on, and withdrawn when it is turned off or unloads
(`features/extensions/tests/interop.rs`). A language two owners register
is the first's, and the app's come first. `EXT_LANGUAGES` answers every
language, the app's too, each with `from` (`app` or the extension's id);
with `id`, `fence` or `path` it answers the one that names, or `null`.

Highlighting is drawn in theme tokens (`syntax.keyword`, `syntax.string`,
…: `features/editor/src/theme.rs` `TOKENS`), so an extension's `[tokens]`
or `[[themes]]` restyle every language, and a language never ships
colours.

The app drives the editor's own registry from it
(`architect_editor::syntax::drive`), so a language is highlighted in
fences and in `CodeEditor` from the moment its extension is on until it
is turned off, and editors on screen draw again.

### Adding a language

1. **The grammar.** It is linked into the binary: a grammar file
   (`grammar = "x.wasm"`) is refused at load, saying so, because the
   editor's tree-sitter cannot load a grammar at run time. Pick one:
   - one the editor has compiled in (Rust, TypeScript, JavaScript, Python,
     JSON, Bash, Markdown): `grammar = "linked"` (the default) and an id or
     a fence that names it;
   - one the app links: `grammar = "linked:<name>"`, the name the app gave
     `architect_editor::syntax::link_grammar(name, grammar, query)`;
   - your own, from a linked extension: depend on the grammar's crate,
     call `architect_editor::syntax::link_grammar("<name>",
     <crate>::LANGUAGE, Some(<crate>::HIGHLIGHTS_QUERY))` when it is built
     (any `LanguageFn` converts), and write `grammar = "linked:<name>"`.
2. **The query**, optional: `highlights = "queries/highlights.scm"` in the
   package, used in place of the grammar's own. Use the standard capture
   names (`@keyword`, `@string`, `@comment`, …): each is drawn in its
   `syntax.*` token.
3. **What it answers to:** `extensions` and `fences`.
4. **A server**, optional: `[languages.server] command, args`.

Read `EXT_LANGUAGES {id}` to see it registered. A language the editor
cannot highlight (a `linked:` name nothing linked, a query tree-sitter
refuses) stays in the registry but is left out of the editor's, and
`architect_editor::syntax::problems` says why.

**A language's server** (`[languages.server] command, args`) runs only
when the extension asks `[capabilities] languages = ["servers"]` and a
person granted it (a server is a program on their computer): the host
starts it, one per language and workspace root, while a code editor of
the language is open, restarts it with backoff when it dies, and stops
it when the last editor closes or the extension is turned off
(the language-server client went with the Architect app; an app
brings one). Its diagnostics are drawn in the code editor; hover, completions,
go to definition, format and rename are the `EDITOR_*` actions, for
keys, ⌘K and agents.

### An editor in your view

A linked extension mounts the editor's component from `architect-editor`
(features `ui` and a renderer) in its view, over a document of its own,
and hears its changes through `on_transaction`
(`features/extensions/tests/editor_view.rs`). A process extension's remote
view, a WASI view and a page have no way to mount one yet: they draw
through the edit stream into `app-shell:crates/app-shell-extensions/src/remote.rs`
`RemoteDocument`, which has no host-drawn elements (#512).

## The network and secrets

Git, Copilot, Remotely Save and Smart Connections need the network off
the desk, where a process extension cannot run (#310). An extension
never opens a socket: a component's world has no `wasi:http` or
`wasi:sockets` (one that imports them is refused before it is linked),
and a script or a page has no network of its own. It asks the host to
`fetch`, and the host checks every request against the manifest's
`capabilities.hosts` as a person granted them — the hosts the consent
screen lists. The model is `features/extensions/src/hostcall.rs`
`Fetch` and `may_reach_url`; the native host is
`features/extensions/src/net.rs`.

| call | Luau `host.` / a page's `net.` / a component's import | answer | needs |
|---|---|---|---|
| `fetch {url, method?, headers?, body?, timeout_ms?, max_bytes?, stream?, binary?}` | `fetch(url, options?)` / `fetch(url, options?)` / `fetch` | `{status, url, headers, body}`; with `stream`, `{status, url, headers, stream}` | the URL's host in `capabilities.hosts` |
| `read_body {stream, max?}` | `read_body(stream, max?)` / `read(stream, max?)` / `read-body` | `{chunk, done}` | its own stream |
| `close_body {stream}` | `close_body(stream)` / `close(stream)` / `close-body` | whether it was open | its own stream |
| `secret {name}` | `secret(name)` / `secret(name)` / `secret-set` | `{set, title}`, never the value | one of its `[[secrets]]` |
| `ask_secret {name}` | `ask_secret(name)` / `askSecret(name)` / `ask-secret` | whether the person gave one | one of its `[[secrets]]` |

```toml
[capabilities]
hosts = ["api.example.com", "*.example.org", "127.0.0.1:8080"]
```

A host entry is a name, a domain's subdomains (`*.example.org`, and the
domain itself), a name and port, or `*` (`capabilities::reaches_host`).
Only `http` and `https` are fetched.

**Redirects** are the host's: the client follows none
(`reqwest`'s `Policy::none`), and the host follows each one itself,
checking its URL as it checked the first. A redirect to a host the
extension may not reach is refused (`refused (hosts): redirected to …`),
not followed. A `303`, and a `301` or `302` after a `POST`, go on as a
`GET` without the body; `Authorization` and `Cookie` go only to the host
they were set for; at most `net::MAX_REDIRECTS`.

**Limits**: a request waits `net::TIMEOUT` for its response unless it
says (`timeout_ms`, at most `net::MAX_TIMEOUT`); a body read whole is at
most `net::MAX_BODY` (`max_bytes` may say less), a streamed one
`net::MAX_STREAM`; what it sends at most `net::MAX_UPLOAD`. Past one, the
call fails. The headers that are the host's (`Host`, `Content-Length`,
`Connection`, `Transfer-Encoding`, `Upgrade`) it may not set.

**Streams**: with `stream`, the answer has no `body` but a `stream` id;
`read_body` gives its next part (at most `max` bytes, `net::CHUNK` by
default; text never splits a character), `done` once it is all read, and
the stream closes itself then. Only the extension that fetched it reads
it, at most `net::MAX_STREAMS` at once, and they go when it unloads. A
part waits at most the request's time limit. `binary` gives the body,
and each part, as base64.

**Secrets**: what an API needs that the extension must not hold. A
manifest declares them; the person enters each once, when the extension
first uses it (or asks, `ask_secret`): the host asks through the app's
prompt (`hostui::HostUi::prompt`, a `PromptKind::Secret` prompt, typed
hidden), and keeps it in `net::Secrets` — `<app dir>/extension-secrets.json`
in the sandbox, written `0600`, never in the extension's folder, and refused
to its `read_file`. A request names it in a header's value; the host puts
it in, only on a request to one of the secret's hosts, and leaves it off
a redirect anywhere else. No call answers its value: `secret` says
whether it is set.

```toml
[[secrets]]
id = "api_key"
title = "Example API key"
description = "From your account's settings page."
hosts = ["api.example.com"]   # within capabilities.hosts; none: all of them
```

```lua
local r = host.fetch("https://api.example.com/v1/items", {
    headers = { Authorization = "Bearer {{secret:api_key}}" },
})
local s = host.fetch({ url = "https://api.example.com/feed", stream = true })
repeat local part = host.read_body(s.stream) until part.done
```

A secret in the URL or the body, one it does not declare, or one sent to
a host outside its `hosts` is refused (`refused (secrets)`). A person
forgets one with `Secrets::forget`; `Host::with_secrets` gives the host
its store (in memory for the run otherwise).

**Where it runs**: the native host sends each request on a runtime of its
own (two threads, `ext-net`), so a fetch works from whichever executor
asks — a component's thread, a script's, a page's. The client is
`architect-extensions`' feature `net` (reqwest), which the app turns on
in its binary's crate, not its UI crates: reqwest reaches
tower-http, which `cargo xtask boundaries` keeps out of them. A build
without it keeps secrets and answers a fetch `unsupported`. In a browser
(`web::WebHost`) the browser's own `fetch` sends it, where CORS lets it;
one the browser refuses answers `unsupported` saying so. There the
browser hides where a redirect goes, so a redirect is not followed but
answered `unsupported`, and secrets are not kept yet. A streamed body
there is read whole by the browser and given in parts.

Each call is the `ext.host_call` span, with `perm.capability` `hosts`
(or `secrets`), `perm.decision` and `perm.reason`, and
`http.request.method`, `server.address`, `http.response.status_code`,
`ext.redirects`, `ext.bytes` and the secrets named (`ext.secrets`, never
a value); a refusal is a line in its log.

Tests, against a local server (`features/extensions/tests/support/server.rs`),
no real network: `features/extensions/tests/net.rs` (a declared host
reached and another refused, directly and through a redirect; a Luau
script the same; a streamed body read in parts by its own extension
only; the time and size limits; a secret asked for once, put into
requests to its host, taken off a redirect elsewhere, never readable;
document hooks need `reads = ["documents"]`, declared, made at run time
and granted), `features/extensions/tests/wasi.rs`
(`a_component_fetches_from_its_host_and_is_refused_another`, and
`a_component_makes_edits_lists_and_reads_a_document` for the document
imports).

## Server-side parts on the node

The org's node does sync, access control and only what truly needs a
server (#710). Extension code runs on the person's device or in their
browser, against their own copy of the vault. The one exception is a
**node action** (#583): code that holds a secret the person's devices
must never have, such as a bank-link token exchange that needs the
client secret (Finance, #370).

```toml
[capabilities]
hosts = ["sandbox.plaid.com"]

[[secrets]]
id = "client_secret"
title = "Plaid secret"
hosts = ["sandbox.plaid.com"]

[[actions]]
id = "LEDGER_EXCHANGE"
name = "Link a bank"
script = "node/exchange.luau"
runs = "node"                    # the default is "app"
secrets = ["client_secret"]      # why it needs a server: at least one
```

**The bar.** `Manifest::check` keeps it high. A node action:
- is a `luau` extension's action with a script;
- holds at least one of its `[[secrets]]`;
- is never a preset, a toggle or a window opener.

An app action names no `secrets`. Anything else that seems to need a
server (a webhook, a schedule that runs while every device sleeps) is an
owner's decision first, not a new kind of node action.

**What the node runs.** `Manifest::node_part` is all of the extension the
node loads:
- its node actions;
- the secrets they hold;
- of its capabilities, only `hosts` and `effect`.

It gets no views, hooks, workers, background, settings, documents,
interface or ports. On the node a script has `host.fetch` to its declared
hosts (each redirect checked). Its secrets go into a request's headers
(`{{secret:client_secret}}`), put in by the host, never readable by the
script (["The network and secrets"](#the-network-and-secrets)). The
deployment that names the extension, by pinned version, hash and
signature through Nix, is the node's consent.

**One implementation.** `NodeActions` (`features/extensions/src/node.rs`) is one
`#[architect::rpc]` service:
- `list`: any member;
- `run`: a member (`write`), audited;
- `secrets` and `set_secret`: an admin.

`NodeHost` (feature `node`) is its one implementation. The node loads
each installed extension's node part on a host of its own and serves it
on the org lane (`/org/{slug}/vox`) behind the org's permissions gate
(`node::PERMITS`).

Every client holds the generated `NodeActionsClient`: the desktop, iOS
and the browser. An app's host registers a node action as a forwarder to
it (`Host::with_node`, a `node::NodeLane`), so ⌘K, the CLI, MCP and a
script run it like any other action. With no node (signed out, or an org
whose node doesn't run the extension), running it says where it runs.

**Secrets are insert-only.** A node's secrets come from one of two places:
- **its operator**: a folder of files, `<dir>/<ext id>/<secret id>`, as
  sops-nix writes them (`NodeHost::with_operator_secrets`); the lane
  then sets none;
- **an org admin**: through `set_secret`, kept in the node's private file
  (`NodeHost::new`).

No call reads a value back: `secrets` says which are set.

The example is the showcase's Bank link (`examples/showcase/bank-link`):
a token exchange against a local mock of Plaid. Its test is
`features/extensions/tests/node.rs`. It covers:
- an app's host runs the exchange over vox behind a gate, through the
  generated client;
- the bank hears both secrets, and the app holds neither;
- a viewer is refused, a missing secret is named, and secrets are
  insert-only;
- an operator's node takes no secret from the lane;
- an undeclared host is refused;
- the manifest's rules.

**Not yet.** Node-kept data, so an access token stays on the node for
later calls, is a follow-up. So are WASI components' node actions.

## MIDI, OSC and control surfaces

CSI, DrivenByMoss, ReaLearn, LBX Stripper and Playtime talk to devices:
MIDI controllers, OSC apps on a tablet, motor-fader surfaces (#311). An
extension does not open a device itself. It asks the host, and the host
checks each port against the manifest's `capabilities.ports` as a person
granted them; the consent screen lists them like `hosts`. The ports are the
app's: it gives the host an `architect_input::PortBackend`
(`Host::with_ports`): OSC over UDP (`architect_input::UdpOsc`), say,
and a virtual MIDI port. A
system MIDI backend (`midir`, CoreMIDI) is the app's to add behind the same
trait. The model is `features/input/src/ports.rs` and the host side is
`features/extensions/src/ports.rs`.

```toml
[capabilities]
ports = [
    "midi:X-Touch*",        # MIDI ports by name; a trailing * any it begins; midi:* any
    "osc:127.0.0.1:9000",   # an OSC endpoint it sends to; osc:<host>:* any port there; osc:* any
    "osc:listen:9001",      # a port of this computer it hears OSC on
]
```

| call | Luau `host.` / a page's `ports.` / a component's import | answer | needs |
|---|---|---|---|
| `ports {kind?}` | `ports(kind?)` / `list(kind?)` / `ports` | `[{kind, name, direction}]` (MIDI ports; OSC endpoints are not listed) | any `capabilities.ports` |
| `open_port {kind, name, listen?}` | `open_port(kind, name, listen?)` / `open(kind, name, listen?)` / `open-port` | `{port, kind, name}` | `capabilities.ports` naming it (and `osc:listen:<listen>`) |
| `port_send {port, message}` | `port_send(port, message)` / `send(port, message)` / `port-send` | `null` | its own port |
| `port_receive {port, max?}` | `port_receive(port, max?)` / `receive(port, max?)` / `port-receive` | `[message]` that no hook took | its own port |
| `close_port {port}` | `close_port(port)` / `close(port)` / `close-port` | whether it was open | its own port |
| `targets` | `targets()` / `targets()` / `call` | `[{id, title, min, max, step?, count, values}]` | nothing |
| `surfaces` / `surface_bank {surface, bank}` | `surfaces()`, `surface_bank(surface, bank)` / `surfaces()`, `setBank(surface, bank)` / `call` | its surfaces as they run / the bank it is on | its own surfaces |

**Messages** are JSON (`PortMessage::to_json`, `from_json`). A MIDI one is
`{bytes, type, channel, number, value}`. `type` is one of `note_on`,
`note_off`, `cc`, `program`, `pitch_bend`, `sysex` or `other`, and channels
are 1–16. An extension sends either `{bytes: [...]}` or `{type, channel,
number, value}`. An OSC one is `{address, args}`. An argument that is a
whole number is sent as an `i`, any other number as an `f`, a string as an
`s`, `true` and `false` as `T` and `F`, and `null` as `N`. Bundles are read
into their messages.

**What a port hears** is the event `port.message`, sent only to the
extension that opened the port. Its detail is `{port, kind, name, message}`.
A hook on it, in the manifest or made with `subscribe`, runs for each
message. So a background extension (`[background]`) sleeps until its device
says something and is started by the first message after it went idle.
Its ports stay open while it sleeps. A message that started no hook waits
for `port_receive`: there was no hook, or a run of it had not ended. The
last `ports::INBOX` (1024) messages are kept. When the extension unloads,
every port it opened closes.

```toml
[background]
idle = "5m"

[[hooks]]
on = "port.message"
script = "echo.luau"
function = "heard"
```

```lua
-- echo.luau
return { heard = function(event)
    local d = event.detail          -- {port, kind, name, message}
    if d.kind == "midi" and d.message.type == "note_on" then
        host.port_send(d.port, { type = "note_on", channel = d.message.channel,
                                 number = d.message.number + 12, value = d.message.value })
    end
end }
```

### Control surfaces

A control surface is a contribution, not code. The app offers **targets**
(`Host::with_targets`, an `architect_input::Targets`). Each kind is an
`architect_input::Target` with a `TargetKind` (an id, a range and how many
instances there are) and implements read, write, an optional label (a
track's name, for a scribble strip), touch and observe. Studio offers track
volumes, FX parameters and the transport (studio#381). Offering targets declares the point `control.surfaces`. An
extension contributes a surface to it as an `architect_input::SurfaceDesc`:

```toml
[[contribute."control.surfaces"]]
id = "osc"
title = "Lab OSC surface"
port = "osc:127.0.0.1:9000"   # or midi:<port name>; within capabilities.ports
listen = 9001                 # OSC: where the device sends (osc:listen:9001)
bank_size = 8                 # strips a bank is (none: the highest strip + 1)
controls = [
    { id = "fader1", kind = "fader", strip = 0, input = "PB ch1", touch = "Note104 ch1", target = "track.volume" },
    { id = "strip1", kind = "display", strip = 0, feedback = "sysex:F0 00 00 66 14 12 00 {text} F7", target = "track.volume" },
    { id = "tempo", kind = "knob", input = "/lab/tempo", target = "lab.metronome.tempo", takeover = "pickup" },
    { id = "pan1", kind = "encoder", strip = 0, input = "CC16 ch1", target = "track.pan" },
    { id = "bank+", kind = "button", input = "Note47 ch1", target = "bank.next" },
    { id = "bank-", kind = "button", input = "Note46 ch1", target = "bank.prev" },
]
```

The host opens the surface's port (one its contributor's
`capabilities.ports` names) and runs it (`architect_input::Surface`):

- A control's `input` is a MIDI message (`CC7 ch1`, `PB ch1`, `Note16
  ch1`) or an OSC address whose first argument is the value (0–1).
  `feedback` is what it is sent back: by default its input for a fader,
  knob, encoder or button. A display takes `sysex:… {text} …` or an OSC
  address. Kinds are `fader`, `knob`, `button`, `encoder`, `display` and
  `led`.
- **Banks**: a strip control moves instance `bank × bank_size + strip`. A
  `bank.next` or `bank.prev` button, or `surface_bank`, switches the bank.
  The device is then sent every control's feedback: motor faders move,
  scribble strips are renamed.
- **Feedback**: a target that changes elsewhere (its `observe`, or the
  app's `Targets::changed`) moves the controls on it.
- **Touch**: a touched fader is not moved under the finger, and when it
  is let go it goes to where the target is. The target hears `touch`, for
  automation.
- **Takeover**: `jump` (the default) writes at once. With `pickup`, a
  knob writes nothing until it reaches the target's value or crosses it.
- An encoder's message is detents, signed (CC 1–63 up, 65–127 down; an
  OSC argument ±n), each one the kind's `step`.

`Host::surfaces` (and the `surfaces` call) say how each surface is: its
bank, how many banks there are, and why it is not running (a bad
description, a port it may not open, or one that would not open). That
reason is also a line in its log. When the item goes, its surface stops
and its port closes.

**Where it runs**: only on the native host. In a browser (`web::WebHost`)
the port calls answer `unsupported`: a browser has no UDP, and Web MIDI is
not wired yet. Each call is the `ext.host_call` span with `perm.capability`
`ports`, and a refusal names what it lacked (`ports.midi:Other`).

Tests, with no hardware: `features/extensions/tests/ports.rs`. A
background extension hears a virtual MIDI port and an OSC endpoint over
localhost UDP, is started again by a message, and answers through both.
What no hook takes waits for `port_receive`. A port outside its
capabilities is refused, and so are another extension's port, `ports`
with none, a bad entry and a surface on a port it may not open. A fake
eight-fader surface moves test targets with feedback, banks and pickup.
`features/input/tests/surface.rs` tests the engine: banks, feedback,
touch, pickup and an encoder. `features/input/tests/ports.rs` tests OSC
over UDP.

## The extensions manager

One place in every app for everything that customizes it
(`features/extensions/src/manager/mod.rs` `Manager`, feature `manager`):
a registry of kinds (`ItemKind`: list, get, run an op). It comes with
extensions (any runner, which copy runs and the way back to the
built-in), Luau scripts and their hooks, workers (process extensions'
processes), themes and the interface files (layouts and `keys.json`)
(`features/extensions/src/manager/kinds.rs`). An app adds a kind of its
own — audio plugins, a page kind, WASI components — by implementing
`ItemKind` and calling `Manager::register`. Agents get every operation
as `MANAGER_KINDS`, `MANAGER_LIST {kind?, query?}`, `MANAGER_GET {kind,
id}` and `MANAGER_RUN {kind, id, op, args?}`; a destructive op from
outside asks the person first (`Manager::confirm_on`).

The manager is itself a linked-in extension
(`features/extensions/src/manager/extension.rs` `ManagerExtension`,
id `manager`), overridable by a development copy. Its page
(`app-shell:crates/app-shell-extensions/src/page.rs` `ManagerPage`, feature `page`, in
architect-ui's components and the theme's variables) is a settings
section and a workspace view (`use_manager_page`, `ManagerView`: it
fills its pane, the list and the item's page each scrolling on their
own; a pane narrower than `NARROW` shows the list, then the item picked
with the way back), with
⌘O's `ext:`; an extension's page there shows what it adds, what it asks
for (granted or not, each grant revocable), versions and pin, where it
runs and its log, and installs through the consent prompt. A project's
missing extensions use the same components (`MissingExtensions`).

## Packaging

A signed `.archext` archive: `extension.toml` (id, version, `api`
version, capabilities, contributions), the `.wasm` or a program per
platform, Luau, pages, assets. Installed from a registry (a git
repository of an index, reviewed by pull request, as Zed's), from a
file, or from a peer, and checked the same way each time: see
[Distribution](#distribution). Hot reload re-instantiates the component.

## Distribution

How an extension gets from the repository that builds it to a person's
app (#182).

### The archive

An `.archext` is a gzipped tar (`tar tzf x.archext` lists it):

| entry | what it is |
|---|---|
| `archext.json` | the content index: format `1`, the extension's id, version and kind, the target triples it has builds for, and every other file with its size and SHA-256 |
| `archext.json.minisig` | a detached minisign signature over the index's bytes, trusted comment `archext <id> <version>` |
| `extension.toml`, Luau, the `.wasm`, pages, assets | the package's files, as in its folder |
| `bin/<target-triple>/<program>` | a process extension's program, once per platform |

Packing is deterministic (sorted, dated 0, owned by no one): the same
files make the same bytes. The format is
`features/extensions/archive/src/lib.rs` (`architect-extension-archive`:
`pack`, `open`, `Verified::extract`, `ContentIndex`, `ArchiveError`);
signing is its `sign` feature (`features/extensions/archive/src/sign.rs`
`SecretKey`, `sign_archive`), which apps never turn on.

### Signing and trust

The signature is minisign's (Ed25519 over the BLAKE2b-512 of the index),
so `minisign -V -p key.pub -m archext.json -x archext.json.minisig` checks
an unpacked one too. Release archives are signed with the **org release
key**, the same key that signs every FTS release's `SHA256SUMS`
([release](@/architecture/release.md)); its secret half lives only in the
repositories' tag-only `release-signing` environments.

An app trusts the org key by default: `Installs::new` holds `Trust::org`
(`ORG_KEY`, the public half, compiled in); `Installs::with_trust` replaces
it. A registry's own keys are trusted for that registry's archives only,
when the app says so (`FolderRegistry::trusting`,
`GitRegistry::trusting`; `Source::trust`). Whichever way an archive came
— a registry (`Location::Archive` in its index, fetched as
`Fetched::Archive`), a file (`EXT_INSTALL_FILE`), a peer
(`Extensions::install_archive`) — `Installs` opens it, never the source,
and refuses it before writing anything when:

| it is | `ArchiveError` |
|---|---|
| unsigned | `Unsigned` |
| signed by a key not trusted here (the error names the key id and the keys trusted) | `UntrustedKey` |
| signed, but its index changed since | `IndexTampered` |
| a file's bytes are not what the signed index says | `FileTampered` (with the path) |
| holding a file the index does not list, or missing one it does | `Unlisted`, `Missing` |
| not a gzipped tar, or a path leaving the package, a link | `NotAnArchive`, `Unsafe` |

The index's `sha256` for the archive is checked first
(`InstallError::Mismatch`): integrity of what the registry pinned. The
signature is what makes it authentic: whoever can write a release's
assets can swap both the archive and a hash, not the key.
`features/extensions/archive/tests/archive.rs` and
`features/extensions/tests/distribution.rs` are the refusals, each
asserted by its error.

### Per platform

A process extension ships `bin/<target-triple>/<program>` for each of
macOS arm64 and x86_64 (`aarch64-apple-darwin`, `x86_64-apple-darwin`) and
Linux x86_64 and aarch64 (`x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`; Windows later); its manifest names the
program once, `[entry] path = "bin/<program>"`. Installing extracts this
platform's build to `bin/<program>` (executable) and leaves the others
out, so the runner needs nothing new. A WASI component, Luau, TypeScript
and data are platform-neutral: no `bin/`.

The registry's entry lists the platforms (`platforms`), so the answer
comes before a download: a preview says `runs_here: false` and
`cannot_run: "no build for this platform (…); it has builds for …"`
(`Preview::cannot_run`, `Listing::no_build_for`), a project's plan says
`Missing::CannotRun("process: no build for …")`, and an install is
refused with `ArchiveError::NoBuild`.

### Which operating systems: `platforms`

Every kind can say which operating systems it runs on (#578):
`platforms = ["linux"]` in `extension.toml`, Rust's `target_os` values
(`linux`, `macos`, `windows`, `ios`, `android`) and `web` for a browser;
absent means everywhere (`features/extensions/src/platform.rs`). Off
those, the extension is kept and listed, never started:

- a linked one is still a built-in member of the distribution (its crate
  compiles everywhere, stubbing what it cannot do), but its `build` never
  runs (`Extensions::with_linked`);
- the host refuses to load it with `LoadError::NotHere` ("Linux only: not
  on macOS"), and does not try again on the next sync; the same state as
  a process extension in an app with no process runner (`NoRunner`), the
  web's and iOS's (`LoadError::is_not_here`);
- the extensions page shows "Can't run here" with why
  (`browse::Row::not_here`), `EXT_LIST` its `platforms` and `not_here`,
  `APP_STATUS` the reason;
- its data (`<data root>/<id>/`) and install are left where they are;
- a `[requires]` whose only provider cannot run here says so: "its
  provider cannot run here: PipeWire is Linux only: not on macOS"
  (`Host::not_here`, for other extensions too);
- the store refuses to install it here, before writing anything, with the
  same words (`InstallError::NotHere`), and its preview says why
  (`Preview::cannot_run`, `Preview::runs_on`).

`Host::set_platform` checks against another platform: a host that knows
better than its build, or a test (`features/extensions/tests/platforms.rs`).
It comes before `Extensions::new`.

- A value this build does not know (`visionos`) is kept, with a warning
  (`Manifest::warnings`, `EXT_CHECK`'s `warnings`): a newer platform's name
  stays forward compatible, and a typo of a known one cannot be told from
  it.
- "Not here" is worked out when it loads: one that tried and could not is
  not tried again while its copy is the same, so a change of platform
  (`Host::set_platform` after start) shows after a restart.
- A copy that cannot run here still counts as the copy of its id: an
  installed or development copy for another platform shadows a built-in
  that could run, and the page says so ("Can't run here"); remove it, or
  "Use the built-in", to run the built-in again.

### Updates

`EXT_UPDATE {check: true}` (`Extensions::updates`) installs nothing: for
each install with a newer version its pin allows, the version, the
registry's `changelog` line, a newer one past the pin (`past_pin`), and
`asks_more`, what it asks past what was granted (the newer archive is
fetched and verified to read it). Applying is `EXT_UPDATE` as before: one
whose capabilities grow waits in `needs_consent` for a person's yes. The
manager's rows say "update available: 1.3.0 — <changelog>" within the pin
(`browse::Row::changelog`).

### Releasing one: the reusable workflow

An extension's repository calls `.github/workflows/extension-release.yml`
on its release tag:

```yaml
jobs:
  extension:
    uses: FastTrackStudios/architect/.github/workflows/extension-release.yml@<architect tag>
    with:
      package: extension                 # the folder with extension.toml
      build: cargo build --release --locked --target "$TARGET" -p studio-extension
      binary: target/$TARGET/release/studio-extension
      architect-ref: <architect tag>
    secrets:
      EXT_SIGNING_KEY: ${{ secrets.TASK_RELEASE_MINISIGN_KEY }}
      REGISTRY_TOKEN: ${{ secrets.FTS_REGISTRY_TOKEN }}
```

`plan` reads the manifest; `build` (a process extension only) builds the
program once per platform on a runner of that platform (`platforms`,
`runners`); `pack` stages the package and the builds and runs
`cargo xtask ext-pack` (and the caller's `prepare`, for a WASI component),
holding no secret; `sign` runs in the caller's `release-signing`
environment, signs with `cargo xtask ext-sign --expect <org key>` (a key
that is not the org key's is refused), verifies, and uploads the archive
to the release; `index` opens a pull request on the registry adding the
version's entry. The secrets are passed by name: a called job sees an
environment's secrets only through what its caller passes.

People use the same commands: `cargo xtask ext-pack <dir>`,
`cargo xtask ext-sign <archive> --key <file>`, `cargo xtask ext-verify`,
and `cargo xtask ext-keygen` for a throwaway key to try it with (never a
release's).

### The registry

`index.toml` in the registry's repository, one `[[extensions]]` entry a
version: `archive` (a release asset's URL, or a path in the repository)
and its `sha256`, `platforms`, `changelog`, beside the older `path` and
`git` + `rev` locations. `examples/registry/` is the skeleton
(`examples/registry/index.schema.json` the schema,
`examples/registry/README.md`), with a sample signed by a throwaway key;
`cargo xtask ext-index <dir>` checks an index — the schema, each archive's
hash and signature, and that the archive is what its entry says
(`features/extensions/archive/src/registry.rs` `check`) — and is the
registry's CI.

### A third party's extension in an app

An app searches and installs from registries kept by `Registries`
(`features/extensions/src/registries.rs`, `Extensions::with_registries`):

- **the store's index**, `extensions/index.toml` in architect's own
  repository, given with `Registries::with_builtin`: always there, never
  written, never removed. It is a git registry in a
  folder of a repository (`GitRegistry::in_dir`, `Registry::in_dir`): only
  that folder is checked out (sparse, about 3 MB). A publisher lists its
  extension there by pull request; its archives are signed with the org
  key, or a publisher key the entry's registry trusts. A release lands
  there through the reusable
  `.github/workflows/extension-release.yml` (`registry:
  FastTrackStudios/architect`, `registry-dir: extensions`), and every pull
  request that changes the index is checked by
  `.github/workflows/extension-index.yml` (`cargo xtask ext-index
  extensions`: each archive's hash, signature and platforms);
- **registries a person adds** in Settings → Extensions (the manager
  page's "Registries and development installs") or with
  `EXT_REGISTRY_ADD {name, git [+ dir] | folder, keys}`, kept in
  `<app dir>/extension-registries.json`.

Each `Registry` carries its **publishers' keys** (`Publisher`: a name and
the base64 line of a minisign `.pub`). They are that registry's
`Source::trust`: a third party (Studio) signs its `.archext` with its own
key (`cargo xtask ext-keygen`, `ext-sign`), its registry entry carries the
public half, and the app installs an archive from that registry signed by
that key while refusing one signed by any other (`UntrustedKey`). An
archive installed from a file (`EXT_INSTALL_FILE`) is checked against the
org key and every registered publisher's (`Extensions::publishers`,
`Installs::read_archive_trusting`). Adding keys from outside the app (an
agent, the CLI) needs a person's yes (`confirmation_required`).

`EXT_REGISTRY_REFRESH` pulls the indexes (the app does once at start, off
the window's thread), `EXT_REGISTRY_REMOVE` drops one the person added,
`EXT_REGISTRIES` lists them.

**A development install** loads a package folder (its `extension.toml`
and, for a process extension, its program) into the running app,
unsigned, as a development copy (`Extensions::dev`): `EXT_DEV {folder}`,
or from a shell `architect-desktop extensions dev <folder>` (over the
running app's control socket; `undev` ends it). It shadows every other
copy of its id, reloads on save, and is loaded again on the next start.
It is refused unless development installs are on: the switch in
Settings → Extensions (`EXT_DEV_INSTALLS {on}`; turning it on from outside
needs a person's yes), or a debug build (`Registries::with_dev_forced`).

Tests: `features/extensions/tests/registries.rs`.

## Compared with REAPER

REAPER's extension SDK declares 878 functions (`reaper_plugin_functions.h`),
ReaScript exposes about 730 of them to Lua, EEL2 and Python, and
extensions add their own (SWS, js_ReaScriptAPI, ReaImGui; Ultraschall's
framework adds 1,908 Lua functions). Native extensions also take hooks:
intercepting actions before and after they run, key processing, control
surfaces, timers, lines in the project file, new media formats, menus,
toolbars, preference pages, docked windows, and a callback on the audio
device's buffers. Where this design stands against all of it:

| REAPER | Here | To close the gap |
|---|---|---|
| Actions: registry, toggle state, stable ids, sections | **better**: schemas, enabled-with-why, effects, nested contexts | ship `checked` |
| Custom and cycle actions | **better**: conditional custom actions and Luau | — |
| Docked windows, screensets | **better**: views and the workspace | — |
| Preference pages | **better**: settings declared once, page generated | — |
| Exporting functions to others (`API_`) | **better, once done**: contributed actions are everyone's | extensions export typed services that others import, with version ranges |
| Distribution (ReaPack) | **better**: hot reload, permissions, sandbox | ReaPack's pinning, version choice, pre-releases, dependencies, per-platform builds, offline bundles |
| Model access (878 getters and setters) | **partial**: actions are coarser | generate the typed model services (tracks, items, takes, FX parameters, envelopes, MIDI) into the WIT world, not only `actions.run` |
| Undo blocks | **missing** | transactions: `begin`/`commit`/`abort` as one undo step; a script is one undo step by default |
| Extension data in the project (`projectconfig`, `P_EXT`, `ProjExtState`) | **partial**: view state only | a per-project and per-object store, namespaced by extension, saved with the session and part of undo |
| Raw state chunks | **missing, by choice** | typed snapshot get/apply per object, destructive and permission-gated |
| Overriding an action (`hookcommand` returning true) | **missing** | action middleware before and after: veto, rewrite arguments, replace — behind an `override` capability, shown in the action list as "overridden by …" |
| After an action (`hookpostcommand`) | **covered**: `watch` | the event carries who ran it and its result |
| MIDI/OSC values into actions | **partial** | a binding passes `{value, resolution, relative, source}` as a standard argument |
| Control surfaces (`csurf`: banks, touch, feedback) | **partial**: streams give feedback | a surface contribution: track banks, touch state, last-touched and focused FX, MIDI/OSC ports by capability — on frame documents |
| Timers, background scripts (`defer`), `atexit`, startup scripts | **partial** | since #308: a WASI view's timers, `every`/`at` hooks, document events, the app's own events (`app:<name>`: Studio's project opened, saved, closed) and background extensions started on their first event; still missing: a run state shown as `checked`, a relaunch rule, an exit hook |
| Marker actions (an action when the playhead crosses a marker) | **missing** | triggers on transport time |
| Drawing (gfx, ReaImGui) | **better**: egui behind a ReaImGui-shaped Luau API, and `gfx`; a runtime extension's own GPU drawing as a retained layer tree the host keeps (`features/scene`: fragments by hash, motion on the wall clock or the playhead, hit regions and drags with inertia, the engine's lock-free values), so the extension is off the frame path, linked in or as a process (#837, #840) | bytes on the wire to the browser (#835); presentation above 60 fps measured (#839) |
| A native renderer in a window of its own (an extension's own OpenGL/Vulkan window, video) | **better on Linux and macOS**: a `process` view runs the program from the package in its own crash domain and shows its GPU frames (a dma-buf, an `IOSurface`, or shared memory) in a pane, tab or pop-out window, input forwarded, in every build of the window (#838) | Windows (DXGI shared handles); the frame as a layer of the compositor's tree |
| Drawing into REAPER's own windows (js_ReaScriptAPI) | **better**: overlay layers in a view's coordinates | — |
| Menus and toolbars (`hookcustommenu`, `toolbar_icon_map`) | **partial** | contributions to named menus and to a target's context menu (also just before it shows), toolbar items with icons and toggle state |
| New media formats, project importers (`pcmsrc`, `pcmsink`, `projectimport`) | **missing** | decoder, encoder and importer contributions, off the audio thread, with peaks |
| Realtime audio (`Audio_RegHardwareHook`, JSFX) | **JSFX: covered in Studio (ysfx); the rest missing** | a separate `processor` world on the `no_std` audio-graph contract: memory taken up front, no imports in the hot path, compiled ahead, swapped between blocks; a device-buffer tap for meters. CLAP already covers compiled third-party DSP |
| Linked files, "edit with" (`file_in_project`, `editor`) | **missing** | project resource references (consolidate, rename) and an "edit with" opener |
| Accessibility (OSARA, a native extension) | **missing** | an AccessKit tree for the app's and extensions' views; the surface API and focus/selection streams cover what OSARA hooks |

What REAPER cannot do and this does: sandbox and permissions (a REAPER
extension can take the program down, and does), hot reload of everything
(native extensions need a restart), typed and versioned APIs (REAPER's
are raw pointers and integer command ids), real UI for extensions on
every platform (REAPER's is Win32 through SWELL), drawing in a view's
coordinates instead of over its window, and phones, tablets and the web.

**In order of priority:** transactions and undo; the typed model in the
WIT world; extension data in the project; action middleware; the surface
contribution and binding values; lifecycle, timers and transport
triggers; a canvas and menu/toolbar contributions; codecs and importers;
the realtime `processor` world; accessibility, resource references and
the registry's ReaPack features.

## Finding and installing

**The extensions page** (a [settings section](@/architecture/settings-page.md)
and a workspace view, on every platform): search every registry the
person has (by name, what it adds, its kind — scripts, views, JSFX,
themes); each extension's page says what it adds, the permissions it
asks for, its versions, and where it runs (a process extension says
"desk only"); install, update, pin a version, disable, remove. ⌘O finds
extensions too (`ext:`). Updates arrive as the registry changes — "update
available", with the registry's changelog line, within the pin — and
apply by hot reload.

## What a project needs

A project names the extensions it uses, in its document's `meta`: each
id, the version range it was made with, and whether it is **required**
(the project's sound or structure depends on it: a JSFX on a track, an
extension's data a view reads) or **optional** (a convenience: an
overlay, a panel).

- **Opening it:** a missing required extension is listed — "This song
  uses Saturation (JSFX) and Lyrics Lane" — with **Install all**: one
  press, one permissions prompt for everything, installed and loaded
  without a restart, and the project opens. Optional ones are offered,
  not asked.
- **Joining a live session:** the same prompt when the link is opened.
  The host's app can hand the packages over the session's own peer
  link, so a person joining across the room needs no registry and no
  network beyond the session; a package is checked against its signature
  whichever way it came.
- **Small extensions travel with the project**, if the project's owner
  chooses: their packages (a JSFX's source and files, a component's
  `.wasm`) are blobs in the project file, so opening the file is enough
  to play it — still installed through the same prompt, never run
  silently.
- **What cannot run here** (a process extension on an iPad, a newer
  version than this app supports): the project still opens, the
  extension's data is kept untouched, its views say what they need, and
  a required effect is bypassed and marked — never dropped.

### Where a vault and a project name them

A vault names the extensions it needs in `.architect/extensions.toml`; a
project, in its own folder's `.architect/extensions.toml` (beside its
`deck.json`). Both are `Requirements`, the same a project document keeps
in its `meta` (`features/extensions/src/doc.rs`):

```toml
[[extensions]]
id = "fts.metronome"
name = "Metronome"
version = "^0.1"     # the versions it works with; "*" for any
required = true      # false: offered, not asked
```

Small extensions travel in the folder as signed archives,
`.architect/extensions/*.archext` (`vault_needs::Carried`). What a vault
carries is held to more than a registry's archive:

- **It is a source for that vault's plan only.** It is never one of the
  app's registries: no update, search, preview or other install takes from
  it, and only the ids the vault names are listed. `Extensions::open_vault`
  and `EXT_NEEDS` change nothing; `Extensions::accept_vault` searches it
  for those installs alone, after the person's registries (on a tie, a
  registry's copy wins).
- **Its signer must be the org key, or the key the installed copy of that
  id is signed by.** A registry's publisher key is not enough: a vault
  cannot bring a new publisher. Every install records its signer
  (`Install::signer_key`), and an update signed by another key is refused
  (`InstallError::SignerChanged`), from a vault or a registry.
- **It is read cheaply and once.** An archive is at most 16 MB
  (`vault_needs::MAX_ARCHIVE`) and the needs file 64 KB (`MAX_NEEDS`). A
  plan reads only the signed index and the manifest
  (`architect_extension_archive::peek`), cached by the file's path, size
  and time; every file is checked when it is installed.
- **No link is followed**, at any level (`.architect` itself included): a
  link, or anything but a plain file, is refused with why.

What cannot be installed is said, never forced:

- a newer version installed than the range takes: "0.2.0 is installed;
  this wants an older one" (`Missing::WantsOlder`);
- a version outside the installed copy's pin (`Missing::OutsidePin`);
- an install by id that would go older or past a pin is refused
  (`InstallError::Downgrade`) unless a person names that version
  (`EXT_INSTALL {explicit: true}`, `Extensions::install_with_in`).

`Extensions::open_vault` plans it (`features/extensions/src/vault_needs.rs`
`VaultNeeds`: the `Plan`, the refused archives, a file that does not
read); `EXT_NEEDS {folder}` answers the same to an agent. The app draws it
over the vault, or over the project on show, planned on a thread of its own and again only while a registry's refresh
could change it: each required one, each optional one, each that cannot be
had with why, and **Install all**. Its previews (`Extensions::previews_for`)
are one consent prompt, listing in words what each asks, and what was
dropped and why; nothing installs before that one yes
(`Extensions::accept_vault` with no grant answers `NeedsConsent` for each),
and after it everything is installed and loaded without a restart.
"Not now" puts it away for the run.

Why `.architect/`: it is the vault's own folder (the decks' layouts are
there) and syncs with the vault's files today. #683 narrows which
top-level dot-folders sync, the owner's decision; this needs `.architect/`
(`extensions.toml` and `extensions/`) to keep syncing.

A share link names its own extensions with the share, not the vault's
file (see "A share link's extensions").

Tests: `features/extensions/tests/vault_needs.rs`.

### A person's installs on all their devices

What a person trusted and installed on one device installs on their
others with the same grants and no new prompt, and what they removed or
revoked goes there too (#683's decision). Their record is in the vault's
own folder, which syncs: `.architect/people/<account id>/extensions.toml`
(`features/extensions/src/person_sync.rs` `PersonInstalls`), **signed by
the device that wrote it** (`extensions.toml.minisig`).

- **Who it is from.** Each device has a key of its own (`DeviceKeys`:
  `<app dir>/device-key`). A device applies a record only when it is
  signed by its own key or by one of the person's devices it trusts; the
  person says so once a device ("It's mine: trust it", `DeviceKeys::trust`).
  A record anyone else wrote, or an unsigned one, is refused and nothing
  of it applies (`RecordError`). A node also refuses anyone but the person a
  write under `.architect/people/<id>/`
  (`features/vault/vault-live/src/write_guard.rs` `ExtensionPaths`).
- **What it says.** Each entry is the last word on its id (`at_ms`): its
  version, pin, signer, archive (in `.architect/extensions/`, its SHA-256
  pinned), grants, or that it was removed. Each install, removal or revoke
  made here is published at once (`Extensions::sync_person`,
  `on_installs_changed`); `Extensions::apply_installs` applies the entries
  newer than this device's own last word.
- **The same rules on every device.** The archive the entry pins, at most
  16 MB, no link followed, signed by the key the person's own device named;
  no downgrade, no step past a pin; grants written only once it installed.
- **Actions.** `EXT_INSTALLS_APPLY` and `EXT_INSTALLS_PUBLISH` act for the
  signed-in person only: the app's own call or that person's; anyone else
  is refused.

Other people (the vault's other members, a share's guests) are not in a
person's record: grants never transfer. They get the vault's (or link's)
needs list and are prompted once.

vault-live's manifest lists `.architect/` (the only top-level dot-folder
it syncs; `features/vault/vault-live/src/sync.rs`).

Tests: `features/extensions/tests/person_sync.rs`.

### A share link's extensions

A share link's guests run what the owner chose for that link, not the
vault's list: a shared recipe opens in the meal-plan extension's view. It
is kept beside the share's note, as its comments are, in
`_architect/shares/<code>.extensions.toml`
(`features/extensions/src/share_needs.rs`), so it syncs with the share and
the node serves it with it. The share's record (`architect-spaces`
`Share`) is unchanged: that crate does not depend on this one, and the
file reuses `Requirements` as it is.

```toml
[[extensions]]               # the link's Requirements
id = "health.meal-plan"
name = "Meal plan"
version = "^1.2"

[[extensions]]
id = "setlist"               # no package: the distribution's linked core
required = false

[[packages]]                 # signed archives in _architect/shares/<code>.extensions/
id = "health.meal-plan"
version = "1.2.0"
archive = "health.meal-plan-1.2.0.archext"
sha256 = "…"

[hosts]                      # what the owner lets each fetch, of what it asks
"health.meal-plan" = ["api.example.com"]
```

An audience's link has a file of its own, or takes its share's
(`share_needs::file_for`).

**The owner** chooses when sharing: `EXT_SHARE_NEEDS_SET {folder, code,
extensions: [{id, required?, hosts?}]}` (`Extensions::set_share_needs`).
A built-in comes from the guest's linked core; an installed one is copied
in as its signed archive from its registry, verified, its SHA-256 pinned.
A development copy or an unsigned install is refused (guests take signed
packages only), and so is a host its manifest does not ask for.
`EXT_SHARE_NEEDS {folder, code}` reads it back.

**The guest host** (the web app's, G1; iOS's; the desktop's) calls the
model, which is wasm-clean (`verify_package` with feature `archive`):

1. `LinkNeeds::from_toml(text)`: the link's file, checked (at most 64 KB,
   each id once, packages only for named ids at a version their range
   takes, plain `.archext` names, a SHA-256 each, no host `*`).
2. `plan_link(&needs, &here)`, with `here` the guest's running ids and
   versions (its linked core's among them): `ready`, `fetch` (each
   requirement with its package), `missing` (`NotInCore`); `prompt()` is
   "This link uses Meal plan". One consent for `fetch`, as for a vault.
3. For each package, its bytes (from the node's `/s/<code>/…` route for
   `packages_dir(code)/<archive>`, or a peer): `verify_package(&bytes,
   &needs, &package, &trust, platform)`, with `trust` the org key and the
   share owner's publishers. It checks the pinned SHA-256, the signature
   and every file, the id and version, and `platforms`; it answers the
   manifest, the verified files and the guest capabilities.
4. Run it within `guest_capabilities(&manifest.capabilities,
   needs.hosts_of(id))`: read-only `ShareGuest` documents of the share and
   the selection, notifications, prompts, menus and badges; no writes, no
   actions past its own, no folders, ports, language servers or peer
   connections; `fetch` only to the approved hosts it also asks for. Refuse
   `[[secrets]]` outright.

**Who writes it.** Only the share's owner: the node's write guard
(`features/vault/vault-live/src/write_guard.rs` `ExtensionPaths`, installed
by the node over its shares) refuses any other account a write to
`<code>.extensions.toml` or `<code>.extensions/`. Reads and writes on the
owner's device follow no link and check sizes before reading
(`vault_needs::real`).

**What the guest host must hold to** (G1):

- the `documents` a guest's extension reads are the share's, read-only
  (`ShareGuest`), never the vault's;
- it takes no trust from the vault's files: `trust` for `verify_package`
  is the org key and the keys the guest host itself holds for the share's
  owner, never a key named in the link's file or the vault;
- the consent is the guest's own, per person; nothing from the owner's
  grants carries over.

Tests: `features/extensions/tests/share_needs.rs`,
`features/vault/vault-live/src/write_guard.rs`.

## On each platform

| | Data | WASM | Scripts | Processes |
|---|---|---|---|---|
| macOS, Linux, Windows | yes | compiled | yes | yes |
| Android | yes | compiled (Play allows code in a VM) | yes | no |
| iOS | yes | interpreted (Pulley), in the release build (#707) | yes | no |
| Web | yes, and linked ones compiled into the web build | the browser's WebAssembly (components transpiled) | not yet (Luau is C++, #495) | no |

On iOS, a catalogue of third-party features is the App Store's §4.7
case (an index, moderation, consent); themes, keymaps, frame views and a
person's own scripts are not.

**Installed WASM extensions on iOS (decided 2026-11-14, #716).** The
App Store build ships `wasi`, so a person can install WASM component
extensions in the iPhone and iPad app, positioned under guideline 2.5.2
the way Obsidian's community plugins are:

- **The person installs them.** Nothing is downloaded or turned on by
  the app itself; an extension comes from a registry or a file the
  person chose, after a consent prompt that lists its capabilities.
- **Sandboxed add-ons.** A component runs in Wasmtime with no ambient
  access: no files, network, or app state but what its manifest asks for
  and the person granted (`hostcall::check`), each instance under a
  memory, fuel and time limit.
- **Interpreted, not compiled to native code.** Pulley interprets each
  component's bytecode: no JIT, no executable memory, no W^X exception
  (`wasi::PULLEY`).
- **They cannot change the app's core features.** An extension adds
  views, actions and handlers through the host's contribution points;
  it cannot replace the app's code, its sign-in, its purchases or its
  review-relevant behaviour, and it can be turned off or removed at any
  time with its data kept.

## Risks

- The component tooling is young: WASI 0.3 (async components) needs
  Wasmtime 46+, and the newest needs Rust 1.95 — past the fleet's 1.94.
- Pulley on iOS is a lower-tested tier (Wasmtime's memory reservation
  is down to `PULLEY_RESERVATION`, 64 MiB, with no guard, for App Store
  builds).
- App Review: installed WASM extensions on iOS rest on guideline 2.5.2's
  reading for user-installed, sandboxed, interpreted add-ons that cannot
  change the app's core features (Obsidian's community plugins are the
  precedent; "On each platform" above). A reviewer who reads it
  otherwise would have the iOS build ship without `wasi`
  (`cargo xtask dist ios --no-wasi`) until it is settled; linked
  extensions, Luau and pages are unaffected.
- Frame documents have to be expressive enough for real views without
  growing into a second Dioxus.
- Old WIT versions are kept forever.

## The registry

A git repository of an index, reviewed by pull request (Zed's
`extensions` repository): each version an entry naming its signed
archive, which the extension's own release workflow built, signed and
published as a release asset ([Distribution](#distribution)). The app
reads the registry's index, installs by id and version, and checks the
signature. A second registry (an org's own, a
local folder) is one more git URL in the settings, and a **ReaPack
repository** (`index.xml`) is a source too: its packages install as the
app's package kinds (Studio's JSFX, scripts, presets).

## Open questions

1. **First target:** scripts and data first (small, every platform, no
   WIT tooling), then processes over vox (the transport exists), then
   components?
2. **Text input and IME across the boundary,** and focus moving between
   the app and an extension's view: the hardest part of remote views.
