+++
title = "Snapshot quiesce"
description = "WriteGate, GatedLayer and the bounded pause → checkpoint → step cycle for consistent on-disk snapshots."
weight = 64
+++

`architect::snapshot` (feature `snapshot`, native only) gives a server a short,
bounded moment where no new request starts and the sqlite WAL is folded into
the main db files, so whatever the app does next (a `git commit` of the data
root, a tarball, an rsync) sees consistent files. The copy step itself stays in
the app: Task keeps its git snapshot engine.

```rust,ignore
use architect::snapshot::{WriteGate, quiesce, wal_checkpoint};

let gate = WriteGate::new(); // holds last at most DEFAULT_MAX_HOLD (30 s)
let router = backend.into_router().gated(&gate); // every vox call parks at the gate
EngineHost::new(router, bind).serve().await?;

// A snapshot cycle:
let commit = quiesce(
    &gate,
    || async { wal_checkpoint(&sqlite_conns).await; },  // checkpoint callback
    || engine.commit_phase(),                            // the app's local step
)
.await?;            // Err(GateTimeout) if checkpoint + step outlived the hold
engine.push_phase(commit).await; // slow network work after the gate reopens
```

## Pieces

| Item | What it does |
|---|---|
| `WriteGate` | Global door. `wait_open()` parks while a hold is alive; `pause()` returns a `GateHold`, dropping it reopens. One hold at a time. |
| `GatedLayer` | vox `Handler` wrapper: waits at the gate, then dispatches. Shape queries pass through. |
| `LayerRouter::gated(&gate)` | Wraps every handler of a built router; the result is still a `LayerRouter`. Call it last (routers merged afterwards are not gated). |
| `quiesce(&gate, checkpoint, step)` | Pause, run `checkpoint`, run `step`, release. |
| `wal_checkpoint(conns)` | With `server-seaorm`: `PRAGMA wal_checkpoint(TRUNCATE)` per connection, best-effort; returns the failures. |

## The maximum hold

Every hold is bounded by `WriteGate::max_hold()` (default `DEFAULT_MAX_HOLD` =
30 s, set with `WriteGate::with_max_hold`). This is the answer to "a stuck
snapshot freezes every write":

- a parked request waits at most until the hold's deadline; then `wait_open`
  returns `Admission::Expired`, logs a warning and the request proceeds;
- `quiesce` cancels checkpoint + step at the same deadline and returns
  `GateTimeout`, so a cycle never reports a snapshot taken after writes resumed.
  Make the step cancel-safe (child processes with `kill_on_drop`).

Keep pushes and uploads after `quiesce` returns: only the local commit needs
the quiesce.

## What the gate does not do

It closes the door; it does not drain the room. Requests already past
`wait_open` finish normally, and the gate is never held across a request body,
so long-lived subscribe streams cannot pin it. sqlite's own locking serializes
those stragglers against the checkpoint.

Tests: `crates/architect/tests/snapshot.rs` (a vox write blocks while gated and
completes on release; the hold maximum admits waiters; `quiesce` order and
timeout; `wal_checkpoint` empties the `-wal` file). Extracted from Task's
`task:apps/server/src/snapshot.rs` (STA-399).
