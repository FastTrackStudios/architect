# Surfaces: every way a person or an agent reaches an extension

What this covers: each place an extension shows up or can be reached, with its manifest key, its action ids or commands, a real example to copy, and the test that proves it works.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` ("Contributions", "Favourites", "Inline embeds", "The app's interface", "The extensions manager", "Distribution"), `docs/content/architecture/toolbars.md`, `docs/content/architecture/commands.md`, `docs/content/architecture/actions-everywhere.md`, `docs/content/architecture/embedded-views.md`, `docs/content/features/remote-views.md`, `docs/content/guide/cli-and-mcp.md`, `docs/content/guide/commands.md`.

## The one rule underneath

Every surface below runs an **action** on the app's Action List. An extension's `[[actions]]` go there under its namespace: `showcase.toolbar` owns `SHOWCASE_TOOLBAR_*` (`features/extensions/src/manifest.rs` `Manifest::action_prefix`). A contribution may name only its own actions, or ones its `[capabilities] actions` reach (`features/extensions/src/contrib.rs` `Contribution::check`, `Rules::reaches`). If a manifest names anything else, the load is refused, and the refusal shows up in `APP_STATUS`. The app does not crash.

When an extension's action is registered, `features/extensions/src/host.rs` `info` fills it in:
- category: `Extensions`;
- group: the manifest's `name`, unless the action sets `group`;
- description: "From the extension …" when the action has none;
- `shortcut` and `params` come from the action itself.

Every run carries an `Invocation` (`crates/architect/src/action_list.rs` `Invocation`) that says where it came from:
- the source: palette, menu, context menu, key, hook, link, open, agent, script or toolbar;
- the target and its id, the pane, the view, the path, the URL.

A Luau action reads it as `ctx`. So one action can serve every surface, and it can tell which one ran it.

| Surface | Manifest key | Reached by |
|---|---|---|
| ⌘K palette | `[[actions]]`, `[[palette_sources]]`, `[[palette_views]]` | ⌘K; `COMMANDS_RUN`, `COMMANDS_SHOW` |
| ⌘O quick open | `[[openers]]` with `kind` | ⌘O; `COMMANDS_SHOW {"overlay":"quick-open"}` |
| Keys | `[[keys]]`, `[[actions]] shortcut`, `[[keymaps]]` | the keymap; `EXT_PALETTE` lists the keys |
| Actions CLI and control socket | (every action) | `<app> actions run ID`, `architect-actions` |
| MCP | (every action) | the `actions` / `run` tools, or one tool per action under `Tools::Named` |
| Toolbars, ribbon, status, bar, menus, context menus, slots | `[[toolbars]]`, `[[tabs]]`, `[[status]]`, `[[bar]]`, `[[menus]]`, `[[context]]`, `[[slots]]` | clicks; `TOOLBAR_*`, `EXT_CONTRIBUTIONS` |
| Favourites | `[[favourites]]` | a note type's shell slots |
| Notifications | `capabilities.ui = ["notify"]`, `capabilities.device = ["notifications"]` | `host.notify`, `os_notify` |
| Openers | `[[openers]]` with `files`, `doc_kinds` or `urls` | opening a file or link; `EXT_OPEN` |
| Fence views | `[[embeds]]` with `claims.fence` | a ```` ```<lang> ```` block; `EXT_EMBEDS` |
| Remote views' input | `[[views]]` of a process or WASI extension | pointer, wheel, keys, IME, focus |

## ⌘K: the palette

**What it is.** ⌘K and ⌘O are the shell's palette; the extensions' sources, openers and palette views are kept in step with what is loaded by `app-shell:crates/app-shell-extensions/src/palette.rs`, which also serves `COMMANDS_SHOW`. Every enabled action is a palette row, so an extension's `[[actions]]` appear in ⌘K as soon as it loads, with no extra key. If an action has `params`, the palette asks for each argument in turn (`features/commands/src/params.rs`). A `toggle = true` action shows its tick there.

