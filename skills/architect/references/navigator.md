# The navigator (`architect-navigator`)

A port of Notebook Navigator (the Obsidian plugin) as architect's sidebar:
a folder tree beside a list of notes. The crate is `features/navigator`.
Navigator, the built-in extension that makes it a feature, is
`features/navigator/extension` (`architect-navigator-extension`).

**Licence.** `architect-navigator` is GPL-3.0-or-later. With Navigator,
the extension that links it, it is the only GPL code in architect (`features/navigator/README.md`, `features/navigator/LICENSE`).
An app that links it ships under the GPL. Nothing else in architect depends
on it. Never move its code into another crate. A headless tool or a server
that does not link it stays MIT OR Apache-2.0.

## Rules

- The navigator reads a vault only through `NavigatorSource`
  (`features/navigator/src/source.rs`). A vault becomes a source through an
  adapter in this crate, not by teaching the navigator its types.
- The navigator does not open notes. `NavigatorView`'s `on_open` asks the
  app, and the app opens the note in its workspace (`Target::Here`, `Tab`,
  `Split`, by `OpenIn`). `Open::browsing` says the arrow keys moved to
  the note; show it in place rather than open one more tab. The app tells it which note is on show with
  `Navigator::set_active`, which lights the note and reveals it.
- The model is plain data (`features/navigator/src/state.rs`
  `NavigatorState`). Test behaviour against the model
  (`features/navigator/tests/model.rs`). Test the drawing in a headless
  Blitz document (`features/navigator/tests/view.rs`).
- Rows are virtualized (`features/navigator/src/virtualize.rs` `Offsets`):
  only the window is drawn, and the scroll belongs to the state, not to a
  box. Keep it that way. 100,000 notes are tested
  (`features/navigator/tests/scale.rs`).
- Colours come from the theme roles `navigator.*`
  (`features/navigator/src/ui/theme.rs` `TOKENS`). Add them to the app's
  `App` layer with `with_tokens`. Don't use literals.

## The source

```rust,ignore
pub trait NavigatorSource: Send + Sync {
    fn name(&self) -> String;                 // the vault's name: the root row
    fn folders(&self) -> Vec<String>;         // every folder path, the root ("") left out
    fn files(&self) -> Vec<NoteFile>;         // every file
    fn subscribe(&self, listener: Listener) -> Subscription; // changes, until dropped
    // default methods: resource_url, write, search_text (full text; None by default)
}
```

- `NoteFile` has `path` (`Projects/Plan.md`), `created`, `modified`
  (`Millis`), `properties` (`PropertyValue`), `tags`, `preview`,
  `feature_image` and `tasks` (`TaskCounts`).
- `None` means "not worked out yet". Send a `SourceEvent::FileModified`
  when the value arrives.
- `NavigatorSource::write` does `FileOp`s: `CreateFolder`, `CreateFile`
  (a periodic note made), `Move`, `SetProperty`. A source that can write
  implements it; the default is read-only.
- `SourceEvent` has these variants: `FolderCreated`, `FolderDeleted`,
  `FolderRenamed`, `FileCreated`, `FileModified`, `FileDeleted`,
  `FileRenamed` and `Reset` (read everything again).
- `MemorySource` keeps a vault in memory. `MemorySource::synthetic` makes
  a large one for load tests.
- `VaultSource` (feature `vault`, native; `features/navigator/src/vault_source.rs`)
  is architect's vault (`vault::actions::OpenVault`): folders (empty ones
  too), notes with their properties, tags, preview, word and task counts
  and feature image (the vault path of the first of
  `FEATURE_IMAGE_PROPERTIES` naming an image, else of the first image
  embedded), bases and attachments. `resource_url` gives a vault path (or
  an embed's bare name) as a `file:` URL the desk loads; `write` does the
  navigator's `FileOp`s on disk (a folder made, any file moved, a note's
  through `rename` so backlinks follow, a property set). Its `save`,
  `create_note` and `rename` say what changed at once; `watch` hears edits
  made outside the app. `search_text` is the vault's full-text search
  (`OpenVault::search_text`, over `vault_live::sync::search_pages`, MIT).
  `note_from_text` derives a `NoteFile` the same way
  from a text you hold (the sandbox's in-memory vault reads the demo vault's
  files with it).
  Tests: `features/navigator/tests/vault_source.rs`.

