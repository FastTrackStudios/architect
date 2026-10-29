//! Which vox calls are exported as spans: a quick, clean, silent call is
//! dropped (its timing is `rpc.server.duration`), however much it logged
//! below a warning; a slow one, a failed one, one that warned and every span
//! that is not a vox call are kept.
// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `expect()` is the assertion.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, SystemTime};

use architect_telemetry::calls::QuietCalls;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, Status, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

const MIN: Duration = Duration::from_millis(5);

fn span(
    tracer: &opentelemetry_sdk::trace::SdkTracer,
    name: &'static str,
    vox: bool,
    took: Duration,
    tweak: impl FnOnce(&mut opentelemetry_sdk::trace::Span),
) {
    spanned(tracer, name, vox, false, took, tweak);
}

fn spanned(
    tracer: &opentelemetry_sdk::trace::SdkTracer,
    name: &'static str,
    vox: bool,
    streaming: bool,
    took: Duration,
    tweak: impl FnOnce(&mut opentelemetry_sdk::trace::Span),
) {
    let start = SystemTime::now();
    let mut attributes = vec![KeyValue::new("rpc.service", "Items")];
    if vox {
        attributes.push(KeyValue::new("rpc.system", "vox"));
        attributes.push(KeyValue::new("rpc.streaming", streaming));
    }
    let mut span = tracer
        .span_builder(name)
        .with_start_time(start)
        .with_attributes(attributes)
        .start(tracer);
    tweak(&mut span);
    span.end_with_timestamp(start + took);
}

#[test]
fn only_the_slow_failed_and_warning_calls_are_exported() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(QuietCalls::new(
            SimpleSpanProcessor::new(exporter.clone()),
            MIN,
        ))
        .build();
    let tracer = provider.tracer("test");

    span(&tracer, "quick", true, Duration::from_micros(300), |_| {});
    span(&tracer, "slow", true, Duration::from_millis(12), |_| {});
    span(&tracer, "failed", true, Duration::from_micros(300), |s| {
        s.set_status(Status::error("no such item"));
    });
    span(&tracer, "chatty", true, Duration::from_micros(300), |s| {
        s.add_event(
            "vox caller sending request",
            vec![KeyValue::new("level", "DEBUG")],
        );
    });
    span(&tracer, "warned", true, Duration::from_micros(300), |s| {
        s.add_event(
            "tempo-map build failed",
            vec![KeyValue::new("level", "WARN")],
        );
    });
    // A subscription lasts as long as it is held: its length is no latency.
    spanned(
        &tracer,
        "subscribed",
        true,
        true,
        Duration::from_secs(50),
        |_| {},
    );
    spanned(
        &tracer,
        "subscription failed",
        true,
        true,
        Duration::from_secs(50),
        |s| {
            s.set_status(Status::error("hung up"));
        },
    );
    span(
        &tracer,
        "song.read_back",
        false,
        Duration::from_micros(300),
        |_| {},
    );

    provider.force_flush().expect("flush");
    let mut names: Vec<String> = exporter
        .get_finished_spans()
        .expect("spans")
        .into_iter()
        .map(|s| s.name.into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "failed",
            "slow",
            "song.read_back",
            "subscription failed",
            "warned"
        ]
    );
}
