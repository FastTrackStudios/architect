+++
title = "Theming"
description = "Runtime themes: a small palette resolved into every semantic token, a cascade of layers from the app's default to a transient preview, theme files that hot-reload, THEME_* actions, and how the shell, architect-ui and view kinds follow one store."
weight = 69
+++

> **Status: landed (2026-11-03)** in `architect-theme` (`features/theme`):
> the model (wasm-clean), the files (`fs`), the actions (`actions`) and the
> Dioxus store (`dioxus`). The shell (`features/shell/src/shell/mod.rs`
> `ShellTheme`, `AppShell`), architect-ui's root
> (`features/ui/architect-ui/src/theme/runtime.rs` `ThemeStoreProvider`)
> and paint panes (`features/workspace/src/paint/widget.rs` `PaintPane`)
> follow it. The lab uses it (`apps/lab/ui/src/theme.rs`). Not yet: the
> Appearance settings page (#185) and persisting the layers.

An app built on architect is themeable at run time, all of it: the shell,
architect-ui's components, a GPU paint view, an extension's view. One
store per app holds the themes and the person's choices, resolves the
theme showing into one flat map of tokens, and tells everything that
draws when that map changes. The model is herdr's: a theme is a few
colours, every other colour is derived from them, and a file saved on
disk shows at once.

## A theme

A theme is a **palette** (a dozen colours or fewer), its **appearance**
(light or dark), maybe a parent it **extends**, and **overrides** by
token name (`features/theme/src/model.rs` `ThemeFile`, `Palette`).

| Palette colour | What it is | Tokens made from it |
|---|---|---|
| `background` | the ground | `background`, `desk`, every surface step |
| `foreground` | the text | `foreground`, `ink`, `ink-2`, `ink-3`, the muted ink |
| `accent` | the brand | `primary`, `ring`, `brand`, `brand-soft`, `accent` (the wash), `chart-1` |
| `surface` | cards, popovers | `card`, `popover` (else a step off the background) |
| `border` | edges | `border`, `input`, `sidebar-border` (else a mix) |
| `safe` / `warn` / `danger` / `info` | status | the same names, `signal-safe` / `-warn` / `-danger`, `destructive` |
| `chart` | up to five series | `chart-1` … `chart-5` |

Every palette colour is optional: a theme takes what it leaves out from
its parent, and then from the built-in default for its scheme
(`features/theme/src/resolve.rs` `default_palette`).

## Resolving

`resolve(theme, registry)` (`features/theme/src/resolve.rs` `resolve`)
walks the `extends` chain, lays the palettes over each other, derives
every token (`derive`), then applies the overrides, the parents' first.
The derivation mixes in OKLab (`features/theme/src/color.rs` `Rgba::mix`),
branches on light and dark (a dark theme's surfaces step toward the
text, a light theme's toward white), and fixes contrast for the ink: `ink-2`,
`ink-3` and the muted ink are moved toward white or black until they
reach 4.5:1 on every shell fill they sit on, the accent's wash over a
pill included (`with_contrast`); text on the accent or the danger colour
picks the deep shade or white that reads better. It is deterministic: the
same theme and registry give the same map, byte for byte.

The result is a `Resolved` (`features/theme/src/resolve.rs` `Resolved`):
the token map, the scheme, the theme's id and a revision. Read a token
with `get` (an `Option`), `value` or `color`. An unknown name answers
magenta (`#ff00ff`) and is logged once per name, so a typo shows on
screen instead of quietly falling back.

The token names (`features/theme/src/resolve.rs` `names`):

- architect-ui's, the CSS variables of `fts-theme.css`: `background`,
  `foreground`, `card`, `popover`, `primary`, `secondary`, `muted`,
  `accent`, `destructive` (each with `-foreground`), `border`, `input`,
  `ring`, `chart-1`…`chart-5`, `sidebar-*`, `signal-*`.
- the status colours: `safe`, `warn`, `danger`, `info`.
- the shell's: `desk`, `sheet`, `title-bar`, `pill`, `rule`, `active`,
  `ink`, `ink-2`, `ink-3`, `brand`, `brand-ink`, `brand-soft`, `scrim`,
  `shadow-lift`, `shadow-float`, `font`. The shell's accent is `brand`
  here, because `accent` is architect-ui's hover wash.
- the rest: `font-sans`, `font-serif`, `font-mono`, `radius`, `spacing`,
  `letter-spacing`, `shadow-*`.

An override may name any token, these or a new one. That is how an app
adds its own roles (below).

## Choosing: the cascade

What shows is chosen by **layers**, lowest first
(`features/theme/src/store.rs` `LayerKind`):

1. `Builtin`: `FastTrackStudio` light and dark, following the system.
2. `App`: the app's default (Studio's own pair).
3. `Org`: the organization's default.
4. `Person`: the person's, on every device.
5. `Device`: this device's.
6. `Workspace`: a workspace's or screenset's look.
7. `Pick`: an explicit pick (a picker, `THEME_SET`).
8. `Preview`: a transient preview (`THEME_PREVIEW`).

Each layer may set a **choice**, an **appearance** and **token overrides**
(`Layer`). The highest layer that sets a choice wins, and the same goes
for appearance. Overrides merge, the higher over the lower. A choice is
`ThemeChoice::One(id)` (always that theme) or `ThemeChoice::Pair { light,
dark }`. Appearance is `System`, `Light` or `Dark`. `System` follows the
OS's scheme, which the host tells the store (`ThemeStore::set_system`).
Picking a theme (`ThemeStore::pick`) replaces only the half of the pair
that matches its own lightness: picking a dark theme keeps the light one.

A person's choices live in layers, never in theme files: the files are
authored, the picks are settings. The store does not persist layers. The
app saves the ones it owns (`ThemeStore::layer`) and sets them again at
start.

## The store

`ThemeStore` (`features/theme/src/store.rs` `ThemeStore`) is one per app,
`Send` and `Sync`, and its clones share it. Every write (`set`, `update`,
`pick`, `preview`, `set_token`, `set_system`, `set_registry`, `reload`)
composes the layers, resolves again, and returns `false` when nothing
that shows changed. On a real change it bumps the revision and calls each
`subscribe`r once, on the thread that made the change.

A consumer keeps a typed slice and compares it, so it redraws only when
its own part changes (herdr's `Vt::set_theme`): implement `FromResolved`
and hold a `Projected<T>`. Its `update` returns true only when `T`
differs.

## Theme files (feature `fs`)

A theme file is `<id>.json` in a themes folder: JSON, comments allowed,
read strictly. An unknown field, a palette value that is not a colour, or
a token name that is not lowercase-and-dashes is a `Problem` naming the
file and the field (`themes/night.json: palette.accent: "teal-ish" is not
a colour`).

```json
// themes/studio-night.json
{
  "label": "Studio Night",
  "appearance": "dark",
  "extends": "fasttrackstudio-dark",
  "palette": {
    "accent": "#f59e0b",
    "danger": "#ef4444",
    "chart": ["#f59e0b", "#38bdf8", "#34d399", "#fbbf24", "#f472b6"]
  },
  "overrides": {
    "radius": "0.375rem",
    "arrange-clip": "#1e293b"
  }
}
```

- `ThemeDirs` (`features/theme/src/fs.rs` `ThemeDirs`) lists the places:
  folders, and extensions' files. A later place wins over an earlier one
  for the same id, and over a built-in theme. `install_into(store)` loads
  them and makes `reload` (and `THEME_RELOAD`) read them again.
- **Last good.** A file that no longer reads is reported, and the theme
  it held stays as it was (`ThemeDirs::load`).
- **Watching.** `fs::watch(store, dirs)` (`notify`) reloads after a burst
  of saves settles, the theme showing included.
- **Shipped themes.** `fs::install(dir, shipped)` writes an app's own
  themes into the person's folder. A missing file is written, a file that
  is exactly an earlier shipped version (`Shipped::previous`) is upgraded,
  and anything else is the person's edit and is kept.
- **Extensions.** An extension declares `[[themes]] path = "themes/x.json"`
  (`features/extensions/src/manifest.rs` `ThemeDecl`,
  `Manifest::theme_files`). The app adds them with
  `ThemeDirs::extension(id, folder, manifest.theme_files())`. The
  extension host does not touch the store itself. That glue is the app's,
  because the store is the app's.

architect's built-in themes are files too (`features/theme/themes/`,
`features/theme/src/builtin.rs` `FILES`): `FastTrackStudio` (the default),
and the light and dark halves of every former architect-ui preset
(Catppuccin, Nord, Dracula, Gruvbox, Solarized, Tokyo Night, Rosé Pine,
Everforest, Zen, Obsidian, tweakcn's set). Each keeps its old look:
architect-ui's tokens are overrides where the derivation would differ,
and the shell's tokens are derived from its palette.

## Actions (feature `actions`)

`actions::register(list, store)` (`features/theme/src/actions.rs`
`register`) puts the store on the Action List, for ⌘K, keys, MCP and the
command line:

| id | effect | does |
|---|---|---|
| `THEME_LIST` | read-only | every theme: id, label, appearance, source, problems |
| `THEME_GET` | read-only | the choice, appearance, system scheme, the theme showing, revision, tokens (all, or `tokens: ["brand", "ink-*"]`) |
| `THEME_SET` | mutating | the `Pick` layer: `one`, or `light` / `dark`, and/or `appearance` |
| `THEME_PREVIEW` / `THEME_PREVIEW_END` | mutating | the `Preview` layer |
| `THEME_TOKEN_SET` / `THEME_TOKEN_RESET` | mutating | an override in the `Person` layer |
| `THEME_RELOAD` | mutating | the files read again |
| `THEME_PROBLEMS` | read-only | every problem, with its file and field |

Every write answers `{changed, revision}`. None is destructive, since each
is undone by another, so a caller that confirms writes asks for each of
them like any other mutating action.

## In a Dioxus app (feature `dioxus`)

```rust,ignore
let theme = architect_theme::use_theme_store(ThemeStore::default);
// at start, after the dirs: ThemeDirs::new().folder(dir).install_into(&theme.store());
// on the OS's change (a Blitz window's WindowEvent::ThemeChanged):
theme.set_system(Scheme::Dark);
```

`use_theme_store` (`features/theme/src/dioxus.rs` `use_theme_store`)
gives the store to everything below and keeps a signal of the theme
showing. A change from any thread (an action, the watcher) arrives on the
next turn. Below it:

- `AppShell` with no `theme` prop follows the store
  (`ShellTheme::from_resolved`). `WorkspaceView` takes the shell's.
- `ThemeStoreProvider` is architect-ui's `ThemeProvider` whose CSS
  variables are the store's (`ThemeState::from_resolved`).
- `PaintPane` gives its view the whole resolved theme as `ThemeTokens`
  (`features/workspace/src/ctx.rs` `ThemeTokens`, `From<&Resolved>`) and
  requests a frame only when they change (`Painter::set_theme`). A paint
  view reads `ctx.theme.color("brand")`.
- `use_themed::<T>()` is any typed slice as a memo. Its readers rerun only
  when the slice changes.

A host without Dioxus (a REAPER extension, a process extension's guest)
reads `ThemeStore::current` and `subscribe`s.

## A domain app's own roles

architect's tokens are semantic: ground, ink, brand, status. A domain app
names its components' roles on top of them, as tokens of its own: Studio's
arrangement clip, track header, meter ladder, MIDI note, piano key. Two
ways, used together:

1. **Defaults derived in the app.** The app's slice (`struct ArrangeTheme
   { clip, clip_selected, grid, … }`) implements `FromResolved` and
   derives each role from architect's tokens, its own name first:
   `r.get("arrange-clip").unwrap_or_else(|| mix(card, brand, 0.2))`. Every
   theme then paints the arrangement sensibly with no knowledge of it.
2. **Themes that say more.** A theme file sets the role directly in
   `overrides` (`"arrange-clip": "#1e293b"`), and so does a person with
   `THEME_TOKEN_SET`. Nothing in architect needs to know the name.

Frame's theme roles map the same way: a Frame surface reads its roles
from the resolved map, prefixed by the app (`frame-…`), with architect's
tokens as the fallback. Keep role names lowercase-and-dashes and prefixed
by the domain, so two apps' roles never collide in one map.
