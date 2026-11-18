# Stage (c), two clocks: superseded by the README one folder up.

# The setlist window at 5120×1440

One stock Architect window, 5120×1440, both sidebars, the status bar and the
Now Playing bar hidden, playing **Drive To LA** from JT's Senior Recital:

| left | centre | right |
|---|---|---|
| Ignition's 3D room (`ignition/room`, a `kind = "process"` view; Ignition `main` `8f04c0a`) | **Studio's own** arrangement over **Studio's own** mixer (`studio.arrangement/arrangement`, `studio.mixer/mixer`: `session_ui`'s panels, the ones `studio-desktop` mounts), on **Studio's engine** (the `daw` extension runs `studio engine`) playing the song's multitracks into a private PipeWire null sink; studio#519 `fffcf831` | Keyflow's chart text (`vault.note`) beside the chart (`keyflow/chart` on a session; `feat/runtime-chart` `ce7dbd7`) |

Nothing is compiled into the app: `daw`, `studio.arrangement`, `studio.mixer`,
`keyflow` and `ignition` are development copies of runtime extensions in one
isolated instance of `architect-desktop --features process-views`, Architect
`runtime-compositor-setlist` `d46354fc`.

## Files

- `setlist-window-5120x1440-dark-playing.png`: dark, playing (the engine at 72 s).
- `setlist-window-5120x1440-light-playing.png`: light, playing (78 s). Studio's panes stay dark as `studio-desktop` itself does under a light desktop (studio `pr-media/arrangement-mixer`, `reference-studio-desktop/studio-desktop-1280-light-desktop-stays-dark.png`).
- `setlist-window-5120x1440-dark-playing-28s.mp4`: 28 s of playback from 57 s.
- `stage-b-placeholder/`: the earlier stage with studio#519's rejected placeholder drawing, kept for comparison only.

All captured from a headless GPU Loft session (RTX 4080) whose one output is
exactly 5120×1440 at 60 Hz, with `loft-shot` and `wf-recorder`. `APP_SHOT`
was not used: it repaints on the CPU and draws a process view black.

## Which pane follows which clock

- **Studio's engine:**
  - the arrangement's playhead and the mixer's meters (Studio's own window protocol, attached to the engine);
  - **the room**: Ignition's `studio` clock, the engine's socket (`$XDG_RUNTIME_DIR/fts/engine-<pid>.sock`, log "following the engine … a song of the night is open song=Drive To LA").
  - Seeking the engine alone moves the room: the intro's amber wash at 5 s, magenta at 120 s.
- **Its own clock: the chart.** It is on a session Keyflow hosts and serves (`KEYFLOW_SESSION_PLAY`), started at the same moment as the engine (`just setlist-window --play`). Keyflow cannot follow Studio's engine, and the engine publishes no session transport (studio#522). The chart also draws no playhead and stays on page 1 while playing (keyflow#97).

## Frame time, the whole window, playing

`APP_FRAMES` (`features/workspace/src/frames.rs`), 20 s, all five panes on
screen, fullscreen 5120×1440, the window's thread CPU from `/proc`:

| | p50 | p99 | max |
|---|---|---|---|
| frame to frame (what reached the screen) | 16.7 ms | 37.6 ms | 63.4 ms |
| each redraw on the window's thread (includes waiting on the 60 Hz present) | 14.2 ms | 32.5 ms | 55.4 ms |

- **56.4 fps presented.**
- **10.0 ms of CPU a frame** on the window's thread (56.6% busy).
- The machine was heavily shared while measuring (load ~43; another session's compositor at ~20 cores), so the p99 and max are noisy.
- With the placeholder drawing, the same measurement gave 56.5 fps and 8.3 ms of CPU a frame. Of that CPU (perf, placeholder run):
  - Blitz painting the page and its sub-documents ~29%;
  - re-composing the extensions' scenes every frame ~19% (`architect_scene::layers::replay` copies each path into a fresh recording, #834);
  - Vello's encoding ~20%.

**What capped it:** first, the 60 Hz output (the Loft headless output offers
only 60 Hz). Below that, the window's own thread: 10 ms of CPU a frame is
about 100 fps at best here.

**Unproven:**
- **240 fps on the ultrawide.** It needs a high-refresh output to show, and the window's CPU per frame cut to under ~4 ms first.
- The GPU's raster time is not separated.
- Ignition's room arrives over shared memory, not dma-buf (one copy each way at 5K).

## Run it

```sh
SETLIST_SESSION="<…>/Drive To LA/Drive To LA.rpp" \
SETLIST_STUDIO=<studio at studio#519> SETLIST_KEYFLOW=<keyflow> SETLIST_IGNITION=<Ignition> \
  just setlist-window --play 60
```

`scripts/setlist-window.sh`:
- builds each program in its own repo's dev shell;
- assembles the five packages;
- opens `examples/layouts/setlist-window.json`;
- makes Keyflow's session from the chart;
- points the chart at it and the room at the engine.

A window manager that opens the window smaller needs it fullscreen; a resize no longer crashes. The song folder's name is what Ignition matches to the night's song.

Gaps and fixes: architect#840 (the comments of 2026-11-18), #851, studio#522, keyflow#97.
