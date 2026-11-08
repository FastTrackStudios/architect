# The navigator (`architect-navigator`)

A port of Notebook Navigator (the Obsidian plugin) as architect's sidebar:
a folder tree beside a list of notes. The crate is `features/navigator`.
Navigator, the built-in extension that makes it a feature, is
`features/navigator/extension` (`architect-navigator-extension`).

**Licence.** `architect-navigator` is GPL-3.0-or-later. With Navigator,
the extension that links it, it is the only GPL code in architect (`features/navigator/README.md`, `features/navigator/LICENSE`).
An app that links it ships under the GPL. Nothing else in architect depends
on it but the app that draws it. Never move its code into another crate.
Pages and extensions fill it through the MIT `architect-navigator-api`
(below, "Providers"). A headless tool or a server
that does not link it stays MIT OR Apache-2.0.

## Rules

- The navigator reads a vault only through `NavigatorSource`
  (`features/navigator/src/source.rs`). A vault becomes a source through an
  adapter in this crate, not by teaching the navigator its types.
- The navigator does not open notes. `NavigatorView`'s `on_open` asks the
  app, and the app opens the note in its workspace (`Target::Here`, `Tab`,
  `Split`, by `OpenIn`). `Open::browsing` says the arrow keys moved to
  the note; show it in place rather than open one more tab. `OpenIn::NewDeck`
  is a ⌘-click or a middle-click on a row the app loads as a deck (a
  project's folder, a setlist's song): the app says which rows those are
  with a `DeckRows` in context (`features/navigator/src/ui/tree_rows.rs`
  `DeckRows`); a middle-click on any other row is `OpenIn::Tab`. The app tells it which note is on show with
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
    // default methods: resource_url, can_write, write, search_text (full text),
    // read_text (a template's, notes merged); None or false by default
}
```

- `NoteFile` has `path` (`Projects/Plan.md`), `created`, `modified`
  (`Millis`), `properties` (`PropertyValue`), `tags`, `preview`,
  `feature_image` and `tasks` (`TaskCounts`).
- `None` means "not worked out yet". Send a `SourceEvent::FileModified`
  when the value arrives.
- `NavigatorSource::write` does `FileOp`s: `CreateFolder`, `CreateFile`
  (a note, canvas or base made), `Move`, `SetProperty`, the tag ops,
  `Copy`, `MoveFolder`, `CopyFolder` and `Trash`. A source that can write
  implements it and says so with `can_write` (the view offers New note);
  the default is read-only. `read_text` gives a file's text: templates and
  merged notes are read through it.
- `SourceEvent` has these variants: `FolderCreated`, `FolderDeleted`,
  `FolderRenamed`, `FileCreated`, `FileModified`, `FileDeleted`,
  `FileRenamed` and `Reset` (read everything again).
- `MemorySource` keeps a vault in memory (`with_text` for a file's text,
  `trashed` for what went to the trash). `MemorySource::synthetic` makes
  a large one for load tests.
- `VaultSource` (feature `vault`, native; `features/navigator/src/vault_source.rs`)
  is architect's vault (`vault::actions::OpenVault`): folders (empty ones
  too), notes with their properties, tags, preview, word and task counts
  and feature image (the vault path of the first of
  `FEATURE_IMAGE_PROPERTIES` naming an image, else of the first image
  embedded), bases and attachments. `resource_url` gives a vault path (or
  an embed's bare name) as a `file:` URL the desk loads; `write` does the
  navigator's `FileOp`s on disk (a folder made, any file moved, a note's
  through `rename` so backlinks follow, a property set; files copied,
  folders moved with `OpenVault::move_folder` so links by path follow,
  folders copied, and things put in the vault's `.trash` with
  `OpenVault::trash`, all MIT in `features/vault/vault/src/actions/edit.rs`);
  `read_text` is `OpenVault::text`. Its `save`,
  `create_note` and `rename` say what changed at once; `watch` hears edits
  made outside the app. `search_text` is the vault's full-text search
  (`OpenVault::search_text`, over `vault_live::sync::search_pages`, MIT).
  `note_from_text` derives a `NoteFile` the same way
  from a text you hold (the sandbox's in-memory vault reads the demo vault's
  files with it).
  Tests: `features/navigator/tests/vault_source.rs`.

## Every vault at once: mounts (#443)

Org = vault: each org the person reaches has one vault, and the navigator
holds them all, with the providers, as one source
(`features/navigator/src/mounts.rs` `Mounts`):

```rust,ignore
let all = Mounts::new("Navigator")
    .with(Mount::provider("tasks/queue", "Inbox", Place::Top).icon("inbox").flat(), queue)
    .with(Mount::vault(OrgBadge::new("codywright", "Cody Wright", "#047857")), cody_vault)
    .with(Mount::vault(OrgBadge::new("fasttrackstudio", "FastTrackStudio", "#1d4ed8")), fts_vault);