**Manifest keys.**
- `[[palette_sources]]` adds rows of the extension's own, ranked by the palette's engine. Fields:
  - `id`: also the prefix that asks only this source (`show:`);
  - `title`;
  - `action`: given `{query}`, answers `{items: [{id, label, detail?, group?}]}`;
  - `run`: given `{id}` when a row is picked;
  - `ranking`: `open` or `commands`.
- `[[palette_views]]` (`overlay = "palette" | "quick-open"`, `view = …`) is for replacing the palette itself. The first loaded extension that draws an overlay draws it.

**Actions.**

| id | what it does |
|---|---|
| `COMMANDS_RUN {id, args?}` | runs any command by id, a view's own `use_command` ones included; each runs with its own effect (`features/commands/src/bridge.rs` `use_command_actions`) |
| `COMMANDS_LIST` | every command with its effect, enabled or not and why |
| `COMMANDS_SHOW {overlay, query?}` | opens ⌘K or ⌘O with its field set (`"query": "show:"`) |
| `COMMANDS_STATE` | which overlay is open and what its field says |
| `EXT_PALETTE` | the loaded palette views, keys and sources, each with its extension |

An extension's action is itself on the Action List, so `actions run SHOWCASE_TOOLBAR_FOCUS` works directly. You need `COMMANDS_RUN` only for a `use_command` id, which is not an action.

**Example.** `features/extensions/tests/fixtures/examples/showcase/openers-sampler/extension.toml`:

```toml
[[palette_sources]]
id = "show"
title = "Showcase"
action = "SHOWCASE_OPENERS_FIND"   # {query} -> {items: [{id, label, detail, group}]}
run = "SHOWCASE_OPENERS_GO"        # {id}
ranking = "open"
```

**Code.** `app-shell:crates/app-shell-extensions/src/interface.rs` `use_palette` keeps the shell's palette slot in step with what is loaded. `features/commands/src/lib.rs` `RUN_ACTION`, `SHOW_ACTION`.

**Tests.** `features/extensions/tests/interop.rs` `palette_views_keys_and_sources_are_registered_while_loaded` covers three things:
- views, keys and sources are listed by `EXT_PALETTE`;
- they go when the extension is turned off;
- a bad chord, action, overlay, view, id or ranking is refused at parse.

`features/extensions/tests/showcase_content.rs` `the_openers_sampler_finds_opens_and_adds_its_types` covers the source's find and open.

## ⌘O: quick open sources

An `[[openers]]` table with a `kind` and no `files`, `doc_kinds` or `urls` is one of ⌘O's sources. Its fields:
- `kind`: what its items are;
- `title`;
- `prefix`: what a query starts with to ask only this source;
- `action`: given `{query}`, answers `{items: [{id, title, detail?, recency?}]}`;
- `open`: given `{id}` when an item is picked, plus `in` (`here`, `tab`, `split`) when its params name it.

See `features/extensions/src/contrib.rs` `OpenerDecl`. Example: `app-shell:crates/app-shell-extensions/theme-picker/extension.toml` (`kind = "theme"`, `prefix = "theme"`, `THEME_PICKER_FIND` / `THEME_PICKER_PICK`). `COMMANDS_SHOW {"overlay":"quick-open","query":"theme:"}` opens ⌘O on that one source.

## Keys

There are three kinds:

| Key | Fields | Where it fires |
|---|---|---|
| `[[keys]]` | `chord` (`⌘K`, `⇧⌘P`, `⌃O`, `A`), `action`, `when` (empty: everywhere; `view:<view>` or `view:<extension>/<view>`: while that view has the focus) | the app's keymap, after the commands' own shortcuts and the app keymap, under the person's rebinds (`features/extensions/src/contrib.rs` `KeyDecl`) |
| `[[actions]] shortcut` | `shortcut = "Mod-Alt-D"` | the action's own key, shown in ⌘K and the action list (`features/extensions/src/manifest.rs` `ActionDecl`) |
| `[[keymaps]]` | `key` (`Tab`, `Shift-Tab`, `Mod-Enter`), `when` (`editor`, `node:list-item`, `node:table`), `action` | the editor only, while it has the focus and the cursor is in that node; needs `capabilities.writes = ["editor"]` |

