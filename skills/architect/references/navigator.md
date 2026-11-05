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
}
```

- `NoteFile` has `path` (`Projects/Plan.md`), `created`, `modified`
  (`Millis`), `properties` (`PropertyValue`), `tags`, `preview`,
  `feature_image` and `tasks` (`TaskCounts`).
- `None` means "not worked out yet". Send a `SourceEvent::FileModified`
  when the value arrives.
- `SourceEvent` has these variants: `FolderCreated`, `FolderDeleted`,
  `FolderRenamed`, `FileCreated`, `FileModified`, `FileDeleted`,
  `FileRenamed` and `Reset` (read everything again).
- `MemorySource` keeps a vault in memory. `MemorySource::synthetic` makes
  a large one for load tests.
- `VaultSource` (feature `vault`, native; `features/navigator/src/vault_source.rs`)
  is architect's vault (`vault::actions::OpenVault`): folders (empty ones
  too), notes with their properties, tags, preview, feature image (the
  first of `FEATURE_IMAGE_PROPERTIES` naming an image, else the first image
  embedded; a `file:` URL the desk loads) and task counts, bases and
  attachments. Its `save`, `create_note` and `rename` (backlinks follow)
  say what changed at once; `watch` hears edits made outside the app.
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

The lab's Vault tab does this (`apps/lab/ui/src/vault.rs` `FilesView`,
`apps/lab/ui/src/extensions.rs`).

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

The lab's Vault tab does exactly this (`apps/lab/ui/src/vault.rs`
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
    come from `features/navigator/src/grouping.rs` `by_value`, which Bases
    can share. Notes without the value go last, under "None". A heading
    folds (`toggle_group`, keyed by `collapse_key`).
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
- **Keys** (`NavigatorState::key`): ↑ ↓ Home End PageUp PageDown in both
  panes.
  - Folders: → expands, then moves to the list. ← collapses, then goes to
    the parent.
  - Notes: ⇧↑/⇧↓ and ⇧Home/⇧End extend the selection. ⌘A selects every
    note.
  - Return opens a note: ⇧ in a tab, ⌘ beside.
  - A note the keys move to opens after `KEY_OPEN_SETTLE`.
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
  `NAVIGATOR_DETACH_FOLDER_NOTE`, `NAVIGATOR_OPEN_FOLDER_NOTE_TAB`.
  Without the extension, `NavigatorView` puts its own on the palette.
- **Settings.** One section, `navigator` (`NavigatorSettings`, a
  `#[derive(Settings)]` struct).
- **Kept between runs.** `ViewState` (`Navigator::view_state`, `restore`):
  the open folders, the manual order, the selected folder, the panes'
  sizes, the pins, the folded groups, each place's own list and the
  colours. It is a facet struct, so the app stores it where it keeps its
  settings.

## Later layers

These are tracked in #332 (the list in full, #335, is done):

- the real vault as a source (#353);
- the calendar and periodic notes;
- tags and properties;
- search, shortcuts, recents, profiles and hiding;
- file operations, templates, icons and colours.

Each adds variants to `NavRowKind` and `ListRowKind`, and adds `NavTarget`
(both `#[non_exhaustive]`).
