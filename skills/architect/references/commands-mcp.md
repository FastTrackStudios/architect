# Commands and MCP: the Action List

The full guide is `docs/content/architecture/commands.md`; this page is what an agent must not get wrong. Code: `crates/architect/src/action_list.rs` (`architect::action`, feature `action-list`), `crates/architect/src/schema.rs` (`json_schema`), `features/mcp` (`architect-mcp`; feature `http` for the HTTP transport), `crates/architect-action-derive/src/lib.rs` (`#[architect::actions]`), `crates/architect-rpc-derive/src/lib.rs` (`#[architect::service(actions)]`).

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `ActionList` | registry, run doors, events, validation | `crates/architect/src/action_list.rs` `ActionList`, `Effect`, `ActionInfo`, `ActionError`, `ActionEvent` |
| Caller | who runs an action, plus one typed context value | `crates/architect/src/action_list.rs` `ActionCaller`, `RawJson` |
| Schema | facet type → JSON Schema text | `crates/architect/src/schema.rs` `json_schema` |
| `#[architect::actions]` | trait of `fn(&self)` → `ActionMeta` consts + `register_<x>_actions` | `crates/architect-action-derive/src/lib.rs` `actions`; `crates/architect-action-derive/tests/smoke.rs` |
| Service projection | every method of a service → one async action | `crates/architect-rpc-derive/src/lib.rs` `service`; `examples/greeter/tests/mcp.rs` |
| Authorize hook | `PermissionEngine::authorize(principal, id)` → `check(principal, "action/<id>", "run")` | `features/permissions/src/lib.rs` `authorize`, `action_resource` |
| MCP server | JSON-RPC over the list: generic and/or named tools | `features/mcp/src/lib.rs` `Server`, `Tools`, `ErrorStyle`, `ToolError`, `serve_stdio` |
| MCP HTTP | `POST /mcp`, origin guard, optional bearer, loopback serve | `features/mcp/src/http.rs` `router`, `serve`, `serve_loopback_or` |
| MCP on `EngineHost` | feature `host`: `.mcp(server, token)` (loopback guard) and `.mcp_at("/org/{slug}/mcp", \|head\| async { Ok(McpTarget::new(server, caller)) / Err(impl IntoResponse) })` (no origin check; the resolver authenticates) | `features/mcp/src/host.rs` `McpHost`, `McpTarget`; `examples/greeter/tests/host_mounts.rs` |
| MCP discovery | `<config>/fts/<name>-mcp.json` and a project's `.mcp.json` | `features/mcp/src/discovery.rs` `publish`, `read`, `unpublish`, `project_mcp_json` |
| Keys | keymap bindings name action ids | `features/input/src/processor.rs` `InputProcessor` (see `docs/content/architecture/input.md`) |
| Telemetry | one `action` span per run, stage timings | `features/telemetry/src/timing.rs` `Stages`, `DIRECTIVES` |

## Registering

```rust
static HITS: ActionMeta = /* id "DRUMS_HITS", label, description, ... */;
list.register(&HITS, Effect::ReadOnly, |q: Query| {
    Ok(Hits { times: hits(&q.track, q.limit.unwrap_or(10)) })
})?;                                         // Result: a duplicate id is Err(Duplicate), first stays
list.register_async(&ASK, Effect::ReadOnly, move |q: Ask| {
    let client = client.clone();
    async move { client.ask(q.name).await.map_err(|e| e.to_string()) }
})?;
list.register_with(&TASKS_LIST, Effect::ReadOnly, |cx: &ActionCaller, a: ListArgs| {
    let Org(org) = cx.context::<Org>().ok_or("no org to act in")?;
    serde_json::to_vec(&org.list(a)?).map(RawJson::from).map_err(|e| e.to_string())
})?;
```

- Handlers return `Result<R, String>`; `R` is a facet type, `()` for none, or `RawJson` (JSON another serializer wrote; checked to be one value, no result schema). Arguments are always facet types.
- `register_dynamic` / `register_dynamic_async` take handlers already erased to JSON.
- `set_enabled(id, || Result<(), String>)` disables with a reason (`ActionInfo::enabled` / `disabled_reason`, `ActionError::Disabled`). Register first: an unknown id is `Unknown`.
- **Ids:** `AREA_VERB`, SCREAMING_SNAKE, starting with a letter. The id is the contract (keymaps, menus, REAPER named commands store it): never rename one. `shortcut` in `ActionMeta` is display-only.
- **Derive path:** `#[architect::actions(namespace = "SESSION")]` on a trait; methods are `&self` only, returning nothing or `Result<(), E: Display>`. They land as `Effect::Mutating` with no args or schema, and a duplicate arriving this way is logged and kept out (only `validate()` reports it). For typed or read-only actions call `register` by hand.
- **A service as actions:** `#[architect::service(actions)]` (or `actions = "NS"`), then `<snake>_actions::register(backend, &list)`. One async action per method, id `<NS>_<METHOD>`, doc comment as description, `http::<Method>Args` as params. Per method `#[action(effect = read_only | mutating | destructive, id, name, category, skip)]`; the effect defaults to `mutating`, so mark readers `read_only`. Needs architect's `action-list` and the HTTP face, so not for `context = T` traits.

## Running

| Door | Who | Authorizer | Async actions |
|---|---|---|---|
| `run_json(id, args)`, `run(id)` | the app itself (keys, menus) | not asked | refused (`Failed`, "run it with run_json_async") |
| `run_json_as(principal, ...)`, `run_as` | a model, a peer | asked | refused |
| `run_json_with(&caller, ...)` | a caller with a context | asked unless `ActionCaller::app()` | refused |
| `run_json_async`, `run_json_async_as`, `run_json_async_with` | anything that can await | as above | run (sync actions too) |

