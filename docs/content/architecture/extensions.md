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
> **Canvases landed (2026-11-01)** in `architect-canvas`
> (`features/canvas`): a view's coordinate spaces
> (`features/canvas/src/space.rs` `Spaces`: time across, lanes down),
> layers drawn over a view by kind on a plane, passing the pointer
> through except over the regions they claim
> (`features/canvas/src/layer.rs` `Layer`, `Overlays`), an
> immediate-mode UI in ReaImGui's shape (`features/canvas/src/imgui.rs`
> `Ui`: windows, text, buttons, checkboxes, sliders, progress bars,
> `SameLine`, ids), and both in front of Luau as `ImGui` and `Canvas`
> on a live script, a frame at a time
> (`features/canvas/src/luau.rs` `Session`; scripts gained `defer` and
> modules, `features/scripting/src/luau.rs` `Runtime::start`, `Live`,
> `Module`). On Blitz, a layer mounts in its view and a window is a
> workspace pane (`features/canvas/src/ui.rs` `CanvasLayers`,
> `window_view`, `use_tool_windows`); an extension declares an overlay
> in its manifest (`[[overlays]]`, `features/extensions/src/manifest.rs`
> `OverlayDecl`). The lab's arrangement has a marker lane over it
> (`apps/lab/ui/src/marker_lane.luau`). Drawn as boxes, not through
> egui: Blitz has no texture widget yet. Not yet: egui (or Dear ImGui)
> behind the same API, `gfx` images and fonts, the WIT interface,
> layers from Dioxus views or frame documents, the extension host
> starting overlays itself, Studio's arrangement publishing its spaces.
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
> `plan`), and the `EXT_*` actions. The lab's metronome is one
> (`apps/lab/extensions`). Not yet: WASM runners, overlays, signed
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
> (`features/extensions/src/remote.rs` `RemoteView`, a Blitz sub-document
> filled by `RemoteDocument`, which checks every edit, and a placeholder
> when the process is gone). The wire is architect's own, versioned
> (`features/extensions/src/wire.rs` `WIRE`). The lab's clicker is one
> (`apps/lab/clicker`); `features/extensions/tests/remote_view.rs` clicks
> through a real child process and kills it. Not yet: stdio, lazy start
> and idle stop, state handed over on a reload, the host service an
> extension calls back into, pointer, wheel and IME events, the
> allowlist, Windows.
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
  the host document) sized by the pane it is in. A shared-tree mount
  (content-sized, routed events) is the later step, if panels need it.
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
| WASM components | rebuilt (`cargo component build`), re-instantiated: seconds, state kept |
| WASM views' `rsx!` | Dioxus's own hot reload: the extension's `VirtualDom` takes template changes from the dev server, so markup and style edits apply without rebuilding the `.wasm` — instant |
| Processes | rebuilt and restarted by the supervisor, state handed over; in development the extension can run under `dx serve` with Rust hot-patching (it is its own binary), so most code edits apply without a restart at all |

**The author's loop:** `architect ext dev <path>` builds the extension,
installs it into the running app as a development copy (marked as such,
its capabilities granted for the session), watches its sources, and
reloads on every save; its logs and panics stream to the terminal and to
the app's diagnostics. A development copy shadows an installed one with
the same id until the session ends.

## Immediate-mode UI for scripts (ReaImGui's)

A script that wants a window draws it every frame, ReaImGui's way. We do
not build a toolkit for it, and we do not need C++:

- **egui draws it.** egui is pure Rust (MIT/Apache) and builds for every
  target we ship, the web included; `egui-wgpu` renders into a texture on
  the app's own wgpu device, and a Blitz custom widget shows it — so a
  script's window is a pane like any other: docked, tabbed, stacked,
  saved in screensets. The widget forwards the pointer and keys to egui.
  Its style comes from the shell's theme.
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

## Capabilities

An extension's manifest names what it needs: the network hosts, the
folders, the action namespaces it may run, whether it may change or
destroy (an action's effect: read-only, mutating, destructive). The
person grants them when installing, and can revoke them. Every call
across the boundary is checked, and the span records `ext.id` and the
`perm.*` outcome ([logging](@/architecture/diagnostics.md)). Each
extension runs in its own Wasmtime store with fuel and memory limits,
off the audio thread and off the engine's thread.

## Packaging

A signed archive: `extension.toml` (id, version, `api` version,
capabilities, contributions), the `.wasm`, assets. Installed from a
registry (a git repository of manifests reviewed by PR, as Zed's, to
start), or from a file. Hot reload re-instantiates the component.

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
| Timers, background scripts (`defer`), `atexit`, startup scripts | **partial** | timer and frame-tick imports; long-running scripts with a run state shown as `checked`, a relaunch rule, an exit hook; lifecycle events (app started, project opened, saved, closed) |
| Marker actions (an action when the playhead crosses a marker) | **missing** | triggers on transport time |
| Drawing (gfx, ReaImGui) | **covered**: egui behind a ReaImGui-shaped Luau API, and `gfx` | — |
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
extensions too (`ext:`). Updates arrive as the registry changes, and
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

## On each platform

| | Data | WASM | Scripts | Processes |
|---|---|---|---|---|
| macOS, Linux, Windows | yes | compiled | yes | yes |
| Android | yes | compiled (Play allows code in a VM) | yes | no |
| iOS | yes | interpreted (Pulley) | yes | no |
| Web | yes | the browser's WebAssembly (components transpiled) | yes | no |

On iOS, a catalogue of third-party features is the App Store's §4.7
case (an index, moderation, consent); themes, keymaps, frame views and a
person's own scripts are not.

## Risks

- The component tooling is young: WASI 0.3 (async components) needs
  Wasmtime 46+, and the newest needs Rust 1.95 — past the fleet's 1.94.
- Pulley on iOS is a lower-tested tier, and Wasmtime's default memory
  reservation has to come down for App Store builds.
- Frame documents have to be expressive enough for real views without
  growing into a second Dioxus.
- Old WIT versions are kept forever.

## The registry

A git repository of manifests, reviewed by pull request (Zed's
`extensions` repository): each extension is a manifest naming its source
repository and a commit or tag; a CI job builds the `.wasm` (or checks the
data), signs the archive and publishes it as a release asset. The app
reads the registry's index (a file the CI keeps), installs by id and
version, and checks the signature. A second registry (an org's own, a
local folder) is one more git URL in the settings, and a **ReaPack
repository** (`index.xml`) is a source too: its packages install as the
app's package kinds (Studio's JSFX, scripts, presets).

## Open questions

1. **First target:** scripts and data first (small, every platform, no
   WIT tooling), then processes over vox (the transport exists), then
   components?
2. **Text input and IME across the boundary,** and focus moving between
   the app and an extension's view: the hardest part of remote views.