## Recipe: Navigator, the extension, as a sidebar

A user-facing feature is a linked extension, so an app links Navigator in
and mounts its view (`features/navigator/extension/src/lib.rs`
`NavigatorExtension`, `NavigatorMount`, `FilesView`):

```rust,ignore
let extensions = extensions.with_linked(&architect_navigator_extension::NavigatorExtension)?;
// The dock's view: the app's navigator, in context, drawn by Navigator.
let nav = use_navigator(|| Arc::new(source));
use_context_provider(|| Signal::new(NavigatorMount {
    navigator: nav, on_open, on_context_menu: Some(menu), on_focus_editor: None, phone,
    // The shell sidebar's columns (`SidebarLayout::columns`), and New note
    // in the list's header (given the folder on show); both optional.
    columns: Some(shell.sidebar_layout().columns()), on_new_note: Some(new_note),
}));
rsx! { ExtensionView { ui, extension: "navigator".into(), view: "files".into(), bare: true } }
```

- Its settings section is the extension's (`[[sections]]` in
  `features/navigator/extension/extension.toml`, the keys of
  `NavigatorSettings`). Don't register `NavigatorSettings::section()` in the
  app as well. When you add a field to `NavigatorSettings`, run
  `features/navigator/extension/tests/both_ways.rs`
  `its_section_is_navigator_settings`. It prints the section to paste.
- Its `NAVIGATOR_*` actions are on the Action List, the CLI and MCP. They
  answer with `NAVIGATOR_STATE`'s shape. With no navigator on show they
  fail and say so. `NavigatorView` drops its own palette commands under
  the extension (`commands: false`).
- `EXT_CLONE {"id":"navigator"}` makes a development copy that runs as a
  process. Without the app's `NavigatorMount`, it draws a sample vault
  (`sample`).

The sandbox's Vault tab does this (`apps/ui/src/vault.rs` `FilesView`,
`apps/ui/src/extensions.rs`).

## Recipe: the navigator without extensions

```rust,ignore
// Where the page lives, so the dock's view can come and go:
let nav = architect_navigator::ui::use_navigator(|| Arc::new(source));
use_context_provider(|| nav);
// The dock's view:
rsx! { NavigatorView {
    navigator: nav,
    phone,
    on_open: move |o: Open| { /* workspace.open(note for o.path, o.how) */ },
    on_context_menu: move |(path, e)| pane.context_menu("file", path, &e),
    // Optional: the shell sidebar's columns (`SidebarLayout::columns`):
    // one is one pane at a time, two is folders beside notes.
    columns: Some(shell.sidebar_layout().columns()),
    // Optional: New note in the list's header, given the folder on show.
    on_new_note: move |folder: String| { /* make a note there, open it */ },
} }
// When the note on show changes:
nav.set_active(Some(path));
```

The sandbox's Vault tab does exactly this (`apps/ui/src/vault.rs`
`FilesView`: a `VaultSource` over the demo vault, `lab_vault` for a
film's in-memory one).

## What it does

- **Panes.**
  - Side by side or stacked (`NavigatorSettings::orientation`). The folders'
    pane is 200 wide by default and resizable.
  - One pane at a time on a phone, or when two panes are turned off.
  - Two side-by-side panes in a sidebar narrower than the folders plus 150
    become one pane at a time (or stacked), by `narrow_layout`.
  - One-at-a-time panes slide over each other in `transition_ms`, and start
    on `start_pane`.
  - Code: `features/navigator/src/layout.rs` `resolve`.
- **Folders.**
  - Expand and collapse. An optional root row.
  - Manual order: `NavigatorState::move_child`, `reset_order`.
  - Note counts.
  - Auto-reveal of the active note: `NavigatorState::set_active`, `reveal`.
  - Collapse or expand all: `collapse_or_expand_all` (with smart collapse).
  - Back and forward through the last 100 folders: `back`, `forward`.
