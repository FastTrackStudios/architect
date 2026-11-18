# The setlist window: stage (b), placeholder arrangement

**This is not the final picture.** The arrangement and mixer panes here are
studio#519's first drawing (`feat/arrangement-mixer-runtime-extensions`
`25984ea8`), which the owner rejected as a re-implementation. Studio's real
views (`apps/session/ui`, as `studio-desktop` mounts them) replace them in
stage (c), so every file here is named `…-placeholder-arrangement…`.

One stock Architect window at 5120×1440, both sidebars, the status bar and
the Now Playing bar hidden, playing **Drive To LA** from JT's Senior Recital:

| left | centre | right |
|---|---|---|
| Ignition's 3D room (`ignition/room`, a `kind = "process"` view, Ignition#19 `6ee81451`) | Studio's arrangement (`studio.arrangement/arrangement`) over its mixer (`studio.mixer/mixer`), the `daw` engine playing the song's multitracks into a private PipeWire null sink | Keyflow's chart text (`vault.note`) beside the chart (`keyflow/chart`, `feat/runtime-chart` `ce7dbd7`) |

Nothing is compiled into the app: all five (`daw`, `studio.arrangement`,
`studio.mixer`, `keyflow`, `ignition`) are development copies of runtime
extensions in one isolated instance. Architect: `runtime-compositor-setlist`
`f9f6b57a` (`runtime-compositor` plus `runtime-compositor-arrangement` and
`runtime-compositor-process-macos`, and this work's fixes).

## Files

- `setlist-window-5120x1440-dark-playing-placeholder-arrangement.png`: dark, playing (engine at ~80 s).
- `setlist-window-5120x1440-light-playing-placeholder-arrangement.png`: light, playing (~92 s).
- `setlist-window-5120x1440-dark-playing-28s-placeholder-arrangement.mp4`: 28 s of playback, from 57 s.

All captured with `loft-shot` / `wf-recorder` from a headless GPU Loft
session (RTX 4080) whose one output is exactly 5120×1440 at 60 Hz. `APP_SHOT`
was not used (it repaints on the CPU and draws a process view black).

## Which pane follows which clock

Not one transport yet.

- **The engine's clock** (the app's playhead deck, which the `daw` engine leads): the arrangement's playhead and the mixer's meters.
- **Keyflow's session clock** (presence `lead/play` of a session Keyflow hosts and serves on `ws://127.0.0.1:4143/vox`): the chart (pane `session:<id>`) and the room (`--clock session:…@drive-to-la`).
- The launcher starts both at the same moment (`--play 60`). They agree to within the two starts, then run apart.

The engine does not publish the session's transport (studio#522). Keyflow's
session chart draws no playhead (keyflow#97, which also covers: no text
editor beside a session chart, and the engine's session file not opening in
Keyflow).

## Frame time, the whole window, playing

`APP_FRAMES` (new, `features/workspace/src/frames.rs`): each redraw of the
main window timed on its thread, 20 s while playing, all five panes on screen,
fullscreen 5120×1440:

| | p50 | p99 | max |
|---|---|---|---|
| from one frame to the next (what reached the screen) | 16.7 ms | 35.6 ms | 42.9 ms |
| each frame's work on the window's thread (includes waiting on the 60 Hz present) | 16.2 ms | 32.9 ms | 39.0 ms |

- **56.5 fps presented.**
- The window's thread used **8.3 ms of CPU a frame** (44.7% busy at 53.6 fps over 10 s, from `/proc`). That's the honest work figure; the timed redraw includes the vsync wait.
- Of that CPU (perf):
  - Blitz painting the DOM and the sub-documents ~29%;
  - composing the extensions' scenes ~19% (`architect_scene::layers::replay` copies every path into a new recording each frame: `memmove` alone 18%, #834);
  - Vello's encoding (`render_to_texture`) ~20%.
- The machine is shared (load 10–30 while measuring).

**What capped it:** the 60 Hz output first. Below that, the window's own thread: about 8 ms of CPU a frame is ~120 fps at best on this machine, half the 240 fps goal, before the GPU's raster, which is not measured.

**Unproven:**
- **240 fps on the owner's ultrawide**: no high-refresh output here, and the CPU per frame must halve first.
- **Ignition's room**: its frames come over shared memory, not dma-buf (Ignition's Bevy renderer on wgpu 29 speaks the wire itself over shared memory, `docs/content/architecture/process-views.md`). That's one copy each way at 5K.

Found and fixed on the way: the dock menu's resolve serialised every action's JSON Schema after each action run, about 10% of the window's thread while playing. Fixed by caching each type's schema in `85af8196`. The gap's p99 went from 68.6 to 35.6 ms.

## Run it

```sh
SETLIST_SESSION=<drive-to-la.session> SETLIST_STUDIO=… SETLIST_KEYFLOW=… SETLIST_IGNITION=… \
  just setlist-window --play 60
```

(`scripts/setlist-window.sh`: builds each program in its repo's dev shell, assembles the five packages, opens `examples/layouts/setlist-window.json`, chooses the engine for `fts.session`, makes Keyflow's session from the chart and points the chart and the room at it.)

Gaps and fixes: architect#840 (the comment of 2026-11-18), #851, studio#522, keyflow#97.