// `@codywright/Bible Study/Psalm 23.md`, `~tasks:queue/Today/Write it.md`
```

- A `Mount` is a first path segment: `@<org>` for a vault, `~<provider>`
  for a provider (`mounts::split` reads a path back). `Mounts` lists every
  mount's folders and files under its id, sends their events on with it,
  and routes a write, a read or a URL to its mount. A move between two
  vaults is refused (it changes who owns the note); tag and property-key
  operations go to every vault that writes. `Mounts::describe` changes a
  mount's count or place (a `Reset` follows).
- `NavigatorSource::mounts` says what a source is made of; `VaultIndex`
  keeps it (`mounts`, `mount_of`, `in_vault`). With mounts,
  `NavigatorState::nav_rows` is the sectioned left list: each
  `Place::Top` mount as a row with no header (its containers nest under
  it unless `Mount::flat`; `Mount::count` on it), the shortcuts, Recent
  Files/, Vaults/ (every vault's top-level folders, one flat list by name,
  `tree::VAULTS`), Assets/ (`tree::ASSETS`), Tags/ (the "Tags" row, from
  the vaults alone, merged by name) and the Properties. A section folds
  with `toggle_section`, kept in `ViewState::sections_folded`.
- `VaultIndex::folder_name` names a mount's root by its title and
  `display_path` writes a folder as a row shows it (a vault's without its
  org, a provider's under its title); a list grouped by folder leaves the
  folder off its rows. `NavigatorState::note_badge` washes a note's row in
  a tag's or a property's list with its vault's org.
- `NavigatorState::badge` is a Vaults/ or Assets/ row's `OrgBadge` (no
  mark: the row is washed in the org's colour, `OrgBadge::tint`, at the
  theme's `navigator.org-wash` (a share 0–1, 0.08 by default, 0 off;
  `features/navigator/src/ui/theme.rs` `NUMBERS`), unless it has its own
  background; the org is its `data-org` and its description), `is_wiki` a folder whose
  folder note says `wiki: true` (a book mark), `mount_icon` a top row's
  icon. `Mount::listed` is how a mount's folders list unless a person set
  their own (`settings_for`). A tag or a property lists vault notes only.
- One source with no mounts draws as before: the folder tree.
- Tests: `features/navigator/tests/mounts.rs`.

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
    // A provider's mount only: shows the vault's navigator instead.
    on_vault: None,
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

## Notebook Navigator parity (layer 7)

- **Properties:** `NavigatorState::rename_property_key` (its count of notes
  that have the new key already, which the UI asks about),
  `delete_property_key`, `rename_property_value`; the vault-wide ops
  `FileOp::RenamePropertyKey`, `DeletePropertyKey`, `RenamePropertyValue`,
  expanded note by note by `properties::expand` (a source writes them as
  `SetProperty`/`RemoveProperty`); `NavigatorSettings::after_property_ops`
  moves the settings' key lists. Actions `NAVIGATOR_PROPERTY_RENAME`,
  `NAVIGATOR_PROPERTY_DELETE`.
- **Orders:** `move_nested` (`tag:<t>`, `property:<k>=<v>`),
  `reset_child_order`, kept in `ViewState::tag_child_order` and
  `property_value_order`; `NAVIGATOR_MOVE_CHILD`. `spring_open` takes a
  `NavTarget` (folders and tags).
- **Providers:** `Snapshot::ordered` keeps a provider's node order
  (`NavigatorSource::child_order`).
- **Settings added:** collapse scope and keep-root, accordion, expand on
  select, open first note, folder sort, indent guides, count leader, root
  spacing, Properties row, tooltips, the rainbow (`rainbow`,
  `rainbow::colours`), pinned shortcuts (`NavigatorState::pinned_rows`),
  homepage (`NavigatorState::homepage`, `NAVIGATOR_OPEN_HOMEPAGE`), new
  note in tab, phone zoom (`zoom_phone`), folder note type, template
  engine (`template::templater`).
- **Profiles:** `VaultProfile::property_keys`, `periodic_folder`.
- **Extensions' menu rows:** `ui::MenuExtra` (rows and a runner, given by
  the app over its `[[context]]` on `navigator:folder|tag|property`).
- **Events:** `storage-ready`; `NAVIGATOR_STATE` says `ready`.
- **Full text:** `NavigatorSource::search_excerpts`,
  `NavigatorState::set_full_text_hits`, `excerpt`.

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

## Providers: what fills the panes (#356)

The navigator is the app's only left sidebar. The vault fills it by
default; a page or an extension fills it while its view is on show.

- **The API is MIT.** `architect-navigator-api` (`features/navigator-api`,
  MIT OR Apache-2.0) holds `NavigatorProvider`, `Snapshot`, `Node`, `Item`,
  `Open`, `Method`, `serve`, `ActionProvider` and `date_millis`. A provider
  implements it and never depends on `architect-navigator` (GPL). Keep
  anything an MIT crate needs in the API crate, never in the GPL one.
- **The trait** (`features/navigator-api/src/lib.rs` `NavigatorProvider`):
  `snapshot()` answers the title, the `Node` tree (id, title, icon, colour,
  children) and every `Item` (id, container, title, preview, dates, icon,
  thumbnail, tags, `progress`, badges, properties, `open`).
  `Snapshot::ordered` keeps the nodes in the order given (Today before
  Later) instead of by name, until the person orders them by hand
  (`NavigatorSource::child_order`, `VaultIndex::source_order`). `open(item)`,
  `move_items(items, to)` and `create(container, title)` default to nothing
  or an error. Every method is async (`ProviderFuture`). An item that is
  a context (a song, a project) opens with `Open::Context {context_type,
  id, title, folder}`: a click loads it onto the deck on show, a ⌘-click or
  a middle-click onto a new deck (`apps/ui/src/vault.rs` `ProviderFiles`,
  `apps/ui/src/setlists.rs` the sandbox's setlists).
- **Serving it:** declare `[[navigator_providers]] id, title, icon, views,
  actions` and the actions it answers (`<actions>_SNAPSHOT`, `_OPEN`,
  `_MOVE`, `_CREATE`), and where the left list shows it (#443):
  `section` (`top`: a row with no header, Inbox, Calendar, Projects;
  `assets`: its containers in Assets/; else a folding section of its own),
  `position` (low first), `visible` (`false`: left out, and never read)
  and `nest` (`false`: its containers are the list's groups, not rows).
  A `Snapshot` says its row's `count`, `grouped()` (its nodes the list's
  groups, in order) and `group_by(key, order)` (its lists grouped by an
  item property, in that order) and `calendar(CalendarGrid)` (a month grid
  under its top row, over its tree: each day's `GridDay` names the day
  node a click selects, its `count` of dots, its `label` and `colour`;
  `ProviderSource::section_calendar` maps it to folders as
  `Mount::calendar`, drawn by `features/navigator/src/ui/calendar.rs`
  `SectionGrid`, its days taking drops; the Calendar's, #444); a `Node` says its `org` and whether it is
  `shared` (an asset library's org, a project shared through a Space: the
  row's org wash and shared mark, `NavigatorSource::folder_meta`,
  `NavigatorState::badge`, `is_shared`). In `build`,
  `ext.navigator("<ACTIONS>", provider)`
  (`features/extensions/src/linked.rs` `Builder::navigator`). A process,
  Luau or page extension answers those actions itself.
- **The app** reads every provider through its actions
  (`ActionProvider` over the host's Action List) and draws it with a
  `ProviderSource` (`features/navigator/src/provider_source.rs`): nodes as
  folders, items as notes in their container's folder, titles made unique.
  A provider's items never fill the Properties section: their fields are
  the provider's, not the vault's (`NavigatorSource::browses_properties`,
  false for a `ProviderSource`; kept as `VaultIndex::browses_properties`).
  Files with no project folder on the device shows its empty state
  (`features/files/extension/src/nav.rs` `empty_state`) instead of an error.
  `ProviderSource::item_at` reads an opened path back as its item; a drag
  onto a folder and New come back as `ProviderRequest`s. There is one
  navigator, with no switcher: `apps/ui/src/vault.rs` `FilesView` mounts
  each shown provider in the app's `Mounts` beside every org's vault
  (`ProviderFeed` polls its snapshot every three seconds and after each
  request, and sets its mount's count and groups); a page with a provider
  selects its row. The org vaults come from `apps/ui/src/vaults.rs`
  `catalog` (`vaults.json` `orgs`, or the file `ARCHITECT_ORG_VAULTS`
  names: `[{org, name, colour, icon, dir}]`); the app's own vault is its
  org's (`app_vault_org`). A note of another org's vault opens as the pane
  state `@<org>/<path>` (`LabVault::vault_of`), so the app's vault never
  switches, and its links resolve in its own vault.
- **Assets/** is the Files extension's `assets` provider
  (`features/files/extension/src/nav.rs` `AssetsNav`, `FILES_ASSETS_NAV`):
  every org's libraries, flat, each its org's.
- **Examples:**
  - `extensions/tasks/src/queue.rs` `QueueNav` (`TASKS_QUEUE_NAV`, the Inbox
    row): the Queue, one list grouped Inbox (captures, `INBOX_LIST`),
    Today (due or scheduled today, and overdue), Upcoming, Waiting; its
    count the captures to process. A capture opens the app's triage hook
    `INBOX_PROCESS_CARD {path}` (`apps/ui/src/capture.rs` `process_card`,
    `ProcessCard`'s shape from #445; the inbox deck until then); dropped
    on a task group it becomes a task there (`INBOX_PROCESS`).
  - `extensions/projects/src/nav.rs` `ProjectsNav` (the Projects row):
    projects nested by folder or `parent`/`up` (`parent_of`), each with its
    icon, home `org` and shared mark (`space`); a project's contents
    grouped by `section` in `SECTIONS` order: Songs (`type: song` or a
    `song` tag, by `project`/`projects`/`album` or folder; in `sort_index`
    order, loaded onto a deck), Notes, Sub-projects, Tasks (`snapshot_of`).
    A property group in a provider's order keeps its notes' hand order
    (`sort_index`) first.
  - `extensions/calendar/src/nav.rs` `CalendarNav` (the Calendar row:
    events grouped by when they come; #444's month grid takes its place).
  - `extensions/tasks/src/nav.rs` `TasksNav` (status groups; off the
    list, folded into the Queue).

  Tests: `features/navigator/tests/provider.rs`,
  `features/navigator/tests/mounts.rs`, `features/navigator-api/src/lib.rs`
  `tests`, `extensions/tasks/src/queue.rs` `tests`,
  `extensions/projects/src/nav.rs` `tests`, `features/spaces/tests/projects.rs`
  (a project shared into another org through a Space reaches its folder
  and nothing else of the home org's vault).

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
  - A project's glyph: `architect_navigator_api::PROJECT_ICON` (`box`) for
    every project row (Projects, sub-projects, a vault folder with a
    `project.md`, that note, a deck tab), its own `icon`/`color` over it;
    `NavigatorState::project_page`, `FolderMeta::colour`, `Node::colour`.
  - Auto-reveal of the active note: `NavigatorState::set_active`, `reveal`.
    `set_active_with(path, false)` lights it without revealing it (a deck's
    project stays selected); `show_folder` selects a row with the rows above
    it opened (the row a deck starts on, `apps/ui/src/vault.rs` `follow_deck`).
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
    Bases' `group_by` (`features/vault/vault-proto/src/grouping.rs`, MIT, the
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
- **File operations** (`features/navigator/src/state/files.rs`,
  Notebook Navigator's `FileSystemService`): each returns an
  `Outcome::Write` for the source. `new_note` (a `NewNote`: a note,
  canvas or base, `NewKind`, named `Untitled`, by a typed name or by a
  name format, from a template; it answers `Made::Ask` with the prompts
  to answer first, or `Made::Done` with the write, the cursor and a
  warning), `new_folder`, `rename_file` (the extension kept),
  `rename_folder` (its folder note named after it follows), `move_files`,
  `copy_files` (numbered when taken), `move_folder`, `duplicate`
  (`Plan 1`, opened), `duplicate_folder`, `trash` (and `trash_question`;
  the view asks first by `confirm_delete`), `merge` (`MergeSeparator`;
  the first note's frontmatter kept), `open_all` (the view asks at
  `OPEN_ALL_WARNING`, 15). `check_name` says what a name cannot be
  (empty, a leading period, `:` or `/`) and what breaks links (`#`, `|`,
  `^`, `%%`, `[[`, `]]`). `drop_on` takes notes and folders on folders,
  copying with the copy key (⌥ on a Mac, Ctrl elsewhere);
  `drop_on_shortcuts` moves a shortcut dragged among them, or adds what is
  dropped there; `spring_open` opens a folder held under a drag
  (`spring_loaded_folders`, `spring_loaded_delay_ms`,
  `spring_loaded_next_delay_ms`). The view renames in place
  (`ui::Renaming`, `Navigator::renaming`), asks with `ui::Ask`, and its
  row menu on a folder makes notes, canvases, bases and folders, runs the
  create-note commands, moves or copies the selection there, opens all,
  sets the folder template, renames, duplicates, moves and deletes it.
  Tests: `features/navigator/tests/files.rs`.
