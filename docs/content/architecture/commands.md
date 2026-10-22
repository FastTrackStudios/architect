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

pub fn register(list: &ActionList) -> Result<(), ActionError> {
    list.register(&DRUMS_HITS, Effect::ReadOnly, |q: Query| {
        let n = q.limit.unwrap_or(10);
        Ok(Hits { times: hits(&q.track, n) })
    })?;
    list.set_enabled("DRUMS_HITS", || {
        project_open().then_some(()).ok_or_else(|| "no project is open".to_owned())
    })?;
    Ok(())
}
```

- `register::<A, R, _>(meta, effect, |A| -> Result<R, String>)` adds a
  typed action. A second registration of an id is refused with
  `ActionError::Duplicate` and the first stays.
- `register_dynamic(meta, effect, params, result, handler)` adds one
  whose handler is already JSON in, JSON out (an rpc method, a script).
- Existing `#[architect::actions]` traits plug in unchanged: `ActionList`
  implements `ActionBackend`, so the generated
  `register_<trait>_actions(&list, imp)` lands every action in the list
  (as `Mutating`, no args, no result). A duplicate id that comes this
  way is logged and kept out; `validate()` reports it.
- `set_enabled(id, || Result<(), String>)` says when an action can run.
  `Err(why)` disables it: `ActionInfo` shows `enabled: false` and
  `disabled_reason`, and a run answers `ActionError::Disabled`.
- Keep one `register(list)` function per area module and call them all
  from one place at startup. The registry test builds the list and runs
  the list's own checks:

```rust
#[test]
fn the_action_list_is_sound() {
    let list = ActionList::new();
    my_app::actions::register(&list).unwrap();
    list.validate().unwrap();          // unique ids + shortcuts, AREA_VERB ids, schemas are JSON objects
    list.probe_hostile_args().unwrap(); // `{}` never panics; malformed JSON is always refused
}
```

  `probe_hostile_args` runs handlers, so build the list against test
  doubles, not a live app.

## 3. Running

- `run_json(id, args) -> Result<Vec<u8>, ActionError>` is the one door:
  JSON in, JSON out. An action that takes nothing accepts empty input or
  `{}`; one that gives nothing answers `null`.
  It runs as the app itself: the authorizer is not asked.
- `run(id)` is what a key or a menu calls: no arguments.
- `run_json_as(principal, id, args)` / `run_as(principal, id)` run for a
  caller from outside (a model, a peer). With an authorizer set
  (`set_authorizer(engine)`), `PermissionEngine::authorize(principal, id)`
  decides first; its default is `check(principal, "action/<id>", "run")`,
  so existing rule sets answer it.
- `infos()` / `info(id)` describe the list: id, labels, category, group,
  shortcut, effect, `enabled` / `disabled_reason`, and `params_schema` /
  `result_schema` as JSON Schema text. `infos_for(principal)` /
  `info_for(principal, id)` show only what that caller may run.
- `ActionError` is one of `Unknown(id)`, `BadArgs { expected, .. }`
  (carries the params schema so the caller can correct itself),
  `Failed { message }`, `Denied { reason }`, `Disabled { reason }` or
  `Duplicate(id)`.
- A handler that panics is caught and answered as `Failed` with
  `message` `"panicked: ..."` (in `panic = "unwind"` builds, the
  default); the list keeps working.

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

### Async actions

An action that awaits (a service call, a query, a peer) registers with
`register_async`. Its arguments are decoded when it is called, before
the handler runs, so a bad call is refused without running anything:

```rust
list.register_async(&ASK, Effect::ReadOnly, move |q: Ask| {
    let client = client.clone();
    async move { client.ask(q.name).await.map_err(|e| e.to_string()) }
})?;

let out = list.run_json_async("PEER_ASK", br#"{"name":"ada"}"#).await?;
```

- `run_json_async` / `run_json_async_as` run every action, sync or async,
  so a caller that can await (the MCP server, a script) uses them for
  everything.
- The sync `run_json` refuses an async action (`Failed`, "run it with
  run_json_async") rather than block the thread it was called on. A key
  or menu handler spawns `run_json_async` on the app's runtime.
- A panic inside the future is caught and answered as `Failed`, as for
  a sync handler. The enabled check and the authorizer run before the
  future exists.
- `probe_hostile_args` only decodes an async action: its future is
  dropped unpolled, so the probe never does what the action does.
- `register_dynamic_async` takes an `AsyncHandler` already erased to
  JSON in, a future of JSON out.

## 5. MCP: a model on the same list

`architect-mcp` serves an `ActionList` as a Model Context Protocol
server. It offers five generic tools, not one per action (an app has
hundreds):

| Tool | What it does |
|---|---|
| `actions` | the list, filtered by text, category or effect |
| `describe` | one action: what it does, takes and gives (JSON Schema) |
| `run` | run one action with its arguments |
| `batch` | run several in order, stopping at the first failure (at most `MAX_BATCH_STEPS`, 64; not atomic) |
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
- Every call runs as a `Principal`: `with_principal(p)` for the server
  (`Anonymous` unless set), or `handle_as(p, message)` per request. With
  an authorizer on the list, `actions`, `describe` and `actions://list`
  show that principal only what it may run, and `run` and each `batch`
  step are authorized on their own.
- A `batch` is not atomic: steps that ran stay done when a later one
  fails. Over `MAX_BATCH_STEPS` steps it is refused whole.
- `actions` and `describe` carry `enabled` and `disabled_reason`.
- The HTTP face refuses any browser `Origin` that isn't localhost (the
  MCP DNS-rebinding guard) and requires the bearer token when one is
  given. Bind to loopback, make it opt-in, and generate the token.
