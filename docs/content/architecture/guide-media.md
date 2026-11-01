+++
title = "Guide media"
description = "One scenario file, three outputs: a headless film (GIF, WebM, MP4, poster), a test, and (next) an in-app tour."
weight = 66
+++

A guide needs pictures that move, they have to come from the real UI,
and they have to stay right when the UI changes. `architect-guide`
(`features/guide`) gets all three from one file, a **scenario**: a list of
steps over an app. The same steps are

- **filmed**, headlessly on Blitz with a virtual clock, a drawn pointer and
  every window on one desktop, into a GIF, WebM, MP4 and a poster PNG;
- **played as a test** with no frames, every step's `done` asserted;
- **toured** in the app, its target ringed and its text shown, moving on
  when `done` holds (the overlay is next; see Status).

The design generalises Studio's proposal (`studio:docs/design/guide-media.md`,
studio#330) to any architect app: no DAW and no `.session`. The "app" is
any Dioxus root plus its `ActionList`.

## A scenario

Styx, read into facet types (`features/guide/src/model.rs` `Scenario`) by
`features/guide/src/lib.rs` `load`. Styx is the fleet's config idiom and
`facet-styx` its loader, already in the workspace: no TOML.

```styx
id palette-run
title "Run any command from ⌘K"
view {width 1280, height 800}
capture {fps 20, width 960}
steps (
    {
        id open
        text "⌘K opens the palette: every command the app has."
        do @run{id LAB_SHOW_COMMANDS}
        done {
            on (@changed"commands")
            check ({id COMMANDS_STATE, has {overlay palette}})
        }
        hold 0.6
    }
    {
        id run
        target @palette-row"LAB_GO_SCHEDULE"
        do @click{}
        done {on (@changed"lab"), check ({id LAB_STATE, has {tab schedule}})}
    }
)
```

A step (`features/guide/src/model.rs` `Step`) is:

- **`do`** (`Do`): an Action List action by id with arguments
  (`@run{id …, args {…}}`), or a gesture on the step's `target`:
  `@click{}`, `@drag{to <target>, edge right}` or `@drag{dx 120}`,
  `@scroll{dx 400}`, `@type{text "…"}`, `@key{key Enter, mods (meta)}`,
  `@pause{seconds 0.5}`.
- **`target`** (`Target`): a query resolved to a rectangle at the frame it
  is needed, so a layout change moves it. `@id"…"`, `@testid"…"`,
  `@css"…"`; the workspace's `@tab"<pane>"`, `@pane"<pane>"`,
  `@group"<pane>"` (the group holding it), `@tabs"<pane>"` (its tab bar),
  `@split"<pane>"` (the divider after it); the palette's
  `@palette-row"<command>"`; an element by its whole text, `@text"…"`
  (the last in the page, so a menu or popover over it wins: a menu's
  title or row, a settings section, an org in the switcher); an app's own
  `@app{kind …, name …}`; a point `@at{x, y}`.
- **`done`** (`Done`): Action List events to wake on (`@ran"ID"`,
  `@changed"topic"`) and **checks**: a read-only action and the subset of
  its JSON result it must contain (`features/guide/src/lib.rs` `contains`:
  objects by key, arrays by position and length). There is no second query
  language: a predicate is an action. Unset, the step is done when its
  `do` is.
- **capture directives**: `text` (the tour's copy), `hold` (seconds kept
  after), `hide` (setup, left out of the film); per scenario `capture`
  sets `fps`, `crop` (a target), `pad`, `width`, `outputs`
  (`gif webm mp4 png`), `cursor`, and `desktop` (below).

Styx reads `r"` as a raw string's start, so a tag ending in `r` cannot
take a quoted payload: that is why the tab bar's target is `@tabs"…"`.

## The runner

`features/guide/src/run.rs` `run` plays a scenario over an `App` (its root,
its `Arc<ActionList>`, its fonts, its resolvers). It opens a
`DioxusDocument` of the root, as `architect-story-snapshots` and
`ui-snapshot` do, with the Action List, the window's size (`ViewSize`) and
the desktop's windows (`architect_workspace::Windows`) in root context.

- **Actions** run on the list. An id the list does not have goes through
  `COMMANDS_RUN` (`features/commands/src/bridge.rs` `use_command_actions`),
  which runs any command on the window's thread, a view's own included.
- **Gestures** become `UiEvent`s through each document's own
  `handle_ui_event`, the path a window's shell drives: the pointer travels
  eased over frames, presses, moves, lets go. A drag goes through the
  app's real drag path, so its drop zones and its animations play.
- **Time is virtual**: frame *n* is `n / fps` seconds. Stylo's
  animations and transitions are resolved at it, and the app's motions
  (`architect_shell::motion`, [Motion](@/architecture/motion.md)) run on
  it: the runner moves a `VirtualClock` to each frame's time, so a film
  shows every in-between frame and is the same each run. A Dioxus task's
  own timers run on the wall clock; each frame waits until the documents
  have been idle for a few milliseconds before it is taken
  (`Runner::quiesce`), and `done` waits on state, never on time (while a
  motion runs, the wait is frames).
