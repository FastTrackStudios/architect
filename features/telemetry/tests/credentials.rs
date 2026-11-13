//! `init_tracing_full`'s subscriber never passes on a record that can
//! carry a bearer, whatever `RUST_LOG` says.
// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `unwrap()` is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt as _;

/// Every event's target that reached it.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Seen {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        self.0
            .lock()
            .unwrap()
            .push(event.metadata().target().to_owned());
    }
}

/// The registry `init_tracing_full` builds on, at the most permissive
/// filter an operator could write: TRACE everywhere and TRACE for
/// tungstenite's own modules by name. A layer added on top (as fmt, Sentry
/// and the OTLP bridge are) sees everything else, and nothing at DEBUG or
/// TRACE from tungstenite.
#[test]
fn no_directive_lets_tungstenite_frames_through() {
    let seen = Seen::default();
    let filter = tracing_subscriber::EnvFilter::new(
        "trace,tungstenite=trace,tungstenite::protocol=trace,tokio_tungstenite=trace",
    );
    let subscriber = architect_telemetry::capped_registry(filter).with(seen.clone());
    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(target: "tungstenite::protocol", "frame: Bearer secret");
        tracing::debug!(target: "tungstenite", "frame: Bearer secret");
        tracing::trace!(target: "tokio_tungstenite::compat", "frame: Bearer secret");
        tracing::info!(target: "tungstenite::handshake", "connected");
        tracing::trace!(target: "my_app::sync", "a trace of the app's own");
        tracing::trace!(target: "tungstenite_like", "not tungstenite");
    });
    assert_eq!(
        *seen.0.lock().unwrap(),
        ["tungstenite::handshake", "my_app::sync", "tungstenite_like"]
    );
}

/// The list is the framework host's own (`architect::host`), so a library
/// added there is capped here too.
#[test]
fn the_capped_libraries_are_the_hosts() {
    assert_eq!(
        architect_telemetry::CREDENTIAL_LOGGING_TARGETS,
        architect::host::CREDENTIAL_LOGGING_TARGETS
    );
}