`[[keys]]` go to the shell through `commands.set_bound_keys` (everywhere) and `commands.set_view_keys` (`when = "view:…"`, the keymap's `view:<extension>/<view>` context) in `app-shell:crates/app-shell-extensions/src/interface.rs` `use_palette`. A view key holds while a part of that view has the focus: plain letters bound there run their action and never reach the view, linked in, in a process copy or in a `kind = "process"` view alike; a letter typed into a text field in it still types. Use it for a view's own single-key commands (a chart's note input) instead of reading raw keys and keeping a keymap of your own. They are withdrawn when the extension is turned off. The person rebinds any of them in the action list window (`architect_commands::BindingEditor`). The escape hatch, Ctrl+Alt+Shift+Escape, cannot be rebound.

**Examples.**
- `features/extensions/tests/fixtures/examples/showcase/templater-lite/extension.toml`: `SHOWCASE_TEMPLATER_DATE` with `shortcut = "Mod-Alt-D"`, and a `[[slash]]` row for the same action.
- `features/extensions/tests/fixtures/sandbox/extensions/outline`: `[[keymaps]]` for Tab and Shift-Tab.

**Tests.**
- `features/extensions/tests/interop.rs` `palette_views_keys_and_sources_are_registered_while_loaded`: `[[keys]]`.
- `features/extensions/tests/showcase.rs` `templater_lite_declares_its_page_its_slash_row_its_key_and_its_reach`: `shortcut` and `[[slash]]`.
- `features/extensions/tests/editor.rs` `its_slash_row_and_keymaps_are_contributions` and `tab_and_shift_tab_indent_a_list_item_and_move_between_cells`: `[[keymaps]]`.

## The command line

### The extension author CLI: `architect-ext` (npm)

Its real name is the npm package `@fasttrackstudio/architect-cli`, binary `architect-ext`. Source: `features/extensions/web/cli/cli.mjs` (`tsMain` for the TypeScript commands), README `features/extensions/web/cli/README.md`. Node 24 or later.

| Command | What it does |
|---|---|
| `architect-ext new --typescript <id> [dir]` | scaffolds a `kind = "typescript"` extension: a manifest, an action, its types |
| `architect-ext types [dir]` | writes `types/architect.d.ts` and `types/actions.d.ts` (each action's arguments, from its `params`) |
| `architect-ext check [dir]` | checks the manifest and its files, then `tsc --noEmit` (needs `typescript`) |
| `architect-ext pack [dir] [--out F] [--key F]` | writes the `.archext`, signed with a minisign secret key from `--key` or `ARCHITECT_EXT_SIGNING_KEY` (unsigned without one; an app installs only signed archives) |
| `architect-ext keygen <dir>` | a throwaway key pair, `architect-ext.key` and `architect-ext.pub`, never a release key |
| `architect-ext build-web <component.wasm> <out-dir> [--manifest] [--types] [--name] [--budget]` | makes a WASI component ready for a web page: jco, `architect.json`, `architect.lock.json`, typed actions, a size report |

Tests: `features/extensions/web/cli/test/pack.test.mjs`, `features/extensions/web/cli/test/ts.test.mjs`, `features/extensions/web/cli/test/typegen.test.mjs`.

The repo has its own versions of the archive commands as xtasks (`xtask/src/ext.rs`), which the release workflow uses:

```sh
cargo xtask ext-pack <dir> [--out F]        # an unsigned .archext
cargo xtask ext-sign <archive> --key <file> # or EXT_SIGNING_KEY; --expect <key id>
cargo xtask ext-verify <archive>            # opens it as an app would
cargo xtask ext-keygen <dir>                # a throwaway pair
cargo xtask ext-index <registry>            # checks a registry's index.toml
```

`archext install <archive> --id … --version … --sha256 … <out>` checks and unpacks an archive with no app running. It is what Nix's `lib.architectExtension` uses (`features/extensions/archive/src/bin/archext.rs`).

### The actions CLI and the control socket

Every action, extensions' included, is reachable from a shell (`features/control/src/cli.rs` `ActionsCommand`):
- `<app> actions …` reaches the running app, when the app mounts `ActionsCommand` in its own CLI;
- `architect-actions --app <name|socket> …` reaches any running app (`features/control/src/bin/architect-actions.rs`).

```sh
architect-actions --app <app> list --filter metronome [--category Extensions] [--json]
architect-actions --app <app> describe FTS_METRONOME_COUNT_IN
architect-actions --app <app> run FTS_METRONOME_COUNT_IN --arg bars=2
architect-actions --app <app> run EXT_VIEW_OPEN --json '{"extension":"lab.board","view":"board"}'
architect-actions --app <app> batch '[{"id":"EXT_ENABLE","args":{"id":"lab.board"}}]'
architect-actions --app <app> events --follow
architect-actions --app <app> shot /tmp/ext.png [--pane ID]
architect-actions --app <app> apps
```

- The socket is `<runtime dir>/fts/<app>.sock`, one JSON line per request (`features/control/src/endpoint.rs`).
- `--headless` starts the list without a window.
- Exit codes: `3` no such action, `4` bad arguments, `5` not allowed, `7` a person must approve first (rerun with `--ticket`), `69` no app answered.

**The dev loop** that the generated guide teaches (`features/control/src/guide_rules.md`):

```sh
architect-actions --app <app> run EXT_CHECK --arg folder=/path/to/extension
architect-actions --app <app> run APP_STATUS
architect-actions --app <app> shot /tmp/ext.png
```

- `EXT_CHECK {folder}` loads the folder once on its own and renders each view once. It answers `ok`, or the exact manifest field, script line or view that failed (`features/extensions/src/check.rs`).
- `EXT_CLONE {id, as?}` copies a built-in or installed extension into the development folder under a new id. Edit that copy; what the app ships is read-only to you (`features/extensions/src/clone.rs`).
- `EXT_DEV {folder}` loads a folder as a development copy. It shadows other copies and reloads on save.

**The generated `AGENTS.md`.**
- On every start, the app writes `<app dir>/AGENTS.md` and `actions.schema.json` from its live Action List (`features/control/src/guide.rs` `Guide`).
- `Guide::keep_written` rewrites both whenever an action joins or leaves. So an extension's actions appear in the guide while it is loaded, grouped by their id's area.
- Never edit the file: it is overwritten.

Tests:
- `features/extensions/tests/check.rs` `ext_check_and_ext_clone_are_actions_and_a_broken_dev_copy_is_a_problem` and `the_metronome_checks_clean_and_a_broken_copy_says_exactly_why`.

## MCP

There is no extension-specific MCP code. An extension's actions are on the Action List, and the MCP server serves that list (`features/mcp/src/lib.rs` `Server`).

- **Generic tools (the default):** `actions`, `describe`, `run`, `batch`, `watch`. A model finds `FTS_METRONOME_COUNT_IN` with `actions` (filter `metronome` or category `Extensions`), reads its schema with `describe`, and calls `run`.
- **Named tools:** `Server::with_tools(Tools::Named)` lists each action as its own tool, its id in lower case (`FTS_METRONOME_COUNT_IN` becomes `fts_metronome_count_in`), with `params` as the input schema and the effect as hints. `Tools::Both` serves both.
- **Transports:**
  - stdio, bridged to the running app's socket (`features/mcp/src/bridge.rs` `serve_stdio_via`), or over a headless list;
  - loopback HTTP (`features/mcp/src/http.rs` `Server::serve_loopback`; `FTS_MCP=0` turns it off).
- **Join and leave.** An extension's actions come and go with it. A model sees the change through `watch` (`Changed` events), and the next `actions` call reflects it.
- **Rules.** The usual Action List rules apply:
  - a destructive action from outside answers `confirmation_required` with a ticket;
  - an authorizer hides what the caller may not run.

Tests: `features/mcp/tests/parity.rs`, `features/mcp/tests/confirmation.rs`.

## Toolbars, the ribbon, the status bar, menus and context menus

All of these are declared in the manifest, or sent at run time (`ext.contribute{…}` in Luau). They are withdrawn when the extension unloads. `EXT_CONTRIBUTIONS` lists every extension's contributions and runtime state.

| Key | What it adds | Fields |
|---|---|---|
| `[[toolbars]]` + `[[toolbars.items]]` | a toolbar | `id`, `title`, `where` (`dock:left/right/bottom`, `float`, `window`, `pane`, `view:<kind>`, `typing`, `hidden`), `when`, `style`; an item's `kind` (`button`, `separator`, `group`, `widget`), `action`, `icon`, `label`, `right_click` (`menu`, `arm`, `none`, an action id), `menu`, `colors` |
| `[[tabs]]` | an app section showing one of its views (the desk ribbon's icons are the sections) | `id`, `title`, `icon` (Lucide), `view` |
| `[[slots]]` | fills a shell region with one of its views; the latest claim wins | `slot` (`ribbon`, `left`, `right`, `tabs`, `status`, `title`), `view` |
| `[[status]]` | a status-bar item whose text it changes at run time | `id`, `text`, `icon`, `hint`, `side`, `action` |
| `[[bar]]` | a button on a phone's action bar | `action`, `title`, `icon` |
| `[[menus]]` | a row in a named menu-bar menu | `menu` (`View`), `action` |
| `[[context]]` | a row in a target's context menu | `target` (`pane`, `tab`, `view:<kind>`, `file`, `file:<ext>`, `doc:<kind>`, `link`, `selection`, `toolbar`, `toolbar-item`), `action`, `title` |

**Toolbar facts.**
- A button runs its action with source `toolbar` (`app-shell:crates/app-shell-extensions/src/toolbar_ui.rs` `ToolbarStrip`).
- A `toggle = true` action shows on or off. The extension sets it with runtime state of kind `checked`, keyed by the action's id (`ext.state{kind = "checked", key = "…", text = "on"}`).
- `where = "typing"` puts it on a phone's Typing Toolbar over the keyboard, while its `when` is in force (`view:<kind>`, `editor:<context>`) (`app-shell:crates/app-shell-extensions/src/typing_ui.rs`).
- A person's copy is a `*.toml` file in the app's toolbar folder, used in place of the extension's (`features/extensions/src/toolbars.rs` `Toolbars`).
- The actions are `TOOLBAR_LIST`, `TOOLBAR_GET`, `TOOLBAR_SAVE`, `TOOLBAR_RESET`, `TOOLBAR_ARM` / `TOOLBAR_DISARM`, plus the editor's `TOOLBAR_INSERT`, `TOOLBAR_MOVE`, `TOOLBAR_SET` (`features/extensions/src/toolbar_actions.rs`).

**Example.** `features/extensions/tests/fixtures/examples/showcase/toolbar-sampler/extension.toml` has every one of these:

```toml
[[actions]]
id = "SHOWCASE_TOOLBAR_FOCUS"
name = "Toolbar sampler: focus mode"
effect = "mutating"
toggle = true
script = "focus.luau"

[[toolbars]]
id = "showcase"
title = "Showcase"
where = "dock:right"

[[toolbars.items]]
action = "SHOWCASE_TOOLBAR_FOCUS"
icon = "focus"
label = "Focus"

[[status]]
id = "focus"
text = "Focus off"

[[bar]]
action = "SHOWCASE_TOOLBAR_FOCUS"
title = "Focus"
icon = "focus"

[[context]]
target = "file:md"
action = "SHOWCASE_TOOLBAR_WHERE"
title = "Showcase: where is this note"
```

`SHOWCASE_TOOLBAR_WHERE` answers `{source, target, id, path}`: run it from each surface to see the invocation.

**Tests.**
- `features/extensions/tests/showcase_ui.rs` `the_toolbar_samplers_toggle_toolbars_status_and_context_rows` and `the_toolbar_samplers_focus_mode_comes_back_after_a_restart`.
- `features/extensions/tests/toolbars.rs`:
  - `a_toggle_says_whether_it_is_on_and_its_toolbars_are_where_they_say`;
  - `its_entries_on_a_tab_and_a_file_run_told_what_they_were_opened_on`;
  - `a_bad_toolbar_is_a_problem_app_status_lists_not_a_crash`;
  - `a_persons_copy_is_used_in_its_place_and_an_action_arms`;
  - `context_targets_are_points_their_features_declare`.
- `features/extensions/tests/contributions.rs` `an_extensions_contributions_come_and_go_with_it` covers tabs, status, slots and hooks.
- `features/extensions/src/toolbars.rs` `the_typing_toolbar_is_the_editor_s_own_then_those_for_the_focus`.

## Favourites

A note type's favourites are a phone shell's tabs and its one primary button, shown while a note of that type is open. The extension that owns the type declares them as data:

```toml
[[favourites]]
note_type = "setlist"

[[favourites.tabs]]
id = "setlist"
label = "Setlist"
icon = "list-music"
target = { view = "page" }       # or { action = "ID" }, or { place = "#heading" }

[favourites.primary]
id = "play"
label = "Play"
icon = "play"
target = { action = "PLAYER_TOGGLE" }
state = "playing"
on = { label = "Pause", icon = "pause" }
```

- The format is `features/favourites/src/lib.rs` `TypeFavourites`, checked by `TypeFavourites::check`.
- Limits: at most five tabs, labels of at most 24 characters, Lucide icon names.
- A target is the extension's own view, an action it may run that is not destructive, or a `place` relative to the note. Anything else is left out (`architect_favourites::may_run`).
- A node lists every declaration at `GET /_ext/favourites.json`.
- No manifest in this repo declares one; the setlist type is Studio's.

Tests: the unit tests in `features/favourites/src/lib.rs`.

## Notifications

There are two kinds, both host calls the extension must be granted:

| Call | Capability | What it does |
|---|---|---|
| `notify {text, kind?, title?}` | `capabilities.ui = ["notify"]` | an in-app toast (`info`, `success`, `warning`, `error`) and a line in its log |
| `os_notify {id?, title, body?, at_ms?, after_s?}`, `cancel_os_notify {id}` | `capabilities.device = ["notifications"]` | an OS notification, now or scheduled, kept under `<extension>/<id>` and taken back when the extension unloads |

Each runtime makes these calls its own way:
- Luau: `host.notify(text, kind, title)`, or `host.call` for the device ones;
- WIT: `call`;
- from outside: `EXT_HOST_CALL`.

The request type is `features/extensions/src/hostcall.rs` `HostRequest`.

**Example.** `features/extensions/tests/fixtures/examples/showcase/notify-sampler/toast.luau`:

```lua
local text = (args and args.text) or "Hello from the showcase"
host.notify(text, (args and args.kind) or "info", "Notify sampler")
return text
```

Tests:
- `features/extensions/tests/showcase.rs` `the_notify_sampler_toasts_copies_picks_asks_and_opens_a_menu` and `the_notify_sampler_asks_for_nothing_past_the_interface`.
- `features/extensions/tests/host_ui.rs` `every_interface_call_is_refused_outside_its_capabilities`.
- `features/extensions/tests/host_device.rs` `an_extensions_notifications_go_when_it_unloads_or_loses_them`.

## Renderers: how a note type's text reads in the editor

`[[renderers]] note_type = "recipe", action = "X_RENDER"`: while a note of that type is open, the app runs the action with `{text}` (its body) on each change and draws its answer, `{items: [RenderItem]}`, over every line but the caret's, as markdown's live preview. Items (`features/extensions/src/render.rs`): `conceal {from, to}`, `replace {from, to, text, look}`, `mark {from, to, look}`, `line {at, look}`, byte offsets into the text; looks are the app's (`heading`, `strong`, `muted`, `accent`, `chip`, `chip-accent`, `code`), not CSS. The text stays the source of truth: never rewrite it to render it.

## Openers: a file type, a document kind, a link

An `[[openers]]` table that claims something opens it with the extension:
- `files`: extensions without the dot, matched in any case;
- `doc_kinds`: document kinds;
- `urls`: regular expressions over the whole URL.

What it opens shows in its `view`, given the path or URL as the pane's state, or runs its `action`, given `ctx.path` / `ctx.url`. The first loaded opener that claims it wins (`features/extensions/src/host.rs` `Host::open`). Opening a file raises `doc.opened`. In the window, a Ctrl- or Cmd-click on a link goes through `app-shell:crates/app-shell-extensions/src/interface.rs` `ExtensionUi::open_link`. `[[doc_kinds]]` registers a document kind with its file extensions and view.

| id | what it does |
|---|---|
| `EXT_OPEN` | `{path}`, `{url}` or `{doc_kind, path?}` to its opener |
| `EXT_VIEW_OPEN` | `{extension, view, state?, placement?}` |

**Example.**

```toml
[[openers]]
id = "kf"
title = "Keyflow chart"
files = ["kf"]
view = "chart"

[[openers]]
id = "issue"
title = "GitHub issue"
urls = ['^https://github\.com/[^/]+/[^/]+/issues/\d+$']
action = "LAB_BOARD_ISSUE"
```

`features/extensions/tests/fixtures/examples/showcase/openers-sampler/extension.toml` claims `^https://showcase\.example/[A-Z][0-9]+$` with `SHOWCASE_OPENERS_URL`.

Tests: `features/extensions/tests/contributions.rs` `an_extensions_contributions_come_and_go_with_it` (`EXT_OPEN` on `songs/intro.kf`, a URL, and an unclaimed `a.rpp`); `features/extensions/tests/showcase_content.rs` `the_openers_sampler_finds_opens_and_adds_its_types`.

## Code-block fence views (inline embeds)

An extension draws a block of a note in place. The block can be:
- a fenced block, by the first word of its info string;
- a note, by its frontmatter `type:`;
- an `![[…]]` embed, by the type of what it points at.

```toml
[[embeds]]
id = "chart"
title = "Showcase chart"
view = "chart"                           # one of its [[views]]
claims = { fence = ["showcase-chart"] }  # also note_type = [...], embed_type = [...]
max_height = 320                         # points; 480 when unset, then it scrolls
```

(the format is `features/extensions/src/contrib.rs` `EmbedDecl`, `EmbedClaims`, `Claim`.)

- **Host side.** The host asks `features/extensions/src/embeds.rs` `Embeds::resolve` whether a block is claimed, and mounts `EmbedView` with the block's text as `source` and the host's props (the info string, the frontmatter) as `props`. The view redraws when either changes. With no claim, or once the extension goes, the plain block (`fallback`) shows.
- **The view's side.** It reads its input with:
  - `embed()` in Luau;
  - `linked::use_embed` in Rust;
  - `wire::EMBED_EVENT` in a process;
  - `architect_extension_wasi::embed()` in a WASI component.
- **Sizing.** A process or WASI embed is a sub-document sized to its content, between `embeds::MIN_HEIGHT` and `max_height`.
- **Inspecting.** `EXT_EMBEDS` lists every claim, or `{claim: "fence:showcase-chart"}` answers which extension draws one.

Tests:
- `app-shell:crates/app-shell-extensions/tests/remote_view.rs` `a_process_view_inline_as_an_embed_is_given_its_block_and_sized_to_it`.
- `features/extensions/tests/showcase_content.rs` `chart_claims_its_fence_and_counts_a_bases_rows_by_a_property` and `dataview_lite_claims_its_fence_and_answers_each_kind_of_query`.

## Remote input: pointer, wheel, keys, IME, focus

A process or WASI extension's view is a remote view: a Blitz sub-document in the pane (`app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteView`, `RemoteDocument`). It has no manifest key of its own. Every `[[views]]` entry of a process or WASI extension gets this input, from wire 7 (`features/extensions/src/wire.rs` `INPUT_WIRE`). From wire 8 (`BYTES_WIRE`) a process's scene batches cross as bytes beside its edits (`Frame::Batches`, `ProcessExtension::scene_view`; `docs/content/features/remote-views.md`, "On the wire"). A browser pane gets the same events (`features/extensions/src/web.rs` `Pane`).

| Handler | Carries |
|---|---|
| `onclick`, `onmouse*`, `oncontextmenu` | `Pointer`: where, buttons, modifiers |
| `onpointerdown/move/up/cancel/enter…` | `PointerData`: id, type (`mouse`, `pen`, `touch`), pressure, tilt |
| `ontouchstart/move/end/cancel` | `TouchData` |
| `onwheel` | `WheelData`, signed as the web's: a positive `delta_y` scrolls down |
| `onscroll` | `ScrollData` |
| `oncompositionstart/update/end` | the input method's text, then what it committed |
| `onkeydown`, `onkeyup`, `oninput`, `onfocus`, `onblur` | keys and focus |

**Behaviour you can rely on.**
- **Capture.** A press holds the pointer: the moves and the release go to the element pressed, wherever the pointer is. A release lost outside the pane arrives as `pointercancel`, or as `pointerup` if the view has no cancel handler (`RemoteDocument::let_go`).
- **Coalescing.** Moves, wheels, scrolls and composition updates are coalesced while the view is still answering an earlier event (`wire::Coalescer`, at most 64 waiting).
- **IME.** A focused field turns the window's IME on, with the caret area at the pane's place. Focus leaving mid-composition ends it. The view's clipboard is the window's (`remote::HostShell`).
- **Budget.** Past `remote::REMOTE_BUDGET` (33 ms), the pane keeps its last frame with a busy mark. The app is never blocked.
- **Old processes.** A process at wire 6 or below gets none of the newer kinds of event.
- **Inline embeds.** An inline embed's events, focus and keys reach it as they would any sub-document.
- **The Typing Toolbar** (`app-shell:crates/app-shell-extensions/src/typing_ui.rs`) keeps the field's focus through a press.

**Tests.** `app-shell:crates/app-shell-extensions/tests/remote_input.rs`:
- `a_press_holds_the_pointer_and_its_moves_and_release_reach_the_element_pressed`;
- `a_release_lost_outside_the_pane_cancels_the_press`;
- `focus_leaving_mid_composition_ends_it`;
- `a_wheel_reaches_the_view_and_scrolls_its_list`;
- `an_input_method_composes_into_a_field_in_the_view`;
- `a_finger_is_a_touch_and_a_touch_pointer`;
- `moves_while_the_view_is_busy_are_coalesced_to_the_latest`;
- `round_trips_stay_inside_the_remote_budget`.

Run them in `app-shell:crates/app-shell-extensions` with `cargo test --test remote_input`. The browser side is `just ext-pane-browser`.

## What to check before calling a surface done

1. `EXT_CHECK {folder}` answers `ok`.
2. `EXT_CONTRIBUTIONS` (or `EXT_PALETTE`, `EXT_EMBEDS`, `TOOLBAR_LIST`) shows the contribution under the extension's id.
3. Each action runs from the CLI with the arguments the surface would pass.
4. `actions shot` shows it on screen.
5. After `EXT_DISABLE {id}`, the contribution is gone and the plain fallback is back.