Order inside a run: lookup (`Unknown`) → authorize for non-app callers (`Denied`) → enabled check (`Disabled`) → argument check (one well-formed JSON value, depth ≤ `MAX_ARGS_DEPTH` 128, nothing trailing; else `BadArgs { expected: params schema }`) → handler inside a panic guard (`Failed`, `"panicked: ..."`) → `Ran` event. Empty args become `{}` for an action that takes something, `null` otherwise. Async actions decode their arguments before the future exists.

- `ActionCaller::app()` is trusted; `ActionCaller::new(principal)` is checked; `with_context(value)` sets the one context (a second replaces the first), `context::<T>()` reads it. Keys, menus and `probe_hostile_args` pass no context, so a handler that needs one must refuse without it. Clone the caller into an `async move`.
- **Listing:** `infos()` is unfiltered (the app's view); `infos_for(&who)` / `info_for` show only what `who` may run.
- **Watching:** `changed(topic, detail)` records app state changes; `events_since(since, max, wait)` long-polls; the list keeps `EVENT_HISTORY` (4096) events and gives no gap signal to a watcher that falls further behind.
- **Validation:** `validate()` (duplicate ids, bad id shape, duplicate shortcut, params/result schema not a JSON object) and `probe_hostile_args()` (runs every sync taking action with `{}` and `HOSTILE_ARGS`; async actions are only decoded). It RUNS sync handlers: build the list on test doubles, never on a live app.

## MCP server

```rust
let server = architect_mcp::Server::new("session", env!("CARGO_PKG_VERSION"), list.clone())
    .with_principal(principal)                 // Anonymous unless set
    .with_tools(architect_mcp::Tools::Named)   // or Generic (default) / Both
    .with_tool_name("GREETER_GREET", "greet")  // keep an agent's existing name
    .with_errors(architect_mcp::ErrorStyle::Text);
if let Some(serving) = server.serve_loopback_or(47_811, "FTS_MCP_PORT", "FTS_MCP_TOKEN").await? {
    serving.publish("session")?;               // <config>/fts/session-mcp.json, 0600
}
```

- **Generic tools:** `actions {filter?, category?, effect?}`, `describe {id}`, `run {id, args?}`, `batch {steps, stop_on_error=true}` (≤ `MAX_BATCH_STEPS` 64, not atomic), `watch {since?, max, wait_ms}`; resource `actions://list`. Unknown top-level argument keys are refused. Results are capped at `MAX_TEXT`; page large answers.
- **Named tools:** one tool per action, named its lower-cased id unless `with_tool_name` says otherwise; `inputSchema` is the params schema, hints come from `Effect`. `Tools::Both` lists the generic five first and skips a named tool that collides with one.
- **Per-tool Rust API** (an app with its own MCP endpoint): `server.tools_for(&principal)` gives `tools/list` entries; `server.call_tool(&caller, name, args).await` gives the `tools/call` result. `Err` is only `ToolError::NoTool` or `ToolError::BadParams`; a tool that ran and failed is `Ok` with `isError: true`.
- **Per-request principal:** `handle_as(p, message)` or `handle_with(&caller, message)` (context reaches `register_with` handlers; a caller with no principal runs as the server's, never as the trusted app). `handle_async` is the core; `handle` drives it on the calling thread.
- **stdio:** `serve_stdio(&server)`. **HTTP:** `http::serve(listener, server, token)` or `serve_loopback(_or)`; the app picks the token variable. `project_mcp_json(dir, name, url)` writes the URL (never the token) into a committed `.mcp.json`.
- **Schemas a model reads** are flat (v0.9.24): a documented field is `{"type", "description"}`, an `Option<T>` field is `T` and not required, `#[facet(deny_unknown_fields)]` adds `"additionalProperties": false`.

## Pitfalls

1. `run_json` refuses async actions; a key or menu handler spawns `run_json_async` on the app's runtime.
2. The app doors skip the authorizer by design. Don't assume a key or menu run is authorized, and give the list the same engine as the router's `PermissionsGate` (`set_authorizer`): otherwise there are two planes.
3. The MCP HTTP transport calls `handle`, so every request runs as the server's one principal. The bearer token is a gate, not an identity; per-request principals need your own transport around `handle_as` / `handle_with`.
4. The origin guard checks an `Origin` header only when present, and does not check `Host`. Bind loopback, make serving opt-in (`FTS_MCP=0` turns `serve_loopback` off), and set a token for any app with destructive actions.
5. `batch` with missing or non-array `steps` runs zero steps without an error.
6. `#[architect::service(actions)]` handlers do not see the caller; a per-org service registers by hand with `register_async_with`. A projected failure is the error's `Display`, not the HTTP face's `{code, message}`.
7. `validate()` checks schemas are JSON objects only; it does not prove the decoder and schema agree beyond that.
8. Menu and palette adapters from `ActionInfo` are not in architect (I13). architect-ui `Command*` / `Menubar*` take their own items; Session builds its menu from `infos()` (`studio:apps/session/daw/src/app_menu.rs`). The launcher (`features/launcher/architect-launcher-core`) is a separate catalogue with no architect dependency and no bridge to the list.
9. The queued-window pattern for UI-only effects is Session's (`studio:apps/session/daw/src/actions.rs` `WINDOW`, `take_window`): a programmatic run never opens a dialog.
