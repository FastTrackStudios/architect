//! Native-only Sentry error/crash telemetry for the Task apps.
//!
//! The DSN is **never** committed. It is resolved at runtime from the
//! build-time `TASK_SENTRY_DSN` (baked into shipped clients via a GH
//! Actions secret) or, failing that, the process env (the server).
//!
//! Callers must hold the returned [`sentry::ClientInitGuard`] for the
//! whole process lifetime — dropping it flushes and disables reporting.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    /// Initialise Sentry and return the guard. `service` is attached as a
    /// tag so events can be filtered per app (server / mobile / desktop).
    ///
    /// Returns `None` when no DSN is configured (dev / local builds) — the
    /// apps then run without telemetry.
    pub fn init(service: &str) -> Option<sentry::ClientInitGuard> {
        let dsn = option_env!("TASK_SENTRY_DSN")
            .map(str::to_owned)
            .or_else(|| std::env::var("TASK_SENTRY_DSN").ok())
            .filter(|s| !s.trim().is_empty())?;

        let guard = sentry::init((
            dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                send_default_pii: false,
                ..Default::default()
            },
        ));

        let svc = service.to_owned();
        sentry::configure_scope(|s| s.set_tag("service", svc));

        Some(guard)
    }

    /// The tracing layer that forwards `error!`/`warn!` events (and spans
    /// as breadcrumbs) to Sentry. Compose into a
    /// [`fn@tracing_subscriber::registry`].
    pub fn tracing_layer<S>() -> sentry_tracing::SentryLayer<S>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        sentry_tracing::layer()
    }

    /// Convenience: initialise Sentry **and** install a global tracing
    /// subscriber (env-filter + fmt + the Sentry layer) in one call.
    ///
    /// Uses `.try_init()` so a later subscriber init (e.g. dioxus) failing
    /// to become the global default is a no-op rather than a panic.
    /// Returns the Sentry guard — hold it for the process lifetime.
    pub fn init_tracing(
        service: &str,
        env_filter_default: &str,
    ) -> Option<sentry::ClientInitGuard> {
        init_tracing_full(service, env_filter_default).0
    }

    /// [`init_tracing`] plus the OTLP guard, for callers that want both.
    ///
    /// Both guards must outlive the process; dropping the OTel one flushes
    /// and stops the exporters. The client apps use [`init_tracing`] and
    /// leak the OTel guard (see below) because their `main` hands control
    /// to a UI event loop that never returns normally.
    pub fn init_tracing_full(
        service: &str,
        env_filter_default: &str,
    ) -> (Option<sentry::ClientInitGuard>, super::OtelHandle) {
        let guard = init(service);

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(env_filter_default));

        let registry = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_layer());

        // With the `otel` feature the client exports the same
        // traces/logs/metrics the server does — the whole point of
        // instrumenting the RPC layer is that a client call and the server
        // work it triggers can be looked at together.
        #[cfg(feature = "otel")]
        {
            let service: &'static str = Box::leak(service.to_owned().into_boxed_str());
            if let Some((otel_guard, layers)) = super::otel::init(service) {
                let _ = registry.with(layers).try_init();
                return (guard, Some(otel_guard));
            }
        }

        let _ = registry.try_init();
        (guard, None)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{init, init_tracing, init_tracing_full, tracing_layer};

/// The OTLP guard, or `()` when the `otel` feature is off — so
/// [`native::init_tracing_full`]'s signature doesn't change with the
/// feature.
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub type OtelHandle = Option<otel::OtelGuard>;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "otel")))]
pub type OtelHandle = Option<()>;

