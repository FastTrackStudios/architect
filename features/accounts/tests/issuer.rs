//! The accounts against the real issuer: `apps/auth-server` in process,
//! seeded with its dev people, over HTTP on loopback.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use architect::action::ActionList;
use architect_accounts::{AccountStore, Accounts, Issuer, IssuerError, MemorySecrets, Status};
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

const PASSWORD: &str = auth_server::dev::DEV_PASSWORD;

/// The issuer, served on loopback: its origin, and its router (to act as
/// a person's browser).
async fn issuer() -> (String, axum::Router) {
    issuer_with(true).await
}

/// The test app's client as the Architect app's is registered: public, a
/// loopback redirect on any port and the app's scheme, and allowed to
/// trade its token for a session when `issue_sessions`.
fn native_client(issue_sessions: bool) -> architect_auth::OidcClientConfig {
    architect_auth::OidcClientConfig {
        client_id: "architect-test".into(),
        client_secret: None,
        name: "Architect (test)".into(),
        redirect_uris: vec![
            "http://127.0.0.1/oauth/callback".into(),
            "app.fasttrackstudio.architect:/oauth/callback".into(),
        ],
        scopes: vec!["openid".into(), "profile".into(), "email".into(), "offline_access".into()],
        public_client: true,
        skip_consent: true,
        disabled: false,
        issue_sessions,
    }
}

async fn issuer_with(issue_sessions: bool) -> (String, axum::Router) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
        oidc_clients: vec![native_client(issue_sessions)],
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    assert!(
        auth_server::dev::seed(&auth, &config.database_url)
            .await
            .unwrap()
    );
    let app = server::app_router_with_db(&config, auth, db).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    (origin, app)
}

