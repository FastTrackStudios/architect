# architect-navigator

The navigator: a port of [Notebook Navigator](https://github.com/johansan/notebook-navigator),
an Obsidian plugin by Johan Sanneblad, as architect's sidebar. A folder tree beside a list of
notes, side by side, stacked, or one pane at a time with a slide between them; virtualized to
100,000 notes; driven by the mouse, the keyboard and the palette's `NAVIGATOR_*` commands.

**This is the one GPL crate in architect.** It is licensed GPL-3.0-or-later (`LICENSE`), because
it carries Notebook Navigator's design and code. An app that links it ships under the GPL. The
rest of architect is MIT OR Apache-2.0 and does not depend on this crate; a tool or server that
leaves it out stays permissive.

- The model (default features, wasm-clean): `NavigatorSource` and `MemorySource`, the folder
  index, the tree and list rows, selection and history, the virtual window, the keys, the
  pane layout, `NavigatorSettings`.
- `ui`: the Dioxus view on architect's shell, `use_navigator` and `NavigatorView`.

How to use it: `skills/architect/references/navigator.md`. The lab's Vault tab shows it over the
lab's notes (`apps/lab/ui/src/vault.rs`).
