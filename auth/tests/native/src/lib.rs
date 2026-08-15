#[cfg(test)]
mod permissions_gate_tests;

/// In-memory SQLite `ArchitectAuth` — the storage every vox round-trip
/// test mounts behind the service.
#[cfg(test)]
async fn open_auth() -> auth::ArchitectAuth<auth::backend_db::AuthSeaOrmStorage> {
    use auth::backend_db::{AuthSeaOrmStorage, Migrator};
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    Migrator::up(&db, None).await.expect("run migrations");
    auth::ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(AuthSeaOrmStorage::new(db))
        .build()
        .expect("build auth")
}

/// Probe service for the wire-format test: echoes back whatever token
/// `AuthServerMiddleware` extracted into the request extensions. Lives
/// in a private module so the macro-emitted `pub` plumbing stays
/// crate-internal.
#[cfg(test)]
mod session_probe {
    #[vox::service]
    pub trait SessionProbe {
        #[vox::context]
        async fn seen_token(&self) -> Option<String>;
    }

    #[derive(Clone)]
    pub struct SessionProbeService;

    impl SessionProbe for SessionProbeService {
        async fn seen_token(&self, cx: &vox::RequestContext<'_>) -> Option<String> {
            cx.extensions()
                .get_cloned::<auth::transport::vox::AuthVoxContext>()
                .and_then(|context| context.token)
        }
    }
}

// r[verify auth.core.secret-minimum]
#[test]
fn builder_rejects_short_secret() {
    let err = match auth::ArchitectAuth::builder().secret("short").build() {
        Ok(_) => panic!("short secret should be rejected"),
        Err(err) => err,
    };
    assert!(matches!(err, auth::config::ConfigError::SecretTooShort));
}

