# architect-navigator-api

The navigator's provider API, **MIT OR Apache-2.0**: what a page or an extension implements to
fill the app's one left sidebar, the navigator, with its own containers and items (#356).

The navigator itself (`architect-navigator`, a port of Notebook Navigator) is GPL-3.0-or-later.
This crate is the boundary: an MIT crate implements `NavigatorProvider` from here and never links
the GPL one. Only the app that draws the navigator links it.

- `NavigatorProvider`: `snapshot()` (the title, the `Node` tree and every `Item`), and optional
  `open`, `move_items`, `create`.
- `Method`, `serve`: the provider as actions (`<PREFIX>_SNAPSHOT`, `_OPEN`, `_MOVE`, `_CREATE`),
  what `architect_extensions::linked::Builder::navigator` registers.
- `ActionProvider`: a provider read through its actions, how the app reads every one.

An extension declares it in its manifest (`[[navigator_providers]]`). How to use it:
`skills/architect/references/navigator.md` ("Providers") and
`docs/content/architecture/extensions.md` ("Navigator providers"). Examples:
`extensions/projects/src/nav.rs`, `extensions/tasks/src/nav.rs`.
