//! Routes beyond `POST /<prefix>/<method>`: `{param}` and `{*rest}` path
//! segments (in the method's path and the service prefix), the `get` /
//! `put` / `delete` / `any` verbs, and `limit = …` body caps — served by
//! the generated router over a real socket and called through the typed
//! client.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::as_conversions,
    clippy::significant_drop_tightening,
    clippy::derive_partial_eq_without_eq
)]

use architect::Layer as _;
use architect::http::axum::body::Body;
use architect::http::axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

mod notes {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[architect::error]
    pub enum NoteError {
        #[error("no such note")]
        NotFound,
    }

    /// `{slug}` in the service path binds every method's `slug`.
    #[architect::service(path = "org/{slug}")]
    pub trait Notes {
        /// A note's text: a plain JSON reply on a `GET` with path params.
        #[http(get, path = "notes/{id}")]
        async fn note(&self, slug: String, id: u32) -> Result<String, NoteError>;
        /// Replace a note: `text` rides the JSON body, `id` the path.
        #[http(put, path = "notes/{id}")]
        async fn set_note(&self, slug: String, id: u32, text: String) -> Result<(), NoteError>;
        /// Forget a note.
        #[http(delete, path = "notes/{id}")]
        async fn drop_note(&self, slug: String, id: u32) -> Result<(), NoteError>;
        /// `{*path}` takes the rest of the path, slashes and all.
        #[http(get, path = "files/{*path}")]
        async fn file(&self, slug: String, path: String) -> Result<String, NoteError>;
        /// A JSON body past axum's 2 MiB default.
        #[http(post, path = "bulk", limit = "4MiB")]
        async fn bulk(&self, slug: String, text: String) -> Result<u64, NoteError>;
    }

    #[derive(Clone, Default)]
    pub struct Backend {
        notes: Arc<Mutex<BTreeMap<(String, u32), String>>>,
    }

    impl Notes for Backend {
        async fn note(&self, slug: String, id: u32) -> Result<String, NoteError> {
            self.notes
                .lock()
                .unwrap()
                .get(&(slug, id))
                .cloned()
                .ok_or(NoteError::NotFound)
        }
        async fn set_note(&self, slug: String, id: u32, text: String) -> Result<(), NoteError> {
            self.notes.lock().unwrap().insert((slug, id), text);
            Ok(())
        }
        async fn drop_note(&self, slug: String, id: u32) -> Result<(), NoteError> {
            self.notes
                .lock()
                .unwrap()
                .remove(&(slug, id))
                .map(|_| ())
                .ok_or(NoteError::NotFound)
        }
        async fn file(&self, slug: String, path: String) -> Result<String, NoteError> {
            Ok(format!("{slug}:{path}"))
        }
        async fn bulk(&self, _slug: String, text: String) -> Result<u64, NoteError> {
            Ok(text.len() as u64)
        }
    }
}

async fn serve(app: architect::http::axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn text(response: architect::http::axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = architect::http::axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn json(method: &str, path: &str, body: String) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[test]
fn the_paths_table_keeps_the_templates() {
    assert_eq!(
        notes::http::PATHS,
        &[
            ("note", "/org/{slug}/notes/{id}"),
            ("set_note", "/org/{slug}/notes/{id}"),
            ("drop_note", "/org/{slug}/notes/{id}"),
            ("file", "/org/{slug}/files/{*path}"),
            ("bulk", "/org/{slug}/bulk"),
        ]
    );
    assert_eq!(architect::http::encode_segment(&"a b/c"), "a%20b%2Fc");
    assert_eq!(architect::http::encode_rest(&"a b/c.txt"), "a%20b/c.txt");
}

#[tokio::test]
async fn path_params_and_verbs_round_trip_over_a_real_socket() {
    use notes::{NoteError, NotesHttpClient};

    let app = architect::layers![notes::Service].provide_http(&notes::Backend::default());
    let client = NotesHttpClient::at(serve(app).await);

    // GET, PUT and DELETE share one path; each verb reaches its method.
    let err = client.note("acme".into(), 7).await.unwrap_err();
    assert_eq!(err.app(), Some(&NoteError::NotFound));
    client
        .set_note("acme".into(), 7, "hello".into())
        .await
        .unwrap();
    assert_eq!(client.note("acme".into(), 7).await.unwrap(), "hello");
    // The prefix parameter is a real key: another org sees nothing.
    assert!(client.note("other".into(), 7).await.is_err());
    // A parameter that needs encoding survives the trip.
    client
        .set_note("a b/c".into(), 1, "spaced".into())
        .await
        .unwrap();
    assert_eq!(client.note("a b/c".into(), 1).await.unwrap(), "spaced");
    client.drop_note("acme".into(), 7).await.unwrap();
    assert!(client.note("acme".into(), 7).await.is_err());

    // `{*path}` keeps its slashes.
    assert_eq!(
        client
            .file("acme".into(), "docs/a b/readme.md".into())
            .await
            .unwrap(),
        "acme:docs/a b/readme.md"
    );
}

#[tokio::test]
async fn a_route_answers_its_verb_parses_its_params_and_caps_its_body() {
    let app = notes::http::router(notes::Backend::default());

    // Only the declared verbs answer.
    let response = app
        .clone()
        .oneshot(json("POST", "/org/acme/notes/1", "{}".into()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    // A parameter that does not parse is a 400 with the usual envelope.
    let (status, body) = text(
        app.clone()
            .oneshot(
                Request::get("/org/acme/notes/seven")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains(r#""code":"invalid_path""#), "{body}");

    // 3 MiB of JSON: over axum's 2 MiB default, under `bulk`'s 4 MiB.
    let big = format!(r#"{{"text":"{}"}}"#, "x".repeat(3 << 20));
    let (status, body) = text(
        app.clone()
            .oneshot(json("POST", "/org/acme/bulk", big.clone()))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, (3_u64 << 20).to_string());
    // The same body on a route without `limit` is refused.
    let response = app
        .oneshot(json("PUT", "/org/acme/notes/1", big))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
