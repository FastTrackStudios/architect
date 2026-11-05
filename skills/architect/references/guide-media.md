# Guide media: scenarios as films, tests and tours

One `.styx` scenario per thing a guide shows. `architect-guide`
(`features/guide`) plays it headlessly over the app's real UI: filmed into
a GIF / WebM / MP4 and a poster PNG, or as a test asserting every step;
and live in the running app as a guided tour. A story carries its own.
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
| `do` | `@run{id, args}` an Action List action; gestures on `target`: `@click{}` (`@click{secondary true}` right-clicks, `@click{mods (ctrl)}` Ctrl-clicks), `@drag{to, edge, dx, dy, seconds}`, `@scroll{dx, dy, seconds}` (a flick), `@type{text}`, `@key{key, mods}`, `@pause{seconds}` | `features/guide/src/model.rs` `Do` |
| `target` | `@id`, `@testid`, `@css`, `@tab`/`@pane`/`@group`/`@tabs`/`@split` (by a pane id), `@palette-row` (by command id), `@text` (an element by its whole text, the last in the page), `@app{kind, name}` (the workspace's `card` and `card-close`: a phone switcher's card and its ✕, by pane id), `@at{x, y}` | `features/guide/src/model.rs` `Target` |
| `done` | `on` (`@ran"ID"`, `@changed"topic"`, any of them) then every `check {id, args, has}`: a **read-only** action (an async one waited for by the headless runner) whose JSON result contains `has` | `features/guide/src/model.rs` `Done`, `Check`; `features/guide/src/lib.rs` `contains` |
| `hold`, `hide`, `text` | seconds kept after; left out of the film (setup; a tour runs its `@run` itself); the tour's copy | `features/guide/src/model.rs` `Step` |
| `safe` (scenario) | a tour may play it over the person's own data; off, only in the app's playground | `features/guide/src/model.rs` `Scenario` |
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
- A step with a `target` and no `do` points at it: done once the target
  is on screen (a tour's "this is the trigger").
- Write a scenario a tour can follow: a `done` on every step a person
  must do (else the tour waits for Next), `text` on every shown step,
  setup steps `hide true` and `@run` (a tour performs only those).
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
2. Put what checks read on that list: `WorkspaceView`'s `actions` prop
   (`architect_workspace::WorkspaceActions`: `WORKSPACE_LAYOUT` and every
   workspace action, `workspace` events), `architect_commands::use_command_actions`
   (`COMMANDS_RUN`, `COMMANDS_STATE`, `commands` events), and the app's own
   (`apps/lab/ui/src/guide.rs` `use_lab_actions`).
3. A media crate: `features/guide/src/run.rs` `App` (root, list, fonts,
   resolvers) and a `main` over `features/guide/src/cli.rs` `main`; a test
   over `features/guide/src/run.rs` `test_dir` (`apps/lab/media`).
4. An app's own targets: `App::resolver`, a `fn(kind, name) -> Option<String>`
   CSS selector.

## Tour it in the app (feature `ui`)

Public API (pre-1.0, new): `features/guide/src/tour.rs` `use_tours`,
`TourSet` (`new`, `from_sources`, `resolver`, `playground`, `on_move`),
`Moved`, `Tours` (`start`, `start_at`, `next`, `back`, `stop`, `playing`,
`offered`), `StartTour` (`id`, `step`), `TourLayer`, `TourState`, `hint`,
`place`; `features/guide/src/locate.rs` `find_live`,
`find_in`, `Resolver`.

1. `let tours = use_tours(list, TourSet::from_sources([include_str!(…)])?
   .resolver(architect_workspace::target_selector)
   .resolver(architect_commands::target_selector).playground(…))` at the
   root, beside the app's Action List.
2. Mount `TourLayer {}` once, the **last** child of the root, and never
   conditionally (Blitz: a layer inserted over panes collapses them).
3. Offer them: `GUIDE_TOUR_START {id}` is on the list (MCP, scenarios);
   give ⌘K a row per tour with `architect_commands::use_command(…, move ||
   { let _ = tours.start(&id); })` (the lab: `apps/lab/ui/src/lib.rs`
   `LabApp`, `apps/lab/ui/src/guide.rs` `tours`).
4. Safety: a scenario without `safe true` starts only through
   `TourSet::playground` (set up sample data, or refuse with a reason).
5. A tour of chapters (`apps/lab/scenarios/architect-tour.styx`): each
   chapter is hidden `@run` setup steps then one shown step with `text`
   and a `target`. Back and `GUIDE_TOUR_START {id, step}` rerun the setup
   right before a step, so each chapter's setup must stand alone (go to
   the page, load the layout, close what the last chapter opened:
   `COMMANDS_SHOW {overlay none}`, `EXT_WINDOW_CLOSE`). Remember the
   place with `TourSet::on_move` and resume with `Tours::start_at`
   (`apps/lab/ui/src/guide.rs` `tours`). Test it live with a generated
   walk (`apps/lab/media/tests/tour.rs`).
6. Films: `FILM_CAPTION`, `FILM_TERMINAL` and `FILM_WRITE` (the lab's
   media crate, `apps/lab/media/src/lib.rs`) caption a GIF, show CLI
   lines, and save a file in the run's fresh lab folder (theme files,
   pages, dev copies reload live headless: `LabFiles`). A page draws in
   a document of its own, so `@text` cannot find its text: wait with
   `@pause`, or check `PAGES_STATUS`. An async read-only action (an
   extension's, `EXT_*`) is waited for when `check`ed, natively
   (`features/guide/src/predicate.rs` `checks_waiting`, up to 5 s); in a tour wait
   on a target instead (a step with a `target`, no `do`, and
   `done {timeout …}`).

| Action | What |
|---|---|
| `GUIDE_TOUR_START {id}` | start a tour (fails: no such tour, or unsafe with no playground) |
| `GUIDE_TOUR_STATE` | `{scenario, step, index, of, waiting, found}` (read-only; `scenario` empty when none plays) |
| `GUIDE_TOUR_LIST` | the tours on offer `{id, title, safe}` |
| `GUIDE_TOUR_NEXT` / `_BACK` / `_SKIP` | move it from outside; each move is a `guide` change event |

To film or test a tour, write a scenario that starts it and follows it,
checking `GUIDE_TOUR_STATE` after each step:
`apps/lab/scenarios/tour-palette-run.styx`.

## A story's scenarios (feature `story`)

- `#[story(category = "…", scenario = "x.styx")]`, once per file,
  relative to the story's source file: `Story::scenarios`
  (`features/story/architect-story-core/src/lib.rs` `Story`).
- `architect-story-shell` feature `tours`: the Lookbook offers them as
  tours, a button per scenario on its story.
- `features/guide/src/story.rs` `test_stories` plays them as tests over
  the story alone (`app`); `features/guide/src/cli.rs` `main_stories` is
  the media command over them. Example:
  `features/ui/architect-ui/src/components/collapsible.styx`, tested by
  `features/ui/architect-ui/tests/scenarios.rs`.
- A story has no checks of its own; point steps at what must be on
  screen (a target-only step) instead.

## Run it

- `just media [ids]` films the lab's scenarios; `just media-test` plays
  them as tests; `cargo test --workspace -- every_lab_scenario` is the test
  in CI (`apps/lab/media/tests/scenarios.rs`).
- ffmpeg and gifski come from the dev shell (`devenv.nix`).
- Media never go on `main`: `just media-publish` commits them to the orphan
  `media` branch; docs embed `raw.githubusercontent.com/FastTrackStudios/architect/media/<id>.gif`.
