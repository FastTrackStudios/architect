+++
title = "Extensions"
description = "architect-extensions: packages that add actions, settings, views and document data at run time; data, Luau and process kinds; git, folder and ReaPack sources; hot reload; a project's required extensions under an org's policy."
weight = 51
+++

## What it is and when to use it

An extension adds actions, settings sections, views, openers, keys and
document data to an app while it runs. It is a folder with an
`extension.toml` and one of four kinds: `data` (declarations only), `luau`
(actions as [scripts](@/features/scripting.md)), `process` (its own program,
over vox, see [Remote views](@/features/remote-views.md)) or `wasm` (planned).
Each `[[views]]` entry names the view kind that draws it (`kind`, `dioxus`
when it names none); a kind the app has no provider for refuses the load
with the reason (see [Embedded views](@/architecture/embedded-views.md)).
Extensions install from git registries, folder registries or ReaPack
indexes; a built-in can be shadowed by a newer install or a development copy;
editing a loaded extension reloads it as a diff, and a broken save keeps the
last good copy. Each extension's capabilities bound what its actions may do.
A project names the extensions it needs; opening it plans them against the
org's approved list.

Use it for optional and third-party features that should not need a rebuild.
Built-in features are crates. Design: [Extensions](@/architecture/extensions.md);
the build-time side is [Extensibility](@/architecture/extensibility.md).

Example: `examples/extension`, a registry with a Luau extension and a process extension loaded by a small host, hot-reloaded on save; walked through in [Write an extension, end to end](@/getting-started/extension.md).

<!-- media: extension-install-reload — installing the metronome from the registry: its section appearing on the settings page and its action in ⌘K; editing its manifest and the section reloading -->
<!-- media: extension-project-requirements — opening a project that needs an extension: the prompt offering to install it, then the extension loaded -->

## A minimal example

The manifest, from `apps/sandbox/extensions/metronome/extension.toml`:

```toml
id = "fts.metronome"
name = "Metronome"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "FTS_METRONOME_COUNT_IN"
name = "Metronome: count in"
params = '{"type":"object","properties":{"bars":{"type":"integer","minimum":1}}}'
script = "count_in.luau"

[[sections]]
id = "metronome"
title = "Metronome"

[[sections.settings]]
key = "fts.metronome.accent"
title = "Accent the downbeat"
control = "switch"
default = true
```

The host side, `architect-extensions` with `host`, after
`apps/ui/src/extensions.rs`:

```rust
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::VersionReq;

let sections = SharedSections::new(architect_settings::Registry::new());
let ext = Extensions::new(Host::new(list.clone(), sections), Installs::new(app_dir.join("extensions")))
    .with_source(FolderRegistry::new(registry_dir));   // or GitRegistry, ReaPackSource
ext.install_actions()?;                                // EXT_LIST, EXT_SEARCH, EXT_INSTALL, ..
ext.install("fts.metronome", &VersionReq::STAR)?;
// On a timer (the sandbox polls every 500 ms): ext.reload_changed();
```

## Contributions

