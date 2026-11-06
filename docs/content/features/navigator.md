+++
title = "Navigator"
description = "Navigator: folders beside notes in the sidebar, a port of Notebook Navigator (GPL-3.0). A built-in extension over architect-navigator: groups, pins, images, tags, folder notes, a manual order, each folder's and tag's own list."
weight = 55
+++

## What it is and when to use it

Navigator is the sidebar of an app that keeps notes in a vault: a folder
tree beside a list of the selected folder's notes. It is a port of
[Notebook Navigator](https://github.com/johansan/notebook-navigator), the
Obsidian plugin by Johan Sanneblad, ported spec for spec. Use it when an
app shows a vault in a sidebar (Task, the lab's Vault tab).

It comes in two parts:

- **The navigator**, `architect-navigator` (`features/navigator`). It holds
  the model, which is plain data and builds for wasm. Its `ui` feature adds
  the Dioxus view. It reads a vault through `NavigatorSource`.
- **Navigator**, the built-in extension, `architect-navigator-extension`
  (`features/navigator/extension`). It packages the navigator as a feature,
  as every user-facing feature is (see
  [Extensions](@/architecture/extensions.md)). Its manifest declares its
  sidebar view, its `NAVIGATOR_*` actions, its rows on a note's context menu
  and its settings section, "Navigator". An app links it in, and
  `EXT_CLONE {"id":"navigator"}` makes a development copy that runs as a
  process.

**Licence: GPL.** Both crates are GPL-3.0-or-later, because they carry
Notebook Navigator's design and code (`features/navigator/LICENSE`,
`features/navigator/README.md`). They are the only GPL crates in architect.
An app that links either one ships under the GPL. Nothing else in architect
depends on them, and a tool or server that leaves them out stays MIT OR
Apache-2.0.

## A minimal example

The app makes the navigator over its vault, puts it in context, and
mounts Navigator's view in its left dock:

```rust,ignore
// At startup: the built-in extension, linked in.
let extensions = extensions.with_linked(&architect_navigator_extension::NavigatorExtension)?;

// The left dock's view.
let nav = architect_navigator::ui::use_navigator(|| Arc::new(source));
use_context_provider(|| Signal::new(architect_navigator_extension::NavigatorMount {
    navigator: nav,
    on_open,            // opens `Open::path` in the workspace, by `Open::how`
    on_context_menu: Some(on_context_menu),
    on_focus_editor: None,
    phone,
    columns,            // the shell sidebar's: one pane, or folders beside notes
    on_new_note,        // New note in the list's header, given the folder on show
}));
rsx! { ExtensionView { ui, extension: "navigator".into(), view: "files".into(), bare: true } }
```

An app without extensions mounts `architect_navigator::ui::NavigatorView`
directly. Its palette commands are then the view's own.

## Code tour

- `features/navigator/src/source.rs`: `NavigatorSource`, `NoteFile`
  (path, times, properties, tags, preview, feature image, tasks, words),
  `SourceEvent`, `FileOp` and `MemorySource`. A source that can be written
  implements `NavigatorSource::write`, which folder notes and the manual
  order use. `NavigatorSource::resource_url` gives the view a URL for an
  image in the vault.
- `features/navigator/src/list.rs`: `rows` builds the list. It takes a
  folder's or a tag's notes (`files_of`), leaves out folder notes and
  drawings' previews, sorts them (`sort`), puts the pinned ones first, and
  groups them. Each row carries its `Lines` and its `Thumb`, so `height` is
  known before it is drawn.
- `features/navigator/src/grouping.rs`: `by_value`, grouping by a
  property's value as Bases does, with "None" last. It knows nothing of
  notes.
- `features/navigator/src/appearance.rs`: `Appearance`, a folder's or a
  tag's own list. `Appearance::over` applies it to the settings.
- `features/navigator/src/state.rs`: `NavigatorState` holds everything.
  Pins: `toggle_pin`, `pin_folder_notes`. Groups: `toggle_group`. Own
  lists: `set_appearance`, `edit_appearance`. Tags: `select_tag`. Folder
  notes: `convert_to_folder_note`, `set_as_folder_note`,
  `detach_folder_note`, `open_folder_note`. The manual order:
  `move_in_order`. `ViewState` keeps pins, folded groups, own lists and
  colours between runs.
- `features/navigator/src/period.rs`: `Period`, a day, week, month,
  quarter or year, and `WeekRules`, Moment's weeks (ISO, or from Sunday or
  Saturday). `Period::filter_token` and `DateFilter::parse` are the search's
  date filters (`@2026-11`, `@2026-Q4`, `@c:2026-11-01..2026-11-07`,
  `@today`).
