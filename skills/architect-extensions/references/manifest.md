# `extension.toml`: the manifest, key by key

What every key of an extension's manifest is, its type, its default, and
what the parser refuses. Derived from the serde structs and their checks:
`features/extensions/src/manifest.rs` `Manifest` (`Manifest::parse`,
`Manifest::check`) and `features/extensions/src/contrib.rs` `Contribution`
(`Contribution::check`).

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` (the long form),
`docs/content/features/extensions.md` (the contributions table),
`docs/content/architecture/embedded-views.md` (view kinds, web views),
`docs/content/architecture/pages.md` (`page =` views),
`docs/content/architecture/extensibility.md`,
`docs/content/architecture/settings-page.md` (`[[sections]]`),
`docs/content/architecture/toolbars.md` (`[[toolbars]]`),
`docs/content/architecture/theming.md` (`[[themes]]`, `[tokens]`).

## A complete sample

Every key below is a real one, and the manifest passes
`Manifest::parse`'s checks (it needs its files beside it to load).
`EXT_CHECK` on a package folder reports what is wrong, field by field.

```toml
id = "acme.metronome"            # a-z 0-9 . - _ ; owns the ACME_METRONOME_ actions
name = "Metronome"
version = "0.2.0"                # semver
kind = "luau"                    # data | luau | typescript | wasm | process
api = "0.1"
description = "A click, and the actions to count in."
authors = ["Acme"]
platforms = ["linux", "macos", "windows", "web"]   # empty: everywhere

[capabilities]
actions = ["TRANSPORT_"]         # action prefixes past its own
effect = "mutating"              # the most its actions may change
reads = ["selection"]
ui = ["notify"]

[[actions]]
id = "ACME_METRONOME_COUNT_IN"
name = "Count in"
group = "Metronome"
shortcut = "⇧⌘M"
params = '{"type":"object","properties":{"bars":{"type":"integer"}}}'
script = "count_in.luau"

[[actions]]
id = "ACME_METRONOME_CLICK"
name = "Click"
effect = "mutating"
toggle = true                    # ticked while it writes state `checked`
script = "click.luau"

[[views]]
id = "panel"
title = "Metronome"
icon = "timer"
page = "pages/panel.tsx"
home = "right"

[[sections]]
id = "metronome"
title = "Metronome"
icon = "timer"

[[sections.settings]]
key = "acme.metronome.accent"    # <id>.<name>
title = "Accent the downbeat"
control = "switch"
default = true

[[hooks]]
on = "startup"
action = "ACME_METRONOME_CLICK"

[[status]]
id = "bpm"
text = "120 bpm"
icon = "timer"
action = "ACME_METRONOME_CLICK"

[[toolbars]]
id = "transport"
title = "Transport"
where = "dock:right"

[[toolbars.items]]
action = "ACME_METRONOME_CLICK"
icon = "timer"
label = "Click"

[[themes]]
path = "themes/metronome-night.json"

