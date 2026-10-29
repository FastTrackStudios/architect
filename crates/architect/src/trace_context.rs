//! W3C trace context over vox — one trace across the caller and the
//! services it calls.
//!
//! Without it every dispatched call starts a trace of its own: a window that
//! reads a song back through a few thousand facade calls leaves a few
//! thousand one-span traces, none of them tied to the read that made them.
//! With it, a call made inside a span carries that span's context as
//! `traceparent` (and `tracestate`) metadata, and the router opens the call's
//! span as its child ([`LayerRouter`](crate::LayerRouter)'s dispatch), so the
//! read is one trace with its calls under it.
//!
//! The client half is [`TraceContextMiddleware`]. [`clients!`](crate::clients)
//! attaches it to every client it builds; a client made another way takes it
//! with `client.with_middleware(TraceContextMiddleware)`.
//!
//! Both halves go through the global text-map propagator, which the app's
//! telemetry installs (`architect-telemetry` sets W3C trace context). Without
//! the `telemetry` feature, or with no propagator installed, nothing is
//! written and nothing is read.

/// Metadata key carrying the W3C `traceparent` header.
pub const TRACEPARENT_METADATA_KEY: &str = "traceparent";

/// Metadata key carrying the W3C `tracestate` header.
pub const TRACESTATE_METADATA_KEY: &str = "tracestate";

/// Whether this build propagates trace context (architect's `telemetry`
/// feature). [`clients!`](crate::clients) reads it in the consumer crate,
/// where architect's features are out of `cfg!`'s reach.
#[doc(hidden)]
pub const PROPAGATES: bool = cfg!(feature = "telemetry");

/// Client middleware stamping every call with the caller's trace context:
/// the current `tracing` span, as `tracing-opentelemetry` knows it.
///
/// A call made outside any span, or in a build without the `telemetry`
/// feature, goes out as it was.
pub struct TraceContextMiddleware;

impl vox::ClientMiddleware for TraceContextMiddleware {
    fn pre<'a>(
        &'a self,
        _context: &'a vox::ClientContext<'a>,
        request: &'a mut vox::ClientRequest<'_, 'a>,
    ) -> vox::BoxMiddlewareFuture<'a> {
        // Read here, not in the future: `pre` runs in the caller's task,
        // inside its span.
        #[cfg(feature = "telemetry")]
        inject(request);
        #[cfg(not(feature = "telemetry"))]
        let _ = request;
        Box::pin(async {})
    }
}

#[cfg(feature = "telemetry")]
fn inject(request: &mut vox::ClientRequest<'_, '_>) {
    use opentelemetry::trace::TraceContextExt as _;
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    let cx = tracing::Span::current().context();
    if !cx.span().span_context().is_valid() {
        return;
    }
    let mut headers = Headers::default();
    opentelemetry::global::get_text_map_propagator(|p| p.inject_context(&cx, &mut headers));
    if let Some(parent) = headers.traceparent {
        request.push_string_metadata(TRACEPARENT_METADATA_KEY, parent);
    }
    if let Some(state) = headers.tracestate.filter(|s| !s.is_empty()) {
        request.push_string_metadata(TRACESTATE_METADATA_KEY, state);
    }
}

/// The trace context a call carries, as the parent for its span. `None`
/// when it carries none, or none this build can read.
#[cfg(feature = "telemetry")]
#[must_use]
pub fn parent_of(metadata: &vox::Metadata) -> Option<opentelemetry::Context> {
    use opentelemetry::propagation::Extractor as _;
    use opentelemetry::trace::TraceContextExt as _;

    let carrier = Carrier(metadata);
    carrier.get(TRACEPARENT_METADATA_KEY)?;
    let cx = opentelemetry::global::get_text_map_propagator(|p| p.extract(&carrier));
    cx.span().span_context().is_valid().then_some(cx)
}

/// What the propagator writes: the two W3C headers, the only ones it has.
#[cfg(feature = "telemetry")]
#[derive(Default)]
struct Headers {
    traceparent: Option<String>,
    tracestate: Option<String>,
}

#[cfg(feature = "telemetry")]
impl opentelemetry::propagation::Injector for Headers {
    fn set(&mut self, key: &str, value: String) {
        match key {
            TRACEPARENT_METADATA_KEY => self.traceparent = Some(value),
            TRACESTATE_METADATA_KEY => self.tracestate = Some(value),
            _ => {}
        }
    }
}

/// A call's metadata, read as the propagator's carrier.
#[cfg(feature = "telemetry")]
struct Carrier<'a>(&'a vox::Metadata);

#[cfg(feature = "telemetry")]
impl opentelemetry::propagation::Extractor for Carrier<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        use vox::MetadataExt as _;
        self.0.meta_str(key)
    }

    fn keys(&self) -> Vec<&str> {
        vec![TRACEPARENT_METADATA_KEY, TRACESTATE_METADATA_KEY]
    }
}