// ── Wide events ──────────────────────────────────────────────────────
/// Wide events (canonical log lines): ONE context-rich event per unit of
/// work, instead of a scatter of log lines nobody can correlate.
///
/// **The span IS the event.** `architect` already opens exactly one span
/// per vox RPC (`LayerRouter::call_span`) and `tower_http` opens one per
/// HTTP request, so the request-scoped container already exists and
/// already propagates across await points. Enriching that span is
/// strictly better than assembling a parallel struct: the fields land on
/// the exported trace, they inherit the trace id for free, and one query
/// answers "what happened to this request".
///
/// This exists because `tracing`'s macros take a *static* field list —
/// you cannot add a field to a span that the macro did not declare.
/// [`wide::set`] goes around that via the OTel span extension, which takes
/// dynamic keys. That is the whole reason for this module.
///
/// ```ignore
/// wide::set("auth.principal_kind", "anonymous");
/// wide::set("perm.decision", "would_deny");
/// wide::set("perm.resource", resource.to_string());
/// ```
///
/// Conventions, so the fields stay queryable:
/// - Dotted, namespaced keys: `auth.*`, `perm.*`, `org.*`, `rpc.*`.
/// - One name per concept, everywhere. The article's core complaint is
///   the same id logged five different ways; a wide event with
///   inconsistent keys is just as unaggregatable.
/// - High cardinality is the POINT (user ids, org slugs). Do not
///   pre-aggregate into buckets to "save space".
/// - Never put a secret, token, or password in a field. These are
///   exported off-box.
#[cfg(not(target_arch = "wasm32"))]
pub mod wide {
    /// Attach a field to the current unit of work.
    ///
    /// A no-op when nothing is listening (no OTel layer installed, or the
    /// `otel` feature is off) — call it freely from library code without
    /// gating the call site.
    #[cfg(feature = "otel")]
    pub fn set(key: &'static str, value: impl Into<opentelemetry::Value>) {
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        tracing::Span::current().set_attribute(key, value);
    }

