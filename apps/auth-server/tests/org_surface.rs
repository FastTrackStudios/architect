//! The organization HTTP surface.
//!
//! The engine has had organizations, members, invitations and roles for
//! a long time, all with flows and tests, and none of it was reachable
//! over HTTP — `http.rs` mounted the OIDC provider and the core session
//! JSON and stopped there, because the command structs derive no serde
//! and each route needs a hand-written extractor.
//!
//! That left the one question a *relying party* has to ask —
//! "which orgs does this token belong to, and with what role" —
//! with no answer at all, so every relying party kept its own copy of
//! the membership table and the identity server stopped being the
//! authority on its own data.
//!
//! These drive the real `app_router`, so they exercise routing,
//! extraction, the membership fence and error mapping together.

// Same reasoning as `http_surface.rs`: in an integration-test crate the
// `unwrap()` IS the assertion, and `clippy.toml`'s allow-in-tests does
// not reach the helpers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

async fn app() -> axum::Router {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "https://auth.fasttrackstudio.app".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    server::app_router(&config, auth).expect("router builds")
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is json")
}

/// Make an account and return its bearer token.
async fn sign_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{email}","password":"correct-horse-battery-staple"}}"#
                )))
                .unwrap(),
        )
        .await
        .expect("sign up");
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["token"]
        .as_str()
        .expect("token")
        .to_owned()
}

async fn post(app: &axum::Router, path: &str, token: &str, body: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .expect("request")
}

async fn get(app: &axum::Router, path: &str, token: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::get(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request")
}

/// Create an org, then read it back off `/list` with the caller's role.
///
/// The headline: one call answers "which orgs, and what role", which is
/// the pair an authorization fence needs and the reason this surface is
/// mounted. The slug matters as much as the id — it is what an operator
/// types and what a relying party's own directories are named after.
#[tokio::test]
async fn creating_an_org_puts_it_on_the_callers_list_with_a_role() {
    let app = app().await;
    let token = sign_up(&app, "cody@fasttrackstudio.app").await;

    let response = post(
        &app,
        "/auth/organization/create",
        &token,
        r#"{"name":"Cody Wright","slug":"codywright"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = json_body(response).await;
    assert_eq!(created["organization"]["slug"], "codywright");
    assert_eq!(created["organization"]["name"], "Cody Wright");
    let org_id = created["organization"]["id"]
        .as_str()
        .expect("org id")
        .to_owned();

    let response = get(&app, "/auth/organization/list", &token).await;
    assert_eq!(response.status(), StatusCode::OK);
    let list = json_body(response).await;
    let orgs = list.as_array().expect("a list");
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0]["organization"]["slug"], "codywright");
    assert_eq!(orgs[0]["organization"]["id"], org_id);
    assert_eq!(
        orgs[0]["membership"]["role"], "owner",
        "whoever creates an org owns it"
    );
}

/// Several orgs, one account — the shape a relying party actually
/// consumes.
#[tokio::test]
async fn one_account_carries_many_orgs() {
    let app = app().await;
    let token = sign_up(&app, "cody@fasttrackstudio.app").await;

    for (name, slug) in [
        ("Cody Wright", "codywright"),
        ("CBU", "cbu"),
        ("Days to Praise", "days-to-praise"),
    ] {
        let response = post(
            &app,
            "/auth/organization/create",
            &token,
            &format!(r#"{{"name":"{name}","slug":"{slug}"}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "creating {slug}");
    }

    let list = json_body(get(&app, "/auth/organization/list", &token).await).await;
    let mut slugs: Vec<&str> = list
        .as_array()
        .expect("a list")
        .iter()
        .map(|b| b["organization"]["slug"].as_str().expect("slug"))
        .collect();
    slugs.sort_unstable();
    assert_eq!(slugs, ["cbu", "codywright", "days-to-praise"]);
}

/// The fence. Someone else's org must not appear on your list, and must
/// not be readable by id either — otherwise membership is something an
/// outsider can probe for.
#[tokio::test]
async fn another_accounts_org_is_neither_listed_nor_readable() {
    let app = app().await;
    let owner = sign_up(&app, "cody@fasttrackstudio.app").await;
    let stranger = sign_up(&app, "mallory@example.invalid").await;

    let created = json_body(
        post(
            &app,
            "/auth/organization/create",
            &owner,
            r#"{"name":"CBU","slug":"cbu"}"#,
        )
        .await,
    )
    .await;
    let org_id = created["organization"]["id"].as_str().expect("org id");

    let list = json_body(get(&app, "/auth/organization/list", &stranger).await).await;
    assert!(
        list.as_array().expect("a list").is_empty(),
        "a stranger belongs to nothing"
    );

    let response = get(&app, &format!("/auth/organization/{org_id}"), &stranger).await;
    assert!(
        response.status().is_client_error(),
        "a non-member read {}, expected a refusal",
        response.status()
    );

    let response = get(
        &app,
        &format!("/auth/organization/{org_id}/members"),
        &stranger,
    )
    .await;
    assert!(
        response.status().is_client_error(),
        "a non-member listed members: {}",
        response.status()
    );
}

/// No credential is 401, not 500 — and not an accidental 200.
#[tokio::test]
async fn the_org_surface_requires_a_session() {
    let app = app().await;
    for path in [
        "/auth/organization/list",
        "/auth/organization/00000000-0000-0000-0000-000000000000",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} without a credential"
        );
    }
}