- Tool results are capped at `MAX_TEXT`. An action that can answer with
  a lot should page.
- Async actions are awaited. `Server::handle_async` is the core;
  `Server::handle` drives it to the end on the calling thread, which is
  what `serve_stdio` and the HTTP transport (on a blocking-pool thread of
  its runtime) do.

### Named tools: keeping an agent's tool names

An agent that already calls an app by tool name (`create_task`,
`greet`) keeps those names when the app moves onto the list. Named mode
offers every action as a tool of its own:

```rust
let server = architect_mcp::Server::new("greeter", "0", list)
    .with_tools(architect_mcp::Tools::Named)
    .with_tool_name("GREETER_GREET", "greet"); // else `greeter_greet`
```

- The tool name is the id in lower case unless `with_tool_name` gives
  one. Two actions on one name: the first by id is offered.
- Its description is the action's, its `inputSchema` the action's params
  schema (an object; a non-object argument is taken as `{"args": …}`),
  and its hints come from the `Effect` (`readOnlyHint`, `destructiveHint`).
- The principal sees only the tools for actions it may run, as with
  `actions`.
- `Tools::Both` offers the five generic tools first and the named ones
  after; an action whose tool name is a generic tool's is not offered
  by name. `Tools::Generic` (the default) is the five alone.

## 6. A service as actions

`#[architect::service(actions)]` projects every method of a service into
an Action List with one call, so keys, menus, scripts and a model run the
same call the vox and HTTP faces serve:

```rust
#[architect::service(actions)]
pub trait Greeter {
    /// Greets a known name.
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    /// How many greetings so far.
    #[action(effect = read_only)]
    async fn count(&self) -> Result<u32, GreetError>;
}

greeter_actions::register(backend.clone(), &list)?; // GREETER_GREET, GREETER_COUNT
```

- Every method becomes an async action. Its id is `<NS>_<METHOD>`, where
  NS is the trait name in `SCREAMING_SNAKE` minus a trailing `Service`
  (`actions = "NS"` sets it). The doc comment is the description,
  `http::<Method>Args` (the HTTP body) is what it takes, and the `Ok` type
  is what it gives. The error's `Display` is the failure message.
- `#[action(effect = read_only | mutating | destructive, id = "…", name =
  "…", category = "…", skip)]` on a method adjusts it. The effect
  defaults to `mutating`, so mark readers `read_only`: a model trusts it.
- `register` goes through the same host the HTTP face uses, so a sync
  backend's dispatcher is honoured. `IDS` lists what it registers.
- It needs architect's `action-list` feature and the HTTP face (so not
  `context = T` yet). The `effect` attribute is refused without `actions`.
- `examples/greeter/tests/mcp.rs` is the reference test: `greet` listed,
  described and run over MCP gives what `POST /greeter/greet` gives, from
  one backend.

## 7. Migrating a trait from `#[architect::rpc]` to `#[architect::service]`

`#[architect::rpc]` is the vox face only. `#[architect::service]` is the
same expansion plus the HTTP face, and with `actions` the Action List as
well. The vox items keep their names, so the switch is a short recipe.

**The trait.**

1. Change the attribute to `#[architect::service]` (or
   `#[architect::service(actions)]`). Keep the rpc options (`sync_client`,
   `ops`, `scopes(...)`, `path = "…"`).
2. Make every `Result` error an `#[architect::error]` enum. The HTTP face
   answers with its `HttpError` status and code, and the clients fold
   transport failures into it through `From<TransportError>`.
3. Check the arguments make a sensible JSON body. Each method's
   arguments become `http::<Method>Args { arg: … }`. An argument named
   `token` may come from `Authorization: Bearer` instead.
4. Check the paths: `POST /<prefix>/<method-kebab>`. The prefix is the
   kebab trait name minus `Service`; `path = "…"` on the trait and
   `#[http(path = "…")]` on a method move them.
5. With `actions`, mark each method's effect. Readers get
   `#[action(effect = read_only)]`, deletes get `destructive`, and
   anything a person must never trigger by accident gets `skip`.

