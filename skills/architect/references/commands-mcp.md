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
| Shared shape | the requests both faces take and the answers both give: `Method` (`TOOLS` = MCP's generic tools), `Request::parse`, `Answer`, `Failure` + `Code` (→ exit codes) | `features/control/src/protocol.rs`; `features/control/src/surface.rs` `Surface` (`answer`, `listing`, `schema`, `api_schema`) |
| Control socket | feature `server`: the app's user-only socket (`0600`; a named pipe on Windows), NDJSON `{id, method, params}` → `{id, result}` / `{id, error}` | `features/control/src/server.rs` `Control` (`new`, `at`, `with_caller`, `serve`, `serve_if_wanted`), `Serving`, `Peer`; `features/control/src/endpoint.rs` `socket_path`, `wanted` (`FTS_CONTROL`) |
| CLI | feature `cli`: `actions list/describe/run/batch/schema/events/apps`, `--app`, `--headless`; the generic client `architect-actions` | `features/control/src/cli.rs` `Cli`, `ActionsArgs`, `Headless`, `ListOnly`; `features/control/src/client.rs` `Client` |
| MCP over the socket | feature `bridge`: MCP on stdio for a running app | `features/mcp/src/bridge.rs` `serve_stdio_via`, `Bridge` |
| Parity | feature `parity`: the same set and answers over MCP and the socket, for an app's list | `features/mcp/src/parity.rs` `Parity`; `features/mcp/tests/parity.rs`; `cargo xtask surfaces` (`xtask/src/surfaces.rs`) |
| Workspace | every layout operation as an action with an explicit target; a headless host for servers and tests | `features/workspace/src/control/mod.rs` (`State`, `Env`, `Answer`, `Reason`, one fn per action), `features/workspace/src/control/list.rs` `register`, `HeadlessWorkspace`, `ids`; `features/workspace/tests/actions.rs` |
| Extensions | Run-time actions and settings from packages: `EXT_LIST` / `EXT_SEARCH` / `EXT_INSTALL` / `EXT_UNINSTALL` / `EXT_RELOAD`; an extension's actions under its id's prefix | `features/extensions/src/actions.rs`; `features/extensions/src/host.rs` `Host`; `features/extensions/tests/metronome.rs` |
| Keys | keymap bindings name action ids; an app's `Commands` runs them in the live contexts, the person's layer over the app's | `features/input/src/processor.rs` `InputProcessor`, `features/commands/src/keymap.rs` `Keymap` (see `docs/content/architecture/input.md`) |
| Scripts | Luau over the list: `SCRIPT_RUN` / `SCRIPT_SAVE` / `SCRIPT_DESCRIBE` (and `SCRIPT_LIST`, `SCRIPT_DELETE`, `_ORG` forms); each saved script an action `SCRIPT_MY_<NAME>` / `SCRIPT_ORG_<NAME>`; a run is one undo step | `features/scripting/src/actions.rs` `Scripting`; `features/scripting/src/luau.rs` `Runtime`, `Report`; `features/scripting/tests/scripts.rs` |
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
- `set_enabled(id, || Result<(), String>)` disables with a reason (`ActionInfo::enabled` / `disabled_reason`, `ActionError::Disabled`). `set_checked(id, || bool)` makes it a toggle (`ActionInfo::checked`, `None` when it is not one; the menus tick it). Register first: an unknown id is `Unknown`.
- `unregister(id)` takes an action off the list (an `Unregistered` event; the id is free again): for actions that come and go, such as saved scripts.
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

Order inside a run: lookup (`Unknown`) → authorize for non-app callers (`Denied`) → enabled check (`Disabled`) → confirmation for non-app callers when on (`ConfirmationRequired`, or `Denied` for a bad ticket; below) → argument check (one well-formed JSON value, depth ≤ `MAX_ARGS_DEPTH` 128, nothing trailing; else `BadArgs { expected: params schema }`) → handler inside a panic guard (`Failed`, `"panicked: ..."`) → `Ran` event. Empty args become `{}` for an action that takes something, `null` otherwise. Async actions decode their arguments before the future exists.

- `ActionCaller::app()` is trusted; `ActionCaller::new(principal)` is checked; `with_context(value)` sets the one context (a second replaces the first), `context::<T>()` reads it. Keys, menus and `probe_hostile_args` pass no context, so a handler that needs one must refuse without it. Clone the caller into an `async move`.
- **Confirmation tickets** (`crates/architect/src/action_list/confirm.rs`): off until `list.require_confirmation(CONFIRM_TTL)` — only an app with a person and a sheet turns it on (a headless server would leave agents waiting). Then a caller with a principal running an `Effect::Destructive` action, or a call `set_confirm(id, |args| …)` says needs it (overrides the effect per call: a mutating action that should ask, a command runner asking only for some commands), gets `ActionError::ConfirmationRequired { id, ticket, expires_in }`. The person sees `pending_confirmations()` (`Confirmation`: action, JSON args, `caller` as `Principal::describe`, time left) and answers with `approve(ticket)` / `deny(ticket)` — the app's own door, never put on the list. The caller retries with `ActionCaller::with_ticket(t)`: one run of exactly that call (same caller, id, args) before expiry; a reused, expired, denied, made-up or mismatched ticket is `Denied` with why, a still-pending one answers `ConfirmationRequired` again. The app itself (`ActionCaller::app()`: keys, menus, palette) is never asked. A caller made `ActionCaller::confirming_writes()` (an agent's generated UI: `architect-a2ui`'s button presses) asks before every non-read-only call, whatever the action's effect or `set_confirm`; with confirmation off its writes are `Denied`, never run unasked. Every ticket change is `Changed { topic: CONFIRM_TOPIC ("confirmation"), detail: "<ticket> pending|approved|denied" }`. The sheet is `architect_commands::ConfirmSheet` (feature `ui`, reads `Arc<ActionList>` from context); the lab wires it in `apps/lab/ui/src/confirm.rs`.
- **Commands carry their effect** (`features/commands/src/palette.rs` `Effect`; `Command::effect`, `.destructive()`, `.read_only()`; `Mutating` unless said; an action's comes from its `ActionInfo`; a custom action takes its most far-reaching step's). `COMMANDS_RUN` (`features/commands/src/bridge.rs` `use_command_actions`) is no blanket write: its `set_confirm` check gives a destructive command (or destructive action) a ticket, and an Action List action run through it is authorised as itself (refused `Failed`, "not allowed"). `COMMANDS_LIST` (read-only, `LIST_ACTION`) lists every command with `effect` (`read-only` / `mutating` / `destructive`), enabled and why not; read it before running a command. `COMMANDS_SHOW` (`SHOW_ACTION`, `ShowCommands`) opens the palette or ⌘O with its field set; `{"overlay": "quick-open", "query": "theme:"}` lists one opener's items alone (`Commands::show_quick_open`); `Commands::register_opener` / `remove_opener` keep ⌘O's openers past a component's life (an extension's `[[openers]] kind`). The sheet names the inner command of a `COMMANDS_RUN` call. Mark a command `.destructive()` instead of a `set_confirm` on `COMMANDS_RUN` (`apps/lab/ui/src/confirm.rs`).
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
- **Structured results:** a tool that ran and answered a JSON object (within `MAX_TEXT`) also carries it as `structuredContent`, beside the text block (`features/mcp/src/lib.rs` `text_result`).
- **Confirmation:** a `ConfirmationRequired` is `isError: true` with `confirmation_required: true`, `ticket`, `expires_in_ms` (text style names the ticket). Retry with `ticket` on `run` and on a `batch` step, `_ticket` (`NAMED_TICKET`) among a named tool's arguments; `watch` hears the answer.
- **Named tools:** one tool per action, named its lower-cased id unless `with_tool_name` says otherwise; `inputSchema` is the params schema, hints come from `Effect`. `Tools::Both` lists the generic five first and skips a named tool that collides with one.
- **Per-tool Rust API** (an app with its own MCP endpoint): `server.tools_for(&principal)` gives `tools/list` entries; `server.call_tool(&caller, name, args).await` gives the `tools/call` result. `Err` is only `ToolError::NoTool` or `ToolError::BadParams`; a tool that ran and failed is `Ok` with `isError: true`.
- **Per-request principal:** `handle_as(p, message)` or `handle_with(&caller, message)` (context reaches `register_with` handlers; a caller with no principal runs as the server's, never as the trusted app). `handle_async` is the core; `handle` drives it on the calling thread.
- **stdio:** `serve_stdio(&server)`. **HTTP:** `http::serve(listener, server, token)` or `serve_loopback(_or)`; the app picks the token variable. `project_mcp_json(dir, name, url)` writes the URL (never the token) into a committed `.mcp.json`.
- **Schemas a model reads** are flat (v0.9.24): a documented field is `{"type", "description"}`, an `Option<T>` field is `T` and not required, `#[facet(deny_unknown_fields)]` adds `"additionalProperties": false`.

## The command line (`architect-control`)

The rule: every action is CLI- **and** MCP-accessible. Guide: `docs/content/guide/cli-and-mcp.md`.

```rust
let surface = Surface::new("task", env!("CARGO_PKG_VERSION"), list.clone()).with_principal(principal);
let mcp = architect_mcp::Server::over(surface.clone());          // same surface → same caller
let _serving = Control::new("task", surface)?.serve_if_wanted()?; // keep it alive
// in main: `task actions …`
Cli::new("task").headless(|| Ok(Box::new(ListOnly::new(surface_for_ci())))).main(std::env::args_os().skip(1));
```

- MCP's generic tools ARE the protocol's methods: never hand-write a tool table or a CLI command per action. `schema` (socket) = `actions://schema` (MCP resource); `hello` is the socket's handshake.
- Failures carry `code` (`unknown`, `bad_args`, `failed`, `denied`, `disabled`, `duplicate`, `confirmation_required` with `ticket` + `expires_in_ms`, `bad_request`, `no_method`); MCP's JSON errors carry it too. A confirmation is retried with `actions run ID … --ticket T` (a `batch` step's `ticket`), the same ticket flow as MCP's `run`. Exit codes: `Code::exit` (1 failed, 2 usage, 3 unknown, 4 bad args, 5 denied, 6 disabled, 7 confirmation, 69 no app).
- The socket's caller is the surface's principal unless `Control::with_caller(|peer| …)` says otherwise; never the trusted app. A hidden action is `unknown` to `describe` and `denied` to `run`, as over MCP.
- `serve_if_wanted`: on in debug builds, off in release; `FTS_CONTROL=0|1` overrides. `serve` always.
- `FTS_CONTROL_DIR=<dir>` (`endpoint::DIR_ENV`) is where sockets go instead of `<runtime dir>/fts`: an iOS Simulator app serves in a short folder on the Mac (`scripts/mac-live-check.sh ios-act`).
- `--headless` builds the list in-process; an app whose actions come from components mounts its root with `architect_guide::run::Headless` and pumps it (`cli::Headless::pump`) — the lab: `apps/lab/desktop/src/lib.rs` `HeadlessLab`.
- `watch` is invalidation: `lost: true` (app restarted, or history overran) → re-read state.
- An app that serves either face needs `Parity::new(&server).check()` in a test; `cargo xtask surfaces` (CI) fails otherwise. `Parity` runs only read-only actions unless given `.run(id, args)`: never point it at a live app's real data.
- Windows: the named pipe and its client are written but not yet compiled in CI.

