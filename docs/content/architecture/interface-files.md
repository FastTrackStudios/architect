+++
title = "The interface as files, the chrome as slots"
description = "The shell's regions are slots a view can fill, and the window — its slots, its look, its arrangement, its keys — is files under the app's folder that an agent or a person edits live, a broken one keeping its last good version."
weight = 70
+++

> **Status: landed (2026-11-03).** The shell's slots:
> `features/shell/src/shell/chrome.rs` `Chrome`, `Slot`, `Filler`. The
> files: `features/workspace/src/home.rs` `Home` (feature `home`), its
> actions (`home::actions`) and its window glue (`home::ui`). The lab
> uses both (`apps/lab/ui/src/lib.rs` `LabApp`). Not yet: the `.state/`
> of sidebar widths and folds (the shell keeps them in memory), a
> `layout.switch` key, a generated guide (#268 H2), the escape-hatch key
> (H7), cloning a built-in (`WORKSPACES_CLONE`, on its own branch).

herdr's desktop app keeps its whole interface in files an agent can
edit, and draws its own sidebar, tab bar and status bar with the same
parts an agent has: nothing in its default interface is special. This
is architect's version, over the shell and the workspace it already has.

## The chrome is slots

The shell's regions are named slots (`Slot`):

| slot | what the shell draws there |
|---|---|
| `ribbon` | a desk's ribbon at the window's left edge (`DeskRibbon`) |
| `left` | the left sidebar's content (the app's `left`, or a page's) |
| `right` | the right side's content |
| `tabs` | the page's tab in a desk's title row (`DeskTab`) |
| `status` | a desk's status bar (`StatusBar`) |
| `title` | a desk's title row (`DeskTitleRow`) |

A `Chrome` above `AppShell` (`use_chrome`) says, per slot, who fills it
instead of the shell (a `Filler`: an extension's view, a layout file's
assignment), whether it is hidden, whether the shell's own is kept over
whatever fills it under (`own`), and a slot's own look (token
overrides). With nothing said, the shell draws its own components —
they are what a slot has when nobody else fills it.
`features/shell/src/shell/snapshot_tests.rs` records every mutation the
shell writes on a desk and a phone and compares it with a recording
from before the regions were slots (`default_chrome.golden`): byte for
byte, with no chrome and with an empty one.

`Chrome` also carries what others add beside the app's own tabs, status
items and bar buttons. An extension's tables arrive there through
`features/extensions/src/interface.rs` `ExtensionUi::chrome`; a layout
file's through `home::ui::chrome_of`. `Chrome::over` puts one over the
other: the layout's word beats an extension's claim.

**The look cascades.** The window's look is the theme store's workspace
layer (`architect_theme::LayerKind::Workspace`); a slot's tokens are
resolved over it (`ThemeStore::resolve_with`), so the components in that
slot read their own `ShellTheme`. A person's pick (`THEME_SET`) is above
both.

## The files

Under the app's folder (the lab's: `architect::settings::app_dir`):

| file | what it decides | applied |
|---|---|---|
| `layouts/<name>.json` | a layout: its slots, its look, its arrangement | live |
| `layouts/.state/<name>.json` | what the person (or an agent, `HOME_SLOT_SET`) changed by hand | live; written by the app |
| `layouts/.state/active` | the layout shown | written by the app |
| `keys.json` | the person's keymap layer (`architect_commands::PersonKeymap`: rebinds, custom actions) | live |
| `layouts/<name>.error`, `keys.error` | why that file does not read | written by the app, gone when it does |

A layout:

```json
{
  "name": "Board",
  "look": { "tokens": { "brand": "#ec4899" } },
  "chrome": {
    "ribbon": { "hidden": true },
    "status": { "view": "lab.board/status-bar" },
    "left":   { "tokens": { "ink": "#fde68a" } }
  },
  "workspace": { "windows": [ … ] }
}
```

- `look`: `theme` (an id) and `tokens`, as the workspace layer.
- `chrome`: per slot, `view` (an extension's `<extension>/<view>`, or
  `shell/<slot>` for the shell's own), `hidden`, `tokens`. A slot that
  is not one of the six is refused.
- `workspace`: `WORKSPACE_EXPORT`'s portable tree, put in place with
  `WORKSPACE_APPLY` when the layout shows or the tree changes.
- Unknown fields are refused.

`Home::poll` reads what changed (the lab every 400 ms). A file that does
not read keeps its last good version and gets `<name>.error` beside it;
each such error is a `home::Problem`. `home::actions::register` makes the
files a source of the list's problems, so `APP_STATUS` lists a broken
layout (source `layout`) or `keys.json` (source `keymap`) with the file's
full path beside every other problem the app has; `HOME_PROBLEMS` answers
the same entries for the files alone (`features/workspace/src/home.rs`
`Home::app_problems`). The app's own layouts (`Home::open`'s
built-ins; the lab's `main` is the shell as it is) are written read-only
on every start: an edit to one is a problem saying to clone it, and an
edited copy found at start is kept as `<name>-edited.json`.

What the person changes by hand is kept in `.state/`, not in the file
someone wrote; a deliberate edit of the file that names the same slot
beats it (that slot's hand change is dropped).

| action | what it does |
|---|---|
| `HOME_LAYOUTS` | every layout (name, built in, shown), the folder, the problems |
| `HOME_LAYOUT_USE` | shows `{name}` |
| `HOME_SLOT_SET` | `{slot, view?, hidden?, tokens?}` or `{slot, clear: true}`, into `.state/` |
| `HOME_PROBLEMS` | what is wrong with the files (the same as `APP_STATUS`'s `layout` and `keymap` problems) |

## An agent changing the window

1. `HOME_LAYOUTS` for the folder and what shows.
2. Write `layouts/<mine>.json` (never a built-in), then `HOME_LAYOUT_USE`.
3. Read `APP_STATUS` (or `HOME_PROBLEMS`, or the `.error` beside the
   file) after each save: a broken file changes nothing.
4. `EXT_CONTRIBUTIONS` for which views extensions offer to put in a
   slot.