**The backend.** No change for an all-async trait. A sync or mixed trait
is served over HTTP through the dispatcher bridge, so the backend needs
`HasDispatcher` (`#[derive(architect::HasDispatcher)]`).

**The crate's features.** `http` (architect's `http`) compiles the router
and `http-client` compiles `<Trait>HttpClient`. With `actions`, the crate
also needs architect's `action-list`. Mount the router with
`backend.into_http_router()` beside `into_router()`
(`examples/greeter/src/main.rs`).

**The clients.** `<Trait>Client` (vox) is unchanged. For an all-async
trait whose methods all return `Result`, both `<Trait>Client` and
`<Trait>HttpClient` now implement the trait. Delete any hand-written
`impl Trait for TraitClient`: it would conflict. Then code that takes
`impl Trait` can be handed either client, or the backend itself. The
prelude adds `<snake>_http` (and `<snake>_actions` with `actions`).

**The tests.** Write the scenario once as an `async fn` over `impl
Trait`. Run it in-process, over `LocalServer` (vox) and over HTTP, as
`examples/greeter/tests/round_trip.rs` does. With `actions`, add the
registry test (`register`, `validate`, `probe_hostile_args`) and one
named-MCP check that a method answers as its HTTP route does
(`examples/greeter/tests/mcp.rs`). Delete any hand-written axum route,
DTO or MCP tool the generated face now covers, along with its test.

**When to stay `rpc`.**

- The trait has an ambient `context = T`. Neither the HTTP face nor the
  projection can pull `T` out of a request yet.
- Nothing outside the process tree should reach it: a plugin-to-host or
  engine-internal trait, or a REAPER main-thread bridge, where an HTTP
  route would only be surface to guard.
- Its traffic is live signal or bulk bytes: meters, playheads, audio or
  files. Those are `#[subscribe]` streams, `PubSub` or `architect::blob`,
  not JSON request/response.
- Its arguments can't be a JSON body a person or a model would write
  (opaque handles, large binary).

Otherwise prefer `service`, because the HTTP face and the action
projection come from the same declaration.

### Serving on loopback and being found

An app that serves its list live does three things: bind loopback,
say where it is, and name it for Claude Code. `architect-mcp` does all
three, so the app does not hand-roll port, token or file handling:

```rust
// feature `http`; on the app's tokio runtime
let server = architect_mcp::Server::new("session", env!("CARGO_PKG_VERSION"), list.clone());
// 127.0.0.1, port from FTS_MCP_PORT (else 47811), token from FTS_MCP_TOKEN
// when non-empty; Ok(None) when FTS_MCP=0.
if let Some(serving) = server
    .serve_loopback_or(47_811, "FTS_MCP_PORT", "FTS_MCP_TOKEN")
    .await?
{
    // ~/.config/fts/session-mcp.json: {"url": ..., "token": ...}, mode 0600
    serving.publish("session")?;
}
// a dev tool or build step, not every launch: the repo's .mcp.json
architect_mcp::project_mcp_json(repo_root, "session", "http://127.0.0.1:47811/mcp")?;
```

- `serve_loopback(port_env, token_env)` takes any free port when the
  variable is unset; `serve_loopback_or` gives a fixed default, which a
  committed `.mcp.json` needs. A port variable that is not a port is an
  error, as is a port already taken.
- The discovery file `<config>/fts/<name>-mcp.json` is how any client
  finds the port and the token (`architect_mcp::read(name)`). It is
  written to a `0600` temporary file and renamed into place, so the
  token is never readable by another user. Call `unpublish(name)` when
  the app stops serving.
- `.mcp.json` carries the URL only, never the token: it is committed.
  `project_mcp_json` keeps the other servers in the file and writes
  nothing when the entry already matches.
- The token stays optional. Without one, anything on this machine can
  run every action; set a token for an app with destructive actions.

## Rules

1. **One command per capability.** If a person or an agent can do it, it
   is an action in the app's list. Don't add a bespoke MCP tool, menu
   handler or palette entry beside it.
2. **A programmatic run never opens a dialog.** UI-only effects queue a
   window for the UI to open (Session's
   `studio:apps/session/daw/src/actions.rs` is the reference).
3. **Pick the effect honestly.** A model decides what it may run without
   asking from `Effect`.

## Known gaps

These are tracked in the Architect project. Until they land:

- `run_json` (keys, menus) refuses async actions: spawn
  `run_json_async` instead.
- A service's action failure is its error's `Display`, not the HTTP face's
  structured `{code, message}`.
- The router's `PermissionsGate` and the list's authorizer are separate
  planes: give the list the same engine (`set_authorizer`) the router's
  gate uses. Don't expose the list beyond loopback without a token.
- The MCP HTTP face has one principal per server; a transport that
  resolves a caller per request calls `Server::handle_as`.