- **Every window**: the runner is the app's `WindowSystem`
  (`features/workspace/src/host/windows.rs`). A tab or a dock dragged out
  opens a pop-out, a second document on the desktop at the place the
  workspace chose. The pointer is held by the window it pressed in; let go
  outside it, that window's host hears where (`on_release_outside`), as the
  desk launcher (`features/workspace/src/native.rs`) does.

`features/guide/src/run.rs` `standard` is the resolvers every app has:
CSS and `data-testid`, `architect_workspace::target_selector`
(`features/workspace/src/guide.rs`, over the host's `data-tab`,
`data-pane`, `data-group-tabs`, `data-tab-bar` and `data-split-after`
attributes) and `architect_commands::target_selector` (`data-command` on
a palette row). An app adds its own with `App::resolver`.

What a check reads, an app puts on its list: `WORKSPACE_LAYOUT`
(`features/workspace/src/host/actions.rs` `use_workspace_actions`: every
window's groups, tabs, splits and docks, and a `workspace` change event
on each change), `COMMANDS_STATE` (the overlay on show and the palette's
field), and the app's own (the lab's `LAB_STATE`,
`apps/lab/ui/src/guide.rs`).

## The film

`features/guide/src/capture.rs` `Recorder` is the run's `Film`. Each
frame kept, every window is painted on the CPU rasteriser (no GPU), put on
one canvas with a title bar when `capture.desktop` is set (else the main
window edge to edge), with the pointer and a press's ripple drawn over it
(the film's own layer: the app never sees it), cropped and scaled, and
written as a PNG. `encode` makes the media: gifski for the GIF, ffmpeg
for WebM (VP9) and MP4 (H.264), the last frame as the poster. Both are
run as programs; the dev shell has them (`flake.nix`). A still is a
scenario of one step.

An app's media binary is three lines over `features/guide/src/cli.rs`
`main`. The lab's is `apps/lab/media` (`lab-media`), its scenarios in
`apps/lab/scenarios`:

```sh
just media                     # every scenario, into target/media
just media showcase-layout     # one
just media-test                # every scenario as a test, no frames
```

## The test

The same `run` with no film: `features/guide/src/run.rs` `test_dir` plays
every scenario in a directory and reports each failure with its step and
what its check's action gave back:

```text
tab-reorder: step "drag": check WORKSPACE_LAYOUT wants {"windows":[…]} (in 5s)
  it gave: {"active":"note-welcome","windows":[…]}
```

`apps/lab/media/tests/scenarios.rs` runs the lab's under `cargo test`.

## Where the media live

Not in git on `main`. An orphan `media` branch holds only the generated
files (`<id>.gif`, `.webm`, `.mp4`, `.png`); the README and docs embed
`https://raw.githubusercontent.com/FastTrackStudios/architect/media/<id>.gif`,
the poster PNG as the still. `just media-publish` regenerates every
scenario and commits the outputs to that branch in a temporary worktree,
never touching the checkout it is run from; `just media-publish 1` pushes
it too.

## The lab's scenarios

| Scenario | Shows |
|---|---|
| `apps/lab/scenarios/palette-run.styx` | ⌘K opened, a command found by typing, picked |
| `apps/lab/scenarios/tab-reorder.styx` | a tab dragged along its bar |
| `apps/lab/scenarios/stacked-tabs.styx` | a group stacked, its strip scrolled |
| `apps/lab/scenarios/showcase-layout.styx` | the README's showcase: notes opened, a split by dragging a tab to a group's side, a vertical one, two dividers pulled, a group stacked and filled, the sidebar dragged out by its grip into a window of its own |

The rest are the guide's pictures, one per thing a page shows, each named
for the picture it makes (`workspace-split-groups`, `menu-bar-in-window`,
`settings-search-to-row`, …): the docs embed them by that id. A Vault
scenario closes the Clicker pane first, in hidden steps: it is a process
extension's view, and headless its program is not running.

## Status

Built: the model and its loader (wasm-clean), the runner with multiple
windows, the film and its encoders, the CLI and recipes, the test mode,
the workspace's and the commands' resolvers and actions, the text target,
the lab's scenarios.

Next:

- **The in-app tour**: an overlay in `architect-shell` (root-absolute: no
  `position: fixed` on Blitz) that rings the resolved target, shows the
  step's `text`, and moves on when `done` holds, watching the Action List
  in process (`ActionList::events_since`). Resolving a target in a live
  window needs the document's rects, which a component cannot reach today.
  A scenario's `safe` already says whether a tour may run it over a
  person's own data.
- **A story's scenarios**: a story carrying scenarios, played in the story
  shell. The runner already takes any root.
- **Keys through the window**: a headless document has no window, so
  window-level chords (`WindowKeys`) do not fire; a scenario runs the
  command (`@run{id LAB_SHOW_COMMANDS}`) where a person presses ⌘K.
