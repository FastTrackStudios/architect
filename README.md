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
| played from 0 | 5.96 s | 6.30 s |
| seek 66 s (Interlude) | 69.06 s | 69.29 s |
| seek 93 s (Pre-Chorus 2) | 96.17 s | 96.67 s |
| light theme | 101.68 s | 101.93 s |

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

- **Architect:** `runtime-compositor-setlist` `a2f129c9`.
  - It is `main` `4d537fb8` plus my unmerged `dev --layout <file>.json` and `--hide now-playing`, the `now_playing` slot (the layout file hides the Now Playing bar), the opt-in process-view stats line, and the schema cache, all to come as PRs.
  - The provider choice (#872), a provider using its own service (#873) and images in a runtime view (#874) are on `main`.
- **Studio:** `feat/arrangement-mixer-runtime-extensions` `b434c7de`.
- **Keyflow:** **keyflow#104's branch** `feat/uses-fts-session` `5e067131` (`[uses] "fts.session"`), not yet in Keyflow `main`. It sits on `82c6b97`, so it lacks `main`'s #102 (note input's bars, the count-in) and #103 (a session as a vault document).
- **Ignition:** `main` `a540496`.
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

## Frame time

(filled in from the recording run)