[tokens]
"metronome.beat" = "$brand"      # namespace: the id's last part
```

## Unknown keys: mostly ignored, three tables refuse

`Manifest`, `ActionDecl`, `ViewDecl`, every contribution `*Decl` and
`Capabilities` have no `deny_unknown_fields`, so serde **drops a key it
does not know without a word**. A misspelt `plcement = "split"` loads as
the default `tab`. Check spelling against the tables here.

Refused (the manifest does not read, `EXT_CHECK` names the field):
`[[themes]]` (`features/extensions/src/manifest.rs` `ThemeDecl`),
`[source]` (`SourceDecl`) and `[[share_layers]]`
(`features/extensions/src/share_layers.rs` `ShareLayersDecl`).

`[[favourites]]` is read one declaration at a time
(`features/favourites/src/lib.rs` `TypeFavourites`, all
`deny_unknown_fields`): a misshapen one is dropped and reported
(`Manifest::favourites_dropped`, `EXT_CHECK`), never fatal.

A known key with the wrong type or a bad enum value (`effect =
"sometimes"`) is a parse error for the whole manifest.

## Identity

Top-level keys of `features/extensions/src/manifest.rs` `Manifest`.

| key | type | default | rule |
|---|---|---|---|
| `id` | string | required | `a-z 0-9 . - _`, non-empty; owns everything it adds |
| `name` | string | required | |
| `version` | string | required | semver (`Version::parse`) |
| `kind` | `data` \| `luau` \| `typescript` \| `wasm` \| `process` | required | `Kind` |
| `api` | string | `"0.1"` | at `0.x` the minor must equal this build's (`API`, `api_ok`): `"0.2"` is refused |
| `description` | string | `""` | |
| `authors` | string list | `[]` | |
| `platforms` | string list | `[]` (everywhere) | see "Platforms" |
| `reserved` | string list | `[]` | app ids (`APP_…`) it owns when the app allows (`Host::allow_reserved`) |

The action prefix is the id upper-cased with each run of other
characters one `_`: `acme.metronome` → `ACME_METRONOME_`
(`Manifest::action_prefix`). Every `[[actions]] id` starts with it,
unless listed in `reserved`. Example: `features/settings/sheet/extension.toml`
(`reserved = ["APP_SETTINGS"]`).

There is no `publisher` key. Who signed a package is the archive's
signature, not the manifest (`docs/content/architecture/extensions.md`,
"Signing and trust").

## Runtime selection: `kind`, `[entry]`, `[wasm]`, `[source]`

Brief; the runtimes have their own reference file.

| table.key | type | default | for |
|---|---|---|---|
| `entry.path` | string | `""` | a `process` program or a `wasm` component, in the package |
| `entry.args` | string list | `[]` | a process's arguments |
| `wasm.component` | string | `""` | a `wasm` component (wins over `entry.path`) |
| `source.git` | string | `""` | repository of a built-in's crate (`EXT_CLONE`) |
| `source.rev` | string | `""` (`main`) | commit, tag or branch |
| `source.path` | string | `""` | the crate's folder in that repository |
| `source.bin` | string | `""` | the binary; `entry.path`'s file name when empty |

Every package path (`entry.path`, `wasm.component`, `source.path`,
scripts, pages, themes) must stay in the package: no `..`, not absolute
(`ManifestError::BadPath`). `[source]` refuses unknown keys.

Examples: `apps/sandbox/clicker/extension.toml` (process),
`apps/sandbox/wasi-counter/extension.toml` (`[wasm]`),
`extensions/tasks/extension.toml` (`[entry]` + `[source]`),
`examples/showcase/recipe-scaler/extension.toml` (`typescript`),
`examples/extension/webview-board/package/extension.toml` (`data`).

## `[[actions]]`

`features/extensions/src/manifest.rs` `ActionDecl`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required | in its prefix; unique |
| `name` | string | required | |
| `description` | string | `""` | |
| `group` | string | `""` | ⌘K / menu grouping |
| `shortcut` | string | `""` | display and keymap (`⇧⌘M`) |
| `effect` | `read_only` \| `mutating` \| `destructive` | `read_only` | `Reach` |
| `params` | string | none | JSON Schema text |
| `script` | string | none | `.luau` (kind `luau`) or `.ts`/`.js` (kind `typescript`) |
| `preset` | `{ action, args? }` | none | another action with fixed args (`args`: JSON text) |
| `toggle` | bool | `false` | on/off from runtime state of kind `checked` keyed by its id |
| `runs` | `app` \| `node` | `app` | `node`: on the org's node (#583) |
| `secrets` | string list | `[]` | the `[[secrets]]` a node action holds |

Refused: an action with no `script`, no `preset` and not a window's
`opens`, in a `data`, `luau` or `typescript` extension (`NoRun`); a script
its kind does not run (`ScriptKind`). A `runs = "node"` action must be
`luau`, have a `script`, name at least one of its `[[secrets]]`, and be no
preset, toggle or window opener; an app action may not name `secrets`
(`Manifest::check_node_actions`).

```toml
[[actions]]
id = "DEMO_WEBVIEW_BOARD_OPEN"
name = "Open the web view board"
preset = { action = "EXT_VIEW_OPEN", args = '{"extension": "demo.webview-board", "view": "board"}' }
```

Examples: `examples/extension/registry/hello/extension.toml` (script),
`examples/extension/webview-board/package/extension.toml` (preset),
`examples/showcase/bank-link/extension.toml` (`runs = "node"`).

## `[[views]]` and view kinds

`features/extensions/src/manifest.rs` `ViewDecl`. A view's workspace kind
is `<extension id>/<view id>` (`ViewDecl::workspace_kind`).

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required | |
| `title` | string | required | |
| `icon` | string | `""` | Lucide name |
| `kind` | string | `"dioxus"` | the view kind that draws it |
| `document` | string | none | a frame document in the package (a data view) |
| `page` | string | none | TSX/JS, `.luau` elements or `.a2ui.json` page |
| `src` | string | none | the HTML file a `web` view opens; required for `web`, refused otherwise |
| `program` | string | none | the program in the package a `process` view runs; required for `process`, refused otherwise; must be in `capabilities.programs` |
| `args` | string list | `[]` | the `program`'s arguments; only a `process` view has them; `{{setting:<key>}}` puts in one of its own settings (a change restarts the program), never a secret |
| `home` | `main` \| `left` \| `right` \| `bottom` | `main` | where it lives |
| `placement` | `tab` \| `split` \| `overlay` \| `popup` \| `zoomed` \| `float` | `tab` | how it opens |
| `width`, `height` | float (points) | none | a popup's, overlay's or float's size |
| `on_unload` | `placeholder` \| `close` | `placeholder` | its panes when it unloads |

`kind` values the app registers (`features/workspace/src/kinds.rs`
`DIOXUS`, `PAINT`, `WEB`, `PROCESS`): `dioxus` (a process's or linked Dioxus view,
drawn remotely or in the app's document), `paint` (a scene recorded each
frame), `web` (its own HTML/CSS/JS, #222), `process` (a program in the
package draws it on its own GPU device, #838). Apps may register more
(`frame.surface`, `genui`; `docs/content/architecture/embedded-views.md`).
A kind the app has no provider for refuses the load
(`host::LoadError::ViewKind`).

- **Remote views**: the default `dioxus` kind in a `process` or `wasm`
  extension; nothing extra to declare. `apps/sandbox/clicker/extension.toml`.
- **Paint**: `kind = "paint"`. `apps/sandbox/sketch/extension.toml` (`graph`).
- **Scenes** are not a view kind: a remote view's element painted as an
  anyrender scene (`features/extensions/src/scene.rs`). No manifest key.
  `apps/sandbox/sketch/extension.toml` (`sketch`, `arrangement`).
- **Web views**: `kind = "web"` plus `src`.
  `examples/extension/webview-board/package/extension.toml`.
- **Process views**: `kind = "process"` plus `program` (and `args`), any
  extension kind, with `capabilities.programs` naming the program. The
  app's process-view pane runs it from the package and shows its GPU
  frames, whichever copy is loaded; Linux only, elsewhere the pane says
  why (`features/extensions/src/process_view.rs`,
  `docs/content/architecture/process-views.md`).
  `examples/extension/process-view/package/extension.toml`.
- **Page views**: `page = …`. `examples/showcase/recipe-scaler/extension.toml`.
- **Held views** (one extension's view inside another's remote view,
  #764): there is no `embeddable` or `holds` key. The holding view renders
  a placeholder at run time (`features/extensions/src/embedded.rs`
  `VIEW_ATTR`, `EmbeddedView`); the held view runs with its own
  capabilities. Test: `features/extensions/tests/remote_embedded.rs`.

<!-- doc-check: off -->
```toml
[[views]]
id = "board"
title = "Board (web view)"
kind = "web"
src = "dist/index.html"

