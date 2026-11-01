# Guide media: scenarios as films, tests and tours

One `.styx` scenario per thing a guide shows. `architect-guide`
(`features/guide`) plays it headlessly over the app's real UI: filmed into
a GIF / WebM / MP4 and a poster PNG, or as a test asserting every step.
The design and status: `docs/content/architecture/guide-media.md`.

## Write a scenario

```styx
id tab-reorder
title "Reorder tabs by dragging"
view {width 1600, height 900}
capture {fps 20, crop @tabs"note-welcome", pad 40, width 960}
steps (
    {id vault, do @run{id LAB_GO_VAULT}, hide true}
    {
        id drag
        text "Drag a tab along the bar."
        target @tab"note-welcome"
        do @drag{to @tab"arrange", edge right, seconds 1.0}
        done {
            on (@changed"workspace")
            check ({id WORKSPACE_LAYOUT, has {windows ({groups ({tabs (note-shell arrange note-welcome)} {tabs (graph)})})}})
        }
        hold 1.0
    }
)
```

| Field | What it is | Code |
|---|---|---|
| `do` | `@run{id, args}` an Action List action; gestures on `target`: `@click{}`, `@drag{to, edge, dx, dy, seconds}`, `@scroll{dx, dy, seconds}` (a flick), `@type{text}`, `@key{key, mods}`, `@pause{seconds}` | `features/guide/src/model.rs` `Do` |
| `target` | `@id`, `@testid`, `@css`, `@tab`/`@pane`/`@group`/`@tabs`/`@split` (by a pane id), `@palette-row` (by command id), `@text` (an element by its whole text, the last in the page), `@app{kind, name}`, `@at{x, y}` | `features/guide/src/model.rs` `Target` |
| `done` | `on` (`@ran"ID"`, `@changed"topic"`, any of them) then every `check {id, args, has}`: a **read-only** action whose JSON result contains `has` | `features/guide/src/model.rs` `Done`, `Check`; `features/guide/src/lib.rs` `contains` |
| `hold`, `hide`, `text` | seconds kept after; left out of the film (setup); the tour's copy | `features/guide/src/model.rs` `Step` |
| `capture` | `fps`, `crop` (a target), `pad`, `width`, `outputs (gif webm mp4 png)`, `cursor`, `desktop {width, height}` (every window on one canvas, with title bars) | `features/guide/src/model.rs` `Capture` |

Rules:

- A predicate is an action. Need to check something? Put a read-only
  action on the app's list that answers it; don't add a query language.
- A tag ending in `r` cannot take a quoted payload (styx reads `r"` as a
  raw string): hence `@tabs"…"` for a tab bar.
- Headless there is no window: window chords (`WindowKeys`) don't fire.
  Run the command (`@run{id …}`); a view's own command works too, through
  `COMMANDS_RUN`.
- Prefer state waits (`done`) to `@pause`.
- Motion is filmed: the runner drives `architect_shell::motion`'s
  `VirtualClock` (`features/shell/src/motion.rs`), so an app animation on
  `Motion` shows its in-between frames; animate with it, not with a
  Dioxus task's timer (wall clock, not filmed). `@scroll` is a trackpad
  flick (fast, then coasting), signed as the platform sends it: a
  negative `dx` brings what is to the right into view. Give a step a `hold` long enough for its
  motion to settle (about 0.5 s).

## Make an app playable

1. Give its root the Action List the runner puts in context
   (`Arc<ActionList>`), and leave out what needs a window when there is
   none (the lab: `apps/lab/ui/src/lib.rs` `LabApp`).
2. Put what checks read on that list: `architect_workspace::use_workspace_actions`
   (`WORKSPACE_LAYOUT`, `workspace` events), `architect_commands::use_command_actions`
   (`COMMANDS_RUN`, `COMMANDS_STATE`, `commands` events), and the app's own
   (`apps/lab/ui/src/guide.rs` `use_lab_actions`).
3. A media crate: `features/guide/src/run.rs` `App` (root, list, fonts,
   resolvers) and a `main` over `features/guide/src/cli.rs` `main`; a test
   over `features/guide/src/run.rs` `test_dir` (`apps/lab/media`).
4. An app's own targets: `App::resolver`, a `fn(kind, name) -> Option<String>`
   CSS selector.

## Run it

- `just media [ids]` films the lab's scenarios; `just media-test` plays
  them as tests; `cargo test --workspace -- every_lab_scenario` is the test
  in CI (`apps/lab/media/tests/scenarios.rs`).
- ffmpeg and gifski come from the dev shell (`flake.nix`).
- Media never go on `main`: `just media-publish` commits them to the orphan
  `media` branch; docs embed `raw.githubusercontent.com/FastTrackStudios/architect/media/<id>.gif`.
