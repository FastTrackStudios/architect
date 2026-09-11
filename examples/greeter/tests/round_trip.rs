//! One scenario, three wires — and a mock swapped in for a fourth.
//!
//! The generated clients implement `Greeter`, so the scenario is a
//! generic `async fn`; which transport it ran on is decided here, once.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect::{Layer as _, LocalServer, Scope, Services as _};
use example_greeter::{
    GreetError, Greeter, GreeterClient, GreeterHttpClient, Memory, greeter_layer,
};
use futures::StreamExt as _;

async fn scenario(client: &impl Greeter) {
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
    assert_eq!(
        client.greet("nobody".into()).await.unwrap_err(),
        GreetError::Unknown("nobody".into())
    );
    assert_eq!(client.count().await.unwrap(), 1);
}

async fn serve_http(app: architect::http::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn in_process() {
    scenario(&Memory::default()).await;
}

#[tokio::test]
async fn over_vox() {
    let scope = Scope::new();
    let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
    let client: GreeterClient = local.establish().await.unwrap();
    scenario(&client).await;
    scope.close().await;
}

#[tokio::test]
async fn over_http() {
    let backend = Memory::default();
    let base = serve_http(backend.clone().into_http_router()).await;
    scenario(&GreeterHttpClient::at(base)).await;
}

#[tokio::test]
async fn streams_over_http() {
    let backend = Memory::default();
    let base = serve_http(backend.clone().into_http_router()).await;
    let client = GreeterHttpClient::at(base);
    let mut greeted = client.greeted().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    Greeter::greet(&client, "ada".into()).await.unwrap();
    assert_eq!(greeted.next().await.unwrap().unwrap(), "hello ada");
}

/// A mock merged over the bundle wins, on both wires.
#[tokio::test]
async fn a_mock_replaces_the_real_greeter() {
    #[derive(Clone)]
    struct Mock;
    impl Greeter for Mock {
        async fn greet(&self, _: String) -> Result<String, GreetError> {
            Ok("mocked".into())
        }
        async fn count(&self) -> Result<u32, GreetError> {
            Ok(99)
        }
    }
    let bundle = Memory::layers().merge(greeter_layer(Mock));

    let scope = Scope::new();
    let local = LocalServer::serve(bundle.provide(Memory::default()), scope.clone());
    let client: GreeterClient = local.establish().await.unwrap();
    assert_eq!(client.greet("cody".into()).await.unwrap(), "mocked");
    scope.close().await;

    let base = serve_http(
        Memory::layers()
            .merge(greeter_layer(Mock))
            .provide_http(&Memory::default()),
    )
    .await;
    assert_eq!(GreeterHttpClient::at(base).count().await.unwrap(), 99);
}