[[views]]
id = "room"
title = "Room"
kind = "process"
program = "bin/ignition"          # in the package; in capabilities.programs
args = ["--process-view"]
```
<!-- doc-check: on -->

## `[[overlays]]`: layers over another view

`features/extensions/src/manifest.rs` `OverlayDecl`;
`features/extensions/src/overlay.rs`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required | unique among its contributions |
| `view` | string | required | the view kind it draws over (`session.arrange`) |
| `plane` | `under` \| `over` \| `top` | `over` | anything else: `BadPlane` |
| `script` | string | `""` | a Luau canvas script (kind `luau` only) |
| `draws` | string | none | one of its own `[[views]]` |

Exactly one of `script` and `draws`. With `draws`, a `view` not its own
(not starting `<id>/`) needs `capabilities.over` to name it.

```toml
[[overlays]]
id = "ruler"
view = "example.timeline"
plane = "over"
script = "ruler.luau"
```

Examples: `examples/extension/registry/hello/extension.toml` (script);
`draws` only in `features/extensions/tests/remote_overlay.rs`.

## `[[workers]]`

`features/extensions/src/manifest.rs` `WorkerDecl`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required | what a view spawns it by; unique |
| `title` | string | `""` | |
| `script` | string | none | a Luau script (`luau`, `process`) |
| `entry` | string | none | a program's name for it (`process`, `wasm`), or a `.wasm` file in the package |

Exactly one of `script` and `entry` (`Manifest::check_workers`).

```toml
[[workers]]
id = "primes"
title = "Count primes"
script = "workers/primes.luau"
```

Examples: `apps/sandbox/clicker/extension.toml` (script),
`apps/sandbox/wasi-counter/extension.toml` (entry),
`examples/showcase/timers/extension.toml`.

## `[[hooks]]`: schedules, document events, app events

`features/extensions/src/contrib.rs` `HookDecl`.

| key | type | default | notes |
|---|---|---|---|
| `on` | string | `""` | an event; empty for a schedule |
| `action` | string | none | its own action, or one it reaches |
| `script` | string | none | a Luau or TS/JS file (its kind's) |
| `function` | string | none | the function of the table `script` returns |
| `every` | string | none | `30s`, `5m`, `1h30m`, `1d`; at least a second |
| `at` | string | none | `09:30` or five cron fields |

One of `action` or `script`; an event or a schedule, not both.

Events (`features/extensions/src/contrib.rs` `EVENTS`): `startup`,
`workspace.changed`, `doc.opened`, `theme.changed`, `selection.changed`,
`extension.loaded`, `extension.unloaded`, `doc.created`, `doc.saved`,
`doc.renamed`, `doc.deleted`, `app.focused`, `app.blurred`,
`port.message`; plus `changed:<topic>` (any Action List topic) and
`app:<name>` (an app-defined event). `doc.*` hooks need `reads =
["documents"]`; `app:<name>` needs `capabilities.events` to cover the
name.

```toml
[[hooks]]
on = "port.message"
script = "echo.luau"
function = "heard"
```

Examples: `apps/sandbox/extensions/surface/extension.toml`,
`examples/showcase/timers/extension.toml` (schedules),
`examples/showcase/surface/extension.toml`.

**`[background]`** (`features/extensions/src/events.rs` `BackgroundDecl`):
`idle` (duration string, default `"5m"`, at least `1s`). The extension
starts on its first event, stops when idle. Refused for `kind = "data"`.
Example: `apps/sandbox/extensions/surface/extension.toml`.

## `[[sections]]`: settings

`features/extensions/src/manifest.rs` `SectionDecl`, `SettingDecl`,
`ChoiceDecl`. Sugar for `[[contribute."settings.sections"]]`.

| `[[sections]]` key | type | default | notes |
|---|---|---|---|
| `id` | string | required | unique on the settings page |
| `title` | string | required | |
| `icon` | string | `""` | |
| `group` | string | none (`Extensions`) | `""`: the first, untitled group |
| `settings` | table list | `[]` | `[[sections.settings]]` |
| `view` | string | none | one of its `[[views]]` draws the section instead |

| `[[sections.settings]]` key | type | default | notes |
|---|---|---|---|
| `key` | string | required | starts `<id>.` |
| `title` | string | required | |
| `control` | string | required | `switch`, `select`, `segmented`, `cards`, `slider`, `stepper`, `text`, `shortcut` |
| `default` | any | required | bool for `switch`; one of `choices`' values; number for `slider`; integer for `stepper`; text otherwise |
| `description` | string | `""` | |
| `block` | string | none | the titled block it sits in |
| `min`, `max`, `step` | float | 0, 100, 1 | `slider`, `stepper` |
| `unit` | string | `""` | |
| `placeholder` | string | `""` | `text` |
| `choices` | `{ value, label, detail? }` list | `[]` | `select`, `segmented`, `cards` |
| `aliases` | string list | `[]` | search words |
| `scope` | `device` \| `synced` \| `project` | `device` | where the value lives |

```toml
[[sections.settings]]
key = "lab.clicker.step"
title = "Clicks per press"
control = "stepper"
min = 1
max = 10
default = 1
```

Examples: `apps/sandbox/clicker/extension.toml`,
`extensions/calendar/extension.toml` (`choices`),
`examples/showcase/settings-theme/extension.toml`.

## Interface contributions

All in `features/extensions/src/contrib.rs` `Contribution`, flattened into
the manifest's top level. Each action named is its own or one its
`capabilities.actions` reach; each view is one of its `[[views]]`; two of
one kind may not share an id.

| table | keys (type) | example |
|---|---|---|
| `[[openers]]` `OpenerDecl` | `id`, `kind`, `title`, `prefix`, `action`, `open` (strings); `files`, `doc_kinds`, `urls` (string lists; `urls` are regexes); `view` | `examples/showcase/openers-sampler/extension.toml` |
| `[[doc_kinds]]` `DocKindDecl` | `id` (`<app>.<kind>`), `name`, `files`, `icon`, `view`, `schema` (default 1; no migrations). A vault file of its first `files` extension that is a document file of the kind syncs between the person's devices and through the org's node like a note, while the extension is compiled in, installed or loaded in the app's window (a development copy too; gone when it unloads or reloads without it, unless another source still declares it: `DocKindDecl::to_doc_kind`, `apps/desktop/src/node/kinds.rs` `follow`) | `apps/sandbox/board/extension.toml` |
| `[[tabs]]` `TabDecl` | `id`, `title`, `icon`, `view` (all required in effect) | `apps/sandbox/board/extension.toml` |
| `[[status]]` `StatusDecl` | `id`, `text`, `icon`, `hint`, `side` (`left`/`right`, default `right`), `action` | `examples/showcase/toolbar-sampler/extension.toml` |
| `[[menus]]` `MenuDecl` | `menu` (menu title), `action` | `extensions/projects/extension.toml` |
| `[[context]]` `ContextDecl` | `target` (`pane`, `tab`, `view:<kind>`, `file`, `file:<ext>`, `doc:<kind>`, `link`, `selection`, `toolbar`, `toolbar-item`), `action`, `title` | `apps/sandbox/extensions/metronome/extension.toml` |
| `[[bar]]` `BarDecl` | `action`, `title`, `icon`: a phone's action bar | `extensions/calendar/extension.toml` |
| `[[slots]]` `SlotDecl` | `slot` (`ribbon`, `left`, `right`, `tabs`, `status`, `title`), `view` | test: `features/extensions/tests/contributions.rs` |
| `[[keys]]` `KeyDecl` | `chord` (`⌘K`, `A`), `action`, `when` (`view:<view>`: only while that view has the focus) | `features/palette/extension/extension.toml` |
| `[[palette_views]]` `PaletteViewDecl` | `overlay` (`palette` or `quick-open`), `view` | `features/palette/extension/extension.toml` |
| `[[palette_sources]]` `PaletteSourceDecl` | `id` (`a-z 0-9 - _`), `title`, `action`, `run`, `ranking` (`open` default, or `commands`) | `examples/showcase/openers-sampler/extension.toml` |
| `[[keymaps]]` (editor-ext `KeymapEntry`) | `key` (`Mod-Enter`), `action`, `when` (`editor` or `node:<kind>`); needs `writes = ["editor"]` | `apps/sandbox/extensions/outline/extension.toml` |
| `[[slash]]` (editor-ext `SlashEntry`) | `label`, `action`, `group`, `desc`, `icon`; needs `writes = ["editor"]` | `examples/showcase/templater-lite/extension.toml` |
| `[[languages]]` `LanguageDecl` | `id`, `name`, `extensions`, `fences`, `grammar` (`linked`, `linked:<name>`), `highlights`, `comment`, `[languages.server]` `command`/`args` | none in this repo; see `features/editor/src/languages.rs` `Language` |
| `[[tab_kinds]]` `TabKindDecl` | `id`, `title`, `scope` (bool), `width` (`compact`/`wide`), `icon`, `colour`, `subtitle`, `views`, `menu` (`{label, action}` list) | `apps/ui/src/setlists.toml` |
| `[[deck_templates]]` `DeckTemplateDecl` | `context`, `screenset`, `layout` (JSON file) | `apps/ui/src/setlists.toml` |

```toml
[[openers]]
id = "link"
title = "Showcase link"
urls = ['^https://showcase\.example/[A-Z][0-9]+$']
action = "SHOWCASE_OPENERS_URL"
```

An opener with `files`, `doc_kinds` or `urls` claims them and opens with
`view` or `action`; one with none is a ⌘O source and needs `kind` and
`action` (and `open` optionally).

A view that a `[[doc_kinds]]` or a file-claiming `[[openers]]` names is a
**document view** (`Manifest::shows_documents`): each document opens in a
pane of its own, id `<extension>/<view>:<path>` (`ViewOpen::pane_id`), so a
second file opens beside the first and the same file again brings its
pane forward. Any other view has one pane, id its kind. The view reads
which document it shows from its pane's state (`use_pane().state()`).

### Toolbars, the ribbon, favourites

`[[toolbars]]` (`features/extensions/src/toolbar.rs` `ToolbarDecl`):

| key | type | notes |
|---|---|---|
| `id`, `title` | string | |
| `where` | string | `dock:left`, `dock:right`, `dock:bottom`, `float`, `window`, `pane`, `view:<kind>`, `typing`, `hidden` |
| `when` | string | context (`view:note`, `editor:markdown`); empty: every one |
| `hide_labels` | bool | |
| `style` | `{ size, spacing, rounding, preset }` | `preset`: `theme`, `accent`, `soft`, `contrast`, `danger` |
| `items` | `[[toolbars.items]]` (`ToolItem`) | |

`ToolItem` keys: `kind` (`button` default, `separator`, `group`,
`widget`), `action`, `label`, `icon`, `image`, `tint`, `hide_label`, `colors`
(`fill`, `ink`, `on_fill`, `on_ink`, `armed_fill`, `armed_ink`),
`right_click` (`menu`, `arm`, `none`, or an action id), `menu` (rows:
`kind` `action`/`heading`/`separator`/`submenu`, `action`, `label`,
`items`), `items` (a group's), `view` (a widget's), `width`, `height`,
`newline`. An `image` is a file of the extension's package: its folder's,
or, for one linked in, one of its `Extension::files` (`Toolbars::embedded_image`).
An SVG `image` is drawn in the item's ink as an icon is, so it follows the
theme: its `currentColor` is the ink, and with `tint = true` every colour
it was drawn in too (a monochrome glyph in a fixed grey; `Toolbars::tinted`;
`apps/sandbox/sketch/icons/nudge.svg`).
An `icon` is a Lucide name the shell knows (`named_icon`); an unknown one draws
a puzzle piece. Examples: `examples/showcase/toolbar-sampler/extension.toml`,
`apps/sandbox/extensions/metronome/extension.toml`.

The desk's **ribbon** is a shell slot (`[[slots]] slot = "ribbon"`) and
where `[[tabs]]` show; there is no separate ribbon table.

**`[[favourites]]`** (`features/favourites/src/lib.rs` `TypeFavourites`):
`note_type`; `[[favourites.tabs]]` with `id`, `label`, `icon`, `target`;
`[favourites.primary]` with `id`, `label`, `icon`, `target`, `state`,
`on = { label, icon, target? }`. A `target` is exactly one of
`{ view = "…" }`, `{ action = "…" }`, `{ place = "…" }`. At most five
tabs. Test: `features/extensions/src/manifest.rs`
`a_manifest_reads_its_favourites_and_refuses_a_bad_one`; no package in
this repo declares one.

## Code-block fence views: `[[embeds]]`

`features/extensions/src/contrib.rs` `EmbedDecl`, `EmbedClaims`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required in effect | unique |
| `title` | string | `""` | |
| `view` | string | none | one of its `[[views]]` |
| `page` | string | none | a page file in the package |
| `claims` | `{ fence, note_type, embed_type }` string lists | `{}` | at least one claim; matched in any case |
| `max_height` | integer (points) | none | not 0; past it the embed scrolls |

Exactly one of `view` and `page`.

```toml
[[embeds]]
id = "bars"
title = "Bars"
view = "bars"
claims = { fence = ["bars"] }
```

Examples: `examples/showcase/chart/extension.toml` (page, fence),
`apps/sandbox/wasi-counter/extension.toml` (view),
`examples/showcase/dataview-lite/extension.toml`.

## `[[base_views]]`

`features/extensions/src/contrib.rs` `BaseViewDecl`, `BaseViewOption`.

| key | type | notes |
|---|---|---|
| `id` | string | the base's `type:`; not `table`, `cards`, `list` |
| `name`, `icon` | string | |
| `options` | list of `{ key, name, kind, choices, default }` | `kind`: `text` (default), `number`, `toggle`, `property`, `select`; `default` is JSON text |
| `view` | string | one of its `[[views]]` |
| `page` | string | a page file |

At most one of `view` and `page`; neither means its code draws it
(`base.<id>`, `Builder::base_view`).

```toml
[[base_views]]
id = "kanban"
name = "Kanban"
icon = "layout-dashboard"
options = [{ key = "columnBy", name = "Columns", kind = "property", default = '"status"' }]
```

Examples: `extensions/tasks/extension.toml`,
`examples/showcase/openers-sampler/extension.toml`.

## `[[navigator_providers]]`

`features/extensions/src/contrib.rs` `NavigatorProviderDecl`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | required in effect | |
| `title`, `icon` | string | `""` | |
| `views` | string list | `[]` | its views that switch the navigator to it |
| `actions` | string | required in effect | prefix; `<actions>_SNAPSHOT` must be its action |
| `section` | string | `""` (`section`) | `top`, `assets` or `section` |
| `position` | integer | none | order within its section |
| `visible` | bool | `true` | |
| `nest` | bool | `true` | |

```toml
[[navigator_providers]]
id = "projects"
title = "Projects"
icon = "box"
views = ["list", "page"]
actions = "PROJECTS_NAV"
section = "top"
position = 3
```

Example: `extensions/projects/extension.toml`.

## `[[project_types]]`

`features/extensions/src/contrib.rs` `ProjectTypeDecl`,
`ProjectPropertyDecl`.

| key | type | notes |
|---|---|---|
| `id` | string | `a-z 0-9 - _` |
| `name`, `icon`, `description` | string | |
| `properties` | list of `{ key, name, kind, choices, default }` | `kind`: `text`, `number`, `date`, `toggle`, `link`, `list`, `select`; keys unique |
| `template` | string | a markdown file in the package |
| `folders` | string list | made in a new project |
| `views` | string list | its own views a new project opens with |

Examples: `extensions/projects/extension.toml`,
`examples/showcase/openers-sampler/extension.toml`.

## `[[capture_targets]]`

`features/extensions/src/contrib.rs` `CaptureTargetDecl`.

| key | type | notes |
|---|---|---|
| `id` | string | `a-z 0-9 - _`; not `note` or `inbox` |
| `name`, `icon`, `description` | string | |
| `action` | string | makes one from a capture |
| `chips` | string list | properties offered, in order |
| `tokens` | list of `{ prefix, key }` | quick-add words; both required |

```toml
[[capture_targets]]
id = "task"
name = "Task"
icon = "check"
action = "TASKS_CAPTURE"
chips = ["projects", "due", "priority", "tags"]
```

Examples: `extensions/tasks/extension.toml`,
`examples/showcase/openers-sampler/extension.toml`.

## `[[tours]]`

`features/extensions/src/contrib.rs` `TourDecl`: `id` (`a-z 0-9 - _ .`,
equal to the scenario's own id), `title` (required), `description`,
`scenario` (a `.styx` file in the package). The scenario is read and
checked at load; a broken tour fails the load.

Examples: `extensions/projects/extension.toml`,
`extensions/tasks/extension.toml`.

## `[[windows]]`, `[[points]]`, `[[contribute."<point>"]]`

`features/extensions/src/interop.rs` `WindowDecl`, `PointDecl`.

| `[[windows]]` key | type | default | notes |
|---|---|---|---|
| `id` | string | required | unique |
| `view` | string | required | one of its `[[views]]` |
| `title` | string | `""` | |
| `kind` | `sheet` \| `window` | `sheet` | |
| `opens` | string | `""` | one of its `[[actions]]`; the host runs it |
| `width`, `height` | float | none | positive |

| `[[points]]` key | type | default | notes |
|---|---|---|---|
| `id` | string | required | `<its id>.<name>`, once |
| `title`, `description` | string | `""` | |
| `schema` | table or JSON text | none (any object) | each item is checked against it |
| `deliver` | string | none | its action, told `{point, items}` |

`[[contribute."<point>"]]`: any table, checked against that point's
schema when the point's owner loads; a bad item is left out and reported
(`APP_STATUS`), never a failed load
(`features/extensions/src/points.rs`).

Examples: `features/settings/sheet/extension.toml` (windows, points),
`apps/sandbox/extensions/surface/extension.toml`
(`[[contribute."control.surfaces"]]`).

## Services: `[provides]`, `[requires]`, `[uses]`

Maps of service name (lower case, with a `.`) to a version.

| table | value | meaning |
|---|---|---|
| `[provides]` | semver version | method `m` is its action `<PREFIX>M` |
| `[requires]` | semver requirement | loads after a provider; refused without one |
| `[uses]` | semver requirement | optional; calls answer "no provider" when none |

A service may not be in both `[requires]` and `[uses]`, and an extension
may not `[requires]` what it provides. It may `[uses]` it: when several
extensions provide a service, the one chosen (`EXT_SERVICE_USE`) answers,
so a provider can be a client of another's (Keyflow hosts sessions and
follows the `daw` engine's).

```toml
[provides]
"architect.tasks" = "0.1.0"

