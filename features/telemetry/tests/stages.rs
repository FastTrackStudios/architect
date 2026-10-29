//! A stage marked in a span is a span of its own under it, over the time
//! it took — a unit of work's stages read as its waterfall — and a stage
//! that took next to nothing is not.
// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `expect()` is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(not(target_arch = "wasm32"))]

use architect_telemetry::timing::Stages;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt as _;

#[test]
fn each_stage_is_a_span_under_its_unit_of_work() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info_span!("action").in_scope(|| {
            let mut stages = Stages::start();
            std::thread::sleep(std::time::Duration::from_millis(3));
            stages.mark("hits.detect_ms");
            stages.mark("hits.write_ms");
        });
    });
    provider.force_flush().expect("flush");
    let spans = exporter.get_finished_spans().expect("spans");
    let named = |name: &str| spans.iter().find(|s| s.name == name);
    let action = named("action").expect("the unit of work");
    let detect = named("hits.detect").expect("the stage that took 3 ms");
    assert_eq!(detect.parent_span_id, action.span_context.span_id());
    assert_eq!(
        detect.span_context.trace_id(),
        action.span_context.trace_id()
    );
    let took = detect
        .end_time
        .duration_since(detect.start_time)
        .expect("ends after it starts");
    assert!(took >= std::time::Duration::from_millis(2), "{took:?}");
    assert!(
        named("hits.write").is_none(),
        "a stage of nothing is no span"
    );
}
