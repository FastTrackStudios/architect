//! The design system, rendered without a browser.
//!
//! These pages are server-rendered and ship no WASM, so every component
//! borrowed from `architect-ui` has to produce its markup and its
//! classes through `dioxus-ssr`. That is not something to assume: a
//! component that reached for the DOM would compile here and render
//! nothing, and the failure would be an unstyled page rather than an
//! error.

// `allow-*-in-tests` in `clippy.toml` does not reach a `tests/` file's
// helper code — see the same note in the auth-server tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_ui::components::{Button, ButtonVariant};
use auth_ui::assets;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use dioxus::prelude::*;
use tower::ServiceExt;

#[test]
fn a_component_renders_its_markup_and_its_classes_without_a_dom() {
    #[component]
    fn Page() -> Element {
        rsx! {
            Button { variant: ButtonVariant::Primary, "Save changes" }
        }
    }

    let html = dioxus_ssr::render_element(rsx! { Page {} });
    assert!(html.contains("Save changes"), "{html}");
    assert!(html.starts_with("<button"), "{html}");
    // The classes are the whole point: without them the sheet this
    // crate serves has nothing to attach to.
    assert!(html.contains("bg-primary"), "{html}");
    assert!(html.contains("rounded-lg"), "{html}");
}

fn router() -> axum::Router {
    axum::Router::new().route("/auth/assets/{file}", get(assets::serve))
}

async fn get_path(path: &str) -> axum::response::Response {
    router()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .expect("serve")
}

#[tokio::test]
async fn the_stylesheet_is_served_as_css_that_can_be_cached_forever() {
    let response = get_path(assets::stylesheet_path()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/css; charset=utf-8")
    );
    // Safe only because the URL names the content; see `assets`.
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let css = String::from_utf8(body.to_vec()).expect("utf-8 css");
    assert!(css.contains("--background"), "design tokens");
    assert!(
        css.contains("bg-primary"),
        "the class the button above asks for"
    );
}

#[tokio::test]
async fn anything_but_the_stylesheet_is_a_404() {
    // A mistyped href should look like a missing file, not quietly
    // serve CSS under whatever name was asked for.
    for path in [
        "/auth/assets/ui.css",
        "/auth/assets/../secrets",
        "/auth/assets/ui-.js",
    ] {
        assert_eq!(
            get_path(path).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

/// The rail is a navigation, so its entries must be links.
///
/// This lives here rather than in `architect-ui` because it is this
/// crate that depends on the guarantee: these pages have no client-side
/// router, so a sidebar entry rendered as a `<button>` would simply not
/// go anywhere.
#[tokio::test]
async fn the_rail_navigates_with_anchors_and_marks_the_current_page() {
    use auth_ui::settings::{Nav, NavOrg, document};

    let nav = Nav {
        current: "/account/sessions".into(),
        name: "Ada Lovelace".into(),
        email: "ada@local.test".into(),
        is_admin: true,
        is_guest: false,
        orgs: vec![NavOrg {
            id: "acme".into(),
            name: "Acme Records".into(),
            role: "owner".into(),
        }],
    };
    let response = document(
        "Sessions",
        "Sessions",
        "Where you are signed in.",
        &nav,
        rsx! {
            p { "the body" }
        },
    );
    let html = render(response).await;

    // The stylesheet is linked, not inlined — see `assets`.
    assert!(
        html.contains(&format!("href=\"{}\"", assets::stylesheet_path())),
        "the sheet is not linked"
    );
    assert!(!html.contains("<style"), "the sheet is still being inlined");

    // Every rail entry is an anchor.
    for href in [
        "/account/profile",
        "/account/passkeys",
        "/orgs/acme",
        "/admin/users",
    ] {
        assert!(
            html.contains(&format!("href=\"{href}\"")),
            "{href} is not a link"
        );
    }
    // And the one being read says so, for a reader who cannot see the
    // highlight.
    assert!(
        html.contains("aria-current=\"page\""),
        "nothing is marked current"
    );
    assert!(html.contains("the body"), "the page body is missing");
}

/// Read a rendered page back out of its response.
async fn render(response: axum::response::Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8(body.to_vec()).expect("utf-8")
}