    /// No-op stand-in so callers compile without the `otel` feature.
    #[cfg(not(feature = "otel"))]
    pub fn set<V>(_key: &'static str, _value: V) {}

    /// Convenience for the very common `impl Display` case — saves every
    /// call site an explicit `.to_string()`.
    pub fn set_display(key: &'static str, value: impl core::fmt::Display) {
        set(key, value.to_string());
    }
}

// ── OpenTelemetry (feature `otel`, native only) ──────────────────────
//
// OTLP export of traces, logs, and metrics to a collector. Doubly
// opt-in: the cargo feature gates the dependency weight, and at
// runtime nothing initializes unless `OTEL_EXPORTER_OTLP_ENDPOINT` is
// set — the local-first "no telemetry" promise holds by default.
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub mod otel {
    use opentelemetry::global;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::Layer;

    /// Re-export for callers that record custom metrics (the server's
    /// HTTP middleware) without adding their own opentelemetry dep.
    pub use opentelemetry;

    /// Owns the three providers; dropping it flushes and shuts down
    /// the exporters. Hold for the process lifetime (like the Sentry
    /// guard).
    pub struct OtelGuard {
        tracer: SdkTracerProvider,
        logger: SdkLoggerProvider,
        meter: SdkMeterProvider,
    }

    impl Drop for OtelGuard {
        fn drop(&mut self) {
            if let Err(e) = self.tracer.shutdown() {
                eprintln!("otel: tracer shutdown: {e}");
            }
            if let Err(e) = self.logger.shutdown() {
                eprintln!("otel: logger shutdown: {e}");
            }
            if let Err(e) = self.meter.shutdown() {
                eprintln!("otel: meter shutdown: {e}");
            }
        }
    }

    /// Whether OTLP export is configured for this process.
    #[must_use]
    pub fn enabled() -> bool {
        std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    }

    /// Resolve the shipped-client OTLP config into the standard
    /// `OTEL_EXPORTER_OTLP_*` env vars, the same way the Sentry DSN is
    /// resolved: build-time first, process env as the override.
    ///
    /// A shipped desktop/iOS build has no one to set env vars for it, and
    /// the exporter is built once at startup — before anyone has signed in
    /// — so a session token can't be the credential. `TASK_OTLP_TOKEN` is
    /// a static ingest token baked in at build time (a GH Actions secret,
    /// exactly like `TASK_SENTRY_DSN`), which the server accepts on
    /// `/otlp` and nothing else.
    ///
    /// Sets nothing that is already set, so a developer pointing a local
    /// build at a local collector always wins. Absent both, this is a
    /// no-op and the app stays telemetry-free — the local-first default.
    fn resolve_client_config() {
        let baked_endpoint = option_env!("TASK_OTLP_ENDPOINT").unwrap_or("").trim();
        if !baked_endpoint.is_empty() && std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_none() {
            // SAFETY: called once, before the exporters (or any other
            // thread that reads env) are built.
            unsafe { std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", baked_endpoint) };
        }

        let baked_token = option_env!("TASK_OTLP_TOKEN").unwrap_or("").trim();
        if !baked_token.is_empty() && std::env::var_os("OTEL_EXPORTER_OTLP_HEADERS").is_none() {
            unsafe {
                std::env::set_var(
                    "OTEL_EXPORTER_OTLP_HEADERS",
                    format!("Authorization=Bearer {baked_token}"),
                )
            };
        }
    }

    /// Initialise the OTLP pipelines (http/protobuf; endpoint and
    /// headers come from the standard `OTEL_EXPORTER_OTLP_*` env vars)
    /// and return the guard plus the tracing layers to compose into
    /// the subscriber registry:
    ///
    /// - a `tracing-opentelemetry` layer — spans become OTel traces;
    /// - the appender bridge — `tracing` events become OTel log
    ///   records (queryable in Loki alongside pod stdout).
    ///
    /// The meter provider is installed globally
    /// (`opentelemetry::global::meter`), so instruments work from
    /// anywhere. Returns `None` when [`enabled`] is false or an
    /// exporter fails to build.
    /// The layers `init` hands back for the caller to attach to its
    /// subscriber. Boxed because the set varies with what built
    /// successfully.
    pub type OtelLayers<S> = Vec<Box<dyn Layer<S> + Send + Sync>>;

    pub fn init<S>(service: &'static str) -> Option<(OtelGuard, OtelLayers<S>)>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
    {
        resolve_client_config();
        if !enabled() {
            return None;
        }
        let resource = Resource::builder().with_service_name(service).build();

        let span_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .build()
            .map_err(|e| eprintln!("otel: span exporter: {e}"))
            .ok()?;
        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(span_exporter)
            .with_resource(resource.clone())
            .build();
        let tracer = tracer_provider.tracer(service);
        global::set_tracer_provider(tracer_provider.clone());

        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .build()
            .map_err(|e| eprintln!("otel: log exporter: {e}"))
            .ok()?;
        let logger_provider = SdkLoggerProvider::builder()
            .with_batch_exporter(log_exporter)
            .with_resource(resource.clone())
            .build();

        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .build()
            .map_err(|e| eprintln!("otel: metric exporter: {e}"))
            .ok()?;
        let meter_provider = SdkMeterProvider::builder()
            .with_periodic_exporter(metric_exporter)
            .with_resource(resource)
            .build();
        global::set_meter_provider(meter_provider.clone());

        // The log bridge MUST NOT ingest the OTel SDK's own diagnostics.
        // The SDK reports export problems through `tracing`; if the
        // bridge turns those into log records it emits more diagnostics,
        // which the bridge ingests again — an unbounded feedback loop
        // that overflows the stack and aborts the process (observed
        // against an unreachable collector). Drop anything originating
        // in the OTel crates before it reaches the bridge.
        let no_otel_feedback = tracing_subscriber::filter::FilterFn::new(|meta| {
            !meta.target().starts_with("opentelemetry")
        });
        let layers: Vec<Box<dyn Layer<S> + Send + Sync>> = vec![
            Box::new(tracing_opentelemetry::layer().with_tracer(tracer)),
            Box::new(
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(
                    &logger_provider,
                )
                .with_filter(no_otel_feedback),
            ),
        ];
        Some((
            OtelGuard {
                tracer: tracer_provider,
                logger: logger_provider,
                meter: meter_provider,
            },
            layers,
        ))
    }
}

// ── wasm: no-op stubs so callers compile cross-target ────────────────
#[cfg(target_arch = "wasm32")]
pub fn init(_service: &str) -> Option<()> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn init_tracing(_service: &str, _env_filter_default: &str) -> Option<()> {
    None
}
