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
  drag-and-drop list works without knowing where the pointer went. A
  release outside the pane reaches the view as a `pointercancel` when the
  pointer comes back with no button held (Blitz holds the pointer for
  custom widgets only, not for a sub-document). A view that listens for
  `pointermove`, but not `mousemove` or `touchmove`, owns its held moves:
  the document doesn't select the text under a drag.
- **Fast events are coalesced.** While the view has not yet answered an
  event, the moves, wheels, scrolls and composition updates that follow
  wait; a run of them on one element keeps only the latest (a wheel's
  deltas are summed), and any other event sends what waits first, in
  order (`Coalescer`). A slow view gets the latest move, not a queue of
  stale ones.
- **The input method.** A field in the view turns the window's IME on
  when it takes the focus, with the caret area moved to where the pane is
  in the window, and its clipboard is the window's (`HostShell`).
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

The budget is 16 ms from key to echo in a local pane (a frame at 60 Hz);
`REMOTE_BUDGET` marks a view busy past 33 ms. 200 held moves sent before
the view answered any reach it as a handful
(`moves_while_the_view_is_busy_are_coalesced_to_the_latest`).

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
  the first time as `<hash>\n<json>` (anyrender's own commands), and after
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
| host: the scene decoded | 3.35 ms, once |
| host: the scene built at its transform (p50 / p99) | 0.47 / 0.56 ms |
| the window's scene recorded (p50 / p99) | 0.62 / 0.86 ms |
| a pan's round trip, pointer to new frame (p50 / p99) | 1.10 / 3.19 ms |

## Tests to read

- `features/extensions/tests/remote_view.rs` (the test binary starts itself
  again as the extension): clicks go through the sub-document and the count
  comes back; the extension's action runs over vox; `kill -9` leaves the app
  running and the placeholder shows why; Start again brings it back;
  unloading shows a placeholder; a declared view the process does not draw,
  or a program that never says hello, fails to load; nonsense frames (unknown
  ids, cycles, a pop past the root, a child that is not there, raw HTML) are
  refused without a crash.
- `features/extensions/tests/remote_input.rs`: a press holds the pointer
  over another element; a release lost outside the pane is a cancel; 200
  moves are coalesced to the latest; a finger is a touch and a touch
  pointer; a wheel reaches the view and scrolls its list; an input method
  composes into a field, the window's IME on and its caret area in window
  coordinates. `features/pages/tests/js.rs`
  `a_tsx_page_gets_pointer_wheel_and_composition_events`.
- `apps/sandbox/sketch/tests/sketch.rs`: a stroke drawn in the process is
  in the window's scene; panning and zooming send the transform, not the
  scene; a graph node drags in a paint view served by its process.
  `features/extensions/src/scene.rs` tests: a scene crosses once and then
  by its hash, both sides forget the same scenes, transforms round-trip.

## Status

Done in architect on Unix desks; the sandbox runs the clicker. Not yet: a stdio
transport, starting lazily and stopping when idle, handing state over on a
reload, a host service extensions call back into, Windows, `mounted` measurements, and an element and CSS allowlist (only raw
HTML is refused today).

- **Task** and **Studio** do not use it yet.
