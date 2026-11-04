+++
title = "A2UI: agents' generated UI"
description = "architect-a2ui: an agent shows the person a form, a list, cards or an approval as A2UI v1.0, drawn with architect-ui from a catalog that is the security boundary, kept in Loro, its buttons running Action List actions that ask before every write."
weight = 54
+++

## What it is and when to use it

An agent that wants the person to fill in a form, pick from a list or
approve something sends an [A2UI](https://github.com/a2ui-project/a2ui)
v1.0 surface instead of text. architect draws it with its own components
(architect-ui, the app's theme), in any pane. It is the A2UI payload of the
generative-UI view kind (#226), mounted through the view-kind contract
([Embedded views](@/architecture/embedded-views.md)).

**We don't maintain protocol code.** The messages, JSON-Pointer data
binding, client functions and validation come from
[`a2ui-base`](https://github.com/Liangdi/a2ui) (MIT), pinned exactly
(`a2ui-base = "=0.3.1"` in `features/a2ui/Cargo.toml`). Its umbrella crate
`a2ui` always pulls in a ratatui backend, so we depend on `a2ui-base`
alone. architect owns only three things:

- **The catalog** (`features/a2ui/src/catalog.rs` `CATALOG`), which is the
  security boundary. Only the components it lists are drawn, and only the
  props it lists are read. A component outside it shows as a labelled
  placeholder and nothing of it is drawn. Its JSON Schema
  (`features/a2ui/catalog.schema.json`) and the system prompt a model is
  given (`prompt`, json-render's `catalog.prompt()` idea) are generated
  from it.
- **The state.** Each surface lives in a Loro document
  (`features/a2ui/src/store.rs` `Surfaces`), so it persists, syncs and can
  be co-edited. An `updateDataModel` upsert is a write into a tree of
  Loro maps, and so is a field the person types into.
- **The command path.** A button's `{"event": {"name": "TASKS_ADD"}}` runs
  the Action List action `TASKS_ADD` (`features/a2ui/src/events.rs`
  `press`). It runs as the surface's author, through the list's
  authorizer, asking the person before any write.

Use it for forms, lists, cards, dashboards and approvals that an agent
composes at run time. Use a Frame surface for a control surface, and
sandboxed HTML (the WebView kind, #230) for a visualisation the catalog
can't express.

## A minimal example

The app puts the surfaces and the `A2UI_*` actions on its list, and the
view in its workspace (the lab: `apps/lab/ui/src/lib.rs` `LabApp`,
`apps/lab/ui/src/vault.rs`):

```rust
let surfaces = architect_a2ui::Surfaces::new(crdt::CrdtDoc::ephemeral(), architect_a2ui::store::ROOT);
architect_a2ui::actions::register(&list, &surfaces)?;
architect_a2ui::ui::use_a2ui(surfaces, (*list).clone()); // in the root component
let views = Views::new().view(architect_a2ui::ui::surfaces_view());
list.require_confirmation(CONFIRM_TTL); // and a ConfirmSheet: writes ask
```

An agent then renders, over MCP or from the command line:

```json
{ "messages": [ { "createSurface": {
    "surfaceId": "signup", "catalogId": "https://architect.fasttrackstudio.dev/a2ui/catalog/v1",
    "dataModel": { "name": "" },
    "components": [
      { "id": "root", "component": "Column", "children": ["name", "save"] },
      { "id": "name", "component": "TextField", "label": "Name", "value": { "path": "/name" } },
      { "id": "save", "component": "Button", "child": "label",
        "action": { "event": { "name": "PEOPLE_ADD", "context": { "name": { "path": "/name" } } } } },
      { "id": "label", "component": "Text", "text": "Save" } ] } } ] }
```

```sh
architect-desktop actions run A2UI_RENDER --json - < signup.json
architect-desktop actions run A2UI_PATCH --json '{"messages":[{"updateDataModel":{"surfaceId":"signup","path":"/name","value":"Ada"}}]}'
architect-desktop actions run A2UI_SURFACE --json '{"surfaceId":"signup"}'
```

Pressing Save asks the person first (the confirmation sheet), because
`PEOPLE_ADD` changes something and an agent wrote the button.

## The actions

| id | effect | does |
|---|---|---|
| `A2UI_RENDER` | mutating | applies A2UI messages. A surface it creates replaces any surface with the same id. It answers the surfaces it touched and any component outside the catalog. |
| `A2UI_PATCH` | mutating | applies `updateComponents`, `updateDataModel` and `deleteSurface` to surfaces that already exist |
| `A2UI_SURFACE` | read-only | returns one surface's components and data model (what the person entered), or every surface's id |
| `A2UI_PRESS` | mutating | presses a component the way the person would. A write waits for the person's yes; run it again with the ticket once they approve. |
| `A2UI_CATALOG` | read-only | returns the catalog: its id, its components, its JSON Schema and the prompt |

A batch of messages is applied whole or not at all, as one undoable step
when the store has a history (`Surfaces::with_history`). Over MCP, an
answer that is an object is also the tool result's `structuredContent`
(`features/mcp/src/lib.rs` `text_result`).

## The catalog

`Text`, `Row`, `Column`, `List`, `Card`, `Divider`, `Button`, `CheckBox`,
`TextField`, `Slider`, `ChoicePicker`, `Tabs` and `Image` are A2UI v1.0's
basic names, so an agent written for another A2UI renderer works. Each is
drawn by architect-ui: `Heading`/`Text`, `HStack`/`VStack`-shaped flex,
`Card`, `Divider`, `Button`, `Checkbox`, `Input`/`Textarea`, `Slider`,
buttons for choices, and tabs as buttons over the active child.
`Modal`, `Video`, `AudioPlayer`, `Icon` and `DateTimeInput` are not in it
yet. They show as placeholders.

What an agent can't do from a surface:

- **Run a local function from a button.** `{"functionCall": …}` actions
  are refused. Bindings may still call `a2ui-base`'s pure client
  functions.
- **Reach anything but the Action List.** No network and no other
  command path. `callFunction` and `actionResponse` messages are refused.
- **Name another catalog.** A `createSurface` with another `catalogId` is
  refused with the one this app draws.
- **Write without asking.** Its events run as
  `ActionCaller::confirming_writes`
  (`crates/architect/src/action_list.rs` `ActionCaller`). Such a caller
  gets a confirmation ticket for every mutating or destructive action, and
  where the list asks no one its writes are refused rather than run.
  Read-only actions run at once. A surface the app rendered itself runs
  as the app.

## Code tour

1. `features/a2ui/src/catalog.rs`: `CATALOG` (`ComponentDef`, `PropDef`,
   `Prop`), `schema`, `prompt`, `refusal`.
2. `features/a2ui/src/store.rs`: `Surfaces` (`apply` checks the whole
   batch, then writes it in one step; `get` reads a `Surface` back;
   `set_data`; `subscribe`), `parse` (an array, one object or JSONL;
   a model's smart quotes and trailing commas are fixed by `a2ui-base`'s payload fixer).
3. `features/a2ui/src/events.rs`: `resolve` (the event's action id and
   its context resolved through `a2ui-base`'s `DataContext`), `press`,
   `caller`, `answered`.
4. `features/a2ui/src/actions.rs`: `register`.
5. `features/a2ui/src/ui.rs` (feature `ui`): `use_a2ui`, `A2uiSurface`,
   `draws` (catalog name → architect-ui), `surfaces_view`.

## Tests to read

- `features/a2ui/tests/render.rs`: a 13-component surface rendered with
  `A2UI_RENDER` and drawn headless on Blitz. The test checks that the
  `Script` placeholder shows, that a live patch of the data model and a
  component redraws, that a mutating press waits for the person's yes and
  then runs with the resolved context, and that the data model survives a
  reload from the document's bytes.
- `features/a2ui/tests/catalog_schema.rs`: the committed schema is the
  catalog's (`A2UI_UPDATE_SCHEMA=1` rewrites it), and everything in it is
  drawn and nothing else is.
- `features/a2ui/src/store.rs` tests: the Loro round trip, all-or-nothing
  batches, the catalog refusal.
- `apps/lab/desktop/tests/actions.rs`: `A2UI_RENDER`, `A2UI_PATCH` and
  `A2UI_SURFACE` give the same answers over MCP and the CLI.

## Status

Done (architect#228):

- the catalog and its generated schema and prompt;
- surfaces in Loro;
- events as actions, confirmed;
- the `A2UI_*` actions over MCP and the CLI;
- the renderer on Blitz, and the lab's "Agent UI" view.
- the default build and `ui` compile to `wasm32-unknown-unknown` (CI's
  wasm job checks both, #253). In the browser a `CrdtDoc` is not `Send`,
  so `Surfaces` holds it in a `send_wrapper::SendWrapper` there
  (`features/a2ui/src/store.rs`); the browser has one thread, so the
  Action List's `Send` handlers can still hold the surfaces.

Not yet:

- **Web host.** The renderer builds for the browser, but no web app
  draws it yet: a web lab target rendering a surface from
  `A2UI_RENDER`, its interactive components (`Slider`, `CheckBox`,
  `TextField`, `Tabs`) and the theme's sheets next to an app's own
  Tailwind sheet are checked in a browser by #253's second part. It needs
  `dx` and a `wasm-bindgen` CLI in the dev shell.
- **The lab's persistence.** The lab keeps its surfaces in an in-memory
  document. An app keeps them in one of its own documents (a `DocFile`, a
  synced doc).
- **Opening the pane.** `A2UI_RENDER` does not open the "Agent UI" pane by
  itself yet. The workspace's operations are actions now (#234), so render
  can open it; that is #254.
- **Agent turns and provenance.** Each render as an attributed agent turn
  waits on #227.

**If upstream stalls.** `a2ui-base` has a single maintainer. We track its
releases every two weeks and move the exact pin deliberately. If it stalls
(no release or answer to an issue for a quarter, or a v1.0 change we need
that it doesn't take), fork it as `FastTrackStudios/a2ui` with an `fts`
branch and pin by tag, as with figcraft. Keep the delta small and offer it
upstream. Until then: no fork, no vendored copy.
