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
app shows a vault in a sidebar (Task, the sandbox's Vault tab).

It comes in two parts:

- **The navigator**, `architect-navigator` (`features/navigator`). It holds
  the model, which is plain data (native: the list groups with Bases'
  `group_by` from `vault-live`). Its `ui` feature adds
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
- `features/navigator/src/list.rs` `property_groups`: grouping by a
  property's value with Bases' `group_by`
  (`features/vault/vault-proto/src/grouping.rs`, MIT, the one grouping in
  architect), with "None" last.
- `features/navigator/src/tags.rs`: `TagTree`, the tags nested by `/` with
  their counts, sort and hand-made root order; `features/navigator/src/properties.rs`:
  `PropertyTree`, the keys named in the settings and their values;
  `features/navigator/src/tree.rs` `tag_rows` and `property_rows` draw them
  under the folders. `features/navigator/src/query.rs`: `Query`, the
  search a ⌘/Ctrl-click builds (tags and properties, AND and OR).
- `features/navigator/src/search.rs`: `FilterSearch`, the search box's
  filter search, read (`parse`) and matched (`matches`): names and
  aliases, tags, properties, dates, folders, extensions and tasks, with
  exclusions, quoted text, and AND and OR between tags and properties.
- `features/navigator/src/hidden.rs`: `Hidden`, a profile's hiding
  compiled: folder, tag, file name and frontmatter patterns, file
  visibility, and the folders kept out of the lists above them.
- `features/navigator/src/profiles.rs`: `Profiles` and `VaultProfile`,
  each with its hiding, banner, `Shortcut`s and recent notes.
- `features/navigator/src/template.rs`: the built-in template engine
  (`render`, `prompts`), numbered names (`numbered_name`, `next_number`)
  and `Templates`: folder templates and create-note commands.
- `features/navigator/src/state/files.rs`: the file operations on
  `NavigatorState` (`new_note`, `new_folder`, `rename_file`,
  `rename_folder`, `move_files`, `copy_files`, `move_folder`,
  `duplicate`, `trash`, `merge`, `open_all`) and `check_name`. On disk
  they go through `OpenVault`'s `copy_file`, `move_folder`, `copy_folder`
  and `trash` (`features/vault/vault/src/actions/edit.rs`, MIT).
- `features/navigator/src/appearance.rs`: `Appearance`, a folder's or a
  tag's own list. `Appearance::over` applies it to the settings.
- `features/navigator/src/state.rs`: `NavigatorState` holds everything.
  Pins: `toggle_pin`, `pin_folder_notes`. Groups: `toggle_group`. Own
  lists: `set_appearance`, `edit_appearance`. Tags: `select_tag`,
  `click_tag`, `add_tag`, `remove_tag`, `clear_tags`, `rename_tag`,
  `delete_tag`, `new_note_in_tag`, `move_tag_root`. Properties:
  `click_property`, `set_property`. Drops: `drop_on`. The search:
  `set_search`, `toggle_search`, `search_vault`, `set_full_text`, and
  `query`, `toggle_query`, `set_query`. Profiles and hiding:
  `select_profile`, `add_profile`, `edit_profile`, `toggle_hidden`, `hide`.
  Shortcuts and recent notes: `add_shortcut`, `move_shortcut`,
  `open_shortcut`, `open_recent`, `toggle_section`. Folder
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

- `features/navigator/tests/files.rs`: notes, canvases and bases made,
  numbered, from folder templates and create-note commands with prompts;
  periodic notes from templates; renames, moves, copies, duplicates, the
  trash and merging; drops on folders and among the shortcuts.
  `features/vault/vault/tests/edit.rs` does the same on disk.

- `features/navigator/tests/search.rs`: the filter search, term by term,
  the box and ⌘/Ctrl-clicks writing to it, kept across runs, the
  vault-wide search, and the full text through the source.
- `features/navigator/tests/profiles.rs`: hiding by pattern, frontmatter,
  tag and file type, show hidden, folders kept out of the lists above,
  profiles switched and kept, shortcuts and recent notes.

- `features/navigator/tests/tags_properties.rs`: the tag tree and the
  property browser, their counts, sorts and orders, the tag operations and
  drops, a rename's colour and selection following it, and the search a
  ⌘/Ctrl-click builds. `features/navigator/tests/vault_source.rs` writes
  tags to markdown on disk; `features/navigator/tests/view.rs` drags a note
  onto a tag.

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
- **Layer 4, done (#337): tags and properties.**
  - The tag tree under the folders: nested tags, a "Tags" row and
    "Untagged", A to Z or by frequency, a hand-made root order, optionally
    the selection's tags alone.
  - Add, remove and clear tags on the notes selected; rename and delete a
    tag, with the tags below it; a new note in a tag; notes dragged onto a
    tag (or Untagged) and a tag onto a tag (or the Tags row). The vault's
    markdown changes, frontmatter and inline `#tags`.
  - The property browser: the keys named in the settings, their values
    and counts, colours and icons; a property set on the notes selected or
    dropped.
  - ⌘/Ctrl-click puts a tag or property into the search with AND, with ⇧
    with OR (`Query`).
  - Not ported: tag and property icons
    chosen by a person and the shortcuts pane (#339), a key's or value's
    rename, child orders by hand (`tagChildOrder`, `propertyChildOrder`).
    Spring-loaded folders came with layer 6.
- **Layer 5, done (#338): search, shortcuts, recents, profiles and
  hiding.**
  - The filter search in a box over the list: names and aliases, tags,
    properties, dates (relative and absolute, created or modified),
    folders, extensions and tasks; AND and OR, exclusions, quoted text. It
    is kept across runs; "Search the whole vault" searches from the root.
    The full-text provider searches the notes' text through architect's
    own index.
  - Shortcuts to notes, folders, tags, properties and saved searches over
    the folders, numbered, reordered, opened with commands 1 to 9.
  - Recent notes, per profile.
  - Vault profiles, each with its hidden items, file visibility, banner
    and shortcuts; switched from the header or with commands.
  - Hiding folders, tags, notes and files by pattern, frontmatter or tag,
    with a show-hidden toggle; folders kept out of the lists above them
    and still browsed.
  - Not ported: Omnisearch (the full text is architect's), a profile's
    own property keys and periodic notes folder, the profile editor in
    the settings (profiles are edited with `NAVIGATOR_PROFILE`), pinning
    recent notes with the shortcuts, and the navigation rainbow. The
    search's highlighting and dragging shortcuts came with layer 6.
- **Layer 6 (#339): file operations and templates, done.**
  - New notes, folders, canvases and bases from a folder's menu, the
    list's header and the palette; inline rename with warnings for names
    that cannot be or break links; duplicate, move, copy, the trash and
    merging; "Open all notes", asking at 15 or more.
  - Drag and drop: notes and folders onto folders (copied with ⌥ on a
    Mac, Ctrl elsewhere), folders that open while a drag is held over
    them, and shortcuts dragged to reorder them or things dropped among
    them to add them.
  - Templates: the built-in engine with title, folder, path, dates with
    offsets, weekdays, time, now, cursor, prompts and numbering; folder
    templates inherited by subfolders; create-note commands with a name
    format, a template and a location; periodic note templates.
  - The search's words marked in names, and its tags in the tree.
  - Not ported: Templater (`<%` commands are left as written), a template
    picker (the template is typed as a vault path), the ribbon and tab-bar
    buttons of create-note commands, Excalidraw drawings made from the
    menu, version history and "reveal in Finder". A new base is
    Obsidian's YAML, not Notebook Navigator's legacy database JSON. The
    app does not yet place the editor's cursor at `Open::cursor`.
- **Still to come.**
  - Rendered PDF covers. A source gives a cover as the note's
    `feature_image`; without one, a PDF shows its badge.
  - Notebook Navigator's `custom` grouping (header notes in a manual sort),
    and character counts.
