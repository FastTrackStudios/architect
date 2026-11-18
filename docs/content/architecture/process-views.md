+++
title = "Process views: another program's GPU frames in a pane"
description = "A pane drawn by a program of its own (its own wgpu device, its own crash domain), its frames shared with the app's window as dma-buf textures or shared memory, with input, resizing and a stopped view that restarts; and the process view kind any extension declares."
weight = 69
+++

> **Status: Linux, built (#579, #641); the `process` view kind for every
> extension, built (#838).** The crate is `features/process-view`
> (`architect-process-view`). macOS, Windows and the browser are designed
> below, not built.

A process view is a view drawn by a program of its own: it renders each
frame on its own GPU device, in its own process, and the app shows the
newest frame in the pane's place. A program that hangs leaves its last
frame up, marked "Not responding"; one that dies leaves "View stopped"
and why, with a Restart button. Nothing it does reaches past its pane.

It is not a process extension's remote view: that one is a Dioxus
document the app draws (the default `dioxus` kind,
`docs/content/architecture/extensions.md`). A process view's pixels are
the program's.

## The parts

- **The wire** (`features/process-view/src/wire.rs`): the messages,
  `ToGuest` (hello with the pane's size and the window's GPU adapter,
  resize, input, theme, release, tick) and `ToHost` (the buffers a
  program draws into, then each frame). They travel as datagrams on a
  socket pair (`features/process-view/src/sock.rs`), with the frames'
  file descriptors beside them. The program finds its end in
  `ARCHITECT_VIEW_FD`.
- **The program's side** (feature `guest`): `guest::serve` serves a
  `PaintView` (`architect_workspace::PaintView`, the same trait a linked
  paint view implements): its own wgpu device on the app's adapter
  (by vendor and device id, else the first Vulkan one), Vello, and a
  ring of buffers it exports.
- **The app's side** (feature `host`): `host::ProcessViewPane`, the pane
  (`ProcessViewWidget`, a Blitz custom widget, plus a status line and the
  stopped state); `host::Session`, one running program, kept in
  `host::sessions` by key so a pane that moves keeps its program.
- **Extensions' views** (feature `extension`): `extension::use_process_views`
  draws any extension's `kind = "process"` view with the pane (below).
- **The demo** (feature `demo`): `demo::Orbits`, the lab's scene, and
  the `process-view-demo` program that serves it.

## Frames

A frame reaches the window one of two ways (`wire::Transport`):

- **dma-buf** (Linux, Vulkan external memory): the program renders into
  textures it exports; the window imports each as a texture of its own
  device (`features/process-view/src/gpu.rs` `import_dmabuf`) and draws
  it in place, no copy. Both devices must be on the same GPU and the
  window's device must have `VULKAN_EXTERNAL_MEMORY_DMA_BUF`
  (`architect-workspace`'s `external-textures` feature asks for it).
- **Shared memory**, where a dma-buf cannot be imported (another GPU,
  another driver, or `ARCHITECT_PROCESS_VIEW_TRANSPORT=shm`): the program
  reads each frame back into a shared mapping; the window uploads it
  (`write_texture`). One copy each way; fine for a pane, not for 240 fps
  at 5K.

The window hands each slot back (`Release`) when it no longer needs it
and says when it showed a frame (`Tick`), so the program never draws
faster than the window shows and never writes a slot being read.

Input (pointer, wheel, keys, IME, focus), the pane's size and scale, and
the app's theme go to the program as they happen; nothing waits for its
answer.

## The `process` view kind: any extension's (#838)

An extension of any kind (data, Luau, TypeScript, WASI or process) may
declare a view drawn by a program in its package, and it is shown the
same whichever copy is loaded: built in, installed or in development.
Nothing has to be compiled into the app.

<!-- doc-check: off -->
```toml
[capabilities]
programs = ["bin/ignition"]

[[views]]
id = "room"
title = "Room"
kind = "process"
# the program in the package that serves the view, and its arguments
program = "bin/ignition"
args = ["--process-view"]
```
<!-- doc-check: on -->

**The shape.** `program` and `args` on the view, as `[entry] path` and
`args` are for a process extension's program: a view names what draws
it, as a `web` view names its `src`. `Manifest::check`
(`features/extensions/src/manifest.rs`) holds that a `process` view names
a `program`, that only a `process` view names one (or `args`), and that
it is a path in the package (no `..`, nothing absolute). The `process`
kind is the app's (`features/workspace/src/kinds.rs` `PROCESS`,
`KindSpec::process`), registered on every platform, so the extension
loads everywhere.

**Consent.** Running a native program is a capability the person says
yes to: `capabilities.programs` names each program, one by one, and
`Manifest::check` refuses a `process` view whose program is not asked
for. It goes through the same consent path as every capability
(`features/extensions/src/capabilities.rs` `grows_from`, `within`,
`may_run_program`; `features/extensions/src/consent.rs`): an installed
copy runs a program only once it is granted, an update that asks for
another program asks again, a grant taken back stops the program within
a second. A built-in or development copy runs with what it asks, as for
every capability. A share link's guest never gets one: the guest profile
grants no program (`features/extensions/src/share_needs.rs`
`guest_capabilities`).

A `kind = "process"` extension's own `[entry]` program is not a
capability today: installing it is the consent (the install's preview
names the program, `manage.rs` `runner_words`). A process view asks even
there; the gate is stronger, not weaker.

Where the person is asked:

- **Installing** (the store, `EXT_INSTALL`): the program is one of the
  lines of the install's consent ("Run bin/x, a program in its package,
  on this computer to draw its views"); nothing is installed without a
  yes.
- **In the pane**, when that yes is all that is missing (a grant taken
  back, an update that asks for a program): the pane asks "Run bin/x, a
  program in its package, on this computer to draw this view?" with
  **Allow**, which widens this device's grant by that program alone
  (`process_view::needs_grant`, `grant_program`). The pane checks again
  every 500 ms, so a grant given or taken back elsewhere shows at once.
- **A development copy** (`architect dev --extension …`, `EXT_DEV`) is
  not asked: it runs with what it asks, as for every capability, because
  the person put it there. Its manifest must still ask for the program
  (`Manifest::check`), so the installed copy it becomes asks.

A program stopped because its grant was taken back or its extension
unloaded is retired (`host::retire`): its pane says why, and its Restart
button starts nothing.

**Settings.** An argument may name one of the extension's own settings,
`{{setting:<key>}}`, the placeholder a fetch's header uses for a secret
(`{{secret:<name>}}`), checked by `Manifest::check` against its
`[[sections.settings]]`. The value (the one set, else the default; text
as it is, a switch `true` or `false`) is put in when the program starts
(`Host::setting_value`), and a change starts the program again
(`extension::refresh`). An argument that is a setting alone and comes
out empty is left out, so a value that may be empty goes as
`--show={{setting:lighting.show}}`. A secret never goes to a program.

<!-- doc-check: off -->
```toml
[[views]]
id = "room"
kind = "process"
program = "bin/ignition"
args = ["--process-view", "--clock={{setting:ignition.clock}}", "--show={{setting:ignition.show}}"]
```
<!-- doc-check: on -->

Why arguments, not the program asking the host: a process extension
reads its settings through the host service over its own connection,
but a process view's program has only the frame socket, and the
programs that serve one (Ignition's Bevy renderer speaks the wire
itself, on another wgpu) would each need a host client. A setting in an
argument needs nothing of the program but its command line, and the
restart is what such programs already do for a new show.

**Starting it.** `ExtensionView` (`features/extensions/src/interface.rs`)
asks `ProcessViews`, which the app provides with
`architect_process_view::extension::use_process_views`. Each pane first
checks the view (`features/extensions/src/process_view.rs` `launch`):

- the platform (`process_view::not_here`): Linux only for now; elsewhere
  the pane says why, as other platform limits do;
- the grant (`Host::capabilities`, what it may reach now);
- the program: a file in the package, executable, and not a link that
  points out of it (`process_view::package_file`, the check a web view's
  files get).

Then it starts the program for that pane (`extension::open`, one session
per `<extension>/<view>#<pane>`), in the package's folder, and draws it
with `ProcessViewPane`.

**Its life.**

- A pane moved to another tab group, a split or a pop-out window keeps
  its program: the session is the process's, found again by the pane's
  key, and its widget greets the program again in the new window.
- A pane closed stops it at the next check (`extension::reap`, every
  250 ms): each pane registers whether it is still in its workspace
  (`extension::register_pane`), and a move to another group, a split or
  a pop-out window keeps the pane in the one workspace model, so the
  program runs on however long the move takes; a tab behind another
  keeps it too, idle (a program draws only when a shown frame ticks it).
  A pane drawn where there is no workspace to ask stops
  `extension::BACKSTOP` (2 s) after it went. A close and a reopen before
  the check keep the same program. Its extension unloading, or its grant
  taken back, stops it at the same check (`extension::fate`).
- A program that dies is started again as a process extension's is
  (`host::Restart`: three times in a row, 250 ms doubling to 10 s,
  counted again after 30 s running), the pane saying why it died; then
  it waits for the Restart button.
- A development copy rebuilt: `EXT_DEV` reloads it (its folder's
  fingerprint changed, `features/extensions/src/watch.rs`), the program's
  stamp (its size and time, and the copy loaded) changes, and the pane's
  session starts the new program in place.

The test is `features/process-view/tests/extension_kind.rs`: a `data`
extension whose only view is the demo program, loaded as an installed
copy (refused until granted, then its frames arrive over shared memory),
stopped when its grant is taken back and when it unloads, loaded again as
a development copy, started again when its program changes; and a
program killed, started again, saying why. The example package is
`examples/extension/process-view` (`just process-view-example`).

## In the app

Seen on Linux in an isolated instance (`architect dev`, a headless Loft
session, #838): the pane in a tab, input (pointer, keys, typed text) and
focus reaching the program, a resize, a move to another group, a move to
a pop-out OS window and back with the same program running throughout
(the pop-out window's device takes the shared-memory fallback), a grant
taken back and given again with Allow. Ignition (the FastTrackStudios/Ignition
repository's lighting extension, id `ignition`), a Bevy renderer on wgpu 29 that
speaks the wire itself over shared memory, ran the same way as a
development copy: its 3D room in its `room` pane, its arguments its
settings (`--camera={{setting:ignition.camera}}`, a pick restarting it). The program gets the app's theme
(`architect_workspace::paint::pane_tokens`), as a linked paint view does.

`APP_SHOT` does not show a process view's frames: it paints the page
again on the CPU (`features/workspace/src/shot.rs`), and the frames are
GPU textures of the window's own device; a pop-out window is not shot at
all. Capture a process view with the compositor (`loft-shot` in a Loft
session) until the shot reads frames back.

## Not built yet

- **macOS**: the program renders into an `IOSurface`-backed texture and
  sends its Mach port (an `IOSurfaceRef` over a Mach message, or the
  surface's global id over the socket on older systems); the window's
  Metal device makes a texture from it (wgpu's Metal hal,
  `create_texture_from_hal` over `newTextureWithDescriptor:iosurface:`).
  Shared memory first (the same upload as Linux's fallback, the socket as
  a Unix socket pair, which macOS has), `IOSurface` after.
- **Windows**: a DXGI shared NT handle (`D3D12` `CreateSharedHandle`,
  duplicated into the app's process), opened by the window's device
  (`OpenSharedHandle`); the socket a named pipe or an `AF_UNIX` socket
  (Windows 10 has them, without descriptor passing, so handles go by
  `DuplicateHandle`). Shared memory first, as on macOS.
- **The layer tree**: the frame as an external-surface layer of the
  compositor (#834, #838's third step), so it composites with 2D layers
  over and under it and survives a move between windows by the scene,
  not the session. Today the widget draws the newest frame as an image
  in its own box.
- **The browser**: a browser does not start programs. The same view
  would be a WASI component or a worker drawing into an `OffscreenCanvas`
  handed to the page; not a process view.