## Agent tools on a running app (#268)

What lets a coding agent change a running app and check its own work (herdr's `status`, `shot`, `check` and generated guide):

- **Unknown argument names are refused** at every door (`crates/architect/src/action_list.rs` `ActionList`): a top-level name the params schema's `properties` don't have answers `bad_args` `unknown argument(s): x; valid arguments: a, b` before anything runs (an action taking nothing: `valid arguments: none`). Open schemas (`additionalProperties`) take any. A destructive call from outside answers `confirmation_required` at once: an agent never waits on a dialog.
- **`APP_STATUS`** (`crates/architect/src/action_list/app.rs` `register`): every `StatusBoard` section (`ActionList::status`) by name, then `problems` (every `Problems` source: `source`, `file`, `field`, `message`) and `confirmations`. A part of the app plugs in on the list it already has: `list.status().section(name, || json)`, `list.problems().source(key, || vec![Problem::new(..)])` (pulled per read) or `.report(key, problems)` (pushed; empty clears). The workspace host gives `workspace` (`features/workspace/src/host/actions.rs` `WorkspaceStatus`), `architect_theme::actions::register` gives `theme` (`features/theme/src/actions.rs` `report`), extensions report with `Extensions::report_problems(list.problems())`.
- **`APP_SHOT {path?, pane?}`** (`features/workspace/src/shot.rs` `use_app_shot`, feature `native`): the launcher keeps the main window's Blitz document in context (`LiveDocument`); a shot repaints it on the CPU (`anyrender_vello_cpu`) at the window's pixel size and scale, on the window's thread; `pane` crops to `WORKSPACE_PANE_GET`'s `rect`. `actions shot [file.png] [--pane id]` runs it. Not shot: pop-out windows, and what a custom widget draws on the GPU only.
- **The generated guide** (`features/control/src/guide.rs` `Guide`): `Guide::new(app).preamble(template).keep_written(surface, app_dir, settle)` writes `<app dir>/AGENTS.md` (header, the app's opening, architect's `RULES`, every action by area with effect and arguments) and `actions.schema.json`, again whenever an action comes or goes. Test it names every action (`apps/lab/desktop/tests/agent_tools.rs`).
- **`EXT_CHECK {folder}`** (`features/extensions/src/check.rs` `Checker`; `Host::checker` for the app's view kinds): manifest (field, TOML line), Luau scripts (file, line), a load in a host of its own, each view rendered once (a process view's first frame, feature `remote`; a `[[views]] page` file through the app's pages, `Checker::with_page_checker`). **`EXT_CLONE {id, as?}`** (`features/extensions/src/clone.rs` `Extensions::clone_to_dev`; needs `with_dev_root`): a linked built-in with a `[source]` and no `as` is cloned from source under its own id and rebuilt on save (`features/extensions/src/source.rs`; its answer's `build`).
- **Workspaces:** `WORKSPACES_CLONE {name, as?}`; `WORKSPACES_SAVE` over a built-in set from outside the app answers `builtin`; `WORKSPACES_RESTORE_BUILTINS` puts the shipped sets back (`Env::shipped`). **The escape hatch** Ctrl+Alt+Shift+Escape (Cmd on macOS) is `Commands::escape_hatch` (`features/commands/src/ui.rs`), heard before any binding; `Keymap::bind` refuses it.

## Scripts (`architect-scripting`)

```rust
let scripting = Scripting::new(list.clone(), Library::new(person_store).with_org(org_store))
    .with_transactions(HistoryTransactions::new(doc.clone(), history.clone())); // feature `crdt`
scripting.install()?;  // SCRIPT_* actions + every saved script
```

- Features: default is the model (`Script`, `Scope`, `action_id`, `ScriptError`, `Transactions`, `ScriptStore`, `MemoryStore`, `Library`; wasm-clean); `luau` the runtime and actions (mlua's Luau, built from C++ source: not on wasm); `crdt` `HistoryTransactions`; `settings` `SettingsStore` (a settings file).
- A script's globals: `actions.list/describe/run/enabled`, `state.query` (read-only actions only), `print`/`log`, `args`; it returns its result. Each action it runs goes through `run_json_async_with` as the run's caller, so a model's script can do exactly what the model may.
- A run opens one step (`Transactions::begin`), commits it at the end, aborts it on failure. With `crdt::History`, every `transact` an action makes while a step is open folds into it (`History::begin` / `commit` / `abort`), so the actions must share the app's one `Arc<Mutex<History>>`.
- Failures carry their line (`ScriptError { kind, line, message }`); as an action the failure is `Failed` with `line N: …`, what ran and the log.
- Saved scripts are read when they run; `save` checks the source first. `SCRIPT_SAVE_ORG` / `SCRIPT_DELETE_ORG` are separate ids so a permission engine can keep the organization's scripts to some people.
- Limits: `Limits { memory, steps }` per run, `MAX_RUNNING` runs at once (a script that runs itself stops there).
- A single-person list: on a multi-tenant server every person's saved scripts would share one list; give each its own list there.
- Live scripts: `Runtime::start(caller, source, args, step)` runs the first part and gives a `Live`; each `Live::frame()` runs what the script handed `defer(fn)` (ReaScript's `reaper.defer`), one step each, the step limit reset. A frame that fails stops it. One-off runs have no `defer`.
- Modules: `Runtime::with_module(Arc<dyn Module>)` adds a global and a `require` path (`require` finds nothing else). A module keeps its state in an `Arc` shared with its host, never as a Luau value (that would keep the state alive); `raise(lua, msg)` fails the script at its line. `mlua` is re-exported from `architect_scripting::luau`.
- Worked example to copy: `examples/scripting` (`examples/scripting/src/lib.rs` `App`: actions + `RoleEngine` authorizer + `Scripting` with `HistoryTransactions` + named-mode MCP; scripts in `examples/scripting/scripts`; `examples/scripting/tests/scripts.rs`, `examples/scripting/tests/mcp.rs`), walked in `docs/content/getting-started/scripting.md`.

## Canvases (`architect-canvas`)

```rust
let (report, session) = Session::start(&runtime, &caller, source, args, "Marker lane").await;
let mut s = session.expect("started");          // None: the first run failed (report says why)
s.publish("session.arrange", Some(spaces));     // the view's time axis and lanes, scrolled
s.pointer("session.arrange", ViewPointer { at, down, context });
s.frame(imgui::Input { pointer, down, sizes, closed }).await;  // one frame: a press, a release, a resize
for layer in s.output().layers_on("session.arrange", Plane::Over) { /* draw */ }
```

- Scripts see `ImGui` (ReaImGui's names: `require 'imgui' '0.10'`, `CreateContext`, `Begin(ctx, name, p_open?, flags?)` → `visible, open`, `Text`, `Button`, `Checkbox`, `SliderDouble`/`SliderInt` → `changed, v`, `SameLine`, `Separator`, `PushID`, `WindowFlags_*`, `Cond_*`) and `Canvas` (`Layer(view, plane?, id?)`, `Spaces`, `TimeToX`, `Lane`, `Line`, `Rect`, `Circle`, `Text`, `Region` → `hovered, clicked, held, context`). Colours are `0xRRGGBBAA`.
- A window is a workspace pane: the UI never moves or titles it; it is laid out at its pane's size (`Input::sizes`), its id is `<context>/<name id>`, closing its pane gives `open == false` once (`Input::closed`).
- A layer drawn into in a frame is redrawn from scratch; untouched it keeps its last drawing. Regions answer from the last frame's hit-test (the last claimed is on top); everywhere else the pointer passes to the view.
- Frames come on change, not a timer: a press and a release must each be its own frame (`use_script_canvas` does this).
- On Blitz: `use_canvases()` at the app, `use_script_canvas(…)` per script, `use_tool_windows(workspace)`, `window_view()` in the `Views`, and `CanvasLayers { view, plane }` as a child of the (positioned) view, which publishes its spaces with `Canvases::publish`. Shapes are boxes (no egui, no `<canvas>` yet).
- An extension declares `[[overlays]] id, view, plane, script` (Luau kind only); the app starts it as a `Session` on a runtime gated by `host::gate(&manifest)`.

## Extensions (`architect-extensions`)

```rust
let host = Host::new(list.clone(), SharedSections::new(registry)); // `ui`: use_extension_sections()
let ext = Extensions::new(host, Installs::new(data_dir.join("extensions")))
    .with_source(GitRegistry::new(url, cache_dir))   // or FolderRegistry, ReaPackSource
    .with_builtin(Package::embedded(include_str!(".../extension.toml"), &[("x.luau", include_str!(".../x.luau"))])?);
ext.install_actions()?;          // EXT_LIST / EXT_SEARCH / EXT_INSTALL / EXT_UNINSTALL / EXT_RELOAD / EXT_CHECK / EXT_CLONE
let _ = ext.sync();              // load the copy of each that runs
// on a timer: ext.reload_changed()  — hot reload; a failed one keeps its last good copy
```

- A package is `extension.toml` + files. Kinds `data` (presets only) and `luau` run built in; `process` needs the runner the app adds: `features/extensions/src/process.rs` `ProcessRunner` (feature `process`, Unix), `host.with_runner(ProcessRunner::new()?.with_processes(processes))` as `examples/extension/host/src/lib.rs` `extensions` does. No `wasm` runner ships yet; a kind with no runner fails to load with `LoadError::NoRunner`.
- An extension's actions must start with its id's prefix (`fts.metronome` → `FTS_METRONOME_`), its setting keys with `<id>.`; sections are owned by it and leave with it. A Luau extension runs inside its `[capabilities]` (`actions` prefixes, the most `effect`) through `luau::Runtime::with_gate`; a model's call still runs as the model.
- Extension actions go on the list with `ActionList::register_forwarded_with` (`crates/architect/src/action_list.rs`): a described action whose handler is handed the caller.
- A reload is a diff: kept actions keep their ids; on any failure nothing is swapped. Values of its settings survive (keys unchanged).
- Which copy runs (`catalog::choose`): a dev copy, else an installed one newer than the built-in, else the built-in. `disable(id)` keeps it off whatever copies exist.
- A project's needs: `Requirements` in its document's `meta` (`doc::requirements`, feature `crdt`); `Extensions::open` makes a `Plan` (install / offer / missing, with an org `Policy`), `accept` installs and loads it.
- Registry installs are copies; a folder registry's are links (an edit there reloads). `GitRegistry` shells out to `git`; `ReaPackSource` fetches through the app's `Fetch` (`LocalFetch` does `file://` only).
- Recipe, a whole host to copy: `examples/extension` — a registry folder with a Luau extension (actions, settings, an `[[overlays]]` ruler with an ImGui tool window), a process extension (`examples/extension/counter/src/main.rs`), and the host (`examples/extension/host/src/lib.rs` `extensions`, `overlays`); walked through in `docs/content/getting-started/extension.md`.

## Driving the workspace

`architect-workspace` puts every layout operation on the list (feature `actions`; under `ui` the host does it when `WorkspaceView` gets `actions: WorkspaceActions::new(list)`; with no window, `HeadlessWorkspace::new(State, Env).install(&list)`). The conduct a model needs:

- **Read, then address by id.** `WORKSPACE_LAYOUT` (groups with ids, tabs, splits, docks, the active pane), `WORKSPACE_PANE_GET {pane}`, `WORKSPACE_NEIGHBOR {pane, direction}`, `WORKSPACE_VIEWS` (what `WORKSPACE_OPEN` takes as `view`). Ids are opaque and never handed out twice: read them from an answer, never make them up. A group or window that went because its panes went to one other leaves an alias, so an id read before still finds them.
- **Every target is a parameter.** `WORKSPACE_OPEN {view, id?, at: {replace|beside|group|split|window|dock|float}}`, `WORKSPACE_MOVE {pane, to: {group|beside|edge|split|window|dock|float}}`, `WORKSPACE_SPLIT {pane, side}`. Nothing falls back to "the active pane".
- **Focus is the person's.** A write moves `active` only with `focus: true`; `WORKSPACE_FOCUS` is the one action about focus. Don't pass `focus: true` unless the person asked.
- **Answers, not errors, for no-ops.** Every write answers `{changed, reason, pane, group, window, screenset, layout}`; `changed: false` comes with a `reason` (`already_open`, `already_zoomed`, `only_tab`, `no_neighbor`, `dock_not_out`, …), so a retry is safe and no second read is needed. An `Err` names what the call got wrong, and changed nothing.
- **Modes, not toggles:** `WORKSPACE_ZOOM {pane, mode: on|off|toggle}`.
- **Revisions.** `WORKSPACE_LAYOUT.revision` goes up with every change of the arrangement, by anyone. Pass `if_revision` on a write planned from a read: if the person changed the layout since, the write answers `changed: false, reason: stale` with the current layout (and its `revision`) instead of running. Each new revision is a `Changed {topic: "workspace"}` event, its detail JSON `{revision, actor, kinds, panes}` (`features/workspace/src/control/mod.rs` `Change`). Treat events as invalidation: on a revision gap, re-read.
- **The tree.** Besides the flat `groups`/`splits`/`docked`, each window has `center` (nested `split`/`group` with `id`, `path` like `center/0/1`, `rect`, `measured`), `docks`, `floats`, `area`, `bounds` (`features/workspace/src/tree.rs`). Rects are window points: measured by the Blitz host, else laid out from shares. A path addresses a split for one revision (`WORKSPACE_RESIZE {path, shares}`).
- **Whole layouts:** `WORKSPACE_EXPORT` → edit → `WORKSPACE_APPLY {layout, keep_unplaced=true}`: built whole first, so one that does not fit changes nothing; panes keep their ids (their views are not rebuilt).
- **Don't close what you didn't open.** `WORKSPACE_CLOSE` is reopenable (`WORKSPACE_REOPEN`); `WORKSPACE_CLOSE_OTHERS`, `WORKSPACE_CLOSE_WINDOW` and `WORKSPACES_DELETE` are `Effect::Destructive`.
- **Saved layouts** ("workspaces"): `WORKSPACES_LIST` first; `WORKSPACES_LOAD {id|name|slot}`, `WORKSPACES_SAVE {name?}` (a name saves over the set of that name), `WORKSPACES_RENAME`, `WORKSPACES_SET_SLOT`, `WORKSPACES_OPTIONS`, `WORKSPACES_RESET`, `WORKSPACES_DELETE`, `WORKSPACES_CLONE {name, as?}`, `WORKSPACES_RESTORE_BUILTINS`; each change is a `Changed {topic: "workspaces"}` event. A built-in set is the person's: a model's save over it answers `builtin`; clone it, then save.
- The view commands (`WORKSPACE_CLOSE_TAB`, `WORKSPACE_SPLIT_RIGHT`, …) are these actions with the active pane as target and `focus: true` (`features/workspace/src/host/commands.rs`).

## Pitfalls

1. `run_json` refuses async actions; a key or menu handler spawns `run_json_async` on the app's runtime (architect-commands' `Commands::run` does, so ⌘K runs saved scripts and service actions).
2. The app doors skip the authorizer by design. Don't assume a key or menu run is authorized, and give the list the same engine as the router's `PermissionsGate` (`set_authorizer`): otherwise there are two planes.
3. The MCP HTTP transport calls `handle`, so every request runs as the server's one principal (the control socket's default is the same: the surface's principal). The bearer token is a gate, not an identity; per-request principals need your own transport around `handle_as` / `handle_with`.
4. The origin guard checks an `Origin` header only when present, and does not check `Host`. Bind loopback, make serving opt-in (`FTS_MCP=0` turns `serve_loopback` off), and set a token for any app with destructive actions.
5. `batch` with missing or non-array `steps` runs zero steps without an error.
6. `#[architect::service(actions)]` handlers do not see the caller; a per-org service registers by hand with `register_async_with`. A projected failure is the error's `Display`, not the HTTP face's `{code, message}`.
7. `validate()` checks schemas are JSON objects only; it does not prove the decoder and schema agree beyond that.
8. The palette and the menu bar over the Action List are `architect-commands` (`features/commands/src/ui.rs` `use_action_list`; `features/commands/src/menu.rs` `MenuBar`). architect-ui `Command*` / `Menubar*` still take their own items. A menu re-reads enabled and ticked states when the list's events move on: after changing what a check reads outside an action, call `list.changed(topic, detail)`. The launcher (`features/launcher/architect-launcher-core`) is a separate catalogue with no architect dependency and no bridge to the list.
9. The queued-window pattern for UI-only effects is Session's (`studio:apps/session/daw/src/actions.rs` `WINDOW`, `take_window`): a programmatic run never opens a dialog.
