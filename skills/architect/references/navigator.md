# The navigator (`architect-navigator`)

A port of Notebook Navigator (the Obsidian plugin) as architect's sidebar:
a folder tree beside a list of notes. The crate is `features/navigator`.

**Licence.** `architect-navigator` is GPL-3.0-or-later, the one GPL crate
in architect (`features/navigator/README.md`, `features/navigator/LICENSE`).
An app that links it ships under the GPL. Nothing else in architect depends
on it. Never move its code into another crate. A headless tool or a server
that does not link it stays MIT OR Apache-2.0.

## Rules

- The navigator reads a vault only through `NavigatorSource`
  (`features/navigator/src/source.rs`). A vault becomes a source through an
  adapter in this crate, not by teaching the navigator its types.
- The navigator does not open notes. `NavigatorView`'s `on_open` asks the
  app, and the app opens the note in its workspace (`Target::Here`, `Tab`,
  `Split`, by `OpenIn`). The app tells it which note is on show with
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

## Recipe: the navigator as a sidebar

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
} }
// When the note on show changes:
nav.set_active(Some(path));
```

The lab's Vault tab does exactly this (`apps/lab/ui/src/vault.rs`
`FilesView`, `lab_vault`).

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
- **Notes.**
  - Each row shows the name, 1–5 lines of preview, the date and the folder.
  - Sorting: `SortOption`. Date groups: `Grouping`.
  - Multi-select: Cmd/Ctrl-click, Shift-click, select all.
- **Keys** (`NavigatorState::key`): ↑ ↓ Home End PageUp PageDown in both
  panes.
  - Folders: → expands, then moves to the list. ← collapses, then goes to
    the parent.
  - Notes: ⇧↑/⇧↓ and ⇧Home/⇧End extend the selection. ⌘A selects every
    note.
  - Return opens a note: ⇧ in a tab, ⌘ beside.
  - A note the keys move to opens after `KEY_OPEN_SETTLE`.
- **Commands** on the palette while the view is mounted: `NAVIGATOR_REVEAL`,
  `NAVIGATOR_COLLAPSE_OR_EXPAND`, `NAVIGATOR_BACK`, `NAVIGATOR_FORWARD`,
  `NAVIGATOR_FOCUS_FOLDERS`, `NAVIGATOR_FOCUS_NOTES`, `NAVIGATOR_SELECT_ALL`,
  `NAVIGATOR_TOGGLE_DUAL_PANE`, `NAVIGATOR_TOGGLE_ORIENTATION`,
  `NAVIGATOR_TOGGLE_DESCENDANTS`, `NAVIGATOR_REORDER_ROOTS`.
- **Settings.** One section, `navigator` (`NavigatorSettings`, a
  `#[derive(Settings)]` struct).
- **Kept between runs.** `ViewState` (`Navigator::view_state`, `restore`):
  the open folders, the manual order, the selected folder and the panes'
  sizes. It is a facet struct, so the app stores it where it keeps its
  settings.

## Later layers

These are tracked in #332:

- the list in full;
- the calendar and periodic notes;
- tags and properties;
- search, shortcuts, recents, profiles and hiding;
- file operations, templates, icons and colours.

Each adds variants to `NavRowKind` and `ListRowKind`, and adds `NavTarget`
(both `#[non_exhaustive]`).
