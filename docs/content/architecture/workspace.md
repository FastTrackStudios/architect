+++
title = "The workspace: docking, tabs and screensets"
description = "One layout system for every app: tab groups in splits, docks on each side, stacked tabs, pop-out windows, per-tab history, and screensets — as flexible as Obsidian, REAPER and Premiere, so no app builds docking again."
weight = 54
+++

> **Status: model, desk host and pop-out windows landed (2026-10-31).**
> The model is `architect-workspace` (`features/workspace/src/model.rs`,
> `ops.rs`, `screenset.rs`, `store.rs`); the host is its `ui` feature
> (`features/workspace/src/host/mod.rs` `WorkspaceView`, `View`, `Views`,
> `TitleRow`), developed in the sandbox's Vault tab
> (`apps/ui/src/vault.rs`). Pop-out OS windows are the `native`
> feature's launcher (`features/workspace/src/native.rs` `launch`); tabs
> reorder within a bar, a finger held on a tab opens its menu, stacked
> tabs slide over each other, ⌘1–9 recall screensets for the screen's
> class, the layout is kept through the app's `Store`, and a pane puts
> its own items on the status line (`use_pane_status`). A dock (the
> sidebar, with its ribbon and groups) pops out into a window of its own
> and docks back (`Workspace::pop_out_dock`, `Workspace::dock_back`; the
> `WORKSPACE_POP_OUT_*_DOCK` and `WORKSPACE_DOCK_BACK_*_DOCK` actions).
> Every operation is an Action List action with an explicit target
> (`features/workspace/src/control/mod.rs`; "On the Action List" below),
> and the view commands run them. Still to come: dragging a tab onto a
> group in another window (it joins
> that window's first group), floats in pop-outs, the app's chords inside
> a pop-out. Studio's dock (`studio:features/ui/dock`) was
> the starting point; Obsidian's workspace is the bar to clear.

An app hands architect its **views** (what a pane can show) and gets back
a workspace a person arranges as they like: any number of tab groups,
split any way, docked to any side, stacked, popped out into windows of
their own, and saved as screensets to recall with a key. The app never
lays out a pane, draws a tab, handles a drag or saves a layout.

What draws a view is its kind: a Dioxus component by default, or a GPU
scene, a web view or a Frame surface a linked crate provides (see
[Embedded views](@/architecture/embedded-views.md)).

The [app shell](@/architecture/app-shell.md) is the frame (title row,
tabs, the separate button, the account); the workspace is what fills its
page area on a tablet and a desk. On a phone it folds into the shell's
navigation (below).

## What a person can do

Everything Obsidian's workspace does, and REAPER's screensets on top:

- **Tabs in every group.** Each tab group has its own tab bar: its tabs,
  a **+** (a new tab), and a menu (the group's: stack, close the group,
  split). A tab shows one view; under the tab bar, the view's own row
  (back and forward, its title, its actions).
- **Split anything.** Drag a tab onto a group's edge to split it there
  (left, right, top, bottom), onto its middle to join it, onto a tab bar
  to put it in that place. Or from the tab's menu: Split right, Split
  down. Splits nest any depth; dividers drag; a double click evens them.
- **Docks on each side.** Left, right and bottom docks hold tab groups of
  their own (split top and bottom). Each docks open, collapses to its
  icons, or hides; its size drags. A dock's groups show their tabs as
  icons (files, search, bookmarks), not names: a dock is narrow.
- **The two-stage sidebar.** Beside a dock, a ribbon: a thin column of
  icons in two groups. **Navigation**, at the top: one icon per view the
  dock offers; a press shows that view in the dock (opening the dock if
  it is collapsed), a press on the one showing collapses the dock.
  **Pinned commands**, under them: any action from the Action List the
  person has pinned there (today's note, a new project, a screenset); a
  press runs it. So the left side is two stages: the ribbon, always
  there, and the dock with the focused view, there when wanted (VS Code's
  activity bar, Obsidian's ribbon and sidedock). Both groups are the
  person's to arrange: drag to reorder, hide, drag a view or a command
  (from the palette, a menu, the action list) onto it to pin it. Either
  side may have a ribbon. At the ribbon's foot, a grip: a press pops the
  dock out into a window of its own, dragged out of the window it pops
  out where it is let go (below).
- **Stacked tabs.** A group's menu turns stacking on (and off): its tab
  bar goes, its tabs become panes side by side, each a fixed width (the
  group's, 700pt by default, narrower where the screen is), in a strip
  that scrolls sideways with a scrollbar under it. Each pane has a spine
  down its left edge (about 40pt): its close ×, its icon, and its title
  written vertically. As the strip scrolls, a pane slides over the ones
  before it (with a shadow), and those leave their spines piled at the
  left edge; the panes still to come pile their spines at the right. A
  press on a spine scrolls its pane into view; the + opens a new pane at
  the end. (Andy Matuschak's sliding notes; Obsidian's "Stack notes".)
- **Windows.** A tab dragged out of the window, or "Move to new window",
  opens a window of its own, which is a full workspace (splits, groups,
  stacks) without docks. Dragging a tab onto another window's group moves
  it there. Closing the main window closes its windows; the layout keeps
  them.
- **A dock in a window of its own.** A dock (left, right or bottom: its
  ribbon and its groups) pops out into an OS window (`dock-left`,
  `dock-right`, `dock-bottom`), by its grip, its tabs' menu or the
  action; the main window's layout closes the gap, and on a desk whose
  shell hosts the left dock the shell's sidebar folds. Tabs moved to that
  dock go to its window while it is out. The grip there (a press, or
  dragged onto the main window), "Dock the sidebar back", or closing its
  window puts it back on its side with its size and groups. Its last tab
  gone, the window closes and the ribbon comes home. A screenset or a
  kept layout with the dock out brings it back out.
- **Pin and link.** A pinned tab is never reused: what would open in it
  opens in a new tab. Linked tabs follow each other: open a song in one,
  its mixer in a linked tab follows.
- **History per tab.** Each tab keeps its own back and forward, for
  views that navigate.
- **Reopen a closed tab** (⌘⇧T), as many times as there are closed tabs.
- **Focus.** One pane is active across every window; keys move it to
  the pane left, right, above or below. Commands that act on "the
  current document" use the most recent main-area pane, so focusing a
  dock does not change their target.
- **Screensets.** A layout is a value: save it under a name, recall it
  from a menu or a key (⌘1–9, REAPER's numbered slots), save changes back
  or **reset to the saved layout** (Premiere's). A screenset holds every
  window and dock, and keeps one layout per form factor, so the same
  "Mix" set is arranged for a desk and for an iPad on its side. A
  project may carry its own screenset.
- **Arrange mode.** Hold **W** (Studio's): every pane shows its name and
  can be dragged or right-clicked anywhere, even a pane whose view has no
  tab bar (a full-bleed rig face).

## Decks: one context each, shown in a screenset

> **Status: in progress (#441).** The model is
> `features/workspace/src/deck.rs` (`DeckState`), its actions
> `features/workspace/src/control/deck.rs`; the strip is the `ui`
> feature's `features/workspace/src/host/deck.rs` (`use_decks`,
> `DeckStrip`, `DeckHandle`). "Deck" is the working name, written once:
> `deck_word!` in `features/workspace/src/lib.rs` (the words a person
> reads and the actions' `DECK_` prefix).

An app with decks calls `use_decks` beside its `WorkspaceView` (giving
it the same `Signal<Workspace>` and `Signal<Screensets>`, its stores, its
templates and the decks a first run opens), draws `DeckStrip` over the
`WorkspaceView` (given `under_strip: true`, and no `store`: the decks
keep the screensets and the layouts), and loads contexts with
`DeckHandle::load`.

Studio has a tab per song and Architect a tab per project, each arranged
its own way. Two primitives, kept apart:

- **A screenset** (`Screenset`, above) is a named, reusable layout:
  splits, docks, which views go where ("Mixing", "Charts + Lyrics").
  Screensets are the person's, kept per device (and, through
  `store::load_split`, synced with the account). Nothing in a screenset
  names a song or a project.
- **A deck** (`Deck`) is one loaded **context** (`DeckContext`: a song, a
  project, a vault, an org root: `kind`, `id`, `title`, its folder) plus
  the screenset it is shown in, plus that context's own changes to it
  (its **override**). The decks are the strip at the top of the main
  window (`Decks`: their order, the active one).

So two decks can show two songs in the one "Mixing" screenset. Switching
decks switches both the content and the layout. A person who rearranges
inside a deck changes only that deck's context: the change is kept as
its override, so the song opens again exactly as it was left. The
screenset itself changes only when they say so:

| On a deck | Does | Action |
|---|---|---|
| Load onto a new deck | a context in a deck of its own (⌘-click or middle-click in the navigator) | `DECK_OPEN {context, new: true}` |
| Load onto this deck | the context replaces the active deck's | `DECK_OPEN {context}` |
| Switch this deck's screenset | shows it in another screenset; its override goes | `DECK_SET_SCREENSET` (or loading a screenset while it shows: ⌘⌥1–9) |
| Save as screenset | its override becomes a named screenset it then uses | `DECK_SAVE_AS_SCREENSET` |
| Reset to the screenset | drops its override | `DECK_RESET` |
| Close, next, previous, move, list | | `DECK_CLOSE`, `DECK_NEXT`, `DECK_PREV`, `DECK_MOVE`, `DECK_LIST` |

The strip: open, close, drag to reorder, ⌘1–9 and ⌃Tab / ⌃⇧Tab to
switch, Duplicate and Pop out into a window from a deck's menu, and a
marker on a deck whose layout differs from its screenset (and a badge an
app sets: a song's unsaved session). While decks are on, ⌘1–9 switch
decks and the screensets' slot keys move to ⌘⌥1–9. On a phone the strip
folds into the tab switcher: a row of decks above its cards.

**Each context type has a default screenset** ("Song", "Project"). An
extension contributes one with `[[deck_templates]]` (a context type, the
screenset, and optionally the layout it ships); the app gives its own the
same way (`DeckTemplate`). The person can pick another default per type
(`Decks::set_default`), which beats every template.

**Where things are kept:**

- the screensets: the workspace's `Store`, as before;
- a context's override: with the context, in its folder's
  `.architect/deck.json` (a project's), or the vault's
  `.architect/decks/<type>/<id>.json` for a context with no folder of its
  own (a song note) ([Interface files](@/architecture/interface-files.md));
  `store::ContextStore` is the seam, `home::ContextFiles` the files;
- the session (which decks are open, in what order, the active one, the
  person's defaults): on the device, per device (`DeckSession`, through a
  `Store`); restored at start, each deck's override read from its context.

**Hidden decks stay live without drawing.** Only the active deck has a
`WorkspaceView`; the others are values (`Deck`: context, screenset,
override, each pane's state and history), so switching unmounts the
outgoing deck's views and draws the incoming deck's from its own panes.
What a context *is* (a vault's index, a project's documents, a song's DAW
session) is the app's, and stays loaded while its deck is open: the app
keeps one per open deck (`Decks::contexts`) and drops it when the deck
closes, so switching never loads it again. A view reads its deck's
context with `use_deck_context`. Budgets per deck (a song's DAW session
is heavy) are the app's to set on that registry; the decks only say which
contexts are open and which one shows.

## Floating windows

Any pane can float — a plugin's editor, a script's panel, a meter, a
chart on a second screen — and floating is one system everywhere, so
custom plugin support arrives on it.

**Inside the window** (every platform, and the only kind on the web —
odysseus's model, AGPL there, so its behaviour, not its code):

- drag by its title bar; near an edge a ghost shows where it would go:
  the **top** maximises it, the **left or right** half-tiles it, or
  docks it as a full-height side panel the page makes room for (a wide
  sidebar folding to its rail when the page would get too narrow); drag
  away restores its size;
- resize from any edge; it keeps inside the window;
- a press brings it to the front;
- **minimise** keeps it alive as a chip in a dock of minimised windows
  (the dock can be dragged and is remembered); a press on the chip
  restores it where it was;
- close; reopen puts it back where it last was.

**As its own OS window** (desk): a float can pop out into a real window
— another screen, kept above the main window or not — and back in.

**Plugin editors (CLAP, VST3) on a desk:** an editor is a native view
the plugin draws itself (an `NSView`, an `HWND`, an X11 window), not
something Blitz can paint. Two ways to show one, the person's choice per
plugin:

- **Embedded in a pane** (REAPER's and Bitwig's): the host places the
  plugin's native view as a child of the app's window, over the pane's
  rectangle, and keeps it there — moved and resized with the pane,
  hidden when its tab is not showing, the pane is collapsed or scrolled
  away, or the window is minimised. So a plugin docks, tabs, stacks and
  goes into screensets like any view. One rule follows from a native
  view sitting above the painted window: nothing the app paints can be
  drawn over it, so while a menu, a drag ghost, a tooltip or another
  float would overlap it, the host hides the plugin's view (or shows a
  still of it) until the overlap ends.
- **Its own window:** a child OS window of the app's (above it, closed
  with it, remembered where it was) — for a plugin that will not
  resize, one that misbehaves when re-parented, or a second screen.

Either way the app's strip (bypass, presets, A/B, the parameter panel)
sits beside it, and opening a plugin from the FX row opens it where it
last was, or brings it forward.

**In a browser** there are no third-party plugin GUIs: no plugin runs
there. The web shows what can draw in a page — the app's own effects
(their UI is the app's), JSFX with their `@gfx` on a canvas, and the
generated parameter panel for anything else. A browser driving a desk's
engine remotely shows that engine's plugins by their parameter panels.

### Window ids: anything can find a window

Every pane and every window has an **id that stays the same**: the one
its maker gave it (a script's `"timer"`, a plugin window's
`fx:<track>/<slot>`), or its view and subject (`session.mixer`,
`studio.song:<id>`). Opening an id that is already open brings that one
forward instead of making another. So other scripts, actions, keys and
outside tools can target it:

<!-- doc-check: off (proposed names) -->
```lua
-- A script makes a floating timer panel; reopening it finds the same one.
local w = workspace.open{ id = "timer", title = "Timer", view = timer_view,
                          float = { size = {220, 90} } }
-- Pin it to the top right, above everything, from this script or any other.
workspace.window("timer"):pin{ corner = "top-right", above = "all" }
```
<!-- doc-check: on -->

- **Actions** take window ids: `workspace.focus { id }`,
  `.float { id, at }`, `.pin { id, corner, above }`, `.minimise`,
  `.close`, `.toggle` — bindable, in ⌘K, runnable by scripts and agents.
- **Outside tools see them:** a popped-out window carries its id to the
  platform — the window's app id and role on Linux (so Loft, or a
  Hyprland rule, can place it: "float `timer` top-right, always on top"),
  its identifier on macOS — so a window manager or an automation pins
  it the same way a script does.
- **Script and extension panels** are panes like any other: docked, in a
  tab group, stacked, floating, minimised, in screensets — and found by
  their id.

## The model

Pure data (no Dioxus, no I/O), facet-serialised, one per window — the
shape of Studio's `Dockspace`, grown to Obsidian's:

<!-- doc-check: off (proposed names; nothing here exists yet) -->
```rust
/// Everything a person has arranged: the main window and its pop-outs.
pub struct Workspace {
    pub windows: Vec<Window>,          // [0] is the main window
    pub panes: Vec<Pane>,              // every open tab, wherever it is
    pub active: Option<PaneId>,        // one, across all windows
    pub recent: Vec<PaneId>,           // most recent first: "the current document"
    pub closed: Vec<Pane>,             // reopen-closed-tab, newest last
}

pub struct Window {
    pub id: WindowId,                  // stable: the one its maker gave, or its view and subject
    pub bounds: Option<Rect>,          // None: the main window, sized by the OS
    pub maximized: bool,
    pub center: Node,                  // the main area
    pub docks: Docks,                  // the main window only
    pub floats: Vec<Float>,            // floating inside this window
    pub minimised: Vec<PaneId>,        // the dock of minimised windows, in order
}

/// A pane floating inside a window.
pub struct Float {
    pub pane: PaneId,
    pub rect: Rect,                    // or snapped:
    pub snap: Option<Snap>,            // Maximised | Half(Side) | EdgeDock(Side, width)
    pub pin: Option<Pin>,              // a corner, and above what
    pub restore: Rect,                 // where it goes when it unsnaps
}

pub enum Node {
    Split { id: NodeId, axis: Axis, children: Vec<(Node, Share)> },
    Group(Group),
}

/// A tab group: its own tab bar, one tab showing (or all, stacked).
pub struct Group {
    pub id: GroupId,
    pub tabs: Vec<PaneId>,
    pub current: usize,
    pub stacked: bool,
    pub chrome: Chrome,                // Tabs | Bare (a full-bleed view; arrange mode still finds it)
}

pub struct Docks { pub left: Dock, pub right: Dock, pub bottom: Dock }
pub struct Dock {
    pub root: Option<Node>,            // its groups (tabs shown as icons)
    pub size: f32,
    pub shown: Shown,                  // Open | Collapsed | Hidden
    pub ribbon: Option<Ribbon>,        // the two-stage sidebar's first stage
}
/// A dock's ribbon: its views, then the commands pinned under them, each
/// in the person's order.
pub struct Ribbon {
    pub views: Vec<ViewKind>,          // navigation: show this view in the dock
    pub commands: Vec<ActionId>,       // pinned: run this action
    pub hidden: Vec<ViewKind>,
}

/// One tab: a view and its state.
pub struct Pane {
    pub id: PaneId,                    // stable and addressable ("timer", "fx:<track>/<slot>")
    pub view: ViewKind,                // what shows it ("session.mixer")
    pub state: String,                 // the view's own, opaque (facet JSON)
    pub title: String, pub icon: Option<String>, // cached, to draw a tab without building the view
    pub pinned: bool,
    pub link: Option<LinkId>,
    pub history: History,              // back / forward states
}
```
<!-- doc-check: on -->

Invariants, restored after every operation (`repair`): a pane is in
exactly one group; a group is in exactly one place; an empty group goes
and its split collapses into its sibling; a split with one child becomes
the child; a split inside a split on the same axis merges into it;
shares are kept positive and summing to one. A window with no panes
closes; the main window keeps one empty group ("New tab").

What a pane remembers while it is not saved (a scroll position, a
cursor) is the view's business and not part of the layout.

## The operations

Every gesture and every key comes down to a few operations, methods on
`Workspace` (`features/workspace/src/ops.rs`: `open`, `move_pane`,
`close`, `set_stacked`, `set_shares`, `set_zoomed`, `pop_out_dock`,
`dock_back`, …). Each is an **Action List action** with an explicit
target (below), and the host names the common ones as **view commands**:
an `architect_commands::Command` it registers with `use_command` while it
is mounted (`features/workspace/src/host/commands.rs` `WorkspaceCommands`,
and `features/workspace/src/host/screens.rs` for the screensets). A
command is its action run with the active pane as the target and
`focus: true`, so a key, a menu, the palette (⌘K), a script and a model
over MCP arrange the workspace through one code path.

### On the Action List

`features/workspace/src/control/mod.rs` holds every action as a function
over a `State` (the workspace, its screensets, the class of screen) and
an `Env` (the app's views, whether pop-outs are OS windows): no Dioxus,
no I/O. Feature `actions` puts them on an
[Action List](@/architecture/commands.md)
(`features/workspace/src/control/list.rs` `register`), run where the
workspace lives:

- the Blitz host, when `WorkspaceView` is given `actions`
  (`WorkspaceActions::new(list)`): a write is sent to the window's thread
  and answered once it ran there; a read is answered at once from a
  mirror the host keeps current (a guide scenario's checks run
  synchronously);
- a `HeadlessWorkspace` (`State` and `Env` behind a lock) for a server, a
  test, or a model arranging a layout to store
  (`features/workspace/tests/actions.rs` drives every action over MCP).

The rules every write keeps (herdr's agent API, which these follow):

- **Targets are named.** A pane, a group, a window or a dock side is a
  parameter; nothing acts on "the active pane" unless the caller passed
  it.
- **Focus stays put.** `Workspace::active` is the person's: a write moves
  it only with `focus: true` (or `WORKSPACE_FOCUS`). A pane opened or
  moved without it shows in its group; the person's group keeps showing
  their tab.
- **Every write answers** `Answer { changed, reason, pane, group, window,
  screenset, layout }`. Nothing changed comes with a `Reason`
  (`already_open`, `already_zoomed`, `only_tab`, `no_neighbor`,
  `dock_not_out`, …), so a retry is safe and no second read is needed. A
  call that names something not there fails, and changes nothing
  (`control::transact`).
- **Ids are opaque and never handed out twice** (`Workspace::next_id`
  only grows, a recalled screenset's included). A group or a window that
  goes because all its panes went to one other leaves an alias
  (`Workspace::aliases`, `Workspace::resolve_group`,
  `Workspace::resolve_window`): an id read before finds them.
- **Modes, not toggles**, where a toggle would make a retry undo itself:
  `WORKSPACE_ZOOM {mode: on | off | toggle}`.
- **Revisions.** The arrangement has a revision that goes up by one
  with every change of it, whoever made it: a person's drag, a key, a
  command, an agent. A view's own state (a scroll, a cursor) does not
  count. Every write takes `if_revision`: planned on an older layout, it
  changes nothing and answers `stale` with the layout as it is now, so an
  agent that read, thought and wrote never undoes what the person did
  meanwhile.
- **Change events.** Each new revision is a
  `Changed { topic: "workspace" }` event whose detail is a JSON
  `control::Change`:
  - `revision`;
  - `actor`: `app` for the app itself, else the caller's principal, such
    as `service:claude`;
  - `kinds`: some of `opened`, `closed`, `moved`, `focus`, `shown`,
    `resized`, `groups`, `tabs`, `docks`, `floats`, `windows`, `zoom`;
  - `panes`: the panes opened, closed or moved.

  A watcher treats it as invalidation: a revision that skips one means
  events were lost, so read `WORKSPACE_LAYOUT` again.

### The layout an agent reads

`WORKSPACE_LAYOUT` (`features/workspace/src/guide.rs` `Arrangement`)
keeps the flat lists a guide check compares: each window's `groups`,
`splits` and `docked`. Beside them it gives `revision`, `active`,
`current_document`, and each window as a tree
(`features/workspace/src/tree.rs`):

- each window's `bounds`, `maximized` and measured `area`;
- `center`: nested `split {id, path, axis, shares, rect, children}` and
  `group {id, path, rect, measured, tabs, current, stacked, chrome,
  zoomed}`. Each tab has its pane, view, title, pin, link and whether it
  can go back or forward;
- `docks`: side, shown, size, popped out, rect and tree;
- `floats`: group, rect, snap, pin, minimised.

A **path** names a place in one tree: `center`, `center/0/1`, `left/0`,
`float/2`. Ids address things; a path finds an anonymous split
(`WORKSPACE_RESIZE {path, shares}`) and holds for one revision only, as a
split's id may after a repair.

**Rectangles** are window points. The Blitz host measures its area and
each group after the frame that drew them (`Measured`), and such a group
says `measured: true`. Any other group is laid out from the window's
measured area by the shares and the docks' sizes. A zoomed group fills
the center. A headless workspace has no rectangles unless its `Env` is
given an area.

| Action | Effect | Takes |
|---|---|---|
| `WORKSPACE_LAYOUT` | read-only | — : every window's groups (ids, tabs, the one showing, stacked), splits, docks' groups, the active pane |
| `WORKSPACE_PANE_GET` | read-only | `pane`: its view, title, state, window, place, group, index, shown, active, history, float |
| `WORKSPACE_NEIGHBOR` | read-only | `pane`, `direction`: the pane showing next to it that way |
| `WORKSPACE_VIEWS` | read-only | — : the views `WORKSPACE_OPEN` takes |
| `WORKSPACE_EXPORT` | read-only | — : the layout as a portable tree |
| `WORKSPACE_OPEN` | mutating | `view`, `id?`, `state?`, `title?`, `at` (`replace`, `beside`, `group`, `split`, `window`, `dock`, `float`), `focus` |
| `WORKSPACE_MOVE` | mutating | `pane`, `to` (`group`, `beside`, `edge`, `split`, `window`, `dock`, `float`), `focus` |
| `WORKSPACE_SPLIT` | mutating | `pane`, `side`, `focus` |
| `WORKSPACE_FOCUS` | mutating | `pane`, or `from` and `direction` |
| `WORKSPACE_ZOOM` | mutating | `pane`, `mode`: its center group alone in its window, the tree kept |
| `WORKSPACE_RESIZE` | mutating | a `split`'s `shares` (or `even`), a `dock`'s `size`, or a `pane` grown toward a `direction` by an `amount` |
| `WORKSPACE_GROUP_SET` | mutating | `group`, `stacked?`, `chrome?`, `current?` |
| `WORKSPACE_TAB_SET` | mutating | `pane`, `pinned?`, `link?` (`""` unlinks), `title?` |
| `WORKSPACE_NAVIGATE` | mutating | `pane`, `go` (back, forward) or `state` |
| `WORKSPACE_DOCK_SET` | mutating | `side`, `shown?`, `size?` |
| `WORKSPACE_DOCK_POP_OUT` / `WORKSPACE_DOCK_BACK` | mutating | `side`, `bounds?` / `side` |
| `WORKSPACE_FLOAT` | mutating | `pane`, `rect?`, `snap?`, `pin?`, `above_all`, `focus` |
| `WORKSPACE_MINIMISE` / `WORKSPACE_RESTORE` | mutating | `pane` (`focus` to restore to the front) |
| `WORKSPACE_WINDOW_SET` | mutating | `window`, `bounds?`, `maximized?` |
| `WORKSPACE_REOPEN` | mutating | `focus` |
| `WORKSPACE_APPLY` | mutating | `layout` (`WORKSPACE_EXPORT`'s), `keep_unplaced` (default true; else unplaced panes close, kept to reopen) |
| `WORKSPACE_CLOSE` | mutating | `panes` (all or none; each kept to reopen) |
| `WORKSPACE_CLOSE_OTHERS` | destructive | `pane` |
| `WORKSPACE_CLOSE_WINDOW` | destructive | `window` |
| `WORKSPACES_LIST` | read-only | — : the saved layouts (each its slot, built in, active, classes), dirty per class, the options |
| `WORKSPACES_LOAD` | mutating | `id`, `name` or `slot` |
| `WORKSPACES_SAVE` | mutating | `name?`: none saves the active set; a name saves over the set of that name, or as a new one |
| `WORKSPACES_RENAME` / `_SET_SLOT` / `_OPTIONS` / `_RESET` | mutating | `id`, `name` / `id`, `slot?` / `save_before_load?`, `slot_keys?` / — |
| `WORKSPACES_DELETE` | destructive | `id` (a built-in set answers `builtin`) |
| `WORKSPACES_MANAGE` | mutating | — : opens the manage sheet (the Blitz host's own) |

`WORKSPACE_APPLY` builds the whole new workspace before it puts it in
place (herdr's `layout.apply`), so a layout that does not fit (a pane
twice, an unknown view) changes nothing. Panes keep their ids, so their
views are not built again; groups and splits keep the ids the tree
carries, and an export applied over itself changes nothing. A float
keeps its snap and pin by its group; ribbons are the person's and stay.

Destructive actions are marked `Effect::Destructive` only: asking the
person before one runs is the Action List's confirmation step, a
separate piece of work.

The view commands:

| Operation | Commands | What it does |
|---|---|---|
| `open` | (⌘O, the views) | here, a new tab, a split (side), a new window, or a dock (side) |
| `move_pane` | `WORKSPACE_MOVE_TO_LEFT_DOCK` / `_RIGHT_DOCK` / `_BOTTOM_DOCK` / `_CENTER`, `WORKSPACE_MOVE_TO_NEW_WINDOW`, `WORKSPACE_MOVE_TO_MAIN_WINDOW`, `WORKSPACE_FLOAT_TAB` | to a group (and index), a group's edge, a new window, a dock |
| `close` / `close_others` / `reopen` | `WORKSPACE_CLOSE_TAB`, `WORKSPACE_CLOSE_OTHER_TABS`, `WORKSPACE_REOPEN_TAB` | |
| a split | `WORKSPACE_SPLIT_RIGHT`, `WORKSPACE_SPLIT_DOWN` | split the active group, moving its tab |
| `activate` / `goto_tab` / `cycle_tab` | `WORKSPACE_FOCUS_LEFT` / `_RIGHT` / `_UP` / `_DOWN`, `WORKSPACE_NEXT_TAB`, `WORKSPACE_PREVIOUS_TAB` | |
| `set_zoomed` | `WORKSPACE_ZOOM_PANE` | the active pane's group alone in its window, and back |
| `go` | `WORKSPACE_GO_BACK`, `WORKSPACE_GO_FORWARD` | the active pane's history |
| `set_pinned` / `set_stacked` / `set_chrome` | `WORKSPACE_PIN_TAB`, `WORKSPACE_STACK_TABS`, `WORKSPACE_TAB_BAR` | toggles |
| `set_dock_shown` / `set_shares` | `WORKSPACE_TOGGLE_RIGHT_DOCK`, `WORKSPACE_TOGGLE_BOTTOM_DOCK`; dragging a divider | |
| `pop_out_dock` / `dock_back` | `WORKSPACE_POP_OUT_LEFT_DOCK`, `WORKSPACE_DOCK_BACK_LEFT_DOCK`, and the same for `RIGHT` and `BOTTOM` | the dock, ribbon and groups, into a window of its own and back; which of the pair is enabled says where the dock is |
| screensets (`recall`, `save`, `save_named`, `reset`) | `SCREENSET_RECALL_1` … `_9` (⌘1–9, unless the option is off), `SCREENSET_SAVE`, `SCREENSET_RESET`, `WORKSPACES_LOAD_<ID>` (one per set), `WORKSPACES_MANAGE` | named with the app's `sets_noun` |
| | `WORKSPACE_ARRANGE` | arrange mode, latched (for touch) |
| | `WORKSPACE_EVERY_PANE` | every open pane, to pick one |

Opening by kind follows Obsidian: a plain open reuses the active tab
unless it is pinned; ⌘ opens a new tab, ⌘⌥ a split, ⌘⌥⇧ a window.

## What an app gives it

<!-- doc-check: off -->
```rust
// The views: what a pane can show. Registered once, like the tabs of the shell.
let views = Views::new()
    .view(View::new("session.mixer", "Mixer", icon!(SlidersVertical), |pane| rsx! { Mixer { pane } })
        .home(Place::Dock(Side::Bottom))          // where it opens when nothing says otherwise
        .navigates(false))                        // no back/forward
    .view(View::new("session.song", "Song", icon!(Music), |pane| rsx! { SongView { pane } })
        .single(false));                          // more than one may be open

// The screensets it ships with; the person's own are kept beside them.
let sets = vec![Screenset::new("Mix").slot(4).layout(FormClass::Desk, mix_desk())];

rsx! {
    AppShell { /* tabs, account, … */
        Workspace { views, screensets: sets, store: workspace_store }
    }
}

// Inside a view: its pane, its state, its title.
let pane = use_pane();
pane.set_title(song.title.clone());
pane.set_state(&SongState { scroll });
pane.open(Open::view("session.mixer").linked());     // a mixer that follows this song
```
<!-- doc-check: on -->

An unknown view kind (a plugin no longer installed) keeps its pane as a
placeholder that says so, and keeps its state until the view comes back.
Panes in background tabs are not built until shown; their cached title
and icon draw the tab.

Where the workspace is kept is a `store` the app passes: the device by
default ([settings](@/architecture/settings.md)), the person's synced
settings, or the project (a project's own screenset).

## Laid out by form factor, sized by the pointer

Two things decide how the workspace looks, separately:

- **The size** decides the layout (the [app shell](@/architecture/app-shell.md)'s
  rule): a phone's, or the full workspace.
- **The pointer** decides the sizes: a mouse or trackpad (fine) gets
  desk-sized tabs, dividers and buttons; a finger or pen (coarse) gets
  44pt targets, wider dividers, and long-press for what a right click
  does. It follows the last pointer used, so a **touchscreen PC** is a
  desk layout that grows its targets the moment it is touched and shrinks
  them when the mouse comes back, and an iPad with a trackpad does the
  opposite. Hover-only affordances (a tab's ×, a row's reset) are always
  shown under a coarse pointer.

| | Desk (fine) | Touchscreen PC (coarse) | iPad | Phone |
|---|---|---|---|---|
| Groups and splits | as arranged | as arranged | as arranged | one pane at a time: the active group's current tab |
| Tab bars | each group's, 32pt | each group's, 44pt | each group's, 44pt | the tab switcher: a grid of cards (live previews), two columns (more on an iPad in compact width), opened from the action bar's tabs button |
| Stacked group | the sliding strip | the strip, swiped | the strip, swiped | its panes one after another, swiped |
| Two-stage sidebar | ribbon + dock | ribbon + dock | ribbon + dock; upright, the dock opens over the page | the ribbon becomes the drawer's header (its icons switch the drawer's view), the dock the drawer |
| Docks | docked, resizable, collapsible | the same, wider handles | docked sideways, over the page upright | the shell's drawers: left, right; the bottom dock a sheet |
| Arrange | hold W, or right-click | long-press a tab, or the ⋯ | long-press, or the ⋯ | the tab switcher's edit mode |
| Windows | real OS windows | real OS windows | iPadOS windows where the platform gives them | none: a pop-out's tabs are a place of their own in the switcher's menu |
| Screensets | one layout per form class (desk; tablet upright and sideways; phone upright and sideways) | the desk's | the tablet's | the phone's |

## It renders in Blitz

The host is Studio's, generalised: pointer events and `get_client_rect`
for every drag (Blitz has no drag-and-drop events), overlays absolute
against the workspace's root (no `position: fixed`), inline styles from
the shell's theme. Stacked tabs place their panes from the strip's own
scroll offset (`features/workspace/src/layout.rs` `slides`): Blitz has no
horizontal `position: sticky` and cannot scroll a box from code, so the
strip keeps the offset itself, moved by a sideways wheel, its scrollbar or
a press on a spine.

**Motion.** Docks open and fold, tabs make room for a dragged one and
settle where it lands, stacked strips scroll continuously, splits grow in
from the drop's edge and fold back when closed, and the drop highlight
follows the pointer, on one clock a film can drive:
[Motion](@/architecture/motion.md).

**Pop-out windows.** dioxus-native opens one window, at start. The
workspace's launcher (`features/workspace/src/native.rs` `launch`) runs
dioxus-native's application inside its own, which opens more while the
app runs: each pop-out is a Blitz view with its own document and
`VirtualDom`, its root the host's pop-out view
(`features/workspace/src/host/windows.rs` `Popout`), reading the main
window's `Signal<Workspace>`. A signal owned by one dom redraws what reads
it in another, both ways (`features/workspace/tests/windows.rs`). The
contexts a view reads (the shell's, the commands', the app's own through
`Bridge`) are carried into each pop-out's root. A tab dragged out of a
window, or "Move to a new window", opens one where it was let go; dragged
out of a pop-out onto the main window, or "Move to the main window", it
comes back; closing a pop-out brings its tabs back. The layout keeps each
window's place. Without the launcher (a browser) a pop-out kept in a
layout shows as a chip that brings its tabs back.

**A dock's window** is a pop-out whose `center` is `None` and whose dock
on the dock's side holds it (`features/workspace/src/model.rs`
`Workspace::dock_window`, `Workspace::dock`, `WindowId::dock`); the
pop-out's view fills its window with it, ribbon and all. Its grip hears
the pointer itself (in the shell's ribbon it is outside the host's root),
and a release outside the window comes from the platform, as a tab's does
(`WindowSystem::on_release_outside`): from the main window the dock pops
out there, from its own window onto the main one it docks back. A ribbon
command pressed in it runs in the main window's dom (the app's handler
lives there).

## From Studio's dock

| Studio today | Here |
|---|---|
| `Dockspace { center, edges, floats, panels }` | `Window { center, docks }` × windows; `panes` |
| `Node::Split { first, second, share }` (binary) | n-ary splits with shares (the repair keeps it tidy) |
| `Stack { tabs, active, chrome }` | `Group`, plus `stacked` |
| `Float` (inside the window) | a real window; an in-window float stays as an option |
| `Panel { kind, state, link }` | `Pane`, plus `pinned`, `history`, cached `icon` |
| `Screenset { variants: FormClass → Dockspace }` | the same, over `Workspace` |
| `DockRequest` queue polled by Session | Action List actions with explicit targets (`WORKSPACE_OPEN`, `WORKSPACE_MOVE`, …) |
| persisted by Session (`utils::prefs`) | a store the app passes |
| `HostPanels { kinds, scope }` (in Session) | `Views` (here) |

Studio's built-in screensets and kinds stay Studio's; the old `DockLayout`
layer that daw-ui uses retires when daw-ui moves to views.

## Open questions


1. **Pop-out windows in Blitz:** answered. dioxus-native does not open
   a second window itself; the workspace's launcher does, and both
   documents read one app's signals (see "It renders in Blitz").
2. **Docks in pop-outs:** answered for a dock popped out whole (its own
   window, above). A tab's pop-out still has none, as in Obsidian.
3. **Linking:** by a group id (Obsidian) or by a subject the views agree
   on (a song id)? Studio's `link` is an id; a subject is more useful
   to a DAW.
4. **A project's screenset:** stored in the project, or in the person's
   settings keyed by project?
