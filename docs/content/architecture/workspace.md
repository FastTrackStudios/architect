+++
title = "The workspace: docking, tabs and screensets"
description = "One layout system for every app: tab groups in splits, docks on each side, stacked tabs, pop-out windows, per-tab history, and screensets — as flexible as Obsidian, REAPER and Premiere, so no app builds docking again."
weight = 54
+++

> **Status: model, desk host and pop-out windows landed (2026-10-31).**
> The model is `architect-workspace` (`features/workspace/src/model.rs`,
> `ops.rs`, `screenset.rs`, `store.rs`); the host is its `ui` feature
> (`features/workspace/src/host/mod.rs` `WorkspaceView`, `View`, `Views`,
> `TitleRow`), developed in the lab's Vault tab
> (`apps/lab/ui/src/vault.rs`). Pop-out OS windows are the `native`
> feature's launcher (`features/workspace/src/native.rs` `launch`); tabs
> reorder within a bar, a finger held on a tab opens its menu, stacked
> tabs slide over each other, ⌘1–9 recall screensets for the screen's
> class, the layout is kept through the app's `Store`, and a pane puts
> its own items on the status line (`use_pane_status`). A dock (the
> sidebar, with its ribbon and groups) pops out into a window of its own
> and docks back (`Workspace::pop_out_dock`, `Workspace::dock_back`; the
> `WORKSPACE_POP_OUT_*_DOCK` and `WORKSPACE_DOCK_BACK_*_DOCK` actions).
> Still to come: dragging a tab onto a group in another window (it joins
> that window's first group), floats in pop-outs, the app's chords inside
> a pop-out. Studio's dock (`studio:features/ui/dock`) was
> the starting point; Obsidian's workspace is the bar to clear.

An app hands architect its **views** (what a pane can show) and gets back
a workspace a person arranges as they like: any number of tab groups,
split any way, docked to any side, stacked, popped out into windows of
their own, and saved as screensets to recall with a key. The app never
lays out a pane, draws a tab, handles a drag or saves a layout.

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
`close`, `set_stacked`, `set_shares`, `pop_out_dock`, `dock_back`, …),
and the host names each one as a **view command**: an
`architect_commands::Command` it registers with `use_command` while it
is mounted (`features/workspace/src/host/commands.rs`
`WorkspaceCommands`, and `features/workspace/src/host/screens.rs` for
the screensets). Those are what the palette (⌘K), the menus and the
keys run, so a menu, a key and the palette arrange the workspace the
same way.

They are not Action List actions one by one. Two actions put the
workspace on the app's [Action List](@/architecture/commands.md), for
scripts, guide scenarios and a model over MCP:

- `WORKSPACE_LAYOUT` (`features/workspace/src/host/actions.rs`
  `use_workspace_actions`, read-only): the arrangement, every window's
  groups and splits, and a `Changed { topic: "workspace" }` event on
  each change;
- `COMMANDS_RUN` (`features/commands/src/bridge.rs`
  `use_command_actions`): runs any command by id, so every command below
  is reachable from a script or MCP through it.

| Operation | Commands | What it does |
|---|---|---|
| `open` | (⌘O, the views) | here, a new tab, a split (side), a new window, or a dock (side) |
| `move_pane` | `WORKSPACE_MOVE_TO_LEFT_DOCK` / `_RIGHT_DOCK` / `_BOTTOM_DOCK` / `_CENTER`, `WORKSPACE_MOVE_TO_NEW_WINDOW`, `WORKSPACE_MOVE_TO_MAIN_WINDOW`, `WORKSPACE_FLOAT_TAB` | to a group (and index), a group's edge, a new window, a dock |
| `close` / `close_others` / `reopen` | `WORKSPACE_CLOSE_TAB`, `WORKSPACE_CLOSE_OTHER_TABS`, `WORKSPACE_REOPEN_TAB` | |
| a split | `WORKSPACE_SPLIT_RIGHT`, `WORKSPACE_SPLIT_DOWN` | split the active group, moving its tab |
| `activate` / `goto_tab` / `cycle_tab` | `WORKSPACE_FOCUS_LEFT` / `_RIGHT` / `_UP` / `_DOWN`, `WORKSPACE_NEXT_TAB`, `WORKSPACE_PREVIOUS_TAB` | |
| `go` | `WORKSPACE_GO_BACK`, `WORKSPACE_GO_FORWARD` | the active pane's history |
| `set_pinned` / `set_stacked` / `set_chrome` | `WORKSPACE_PIN_TAB`, `WORKSPACE_STACK_TABS`, `WORKSPACE_TAB_BAR` | toggles |
| `set_dock_shown` / `set_shares` | `WORKSPACE_TOGGLE_RIGHT_DOCK`, `WORKSPACE_TOGGLE_BOTTOM_DOCK`; dragging a divider | |
| `pop_out_dock` / `dock_back` | `WORKSPACE_POP_OUT_LEFT_DOCK`, `WORKSPACE_DOCK_BACK_LEFT_DOCK`, and the same for `RIGHT` and `BOTTOM` | the dock, ribbon and groups, into a window of its own and back; which of the pair is enabled says where the dock is |
| screensets (`recall`, `save`, `save_as`, `reset`) | `SCREENSET_SAVE`, `SCREENSET_RESET` | |
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
| `DockRequest` queue polled by Session | view commands, run by id (`COMMANDS_RUN` on the Action List) |
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
