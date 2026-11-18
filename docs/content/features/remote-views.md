+++
title = "Remote views"
description = "A process extension draws Dioxus views in its own process; the app shows them in workspace panes over vox, and a placeholder with the reason when the process dies."
weight = 53
+++

## What it is and when to use it

A process extension is a program of its own. For each view the app opens, it
runs a Dioxus `VirtualDom` on its own thread and streams the DOM edits, as
architect's versioned wire edits, over vox on a local socket. The app mounts
each view as a sub-document inside a workspace pane and sends the person's
clicks, keys and form input back. Every edit is checked before it is
applied, so a misbehaving process cannot crash the app. If the process dies,
the pane shows a placeholder with the reason and a Start again button; the
supervisor restarts it with backoff, and the view comes back.

Use it for a tool that may crash, runs another runtime, or should be
developed and restarted apart from the app, but needs real UI. Design:
[Extensions](@/architecture/extensions.md), sections on processes over vox and
on views rendered remotely; the manifest side is
[Extensions](@/features/extensions.md).

Example: `examples/extension`, a registry with a Luau extension and a process extension loaded by a small host, hot-reloaded on save; walked through in [Write an extension, end to end](@/getting-started/extension.md).

<!-- media: remote-view-clicker — the sandbox's Clicker in a Vault pane: clicking it, then kill -9 on its process, the placeholder with the reason, Start again and the counter back -->

## A minimal example

The extension, `architect-extensions` with `guest`, from
`apps/sandbox/clicker/src/main.rs`:

```rust
use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)                       // fn() -> Element
        .action("LAB_CLICKER_CLICKS", |_| async { Ok(serde_json::json!(clicks())) })
        .serve()                                        // the socket the host passes
}
```

Its manifest, `apps/sandbox/clicker/extension.toml`, has `kind = "process"`, an
`[entry]` with the program's path, its `[[actions]]` and a `[[views]]` entry
with `id = "counter"`.

The app, with `ui` and `remote` (desk only), after `apps/ui/src/extensions.rs`
and `apps/ui/src/vault.rs`:

```rust
use architect_extensions::process::{ProcessRunner, Processes};

static PROCESSES: LazyLock<Processes> = LazyLock::new(Processes::default);

let host = Host::new(list, sections)
    .with_runner(ProcessRunner::new()?.with_processes(PROCESSES.clone()));
// Extensions::new(host, ..), then install or dev the extension

View::new("clicker", "Clicker", |_| rsx! {
    architect_extensions::remote::RemoteView {
        processes: PROCESSES.clone(), extension: "lab.clicker", view: "counter",
    }
})
```

## Code tour

1. `features/extensions/src/proto.rs` `ProcessExtension`: the vox service a
   process serves (`hello`, `run`, `event`, `close`, and the `view`
   subscription), and `SOCKET`.
2. `features/extensions/src/wire.rs` `WIRE`, `Hello`, `Open`, `Frame`, `Edit`,
   `ViewEvent`: the versioned edits and events.
3. `features/extensions/src/process.rs` `ProcessRunner` (`with_restart`,
   `with_processes`): starts the program and waits for its hello, which must
   cover what the manifest declares.
4. `features/extensions/src/process.rs` `Process` (`state`, `open`,
   `restart`, `stop`), `State`, `Restart` (three tries with backoff by
   default), `Processes`.
5. `features/extensions/src/guest.rs` `Guest` (`view`, `action`, `serve`) and
   `WireWriter`: Dioxus mutations to wire edits, on the extension's side.
6. `features/extensions/src/remote.rs` `RemoteDocument`: a Blitz document that
   checks and applies each edit within its caps and forwards events.
7. `features/extensions/src/remote.rs` `RemoteView`: the pane's component;
   follows the process and shows the placeholder.

