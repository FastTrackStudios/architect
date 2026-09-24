//! `layers![…].merge(mock).provide(real)`: the merged mock wins, on both wires.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use architect::{Layer as _, LocalServer, Scope};

mod greeter {
    #[architect::service]
    pub trait Greeter {
        async fn greet(&self) -> String;
    }
    #[derive(Clone)]
    pub struct Real;
    impl Greeter for Real {
        async fn greet(&self) -> String {
            "real".into()
        }
    }
    #[derive(Clone)]
    pub struct Mock;
    impl Greeter for Mock {
        async fn greet(&self) -> String {
            "mock".into()
        }
    }
}

#[tokio::test]
async fn a_merged_mock_overrides_the_bundle() {
    let router = architect::layers![greeter::Service]
        .merge(greeter::layer(greeter::Mock))
        .provide(greeter::Real);
    let scope = Scope::new();
    let local = LocalServer::serve(router, scope.clone());
    let client: greeter::GreeterClient = local.establish().await.unwrap();
    let answer = client.greet().await.unwrap();
    scope.close().await;
    assert_eq!(
        answer, "mock",
        "the merged mock should win over the bundle's binding"
    );
}

#[tokio::test]
async fn the_merged_mock_wins_over_http_too() {
    use tower::ServiceExt as _;
    let app = architect::layers![greeter::Service]
        .merge(greeter::layer(greeter::Mock))
        .provide_http(&greeter::Real);
    let response = app
        .oneshot(
            architect::http::axum::http::Request::post("/greeter/greet")
                .body(architect::http::axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = architect::http::axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), br#""mock""#);
}
