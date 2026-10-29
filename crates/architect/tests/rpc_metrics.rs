//! The router records each call's duration as `rpc.server.duration`
//! (OpenTelemetry's RPC conventions), by system, service and method — what a
//! call made many times a second is read by, where its spans would be a
//! stream of thousands. Read back through an in-memory exporter, over an
//! in-process `LocalServer`.

// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `expect()` is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(all(feature = "local", feature = "telemetry", not(target_arch = "wasm32")))]

use architect::local::LocalServer;
use architect::resource::Scope;
use architect::{LayerRouter, Services, layers};
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

mod echo_proto {
    #[architect::rpc]
    pub trait Echo {
        /// Say it back.
        fn echo(&self, text: String) -> String;
    }
}

use echo_proto::EchoClient;
use echo_proto::prelude::EchoService;

#[derive(Clone, architect::HasDispatcher)]
#[dispatch(architect::dispatch::CurrentThreadDispatcher)]
struct EchoBackend;

impl echo_proto::Echo for EchoBackend {
    fn echo(&self, text: String) -> String {
        text
    }
}

impl Services for EchoBackend {
    fn layers() -> impl architect::Layer<Self> {
        layers![EchoService]
    }
}

fn router() -> LayerRouter {
    EchoBackend.into_router()
}

#[tokio::test(flavor = "multi_thread")]
async fn each_call_records_its_duration_by_service_and_method() {
    // Installed before the first call: the router makes its instrument
    // from the provider it finds then.
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    opentelemetry::global::set_meter_provider(provider.clone());

    let server = LocalServer::serve(router(), Scope::new());
    let client: EchoClient = server.establish().await.expect("establish");
    assert_eq!(client.echo("a".into()).await.expect("call"), "a");
    assert_eq!(client.echo("b".into()).await.expect("call"), "b");

    provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("metrics");
    let points: Vec<(u64, Vec<(String, String)>)> = finished
        .iter()
        .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
        .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
        .filter(|m| m.name() == "rpc.server.duration")
        .flat_map(|m| match m.data() {
            AggregatedMetrics::F64(MetricData::Histogram(h)) => h
                .data_points()
                .map(|p| {
                    let mut attributes: Vec<(String, String)> = p
                        .attributes()
                        .map(|kv| (kv.key.to_string(), kv.value.to_string()))
                        .collect();
                    attributes.sort();
                    (p.count(), attributes)
                })
                .collect::<Vec<_>>(),
            other => panic!("rpc.server.duration is not an f64 histogram: {other:?}"),
        })
        .collect();
    let echo = points
        .iter()
        .find(|(_, attributes)| {
            attributes
                .iter()
                .any(|(k, v)| k == "rpc.method" && v == "echo")
        })
        .unwrap_or_else(|| panic!("no rpc.server.duration point for echo in {points:?}"));
    assert_eq!(echo.0, 2, "both calls counted");
    for (key, value) in [("rpc.system", "vox"), ("rpc.service", "EchoRpc")] {
        assert!(
            echo.1.iter().any(|(k, v)| k == key && v == value),
            "{key}={value} in {:?}",
            echo.1
        );
    }
    assert!(
        !echo.1.iter().any(|(k, _)| k == "rpc.scope"),
        "an unscoped call carries no scope: {:?}",
        echo.1
    );
}