When the pane mounts, `RemoteView` waits for the process to be running, then
`Process::open(view)` subscribes to the view over vox. It installs a
`RemoteDocument` as the pane node's sub-document and applies each `Frame` as
it arrives. On the extension's side, `Guest` starts a `VirtualDom` on a
thread for that view, writes its renders through `WireWriter`, and
dispatches the events that come back. When the frames stop (the process
died or was unloaded), the view removes the sub-document, shows the
placeholder, and follows the process when it restarts.

## Input

Every kind of input reaches the view's handlers (wire 7, `INPUT_WIRE` in
`features/extensions/src/wire.rs`), in a process view, a WASI view
(`features/extensions/wit/extension.wit` `event-data`), a TSX or Luau
page (`features/pages/src/api.rs` `PageEvent`: `dx`, `dy`, `pointer_id`,
`pointer_type`, `buttons`, `scroll_top`) and in a browser
(`features/extensions/src/web.rs` `Pane`):

| Handler | Carries |
|---|---|
| `onclick`, `onmouse*`, `oncontextmenu` | `Pointer`: where, buttons, modifiers |
| `onpointerdown`, `onpointermove`, `onpointerup`, `onpointercancel`, `onpointerenter`… | `PointerData`: the pointer's id and type (`mouse`, `pen`, `touch`), pressure, tilt |
| `ontouchstart`, `ontouchmove`, `ontouchend`, `ontouchcancel` | `TouchData`: every finger down, those that changed |
| `onwheel` | `WheelData`, signed as the web's (a positive `delta_y` scrolls down; Blitz's own sign is winit's, the host turns it round) |
| `onscroll` | `ScrollData`: where the element is scrolled to |
| `oncompositionstart`, `oncompositionupdate`, `oncompositionend` | the text an input method is composing, then the text it committed |
| `onkeydown`, `onkeyup`, `oninput`, `onfocus`, `onblur` | as before |

- **A press holds the pointer.** After a `pointerdown`, the moves and the
  release go to the element pressed, over it or not, as a touch is captured
  on the web (in a browser, `web::Pane` calls `setPointerCapture`). A
  drag-and-drop list works without knowing where the pointer went. Blitz
  holds the pointer for custom widgets only, not for a sub-document, so a
  release can happen where the view can't see it. The host then lets go
  for it (`RemoteDocument::let_go`): when the pointer comes back, or leaves
  the window, with no button held, and when the pane loses the focus. The
  view gets a `pointercancel`, or a `pointerup` if it listens for no
  cancel, so a drag is never left grabbed. A view that listens for
  `pointermove`, but not `mousemove` or `touchmove`, owns its held moves:
  the document doesn't select the text under a drag.
- **Fast events are coalesced.** While the view has not yet answered an
  event, the moves, wheels, scrolls and composition updates that follow
  wait; a run of them on one element keeps only the latest (a wheel's
  deltas are summed; a wheel in other units waits as its own), and any
  other event sends what waits first, in order (`Coalescer`). At most 64
  wait. A slow view gets the latest move, not a queue of stale ones.
- **The data follows the event's name** (`wire::kind_of`): a browser
  sends `click` and `contextmenu` as `PointerEvent`s, and they reach the
  view as mouse data, as `onclick` needs.
- **The input method.** A field in the view turns the window's IME on
  when it takes the focus, with the caret area moved to where the pane is
  in the window, and kept there as the pane moves; its clipboard is the
  window's (`HostShell`). Focus leaving mid-composition ends it.
- **Older processes.** A process at wire 6 or below is sent none of the
  new kinds (`EventData::since`); it never had them.

Measured (`cargo test -p architect-extensions --features "remote guest
process wasi" --test remote_input round_trips -- --ignored --nocapture`;
a debug build on THEBATTLESHIP, the host document headless):

| Round trip | n | p50 | p99 |
|---|---|---|---|
| a key into a field, to its echo | 200 | 1.34 ms | 3.76 ms |
| a held move, to its echo | 200 | 1.15 ms | 2.23 ms |
| 500 rows in, then out | 40 | 3.84 ms | 6.63 ms |

