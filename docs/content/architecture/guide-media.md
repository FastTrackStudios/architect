+++
title = "Guide media"
description = "One scenario file, three outputs: a headless film (GIF, WebM, MP4, poster), a test, and an in-app tour; a story carries its own."
weight = 66
+++

A guide needs pictures that move, they have to come from the real UI,
and they have to stay right when the UI changes. `architect-guide`
(`features/guide`) gets all three from one file, a **scenario**: a list of
steps over an app. The same steps are

- **filmed**, headlessly on Blitz with a virtual clock, a drawn pointer and
  every window on one desktop, into a GIF, WebM, MP4 and a poster PNG;
- **played as a test** with no frames, every step's `done` asserted;
- **toured** live in the app: the window dimmed around its target, its
  text in a popover, moving on when `done` holds.

A story (`architect-story`) carries scenarios too, toured in the story
shell and filmed and tested as the sandbox's are.

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
        do @run{id COMMAND_PALETTE_OPEN}
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
  `@click{}` (`@click{secondary true}`: a right-click, its context
  menu; `@click{mods (ctrl)}`: a Ctrl-click), `@drag{to <target>, edge right}` or `@drag{dx 120}`,
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
  objects by key, arrays by position and length); an async one (an
  extension's) is waited for by the headless runner, not by a tour. There is no second query
  language: a predicate is an action. Unset, the step is done when its
  `do` is.
- A step with a `target` and no `do` points at it (a tour's "this is the
  trigger"): it is done once the target is on screen.
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

- **The window's document, offscreen**: each document is made by
  `architect_workspace::native::offscreen_document` (`features/workspace/src/native.rs`),
  from the desk window's own `document_config` (assets, `file:` and `data:`
  URLs, the HTML parser), with a document head (`OffscreenHead`): what
  `document::Style` asks for goes into the document as it settles, so a
  film draws with every app stylesheet the window has. Without a head
  Dioxus drops them, and a film drew each icon button's hidden name beside
  its icon. `apps/sandbox/media/tests/fidelity.rs` holds this: the head
  against `tests/golden/head.txt`, and every icon button's and icon-only
  tab's icon centred within a point. A text target skips hidden text
  (`visibility: hidden`, `opacity: 0`): a tip's name is not what a person reads.
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
field), and the app's own (the sandbox's `LAB_STATE`,
`apps/ui/src/guide.rs`).

## The film

`features/guide/src/capture.rs` `Recorder` is the run's `Film`. Each
frame kept, every window is painted on the CPU rasteriser (no GPU), put on
one canvas with a title bar when `capture.desktop` is set (else the main
window edge to edge), with the pointer and a press's ripple drawn over it
(the film's own layer: the app never sees it), cropped and scaled, and
written as a PNG. `encode` makes the media: gifski for the GIF, ffmpeg
for WebM (VP9) and MP4 (H.264), the last frame as the poster. Both are
run as programs; the dev shell has them (`devenv.nix`). A still is a
scenario of one step.

An app's media binary is three lines over `features/guide/src/cli.rs`
`main`. The sandbox's is `apps/sandbox/media` (`sandbox-media`), its scenarios in
`apps/sandbox/scenarios`:

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

`apps/sandbox/media/tests/scenarios.rs` runs the sandbox's under `cargo test`.

## The tour

`features/guide/src/tour.rs` (feature `ui`) plays a scenario live, over
the running app, for a person to follow. An app offers its tours with
`use_tours(list, TourSet)` and mounts `TourLayer` once, the last thing in
its root: an absolute layer against the root (Blitz has no `position:
fixed`), always mounted and drawn only while a tour plays (an overlay
inserted over panes as they mount collapses them; see
`skills/architect/references/blitz-quirks.md`).

- **Starting one** is an action: `GUIDE_TOUR_START {id}` on the app's
  Action List, so a model over MCP, a scenario and a palette that lists
  the list can; `GUIDE_TOUR_LIST` lists them. The sandbox also gives each a
  ⌘K row (Help › Tour: …) that calls `Tours::start`.
- **The overlay** dims the window around a cutout over the step's target
  (four dim pieces, so the target shows through untouched) and rings it;
  a drag's `to` is ringed dashed. A gesture is *shown*, never performed:
  the popover says what to do ("Drag what is highlighted onto the dashed
  outline", "Press ⇧⌘K", an action by its name and key) under the step's
  `text`, with Back, Skip and Next. The layer takes no presses but the
  popover's (`pointer-events: none`), so the person acts on the app
  itself. The cutout and the popover glide on the shell's motion
  (`architect_shell::motion`, [Motion](@/architecture/motion.md)): a
  filmed tour shows them move.
- **Finding the target in a live window** is
  `features/guide/src/locate.rs` `find_live`: the layer's own mounted
  element is a Blitz node handle (`dioxus_native_dom::NodeHandle`), which
  reaches its document; the target is resolved there as the runner
  resolves it (`find_in`, the same resolvers), and measured from the
  layer's top left. The tour's own layer is left out of an `@text`
  search.
- **Moving on**: the tour watches the Action List in process
  (`ActionList::events_since`), a look a frame. A step with a `done` moves
  on when one of its events is heard and every check holds (the same
  predicates as the test: `features/guide/src/predicate.rs`); a step with
  none moves on at Next. A hidden step is setup: the tour runs its `@run`
  itself and moves on when its `done` holds (or its timeout passes).
  `GUIDE_TOUR_STATE` says where it is, and each move is a `guide` change
  event; `GUIDE_TOUR_NEXT`, `GUIDE_TOUR_BACK` and `GUIDE_TOUR_SKIP` move
  it from outside.
- **Chapters**: Back to a step runs the setup steps right before it again
  (`setup_of`), and `GUIDE_TOUR_START {id, step}` (`Tours::start_at`)
  starts at a step from its setup, so a tour of chapters (setup, then a
  step to read and try) puts the app where each chapter needs it. Write
  each chapter's setup for the whole app it needs, not for where the
  chapter before left it.
- **Remembering the place**: `TourSet::on_move` hears every move
  (`Moved::At`, `Left`, `Finished`); the sandbox keeps the Architect tour's
  place in its folder and resumes it from ⌘K › Take the tour
  (`apps/ui/src/guide.rs` `tours`,
  [The Architect tour](@/getting-started/tour.md)).
- **Tours that come and go**: `Tours::offer(owner, scenarios)` puts an
  owner's tours (an extension's `[[tours]]`) beside the app's, in place of
  what it offered before; none withdraws them, ending one that plays.
  `TourInfo::owner` says whose each is. `Tours::invite(id)` shows a card
  that invites the person to take one (after an install: [Extensions,
  Tours](@/architecture/extensions.md)), and `GUIDE_TOUR_STATE`'s `invited`
  says which.
- **Safety**: a scenario not marked `safe` never plays over the person's
  own data. `TourSet::playground` sets up the app's playground first (a
  sample document, a scratch session); an app with none cannot start
  it. The sandbox's data is all made up, so it is its own playground.

