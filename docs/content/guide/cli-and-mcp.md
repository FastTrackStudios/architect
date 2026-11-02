+++
title = "CLI and MCP"
description = "Every action on the command line, the same as over MCP: the app's control socket, the embeddable actions CLI, headless mode, the same caller and permissions, exit codes, and the parity test that keeps the two faces identical."
weight = 95
+++

Everything an architect app can do is an action on its Action List, and
the rule is that **everything is both CLI- and MCP-accessible**. A model
reaches the list over MCP (`features/mcp`, [the previous chapter](@/guide/commands.md#5-mcp));
a person, a script or CI reaches the same list from the command line
(`features/control`, `architect-control`). In this chapter you will:

1. see the one shape both faces speak, and why they cannot drift;
2. serve the app's control socket;
3. embed the `actions` command in the app's own CLI, with a headless mode;
4. decide who a CLI caller is to the permission engine;
5. read the exit codes;
6. prove parity with one test, which `cargo xtask surfaces` requires.

The lab wires all of it: `apps/lab/desktop/src/lib.rs`, run as
`architect-desktop actions …`.

## 1. One shape, two renderings

MCP's five generic tools (`actions`, `describe`, `run`, `batch`, `watch`)
are the five methods of the control protocol
(`features/control/src/protocol.rs` `Method`). A request is parsed by one
function (`Request::parse`) and answered by one function
(`features/control/src/surface.rs` `Surface::answer`). MCP renders the
`Answer` as a tool result's text (`features/mcp/src/lib.rs` `Server`); the
socket sends the same JSON as a line. Neither face has a table of its
own, so a model and a script asking the same thing get the same bytes.

The socket speaks newline-delimited JSON, one request a line:

```text
→ {"id": 1, "method": "run", "params": {"id": "DRUMS_HITS", "args": {"track": "kick"}}}
← {"id": 1, "result": [0.5, 1.0]}
→ {"id": 2, "method": "run", "params": {"id": "DRUMS_HITS", "args": {"trak": "kick"}}}
← {"id": 2, "error": {"code": "bad_args", "error": "bad arguments: …", "takes": {…}}}
```

It has two methods more: `schema` (every action the caller may run as
one JSON Schema document, which MCP serves as the `actions://schema`
resource) and `hello` (the app, its version, the protocol, the latest
event). `actions schema --api` prints the protocol itself as JSON Schema.

A failure always has a `code`, mapped one to one from the Action List's
typed errors (`Failure::from_action`): `unknown`, `bad_args`, `failed`,
`denied`, `disabled`, `duplicate`, and `bad_request` / `no_method` for a
request the protocol refuses. MCP's JSON errors carry the same `code`.

Events are invalidation, as in herdr: `watch` answers `next` (pass it back
as `since`) and `lost: true` when events were missed (the app restarted,
or the watcher fell more than `EVENT_HISTORY` behind). Read the state
again then; never replay events over a snapshot.

## 2. The control socket

```rust
use architect_control::{Surface, server::Control};

let surface = Surface::new("task", env!("CARGO_PKG_VERSION"), list.clone());
let serving = Control::new("task", surface)?.serve_if_wanted()?; // keep it alive
```

- Where: `<runtime dir>/fts/<app>.sock` (`$XDG_RUNTIME_DIR`, else the
  temporary directory, the user's own on macOS), in a `0700` directory,
  the socket `0600`; a connection from another user is closed unanswered.
  On Windows a named pipe, `\\.\pipe\fts-<user>-<app>`, that refuses
  remote clients (`features/control/src/endpoint.rs`).
- When: `serve_if_wanted` serves in a debug build and not in a release
  one; `FTS_CONTROL=0` turns it off and `FTS_CONTROL=1` on. `serve`
  always serves. A socket left by an app that died is replaced at the
  next start; one a live app answers on is refused (`AddrInUse`).
- How: on a thread and a runtime of its own, so an app with no runtime (a
  Blitz window) serves it too. Each request runs on a blocking thread, as
  MCP's HTTP transport runs one. Dropping the `Serving` stops it and
  removes the socket.

A desk app that serves MCP for a model running elsewhere gets it from the
socket: `architect_mcp::bridge::serve_stdio_via(&socket)` is MCP on stdio
whose every tool call is a request to the running app
(`architect-desktop mcp`).

## 3. The `actions` command

An app embeds the command as a subcommand of its own:

```rust
use architect_control::cli::{Cli, ListOnly};

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("actions") {
        return Cli::new("task")
            .headless(|| Ok(Box::new(ListOnly::new(task::surface()))))
            .main(std::env::args_os().skip(1));
    }
    // … the app
}
```

`ActionsArgs` is a clap `Args` for an app that has its own clap command;
`Cli::execute` writes to any `Write`, which is how tests drive it. The
generic client `architect-actions` (`features/control/src/bin/architect-actions.rs`)
asks any running app.

```text
task actions list [--filter T] [--category C] [--effect read-only|mutating|destructive] [--json]
task actions describe TASKS_CREATE [--json]
task actions run TASKS_CREATE --arg title=Hello --arg due:='"2026-11-02"'
task actions run TASKS_CREATE --json '{"title": "Hello"}'      # `-` reads stdin
task actions run TASKS_DELETE --arg id=t1 --ticket 3f2a…       # once a person approved it
task actions batch '[{"id": "A_B"}, {"id": "C_D", "args": {…}}]' [--keep-going]
task actions schema [--api]
task actions events [--since N] [--follow] [--json]
task actions apps                                              # what is running here
task actions --app studio list                                 # another app, or a socket's path
```

`--arg key=text` passes a string, `--arg key:=json` anything else.
`run` prints a string result as it is and anything else as indented JSON;
`--output json` prints one line of JSON.

**Headless.** `--headless` starts the app's list in the process and asks
it, with no window, for scripts and CI; each command is a fresh start, so
chain steps with `batch`. An app whose actions live in its components
(registered by hooks, answered on the window's thread) mounts its root in
a headless document: `architect_guide::run::Headless`
(`features/guide/src/run.rs`), pumped while the request runs
(`architect_control::cli::Headless::pump`). The lab is that:
`apps/lab/desktop/src/lib.rs` `HeadlessLab`, and
`architect-desktop --headless` keeps it running, serving its socket.

## 4. Who the caller is

The CLI is not the trusted app. Every request runs as a principal the
list's authorizer checks (`ActionList::set_authorizer`, the same engine as
the router's `PermissionsGate`): the surface's
(`Surface::with_principal`, `Anonymous` unless set), exactly as MCP's
`Server::with_principal` is, or the one `Control::with_caller` decides
from the connection's `Peer` (its user and process ids). A caller sees
only what it may run (`actions`, `describe` and `schema` treat the rest
as `unknown`) and a run of the rest is `denied`.

Build `Server::over(surface.clone())` and `Control::new(app, surface)`
from one `Surface` and the two faces make a caller the same principal.

**Confirmation.** A list that asks a person first
(`ActionList::require_confirmation`, `set_confirm`) asks for the CLI as it
does for a model: a destructive run answers `confirmation_required` with
a `ticket`; the CLI prints the ticket and exits 7, the app shows its
sheet (`features/commands/src/confirm.rs` `ConfirmSheet`), and once the
person allows it the same command with `--ticket <T>` runs it, once. A
`batch` step takes `ticket` too. The sheet names the CLI's principal as
the caller, so give the surface one a person recognises
(`service:cli` in the lab).

## 5. Exit codes

| Code | Exit | Meaning |
|---|---|---|
| — | 0 | done |
| `failed` | 1 | the action ran and failed (or panicked) |
| `bad_request`, `no_method` | 2 | the command line or request is wrong (clap's usage errors are 2 too) |
| `unknown` | 3 | no such action (or, to `describe`, not one this caller may see) |
| `bad_args` | 4 | the arguments do not fit; stderr shows the schema |
| `denied` | 5 | the caller may not run it |
| `disabled` | 6 | it cannot run now; stderr says why |
| `confirmation_required` | 7 | a person must approve it first; stderr shows the ticket and the `--ticket` retry |
| `duplicate` | 8 | two actions share an id |
| `unavailable` | 69 | no app answers on that socket |
| `protocol` | 76 | the answer was not one |

The table is `Code::exit` (`features/control/src/protocol.rs`). A batch
exits with its first failure's code.

## 6. Parity, enforced

Both faces render one surface, so they agree by construction. An app
proves it against its real list with one test:

```rust
#[test]
fn mcp_and_the_cli_agree() {
    let app = App::new().unwrap();
    architect_mcp::parity::Parity::new(&app.mcp(agent()))
        .run("TASKS_LIST", None)          // runs of anything that changes state, on test doubles
        .check()
        .unwrap_or_else(|mismatch| panic!("{mismatch}"));
}
```

`Parity::check` (`features/mcp/src/parity.rs`, feature `parity`) serves
the server's surface on a socket of its own and asks both faces the same
things: the tool set against the protocol's methods, the listing and both
resources, every action's `describe`, a run of every read-only action,
the runs you add, an unknown action, a stray parameter, and `watch`.
Copies to read: `examples/scripting/tests/mcp.rs` `mcp_and_the_cli_agree`,
`apps/lab/desktop/tests/actions.rs` (the headless lab, pumped while the
check runs) and `features/mcp/tests/parity.rs` (every failure kind, the
CLI's exit codes, headless mode and the MCP bridge).

`cargo xtask surfaces` (CI) fails when a crate serves MCP and not the CLI
or the reverse, when one that serves either has no `Parity` test, or when
MCP's generic tools stop being built from `Method::TOOLS`.

## Where Task and Studio are

Neither embeds the `actions` command or serves a control socket yet; each
needs `Cli::new("task" | "studio")`, a `Control` beside its MCP server
over the same `Surface`, and the parity test. `cargo xtask surfaces` runs
over this repo only.

## Where to read more

- [The Action List and MCP](@/architecture/commands.md): registering,
  callers, async actions, MCP tools and transports.
- `skills/architect/references/commands-mcp.md`.
