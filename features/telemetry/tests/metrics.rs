//! What the telemetry records as metrics, read back through an in-memory
//! exporter: every stage mark as `app.stage.duration`, by stage and span,
//! and the process's CPU, memory and threads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(not(target_arch = "wasm32"))]

use architect_telemetry::timing;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, Metric, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use tracing_subscriber::layer::SubscriberExt as _;

fn metrics(finished: &[ResourceMetrics]) -> Vec<&Metric> {
    finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
        .collect()
}

fn attributes<'a>(kvs: impl Iterator<Item = &'a opentelemetry::KeyValue>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = kvs
        .map(|kv| (kv.key.to_string(), kv.value.to_string()))
        .collect();
    out.sort();
    out
}

#[test]
fn stages_and_the_process_are_recorded() {
    // Installed before the first mark: the histogram is made from the
    // provider found then, as `otel::init` installs it in an app.
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    opentelemetry::global::set_meter_provider(provider.clone());
    architect_telemetry::process::register(&opentelemetry::global::meter("architect-telemetry"));

    let subscriber = tracing_subscriber::registry().with(timing::Recorder::default());
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("window.slip");
        let _in = span.enter();
        timing::record("window.settle_ms", 3.0);
        timing::record("window.settle_ms", 5.0);
        timing::record("window.replan_ms", 1.0);
    });
    // Outside any span: the stage alone.
    timing::record("load.build_ms", 40.0);
    // Some CPU to measure.
    let mut x = 0u64;
    for i in 0..2_000_000u64 {
        x = x.wrapping_mul(31).wrapping_add(i);
    }
    std::hint::black_box(x);

    provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("metrics");
    let found = metrics(&finished);

    let stage = found
        .iter()
        .find(|m| m.name() == "app.stage.duration")
        .expect("app.stage.duration");
    assert_eq!(stage.unit(), "ms");
    let AggregatedMetrics::F64(MetricData::Histogram(h)) = stage.data() else {
        panic!("a histogram: {:?}", stage.data());
    };
    let mut points: Vec<(Vec<(String, String)>, u64, f64)> = h
        .data_points()
        .map(|p| (attributes(p.attributes()), p.count(), p.sum()))
        .collect();
    points.sort_by(|a, b| a.0.cmp(&b.0));
    let kv = |k: &str, v: &str| (k.to_owned(), v.to_owned());
    assert_eq!(
        points,
        vec![
            (
                vec![
                    kv("span.name", "window.slip"),
                    kv("stage", "window.replan_ms")
                ],
                1,
                1.0
            ),
            (
                vec![
                    kv("span.name", "window.slip"),
                    kv("stage", "window.settle_ms")
                ],
                2,
                8.0
            ),
            (vec![kv("stage", "load.build_ms")], 1, 40.0),
        ]
    );

    #[cfg(target_os = "linux")]
    {
        let names: Vec<&str> = found.iter().map(|m| m.name()).collect();
        for name in [
            "process.cpu.time",
            "process.cpu.utilization",
            "process.memory.usage",
            "process.thread.count",
        ] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        let memory = found
            .iter()
            .find(|m| m.name() == "process.memory.usage")
            .unwrap();
        assert_eq!(memory.unit(), "By");
        let AggregatedMetrics::I64(MetricData::Sum(sum)) = memory.data() else {
            panic!("a sum: {:?}", memory.data());
        };
        assert!(sum.data_points().all(|p| p.value() > 0));
        let cpu = found
            .iter()
            .find(|m| m.name() == "process.cpu.time")
            .unwrap();
        let AggregatedMetrics::F64(MetricData::Sum(sum)) = cpu.data() else {
            panic!("a sum: {:?}", cpu.data());
        };
        let modes: Vec<Vec<(String, String)>> = sum
            .data_points()
            .map(|p| attributes(p.attributes()))
            .collect();
        assert!(modes.contains(&vec![kv("cpu.mode", "user")]), "{modes:?}");
    }
}
