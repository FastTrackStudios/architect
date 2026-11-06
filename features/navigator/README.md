# architect-navigator

The navigator: a port of [Notebook Navigator](https://github.com/johansan/notebook-navigator),
an Obsidian plugin by Johan Sanneblad, as architect's sidebar. A folder tree beside a list of
notes, side by side, stacked, or one pane at a time with a slide between them; virtualized to
100,000 notes; driven by the mouse, the keyboard and the palette's `NAVIGATOR_*` commands.

**This is GPL code.** The crate is licensed GPL-3.0-or-later (`LICENSE`), because it carries
Notebook Navigator's design and code, and so is Navigator, the built-in extension that links it
(`extension/`, `architect-navigator-extension`). They are the only GPL crates in architect. An
app that links either ships under the GPL. The rest of architect is MIT OR Apache-2.0 and does
not depend on them; a tool or server that leaves them out stays permissive.

- The model (default features, wasm-clean): `NavigatorSource` and `MemorySource`, the folder
  index, the tree and list rows, selection and history, the virtual window, the keys, the
  pane layout, `NavigatorSettings`; the calendar's grid, the periodic notes' Moment patterns,
  periods and date filters.
- `ui`: the Dioxus view on architect's shell, `use_navigator` and `NavigatorView`.
- `extension/`: Navigator, the built-in extension: its sidebar view, its `NAVIGATOR_*` actions
  and its settings section, linked into an app or run as a process (`EXT_CLONE`).

How to use it: `skills/architect/references/navigator.md` and `docs/content/features/navigator.md`. The lab's Vault tab shows it over the
lab's notes (`apps/ui/src/vault.rs`).