The target is 16 ms from key to echo in a local pane (a frame at 60 Hz).
`REMOTE_BUDGET` (33 ms) marks a view busy past it, and a test holds the
p99 of a key's and a held move's round trip under it
(`round_trips_stay_inside_the_remote_budget`). 200 held moves sent before
the view answered any reach it as a handful
(`moves_while_the_view_is_busy_are_coalesced_to_the_latest`).

## Holding another extension's view

A remote view can hold another extension's view (`features/extensions/src/embedded.rs`).
A development copy of Bases uses it to draw a third party's base view type
inside its own view. The view renders a placeholder that names
`<extension>/<view>` and carries the input that view is given, an
`EmbedInput` (a base view's is `linked::BASE_VIEW_CLAIM` with its rows):

```rust
use architect_extensions::embedded::EmbeddedView;

rsx! {
    EmbeddedView {
        extension: "lab.kanban",
        view: "base.board",
        input: EmbedInput { claim: BASE_VIEW_CLAIM.into(), source: String::new(), props: rows },
        style: "height: 320px;",
    }
}
```

The host draws the named view over the placeholder's box, sized by it,
clipped to it and to the pane, and in front of the holding view so it takes
its own input. A linked view is drawn in the app's document; a process's or
a WASI component's is a remote view of its own (`remote::RemoteDocument::embedded`
lists the placeholders and their boxes, looked at after each frame and every
`PLACE_EVERY`, so a held view follows a view that scrolls). The input
reaches it as its `use_embed()`, again whenever the holder changes it. A
held view runs as its own extension, with its own capabilities; the holder
reaches nothing of it but its place and its input. At most 64 held views
(`MAX_EMBEDDED`), each input at most 1 MB; a placeholder that names no view
fails the holder, not the app. Test:
`features/extensions/tests/remote_embedded.rs`. Not yet: a page view (TSX,
Luau) held this way, and a held view inside a held view's own document
(each is drawn by the host, one level deep is what's tested).

## Scenes: painting in a remote view

A view can draw an element as an anyrender scene instead of DOM (#175):
a drawing, a canvas, a graph, an arrangement. The scene is recorded in
the extension's process, in content space, and the app draws it in its
own window's scene (Vello through Blitz on the desk, a `<canvas>` in a
browser).

```rust
use architect_extensions::scene::{RemoteScene, Affine};

rsx! {
    RemoteScene {
        content: strokes(),                 // Rc<Vec<RenderCommand>>, in content space
        transform: camera().transform(),    // pan and zoom
        onpointermove: move |e| { /* draw, or pan */ },
        onwheel: move |e| { /* zoom */ },
        onresize: move |size| { /* its size in the pane */ },
    }
}
```

- **What crosses.** Two attributes of the element, so the wire needs
  nothing new and Dioxus's diffing decides what is sent. The content goes
  the first time as `<hash>\n<json>` (a compact form of anyrender's commands: each path as SVG, each colour as RGBA bytes), and after
  that as `<hash>` alone while both sides still hold it: each side keeps
  the last 64 by the same rule (`SceneCache`). The transform is six
  numbers. Panning and zooming change only the transform.
- **Its size.** When the host lays the element out at a new size, it
  sends `architect-resize`, which reaches the view as the element's
  `input` (`RemoteScene`'s `onresize`).
- **Input.** The element's handlers get pointer, wheel and key events in
  its own coordinates, and a press holds the pointer (see Input above).
- **A linked paint view as a process.** `linked::Builder::paint::<V>` (#591)
  serves the same `PaintView` from a process copy: each frame it records
  is a `RemoteScene`'s content, and the element's input comes back to it
  as `ViewInput`s (`scene::remote_paint_root`).
- **Within its pane, unless it may draw over the app.** A scene is
  clipped to its element's box (`scene::framed`). An extension granted
  `overlay` (`capabilities.ui`) has its scenes drawn over the whole
  window instead, by a layer above the panes (`scene::OverlayLayer`;
  [Extensions](@/architecture/extensions.md), "The app's interface").
- **What the host accepts.** At most 4 MB per scene attribute
  (`MAX_SCENE_BYTES`), 100,000 commands (`MAX_COMMANDS`), 1,000,000 path
  elements, and finite numbers within ±10⁷. Layers are balanced: a pop
  with nothing open is refused, nesting stops at 64 (`MAX_DEPTH`), and
  layers left open are closed. Each side keeps at most 64 scenes and
  32 MB of them (`SceneCache`, evicting by count and by bytes, by the
  same rule on both sides). A scene past any of these fails the view,
  not the app. The host decodes a frame's scenes on a thread of its own
  before it applies the frame (`scene::predecode`), so a large one
  doesn't stall the window.
- **Frames paced to the display** (#840). A batch, and each change the
  view's document makes, asks the window for a frame through its
  `Pacer` (`features/workspace/src/pacing.rs`; `remote::HostShell::pacer`).
  Asked right after a frame that waited for its swapchain image (its
  render took at least half a refresh, so it ended at a refresh), the
  requests until the next refresh share one frame started just before
  the next image comes free: the window's thread is free meanwhile and
  the frame shows the newest batch. Started at once instead, such a
  frame built its scene in 0.6 ms and then waited about 7 ms of an
  8.3 ms refresh (a 120 Hz Mac) for its image, its content a refresh
  older than it had to be. After a frame that did not wait (an idle
  window, a frame slower than a refresh) it asks at once, so no wait is
  added where none was saved. `ARCHITECT_FRAME_PACING=0` turns it off.
- **Pans don't resize.** A new transform or scene updates the element's
  widget in place; only a new box tells the view its size, and the
  resizes of a dragged window are coalesced like moves.
- **Limits.** Glyph runs don't cross, because a font's bytes would have
  to go with them, so draw text as paths. Images and a backend's own
  paints don't cross either, and a gradient crosses as its first stop's
  colour. A browser replays the scene with Canvas 2D
  (`features/extensions/src/web_scene.rs`). A paint view in a process
  re-sends its whole scene for every frame it records; a view that pans
  a lot should use `RemoteScene` with a transform.

The example is `apps/sandbox/sketch`: freehand drawing with pan and zoom,
a link graph whose nodes drag (a paint view through `Builder::paint`),
and a 50-track arrangement. It stands in for Excalidraw, Canvas and the
graph view.

Measured (`cargo test -p architect-sandbox-sketch --test sketch
arrangement_budget -- --ignored --nocapture`; debug build, THEBATTLESHIP;
50 tracks of 40 clips, 2,000 fills):

| | |
|---|---|
| the first scene | 236,124 bytes (JSON) |
| a frame of panning | 12 bytes |
| a frame of the sketch's Shift-drag pan and wheel zoom | 30 bytes |
| host: the scene decoded and checked, off the UI thread | 3.63 ms, once |
| host: the scene built at its transform (p50 / p99) | 0.45 / 0.72 ms |
| the window's scene recorded (p50 / p99) | 0.30 / 1.31 ms |
| a pan's round trip, pointer to new frame (p50 / p99) | 1.53 / 4.97 ms |

## Layers: what the host keeps, moves and handles

A paint view that keeps what it draws returns **layers**
(`PaintView::layers`, `features/scene/src/layers.rs`): content by key,
each with a transform, an opacity, a clip and a z. From a process they
cross as a batch of binary operations (`features/scene/src/ops.rs`): a
font, an image or a fragment once by its hash, a layer when it comes or
changes, a layer that only moved as its transform alone. Linked in, the
same layers are composed with no serialization. Either way the host keeps
them and draws them every frame without asking the view, so at 240 fps
(a 4 ms frame) nothing that moves every frame waits on the extension.
Four things are the host's own:

- **Animation** (`features/scene/src/anim.rs`). A layer's `X`, `Y`,
  `ScaleX`, `ScaleY`, `Opacity` or `Visible` is a function of an input:
  a clock (`Clock::Wall`, the app's one monotonic clock; `Clock::Playhead`,
  the active source on the playhead, with a fallback when nothing plays)
  or a named value. The map is a `Linear` (`scale × input + offset`,
  wrapped, clamped) or `Steps`. A playhead line is `X` from the playhead,
  a lyric line `Visible` in steps, a meter `ScaleY` from a value. The host
  reads its clocks once a frame, so a pause, a seek or another rate needs
  no word from the view. It draws every frame only while a line moves on
  a running clock; a stepped layer rests until its next step
  (`features/scene/src/wake.rs` `Alarm`), and a seek wakes every window
  holding animated layers, a pop-out too (`wake::clocks_moved`, called on
  each playhead change).
- **Hit regions** (`features/scene/src/region.rs`). A layer may carry
  regions: a shape in its content space, an id, a cursor, a fill drawn
  over it while hovered or pressed. The host hit-tests them with each
  layer placed as its animations place it now, sets the cursor and draws
  the highlight, and the view hears `RegionEvent::Enter`, `Leave`,
  `Press` and `Release` by id (`PaintView::region`; a `RemoteScene`'s
  `onregion`, sent as the element's `input` named `architect-region`).
  A press on a region is the region's: the view does not get that pointer
  event. It still focuses the view, as a press anywhere else in it does:
  the scene's element takes the keyboard focus and the view hears
  `Focus(true)`, so the key typed after a first click on a chord is the
  view's (`features/extensions/src/remote.rs` `RemoteDocument`; a linked
  `PaintPane` is focused by Blitz; tested linked and as a process in
  `apps/sandbox/sketch/tests/paint_parity.rs`).
- **Drags.** A region's `Drag` turns the pointer's movement along an axis
  (at most one per axis: a fader one, a box two) into a `host.` value,
  scaled, clamped and stepped. A layer bound to that value follows the
  pointer at the display's rate; the view hears the value at most once a
  frame and then the release. It moves the value itself (automation) by
  sending another `initial`. A drag with a `coast` (a time constant up to
  10 s) is flicked: let go while moving, the host carries its value on,
  `from + speed × coast × (1 − e^(−t/coast))` on its wall clock, until it
  stops or reaches an end; a press stops it. A list or an arrangement
  scrolls with inertia with nothing from the view.
- **The engine's values** (`features/scene/src/engine.rs`). An engine
  writes `engine.` values into a fixed table of atomic slots
  (`EngineTable`, `EngineWriter::write`: two atomic stores, no lock, no
  allocation), and a pane granted `capabilities.ui = ["engine"]` reads
  them at compose time. Ungranted, a binding to one reads nothing and the
  layer stands as sent.

- **An engine in a process of its own** (`features/scene/src/engine/shared.rs`).
  A DAW or a lighting renderer is a process extension, so it can't
  write the host's table. It makes one in shared memory instead
  (`engine::shared::create`, a file under `$XDG_RUNTIME_DIR/architect-engines/<id>.values`,
  written whole under a temporary name and renamed in, never truncated),
  with the same two-store writes. It may list `host.` names too: a drag on
  a pane reading the table writes the value it makes into it as it makes
  it (`EngineTable::write_host`, from `Values::write`), and the engine
  reads it lock-free (`EngineWriter::read`), so a fader moves the
  engine's gain while it is held. Which table a pane reads: a pane
  granted `engine` reads the tables of the providers of the services
  its extension `[uses]` or `[requires]` (`engine::read_from`), looked
  for again every `PLACE_EVERY`, so an engine that starts later or
  restarts is found. The host checks the header, the names and the
  length before it maps (`engine::shared::open`).

**On the wire** (#835). A process at wire 8 (`wire::BYTES_WIRE`) is
opened with `ProcessExtension::scene_view`, and its batches cross as
bytes beside its edits. Its frames may be `Frame::Batches { edits,
batches }`. The element's `data-architect-layers` attribute holds the
batch's number (`scene::layers_marker`), and the host takes that batch
at the edit that sets it (`RemoteDocument::apply`). Before this, the
attribute carried the batch as base64 (`scene::layers_value`), and an
older peer still does: a host never opens an older process with
`scene_view`, and a process sends bytes only when opened that way.

- **Ordering against the edits** is exactly the attribute's:
  - a batch applies after every edit before its attribute and before
    every edit after it, in the same frame;
  - a batch no edit names, a name with no batch, or a number sent twice
    fails the view (`features/extensions/tests/scene_bytes.rs`).
- **Back-pressure** is the view's frames': one stream, in order. The
  view's thread awaits each frame's send, and while the host has not
  answered an event, the moves after it are coalesced (see Input above).
- **Copies.** On the guest, a batch is moved, not encoded, from the
  scene component (`scene::outbox`) into the frame. On the host, vox
  deserialises it once, and the receiver keeps a font or an image of at
  least `ops::SHARE_AT` (64 KiB) as a share of the batch's own bytes
  (`Receiver::apply_owned`, `Resource::from_shared`). A smaller one is
  copied, so it does not keep its batch alive.
- **Not yet on this path:** a WASI component's view (the WIT interface)
  and a page in the browser (its worker's messages). They still send the
  attribute.

Values are named by whose they are: `view.` (the view writes them, a
`VALUE` operation of a name and a number), `host.` (a drag, later a
control surface), `engine.` (the engine). Each writer writes its own
space only.

What the host accepts, checked on decode: every number finite and within
±10⁷; at most one animation per property per layer and 4,096 in a pane
(`MAX_ANIMATIONS`); 256 steps an animation; 1,024 values a pane; names of
64 bytes; 4,096 regions a pane and 1,024 a layer; two drags a region, on
different axes, each with its ends in order, a step above zero and its
initial value between them. What fails fails the view, linked in or not
(`Painter::failed` holds the same error a process view's host gives).
A batch is written at the lowest version that holds it: `ASO\x01` for
the first seven operations alone, `ASO\x02` once it animates, writes a
value or carries regions, which a host that reads only the first version
refuses whole.

Measured (`cargo test -p architect-sandbox-sketch --test sketch --
--nocapture`, `cargo test -p architect-scene --test engine --
--nocapture`; debug build with optimizations, voyager):

| | |
|---|---|
| the timeline's clips and motion, once | 70,789 bytes |
| a frame of playback, view sending its playhead's transform (before) | 85 bytes |
| a frame of playback, run by the host | 0 bytes |
| 60 frames of dragging a fader in a process | 0 bytes; its release 46 |
| 40 frames of dragging a box | 0 bytes |
| 188 engine writes at audio-callback rate, 3,436 frames drawn | 0 bytes |
| an engine write (p50 / p99 / max) | 167 ns / 500 ns / 3.2 µs, no allocation |

## Frames: measuring a view in its window

**Measuring.** `APP_FRAMES {seconds, label, path, drive, rate}`
(`features/workspace/src/frames.rs`; without `seconds`, the main
window's always-on meter) has the window record its own frames: per frame (`FrameRecord`) when it began and ended, its draw
(Blitz building the scene's commands) and its render (rasterise,
composite, present, waiting for an image included), the display's
refresh, the first input and the first batch it shows. The summary (`Report`): presented fps, refreshes dropped, the
interval, work, draw and render, input to frame and batch to frame, at
p50, p95, p99 and the most. `drive` feeds the window wheel or drag input
through its own event handler, so a run needs no OS input. `just frames`
(`scripts/frames.sh`) runs the sandbox's Sketch (`timeline`, 2,000 clips;
`timeline-20k`; `arrangement`) linked in (`architect-sandbox`) and as a
process (`architect-desktop dev --extension`), playing,
wheeling, panning and dragging, each in an instance of its own, and
prints a table. On the owner's ultrawide: `WIDTH=5120 HEIGHT=1440 just frames`.

Measured on voyager (MacBook Pro, M1 Max; its built-in display at
120 Hz, which is what it reports; the window 2560 × 1640 pixels; the
dev profile, renderer crates at opt-level 3; a machine shared with
another build, load average 30 to 90, so p99s and fps vary by tens of
percent between runs), 5 s each, fps and p50 / p99 in ms:

| | fps | interval | draw | render | input to frame |
|---|---:|---:|---:|---:|---:|
| `timeline`, playing, linked | 82.1 | 8.54 / 29.4 | 0.70 / 0.94 | 7.62 / 28.5 | — |
| `timeline`, playing, process | 76.7 | 8.50 / 33.6 | 0.64 / 0.92 | 7.67 / 32.8 | — |
| `timeline`, wheel, linked | 72.6 | 8.47 / 37.7 | 0.73 / 1.06 | 7.42 / 36.6 | 8.40 / 37.6 |
| `timeline`, wheel, process | 74.2 | 8.59 / 37.5 | 0.63 / 0.87 | 7.63 / 33.1 | 8.58 / 37.5 |
| `timeline`, drag (host-run box), linked | 83.8 | 9.08 / 39.5 | 0.95 / 13.5 | 7.20 / 35.9 | 8.85 / 44.4 |
| `timeline`, drag, process | 87.6 | 8.90 / 34.3 | 0.80 / 9.26 | 7.26 / 31.5 | 8.84 / 34.3 |
| `timeline-20k`, playing, linked | 25.0 | 33.1 / 122 | 15.6 / 83.9 | 10.2 / 70.6 | — |
| `timeline-20k`, playing, process | 35.7 | 21.4 / 104 | 5.78 / 79.0 | 7.40 / 68.6 | — |
| `timeline-20k`, drag, linked | 56.2 | 11.7 / 66.8 | 8.78 / 11.0 | 1.45 / 57.7 | 11.7 / 66.8 |
| `timeline-20k`, drag, process | 66.5 | 8.60 / 49.8 | 4.80 / 5.54 | 3.38 / 44.5 | 8.58 / 49.8 |
| `arrangement`, wheel, linked | 40.6 | 26.1 / 40.9 | 0.27 / 0.45 | 1.15 / 28.2 | 21.2 / 37.9 |
| `arrangement`, wheel, process | 91.4 | 8.50 / 32.6 | 0.83 / 2.34 | 7.12 / 31.2 | 8.46 / 32.4 |

What it shows, and what it can't:

- **A process view is not late here.** On this machine a runtime view
  draws as soon and as often as the same view linked in; input to frame
  is the same to a tenth of a millisecond at p50. Keyflow's 14 ms
  `present` against 0.46 ms (#840, a 60 Hz Linux session) does not
  reproduce on macOS. The mechanism #840 supposed does: a frame started
  right after another waits most of a refresh for its image (7.3 ms of
  its 8 ms here), one started after an idle gap does not (1 ms). But
  while input arrives the window draws every refresh anyway, linked or
  not: Blitz's document reports itself animating and asks for each next
  frame itself (`APP_FRAMES`' `again_animating`, 84 of 82 frames a
  second), so every frame, linked or not, waits for its image as a
  vsync-locked loop does.
- **What holds a view under 120 fps here is the window's GPU work, not
  the runtime.** With vsync off and the GPU waited for each frame
  (`FTS_PRESENT=immediate FTS_GPU_BARRIER=1`) a frame of the arrangement
  costs 6.8 ms (process) and 8.4 ms (linked) at 2560 × 1640: the page is
  rasterised whole after every wheel or click, which is nearly the whole
  8.3 ms of a 120 Hz refresh, so some frames take two or three. The
  20,000-clip timeline also spends 5 to 9 ms a frame building its clips'
  commands on the host; linked it is slower still (15.6 ms while
  playing). Neither is a runtime cost; both are the compositor's to fix
  (rasterise only the layers that changed; keep a layer's rasterised
  content between frames).
- **A linked Dioxus view is paced to 30 fps.** The linked `arrangement`
  is a component whose scene changes with its state: Blitz paces those
  redraws to `FTS_MAX_FPS` (30 unless set), so it draws at 40 fps and
  21 ms from input to frame where the same view as a process draws at
  91 fps and 8.5 ms.
- **Not shown above 120 Hz.** This display's top rate is 120 Hz. 240 Hz,
  and 5120 × 1440, are the owner's ultrawide's: `WIDTH=5120 HEIGHT=1440
  just frames` there, release profile by default.

## Tests to read

- `features/extensions/tests/remote_view.rs` (the test binary starts itself
  again as the extension): clicks go through the sub-document and the count
  comes back; the extension's action runs over vox; `kill -9` leaves the app
  running and the placeholder shows why; Start again brings it back;
  unloading shows a placeholder; a declared view the process does not draw,
  or a program that never says hello, fails to load; nonsense frames (unknown
  ids, cycles, a pop past the root, a child that is not there, raw HTML) are
  refused without a crash; a `<style>`'s raw CSS is taken as its text.
- The same file: the view's document starts from
  what a linked view inherits from its pane (`BASE_CSS`: no margin, the
  pane's height, the theme's `--ink` and `--font` at 13px), so a view that
  sets no colour is not black on a dark theme; a `<style>`'s
  `dangerous_inner_html` is its sheet, any other element's still fails the
  view; `RemoteView` is keyed by extension and view, so a pane that goes on
  to another process view opens that one.
- `features/extensions/tests/remote_input.rs`: a press holds the pointer
  over another element; a release lost outside the pane is a cancel, or a
  `pointerup` in a view with no cancel handler; a pointer that leaves with
  no button held lets go; focus leaving mid-composition ends it; the round
  trips stay inside `REMOTE_BUDGET`; 200
  moves are coalesced to the latest; a finger is a touch and a touch
  pointer; a wheel reaches the view and scrolls its list; an input method
  composes into a field, the window's IME on and its caret area in window
  coordinates. `features/pages/tests/js.rs`
  `a_tsx_page_gets_pointer_wheel_and_composition_events`.
- `features/extensions/tests/remote_embedded.rs`: a process view holds
  another extension's view, drawn by that extension's own process at the
  placeholder's place, taking its own clicks and redrawn with the
  holder's new input; a placeholder naming no view fails the holder.
- `features/extensions/web/test/rust-pane.test.mjs` (`just ext-pane-browser`):
  `web::Pane` in headless Firefox gets a `click` and a `contextmenu` sent
  as `PointerEvent`s and passes them on as mouse data
  (`apps/sandbox/pane-events`).
- `features/scene/tests/anim.rs`, `region.rs`, `engine.rs`: what an
  animation, a region or a drag evaluates to; when a host draws, rests
  and is woken; what crosses and what is refused; the engine's writer
  timed beside a host composing, with a counting allocator.
  `features/workspace/src/paint/anim_tests.rs`: one view linked in and
  across the wire composes the same at every clock and every frame of a
  drag, and fails the same way.
- `apps/sandbox/sketch/tests/sketch.rs`: a stroke drawn in the process is
  in the window's scene; panning and zooming send the transform, not the
  scene, and tell the view no new size; a graph node drags in a paint
  view served by its process; a scene is clipped to its pane unless the
  extension may draw over the app, where it lands at its window place;
  an oversized, unbalanced or unknown scene fails the view, not the app;
  a pan's round trip stays inside `REMOTE_BUDGET`; the timeline's playback,
  its fader, its drag box and its meter cost the process nothing a frame,
  and a seek wakes the window a resting view is in.
  `features/extensions/src/scene.rs` tests: a scene crosses once and then
  by its hash; both sides forget the same scenes, by count and by bytes;
  transforms round-trip; a stray pop, a scene too deep, too large or out
  of range is refused; framing clips unless overlay.

## Status

Done in architect on Unix desks; the sandbox runs the clicker. Not yet: a stdio
transport, starting lazily and stopping when idle, handing state over on a
reload, a host service extensions call back into, Windows, `mounted`
measurements, images (the sub-document has no net provider, so not even a
`data:` image loads), and an element and CSS allowlist (only raw HTML is
refused today, except a `<style>`'s CSS).

- **Task** and **Studio** do not use it yet.
