//! W3C trace context over vox, end to end over an in-process `LocalServer`:
//! a call made inside a span is dispatched as that span's child, so a caller
//! and the calls it makes are one trace — and a call made outside any span
//! starts a trace of its own, as before.
// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `expect()` is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(all(feature = "local", feature = "telemetry", not(target_arch = "wasm32")))]

use architect::local::LocalServer;
use architect::resource::Scope;
use architect::trace_context::TraceContextMiddleware;
use architect::{LayerRouter, Services, layers};
use opentelemetry::trace::{SpanId, TracerProvider as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

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

fn named<'a>(spans: &'a [SpanData], name: &str) -> Vec<&'a SpanData> {
    spans.iter().filter(|s| s.name == name).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_made_in_a_span_is_dispatched_as_its_child() {
    // Global, not scoped: the router dispatches on the runtime's other
    // threads, which a thread-local default would not reach.
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")))
        .init();

    let server = LocalServer::serve(router(), Scope::new());
    let client: EchoClient = server.establish().await.expect("establish");
    let client = client.with_middleware(TraceContextMiddleware);

    async {
        assert_eq!(client.echo("a".into()).await.expect("call"), "a");
    }
    .instrument(tracing::info_span!("song.read_back"))
    .await;
    // Outside any span: a trace of its own.
    assert_eq!(client.echo("b".into()).await.expect("call"), "b");

    provider.force_flush().expect("flush");
    let spans = exporter.get_finished_spans().expect("spans");
    let parent = named(&spans, "song.read_back");
    let calls = named(&spans, "EchoRpc/echo");
    assert_eq!(parent.len(), 1, "{spans:#?}");
    assert_eq!(calls.len(), 2, "{spans:#?}");
    let parent = parent[0];

    let inside = calls
        .iter()
        .find(|c| c.parent_span_id == parent.span_context.span_id())
        .expect("the call made in the span is its child");
    assert_eq!(
        inside.span_context.trace_id(),
        parent.span_context.trace_id()
    );

    let outside = calls
        .iter()
        .find(|c| c.span_context.span_id() != inside.span_context.span_id())
        .expect("the other call");
    assert_eq!(outside.parent_span_id, SpanId::INVALID);
    assert_ne!(
        outside.span_context.trace_id(),
        parent.span_context.trace_id()
    );
}
