+++
title = "Toolbars: the visual editor"
description = "Edit toolbars in place — + chips, drag and drop, action and widget pickers, item and toolbar settings — and every edit as a TOOLBAR_* action, written debounced with rolling backups."
weight = 71
+++

> **Status: landed (architect#313).** The toolbar format and where a
> toolbar goes are `extensions.md` ("Toolbars"); this page is the editor
> on top of it, modelled on Fergler Advanced Toolbars' edit mode (its
> design only: no code or assets were copied).

## Edit mode

**Edit toolbars** turns it on: `TOOLBAR_EDIT` (a toggle, in ⌘K), or the
row of the same name in a toolbar's context menu (target `toolbar`). Every
toolbar on show then draws as `features/extensions/src/toolbar_ui.rs`
`ToolbarStrip` does in edit mode:

- a **⚙** chip at its start opens the toolbar's settings;
- a **+** chip before each item and at its end opens the insert menu
  (`features/extensions/src/toolbar_editor.rs` `InsertMenu`): an
  **Action…** (the action picker), a **Separator**, a **Widget…** (the
  widget picker) or a **Dropdown** (a button with a menu, its settings
  opened to fill it);
- an item pressed opens its settings; right-clicked, the `toolbar-item`
  context menu (Settings…, Group with the next, Ungroup, Remove, and what
  extensions add for the target);
- an item dragged moves: a button into or out of a group (dropped over a
  group, between its buttons), a whole group by its label (its grip),
  into another toolbar on show. A line shows where it goes; Escape, or a
  drop off every toolbar, leaves it where it was. Where it lands is
  `toolbar_editor.rs` `drop_slot`, pure geometry over the toolbars as
  they were measured when the drag began.

A bar over the window says the editor is on, with **New toolbar** and
**Done** (which writes what is pending and turns it off). It is all drawn
by `toolbar_editor.rs` `ToolbarEditorLayer`, which `ExtensionOverlays`
mounts; the toolbars in one window share `toolbar_editor.rs` `Editor`.

## The pickers

The **action picker** searches the Action List with the palette's own
search (`features/palette/src/rank.rs` `rank`, over
`architect_commands::Commands::all`), so an action is found as ⌘K finds
it. It fills a new button, a dropdown's next row, or a button's
right-click.

The **widget picker** lists every loaded extension's views by extension,
the category; picking one draws it live at a widget's size
(`ExtensionView`) before **Insert** puts it in. The sandbox's Board has a
small one for this, its tempo (`apps/sandbox/board/src/lib.rs`
`TempoWidget`, view `lab.board/tempo`).

## Settings

An item's (`toolbar_editor.rs` `ItemSettings`): its label, its icon (a
Lucide name) or an **image file** (`ToolItem::image`: a PNG, SVG, JPEG,
GIF or WebP, relative to the person's toolbar folder or the extension's
package, drawn as a `data:` URI by `features/extensions/src/toolbars.rs`
`Toolbars::image`), whether its label shows, its colours as theme
tokens (fill, ink, fill while on), its right-click (nothing, its menu,
arm, or an action), a dropdown's rows (up, out, an action, a heading, a
separator), a widget's width, and whether it starts a new row.

