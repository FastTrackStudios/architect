+++
title = "Remote views"
description = "A process extension draws Dioxus views in its own process; the app shows them in workspace panes over vox, and a placeholder with the reason when the process dies."
weight = 53
+++

## What it is and when to use it

A process extension is a program of its own. For each view the app opens, it
runs a Dioxus `VirtualDom` on its own thread and streams the DOM edits, as
architect's versioned wire edits, over vox on a local socket. The app mounts
each view as a sub-document inside a workspace pane and sends the person's
clicks, keys and form input back. Every edit is checked before it is
applied, so a misbehaving process cannot crash the app. If the process dies,
the pane shows a placeholder with the reason and a Start again button; the
supervisor restarts it with backoff, and the view comes back.

Use it for a tool that may crash, runs another runtime, or should be
developed and restarted apart from the app, but needs real UI. Design:
[Extensions](@/architecture/extensions.md), sections on processes over vox and
on views rendered remotely; the manifest side is
[Extensions](@/features/extensions.md).

Example: `examples/extension`, a registry with a Luau extension and a process extension loaded by a small host, hot-reloaded on save; walked through in [Write an extension, end to end](@/getting-started/extension.md).

<!-- media: remote-view-clicker — the sandbox's Clicker in a Vault pane: clicking it, then kill -9 on its process, the placeholder with the reason, Start again and the counter back -->

## A minimal example

The extension, `architect-extensions` with `guest`, from
`apps/sandbox/clicker/src/main.rs`:

```rust
use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)                       // fn() -> Element
        .action("LAB_CLICKER_CLICKS", |_| async { Ok(serde_json::json!(clicks())) })
        .serve()                                        // the socket the host passes
}
```

Its manifest, `apps/sandbox/clicker/extension.toml`, has `kind = "process"`, an
`[entry]` with the program's path, its `[[actions]]` and a `[[views]]` entry
with `id = "counter"`.

The app, with `ui` and `remote` (desk only), after `apps/ui/src/extensions.rs`
and `apps/ui/src/vault.rs`:

```rust
use architect_extensions::process::{ProcessRunner, Processes};

static PROCESSES: LazyLock<Processes> = LazyLock::new(Processes::default);

let host = Host::new(list, sections)
    .with_runner(ProcessRunner::new()?.with_processes(PROCESSES.clone()));
// Extensions::new(host, ..), then install or dev the extension

View::new("clicker", "Clicker", |_| rsx! {
    architect_extensions::remote::RemoteView {
        processes: PROCESSES.clone(), extension: "lab.clicker", view: "counter",
    }
})
```

## Code tour

1. `features/extensions/src/proto.rs` `ProcessExtension`: the vox service a
   process serves (`hello`, `run`, `event`, `close`, and the `view`
   subscription), and `SOCKET`.
2. `features/extensions/src/wire.rs` `WIRE`, `Hello`, `Open`, `Frame`, `Edit`,
   `ViewEvent`: the versioned edits and events.
3. `features/extensions/src/process.rs` `ProcessRunner` (`with_restart`,
   `with_processes`): starts the program and waits for its hello, which must
   cover what the manifest declares.
4. `features/extensions/src/process.rs` `Process` (`state`, `open`,
   `restart`, `stop`), `State`, `Restart` (three tries with backoff by
   default), `Processes`.
5. `features/extensions/src/guest.rs` `Guest` (`view`, `action`, `serve`) and
   `WireWriter`: Dioxus mutations to wire edits, on the extension's side.
6. `features/extensions/src/remote.rs` `RemoteDocument`: a Blitz document that
   checks and applies each edit within its caps and forwards events.
7. `features/extensions/src/remote.rs` `RemoteView`: the pane's component;
   follows the process and shows the placeholder.

When the pane mounts, `RemoteView` waits for the process to be running, then
`Process::open(view)` subscribes to the view over vox. It installs a
`RemoteDocument` as the pane node's sub-document and applies each `Frame` as
it arrives. On the extension's side, `Guest` starts a `VirtualDom` on a
thread for that view, writes its renders through `WireWriter`, and
dispatches the events that come back. When the frames stop (the process
died or was unloaded), the view removes the sub-document, shows the
placeholder, and follows the process when it restarts.

## Tests to read

- `features/extensions/tests/remote_view.rs` (the test binary starts itself
  again as the extension): clicks go through the sub-document and the count
  comes back; the extension's action runs over vox; `kill -9` leaves the app
  running and the placeholder shows why; Start again brings it back;
  unloading shows a placeholder; a declared view the process does not draw,
  or a program that never says hello, fails to load; nonsense frames (unknown
  ids, cycles, a pop past the root, a child that is not there, raw HTML) are
  refused without a crash.

## Status

Done in architect on Unix desks; the sandbox runs the clicker. Not yet: a stdio
transport, starting lazily and stopping when idle, handing state over on a
reload, a host service extensions call back into, Windows; pointer, touch,
wheel and scroll events, IME, and an element and CSS allowlist (only raw HTML
is refused today).

- **Task** and **Studio** do not use it yet.
