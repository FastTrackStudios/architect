//! What a snapshot carries, and — more to the point — what it does not.
//!
//! The interesting assertions here are the negative ones. A test that
//! only checked the organizations came across would still pass with
//! every password hash in the file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use auth_db::snapshot::{self, ExportOptions, SnapshotError};
use auth_db::{
    AuthAccountActiveModel, AuthInviteLinkActiveModel, AuthMemberActiveModel,
    AuthOrganizationActiveModel, AuthSessionActiveModel, AuthUserActiveModel, Migrator,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, EntityTrait, PaginatorTrait, Set};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// A database standing in for production: a user with a password, a
/// live session, an OAuth account holding tokens, an organization they
/// own, and an invite link.
async fn production_like() -> (DatabaseConnection, Uuid, Uuid) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let now = Utc::now();

    let user_id = Uuid::new_v4();
    AuthUserActiveModel {
        id: Set(user_id),
        email: Set(Some("ada@example.com".into())),
        email_verified: Set(true),
        name: Set(Some("Ada Lovelace".into())),
        username: Set(Some("ada".into())),
        display_username: Set(Some("ada".into())),
        image: Set(None),
        two_factor_enabled: Set(true),
        role: Set(Some("admin".into())),
        banned: Set(false),
        ban_reason: Set(None),
        ban_expires: Set(None),
        metadata_json: Set(String::new()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();

    for (provider, password, access) in [
        ("credential", Some("$argon2id$v=19$SUPERSECRETHASH"), None),
        ("github", None, Some("gho_ACTUALLYLIVETOKEN")),
    ] {
        AuthAccountActiveModel {
            id: Set(Uuid::new_v4()),
            account_id: Set(user_id.to_string()),
            provider_id: Set(provider.into()),
            user_id: Set(user_id),
            access_token_ciphertext: Set(access.map(str::to_owned)),
            refresh_token_ciphertext: Set(None),
            id_token_ciphertext: Set(None),
            access_token_expires_at: Set(None),
            refresh_token_expires_at: Set(None),
            scope: Set(None),
            password_hash: Set(password.map(str::to_owned)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();
    }

    AuthSessionActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        token_hash: Set("A-LIVE-SESSION-TOKEN-HASH".into()),
        expires_at: Set(now + chrono::Duration::days(30)),
        ip_address: Set(Some("203.0.113.7".into())),
        user_agent: Set(Some("Firefox".into())),
        impersonated_by: Set(None),
        active_organization_id: Set(None),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();

    let org_id = Uuid::new_v4();
    AuthOrganizationActiveModel {
        id: Set(org_id),
        name: Set("Acme Records".into()),
        slug: Set("acme-records".into()),
        logo: Set(None),
        metadata_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    AuthMemberActiveModel {
        id: Set(Uuid::new_v4()),
        organization_id: Set(org_id),
        user_id: Set(user_id),
        role: Set("owner".into()),
        created_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    AuthInviteLinkActiveModel {
        id: Set(Uuid::new_v4()),
        organization_id: Set(org_id),
        token_hash: Set("A-LIVE-INVITE-LINK-HASH".into()),
        label: Set(Some("Launch week".into())),
        role: Set("member".into()),
        created_by: Set(user_id),
        expires_at: Set(None),
        max_uses: Set(Some(10)),
        uses: Set(3),
        revoked_at: Set(None),
        created_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();

    (db, user_id, org_id)
}

async fn empty_local() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

#[tokio::test]
async fn no_credential_survives_the_export() {
    let (db, _, _) = production_like().await;
    let snapshot = snapshot::export(&db, ExportOptions::default())
        .await
        .unwrap();

    // The whole file, as it would be written to disk and carried away.
    let json = serde_json::to_string(&snapshot).unwrap();
    for secret in [
        "$argon2id$v=19$SUPERSECRETHASH",
        "gho_ACTUALLYLIVETOKEN",
        "A-LIVE-SESSION-TOKEN-HASH",
        "A-LIVE-INVITE-LINK-HASH",
    ] {
        assert!(
            !json.contains(secret),
            "a credential reached the snapshot: {secret}"
        );
    }
}

#[tokio::test]
async fn the_graph_does_survive_the_export() {
    let (db, user_id, org_id) = production_like().await;
    let snapshot = snapshot::export(&db, ExportOptions::default())
        .await
        .unwrap();

    assert_eq!(snapshot.users.len(), 1);
    assert_eq!(snapshot.users[0].id, user_id);
    assert_eq!(snapshot.users[0].email.as_deref(), Some("ada@example.com"));
    assert_eq!(snapshot.users[0].role.as_deref(), Some("admin"));
    assert_eq!(snapshot.organizations.len(), 1);
    assert_eq!(snapshot.organizations[0].id, org_id);
    assert_eq!(snapshot.members.len(), 1);
    assert_eq!(snapshot.members[0].role, "owner");
    // The link is visible with its spend, which is what makes the org
    // page render the same locally as it does in production.
    assert_eq!(snapshot.invite_links.len(), 1);
    assert_eq!(snapshot.invite_links[0].uses, 3);
    assert_eq!(snapshot.invite_links[0].max_uses, Some(10));
}

#[tokio::test]
async fn redaction_keeps_addresses_distinct_without_keeping_them() {
    let (db, _, _) = production_like().await;
    let snapshot = snapshot::export(
        &db,
        ExportOptions {
            redact_emails: true,
        },
    )
    .await
    .unwrap();
    assert!(snapshot.emails_redacted);
    let email = snapshot.users[0].email.clone().unwrap();
    assert!(!email.contains("ada@example.com"));
    assert!(email.ends_with("@local.invalid"), "{email}");
}

#[tokio::test]
async fn an_imported_account_can_sign_in_with_the_one_known_password() {
    let (source, user_id, org_id) = production_like().await;
    let snapshot = snapshot::export(&source, ExportOptions::default())
        .await
        .unwrap();

    let target = empty_local().await;
    snapshot::import(&target, "sqlite::memory:", &snapshot, "$argon2id$local-dev")
        .await
        .unwrap();

    assert_eq!(
        auth_db::AuthUserEntity::find()
            .count(&target)
            .await
            .unwrap(),
        1
    );
    // No session came across: you sign in locally, you do not inherit
    // somebody's live browser.
    assert_eq!(
        auth_db::AuthSessionEntity::find()
            .count(&target)
            .await
            .unwrap(),
        0
    );
    // Exactly one account row per user, and it is the credential one
    // this machine minted — the GitHub account with its live token did
    // not follow.
    let accounts = auth_db::AuthAccountEntity::find()
        .all(&target)
        .await
        .unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].provider_id, "credential");
    assert_eq!(accounts[0].user_id, user_id);
    assert_eq!(
        accounts[0].password_hash.as_deref(),
        Some("$argon2id$local-dev")
    );

    let members = auth_db::AuthMemberEntity::find()
        .all(&target)
        .await
        .unwrap();
    assert_eq!(members[0].organization_id, org_id);
}

#[tokio::test]
async fn an_imported_invite_link_is_visible_and_unusable() {
    let (source, _, _) = production_like().await;
    let snapshot = snapshot::export(&source, ExportOptions::default())
        .await
        .unwrap();
    let target = empty_local().await;
    snapshot::import(&target, "sqlite::memory:", &snapshot, "hash")
        .await
        .unwrap();

    let links = auth_db::AuthInviteLinkEntity::find()
        .all(&target)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].label.as_deref(), Some("Launch week"));
    // Whatever is in `token_hash` locally, it is not production's, and
    // no token hashes to it.
    assert_ne!(links[0].token_hash, "A-LIVE-INVITE-LINK-HASH");
    assert!(links[0].token_hash.starts_with("imported:"));
}

#[tokio::test]
async fn importing_refuses_a_database_that_is_not_local() {
    let (source, _, _) = production_like().await;
    let snapshot = snapshot::export(&source, ExportOptions::default())
        .await
        .unwrap();
    let target = empty_local().await;

    // This is the guard that matters: import writes one known password
    // onto every account it creates.
    let refused = snapshot::import(
        &target,
        "postgres://auth:pw@auth-db.prod.svc.cluster.local:5432/auth",
        &snapshot,
        "hash",
    )
    .await;
    assert!(matches!(refused, Err(SnapshotError::NotLocal(_))));
    // And the message it refuses with carries no password.
    let message = refused.unwrap_err().to_string();
    assert!(!message.contains("pw@"), "{message}");
    assert!(message.contains("***"), "{message}");
}

#[tokio::test]
async fn importing_refuses_a_database_that_already_holds_people() {
    let (source, _, _) = production_like().await;
    let snapshot = snapshot::export(&source, ExportOptions::default())
        .await
        .unwrap();
    let target = empty_local().await;
    snapshot::import(&target, "sqlite::memory:", &snapshot, "hash")
        .await
        .unwrap();

    let again = snapshot::import(&target, "sqlite::memory:", &snapshot, "hash").await;
    assert!(matches!(again, Err(SnapshotError::NotEmpty(1))));
}

#[tokio::test]
async fn a_snapshot_from_another_schema_is_refused_rather_than_half_loaded() {
    let (source, _, _) = production_like().await;
    let mut snapshot = snapshot::export(&source, ExportOptions::default())
        .await
        .unwrap();
    snapshot.version += 1;
    let target = empty_local().await;
    let refused = snapshot::import(&target, "sqlite::memory:", &snapshot, "hash").await;
    assert!(matches!(refused, Err(SnapshotError::Version { .. })));
    assert_eq!(
        auth_db::AuthUserEntity::find()
            .count(&target)
            .await
            .unwrap(),
        0,
        "a refused import must not have written anything"
    );
}
