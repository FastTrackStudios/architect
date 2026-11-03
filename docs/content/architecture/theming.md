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
> follow it. The lab uses it (`apps/lab/ui/src/theme.rs`). An app's own
> namespaced tokens, expressions over other tokens, numbers, provenance
> and `THEME_INSPECT` landed with it (below). Not yet: the Appearance
> settings page (#185) and persisting the layers.

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
the token map, the scheme, the theme's id, where each value came from,
and a revision. Read a token with `get` (an `Option`), `value`, `color`
or `number`. An unknown name answers magenta (`#ff00ff`), or `NaN` from
`number`, and is logged once per name, so a typo shows on screen instead
of quietly falling back. `misses` lists the names asked for and not
found since it was resolved, for tests and an inspector.

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

An override may name any token, these or a new one, and its value may
name other tokens (`mix($border, $foreground, 0.2)`). That is how an app
adds its own tokens (below).

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
`pick`, `preview`, `set_token`, `set_system`, `set_registry`,
`insert_theme`, `remove_theme`, `reload`) composes the layers, resolves
again, and returns `false` when nothing that shows changed. On a real
change it bumps the revision and calls each `subscribe`r once, on the
thread that made the change.

A worker or a render thread reads it without waiting on anyone:
`revision` is an atomic, cheap enough to poll every frame, and `current`
hands out the whole `Resolved` (an `Arc`), never a half-written one. Poll
the revision, and take `current` only when it moved.

A consumer keeps a typed slice and compares it, so it redraws only when
its own part changes (herdr's `Vt::set_theme`): implement `FromResolved`
and hold a `Projected<T>`. Its `update` returns true only when `T`
differs, and its `revision` says which theme it was cut from (a recorded
scene stamps it).

## Theme files (feature `fs`)

A theme file is `<id>.json` in a themes folder: JSON, comments allowed,
read strictly. An unknown field, a palette value that is not a colour, a
token name that is not dot-separated lowercase-and-dashes segments, or an
expression that does not read is a `Problem` naming the file and the field
(`themes/night.json: palette.accent: "teal-ish" is not a colour`).

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
    "arrange.item.edge": "mix($rule, $brand, 0.3)"
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
| `THEME_TOKEN_SET` / `THEME_TOKEN_RESET` | mutating | an override (a literal or an expression) in a `layer`, the `Person` one when left out |
| `THEME_RELOAD` | mutating | the files read again |
| `THEME_PROBLEMS` | read-only | every problem, with its file (or layer) and field; an expression that does not resolve included |
| `THEME_INSPECT` | read-only | each token, or those starting with `prefix`: its value, origin (`derived`, `theme`, `layer`), the layer and theme or layer id that set it, and its expression |

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

## An app's own tokens

architect's tokens are semantic: ground, ink, brand, status. A domain app
names its own on top of them: Studio's arrangement items, track panel,
mixer strips, MIDI notes. They live in the same resolved map, so one
cascade, one inspector and one set of actions cover them, and a person or
an agent can restyle any of them at run time.

**Names.** An app's token is namespaced: segments of lowercase letters,
digits and `-`, separated by `.` (`arrange.item.edge`, `mixer.meter.safe`,
`editor.pitch-0`; `features/theme/src/model.rs` `valid_token`).
architect's own stay one segment. Theme files, layers, `Theme::from_tokens`
and `THEME_TOKEN_SET` all take them.

**Defaults that follow the palette.** A value may name other tokens
(`features/theme/src/expr.rs`):

```text
value := expr | literal            any other CSS value is a literal, kept as is
expr  := $name                     the final value of token `name`
       | mix(arg, arg, t)          two colours mixed in OKLab, t from 0 to 1
       | alpha(arg, t)             a colour with alpha t
arg   := expr | literal            a colour (#fff, rgb(…)) or a number (0.2, 20%)
```

A value is an expression only when the whole of it is one (it starts
with `$`, `mix(` or `alpha(`). Expressions are evaluated last: after
derivation and after every layer's overrides are in, the higher over the
lower. So a later layer's literal wins over a lower layer's expression,
and a reference sees the final value of what it names: a person who sets
`border` moves every app token mixed from `$border`. An unknown name, a
cycle (`$a` → `$b` → `$a`), a value that is not a colour where one goes,
or a `t` outside 0 to 1 draws that token magenta and is a `Problem` naming
the token and the reason (the app layer's `overrides.arrange.a`: `$nope`
names no token), in `ThemeStore::problems` and `THEME_PROBLEMS`. A token that
reads a broken one is magenta too, and says which.

An app ships its defaults as an `App` layer of expressions, so they follow
whatever theme the person picks, and a theme file or a person still says
the last word:

```rust,ignore
store.set(LayerKind::App, Layer::default()
    .token("arrange.item.edge", "mix($border, $foreground, 0.2)")
    .token("arrange.grid.bar", "alpha($ink, 0.18)")
    .token("arrange.cursor.play", "$warn")
    .token("arrange.item.body-alpha", "0.42"));

#[derive(PartialEq)]
struct ArrangePalette { edge: Rgba, grid_bar: Rgba, body_alpha: f32 }

impl FromResolved for ArrangePalette {
    fn from_resolved(r: &Resolved) -> Self {
        Self {
            edge: r.color("arrange.item.edge"),
            grid_bar: r.color("arrange.grid.bar"),
            body_alpha: r.number("arrange.item.body-alpha"), // NaN when missing
        }
    }
}

let mut arrange = Projected::<ArrangePalette>::new(&store.current());
// per frame, on the render thread:
if store.revision() != arrange.revision() && arrange.update(&store.current()) {
    // re-record only what reads the arrangement's slice
}
```

`Resolved::number` reads a plain number (`0.42`, `12`), through a `$ref`
too. A theme's geometry (an item's radius, an edge's width) is numbers in
the app's own units.

**Where a value came from.** `Resolved::source(name)` is a `TokenSource`:
its `origin` (`Derived` from a palette, a `Theme`'s overrides, or a
`Layer`'s), the `layer` (the one that set it, or the one whose choice shows
the theme), the theme or layer `id`, and the `expression` as written when
it was one. `THEME_INSPECT {prefix: "arrange."}` answers the same for an
agent.

**Themes built in memory.** An import from another format (a REAPER
theme) or a document's variables is `Theme::from_tokens(id, scheme,
tokens)`, checked as strictly as a file. `ThemeStore::insert_theme` adds
it or replaces the one of its id while the app runs, and bumps the
revision only when what shows changed. A reload of the files keeps it.

**CSS.** architect-ui writes only architect's (undotted) tokens on the
root (`Resolved::css_tokens`), so app tokens don't flood every document.
An app whose own CSS wants some opts in by namespace:
`ThemeStoreProvider { css_namespaces: vec!["arrange".into()], … }`
writes `arrange.item.edge` as `--arrange-item-edge`
(`Resolved::css_namespaced`, `css_name`). A dotted name whose dashed form
is one of architect's (`ink.2` and `ink-2`) is left out; architect's wins.
Paint views read every token through `ctx.theme`, dotted ones included.

Keep the namespaces the app's own (`arrange.`, `mixer.`, `editor.`), so
two apps' tokens never collide in one map.
