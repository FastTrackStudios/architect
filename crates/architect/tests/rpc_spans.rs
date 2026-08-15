//! The `telemetry` feature's dispatch span, end-to-end over an in-process
//! `LocalServer`.
//!
//! What this pins down is the part that makes traces readable: the span
//! must be NAMED for the method it dispatched. A `MethodId` is a hash, so
//! an uninstrumented (or badly instrumented) router yields spans that all
//! look alike and a Tempo view where every RPC collapses into one bucket.
//! Asserting the `otel.name` / `rpc.method` fields is asserting that a
//! trace can actually be read.
#![cfg(all(feature = "local", feature = "telemetry", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};

use architect::local::LocalServer;
use architect::resource::Scope;
use architect::{LayerRouter, Services, layers};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod echo_proto {
    #[architect::rpc]
    pub trait Echo {
        /// Which instance answered.
        fn whoami(&self) -> String;
    }
}

use echo_proto::EchoClient;
use echo_proto::prelude::EchoService;

#[derive(Clone, architect::HasDispatcher)]
#[dispatch(architect::dispatch::CurrentThreadDispatcher)]
struct EchoBackend {
    name: &'static str,
}

impl echo_proto::Echo for EchoBackend {
    fn whoami(&self) -> String {
        self.name.to_string()
    }
}

impl Services for EchoBackend {
    fn layers() -> impl architect::Layer<Self> {
        layers![EchoService]
    }
}

// ── A subscriber that records the spans the router opens ─────────────────

/// One captured span: its static name plus the string-valued fields we
/// care about.
#[derive(Debug, Clone, Default)]
struct Captured {
    name: &'static str,
    fields: Vec<(String, String)>,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Captured>>>);

impl<S> tracing_subscriber::Layer<S> for Capture
where
    S: tracing::Subscriber,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        struct Visitor(Vec<(String, String)>);
        impl tracing::field::Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.push((field.name().to_owned(), value.to_owned()));
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push((
                    field.name().to_owned(),
                    format!("{value:?}").trim_matches('"').to_owned(),
                ));
            }
        }
        let mut v = Visitor(Vec::new());
        attrs.record(&mut v);
        self.0.lock().unwrap().push(Captured {
            name: attrs.metadata().name(),
            fields: v.0,
        });
    }
}

impl Capture {
    fn rpc_spans(&self) -> Vec<Captured> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.name == "rpc")
            .cloned()
            .collect()
    }
}

fn field<'a>(c: &'a Captured, key: &str) -> Option<&'a str> {
    c.fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn router_for(name: &'static str) -> LayerRouter {
    use architect::Services as _;
    EchoBackend { name }.into_router()
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_span_names_the_method() {
    let capture = Capture::default();
    let _guard = tracing_subscriber::registry()
        .with(capture.clone())
        .set_default();

    let server = LocalServer::serve(router_for("default"), Scope::new());
    let client: EchoClient = server.establish().await.expect("establish");
    assert_eq!(client.whoami().await.expect("call"), "default");

    let spans = capture.rpc_spans();
    assert_eq!(spans.len(), 1, "one span per dispatched call: {spans:?}");
    let span = &spans[0];

    // The service/method the call actually reached — not a hash.
    // `#[architect::rpc] trait Echo` registers its vox service as `EchoRpc`.
    assert_eq!(field(span, "rpc.service"), Some("EchoRpc"));
    assert_eq!(field(span, "rpc.method"), Some("whoami"));
    assert_eq!(field(span, "rpc.system"), Some("vox"));
    // What tracing-opentelemetry renames the exported span to; without it
    // every RPC shows up as "rpc" in the trace view.
    assert_eq!(field(span, "otel.name"), Some("EchoRpc/whoami"));
    // Unscoped mount → empty, not absent (the field is always recorded so
    // the span shape is stable).
    assert_eq!(field(span, "rpc.scope"), Some(""));
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_span_carries_the_instance_scope() {
    let capture = Capture::default();
    let _guard = tracing_subscriber::registry()
        .with(capture.clone())
        .set_default();

    // Same trait mounted under an instance scope — the span must say WHICH
    // instance answered, or a multi-rig trace is ambiguous.
    let router = router_for("default").merge_router_scoped("guitar", router_for("guitar"));
    let server = LocalServer::serve(router, Scope::new());

    let guitar: EchoClient = server.establish().await.expect("establish");
    let guitar = architect::scope_client!(guitar, "guitar");
    assert_eq!(guitar.whoami().await.expect("call"), "guitar");

    let spans = capture.rpc_spans();
    assert_eq!(spans.len(), 1, "one span per dispatched call: {spans:?}");
    assert_eq!(field(&spans[0], "rpc.scope"), Some("guitar"));
    assert_eq!(field(&spans[0], "rpc.method"), Some("whoami"));
}