- **Notes** (`features/navigator/src/list.rs` `rows`).
  - Each row shows the name (1–3 lines), 0–5 lines of preview, the date,
    the folder and task progress. It can also show tags, properties
    (`list_properties`), a word count and an image. Its `Lines` and `Thumb`
    set its `height`.
  - Sorting: `SortOption`, including `Manual`, which sorts by each note's
    `sort_index`. ⌘↑/⌘↓ (`NavigatorState::move_in_order`) returns
    `Outcome::Write` with `FileOp::SetProperty` ranks. The app's source
    writes them (`NavigatorSource::write`).
  - Grouping: `Grouping` (none, date, folder, property). Property groups
    come from `features/navigator/src/list.rs` `property_groups`, over
    Bases' `group_by` (`features/vault/vault-live/src/grouping.rs`, MIT, the
    one grouping in architect). Notes without the value go last, under
    "None". A heading folds (`toggle_group`, keyed by `collapse_key`).
  - Pinned (`toggle_pin`, ⌘⇧P) is a section on top, per kind of place
    (folders, tags). Pinned rows have one line of preview and no date.
  - Images (`thumb`): a property (`feature_image_properties`), the source's
    `feature_image`, or the file itself (raster or SVG). A drawing uses its
    PNG preview. PDF, canvas and base files get a badge. The view gets URLs
    from `NavigatorSource::resource_url`.
  - A tag on a row goes to the tag (`select_tag`, `NavTarget::Tag`).
  - Names and dates from frontmatter: `use_frontmatter` and the
    `frontmatter_*_field` settings.
  - Folder notes (`folder_notes`): `folder_note`, `convert_to_folder_note`,
    `set_as_folder_note`, `detach_folder_note`, `pin_folder_notes`,
    `open_folder_note`. A click on the folder opens its note.
  - A folder's or a tag's own list: `Appearance` (sort, grouping, mode,
    title and preview rows, descendants, row items), set with
    `set_appearance` or `edit_appearance`. `NavigatorState::effective` is
    the settings the list is drawn with.
  - Multi-select: Cmd/Ctrl-click, Shift-click, select all.
- **Tags** (`features/navigator/src/tags.rs` `TagTree`, rows from
  `features/navigator/src/tree.rs` `tag_rows`), under the folders.
  - Nested by `/`, keyed in lower case (`tag_key`), shown as first
    written. A "Tags" row (`TAGGED`, `all_tags_row`) lists every tagged
    note, with the tags one level in; "Untagged" (`UNTAGGED`,
    `show_untagged`) the notes with none.
  - Counts: a tag's own notes, and those below it once each (with
    `include_descendants`).
  - Roots in a hand-made order (`move_tag_root`, `ViewState::tag_order`,
    the reorder arrows), the rest by `tag_sort` (`TreeSort`: A to Z, Z to A,
    by frequency). `scope_tags` limits them to a folder's or a property's
    notes.
  - Operations return `Outcome::Write` with a `FileOp`: `add_tag` (a note
    with the tag or one above it is left; the tags below go),
    `remove_tag` (the tag itself), `clear_tags`, `rename_tag` (with the
    tags below; not into itself), `delete_tag`, `new_note_in_tag`. Once the
    source has done it, `NavigatorState::after_write` moves the tag's
    colour, order, open state, the selection and the search with it.
  - Drops (`drop_on`, `Drag`): notes on a tag tag them, on Untagged clear
    them; a tag on a tag moves under it (`a/b` on `c` is `c/b`), on the
    Tags row to the top. The view asks before a tag moves.
  - Colours are keyed `tag:<key>`; a tag without one takes the nearest
    tag above it's (`NavigatorState::colour`).