[uses]
"architect.projects" = "^0.1"
```

Examples: `extensions/tasks/extension.toml`,
`examples/showcase/services/extension.toml` (`[requires]`).

## Themes and tokens

`[[themes]]`: `path` only (unknown keys refused), a `.json` file in the
package. `[tokens]`: name → value, each name in the namespace of the id's
last part with `_` as `-` (`lab.clicker` → `clicker.`), each value a
literal or an expression over the app's tokens (`mix($card, $brand,
0.22)`).

Examples: `apps/sandbox/clicker/extension.toml`,
`examples/showcase/settings-theme/extension.toml`.

## `[[data]]`

`features/extensions/src/manifest.rs` `DataDecl`: `name` (required),
`scope` (`project` default, or `object`): document data it keeps,
namespaced by its id. No package in this repo declares it.

## `[capabilities]`

`features/extensions/src/manifest.rs` `Capabilities`; all keys optional,
all lists default empty. Semantics are in the capabilities reference.

| key | type | values checked at parse |
|---|---|---|
| `actions` | string list | prefixes; `*` for all |
| `effect` | `read_only` \| `mutating` \| `destructive` | default `read_only` |
| `hosts` | string list | network hosts |
| `folders` | string list | |
| `reads` | string list | `org`, `selection`, `documents`, `editor`, `files` |
| `writes` | string list | `documents`, `editor` |
| `ui` | string list | `notify`, `clipboard`, `pickers`, `prompts`, `menus`, `badges`, `overlay`, `engine` (its views' layers read the engine's live values, `features/scene/src/engine.rs`; declined, a binding to one reads nothing) |
| `events` | string list | an app event name, `<prefix>.*` or `*` |
| `ports` | string list | `midi:<name>`, `osc:<host>:<port>`, `osc:listen:<port>`, `osc:*` |
| `languages` | string list | `servers` |
| `webrtc` | string list | `peer-connections` only |
| `device` | string list | `notifications`, `camera`, `photos`, `health:read:<type>`, `health:write:<type>` |
| `over` | string list | view kinds it may draw an overlay on |
| `programs` | string list | paths in the package (no `..`, nothing absolute) its `process` views run |

An unknown value in a checked list is refused (`BadCapability`); an
unknown key in `[capabilities]` is ignored like any other.

## `[[secrets]]`

`features/extensions/src/manifest.rs` `SecretDecl`.

| key | type | default | notes |
|---|---|---|---|
| `id` | string | `""` | `a-z 0-9 . _ -`, non-empty, unique |
| `title` | string | `""` | what the person is asked for |
| `description` | string | none | |
| `hosts` | string list | `[]` (every one of `capabilities.hosts`) | each within `capabilities.hosts` |

A secret needs `capabilities.hosts` non-empty. A request names it as
`{{secret:<id>}}` in a header value.

Examples: `examples/showcase/secrets-fetch/extension.toml`,
`examples/showcase/bank-link/extension.toml`.

## Platforms

`platforms`: values from `features/extensions/src/platform.rs`
`PLATFORMS`: `linux`, `macos`, `windows`, `ios`, `android`, `web`. Empty
means everywhere. An unknown value is kept with a warning
(`Manifest::warnings`). Elsewhere the extension is listed ("macOS only")
and never loaded. Example:
`examples/showcase-registry/packages/mac-only/extension.toml`.

## `[[share_layers]]`

`features/extensions/src/share_layers.rs` `ShareLayersDecl` (unknown keys
refused).

| key | type | notes |
|---|---|---|
| `note_type` | string | required |
| `page` | glob list | served on every link |
| `reference` | glob list | Reference and Full links |
| `full` | glob list | Full links only |

Patterns are relative to the note's folder; `*`, `?`, `**`; at most 64
patterns, 256 bytes and 8 `*` each; no `..`, `.`, leading `/` or `\`.

```toml
[[share_layers]]
note_type = "song"
reference = ["Media/Proxies/Reference*", "Media/Peaks/**", "*.session"]
full = ["Media/Proxies/**"]
```

No package in this repo; tests in `apps/desktop/tests/shares.rs`.

## Not built yet

- `[[overlays]]` that take the pointer in claimed regions: not yet
  (`docs/content/architecture/extensions.md`, "Overlays").
- `grammar = "<file>.wasm"` in `[[languages]]` parses but is refused
  (`Language::check`): grammars are linked into the binary only.