Besides actions, settings sections, views and themes, an extension adds to the
app's interface with contributions: tables in its manifest, or the same sent
at run time (a process guest, a Luau script's `ext.contribute{…}`, a linked
extension's builder). All of them are one type,
`features/extensions/src/contrib.rs` `Contribution`, checked against the
extension's namespace, capabilities and views when it loads
(`Contribution::check`), and withdrawn when it unloads
(`Contributions::withdraw`). Each kind:

| table | what it adds | type |
|---|---|---|
| `[[openers]]` | a claim on files by extension (`files`), on document kinds or on links by URL pattern (`urls`), opened in one of its views or by an action; with `kind` and no claims, one of ⌘O's sources | `OpenerDecl` |
| `[[doc_kinds]]` | a document kind, its file extensions and the view that shows it, registered while loaded | `DocKindDecl` |
| `[[tabs]]` | an app section (a tab with a title and a lucide icon) showing one of its views | `TabDecl` |
| `[[status]]` | a status-bar item on the left or right; its text changes at run time | `StatusDecl` |
| `[[menus]]` | a row in a named menu of the menu bar, running one of its actions | `MenuDecl` |
| `[[context]]` | a row in a target's context menu: `pane`, `tab`, `view:<kind>`, `file`, `file:<ext>`, `doc:<kind>`, `link`, `selection` | `ContextDecl` |
| `[[bar]]` | a button on a phone's action bar | `BarDecl` |
| `[[toolbars]]` | a toolbar of buttons, separators, groups, dropdowns and widgets, in a dock, a float, a window, a pane or a view kind's header | `features/extensions/src/toolbar.rs` `ToolbarDecl` |
| `[[hooks]]` | an action or Luau script run on an app event: `startup`, `workspace.changed`, `doc.opened`, `theme.changed`, `selection.changed`, `changed:<topic>` | `HookDecl` |
| `[[slots]]` | a shell region (`ribbon`, `left`, `right`, `tabs`, `status`, `title`) filled with one of its views; the latest claim wins | `SlotDecl` |
| `[[embeds]]` | an inline block of a document drawn by one of its views: a fenced block by language, a note by its `type:`, an embed by what it points at | `EmbedDecl` |
| `[[base_views]]` | a view type for Bases (`type:` of a base's view), drawn by one of its views, a page or its code | `BaseViewDecl` |
| `[[project_types]]` | a type of architect's `project` kind: properties on top of the kind's, a `project.md` template, folders and views; the registry `EXT_PROJECT_TYPES` reads | `ProjectTypeDecl` |
| `[[capture_targets]]` | what a capture can become: a choice in the composer and the inbox deck, its chips, its quick-add tokens and the action that makes one; the registry `EXT_CAPTURE_TARGETS` reads | `CaptureTargetDecl` |
| `[[keymaps]]` | keys in the live editor that run its actions (needs `writes = ["editor"]`) | `editor_ext::KeymapEntry` |
| `[[slash]]` | rows of the editor's slash menu that run its actions (needs `writes = ["editor"]`) | `editor_ext::SlashEntry` |

Where a view opens is the view's own `placement` (`tab`, `split`, `overlay`,
`popup`, `zoomed`, `float`: `features/extensions/src/manifest.rs`
`ViewPlacement`); where it lives is its `home`. The app asks
`Contributions` (`Host::contributions`) for what to draw: `tabs`, `status`,
`menus`, `context`, `bar`, `toolbars`, `slots`, `open_path`, `open_url`.
The full manifest syntax, with an example of each table, is in
[Extensions: contributions](@/architecture/extensions.md).

## Code tour

1. `features/extensions/src/manifest.rs` `Manifest` (`parse`, `check`,
   `action_prefix`, `sections`), `Kind`, `Capabilities`, `ActionDecl`,
   `ViewDecl`, `OverlayDecl`.
2. `features/extensions/src/host.rs` `Runner`, `Instance`, `DataRunner`,
   `LuauRunner`, `gate`: how each kind starts, and capabilities as a gate on
   what its scripts may run.
3. `features/extensions/src/host.rs` `Host` (`load`, `unload`): loads a
   package as a diff of actions and sections; on failure the last good copy stays.
4. `features/extensions/src/install.rs` `Source`, `FolderRegistry`,
   `GitRegistry`, `ReaPackSource`, `Installs`.
5. `features/extensions/src/catalog.rs` `choose`: which copy runs (built-in,
   installed or development).
6. `features/extensions/src/extensions.rs` `Extensions` (`install`,
   `uninstall`, `dev`, `disable`, `enable`, `sync`, `reload_changed`, `open`,
   `accept`): ties it together.
7. `features/extensions/src/project.rs` `Requirements`, `Policy`, `plan`: what
   a project needs, what the org allows, what to install or offer.
8. `features/extensions/src/actions.rs`: the `EXT_*` actions.
9. `features/extensions/src/doc.rs`: a project's requirements and an
   extension's data in its document (feature `crdt`).

`EXT_INSTALL { id }` calls `Extensions::install`: find the newest listing that
matches the range and the policy, install it under `Installs`, then `sync`.
`sync` runs `choose` over the copies and `Host::load` on the winner. `load`
starts the kind's runner, builds each action's handler, refuses an id the app
already owns, pushes the settings sections, and swaps the actions in as a
diff. Later, `reload_changed` compares the folders' fingerprints and reloads
only the extensions whose files changed.

## Tests to read

- `features/extensions/tests/metronome.rs`: the sandbox metronome installs from a
  git registry, reloads on save keeping what didn't change, keeps the last
  good copy after a broken save, and takes its settings with it on uninstall;
  a built-in shadowed by a development copy, disabled and enabled; a project's
  requirements installed only when the policy approves them; capabilities
  refusing a mutating action; an app's own action id refused.
- `features/canvas/tests/marker_lane.rs`: an extension overlay inside its capabilities.
- `features/extensions/tests/remote_view.rs`: the process kind.

## Status

Done in architect for the data, Luau and process kinds; the sandbox runs the
metronome and the clicker. Not yet: WASM runners; the host starting
`[[overlays]]` by itself (they are parsed and checked, and run today as
[canvas](@/features/canvas.md) scripts the app starts); signed archives; a
CLI for development copies (`Extensions::dev` is the API).

- **Task** and **Studio** do not use it yet.