- **Properties** (`features/navigator/src/properties.rs` `PropertyTree`,
  rows from `features/navigator/src/tree.rs` `property_rows`): the keys
  named in `property_keys` (none by default), each with its values and
  counts. A value is matched by its path (`value_path`: a link as its alias
  or target, any case) and shown unwrapped (`value_display`). Ids are
  `key:<key>` and `key:<key>=<value>` (`key_id`, `value_id`). A key lists
  its notes with it empty, or all with it with `include_descendants`;
  `NavTarget::Property`. `set_property` writes a value row's value as
  first written (or the key alone, empty) on the notes selected or
  dropped.
- **The search a ⌘/Ctrl-click builds** (`features/navigator/src/query.rs`
  `Query`): `click_tag` and `click_property` with ⌘/Ctrl add the tag or
  property with AND, with ⇧ too with OR, or take it out. `Query::text`
  writes `#tag`, `.key`, `.key=value` (quoted where needed) joined by
  `AND`/`OR`; `Query::parse` reads it back; `Query::matches` filters the
  list (AND binds tighter). The search box writes the same text
  (`NavigatorState::query` reads it back when it is tags and properties
  alone).
- **The filter search** (`features/navigator/src/search.rs`
  `FilterSearch`, Notebook Navigator's `filterSearch.ts`): the search box
  over the list (`NavigatorState::set_search`, `toggle_search`;
  `Navigator::set_search` in the view). Words match names and aliases
  (every word, any case, Latin accents folded); `#tag`, `#`, `.key`,
  `.key=value`, `.key=null`; dates `@today`, `@last7d`, `@thisweek`,
  `@2026-11`, `@2026-Q4`, `@2026-W45`, ranges, with `@c:`/`@m:` for the
  created or modified time; `folder:name`, `folder:/path`, `ext:pdf`,
  `has:task`; `-` leaves out, a `"quoted"` term is text. `AND`/`OR` are
  operators only between tags and properties (`FilterSearch::is_expression`),
  words elsewhere. A ⌘/Ctrl-click writes into a mixed search as a term of
  its own (`search::toggle_term`). `search_vault` selects the root with
  the notes below and opens the box. With `SearchProvider::FullText`
  (setting `search_provider`) the words go to
  `NavigatorSource::search_text` (`NavigatorState::full_text_due`,
  `set_full_text`; the view's `Navigator::update` asks for them) and the
  rest of the search still narrows. Tests: `features/navigator/tests/search.rs`.
- **Vault profiles and hiding** (`features/navigator/src/profiles.rs`
  `Profiles`, `VaultProfile`; `features/navigator/src/hidden.rs` `Hidden`):
  a "Default" profile always, others added (`add_profile`, as the one in
  use or empty), switched (`select_profile`: id, name or number), renamed,
  removed. Each hides folders (`archive`, `temp*`, `*_old`, `/a/b`,
  `/projects/*/drafts`), tags (`archive` and below, `temp*`, `*draft`, a
  path), notes by name or path glob (`hidden_file_names`), by frontmatter
  (`key`, `key=value`) and by tag; `file_visibility` (documents, the types
  architect opens, all); `descendant_excluded` folders keep their notes
  out of the lists above but are still browsed; a `banner` image over the
  folders. `NavigatorState::index` is what shows, `all` the whole vault;
  `toggle_hidden` shows hidden items; `hide` (a `HideTarget`) is the
  context menus' "Hide". `edit_profile` changes the one in use. Tests:
  `features/navigator/tests/profiles.rs`.
- **Shortcuts and recent notes** (`Shortcut`, `ShortcutKind`): notes,
  folders, tags, properties and saved searches, per profile, as
  `NavRowKind::Section`/`Shortcut`/`Recent` rows (and `Banner`) over the
  folders. `add_shortcut` (once per target), `remove_shortcut`,
  `move_shortcut`, `rename_shortcut`, `open_shortcut(n)` (the palette's
  `NAVIGATOR_OPEN_SHORTCUT_1`…`9`); a move or delete follows. Opening a
  note (`set_active`) records it first in the profile's recent notes,
  `recent_notes_count` kept. Settings: `show_shortcuts`,
  `show_recent_notes`.
- **Keys** (`NavigatorState::key`): ↑ ↓ Home End PageUp PageDown in both
  panes, through the folders, the tags and the properties.
  - Folders, tags, keys: → expands, then moves to the list. ← collapses,
    then goes to the parent.
  - Notes: ⇧↑/⇧↓ and ⇧Home/⇧End extend the selection. ⌘A selects every
    note.
  - Return opens a note: ⇧ in a tab, ⌘ beside.
  - A note the keys move to opens after `KEY_OPEN_SETTLE`.
- **The calendar** (`features/navigator/src/calendar.rs` `grid`, drawn by
  `features/navigator/src/ui/calendar.rs` `Calendar`). The folders'
  header toggles it (`NavigatorState::toggle_calendar`, kept in
  `ViewState::calendar`). It sits under the folders side by side, under
  the notes stacked (a vertical split), and with one pane at a time under
  the folders or under both (`calendar_placement`).
  - 1 to 6 weeks (`calendar_weeks`, 1 by default; 6 is the whole month),
    paged by `calendar_move`; Today (`calendar_today`) opens today's note
    if there is one.
  - A day shows a dot when it has a daily note, the note's feature image
    instead (`calendar_feature_images`), a ring for tasks left, today lit,
    the weekend shaded, and the note open ringed. The calendar goes to a
    day's, week's or month's note as it opens (`set_active`).
  - A click on a day opens its daily note, or makes it. A click on a
    week's number, the month, the quarter or the year opens theirs.
    Making a note asks first (`calendar_confirm_create`;
    `Navigator::open_or_create`).
- **Periodic notes** (`features/navigator/src/calendar.rs`
  `PeriodicNotes`). Daily, weekly, monthly, quarterly and yearly notes
  live under `periodic_folder`, each named by a Moment pattern
  (`features/navigator/src/pattern.rs` `Pattern`: `YYYY/YYYYMMDD`,
  `gggg/[W]ww`, `YYYY/YYYYMM`, `YYYY/[Q]Q`, `YYYY` by default; empty turns
  a kind off).
  - `PeriodicNotes::path` writes a period's path. `period_of` reads a
    path's period back, checked by writing it again.
  - Weeks start on `week_start`. A weekly pattern with `W` or `G` makes
    them ISO weeks.
  - `NavigatorState::open_period` and `open_periodic` return
    `Outcome::Open`, or `Outcome::Write` with a `FileOp::CreateFile`
    (a heading until templates, #339).
  - `open_adjacent_period` and `open_parent_period` move from the note
    open: next, previous, or the period it is in (a week's month is its
    first day's).
- **Periods and date filters** (`features/navigator/src/period.rs`).
  `Period` (a `PeriodKind` and its first day) steps, climbs and gives its
  search token (`Period::filter_token`: `@2026-11-05`,
  `@2026-11-01..2026-11-07`, `@2026-11`, `@2026-Q4`, `@2026`).
  `DateFilter::parse` reads Notebook Navigator's date filters (`@c:`,
  `@m:`, ranges, `@2026-w45`, `@today`, `@last7d`, …) for the search.
- **Actions.** Under the extension they are its manifest's (`ACTIONS`):
  `NAVIGATOR_STATE`, `NAVIGATOR_SELECT`, `NAVIGATOR_REVEAL`,
  `NAVIGATOR_COLLAPSE_OR_EXPAND`, `NAVIGATOR_BACK`, `NAVIGATOR_FORWARD`,
  `NAVIGATOR_FOCUS_FOLDERS`, `NAVIGATOR_FOCUS_NOTES`, `NAVIGATOR_SELECT_ALL`,
  `NAVIGATOR_TOGGLE_DUAL_PANE`, `NAVIGATOR_TOGGLE_ORIENTATION`,
  `NAVIGATOR_TOGGLE_DESCENDANTS`, `NAVIGATOR_TOGGLE_COMPACT`,
  `NAVIGATOR_REORDER_ROOTS`, `NAVIGATOR_SORT`, `NAVIGATOR_GROUP`,
  `NAVIGATOR_TOGGLE_GROUP`, `NAVIGATOR_OWN_LIST`, `NAVIGATOR_COLOUR`,
  `NAVIGATOR_PIN`, `NAVIGATOR_PIN_FOLDER_NOTES`, `NAVIGATOR_MOVE`,
  `NAVIGATOR_CONVERT_TO_FOLDER_NOTE`, `NAVIGATOR_SET_AS_FOLDER_NOTE`,
  `NAVIGATOR_DETACH_FOLDER_NOTE`, `NAVIGATOR_OPEN_FOLDER_NOTE_TAB`,
  `NAVIGATOR_TOGGLE_CALENDAR`, `NAVIGATOR_CALENDAR` (the month, and `weeks`,
  `date`, `move`, `today`), `NAVIGATOR_OPEN_DAILY_NOTE`, `…_WEEKLY_…`,
  `…_MONTHLY_…`, `…_QUARTERLY_…`, `…_YEARLY_NOTE` (`date`, `confirm`,
  `tab`), and `NAVIGATOR_NEXT_PERIOD` (⌃⌥→), `NAVIGATOR_PREVIOUS_PERIOD`
  (⌃⌥←) and `NAVIGATOR_PARENT_PERIOD` (⌃⌥↑); the tags and properties:
  `NAVIGATOR_TAGS`, `NAVIGATOR_PROPERTIES`, `NAVIGATOR_TOGGLE`,
  `NAVIGATOR_TAG_ADD`, `NAVIGATOR_TAG_REMOVE`, `NAVIGATOR_TAG_CLEAR`,
  `NAVIGATOR_TAG_RENAME`, `NAVIGATOR_TAG_DELETE`, `NAVIGATOR_TAG_NEW_NOTE`,
  `NAVIGATOR_PROPERTY_SET`, `NAVIGATOR_DROP` and `NAVIGATOR_SEARCH` (a
  whole filter-search `query`, `open`, `clear`; or a ⌘/Ctrl-click, `or` as
  with ⇧); `NAVIGATOR_SELECT` takes `property` and `value` too; the
  search, shortcuts, recents and profiles: `NAVIGATOR_TOGGLE_SEARCH`,
  `NAVIGATOR_SEARCH_VAULT` (`query`), `NAVIGATOR_TOGGLE_HIDDEN`,
  `NAVIGATOR_HIDE` (`folder`, `tag` or `path`; `unhide`),
  `NAVIGATOR_SHORTCUTS` (`add` a kind, `remove`, `move`/`to`, `rename`,
  `open`), `NAVIGATOR_ADD_SHORTCUT`, `NAVIGATOR_OPEN_SHORTCUT_1`…`9`,
  `NAVIGATOR_RECENT` (`open`, `clear`), `NAVIGATOR_PROFILE` (`select`,
  `add`, `remove`, `rename`; `set` the hidden lists, `banner`,
  `file_visibility`) and `NAVIGATOR_SELECT_PROFILE_1`…`3`. A note's
  context menu has "Add to shortcuts" and "Hide note"; a folder's, tag's
  or property's row menu, "Add to shortcuts" and "Hide".
  Without the extension, `NavigatorView` puts its own on the palette.
- **Settings.** One section, `navigator` (`NavigatorSettings`, a
  `#[derive(Settings)]` struct).
- **Kept between runs.** `ViewState` (`Navigator::view_state`, `restore`):
  the open folders, the manual order, the selected folder, the panes'
  sizes, the pins, the folded groups, each place's own list, the
  colours, the open tags and properties, and their hand-made orders; the
  search box's text, the profiles (with their shortcuts and recent
  notes), show-hidden and the folded sections. It is a facet struct: the
  Architect app keeps it in its folder (`apps/ui/src/vault.rs`
  `view_store`, an `architect::settings::Store`).

## Later layers

These are tracked in #332 (the list in full, #335, the calendar and
periodic notes, #336, tags and properties, #337, and search, shortcuts,
recents, profiles and hiding, #338, are done):

- the real vault as a source (#353);
- file operations, templates, icons and colours.

Each adds variants to `NavRowKind` and `ListRowKind`, and adds `NavTarget`
(both `#[non_exhaustive]`).