// r[verify auth.storage.backend-parity]
// r[verify auth.storage.transactions]
// r[verify auth.storage.clock]
#[tokio::test]
async fn sea_orm_storage_runs_core_runtime_flows() {
    use auth::{
        AddTeamMember, ArchitectAuth, AuthAuditEvent, AuthMemberCreate, AuthStorage,
        AuthorizeOrganizationAction, CreateEmailPasswordUser, CreateOrganization,
        CreateOrganizationRole, CreateTeam, CurrentSession, ListTeamMembers, ListTeams,
        SignInEmailPassword, SignOut, backend_db::AuthAuditEventRecordEntity,
        backend_db::AuthSeaOrmStorage, backend_db::Migrator,
    };
    use chrono::Utc;
    use sea_orm::{Database, EntityTrait, PaginatorTrait};
    use sea_orm_migration::MigratorTrait;

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    Migrator::up(&db, None).await.expect("run migrations");

    let storage = AuthSeaOrmStorage::new(db);
    assert_eq!(storage.capabilities().backend, "sea-orm");
    assert!(storage.capabilities().transactions);

    let auth = ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(storage.clone())
        .build()
        .expect("build auth");

    let created = auth
        .create_email_password_user(CreateEmailPasswordUser {
            email: "User@Example.COM".into(),
            password: "correct horse battery staple".into(),
            name: Some("User".into()),
            username: None,
            image: None,
            metadata_json: Some(r#"{"tier":"pro"}"#.into()),
            ip_address: Some("127.0.0.1".into()),
            user_agent: Some("native-db-test".into()),
        })
        .await
        .expect("create user");
    assert_eq!(created.user.email.as_deref(), Some("User@example.com"));

    let current = auth
        .current_session(CurrentSession {
            token: created.token.clone(),
        })
        .await
        .expect("current session");
    assert_eq!(current.user.id, created.user.id);

    auth.sign_out(SignOut {
        token: created.token.clone(),
    })
    .await
    .expect("sign out");
    assert!(matches!(
        auth.current_session(CurrentSession {
            token: created.token
        })
        .await,
        Err(auth::AuthFlowError::SessionExpired)
    ));

    let signed_in = auth
        .sign_in_email_password(SignInEmailPassword {
            email: "User@example.com".into(),
            password: "correct horse battery staple".into(),
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("sign in");
    assert_eq!(signed_in.user.id, created.user.id);

    let organization = auth
        .create_organization(CreateOrganization {
            session_token: signed_in.token.clone(),
            name: "Acme".into(),
            slug: "acme".into(),
            logo: None,
            metadata_json: None,
        })
        .await
        .expect("create organization");
    let admin_delete_org = auth
        .authorize_organization_action(AuthorizeOrganizationAction {
            session_token: signed_in.token.clone(),
            organization_id: organization.organization.id,
            resource: "organization".into(),
            action: "delete".into(),
        })
        .await;
    assert!(admin_delete_org.is_ok(), "owner can delete organization");
    let admin = auth
        .create_email_password_user(CreateEmailPasswordUser {
            email: "admin@example.com".into(),
            password: "correct horse battery staple".into(),
            name: None,
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("create admin");
    storage
        .create_member(AuthMemberCreate {
            organization_id: organization.organization.id,
            user_id: admin.user.id,
            role: "admin".into(),
        })
        .await
        .expect("create admin membership");
    let admin_delete_org = auth
        .authorize_organization_action(AuthorizeOrganizationAction {
            session_token: admin.token,
            organization_id: organization.organization.id,
            resource: "organization".into(),
            action: "delete".into(),
        })
        .await;
    assert!(matches!(
        admin_delete_org,
        Err(auth::AuthFlowError::PermissionDenied)
    ));
    auth.create_organization_role(CreateOrganizationRole {
        session_token: signed_in.token.clone(),
        organization_id: organization.organization.id,
        role: "billing".into(),
        permissions_json: r#"{"team":["create"]}"#.into(),
    })
    .await
    .expect("create dynamic organization role");
    auth.authorize_organization_action(AuthorizeOrganizationAction {
        session_token: signed_in.token.clone(),
        organization_id: organization.organization.id,
        resource: "team".into(),
        action: "create".into(),
    })
    .await
    .expect("SeaORM storage preserves organization permission grants");
    let team = auth
        .create_team(CreateTeam {
            session_token: signed_in.token.clone(),
            organization_id: organization.organization.id,
            name: "Platform".into(),
        })
        .await
        .expect("create team");
    let teams = auth
        .list_teams(ListTeams {
            session_token: signed_in.token.clone(),
            organization_id: organization.organization.id,
        })
        .await
        .expect("list teams");
    assert_eq!(teams.len(), 1);
    auth.add_team_member(AddTeamMember {
        session_token: signed_in.token.clone(),
        organization_id: organization.organization.id,
        team_id: team.id,
        user_id: signed_in.user.id,
    })
    .await
    .expect("add team member");
    let team_members = auth
        .list_team_members(ListTeamMembers {
            session_token: signed_in.token.clone(),
            organization_id: organization.organization.id,
            team_id: team.id,
        })
        .await
        .expect("list team members");
    assert_eq!(team_members.len(), 1);

    storage
        .record_audit_event(AuthAuditEvent {
            actor_id: created.user.id,
            target_id: Some(created.user.id),
            action: "native.audit".into(),
            created_at: Utc::now(),
        })
        .await
        .expect("record audit event");
    let audit_count = AuthAuditEventRecordEntity::find()
        .count(storage.db())
        .await
        .expect("count audit events");
    assert_eq!(audit_count, 1);
}

/// The client kit's metadata key must match the proto's — auth-client
/// deliberately doesn't depend on auth-proto (wasm cleanliness), so the
/// constant is duplicated and pinned here.
#[test]
fn authorization_metadata_key_stays_in_sync() {
    assert_eq!(
        auth_client::AUTHORIZATION_METADATA_KEY,
        auth::transport::vox::AUTHORIZATION_METADATA_KEY,
    );
}

// r[verify auth.transport.vox-schema]
// r[verify auth.sessions.refresh]
// r[verify auth.sessions.refresh-rotation]
// r[verify auth.sessions.refresh-invalid]
#[tokio::test]
async fn vox_session_surface_round_trips_over_local_server() {
    use architect::{LayerRouter, LocalServer, Scope};
    use auth::transport::vox::AuthVoxService;
    use auth::{AuthFlowError, AuthServiceClient, SignInEmailPassword, SignUpEmailPassword};

    let auth = open_auth().await;
    // Mount exactly the way a downstream server does: the rpc-emitted
    // `auth_service_layer` binds the vox service to its descriptor.
    let router = LayerRouter::new().merge(auth::auth_service_layer(AuthVoxService::new(auth)));
    let scope = Scope::new();
    let local = LocalServer::serve(router, scope.clone());
    let client: AuthServiceClient = local.establish().await.expect("establish auth client");

    // sign_up → whoami: the issued token resolves to the new user.
    let signed_up = client
        .sign_up_email_password(SignUpEmailPassword {
            email: "rpc@example.com".into(),
            password: "correct horse battery staple".into(),
            name: Some("Rpc User".into()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: Some("127.0.0.1".into()),
            user_agent: Some("vox-round-trip-test".into()),
        })
        .await
        .expect("sign up");
    let me = client
        .whoami(signed_up.token.clone())
        .await
        .expect("whoami");
    assert_eq!(me.id, signed_up.user.id);

    // refresh rotates: new token works, old token is dead.
    let refreshed = client
        .refresh_session(signed_up.token.clone())
        .await
        .expect("refresh session");
    assert_eq!(refreshed.user.id, signed_up.user.id);
    assert_ne!(refreshed.token, signed_up.token);
    assert_eq!(
        refreshed.session.ip_address.as_deref(),
        Some("127.0.0.1"),
        "refresh preserves session context"
    );
    client
        .current_session(refreshed.token.clone())
        .await
        .expect("refreshed token validates");
    assert!(matches!(
        &client.current_session(signed_up.token.clone()).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::SessionExpired)
    ));

    // refresh with garbage fails like current_session and issues nothing.
    assert!(matches!(
        &client.refresh_session("not-a-token".into()).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::InvalidCredentials)
    ));

    // logout, then the rotated token is dead too.
    client
        .sign_out(refreshed.token.clone())
        .await
        .expect("sign out");
    assert!(matches!(
        &client.whoami(refreshed.token).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::SessionExpired)
    ));

    // login again over the same surface.
    let signed_in = client
        .sign_in_email_password(SignInEmailPassword {
            email: "rpc@example.com".into(),
            password: "correct horse battery staple".into(),
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("sign in");
    assert_eq!(signed_in.user.id, signed_up.user.id);

    scope.close().await;
}

// r[verify auth.transport.vox-schema]
#[tokio::test]
async fn token_store_middleware_matches_auth_server_middleware_wire_format() {
    use std::sync::Arc;

    use architect::{LayerRouter, LocalServer, Scope};
    use auth::transport::vox::AuthServerMiddleware;
    use auth_client::{MemoryTokenStore, StoredSession, TokenStore, TokenStoreMiddleware};
    use session_probe::{
        SessionProbeClient, SessionProbeDispatcher, SessionProbeService,
        session_probe_service_descriptor,
    };

    let router = LayerRouter::new().with(
        session_probe_service_descriptor(),
        SessionProbeDispatcher::new(SessionProbeService).with_middleware(AuthServerMiddleware),
    );
    let scope = Scope::new();
    let local = LocalServer::serve(router, scope.clone());

    let store = Arc::new(MemoryTokenStore::new());
    let probe: SessionProbeClient = local.establish().await.expect("establish probe client");
    let probe = probe.with_middleware(TokenStoreMiddleware::new(store.clone()));

    // Empty store → unauthenticated call, no token on the server side.
    assert_eq!(
        probe.seen_token().await.expect("call without session"),
        None
    );

    // Stored token arrives exactly as AuthServerMiddleware parses it.
    store
        .save(&StoredSession::new("session-token-123"))
        .expect("save session");
    assert_eq!(
        probe.seen_token().await.expect("call with session"),
        Some("session-token-123".into())
    );

    // Cleared store → back to unauthenticated, same clients.
    store.clear().expect("clear session");
    assert_eq!(probe.seen_token().await.expect("call after clear"), None);

    scope.close().await;
}
