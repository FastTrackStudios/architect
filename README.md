# The setlist window at 5120×1440: one session, one transport

One stock Architect window, 5120×1440, both sidebars, the status bar and the
Now Playing bar hidden, playing **Drive To LA** from JT's Senior Recital, in
the owner's layout:

| left | centre | right |
|---|---|---|
| Ignition's 3D room (`ignition/room`, a process view), the cue list (`ignition/cues`) a tab behind it | Studio's own arrangement (`studio.arrangement/arrangement`), the mixer (`studio.mixer/mixer`) under the arrangement only | Keyflow's chart pane (`keyflow/chart`) with its palettes sidebar and icon toolbar; no text pane |

Every pane is a development copy of a runtime extension: `daw`, `studio.arrangement`, `studio.mixer`, `keyflow` and `ignition`. Nothing is compiled into the app.

## One transport

Studio's engine (the `daw` extension, `FTS_DAW_OPEN` the song's `.rpp`,
`FTS_DAW_BAR_ONE=5`) opens the song's session and leads its transport. The
arrangement, the mixer, the chart, the room and the cue list all follow that one
transport, by the handshake agreed on studio#522:

1. `EXT_SERVICE_USE {"service":"fts.session","provider":"daw"}`. Both `daw` and `keyflow` provide `fts.session`; `EXT_SERVICES` then says `in_use: daw`.
2. `KEYFLOW_SESSION_FOLLOW {url, id, chart_path}`.
3. The chart pane on `session:<id>`.
4. `IGNITION_SESSION_SONG` and `IGNITION_SESSION_ROOM`.

There is no second clock: play and seek go to the engine alone (`SESSION_PLAY_FROM`, `SESSION_SEEK` through `DAW_RUN`).

The engine's position and the chart's shown time, read one after the other
(each read is a separate CLI call, so a few tenths apart):

| moment | engine | chart |
|---|---|---|
| played from 0 | 6.02 s | 6.05 s |
| seek 66 s (Interlude) | 69.02 s | 69.06 s |
| seek 93 s (Pre-Chorus 2) | 96.01 s | 96.04 s |
| light theme | 100.00 s | 100.02 s |

Read on screen at 93 s (an earlier run of the same build), the arrangement's playhead is at the start of project bar 63, and the chart's playhead is at the first bar of PRE-CH 2. That is the same bar: the song's bar 1 is project bar 5. The room changes look with each seek (amber intro wash at 5 s, magenta at 120 s), following the same session.

## Files

- `setlist-window-5120x1440-dark-play-from-0.png`: playing, 6 s in.
- `setlist-window-5120x1440-dark-seek-66s-interlude.png`: after the seek to 66 s.
- `setlist-window-5120x1440-dark-seek-93s-prechorus-2.png`: after the seek to 93 s.
- `setlist-window-5120x1440-light-playing.png`: light theme, playing. Studio's panes stay dark, as `studio-desktop` itself does under a light desktop.
- `setlist-window-5120x1440-dark-play-seek-seek-31s.mp4`: play from 0, seek to 66 s at about 10 s, seek to 93 s at about 20 s.
- `stage-c-two-clocks/` and `stage-b-placeholder/`: the earlier stages, superseded.

All captured with `loft-shot` / `wf-recorder` from a headless GPU Loft session (RTX 4080), one output at exactly 5120×1440, 60 Hz. The shell's notifications were muted and the pointer parked off the window.

## Builds

Every repo at its `main`, except Architect's unmerged bits:
- **Architect:** `runtime-compositor-setlist` `f768a769`. It is `main` `6f1fadbb` (past v0.14.0's `9bbad508`) plus my unmerged `dev --layout <file>.json`, `--hide now-playing` and the `now_playing` slot (the layout file uses both), the opt-in process-view stats line, the schema cache and the launcher and layout preset (all coming as PRs). The provider choice (#872), a provider using its own service (#873) and images in a runtime view (#874) are on `main`.
- **Keyflow:** `main` `001c54be` (#102–#111, its `[uses] "fts.session"` included); the package is its folder with `icons/`, no manifest patching.
- **Ignition:** `main` `628e70ad` (#24).
- **Studio:** `main` `8fe8a836` (studio#519 merged): the arrangement, the mixer (its meters the engine's values, `ui = ["engine"]`) and the `daw` engine.
- **Run:** `just setlist-window --play 0` (`scripts/setlist-window.sh`, `examples/layouts/setlist-window.json`).

## The chart column against Keyflow's own window

Compared with Keyflow's capture `pr-media/chart-selection-and-input-modes:captures/kf1280-dark-syntax-preview.png`.

**The same:**
- the pane's icon toolbar (input modes, note values, dot, rest, accidentals, tie, articulations, the palettes toggle);
- the Palettes sidebar (Sections, Key signatures, Time signatures, Tempo with its note glyphs, Chord qualities, Articulations, Repeats & barlines with segno and coda, Dynamics);
- the chart in the app's theme.

**What still differs:**
- Keyflow's capture is in syntax mode, so its foot has the "Keyflow syntax" input row; this pane is in normal mode.
- Keyflow's capture shows its window's chrome (title bar, a Welcome tab); the owner's layout hides the sidebars and bars.
- The chart scrolls to follow the playhead (#100), so the page on screen moves with the song.
- On Keyflow `main` (#109) the Tempo palette's note glyphs draw dark on the dark sheet, hard to see; the other glyph cells and the tinted toolbar icons read well. They were light on #104's branch.

## Frame time

`APP_FRAMES` (#863), all five panes playing, fullscreen 5120×1440, THEBATTLESHIP at load ~17 (still shared):

| | window | gap p50 / p99 / max | each redraw's work p50 / p99 |
|---|---|---|---|
| playing, no recorder (15 s) | **42.1 fps** | 18.5 / 44.5 / 66.0 ms | 18.0 / 42.2 ms |
| during the recording (32 s) | 40.9 fps | 22.3 / 49.7 ms | |

- The recording holds 928 frames in 31.9 s; that is the recorder's own capture, also shared.
- The Loft output is 60 Hz, so nothing above 60 fps can show here, and a redraw's work includes waiting on that present.
- Earlier runs at heavier load: 21–31 fps (load 44–66), and 56.4 fps with Studio's earlier drawing at load ~43 (`stage-c-two-clocks/`).
- 240 fps on the owner's ultrawide is not shown: it needs a high-refresh display, an idle machine and the window's work a frame under ~4 ms.

The clocks during the recording, engine then chart:

| step | engine | chart |
|---|---|---|
| played from 0 | 1.03 s | 1.07 s |
| seek 66 s | 67.03 s | 67.07 s |
| seek 93 s | 94.03 s | 94.06 s |

## The mixer's width

The mixer's strips fill about the left half of its pane. That is Studio's own mixer: `studio:apps/session/ui/src/mixer_panel.rs` draws fixed 86 px strips (`studio:apps/session/ui/src/mcp.rs` `STRIP_W`) at zoom 1.0 on a desktop (`studio:apps/session/ui/src/touch.rs` `mixer_zoom`). Only Studio's record view sets `fill`, which zooms strips to the width. `studio-desktop` mounts the same panel, and Studio's parity test finds 0 pixels different between the two, so it leaves the same space at this width. This is read from the code; `studio-desktop` was not run at 2,304 px.