- **Templates** (`features/navigator/src/template.rs`, Notebook
  Navigator's built-in engine): `render` writes `{{title}}`, `{{folder}}`,
  `{{path}}`, `{{date}}` (and `yesterday`, `tomorrow`, `monday` …
  `sunday`), `{{time}}`, `{{today}}`, `{{now}}`, with offsets (`+1M`,
  `-2d`, `+90m`) and Moment formats (`{{date:YYYY-MM-DD}}`, through
  `Pattern::format_at`), `{{cursor}}`, `{{prompt:Label}}`/`{{value}}`,
  `{{number:000}}` and `{{!name}}`; unknown tokens stay as written, and
  quoted frontmatter values are escaped. `numbered_name`, `next_number`
  and `format_numbered` number a name format by the notes in its folder.
  `Templates` (kept in `ViewState::templates`) holds each folder's
  `FolderTemplate`, inherited by subfolders (`folder_template`), and the
  `TemplateCommand`s ("create note" commands with a name format, a
  template and a location, `CommandLocation`; stable ids). Template
  paths follow their files moved. Periodic notes take
  `daily_template` … `yearly_template`, `{{date}}` in their file name's
  pattern; settings `date_format`, `time_format`, `template_folder`,
  `folder_note_template`. `NavigatorState::set_reader` reads templates
  (`use_navigator` sets it to the source's `read_text`).
- **Icons and colours** (`features/navigator/src/icons.rs` `Icon`,
  `features/navigator/src/state/looks.rs`, Notebook Navigator's
  `MetadataService` and `iconizeFormat.ts`): `NavigatorState::paint`
  (`Paint::Icon`, `Colour`, `Background`) sets or clears one on
  `folder:<path>`, `tag:<tag>`, `property:<key>[=<value>]` or
  `file:<path>`, kept in `ViewState::icons`, `colours` and
  `backgrounds`; with `use_frontmatter`, a note's are read from and
  written to its frontmatter (`frontmatter_icon_field`,
  `frontmatter_color_field`, `frontmatter_background_field`; it returns
  the `FileOp::SetProperty` or `RemoveProperty`). `Icon::parse` reads
  Lucide names (`folder`, `lucide-folder`, `li:folder`), Iconize's
  (`LiFolderOpen`, `PhHouse`), packs (`ph:house`), emoji and vault SVGs;
  `Icon::frontmatter` writes the short form. `folder_icon`, `item_icon`
  and `file_icon` (a note's own, then by name, `file_name_icons` with
  `file_name_icon_map`, then by type, `file_type_icons` with
  `file_type_icon_map`: on by default, with built-ins for markdown,
  canvas, base, PDF, images, audio, MIDI, video and DAW sessions; a
  provider's item's own `Item::icon`); a provider's container's
  `Node::icon` is its folder's (`FolderMeta::icon`); colours inherit by `inherit_folder_colors`,
  `inherit_tag_colors` and `inherit_property_colors`; `color_icon_only`.
  Lucide icons are drawn by name from `features/shell/src/lucide.rs`
  `LucideIcon` (MIT; every icon of the pinned lucide-dioxus as data,
  `features/shell/scripts/lucide.py`). An icon pack's icon is drawn as a
  box until packs come. Paint follows folders and notes moved.
- **Search highlighting** (`features/navigator/src/search.rs`):
  `highlights` gives where the search's words are in a name (byte
  ranges, accents folded), drawn as `mark.nv-search-highlight` in the
  role `navigator.search-highlight`; `FilterSearch::tag_match`
  (`SearchMatch`) lights a tag's row and pills that a `#tag` takes in or
  `-#tag` leaves out.
- **Keys** (`NavigatorState::key`): ↑ ↓ Home End PageUp PageDown in both
  panes, through the folders, the tags and the properties.
  - Folders, tags, keys: → expands, then moves to the list. ← collapses,
    then goes to the parent.
  - Notes: ⇧↑/⇧↓ and ⇧Home/⇧End extend the selection. ⌘A selects every
    note.
  - Return opens a note: ⇧ in a tab, ⌘ beside.
  - A note the keys move to opens after `KEY_OPEN_SETTLE`.
  - The keyboard map (`features/navigator/src/keys.rs` `KeyMap`, Notebook
    Navigator's `keyboardShortcuts.ts`): every key is a `KeyAction` with a
    stable id (`pane:move-up`, `pane:rename` on F2, `pane:delete-selected`,
    `navigation:collapse-or-parent`, `list:select-all`,
    `list:manual-sort-up`, `list:open`, `list:pin`, `search:close`, …),
    bound to `Hotkey`s (`Mod+Shift+P`; `Mod` is ⌘ on a Mac, Ctrl
    elsewhere). The view reads a key with `KeyMap::action` in its
    `KeyScope` and does it with `NavigatorState::run_key`; `vim_keys`
    adds h j k l, g and ⇧G; a person's own (`bind_keys`, `KeyBinding`)
    are kept in `ViewState::keys`.
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
  `file_visibility`) and `NAVIGATOR_SELECT_PROFILE_1`…`3`; the file
  operations and templates: `NAVIGATOR_NEW_NOTE` (`folder`, `kind`,
  `name`, `name_format`, `template`, `no_template`, `prompts`, `tab`;
  it answers the `path` and the `cursor`, or the prompts to answer),
  `NAVIGATOR_NEW_FOLDER`, `NAVIGATOR_RENAME` (no `name`: in place),
  `NAVIGATOR_DUPLICATE`, `NAVIGATOR_MOVE_TO`, `NAVIGATOR_COPY_TO`,
  `NAVIGATOR_DELETE` (`confirm` false to skip the question),
  `NAVIGATOR_MERGE`, `NAVIGATOR_OPEN_ALL`, `NAVIGATOR_OPEN_SELECTED`,
  `NAVIGATOR_FOLDER_TEMPLATE`, `NAVIGATOR_TEMPLATE_COMMANDS` (`add`,
  `remove`) and `NAVIGATOR_TEMPLATE_COMMAND` (`id`); `NAVIGATOR_DROP`
  takes `drag_folder`, `to` (a folder) and `copy`; the icons, colours and
  keys: `NAVIGATOR_ICON`, `NAVIGATOR_COLOUR` (any target, `background`;
  no value asks), `NAVIGATOR_KEYS` (the map; `action` with `keys` or
  `reset`), `NAVIGATOR_KEY` (does a key action by its id) and
  `NAVIGATOR_METADATA` (a target's icon, colour, background and pins;
  `pinned` for all). A note's context menu
  has "Add to shortcuts", "Hide note", "Rename…", "Duplicate", "Move to…",
  "Open the selected in new tabs", "Merge the selected notes",
  "Delete", "Set icon…" and "Set colour…"; a folder's, tag's or property's row menu, "Add to shortcuts"
  and "Hide", and a folder's the file operations.
  Without the extension, `NavigatorView` puts its own on the palette.
- **Settings.** One section, `navigator` (`NavigatorSettings`, a
  `#[derive(Settings)]` struct), synced (`scope = Synced`): each setting
  can be kept on one device (its row's toggle;
  `architect_settings::ScopedValues::set_local`). The Architect app keeps
  the synced scope in `settings-synced.json` until a synced store holds it,
  and moved the values it kept per device there once
  (`apps/ui/src/settings.rs` `adopt_synced`). `zoom` (50 to 200%) sizes the
  navigator apart from the app (CSS `zoom` on its root; `Navigator::zoom`
  takes it off pointer and measured lengths). The vault profiles have an
  editor on the settings page, opened from the profile menu's "Edit
  profiles…", the palette's `NAVIGATOR_EDIT_PROFILES` or
  `NAVIGATOR_PROFILE`'s `edit` (`features/navigator/src/ui/profiles_page.rs`
  `open_profiles_page`, section `navigator-profiles`, there from the first
  time it opens; `NavigatorState::edit_profile_in`).
