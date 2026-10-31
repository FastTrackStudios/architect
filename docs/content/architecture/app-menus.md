+++
title = "App menus"
description = "The menu bar (File, Edit, View, …) built from the Action List: native on macOS, the shell's own elsewhere, enabled and ticked as the actions are."
weight = 55
+++

> **Status: proposed (2026-10-31).** For review before any code.

Today every app builds its menus by hand, differently: Studio's macOS
menu is AppKit code with one selector per item and no enabled or ticked
state; Session builds a File/Edit/View tree from its Action List but
serves it only on Linux (dbusmenu); Task has none. An app's menus are a
view of what it can do, and what it can do is its
[Action List](@/architecture/commands.md), so architect builds them from
it.

## What an app says

Which actions go in which menu, in what order, grouped how. The labels,
the shortcuts (from the keymap, [architect-input](@/architecture/input.md)),
whether an item is enabled, and why not, come from the actions.

<!-- doc-check: off (proposed names) -->
```rust
let menus = MenuBar::standard()                       // App, File, Edit, View, Window, Help, with the platform's own items
    .menu(Menus::FILE, |m| m.actions(["project.new", "project.open"]).section(recent_projects).actions(["project.save", "project.save_as"]))
    .menu(Menus::EDIT, |m| m.group("Edit"))           // every action in the "Edit" group, in registry order
    .menu(Menus::VIEW, |m| m.workspace())             // panes, screensets (live list), arrange, sidebars, full screen
    .menu("Transport", |m| m.group("Transport"))
    .settings("app.settings");                        // "Settings… ⌘," in the app menu (macOS) or Edit (elsewhere)
```
<!-- doc-check: on -->

- **A section is live:** `section(recent_projects)` is a function the
  menu calls when it opens (the recent projects, the screensets, the open
  windows). Studio's View menu of screensets is one line.
- **Ticked items:** an action that is a toggle (`toggleable`) shows a
  tick when on. The Action List gains a `checked` beside `enabled`
  (`set_checked(id, Fn() -> bool)`): the one thing it lacks today.
- **Standard items** — Undo/Redo, Cut/Copy/Paste/Select All, Minimize,
  Zoom, Bring All to Front, Hide, Quit, Enter Full Screen — are the
  platform's, wired to the focused view (a text field gets its own
  Copy), and Undo/Redo to the app's undo actions when it has them.
- The workspace's actions ([workspace](@/architecture/workspace.md)) fill
  View and Window without the app listing them.

## Where it shows

| Platform | The menu bar |
|---|---|
| macOS | the native menu bar (AppKit, through objc2 — as Studio's, generated) |
| Linux desktop | exported over dbusmenu where the shell takes it (Session's `bus.rs`), else the in-window bar |
| Windows, Linux without a global menu | an in-window menu bar in the title row, or behind the ⋯ of the app's title (the [app shell](@/architecture/app-shell.md) decides by style) |
| iPad with a keyboard | the iPadOS menu bar (UIMenuBuilder) |
| Phone | none: the same actions are in the palette and each page's ⋯ |

The tree is built once (`Node`: menu, item, separator, section) and each
backend renders it; enabled and ticked states refresh from the Action
List's change events, not on a timer.

## From what exists

- Session's `app_menu::{entries, layout, Node}` (Action List and keymap
  → a menu tree) moves here; its `MENUS` group table becomes the app's
  `MenuBar` declaration.
- Session's dbusmenu server (`bus.rs`) becomes the Linux backend.
- Studio's `mac_menu.rs` is replaced by the macOS backend; its items
  become actions (Audio Settings, Previous/Next Song) or the workspace's
  (screensets, panels, arrange).

## Open questions

1. **The macOS app menu's About / Services / Hide:** the platform's
   defaults, with the app adding Settings and Check for Updates?
2. **Shortcuts the menu shows but the keymap does not bind:** refuse, or
   show them greyed?
