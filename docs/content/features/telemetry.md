+++
title = "Telemetry"
description = "Wide events: each call is a span you enrich, stages are child spans, calls and stages are metrics, and one trace follows a caller into the calls it makes."
weight = 14
+++

## What it is and when to use it

architect logs in wide events: one span per unit of work, carrying everything
known about it, instead of many log lines. Every vox call already runs inside
a span named `Service/method`; a handler adds fields to it with
`architect_telemetry::wide::set`. `Stages` times named steps inside the work,
each as a child span and an `app.stage.duration` sample. With the `otel`
feature, OTLP exports traces, logs and metrics, including
`rpc.server.duration` per service and method and process metrics, and a W3C
trace context rides each vox call so a caller's span and the server work it
causes land in one trace.

Use it in every server and app instead of `println!` or scattered `info!`
lines. There is no architecture page for the wide-event model; the rules are
in `skills/architect/references/telemetry.md`. The crash log is
[The flight recorder](@/architecture/flight-recorder.md), and per-action
timings are in [The Action List](@/architecture/commands.md).

## A minimal example

`architect-telemetry` with feature `otel` (plus `flight` for the recorder);
`architect` with `host` or `server-axum`, which turn on its `telemetry` feature.

```rust
fn main() {
    let (_sentry, otel) =
        architect_telemetry::init_tracing_full("greeter", "info,architect::timing=trace");
    std::mem::forget(otel); // a UI loop never returns; a server keeps it for all of main
    // serve ..
}

// In a handler, already inside the call's span:
architect_telemetry::wide::set("greet.name", name.clone());
let mut stages = architect_telemetry::timing::Stages::start();
let found = lookup(&name);
stages.mark("greet.lookup_ms");
if let Err(e) = &found {
    architect_telemetry::wide::set_display("greet.err", e);
}
```

OTLP starts only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Clients built
with `architect::clients!` carry the trace context on their own; another
client adds `architect::trace_context::TraceContextMiddleware` with
`with_middleware`.

## Code tour

1. `features/telemetry/src/lib.rs` `init_tracing_full`: Sentry, the
   `EnvFilter` and the credential cap (`capped_registry`: never DEBUG or
   TRACE from `tungstenite`, whose frames carry bearers, whatever
   `RUST_LOG` says), the fmt layer, then the OTLP layers.
2. `features/telemetry/src/lib.rs` `otel` module, `init`: the OTLP exporters,
   the W3C propagator, the global meter and the process metrics.
3. `features/telemetry/src/lib.rs` `wide` module, `set`, `set_display`: an
   attribute on the current span; a no-op without `otel`.
4. `crates/architect/src/layer.rs` `call_span`, `record_call`: the dispatch
   span (`rpc.system`, `rpc.service`, `rpc.method`, the scope) and the
   `rpc.server.duration` histogram.
5. `crates/architect/src/trace_context.rs` `TraceContextMiddleware`,
   `parent_of`: `traceparent` and `tracestate` in the call's metadata.
6. `features/telemetry/src/calls.rs` `QuietCalls`: drops call spans that were
   fast, clean and not streaming; their timing stays in the metric.
7. `features/telemetry/src/timing.rs` `Stages`, `record`: stage events, child
   spans and `app.stage.duration`.
8. `features/telemetry/src/flight.rs` `RingLayer`, `install_panic_recorder`,
   `restore_previous`: the last run's log and the crash log (feature `flight`).

A client call made inside a span goes out with that span's context in its
metadata. On the server, `LayerRouter` reads it with `parent_of` and runs the
handler inside `call_span`, so `wide::set` and `Stages::mark` land on the
child of the caller's span. When the call ends, `record_call` records its
duration. At export, `QuietCalls` keeps the span only if it was slow
(`ARCHITECT_RPC_SPAN_MIN_MS`, default 5 ms), failed or warned.

## Tests to read

- `crates/architect/tests/trace_context.rs`: a call made in a span is
  dispatched as its child.
- `crates/architect/tests/rpc_metrics.rs`: each call records its duration by
  service and method.
- `crates/architect/tests/rpc_spans.rs`: the dispatch span names the method
  and carries the scope.
- `features/telemetry/tests/calls.rs`: only slow, failed and warning calls are
  exported.
- `features/telemetry/tests/stages.rs`: each stage is a span under its unit of work.
- `features/telemetry/tests/metrics.rs`: stages and the process are recorded.
- `features/telemetry/tests/flight.rs`: the crash and flight logs.

The three `crates/architect` tests need features `local` and `telemetry`.

## Status

Done in architect; trace context, `rpc.server.duration` and `QuietCalls` are
in the latest tags. Limits: telemetry is native only; `wide::set` is silent
without `otel`; a call span measures dispatch, not the reply; the flight log
is one hook per process with no size cap or upload.

- **Task** boots telemetry on the server and desktop
  (`task:apps/server/src/main.rs`, `task:apps/desktop/src/main.rs`) and uses
  `wide::set` widely. Its pinned tag predates trace context, the call metric
  and `QuietCalls`; it uses neither `Stages` nor `flight`.
- **Studio** uses `init_tracing_full`, `wide::set` and `Stages` (for example
  `studio:apps/session/daw/src/perf.rs`), and keeps its own log ring
  (`studio:features/app/shell/src/log_ring.rs`) instead of `flight`. Its tag
  predates trace context.
