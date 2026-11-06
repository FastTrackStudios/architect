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
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
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
    a.sign_in_browser();
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
