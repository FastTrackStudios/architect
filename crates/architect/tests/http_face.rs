//! The HTTP face `#[architect::service]` and `#[derive(Entity)]` emit, end
//! to end: the axum router served in-process, the typed `<Trait>HttpClient`
//! over a real socket, the bearer fallback for `token` arguments, the
//! typed error envelope, the `BindHttp` bundle hook next to the vox one,
//! `#[subscribe]` streams as Server-Sent Events, and an entity repo with
//! its event feed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::significant_drop_tightening,
    clippy::derive_partial_eq_without_eq
)]

use architect::Layer as _;
use architect::http::axum::body::Body;
use architect::http::axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

mod greeter {
    use architect::http::HttpError;

    #[derive(Debug, Clone, PartialEq, Eq, ::facet::Facet, thiserror::Error)]
    #[repr(u8)]
    pub enum GreetError {
        #[error("nobody is called {0}")]
        Unknown(String),
        #[error("say who you are first")]
        Unauthenticated,
    }

    impl HttpError for GreetError {
        fn status(&self) -> u16 {
            match self {
                Self::Unknown(_) => 404,
                Self::Unauthenticated => 401,
            }
        }
        fn code(&self) -> &'static str {
            match self {
                Self::Unknown(_) => "unknown",
                Self::Unauthenticated => "unauthenticated",
            }
        }
    }

    #[architect::service]
    pub trait Greeter {
        /// Greets a known name.
        async fn greet(&self, name: String) -> Result<String, GreetError>;
        /// Who the bearer token belongs to.
        async fn whoami(&self, token: String) -> Result<String, GreetError>;
        /// Never fails, takes nothing.
        async fn ping(&self) -> u32;
        /// Optional argument: absent from the body means `None`.
        async fn note(&self, text: Option<String>) -> Option<String>;
        /// Every tick, as it happens.
        #[subscribe]
        fn ticks(&self) -> u32;
        /// Only the ticks at or above `min`.
        #[subscribe]
        fn ticks_over(&self, min: u32) -> u32;
    }

    #[derive(Clone)]
    pub struct Backend {
        pub ticks: architect::PubSub<u32>,
        pub high: architect::PubSub<u32>,
    }

    impl Default for Backend {
        fn default() -> Self {
            Self {
                ticks: architect::PubSub::sliding(16),
                high: architect::PubSub::sliding(16),
            }
        }
    }

    impl GreeterStreamSource for Backend {
        fn ticks_hub(&self) -> &architect::PubSub<u32> {
            &self.ticks
        }
        fn ticks_over_attach(&self, min: u32, sink: architect::EventSink<u32>) {
            // The test publishes the filtered feed into `high` itself;
            // `min` only decides which hub answers.
            if min > 0 {
                self.high.attach(sink);
            } else {
                self.ticks.attach(sink);
            }
        }
    }

    impl Greeter for Backend {
        async fn greet(&self, name: String) -> Result<String, GreetError> {
            if name == "cody" {
                Ok(format!("hello {name}"))
            } else {
                Err(GreetError::Unknown(name))
            }
        }
        async fn whoami(&self, token: String) -> Result<String, GreetError> {
            match token.as_str() {
                "t-cody" => Ok("cody".into()),
                _ => Err(GreetError::Unauthenticated),
            }
        }
        async fn ping(&self) -> u32 {
            7
        }
        async fn note(&self, text: Option<String>) -> Option<String> {
            text
        }
    }

    impl architect::Services for Backend {
        fn layers() -> impl architect::Layer<Self> {
            architect::layers![Service, StreamService]
        }
    }
}

mod widget {
    use std::sync::{Arc, Mutex};

    use architect::{Filter, Page, RepoError, Sort};
    use uuid::Uuid;

    #[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
    #[cfg_attr(feature = "fake", derive(architect::fake::Dummy))]
    #[architect(table_name = "widgets", repo, events)]
    pub struct Widget {
        #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
        pub id: Uuid,
        #[architect(filterable, sortable)]
        pub name: String,
    }

    /// The smallest possible backend: a vector behind a mutex.
    #[derive(Clone, Default)]
    pub struct Memory {
        rows: Arc<Mutex<Vec<Widget>>>,
    }

