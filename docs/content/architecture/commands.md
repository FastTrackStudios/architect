+++
title = "Commands: the Action List"
description = "One registry of everything an app can do. Keys, menus, the palette, a CLI and a model over MCP all run the same action by id."
weight = 61
+++

An app has one list of the things a person or an agent can do in it,
and every surface runs those things through that list. A key, a menu
item, the command palette, a CLI, a script and a model talking MCP do
not keep their own tables: they look an action up by id in the
**Action List** and run it there. So the same id does the same thing
whoever asks, and adding a command once makes it reachable from all of
them.

Three pieces, all opt-in:

| Piece | Where | Feature |
|---|---|---|
| `ActionList`, `Effect`, `ActionInfo`, `ActionEvent` | `architect::action` (`crates/architect/src/action_list.rs`) | `architect/action-list` |
| `json_schema(shape)` | `architect::schema` (`crates/architect/src/schema.rs`) | `architect/action-list` |
| MCP server over a list | `architect-mcp` (`features/mcp`) | `http` for the HTTP transport |
| Stage timings per action | `architect_telemetry::timing` (`features/telemetry/src/timing.rs`) | native only |

## 1. An action

An action is a stable id plus metadata (`ActionMeta`: id, label,
description, category, group, a display-only shortcut), an `Effect`,
and the facet types it takes and gives back.

- **Id:** `AREA_VERB`, screaming snake case (`SESSION_GAIN_STAGE`).
  The id is the contract: keymaps, menus, REAPER named commands and
  scripts store it. Don't rename one.
- **Metadata** comes from `#[architect::actions]`. Each
  `#[action(...)]` method gets a `pub const <METHOD>: ActionMeta`.
- **Effect** tells a caller, a model above all, what running it does:
  - `ReadOnly`: reads only, safe to run any time.
  - `Mutating`: changes the app, undoably.
  - `Destructive`: an undo may not bring it back (deletes, writes to
    disk, renders over a file).
- **Args / Out** are facet types (`()` for none). The list decodes the
  arguments before the handler sees them and describes both as JSON
  Schema, so a caller can find out exactly what to send.

## 2. Registering

```rust
use architect::action::{ActionList, Effect};
use facet::Facet;

#[derive(Facet)]
struct Query {
    /// The track to read.
    track: String,
    limit: Option<u32>,
}

#[derive(Facet)]
struct Hits {
    times: Vec<f64>,
}

pub fn register(list: &ActionList) {
    list.register(&DRUMS_HITS, Effect::ReadOnly, |q: Query| {
        let n = q.limit.unwrap_or(10);
        Ok(Hits { times: hits(&q.track, n) })
    });
}
```

- `register::<A, R, _>(meta, effect, |A| -> Result<R, String>)` adds a
  typed action.
- `register_dynamic(meta, effect, params, result, handler)` adds one
  whose handler is already JSON in, JSON out (an rpc method, a script).
- Existing `#[architect::actions]` traits plug in unchanged: `ActionList`
  implements `ActionBackend`, so the generated
  `register_<trait>_actions(&list, imp)` lands every action in the list
  (as `Mutating`, no args, no result).
- Keep one `register(list)` function per area module and call them all
  from one place at startup. A registry test then asserts the ids the
  app expects are all there.

## 3. Running

- `run_json(id, args) -> Result<Vec<u8>, ActionError>` is the one door:
  JSON in, JSON out. An action that takes nothing accepts empty input or
  `{}`; one that gives nothing answers `null`.
- `run(id)` is what a key or a menu calls: no arguments.
- `infos()` / `info(id)` describe the list: id, labels, category, group,
  shortcut, effect, and `params_schema` / `result_schema` as JSON Schema
  text.
- `ActionError` is one of `Unknown(id)`, `BadArgs { expected, .. }`
  (carries the params schema so the caller can correct itself) or
  `Failed { message }`.

Every run is one `tracing` span named `action` with `action.id` and
`action.ok`. Stages marked inside it with
`architect_telemetry::timing::Stages` (`edit.detect_ms`, ...) land on
that span. The `Recorder` layer keeps finished spans in a ring buffer the
app reads back with `timing::recent(n)`. Add `timing::DIRECTIVES` to the
global `EnvFilter`, or under `RUST_LOG=warn` no action is timed.

## 4. Watching

Each run, and each `list.changed(topic, detail)` the app reports
("project opened", "track renamed"), is an `ActionEvent` with a
sequence number. `events_since(seq, max, wait)` returns what came after
`seq`, waiting up to `wait` for the next event when none has arrived
yet. Long-polls and the MCP `watch` tool are built on it. The list keeps
the last `EVENT_HISTORY` (4096) events.

## 5. MCP: a model on the same list

`architect-mcp` serves an `ActionList` as a Model Context Protocol
server. It offers five generic tools, not one per action (an app has
hundreds):

| Tool | What it does |
|---|---|
| `actions` | the list, filtered by text, category or effect |
| `describe` | one action: what it does, takes and gives (JSON Schema) |
| `run` | run one action with its arguments |
| `batch` | run several in order, stopping at the first failure |
| `watch` | runs and state changes since a point, long-polling |

```rust
let server = architect_mcp::Server::new("session", env!("CARGO_PKG_VERSION"), list.clone())
    .with_instructions("Session's commands. Call `actions` first.");

// stdio, for a model that launches the app:
architect_mcp::serve_stdio(&server)?;

// or live, on loopback (feature `http`): POST /mcp, bearer token
let listener = tokio::net::TcpListener::bind("127.0.0.1:47811").await?;
tokio::spawn(architect_mcp::http::serve(listener, server, Some(token)));
```

- `Server::handle(message)` is the whole protocol (one JSON-RPC message
  in, the reply out), so other transports are a thin loop around it.
- The HTTP face refuses any browser `Origin` that isn't localhost (the
  MCP DNS-rebinding guard) and requires the bearer token when one is
  given. Bind to loopback, make it opt-in, and generate the token.
- Tool results are capped at `MAX_TEXT`. An action that can answer with
  a lot should page.

## Rules

1. **One command per capability.** If a person or an agent can do it, it
   is an action in the app's list. Don't add a bespoke MCP tool, menu
   handler or palette entry beside it.
2. **A programmatic run never opens a dialog.** UI-only effects queue a
   window for the UI to open (Session's
   `apps/session/daw/src/actions.rs` is the reference).
3. **Pick the effect honestly.** A model decides what it may run without
   asking from `Effect`.

## Known gaps

These are tracked in the Architect project. Until they land:

- Registering an id twice silently **replaces** the first action.
- No enabled/disabled state with a reason, no authorize hook, no panic
  guard, and handlers are sync only.
- The router's `PermissionsGate` does not cover in-process
  `ActionList` runs. Don't expose the list beyond loopback without a
  token.