fn accounts(origin: &str, secrets: Arc<MemorySecrets>) -> Accounts {
    let store = Arc::new(AccountStore::in_memory(secrets));
    Accounts::new(store, Issuer::new(origin), "architect-test").without_browser()
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_switches_and_signs_out_with_the_session_revoked() {
    let (origin, _) = issuer().await;
    let secrets = Arc::new(MemorySecrets::new());
    let a = accounts(&origin, secrets.clone());

    let wrong = a
        .sign_in_password("ada@local.test", "not-the-password")
        .await;
    assert!(wrong.is_err());
    assert!(matches!(a.status(), Status::Failed(_)), "{:?}", a.status());
    assert!(secrets.is_empty());

    let ada = a
        .sign_in_password("ada@local.test", PASSWORD)
        .await
        .unwrap();
    assert_eq!(ada.name.as_deref(), Some("Ada Lovelace"));
    let grace = a
        .sign_in_password("grace@local.test", PASSWORD)
        .await
        .unwrap();
    assert_eq!(a.active().map(|x| x.key()), Some(grace.key()));
    assert_eq!(secrets.len(), 2);

    // Back to Ada, by her email.
    assert_eq!(a.switch("ada@local.test").unwrap().key(), ada.key());
    let token = a.store().session(&ada.key()).unwrap().unwrap().token;

    // The devices: this one, once the registration landed.
    if auth_client::this_device::install_id().is_some() {
        let mut listed = Vec::new();
        for _ in 0..40 {
            listed = a.refresh_devices().await.unwrap();
            if listed.iter().any(|d| d.this_one) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(listed.iter().any(|d| d.this_one), "{listed:?}");
    }

    // With the accounts page open, the devices follow a switch.
    a.open_sheet(architect_accounts::Sheet::Accounts);
    let devices = |what: &'static str| {
        let a = a.clone();
        async move {
            for _ in 0..200 {
                if matches!(a.devices(), Some(Ok(_))) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("{what}: {:?}", a.devices());
        }
    };
    devices("the devices").await;
    a.switch(&grace.key()).unwrap();
    devices("grace's devices").await;
    a.switch(&ada.key()).unwrap();
    devices("ada's devices").await;
    a.close_sheet();

    // Signing out ends the session at the issuer and leaves the keychain.
    a.sign_out(None).await.unwrap();
    assert_eq!(secrets.len(), 1);
    assert_eq!(a.active().map(|x| x.key()), Some(grace.key()));
    assert_eq!(
        a.issuer().session(&token).await.err(),
        Some(IssuerError::SessionExpired)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_sign_in_lands_once_its_person_approves_the_code() {
    let (origin, app) = issuer().await;
    let secrets = Arc::new(MemorySecrets::new());
    let a = accounts(&origin, secrets.clone());
    a.open_sheet(architect_accounts::Sheet::SignIn);
    a.sign_in_with_code();
    until("the code", || a.browser().is_some()).await;
    let wait = a.browser().unwrap();
    assert!(wait.url.starts_with(&origin), "{}", wait.url);
    assert!(!wait.opened);

    // Alan, signed in on his phone, approves the code.
    let phone = Issuer::new(&origin)
        .sign_in_password("alan@local.test", PASSWORD)
        .await
        .unwrap();
    let approved = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/device/approve")
                .header(
                    header::COOKIE,
                    format!("architect-auth.session={}", phone.session.token),
                )
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("user_code={}", wait.user_code)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);

    until("the sign-in", || a.active().is_some()).await;
    assert_eq!(
        a.active().and_then(|x| x.email).as_deref(),
        Some("alan@local.test")
    );
    assert_eq!(a.sheet(), None);
    assert_eq!(secrets.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_actions_list_switch_and_sign_out_and_never_answer_a_token() {
    let (origin, _) = issuer().await;
    let secrets = Arc::new(MemorySecrets::new());
    let a = accounts(&origin, secrets.clone());
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &a).unwrap();
    a.sign_in_password("ada@local.test", PASSWORD)
        .await
        .unwrap();
    a.sign_in_password("grace@local.test", PASSWORD)
        .await
        .unwrap();
    let token = a.store().active_session().unwrap().unwrap().token;

    let listed = String::from_utf8(list.run_json("ACCOUNT_LIST", b"null").unwrap()).unwrap();
    assert!(
        listed.contains("ada@local.test") && listed.contains("grace@local.test"),
        "{listed}"
    );
    assert!(!listed.contains(&token), "a token in {listed}");

    let switched = list
        .run_json("ACCOUNT_SWITCH", br#"{"account":"ada@local.test"}"#)
        .unwrap();
    assert!(
        String::from_utf8(switched)
            .unwrap()
            .contains("Ada Lovelace")
    );

    let sheet = list.run_json("ACCOUNT_SIGN_IN", b"{}").unwrap();
    assert!(String::from_utf8(sheet).unwrap().contains("sign-in"));

    list.run_json_async("ACCOUNT_SIGN_OUT", b"{}")
        .await
        .unwrap();
    assert_eq!(
        a.active().and_then(|x| x.email).as_deref(),
        Some("grace@local.test")
    );
    assert_eq!(secrets.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_account_has_its_orgs_and_keeps_the_one_it_is_in() {
    let (origin, _) = issuer().await;
    let secrets = Arc::new(MemorySecrets::new());
    let a = accounts(&origin, secrets);
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &a).unwrap();

    // Signed out: the person's own on this device, nothing to switch.
    assert_eq!(
        a.orgs().current().map(|o| o.id.0.clone()).as_deref(),
        Some(architect_accounts::DEVICE_ORG)
    );
    assert!(list.run_json("ORG_SWITCH", br#"{"org":"acme"}"#).is_err());

    // Grace is in both seeded orgs; the issuer has no personal org, so the
    // device's comes first.
    a.sign_in_password("grace@local.test", PASSWORD)
        .await
        .unwrap();
    until("grace's orgs", || a.orgs().orgs.len() == 3).await;
    let listed =
        String::from_utf8(list.run_json_async("ORG_LIST", b"null").await.unwrap()).unwrap();
    assert!(
        listed.contains("\"slug\":\"acme\"") && listed.contains("\"slug\":\"indie\""),
        "{listed}"
    );
    assert!(listed.contains("\"stale\":null"), "{listed}");
    assert!(
        !listed.contains(&a.store().active_session().unwrap().unwrap().token),
        "{listed}"
    );

    let switched =
        String::from_utf8(list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap()).unwrap();
    let indie = a.orgs().current().cloned().unwrap();
    assert_eq!(indie.slug, "indie");
    assert_eq!(indie.role, architect_orgs::Role::Owner);
    assert!(
        switched.contains(&format!("\"current\":\"{}\"", indie.id.0)),
        "{switched}"
    );
    // The session and the org, where a vox client reads them.
    let active = a.active_session();
    assert_eq!(active.org().map(|o| o.slug), Some("indie".to_owned()));
    // The issuer hears it as the session's active org.
    let token = a.store().active_session().unwrap().unwrap().token;
    let auth = auth_proto::AuthServiceHttpClient::at(origin.clone());
    let mut told = None;
    for _ in 0..200 {
        told = auth
            .current_session(token.clone())
            .await
            .unwrap()
            .session
            .active_organization_id;
        if told.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(told.map(|u| u.to_string()), Some(indie.id.0.clone()));
    assert!(
        list.run_json("ORG_SWITCH", br#"{"org":"nowhere"}"#)
            .is_err()
    );

    // Another account has its own orgs and its own current one.
    a.sign_in_password("alan@local.test", PASSWORD)
        .await
        .unwrap();
    until("alan's orgs", || a.orgs().orgs.len() == 2).await;
    assert_eq!(a.orgs().current().map(|o| o.personal), Some(true));
    a.switch("grace@local.test").unwrap();
    assert_eq!(
        a.orgs().current().map(|o| o.slug.clone()).as_deref(),
        Some("indie")
    );

    // All, kept too.
    list.run_json("ORG_SWITCH", br#"{"org":"all"}"#).unwrap();
    assert!(a.orgs().current().is_none() && a.active_session().scope().is_all());
}


/// The person's browser on the issuer: signed in as `email`, it opens the
/// sign-in page the app opened and follows the issuer's redirect, which
/// must lead to the app's loopback port; answers that redirect.
async fn browser_signs_in(origin: &str, page: &str, email: &str) -> String {
    let session = Issuer::new(origin)
        .sign_in_password(email, PASSWORD)
        .await
        .unwrap()
        .session
        .token;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let reply = http
        .get(page)
        .header("cookie", format!("architect-auth.session={session}"))
        .send()
        .await
        .unwrap();
    assert!(reply.status().is_redirection(), "{}", reply.status());
    let back = reply.headers()["location"].to_str().unwrap().to_owned();
    assert!(
        back.starts_with("http://127.0.0.1:") && back.contains("/oauth/callback?code="),
        "the issuer sends the browser to the app's loopback port"
    );
    back
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_sign_in_comes_back_on_the_apps_loopback_port_signed_in() {
    let (origin, _) = issuer().await;
    let secrets = Arc::new(MemorySecrets::new());
    let a = accounts(&origin, secrets.clone());
    a.sign_in();
    until("the sign-in page", || a.browser().is_some()).await;
    let wait = a.browser().unwrap();
    assert!(wait.url.starts_with(&format!("{origin}/oauth2/authorize?")), "the issuer's own page");
    assert!(wait.url.contains("code_challenge_method=S256"));
    assert!(wait.user_code.is_empty(), "no code to type: the browser comes back");

    let back = browser_signs_in(&origin, &wait.url, "ada@local.test").await;
    // The browser lands on the app's port: a page saying to go back.
    let landed = reqwest::get(&back).await.unwrap();
    assert_eq!(landed.status(), 200);
    assert!(landed.text().await.unwrap().contains("signed in"));

    until("the account", || a.active().is_some()).await;
    assert_eq!(a.active().and_then(|x| x.email).as_deref(), Some("ada@local.test"));
    assert_eq!(a.sheet(), None);
    assert_eq!(secrets.len(), 1, "the session, in the keychain");
    // A session the issuer's services take (what orgs and devices need).
    let orgs = a.refresh_orgs().await.unwrap();
    assert!(orgs.orgs.iter().any(|o| o.slug == "acme"), "Ada's org");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_with_another_state_signs_no_one_in() {
    let (origin, _) = issuer().await;
    let a = accounts(&origin, Arc::new(MemorySecrets::new()));
    a.sign_in();
    until("the sign-in page", || a.browser().is_some()).await;
    let page = a.browser().unwrap().url;
    let back = browser_signs_in(&origin, &page, "ada@local.test").await;
    // A forged or stale callback: the code, another state.
    let (head, _) = back.split_once("&state=").unwrap();
    let _ = reqwest::get(format!("{head}&state=forged")).await.unwrap();
    until("the refusal", || matches!(a.status(), Status::Failed(_))).await;
    assert!(a.active().is_none());
    assert!(a.browser().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_the_issuer_does_not_let_mint_sessions_signs_no_one_in() {
    let (origin, _) = issuer_with(false).await;
    let a = accounts(&origin, Arc::new(MemorySecrets::new()));
    a.sign_in();
    until("the sign-in page", || a.browser().is_some()).await;
    let page = a.browser().unwrap().url;
    let back = browser_signs_in(&origin, &page, "ada@local.test").await;
    let _ = reqwest::get(&back).await.unwrap();
    until("the refusal", || matches!(a.status(), Status::Failed(_))).await;
    assert!(a.active().is_none(), "the code alone is no session");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_cancelled_leaves_the_app_as_it_was() {
    let (origin, _) = issuer().await;
    let a = accounts(&origin, Arc::new(MemorySecrets::new()));
    a.sign_in();
    until("the sign-in page", || a.browser().is_some()).await;
    let page = a.browser().unwrap().url;
    a.cancel_browser();
    assert!(a.browser().is_none());
    assert_eq!(a.status(), Status::Idle);
    // Its person finishes in the browser anyway: nothing lands.
    let back = browser_signs_in(&origin, &page, "ada@local.test").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let _ = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(&back)
        .send()
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(a.active().is_none());
}
