//! Postgres coverage.
//!
//! `auth-db`'s support matrix lists Postgres as "planned" — the
//! migrations are written with SeaORM's backend-agnostic schema builder,
//! but nothing had ever run them against a real Postgres. The central
//! identity server is deployed on Postgres, so that gap is exactly where
//! a production-only failure would hide.
//!
//! Skipped unless `AUTH_TEST_POSTGRES_URL` is set, so the default
//! `cargo test` stays hermetic:
//!
//! ```sh
//! podman run -d --rm -e POSTGRES_PASSWORD=test -e POSTGRES_DB=authtest \
//!     -p 55432:5432 postgres:16-alpine
//! AUTH_TEST_POSTGRES_URL=postgres://postgres:test@localhost:55432/authtest \
//!     cargo test -p auth-server --test postgres
//! ```

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::{ConnectionTrait, Database, Statement};
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

fn postgres_url() -> Option<String> {
    std::env::var("AUTH_TEST_POSTGRES_URL").ok()
}

/// Provision a scratch database and return its URL.
///
/// Each test gets its own. They run concurrently in one binary, and a
/// test that resets schema would otherwise pull the tables out from
/// under a test that is mid-flow.
async fn scratch_database(base_url: &str, name: &str) -> String {
    let db = Database::connect(base_url)
        .await
        .expect("connect maintenance database");
    // One statement per call: Postgres' extended-query protocol rejects
    // multiple commands in a single prepared statement.
    for sql in [
        format!("drop database if exists {name}"),
        format!("create database {name}"),
    ] {
        db.execute(Statement::from_string(db.get_database_backend(), sql))
            .await
            .expect("provision scratch database");
    }
    let (prefix, _) = base_url
        .rsplit_once('/')
        .expect("url has a database segment");
    format!("{prefix}/{name}")
}

fn test_config(database_url: String) -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url,
        secret: "a-secret-at-least-32-bytes-long!!".into(),
        base_url: "https://auth.fasttrackstudio.app".into(),
        oidc_issuer: None,
        session_ttl_seconds: 3600,
        require_email_verification: false,
        passkey_rp_id: None,
        cors_origins: Vec::new(),
        oidc_clients: Vec::new(),
        oidc_allow_dynamic_client_registration: false,
        run_migrations: true,
    }
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent_on_postgres() {
    let Some(url) = postgres_url() else {
        eprintln!("AUTH_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    let scratch = scratch_database(&url, "auth_migration_check").await;
    let db = Database::connect(&scratch)
        .await
        .expect("connect scratch database");

    Migrator::up(&db, None).await.expect("migrations apply");
    // Re-running must be a no-op — an operator relies on that every time
    // a pod restarts with `AUTH_RUN_MIGRATIONS=true`.
    Migrator::up(&db, None)
        .await
        .expect("migrations are idempotent");

    let tables = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "select tablename from pg_tables where schemaname = 'public' \
             and tablename like 'auth_%' order by tablename",
        ))
        .await
        .expect("list tables");
    let names: Vec<String> = tables
        .iter()
        .map(|row| row.try_get_by_index::<String>(0).expect("tablename"))
        .collect();

    for expected in [
        "auth_accounts",
        "auth_api_keys",
        "auth_audit_events",
        "auth_invitations",
        "auth_members",
        "auth_organizations",
        "auth_passkeys",
        "auth_sessions",
        "auth_two_factors",
        "auth_users",
        "auth_verifications",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing table {expected} on postgres; got {names:?}"
        );
    }
}

#[tokio::test]
async fn full_sign_up_and_session_flow_works_on_postgres() {
    let Some(url) = postgres_url() else {
        eprintln!("AUTH_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    let scratch = scratch_database(&url, "auth_flow_check").await;
    let db = Database::connect(&scratch)
        .await
        .expect("connect scratch database");
    Migrator::up(&db, None).await.expect("migrate");

    let config = test_config(scratch);
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    let app = server::app_router(&config, auth);

    let email = "pg@fasttrackstudio.app";
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let token = json["token"].as_str().expect("token").to_owned();

    // Reading the session back proves the write landed in Postgres and
    // the token hash matched on the way out.
    let response = app
        .oneshot(
            Request::get("/auth/session")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("session");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(json["user"]["email"], email);
}
