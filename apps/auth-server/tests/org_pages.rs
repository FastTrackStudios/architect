//! The organization pages, driven the way a browser drives them.
//!
//! Every action on these pages is a form POST answered by a redirect,
//! so a test can follow the same path a person does: post the form,
//! read the `Location`, fetch the next page, assert on the HTML. No
//! JavaScript runs, which is why this is possible at all — and why the
//! same journeys are cheap to re-check in a real browser later.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::panic
)]

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "https://auth.fasttrackstudio.app".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

async fn app() -> axum::Router {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    server::app_router(&config, auth).expect("router builds with no social providers")
}

/// Sign somebody up through the JSON API and keep their bearer token.
///
/// The pages accept `Authorization: Bearer` as well as the cookie, so
/// a test can hold a session without a cookie jar.
async fn signed_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"input":{{"email":"{email}","password":"correct horse battery staple"}}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "sign up {email}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().expect("a session token").to_owned()
}

async fn get(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Post a form and return the `Location` the server redirects to.
async fn post(
    app: &axum::Router,
    uri: &str,
    token: Option<&str>,
    form: &str,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(form.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, location)
}

/// The `id` out of a `/orgs/{id}` redirect.
fn org_id(location: &str) -> &str {
    location
        .trim_start_matches("/orgs/")
        .split(['?', '&'])
        .next()
        .expect("an organization id")
}

#[tokio::test]
async fn an_owner_makes_an_organization_and_sees_themselves_in_it() {
    let app = app().await;
    let owner = signed_up(&app, "owner@example.com").await;

    let (_, empty) = get(&app, "/orgs", Some(&owner)).await;
    assert!(
        empty.contains("not in any organization yet"),
        "a first-time visitor needs to be told what to do"
    );

    let (status, location) = post(&app, "/orgs", Some(&owner), "name=Acme+Records&slug=").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let id = org_id(&location).to_owned();

    let (status, page) = get(&app, &format!("/orgs/{id}"), Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Acme Records"));
    // The slug was derived, because the form left it blank.
    assert!(page.contains("/acme-records"), "derived slug: {page:.400}");
    assert!(page.contains("owner@example.com"));
}

#[tokio::test]
async fn an_invite_link_carries_a_stranger_all_the_way_in() {
    let app = app().await;
    let owner = signed_up(&app, "owner@example.com").await;
    let (_, location) = post(&app, "/orgs", Some(&owner), "name=Acme&slug=acme").await;
    let id = org_id(&location).to_owned();

    // Mint a link. The token comes back in the redirect because it is
    // never legible again.
    let (status, location) = post(
        &app,
        &format!("/orgs/{id}/links"),
        Some(&owner),
        "role=member&label=Launch+week&max_uses=1&expires_days=",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let join_url = link_from(&location);

    // A stranger, not signed in, can see where it leads.
    let (status, page) = get(&app, &join_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Join Acme"), "{page:.400}");
    assert!(page.contains("Sign in and join"));

    // Signed in, following it puts them in the organization.
    let joiner = signed_up(&app, "joiner@example.com").await;
    let token = join_url.trim_start_matches("/join?token=").to_owned();
    let (status, location) = post(&app, "/join", Some(&joiner), &format!("token={token}")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with(&format!("/orgs/{id}")), "{location}");

    let (_, page) = get(&app, &format!("/orgs/{id}"), Some(&owner)).await;
    assert!(page.contains("joiner@example.com"), "{page:.600}");

    // And the allowance is spent, so the next person is turned away —
    // without being told which of the four reasons applies.
    let late = signed_up(&app, "late@example.com").await;
    let (_, page) = get(&app, &join_url, Some(&late)).await;
    assert!(page.contains("not usable"), "{page:.400}");
}

#[tokio::test]
async fn a_revoked_link_shuts_the_door_between_two_visits() {
    let app = app().await;
    let owner = signed_up(&app, "owner@example.com").await;
    let (_, location) = post(&app, "/orgs", Some(&owner), "name=Acme&slug=acme").await;
    let id = org_id(&location).to_owned();
    let (_, location) = post(
        &app,
        &format!("/orgs/{id}/links"),
        Some(&owner),
        "role=member&label=&max_uses=&expires_days=",
    )
    .await;
    let join_url = link_from(&location);

    let (_, before) = get(&app, &join_url, None).await;
    assert!(before.contains("Join Acme"));

    // Find the link's id on the page and revoke it.
    let (_, page) = get(&app, &format!("/orgs/{id}"), Some(&owner)).await;
    let link_id = value_after(&page, r#"name="id" value=""#);
    let (status, _) = post(
        &app,
        &format!("/orgs/{id}/links/revoke"),
        Some(&owner),
        &format!("id={link_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let (_, after) = get(&app, &join_url, None).await;
    assert!(after.contains("not usable"), "{after:.400}");
}

#[tokio::test]
async fn a_member_cannot_see_the_invitation_machinery() {
    let app = app().await;
    let owner = signed_up(&app, "owner@example.com").await;
    let (_, location) = post(&app, "/orgs", Some(&owner), "name=Acme&slug=acme").await;
    let id = org_id(&location).to_owned();
    let (_, location) = post(
        &app,
        &format!("/orgs/{id}/links"),
        Some(&owner),
        "role=member&label=&max_uses=&expires_days=",
    )
    .await;
    let token = link_from(&location)
        .trim_start_matches("/join?token=")
        .to_owned();
    let joiner = signed_up(&app, "joiner@example.com").await;
    post(&app, "/join", Some(&joiner), &format!("token={token}")).await;

    let (_, as_owner) = get(&app, &format!("/orgs/{id}"), Some(&owner)).await;
    assert!(as_owner.contains("Invite links"));
    assert!(as_owner.contains("Delete organization"));

    let (_, as_member) = get(&app, &format!("/orgs/{id}"), Some(&joiner)).await;
    assert!(as_member.contains("Acme"));
    // A member sees the roster but none of the levers.
    assert!(!as_member.contains("Invite links"), "{as_member:.800}");
    assert!(!as_member.contains("Delete organization"));
}

#[tokio::test]
async fn the_pages_send_a_signed_out_visitor_to_sign_in_first() {
    let app = app().await;
    for path in ["/orgs", "/account/profile", "/account/sessions"] {
        let (status, location) = {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            (status, location)
        };
        assert_eq!(status, StatusCode::SEE_OTHER, "{path}");
        assert!(
            location.starts_with("/login?return_to="),
            "{path}: {location}"
        );
        // And it must come back here, not to the root.
        assert!(
            location.contains(&path.replace('/', "%2F")),
            "{path}: {location}"
        );
    }
}

#[tokio::test]
async fn a_profile_edit_shows_up_on_the_page_it_was_typed_on() {
    let app = app().await;
    let person = signed_up(&app, "person@example.com").await;

    let (status, location) = post(
        &app,
        "/account/profile",
        Some(&person),
        "name=Ada+Lovelace&username=ada&image=",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/account/profile?ok="), "{location}");

    let (_, page) = get(&app, "/account/profile", Some(&person)).await;
    assert!(page.contains("Ada Lovelace"), "{page:.600}");
    assert!(page.contains(r#"value="ada""#), "{page:.600}");
}

#[tokio::test]
async fn a_mistyped_password_confirmation_is_caught_before_the_engine() {
    let app = app().await;
    let person = signed_up(&app, "person@example.com").await;

    let (_, location) = post(
        &app,
        "/account/password",
        Some(&person),
        "current_password=correct+horse+battery+staple&new_password=one&confirm_password=two",
    )
    .await;
    assert!(location.contains("error="), "{location}");
    assert!(location.contains("do%20not%20match"), "{location}");
}

#[tokio::test]
async fn the_sessions_page_marks_the_browser_you_are_reading_it_from() {
    let app = app().await;
    let person = signed_up(&app, "person@example.com").await;

    let (status, page) = get(&app, "/account/sessions", Some(&person)).await;
    assert_eq!(status, StatusCode::OK);
    // Without this a person is invited to revoke the session they are
    // currently using.
    assert!(page.contains("This browser"), "{page:.600}");
}

/// The invite URL the redirect carried back, percent-decoded.
fn link_from(location: &str) -> String {
    let raw = location
        .split("token=")
        .nth(1)
        .expect("a minted token in the redirect")
        .split('&')
        .next()
        .expect("a token value");
    percent_decode(raw)
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// The first attribute value following `needle` in some HTML.
fn value_after(html: &str, needle: &str) -> String {
    html.split(needle)
        .nth(1)
        .expect("the attribute")
        .split('"')
        .next()
        .expect("its value")
        .to_owned()
}

/// A linked agent goes wherever its owner goes, at up to the cap, and
/// stops the moment the link is withdrawn. Read as the agent through the
/// same organization list every relying party mirrors from.
#[tokio::test]
async fn a_linked_agent_inherits_the_owners_organizations_up_to_the_cap() {
    let app = app().await;
    let owner = signed_up(&app, "owner@example.com").await;
    let agent = signed_up(&app, "agent@example.com").await;

    let (status, location) = post(&app, "/orgs", Some(&owner), "name=Acme+Records&slug=").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let id = org_id(&location).to_owned();

    // Nothing yet: an account is nobody's agent until it is linked.
    let (status, page) = get(&app, "/orgs", Some(&agent)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("not in any organization yet"));

    // Owner is not a cap on offer, and linking yourself is refused.
    let (_, location) = post(
        &app,
        "/account/agents/link",
        Some(&owner),
        "agent_email=agent%40example.com&max_role=owner",
    )
    .await;
    assert!(
        location.contains("error="),
        "owner is never inheritable: {location}"
    );
    let (_, location) = post(
        &app,
        "/account/agents/link",
        Some(&owner),
        "agent_email=owner%40example.com&max_role=admin",
    )
    .await;
    assert!(
        location.contains("error="),
        "an account cannot be its own agent: {location}"
    );

    let (status, location) = post(
        &app,
        "/account/agents/link",
        Some(&owner),
        "agent_email=agent%40example.com&max_role=member",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.contains("ok="), "the link is made: {location}");

    let (_, page) = get(&app, "/orgs", Some(&owner)).await;
    assert!(
        page.contains("agent@example.com"),
        "the owner sees the agent"
    );
    assert!(page.contains("up to member"));

    // The agent now sees the owner's organization, lowered to the cap.
    let (status, listed) = get(&app, "/orgs", Some(&agent)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(listed.contains("Acme Records"), "inherited: {listed:.600}");
    assert!(
        listed.contains(">member<"),
        "capped to member: {listed:.600}"
    );
    assert!(!listed.contains(">owner<"), "never owner: {listed:.600}");

    // Withdrawn, and gone at once.
    let (_, page) = get(&app, "/orgs", Some(&owner)).await;
    let link_id = page
        .split("name=\"link_id\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the link id is in the form");
    let (status, location) = post(
        &app,
        "/account/agents/unlink",
        Some(&owner),
        &format!("link_id={link_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location}");
    let (_, page) = get(&app, "/orgs", Some(&agent)).await;
    assert!(page.contains("not in any organization yet"));
    let _ = id;
}
