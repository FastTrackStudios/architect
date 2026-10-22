+++
title = "Crash flight recorder"
description = "architect_telemetry::flight — a tracing log ring, crash.log and flight-log.txt in the app dir, and the previous run's last lines at launch."
weight = 63
+++

`architect_telemetry::flight` (the `flight` feature of
`architect-telemetry`, native only) keeps the evidence a crashed run
leaves behind. It adds no dependency and compiles to nothing on wasm.

| Piece | What it does |
|---|---|
| `RingLayer` | A `tracing` layer; every event becomes one line in a process-wide ring of the last 600 (`CAPACITY`). `snapshot()` reads it for a Logs view. |
| `install_panic_recorder(app_dir)` | A panic hook, chained onto the hook installed before it. It puts the panic in the ring, appends the panic, a backtrace and the 40 lines logged before it to `<app_dir>/crash.log` (restarted past 512 KiB), and writes the ring to `<app_dir>/flight-log.txt`. The first call wins; later calls return `false`. |
| `persist()` | Writes the ring to `flight-log.txt` now. Call it on a timer where the system can kill the app without a panic. |
| `restore_previous()` | At launch, after the recorder: the previous run's last 200 lines go into the ring under a marker (`prev| …`), and the file moves to `flight-log-previous.txt`. Returns the number of lines. |

The hook writes inside the panic because a panic on the UI thread aborts
on iOS; nothing after the hook runs.

## Wiring

```rust,ignore
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

tracing_subscriber::registry()
    .with(env_filter)
    .with(tracing_subscriber::fmt::layer())
    .with(architect_telemetry::flight::RingLayer::new())
    .with(architect_telemetry::tracing_layer())
    .init();
architect_telemetry::flight::install_panic_recorder(app_data_dir);
architect_telemetry::flight::restore_previous();
```

```toml
architect-telemetry = { workspace = true, features = ["flight"] }
```

## With `install_panic_logger`

`architect::host::install_panic_logger` logs every panic through
`tracing`. The recorder composes with it in either order, and the panic
reaches the ring once: when an earlier hook's event lands in a
`RingLayer` while the thread is panicking, the recorder skips its own
`PANIC on <thread>` line. Without the logger (or without a `RingLayer`)
the recorder adds that line itself. One process should still install
each hook only once.

`features/telemetry/tests/flight.rs` proves it: each case panics in a
child process and reads `crash.log` and `flight-log.txt` back, with the
recorder alone, the logger first and the recorder first; a last child
restores the flight log.

```sh
cargo test -p architect-telemetry --features flight --test flight
```

## Not yet

- The ring is process-global; one app per process.
- No size cap on `flight-log.txt` beyond the ring's 600 lines.
- No upload: the files stay on the device until the app shows or sends
  them.