    impl WidgetRepo for Memory {
        async fn get(&self, id: Uuid) -> Result<Widget, RepoError> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|w| w.id == id)
                .cloned()
                .ok_or(RepoError::NotFound)
        }
        async fn list(
            &self,
            _page: Page,
            _sort: Option<Sort>,
            _filter: Option<Filter>,
        ) -> Result<WidgetList, RepoError> {
            let items = self.rows.lock().unwrap().clone();
            let total = items.len() as u32;
            Ok(WidgetList {
                items,
                total,
                page: Page {
                    index: 0,
                    size: total,
                },
            })
        }
        async fn create(&self, input: WidgetCreate) -> Result<Widget, RepoError> {
            let row = Widget {
                id: Uuid::new_v4(),
                name: input.name,
            };
            self.rows.lock().unwrap().push(row.clone());
            Ok(row)
        }
        async fn update(&self, id: Uuid, input: WidgetUpdate) -> Result<Widget, RepoError> {
            let mut rows = self.rows.lock().unwrap();
            let row = rows
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or(RepoError::NotFound)?;
            if let Some(name) = input.name {
                row.name = name;
            }
            Ok(row.clone())
        }
        async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|w| w.id != id);
            if rows.len() == before {
                return Err(RepoError::NotFound);
            }
            Ok(())
        }
    }
}

mod counter {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A sync trait: the HTTP face must go through the dispatcher bridge
    /// exactly as the vox host does.
    // `#[http]` alone implies `#[rpc]`.
    #[architect::http]
    pub trait Counter {
        fn bump(&self, by: u32) -> u32;
        fn label(&self, prefix: &str) -> String;
    }

    #[derive(Clone, Default, architect::HasDispatcher)]
    #[dispatch(architect::dispatch::CurrentThreadDispatcher)]
    pub struct Backend {
        n: Arc<AtomicU32>,
    }

    impl Counter for Backend {
        fn bump(&self, by: u32) -> u32 {
            self.n.fetch_add(by, Ordering::SeqCst).wrapping_add(by)
        }
        fn label(&self, prefix: &str) -> String {
            format!("{prefix}:{}", self.n.load(Ordering::SeqCst))
        }
    }
}

use greeter::{GreetError, Greeter as _, GreeterHttpClient};

