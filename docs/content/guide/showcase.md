+++
title = "The extension showcase"
description = "Every capability an extension needs, with its status on desktop, web and iOS and the evidence for it: the checklist #710 keeps current."
weight = 79
+++

This page is the checklist for #710: every capability an extension needs
to stand in for the top Obsidian plugins, the REAPER-class extensions
Studio needs and our own apps (Health #371, Finance #370, Studio's
setlists), with where it runs today. Every PR in that effort updates the
rows it moves, with its evidence.

The ids are #710's. The first column of evidence is an automated check;
the second is screenshots at 390 px and 1280 px wide, light and dark, for
each platform the row claims.

## Run it and capture it

| Platform | Run it | Capture it |
|---|---|---|
| Desktop | `just showcase` (the standard distribution; `just showcase plain` for Architect's own app): a fresh scratch copy of `apps/ui/showcase-vault`, with every folder in `examples/showcase` and the sandbox's outline and surface loaded as development copies (`scripts/showcase.sh`) | `just showcase-shots`: an isolated Loft session per width, 390 px and 1280 px, light and dark, drawn by the app itself (`APP_SHOT`) into `target/showcase/desktop/` (`scripts/showcase-shots.sh`; its steps open vault paths and run actions) |
| Web | `/app/?showcase`: the web app's demo mode on the same vault, in the tab's memory (`apps/web/src/demo.rs`) | - |
| iOS | a TestFlight build (the iOS workstream) | the simulator on airlock |

The desktop screenshots so far come from `just showcase-shots plain`
(Architect's own app with the same vault and examples), while `just
showcase` runs the standard distribution by default; the standard
distribution's shots are `just showcase-shots standard`, after building
it.

`just showcase-publish PUSH=1` puts the screenshots on the `media` branch
under `710-showcase/<platform>/`, where the cells below link them.

## How to read a cell

| Mark | Means |
|---|---|
| ✅ | Works, shown by a showcase example, with a test and screenshots on that platform. |
| ⚠️ | Works in part (the cell says what is missing), **or** the mechanism works and has a test but the showcase has no evidence on that platform yet ("not shown"). |
| ❌ | Does not run there. The cell gives the reason and the issue. |
| n/a | The capability is not about that platform. |

A cell that relies on the in-memory sample documents the app's host
gives extensions today (`apps/ui/src/extensions.rs` `documents`), rather
than the person's own vault, stays ⚠️ until #706 lands.

The seed is the parity re-run of 2026-11-14 against `0ec8b0a9` (every
verdict read from code, nothing run) and the tests named in the evidence
column. iOS cells marked "not shown" are what the shared `apps/ui` host
should do there; nobody has run them on a phone for this page yet.

**Web.** The web app runs its distribution's linked extensions on the
same host as the desktop app (#495, W1: `architect_web::run`,
`apps/web/src/app/extensions.rs`), without Luau. Installed extensions
(Luau, pages, WASI) and the document API over the web vault are #495's
next steps: a web cell that says only "#495" waits on them.

## R: runtimes

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| R1 | A linked extension, compiled into the distribution | ⚠️ not shown; the standard build links the first-party extensions and Studio's | ✅ the Architect app's (Projects, Tasks, Time, Calendar, Day Plans, the theme picker, the Command Palette) run in `/app/?demo` (and `?showcase`, the same mode); the signed-in `/app/` runs the same host, not tested signed in yet; Studio's need a `linked-core` build from Studio (#495) | ⚠️ not shown; same host | Setlist viewer, Tasks, Calendar | `features/extensions/tests/linked.rs`; web: `apps/web/test/app-linked-extensions.test.mjs`, [Calendar 1280](https://github.com/FastTrackStudios/architect/blob/pr-media/web-linked-extensions/calendar-1280-light.png?raw=true), [390 dark](https://github.com/FastTrackStudios/architect/blob/pr-media/web-linked-extensions/calendar-390-dark.png?raw=true) |
| R2 | A Luau script installed at run time; its actions and saved scripts | ⚠️ not shown | ❌ #495 | ⚠️ not shown; interpreted | Templater-lite | `features/extensions/tests/metronome.rs`, `features/extensions/tests/host_service.rs` |
| R3 | A TSX/JS page installed at run time (QuickJS; a Web Worker in browsers) | ⚠️ not shown | ❌ the Worker runner exists, no host (#495) | ⚠️ not shown; QuickJS, no JIT | Templater-lite | `features/pages/tests/js.rs` |
| R4 | A WASI component installed at run time | ❌ the release build has no `wasi` feature (#707); runs in a `wasi` build | ❌ #495, and JSPI (no Safari before 27) | ❌ #707 (Pulley once on) | WASI Counter | `features/extensions/tests/wasi.rs`, `features/extensions/tests/wasi_web.rs` |
| R5 | A process extension | ⚠️ not shown | ❌ by nature; no "desk only" state yet (#578) | ❌ by nature; no "desk only" state yet (#578) | Clicker | `features/extensions/tests/remote_view.rs` |
| R6 | A web view app in a pane | ❌ #222 | ❌ #222 | ❌ #222 | Web app pane | - |
| R7 | TypeScript actions with no Rust | ❌ #536 | ❌ #536 | ❌ #536 | - | - |

## L: install and lifecycle

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| L1 | Install from a registry: a signed archive, a consent prompt listing capabilities | ⚠️ not shown | ❌ #495 | ⚠️ not shown; Luau, pages and data only | - | `features/extensions/tests/distribution.rs`, `features/extensions/tests/registries.rs` |
| L2 | Install on open: a vault, project or share link that needs extensions prompts once | ❌ #578 | ❌ #578, #495 | ❌ #578 | - | - |
| L3 | Disable, enable, remove, pin, update; a development copy with hot reload | ⚠️ not shown | ❌ #495 | ⚠️ not shown | - | `features/extensions/tests/check.rs`, `features/extensions/tests/metronome.rs` |
| L4 | An extension that cannot run here says why, and its data is kept | ❌ #578 | ❌ #578 | ❌ #578 | - | - |

## C: contributions

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| C1 | Actions in ⌘K, keymaps, the CLI, MCP and the control socket | ⚠️ not shown | ⚠️ linked extensions' actions and views in ⌘K, their keys (⌘K is the Command Palette extension's); a browser has no CLI, MCP or socket; an action reading the vault waits on documents (#495) | ⚠️ not shown | Templater-lite | `features/extensions/tests/host_service.rs`; web: `apps/web/test/app-linked-extensions.test.mjs` |
| C2 | Toolbars with toggle state, status items, the phone bar | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Toolbar sampler, Outline | `features/extensions/tests/toolbars.rs`, `features/extensions/tests/contributions.rs` |
| C3 | Context menus (file, editor, row) and a menu at a point | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Toolbar sampler | `features/extensions/tests/toolbars.rs`, `features/extensions/tests/host_ui.rs` |
| C4 | Tabs, panes, slots, windows, popups | ⚠️ not shown | ⚠️ a linked extension's views open as workspace panes; slots, windows and popups not shown | ⚠️ not shown | - | `features/extensions/tests/contributions.rs`, `features/extensions/tests/interop.rs`; web: `apps/web/test/app-linked-extensions.test.mjs`, [Projects](https://github.com/FastTrackStudios/architect/blob/pr-media/web-linked-extensions/projects-1280-dark.png?raw=true) |
| C5 | Settings sections | ⚠️ not shown | ✅ linked extensions' sections (Calendar, Day plans, Tasks…) on the settings page; the host service reads their values from the page from #736 | ⚠️ not shown | Settings and theme | `features/extensions/tests/metronome.rs`; web: `apps/web/test/app-linked-extensions.test.mjs`, [settings](https://github.com/FastTrackStudios/architect/blob/pr-media/web-linked-extensions/settings-1280-light.png?raw=true) |
| C6 | Openers: ⌘O sources and URL claims | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Openers sampler | `features/extensions/tests/contributions.rs` |
| C7 | Note and document kinds, project types, templates, capture targets | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Templater-lite, Openers sampler | - |
| C8 | Base view types | ⚠️ not shown; the Tasks extension's board is one | ❌ #495 | ⚠️ not shown | Openers sampler | - |
| C9 | Navigator providers; file-tree icons and badges | ⚠️ icons and colours through `NAVIGATOR_ICON`; the generic file-row badge is drawn only in a test, not the app's tree | ❌ #495 | ⚠️ same as desktop | Iconize-lite | `features/extensions/tests/host_ui_view.rs` |
| C10 | Embeds: fence renderers and inline embeds | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Dataview-lite | `features/pages/tests/embed.rs` |
| C11 | The editor: commands on the selection, keymaps by node, decorations, slash rows | ⚠️ decorations and widgets shown (the outline example marking TODOs, gutter markers, the count widget): [390 light](https://raw.githubusercontent.com/FastTrackStudios/architect/media/710-showcase/desktop/lab-outline-mark-390-light.png), [390 dark](https://raw.githubusercontent.com/FastTrackStudios/architect/media/710-showcase/desktop/lab-outline-mark-390-dark.png), [1280 light](https://raw.githubusercontent.com/FastTrackStudios/architect/media/710-showcase/desktop/lab-outline-mark-1280-light.png), [1280 dark](https://raw.githubusercontent.com/FastTrackStudios/architect/media/710-showcase/desktop/lab-outline-mark-1280-dark.png); keymaps by node, commands on the selection and slash rows tested only. The gutter markers overlap the list bullets | ❌ #495 | ⚠️ not shown; keys need a keyboard | Outline | `features/extensions/tests/editor.rs` |
| C12 | Favourites and phone tabs | ⚠️ not shown | ❌ #495 for an extension's; the app's own favourites run | ⚠️ not shown | - | - |
| C13 | Themes and tokens | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Settings and theme | `features/extensions/tests/themes.rs` |

## H: host services

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| H1 | The person's real vault through the document API | ⚠️ the API acts on an in-memory sample store (#706); `VAULT_*` actions reach the vault | ❌ #495 | ⚠️ same as desktop (#706) | Templater-lite | `features/extensions/tests/documents.rs` |
| H2 | Document events for the person's own edits | ❌ the vault raises no `doc.*` events (#706); an extension's own edits do | ❌ #495 | ❌ #706 | Dataview-lite | `features/extensions/tests/events.rs` |
| H3 | Vault queries (bases rows, links, tags, search); per-note parsed metadata | ⚠️ the queries run as actions; no per-note headings, list items or tasks (#706) | ❌ #495 | ⚠️ same as desktop | Dataview-lite | - |
| H4 | Extension data and settings that persist | ⚠️ not shown | ❌ kept in memory (#495) | ⚠️ not shown | - | `features/extensions/tests/host_service.rs` |
| H5 | Secrets | ⚠️ not shown | ❌ not in a browser yet (#495) | ⚠️ not shown | Secrets and fetch | `features/extensions/tests/net.rs` |
| H6 | `fetch` limited to the manifest's hosts | ⚠️ not shown | ❌ #495; CORS applies | ⚠️ not shown | Secrets and fetch | `features/extensions/tests/net.rs` |
| H7 | Notifications: in-app toasts; OS notifications | ⚠️ toasts only; OS notifications #709 | ❌ #495 | ⚠️ toasts only (#709) | Notify sampler | `features/extensions/tests/host_ui.rs` |
| H8 | The clipboard | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Notify sampler | `features/extensions/tests/host_ui.rs` |
| H9 | Pickers for files, folders, photos | ⚠️ not shown | ❌ #495 | ❌ a pick answers `unsupported` (#708) | - | `features/extensions/tests/host_ui.rs` |
| H10 | Prompts, confirms, pick-one | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Templater-lite, Notify sampler | `features/extensions/tests/host_ui.rs` |
| H11 | Timers, cron hooks, background workers | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Timers sampler | `features/extensions/tests/events.rs`, `features/extensions/tests/workers.rs` |
| H12 | Services between extensions (`[provides]` / `[requires]`) | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Services sampler | `features/extensions/tests/interop.rs` |
| H13 | MIDI and OSC ports and control surfaces | ⚠️ not shown; a virtual MIDI port and UDP OSC, no system MIDI | ❌ #495 | ⚠️ not shown | Surface | `features/extensions/tests/ports.rs` |
| H14 | Deep links into actions | ❌ #186 | ❌ #186 | ❌ #186 | - | - |
| H15 | Device APIs: camera, photos, HealthKit, location | ❌ #709 | ❌ #709 | ❌ #709 | - | - |
| H16 | An extension's server-side code on the node | ❌ #583 | ❌ #583 | ❌ #583 | - | - |

## U: drawing and input

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| U1 | Remote views: forms, lists, dialogs | ⚠️ not shown | ❌ #495 | ⚠️ not shown; pages only (no processes) | Clicker | `features/extensions/tests/remote_view.rs` |
| U2 | Rich input in remote views: drag, pointer capture, touch, scroll, IME, focus | ⚠️ clicks only (#176) | ❌ #495, #176 | ⚠️ clicks only (#176) | Kanban-lite | - |
| U3 | GPU painting and canvas | ⚠️ linked views only (#591); none for installed (#175) | ❌ #495, #175 | ⚠️ linked only (#175) | Sketch | - |
| U4 | Charts (SVG) | ⚠️ not shown | ❌ #495 | ⚠️ not shown | Chart | - |
| U5 | Accessibility: names and roles for VoiceOver | ❌ #164 | ❌ #164 | ❌ #164 | - | - |

## W: the web app hosts extensions

| Id | Capability | Desktop | Web | iOS | Example | Evidence |
|---|---|---|---|---|---|---|
| W1 | The web app runs its distribution's linked extensions | n/a | ⚠️ `architect_web::run(<Distribution>)` runs the Architect app's own linked extensions (tested); `cargo xtask dist web`'s path for a distribution crate's own wasm (`web` feature) is not run by any test yet (studio#500 is its first) | n/a | Tasks, Calendar | `apps/web/test/app-linked-extensions.test.mjs` (light and dark, 390 and 1280 px) |
| W2 | Installed Luau, pages and WASI run in the browser, with documents and storage | n/a | ❌ #495 | n/a | - | `features/extensions/tests/wasi_web.rs` (the runner, not the app) |

## The example extensions

Each proves a set of the rows above and says, in its manifest and its
note in the showcase vault, which plugins or apps it stands in for.

| Example | Stands in for | Proves | Status |
|---|---|---|---|
| Templater-lite | Templater, QuickAdd | R2, R3, C1, C7, C11, H1, H2, H10 | planned |
| Dataview-lite | Dataview, Tasks, Bases | C10, C8, H3, H2 | planned |
| Kanban-lite | Kanban | U2, H1 | waits on #176 |
| Outline (tables and outline keymaps) | Advanced Tables, Outliner, Editing Toolbar | C11, C2 | in the sandbox (`apps/sandbox/extensions/outline`), loaded by `just showcase`; C11 shown on the desk |
| Iconize-lite | Iconize | C9 | planned |
| Toolbar sampler | Editing Toolbar, Commander | C2, C3 | planned |
| Settings and theme | Style Settings, Minimal Theme Settings | C5, C13 | planned |
| Openers sampler | Homepage, Bases view plugins | C6, C7, C8 | planned |
| Services sampler | Calendar, Day Planner, Tasks | H12 | planned |
| Timers sampler | TaskNotes' pomodoro, Recent Files | H11 | planned |
| Secrets and fetch | Copilot, Remotely Save | H5, H6 | planned |
| Notify sampler | Notice, clipboard and modal users | H7, H8, H10 | planned |
| Chart | Charts, Tracker | U4 | planned |
| Surface | CSI, ReaLearn | H13 | in the sandbox (`apps/sandbox/extensions/surface`) |
| Sketch | Excalidraw, Canvas, Graph | U3, U2, C6 | waits on #175, #176 |
| Search index | Omnisearch, Smart Connections | H11, H1, H2, H4 | waits on #706 |
| Sync-to-URL | Remotely Save, Git | H6, H5, H11 | planned |
| Meal plan | Health (#371) | C7, H12, U4, H9, H15, H7 | waits on #708, #709 |
| Ledger | Finance (#370) | C8, U4, H9, H6, H5, H16, R6 | waits on #583, #222 |
| Web app pane | third-party TypeScript apps | R6 | waits on #222 |
| Setlist viewer | Studio | R1 | in the standard distribution |