/// A member list is a list of people, not of ids the caller has to fan
/// out over.
#[tokio::test]
async fn members_come_back_with_the_person_attached() {
    let app = app().await;
    let token = sign_up(&app, "cody@fasttrackstudio.app").await;
    let created = json_body(
        post(
            &app,
            "/auth/organization/create",
            &token,
            r#"{"name":"CBU","slug":"cbu"}"#,
        )
        .await,
    )
    .await;
    let org_id = created["organization"]["id"].as_str().expect("org id");

    let members = json_body(
        get(
            &app,
            &format!("/auth/organization/{org_id}/members"),
            &token,
        )
        .await,
    )
    .await;
    let members = members.as_array().expect("a list");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["user"]["email"], "cody@fasttrackstudio.app");
    assert_eq!(members[0]["member"]["role"], "owner");
    assert!(
        members[0]["user"].get("password_hash").is_none(),
        "a member list must not carry credentials"
    );
}

/// Invite, accept, and the invitee is on their own list — the whole
/// point of putting memberships here rather than in each relying party.
#[tokio::test]
async fn an_invitation_puts_the_invitee_in_the_org() {
    let app = app().await;
    let owner = sign_up(&app, "cody@fasttrackstudio.app").await;
    let invitee = sign_up(&app, "tom@example.invalid").await;

    let created = json_body(
        post(
            &app,
            "/auth/organization/create",
            &owner,
            r#"{"name":"CBU","slug":"cbu"}"#,
        )
        .await,
    )
    .await;
    let org_id = created["organization"]["id"].as_str().expect("org id");

    let response = post(
        &app,
        "/auth/organization/invite-member",
        &owner,
        &format!(
            r#"{{"organization_id":"{org_id}","email":"tom@example.invalid","role":"member"}}"#
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let issued = json_body(response).await;
    let invitation_id = issued["invitation"]["id"].as_str().expect("invitation id");
    let invite_token = issued["token"].as_str().expect("token");
    assert_eq!(issued["invitation"]["email"], "tom@example.invalid");

    // Before accepting, the invitee belongs to nothing.
    let list = json_body(get(&app, "/auth/organization/list", &invitee).await).await;
    assert!(
        list.as_array().expect("a list").is_empty(),
        "an unaccepted invitation grants nothing"
    );

    let response = post(
        &app,
        "/auth/organization/accept-invitation",
        &invitee,
        &format!(r#"{{"invitation_id":"{invitation_id}","token":"{invite_token}"}}"#),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "the flow returns nothing, so the route must not promise a body"
    );

    let list = json_body(get(&app, "/auth/organization/list", &invitee).await).await;
    let orgs = list.as_array().expect("a list");
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0]["organization"]["slug"], "cbu");
    assert_eq!(orgs[0]["membership"]["role"], "member");
}

/// A role change is visible on the member's own list, because that list
/// is what a relying party authorizes against.
#[tokio::test]
async fn a_role_change_shows_up_on_the_members_list() {
    let app = app().await;
    let owner = sign_up(&app, "cody@fasttrackstudio.app").await;
    let other = sign_up(&app, "tom@example.invalid").await;

    let created = json_body(
        post(
            &app,
            "/auth/organization/create",
            &owner,
            r#"{"name":"CBU","slug":"cbu"}"#,
        )
        .await,
    )
    .await;
    let org_id = created["organization"]["id"].as_str().expect("org id");

    let issued = json_body(
        post(
            &app,
            "/auth/organization/invite-member",
            &owner,
            &format!(
                r#"{{"organization_id":"{org_id}","email":"tom@example.invalid","role":"member"}}"#
            ),
        )
        .await,
    )
    .await;
    post(
        &app,
        "/auth/organization/accept-invitation",
        &other,
        &format!(
            r#"{{"invitation_id":"{}","token":"{}"}}"#,
            issued["invitation"]["id"].as_str().expect("id"),
            issued["token"].as_str().expect("token")
        ),
    )
    .await;

    let user_id = json_body(get(&app, "/auth/session", &other).await).await["user"]["id"]
        .as_str()
        .expect("user id")
        .to_owned();

    let response = post(
        &app,
        "/auth/organization/update-member-role",
        &owner,
        &format!(r#"{{"organization_id":"{org_id}","user_id":"{user_id}","role":"admin"}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let list = json_body(get(&app, "/auth/organization/list", &other).await).await;
    assert_eq!(
        list.as_array().expect("a list")[0]["membership"]["role"],
        "admin"
    );
}

/// A malformed body says which field was wrong. A 400 that does not
/// name the key turns an obvious typo into a debugging session.
#[tokio::test]
async fn a_missing_field_is_named_in_the_error() {
    let app = app().await;
    let token = sign_up(&app, "cody@fasttrackstudio.app").await;

    let response = post(
        &app,
        "/auth/organization/create",
        &token,
        r#"{"name":"No Slug Here"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"], "missing_field");
    assert_eq!(body["message"], "slug");

    let response = post(
        &app,
        "/auth/organization/set-active",
        &token,
        r#"{"organization_id":"not-a-uuid"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"], "invalid_uuid");
    assert_eq!(body["message"], "organization_id");
}