A film can carry a caption and a terminal of its own, drawn over the sandbox
by the media crate and never by the sandbox: `FILM_CAPTION {text}`,
`FILM_TERMINAL {line, clear?, title?}`, and `FILM_WRITE {path, text}`, a
file saved in the run's own lab folder as an editor would
(`apps/sandbox/media/src/lib.rs`; each run gets a fresh, empty folder, read
and watched as a window's lab reads it: `LabFiles`). The README's gallery
is filmed that way: a theme file or a page saved live, the CLI commands an
agent ran.

`apps/sandbox/scenarios/tour-palette-run.styx` films the tour itself: it
starts the `palette-run` tour from the Action List and follows it, each
step done as a person would, checking the tour moved on after each.

## A story's scenarios

`#[story(scenario = "file.styx")]` (once per file, relative to the
story's source) includes a scenario into the story
(`features/story/architect-story-core/src/lib.rs` `Story`, its
`scenarios`). Then:

- `architect-story-shell` with feature `tours` offers every story's
  scenarios as tours over the Lookbook and puts a button per scenario on
  its story;
- `features/guide/src/story.rs` (feature `story`) runs them over the
  story alone (`app`: its render with its knobs' defaults, an Action
  List, its scenarios as tours): `test_stories` plays them as tests,
  `features/guide/src/cli.rs` `main_stories` films them.

The proof is the Collapsible story's
(`features/ui/architect-ui/src/components/collapsible.styx`), played by
`features/ui/architect-ui/tests/scenarios.rs`.

## Where the media live

Not in git on `main`. An orphan `media` branch holds only the generated
files (`<id>.gif`, `.webm`, `.mp4`, `.png`); the README and docs embed
`https://raw.githubusercontent.com/FastTrackStudios/architect/media/<id>.gif`,
the poster PNG as the still. `just media-publish` regenerates every
scenario and commits the outputs to that branch in a temporary worktree,
never touching the checkout it is run from; `just media-publish 1` pushes
it too.

## The sandbox's scenarios

| Scenario | Shows |
|---|---|
| `apps/sandbox/scenarios/palette-run.styx` | ⌘K opened, a command found by typing, picked |
| `apps/sandbox/scenarios/tab-reorder.styx` | a tab dragged along its bar |
| `apps/sandbox/scenarios/stacked-tabs.styx` | a group stacked, its strip scrolled |
| `apps/sandbox/scenarios/showcase-layout.styx` | the README's showcase: notes opened, a split by dragging a tab to a group's side, a vertical one, two dividers pulled, a group stacked and filled, the sidebar dragged out by its grip into a window of its own |

The rest are the guide's pictures, one per thing a page shows, each named
for the picture it makes (`workspace-split-groups`, `menu-bar-in-window`,
`settings-search-to-row`, …): the docs embed them by that id. A Vault
scenario closes the Clicker pane first, in hidden steps: it is a process
extension's view, and headless its program is not running.

## Status

Built: the model and its loader (wasm-clean), the runner with multiple
windows, the film and its encoders, the CLI and recipes, the test mode,
the workspace's and the commands' resolvers and actions, the text target,
the sandbox's scenarios, the in-app tour and its live target path, a story's
scenarios (toured in the story shell, filmed and tested by the runner).

Next:

- **A tour in a browser**: `find_live` reaches a Blitz document; a web
  build needs the same over the DOM (`getBoundingClientRect`).
- **Keys through the window**: a headless document has no window, so
  window-level chords (`WindowKeys`) do not fire; a scenario runs the
  command (`@run{id COMMAND_PALETTE_OPEN}`) where a person presses ⌘K.
