# Telemetry: `architect-telemetry`

Crate `features/telemetry` (`architect_telemetry`). Native only; wasm gets stubs. Guides: `docs/content/architecture/diagnostics.md`, `docs/content/architecture/flight-recorder.md`. Before writing any log line, load the `logging-best-practices` skill: the span is the wide event.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| Sentry init | error/crash reporting; DSN from build time or env; `service` tag; no PII | `features/telemetry/src/lib.rs` `init` |
| Subscriber install | `EnvFilter` + fmt + Sentry layer, `try_init` | `features/telemetry/src/lib.rs` `init_tracing`, `init_tracing_full`, `tracing_layer` |
| Wide events | attach dynamic fields to the current span | `features/telemetry/src/lib.rs` `wide::set`, `set_display` |
| Stage timing | per-stage ms on a span; in-process ring buffer | `features/telemetry/src/timing.rs` `Stages`, `record`, `recent`, `clear`, `Recorder`, `DIRECTIVE`, `DIRECTIVES`, `no_marks` |
| OTLP export | traces + logs + metrics over http/protobuf (feature `otel`) | `features/telemetry/src/lib.rs` `otel::init`, `OtelGuard`, `enabled`, `OtelHandle` |
| Flight recorder | last 600 events in a ring; crash log + last-run log (feature `flight`) | `features/telemetry/src/flight.rs` `RingLayer`, `install_panic_recorder`, `restore_previous`, `snapshot` |
| Span producers | `rpc` span (`rpc.*` fields); Action List `action` span (`action.id`, `action.ok`) | `crates/architect/src/layer.rs`; `crates/architect/src/action_list.rs` |
| Name collision | `architect::host::init_tracing`: fmt only, no Sentry, no OTel | `crates/architect/src/host.rs` `init_tracing` |

## What a consumer must know

**R1. Server boot order** (Task: `task:apps/server/src/main.rs`):
1. `let _sentry = architect_telemetry::init("task-server");` held for all of `main`.
2. `EnvFilter` from `RUST_LOG` with a local default.
3. `registry().with(filter).with(fmt::layer()).with(architect_telemetry::tracing_layer())`.
4. `architect_telemetry::otel::init("task-server", LOG_DEFAULT)`; on `Some((guard, layers))` do `registry.with(layers).init()`, else `registry.init()`.

**R2. Client (desktop/mobile):** `let (_sentry, otel) = architect_telemetry::init_tracing_full("task-desktop", "info");` then `std::mem::forget(otel);` because the OTel guard's `Drop` shuts exporters down and the UI loop never returns (`task:apps/desktop/src/main.rs`).

**R3. Enrich the span, don't log:**
- Dotted keys: `wide::set("org.slug", slug.clone())`, `wide::set("mcp.tool", name.to_owned())`.
- Cast counts to `i64` (`i64::try_from(n).unwrap_or(i64::MAX)`); display-only values with `wide::set_display("media.token_error", &format!("{e:?}"))`.
- A local closure for a fixed key family: `let outcome = |name: &'static str| wide::set("auth.linked_token.outcome", name);` (`apps/auth-server/src/oauth.rs`).

**R4. Stage timing:** in the hot path `let mut stages = architect_telemetry::timing::Stages::start();` then `stages.mark("comp.rank_ms")` (dotted keys, unit in the name). The subscriber must carry `timing::DIRECTIVE` in its `EnvFilter` and a `timing::Recorder::default()` layer; read back with `timing::recent(n)` (Session: `studio:apps/session/daw/src/perf.rs`, `studio:apps/session/daw/tests/perf_slip.rs`). Action runs are timed only with `timing::DIRECTIVES` in the global filter.

**R5. Crash flight recorder (feature `flight`):** add `RingLayer` to the subscriber, call `restore_previous()` at launch and `install_panic_recorder(app_dir)` once per process; a panic writes `<app_dir>/crash.log` and `<app_dir>/flight-log.txt`. Composes with `architect::host::install_panic_logger` in either order.

**R6. Features:** depend on `architect-telemetry` for `wide`/`init`; add `features = ["otel"]` where OTLP is wanted and `flight` for the recorder.

**R7. Runtime switches:** Sentry is inert without a DSN; OTLP is inert unless `OTEL_EXPORTER_OTLP_ENDPOINT` is non-empty; a local collector setting wins over baked config.

## Checklist: new binary or service

1. Add `architect-telemetry` with `otel` only if it exports OTLP.
2. Call `init` (Sentry) first and bind the guard for all of `main`.
3. Build the subscriber once: `EnvFilter` → fmt → `tracing_layer()` → `otel::init` layers. Never call two inits.
4. Put request-scoped facts on the span with `wide::set`: one event per unit of work.
5. Keep `wide` and `timing` calls behind `cfg(not(target_arch = "wasm32"))` in crates shared with wasm.
6. Prove the field lands: a test with `tracing::subscriber::with_default` + `timing::Recorder::default()` reads it back (pattern: the tests in `features/telemetry/src/timing.rs`).

## Pitfalls

- **`wide::set` is a silent no-op without `otel`.** A library crate depending without `otel` becomes live only when a binary in the same build enables it (feature unification).
- **Keys are `&'static str`:** no runtime-built keys.
- **No secrets or tokens in fields:** fields leave the box. `send_default_pii` is false.
- **Per-layer filters are mandatory:** a layer without its own filter exports what the global `EnvFilter` drops. The log bridge must drop `opentelemetry*` targets (feedback loop).
- **Guards must outlive the process** (hence the `forget` in R2).
- **`resolve_client_config` mutates env** with `set_var`; it must run before threads read env (only the `otel` path calls it).
- **Env var names** (never print values): `TASK_SENTRY_DSN` (build time and runtime), `TASK_OTLP_ENDPOINT` and `TASK_OTLP_TOKEN` (build time), `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, `RUST_LOG`.
- **`try_init` silently ignores a second global subscriber:** a late init does not panic, and does not install your layers.
- **Stage marks are trace-level:** add `timing::DIRECTIVE` or they vanish; console layers wrap `no_marks()`. The recorder buffer is process-global (512 entries) and shared by tests: `clear()` first.
- **`OtelHandle` changes type with the `otel` feature.**
- **The flight recorder hook is one per process.**
