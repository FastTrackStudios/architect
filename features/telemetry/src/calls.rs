//! Which vox calls are exported as spans.
//!
//! The router opens a span for every call it dispatches. Most calls are
//! well under a millisecond and come by the thousand: a window reading a
//! song back makes a few thousand, an engine opening a set a few thousand
//! more. As spans they are a flood nobody reads; their count and timings are
//! already the `rpc.server.duration` metric. What a trace is for is the call
//! that was slow, failed, or warned, so that is what is exported.
//!
//! [`QuietCalls`] wraps the exporting processor and drops a finished vox
//! call span that took under the threshold, ended without an error and
//! logged no warning. Every other span passes through untouched, a call's
//! parent included. A subscription (`rpc.streaming`) is dropped whatever
//! its length, unless it failed or warned: its span lasts as long as the
//! subscription, which says nothing about how fast anything was. (Events below a warning do not keep a call: vox's own
//! debug events land on every call span when the filter lets them through.)

use std::time::Duration;

use opentelemetry::Context;
use opentelemetry::trace::Status;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};

/// The env var overriding [`DEFAULT_MIN_CALL`], in milliseconds. `0`
/// exports every call.
pub const MIN_CALL_ENV: &str = "ARCHITECT_RPC_SPAN_MIN_MS";

/// How long a call has to take to be exported as a span when it neither
/// failed nor warned.
pub const DEFAULT_MIN_CALL: Duration = Duration::from_millis(5);

/// The threshold [`MIN_CALL_ENV`] sets, else [`DEFAULT_MIN_CALL`].
#[must_use]
pub fn min_call_from_env() -> Duration {
    std::env::var(MIN_CALL_ENV)
        .ok()
        .and_then(|ms| ms.trim().parse::<f64>().ok())
        .filter(|ms| ms.is_finite() && *ms >= 0.0)
        .map_or(DEFAULT_MIN_CALL, |ms| Duration::from_secs_f64(ms / 1000.0))
}

/// A [`SpanProcessor`] passing on every span but the quiet vox calls.
#[derive(Debug)]
pub struct QuietCalls<P> {
    inner: P,
    min: Duration,
}

impl<P> QuietCalls<P> {
    /// Drop vox call spans quicker than `min` (and clean) before `inner`.
    pub const fn new(inner: P, min: Duration) -> Self {
        Self { inner, min }
    }
}

/// Whether `span` is a vox call that was quick, clean and logged no warning.
#[must_use]
pub fn is_quiet_call(span: &SpanData, min: Duration) -> bool {
    let is_call = span
        .attributes
        .iter()
        .any(|kv| kv.key.as_str() == "rpc.system" && kv.value.as_str() == "vox");
    let streaming = span
        .attributes
        .iter()
        .any(|kv| kv.key.as_str() == "rpc.streaming" && kv.value.as_str() == "true");
    let took = span
        .end_time
        .duration_since(span.start_time)
        .unwrap_or_default();
    is_call
        && (streaming || took < min)
        && !matches!(span.status, Status::Error { .. })
        && !span.events.events.iter().any(warned)
}

/// Whether an event was logged at `WARN` or `ERROR` (`tracing-opentelemetry`
/// records the level as the event's `level` attribute).
fn warned(event: &opentelemetry::trace::Event) -> bool {
    event.attributes.iter().any(|kv| {
        kv.key.as_str() == "level" && matches!(kv.value.as_str().as_ref(), "WARN" | "ERROR")
    })
}

impl<P: SpanProcessor> SpanProcessor for QuietCalls<P> {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        if !is_quiet_call(&span, self.min) {
            self.inner.on_end(span);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(resource);
    }
}
