+++
title = "Async diagnostics"
description = "moiré runtime instrumentation — see what every task is waiting on."
weight = 60
+++

When an async system hangs, the question is usually "which task is
stuck and what is it waiting on?" architect's `diagnostics` feature
wires up [moiré](https://github.com/bearcove/moire) to make that
question answerable from a live dashboard.

## How it works

moiré replaces tokio's named-resource primitives with instrumented
wrappers. Every `Mutex`, `mpsc::channel`, `oneshot`, and `task::spawn`
in `architect::axum_ws` gets a stable name and a backtrace at every
boundary. The dashboard (`moire-web`) shows a live graph of which
task holds which lock, which channel is full, which oneshot is
waiting for a sender that's been dropped.

With the feature off, the primitives compile to plain tokio
passthroughs — zero runtime cost and zero behavior change.

## Wiring

The instrumentation lives in `architect::axum_ws`. Right now the
named entities are:

| Name | What it is |
|------|------------|
| `vox.session.closed` | oneshot signaling a vox session has ended |
| `vox.ws.inbound` | mpsc carrying WebSocket frames from peer → vox |
| `vox.ws.outbound` | mpsc carrying frames from vox → WebSocket |
| (anonymous task) | the IO loop that bridges WebSocket and channels |

A stuck vox session shows up immediately: if `vox.ws.outbound`'s
receiver is gone, the dashboard says so; if `vox.session.closed`
hasn't fired but the task chain is parked, you can see exactly which
`await` point is holding it.

## Enabling

The `diagnostics` cargo feature on `architect` (or the `example`
facade) flips moiré from passthrough mode to instrumented mode:

```toml
example = { workspace = true, features = ["server-axum", "diagnostics"] }
```

The runtime needs to know where the dashboard lives. Set
`MOIRE_DASHBOARD` to the address moire-web is bound to:

```sh
MOIRE_DASHBOARD=127.0.0.1:9119 cargo run -p app-server --features diagnostics
```

Without the env var (or without the feature), instrumentation is
silently inert — the process runs exactly as it would without moiré.

## Running the dashboard

`just moire-web` builds and launches the dashboard from the
flake-pinned source tree (`MOIRE_SOURCE`). First run takes a couple
of minutes; subsequent runs reuse the cargo cache.

```sh
just moire-web                   # dashboard at http://127.0.0.1:9119
```

Or run both side-by-side:

```sh
just diagnostics                 # spawns dashboard + server, both connected
```

## NixOS frame pointer caveat

moiré captures backtraces by walking frame pointers. Pre-25.11 NixOS
ships a glibc compiled without frame pointers, and an aggressive
chain validator in `moire-trace-capture` will panic when it walks
into libc. Three paths to work around:

1. **Update to NixOS 25.11+** — the [cc-wrapper PR
   #399014](https://github.com/NixOS/nixpkgs/pull/399014) (merged
   2025-05-25) enables frame pointers by default. glibc itself is
   still built early in bootstrap and may not pick this up; works in
   practice for most app-level instrumentation.
2. **Apply Forge's patch** — the `forge/nixos-frame-pointer` branch
   tolerates system-frame chain termination as a valid stop signal
   rather than a panic. Pin moiré to that rev once it's pushed.
3. **Run on a non-NixOS host** — the THEBATTLESHIP / starcommand
   workflow handles this transparently if you SSH in.

In all three cases, with `diagnostics` *off*, moiré is inert and the
frame-pointer walk never runs. Everyday builds are unaffected.

## Cost model

| Build flavor | Runtime overhead | Dep tree change |
|--------------|------------------|-----------------|
| Default (no diagnostics) | None — wrappers compile to direct tokio calls | moire crates are in the tree but unused |
| `--features diagnostics` | One backtrace capture per API boundary call (lock acquire, channel send, etc.) | Same crates, with the instrumentation paths active |

The compile-time cost of the moire crates is already paid (vox pulls
them transitively); enabling `diagnostics` doesn't grow the workspace.

## When to reach for it

- A `cargo nextest` test hangs and you can't tell which task is stuck.
- A vox session intermittently fails to close cleanly.
- Production-shaped load reveals a backpressure stall and you want to
  identify the blocking channel.
- A new backend implementor reports their `<T>Repo::create` "just
  hangs" and you need to see which lock or channel they're waiting on.

For garden-variety contract testing (the kind covered by `tests/native`),
`diagnostics` is overkill — keep it off.