fn post(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn text(response: architect::http::axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = architect::http::axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn paths_follow_the_one_scheme() {
    assert_eq!(
        greeter::http::PATHS,
        &[
            ("greet", "/greeter/greet"),
            ("whoami", "/greeter/whoami"),
            ("ping", "/greeter/ping"),
            ("note", "/greeter/note"),
            ("ticks", "/greeter/ticks"),
            ("ticks_over", "/greeter/ticks-over"),
        ]
    );
    assert_eq!(architect::http::path("greeter", "greet"), "/greeter/greet");
    // The helper only kebab-cases. It knows nothing of
    // `#[http(path = "…")]`, which the macro resolves before this is
    // ever reached — so a method declaring `sign-in/email` serves there
    // while this still reads the identifier.
    assert_eq!(
        architect::http::path("/auth/", "sign_in_email_password"),
        "/auth/sign-in-email-password"
    );
}

/// A declared path replaces the derived one and may nest.
///
/// Without this, a path is a function of a Rust identifier — which means
/// renaming a method silently moves a published URL, and no identifier
/// can ever produce a nested one. `#[http(path = "…")]` breaks that
/// coupling; an undeclared method still derives as before.
#[test]
fn a_declared_path_overrides_the_derived_one() {
    mod nested {
        #[architect::error]
        pub enum Oops {
            #[error("nope")]
            Nope,
        }

        #[architect::service(path = "auth")]
        pub trait Doors {
            /// Nests two segments deep.
            #[http(path = "sign-in/email")]
            async fn sign_in_email_password(&self, email: String) -> Result<String, Oops>;
            /// Replaces a long name with a short single segment.
            #[http(path = "session")]
            async fn current_session(&self, token: String) -> Result<String, Oops>;
            /// Undeclared: still the kebab-cased identifier.
            async fn sign_out(&self, token: String) -> Result<(), Oops>;
        }
    }

    assert_eq!(
        nested::http::PATHS,
        &[
            ("sign_in_email_password", "/auth/sign-in/email"),
            ("current_session", "/auth/session"),
            ("sign_out", "/auth/sign-out"),
        ],
        "a declared path wins; an undeclared one is still derived"
    );
}

#[tokio::test]
async fn router_serves_every_method_as_json() {
    let app = greeter::http::router(greeter::Backend::default());

    let (status, body) = text(
        app.clone()
            .oneshot(post("/greeter/greet", r#"{"name":"cody"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#""hello cody""#);

    // No-argument method: an empty body is fine.
    let (status, body) = text(
        app.clone()
            .oneshot(Request::post("/greeter/ping").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "7");

    // Typed error: the status comes from HttpError, the body carries
    // code + message + the error itself.
    let (status, body) = text(
        app.clone()
            .oneshot(post("/greeter/greet", r#"{"name":"nobody"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains(r#""code":"unknown""#), "{body}");
    assert!(body.contains("nobody is called nobody"), "{body}");

    // A body that does not decode is a 422 with the same envelope shape.
    let (status, body) = text(
        app.oneshot(post("/greeter/greet", r#"{"nope":1}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains(r#""code":"invalid_body""#), "{body}");
}

#[tokio::test]
async fn an_absent_optional_argument_reads_as_none() {
    let app = greeter::http::router(greeter::Backend::default());
    let (status, body) = text(
        app.clone()
            .oneshot(post("/greeter/note", "{}"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "null");
    let (_, body) = text(
        app.oneshot(post("/greeter/note", r#"{"text":"hi"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(body, r#""hi""#);
}

#[tokio::test]
async fn a_token_argument_may_ride_the_authorization_header() {
    let app = greeter::http::router(greeter::Backend::default());

    let request = Request::post("/greeter/whoami")
        .header(header::AUTHORIZATION, "Bearer t-cody")
        .body(Body::empty())
        .unwrap();
    let (status, body) = text(app.clone().oneshot(request).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#""cody""#);

    // The body wins when it says something.
    let request = Request::post("/greeter/whoami")
        .header(header::AUTHORIZATION, "Bearer t-cody")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"token":"bogus"}"#))
        .unwrap();
    let (status, _) = text(app.clone().oneshot(request).await.unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = text(
        app.oneshot(
            Request::post("/greeter/whoami")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sync_traits_serve_through_the_bridge() {
    let app = counter::http::router(counter::Backend::default());
    let (status, body) = text(
        app.clone()
            .oneshot(post("/counter/bump", r#"{"by":5}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "5");
    let (_, body) = text(
        app.oneshot(post("/counter/label", r#"{"prefix":"n"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(body, r#""n:5""#);
}

#[tokio::test]
async fn the_bundle_binds_its_http_face_like_its_vox_face() {
    use architect::Services as _;

    // vox: the router mounts the descriptor…
    let vox = greeter::Backend::default().into_router();
    assert_eq!(vox.len(), 2);

    // …and HTTP: the same bundle, the same backend, one axum router.
    let app = architect::layers![greeter::Service].provide_http(&greeter::Backend::default());
    let (status, body) = text(
        app.oneshot(post("/greeter/greet", r#"{"name":"cody"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_typed_client_round_trips_over_a_real_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = architect::http::axum::Router::new()
        .nest("/api", greeter::http::router(greeter::Backend::default()));
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });

    let client = GreeterHttpClient::at(format!("http://{addr}/api"));
    assert_eq!(client.greet("cody".into()).await.unwrap(), "hello cody");
    assert_eq!(client.ping().await.unwrap(), 7);

    // The typed error comes back as the real enum, not a string.
    let err = client.greet("nobody".into()).await.unwrap_err();
    assert_eq!(err.app(), Some(&GreetError::Unknown("nobody".into())));

    // Bearer on the transport feeds the `token` argument.
    let signed_in = client.clone().with_bearer("t-cody");
    assert_eq!(signed_in.whoami(String::new()).await.unwrap(), "cody");
    let err = client.whoami(String::new()).await.unwrap_err();
    assert_eq!(err.app(), Some(&GreetError::Unauthenticated));

    // Direct trait use and the HTTP face agree.
    assert_eq!(
        greeter::Backend::default()
            .greet("cody".into())
            .await
            .unwrap(),
        client.greet("cody".into()).await.unwrap()
    );
}

async fn serve(app: architect::http::axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn subscriptions_stream_as_server_sent_events() {
    use futures::StreamExt as _;

    let backend = greeter::Backend::default();
    // The whole bundle — request/reply *and* streams — binds in one go.
    let app = architect::layers![greeter::Service, greeter::StreamService].provide_http(&backend);
    let client = GreeterHttpClient::at(serve(app).await);

    let mut ticks = client.ticks().await.expect("subscribe");
    let mut high = client.ticks_over(5).await.expect("subscribe filtered");
    // Let both subscriptions attach before publishing.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(backend.ticks.subscriber_count(), 1);
    assert_eq!(backend.high.subscriber_count(), 1);

    for n in [1, 2, 3] {
        backend.ticks.publish(n);
    }
    backend.high.publish(9);

    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(ticks.next().await.unwrap().unwrap());
    }
    assert_eq!(seen, vec![1, 2, 3]);
    assert_eq!(high.next().await.unwrap().unwrap(), 9);

    // Dropping the stream unsubscribes: the hub sees the sink close on
    // its next publish.
    drop(ticks);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    backend.ticks.publish(4);
    backend.ticks.publish(5);
    assert_eq!(backend.ticks.subscriber_count(), 0);
}

#[tokio::test]
async fn an_entity_repo_and_its_events_have_an_http_face() {
    use architect::{Page, RepoError};
    use futures::StreamExt as _;
    use widget::{Widget, WidgetEvent, WidgetEventsHttpClient, WidgetRepoHttpClient};

    assert_eq!(
        widget::widget_http::PATHS,
        &[
            ("get", "/widget/get"),
            ("list", "/widget/list"),
            ("create", "/widget/create"),
            ("update", "/widget/update"),
            ("delete", "/widget/delete"),
            ("events", "/widget/events"),
        ]
    );

    // One wrapped backend serves CRUD and the feed, over vox and HTTP alike.
    let evented = widget::WidgetEvented::new(widget::Memory::default());
    let app = architect::layers![widget::WidgetRepoLayer, widget::WidgetEventsLayer]
        .provide_http(&evented);
    let base = serve(app).await;
    let repo = WidgetRepoHttpClient::at(base.clone());
    let events = WidgetEventsHttpClient::at(base);

    let first = repo
        .create(widget::WidgetCreate {
            name: "gear".into(),
        })
        .await
        .expect("create");
    assert_eq!(repo.get(first.id).await.expect("get").name, "gear");

    // Subscribe: the snapshot carries what exists, then changes follow.
    let mut feed = events.subscribe().await.expect("subscribe");
    match feed.next().await.unwrap().unwrap() {
        WidgetEvent::Snapshot(rows) => assert_eq!(rows, vec![first.clone()]),
        other => panic!("expected a snapshot first, got {other:?}"),
    }
    let second = repo
        .create(widget::WidgetCreate {
            name: "sprocket".into(),
        })
        .await
        .expect("create");
    assert_eq!(
        feed.next().await.unwrap().unwrap(),
        WidgetEvent::Upserted(second.clone())
    );
    let renamed = repo
        .update(
            second.id,
            widget::WidgetUpdate {
                name: Some("cog".into()),
            },
        )
        .await
        .expect("update");
    assert_eq!(renamed.name, "cog");
    assert_eq!(
        feed.next().await.unwrap().unwrap(),
        WidgetEvent::Upserted(renamed)
    );
    repo.delete(first.id).await.expect("delete");
    assert_eq!(
        feed.next().await.unwrap().unwrap(),
        WidgetEvent::Deleted(first.id)
    );

    let listed = repo
        .list(Page { index: 0, size: 10 }, None, None)
        .await
        .expect("list");
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].name, "cog");

    // The typed error survives the trip: 404 on the wire, `NotFound` here.
    let missing = repo.get(uuid::Uuid::new_v4()).await.unwrap_err();
    assert_eq!(missing.app(), Some(&RepoError::NotFound));
    let twice = repo.delete(first.id).await.unwrap_err();
    assert_eq!(twice.app(), Some(&RepoError::NotFound));
    let _: Option<Widget> = None;
}