A toolbar's (`toolbar_editor.rs` `ToolbarSettings`): its title, where it
is, its button size, spacing and rounding and whether labels show
(`ToolbarStyle`), a colour preset (`toolbar.rs` `PRESETS`, `preset`: the
colours every button starts from, a button's own over them,
`ToolColors::over`), the context it is for (`ToolbarDecl::when`:
`view:note`, `editor:markdown`; `ToolbarDecl::applies_to`; a place that
follows the focus reads it), **show another here instead** (which
toolbar a place shows: `Toolbars::show`, the one there hidden), and
**Duplicate** and **Delete** (for an extension's: reset to it).

A dropdown near the window's edge moves left to stay in it, and opens
upward near its bottom (`toolbar_editor.rs` `Kept`): a toolbar in the
right dock no longer has its menu cut off.

## Writing

An edit is `Toolbars::edit` with one of `features/extensions/src/toolbar_edit.rs`'s
operations (`insert`, `remove`, `move_item`, `set`, `configure`, `group`,
`ungroup`), an item named by its place (`toolbar_edit.rs` `At`: `3`, or
`2.1` inside the group at 2). It shows at once, checked as a person's file
is (it may be empty), and is written `toolbars.rs` `DEBOUNCE` after the
last one (`Toolbars::flush_if`). Every write keeps the file it replaces:
the last `BACKUPS` of each in `.backups/` beside them
(`Toolbars::backups`, `TOOLBAR_BACKUPS`); deleting keeps it too. Editing
an extension's toolbar makes the person's copy of it.

## The same edits as actions

Every edit is on the Action List, one item and one field at a time, so an
agent edits a toolbar without rewriting it
(`features/extensions/src/toolbar_actions.rs`). Each writes at once. Run
from an item's context menu (target `toolbar-item`, id `<key>#<at>`) or a
toolbar's (`toolbar`, its key), one needs no arguments.

| action | what it does |
|---|---|
| `TOOLBAR_EDIT` | the editor on or off (`{on}`; a toggle) |
| `TOOLBAR_SELECT` | shows the settings of `{id, at}` or `{id}`, or `{close: true}` |
| `TOOLBAR_NEW` | a new toolbar of the person's own, empty: `{title, where, id}` |
| `TOOLBAR_RENAME` | `{id, title}` |
| `TOOLBAR_DUPLICATE` | `{id, to, title}`: a copy of the person's own, hidden |
| `TOOLBAR_DELETE` | `{id}`: a person's own goes; their copy of an extension's goes |
| `TOOLBAR_SHOW` | `{id, where}`: shown there, whichever was there hidden |
| `TOOLBAR_INSERT` | `{id, at, item}`: an item as `TOOLBAR_GET` shows one |
| `TOOLBAR_REMOVE` | `{id, at}` |
| `TOOLBAR_MOVE` | `{id, from, to, into}`: `into` another toolbar |
| `TOOLBAR_SET` | `{id, at, set}`: an item's fields; no `at`, the toolbar's |
| `TOOLBAR_GROUP` / `TOOLBAR_UNGROUP` | `{id, at, count, label}` / `{id, at}` |
| `TOOLBAR_BACKUPS` | `{id}`: its backups, the newest first |

## The Typing Toolbar

A phone's bar over the keyboard is a toolbar place, `typing`
(`app-shell.md`, "The action bar and the keyboard bar"). Each editor
context's own items are a toolbar, `typing/<context>` (the capture
field's: `typing/capture`), so the editor edits them as it edits any:
move one (`TOOLBAR_MOVE {id: "typing/capture", from: 8, to: 0}`), take
one away (`TOOLBAR_REMOVE`), add an action, a dropdown or a widget
(`TOOLBAR_INSERT`), each per context. The person's copy is
`toolbars/typing-<context>.toml`, used in place of the editor's own until
`TOOLBAR_RESET`. With **Edit toolbars** on, the Typing Toolbar shows its
toolbars' strips above itself, with their + chips and ⚙. An extension's
typing toolbar is edited the same way; **show another here instead**
switches only the toolbar for the same context (`Toolbars::show`).
A position (`at`, `from`, `to`) may be a number as well as `"2.1"`.

## Tests

- `apps/sandbox/media/tests/toolbar_editor.rs`: a toolbar built from empty
  through the UI (an action found by search, a second, a dropdown with a
  row, the tempo widget previewed; grouped from the context menu, a
  button dragged into the group, a drag cancelled with Escape, the group
  dragged past the widget; restyled: label, image icon, fill, rounding,
  preset), the same after a restart, its dropdown kept in the window, and
  edit mode, each against a golden.
- `apps/sandbox/media/tests/typing_toolbar.rs`: the Typing Toolbar on a
  phone's screen, each against a golden: the capture field's own items
  (unchanged from the bar before), the Board's chart's chord line (its sharps
  toggle on, the field keeping the focus through each press, the bar
  scrolled to its dropdown, opened above it, and the tempo widget), a
  person's edit read again after a restart, and the editor's strips above
  the bar.
- `features/extensions/tests/toolbars.rs`
  `a_toolbar_is_built_from_empty_through_its_actions_and_survives_a_restart`:
  the same through `TOOLBAR_*`.
- `features/extensions/src/toolbars.rs` unit tests: a debounced write,
  backups rotating, switching a place, an item between toolbars, image
  icons; `toolbar_edit.rs`: every operation; `toolbar_editor.rs`: where
  a drop lands, a menu kept in the window.