- `features/navigator/src/pattern.rs`: `Pattern`, the part of Moment's date
  format a note's path uses, both ways: `format`, and `parse`.
- `features/navigator/src/calendar.rs`: `PeriodicNotes` turns the settings'
  folder and patterns into paths (`path`) and paths back into periods
  (`period_of`, checked by writing the path again). `grid` is the calendar
  the view draws: weeks of days, each with its note, image and tasks left.
  `NavigatorState::open_period` opens a note, or returns a `FileOp::CreateFile`
  that makes it. `open_adjacent_period` and `open_parent_period` move from
  the note open.
- `features/navigator/src/ui/calendar.rs`: the calendar at the foot of a
  pane: the month, quarter and year with ‹ Today › beside them, then the
  weeks.
- `features/navigator/src/ui/panes.rs`: the panes. It draws the list's
  header (sort, group and "Only this folder" menu), group headings, and the
  note rows with their image, tags, properties and task bar.
- `features/navigator/extension/src/lib.rs`: `NavigatorExtension` and its
  view, `FilesView`. With no `NavigatorMount` in context, as in its process
  copy, the view draws a sample vault. `ACTIONS` lists its actions.
  `features/navigator/extension/src/bridge.rs` carries each action to the
  view on show and brings back its answer.

## Tests to read

- `features/navigator/tests/calendar.rs`: the calendar and the periodic
  notes. It covers Moment's tokens and weeks, the default and the demo
  vault's paths read back, kinds turned off, the grid, opening and making
  notes, next, previous and parent periods, and the date filters.
- `features/navigator/tests/list.rs`: the list pane in full. It covers each
  grouping, folded groups, pins, compact rows, images and badges, a row's
  items, a tag's list, frontmatter names and dates, the manual order and its
  keys, folder notes, and own lists.
- `features/navigator/tests/model.rs`, `features/navigator/tests/scale.rs`
  (100,000 notes) and `features/navigator/tests/view.rs`: layer 1.
- `features/navigator/extension/tests/both_ways.rs`: the manifest declares
  what the extension builds, and its section is `NavigatorSettings` as it
  is. Linked in, it loads and answers.
- `features/navigator/extension/tests/dev_loop.rs`: `EXT_CLONE`, a rebuild
  on save, and back to the built-in. It is ignored by default: run it with
  `--ignored`.

## Status

Notebook Navigator's parts are ported in layers (#332):

- **Layer 1, done (#334).** The crate, `NavigatorSource`, the dual pane,
  the folder tree, virtual scrolling to 100,000 notes, list basics (sort,
  date groups, multi-select) and keys across both panes.
- **Layer 2, done (#335): the list pane in full.**
  - Grouping by date, by folder and by a property's value, with groups that
    fold.
  - A pinned section, and compact rows.
  - Feature images, with thumbnails for SVG and drawings and badges for
    PDF, canvas and base files.
  - Tags you can click, task progress, properties and word counts.
  - Names and dates from frontmatter.
  - A manual order, moved with ⌘↑ and ⌘↓.
  - Notes from the folders below.
  - Folder notes: convert, set, detach, pin all, open in a new tab.
  - Each folder's and tag's own sort, grouping, rows, compact mode and
    notes from below.
  - Navigator as a built-in extension.
- **Layer 3, done (#336): the calendar and periodic notes.**
  - A calendar of daily notes under the folders (under the notes when the
    panes are stacked): dots, feature images, tasks left, today, and the
    note open ringed. It shows one to six weeks.
  - Daily, weekly, monthly, quarterly and yearly notes, each named by a
    Moment pattern under one folder, with Notebook Navigator's defaults.
    A click, or an action, opens one or makes it, asking first.
  - The next, previous and parent periods (⌃⌥→, ⌃⌥←, ⌃⌥↑).
  - The period model the search's date filters use.
  - Not ported: the right sidebar's year panel, the Daily Notes core
    plug-in as a source, month highlights, hover previews, and locales
    other than English (weeks start on Sunday, Monday or Saturday).
    Templates come with #339: until then a new note gets a heading.
- **Still to come.**
  - The real vault as a source, with real editing (#353).
  - Tags and properties in the navigation pane. A tag is selected today
    from a row.
  - Search, and the shortcuts pane.
  - Rendered PDF covers. A source gives a cover as the note's
    `feature_image`; without one, a PDF shows its badge.
  - Notebook Navigator's `custom` grouping (header notes in a manual sort),
    and character counts.