- **API for extensions.** The `NAVIGATOR_*` actions are its API
  (navigation: `NAVIGATOR_SELECT`, `NAVIGATOR_REVEAL`; selection:
  `NAVIGATOR_STATE`; metadata: `NAVIGATOR_METADATA`, `NAVIGATOR_ICON`,
  `NAVIGATOR_COLOUR`, `NAVIGATOR_PIN`), and it raises events as they
  change (`HostRequest::Raise`, `features/navigator/extension/src/lib.rs`
  `EVENTS`): `app:navigator.nav-item-changed`, `selection-changed`,
  `pinned-files-changed` and `metadata-changed`, heard by an extension
  whose `capabilities.events` has `navigator.*`.
- **Kept between runs.** `ViewState` (`Navigator::view_state`, `restore`):
  the open folders, the manual order, the selected folder, the panes'
  sizes, the pins, the folded groups, each place's own list, the
  colours, the open tags and properties, and their hand-made orders; the
  search box's text, the profiles (with their shortcuts and recent
  notes), show-hidden and the folded sections; the folder templates and
  create-note commands (`templates`), icons and backgrounds (`icons`,
  `backgrounds`) and a person's keys (`keys`). It is a facet struct: the
  Architect app keeps it in its folder (`apps/ui/src/vault.rs`
  `view_store`, an `architect::settings::Store`).

## Later layers

These are tracked in #332 (the list in full, #335, the calendar and
periodic notes, #336, tags and properties, #337, and search, shortcuts,
recents, profiles and hiding, #338, are done):

- the real vault as a source (#353);
- what #339 left: a template picker, icon packs, Templater, and the
  ribbon buttons of create-note commands (see the docs page's status).

Each adds variants to `NavRowKind` and `ListRowKind`, and adds `NavTarget`
(both `#[non_exhaustive]`).
