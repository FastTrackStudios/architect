//! The two ways to get a server running on your own machine.
//!
//! # 1. Fresh
//!
//! [`seed`] fills an empty database with a small, *fixed* cast: three
//! people, two organizations, a team, a pending invitation and a live
//! invite link. Fixed matters — every id and address is derived from a
//! constant namespace, so the same seed produces the same database
//! every time, which is what lets a browser test assert on
//! `ada@local.test` being an owner of `acme` rather than on whatever
//! the fixture happened to create.
//!
//! # 2. A copy of production, with the credentials taken out
//!
//! [`snapshot_route`] serves an admin-only, already-sanitised copy of
//! the database — see [`auth_db::snapshot`] for exactly what is left
//! out and why. The scrubbing happens here, on the server, so the
//! plaintext never leaves the machine that holds it; a client that
//! fetched everything and scrubbed afterwards would have had it all in
//! memory on a laptop first.
//!
//! Both refuse to do anything unless they are asked to. Seeding is
//! gated on [`DevConfig::enabled`], which `ServerConfig::load` only
//! sets from `AUTH_DEV_SEED=1`, and the snapshot route needs a real
//! administrator's session.

use architect_auth::crypto::hash_password;
use architect_auth::db::snapshot::{self, ExportOptions};
use architect_auth::{ArchitectAuth, AuthStorage, AuthorizeAdmin};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::oauth::HttpState;

/// The password every seeded account has.
///
/// A constant, and deliberately not a secret: the whole point of a
/// local server is that you can sign in as anybody on it. It is only
/// ever written to a database that [`snapshot::is_local`] approves of.
pub const DEV_PASSWORD: &str = "development-password";

/// The namespace seeded ids are derived in.
///
/// A v5 UUID from a fixed namespace and a fixed name is the same UUID
/// on every machine and every run, so a test can hard-code one.
const DEV_NAMESPACE: Uuid = Uuid::from_u128(0x0191_5eed_0000_4000_8000_000000000001);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DevConfig {
    /// Seed an empty database on boot. `AUTH_DEV_SEED=1`.
    pub enabled: bool,
}

/// The stable id for a seeded thing.
#[must_use]
pub fn dev_id(name: &str) -> Uuid {
    Uuid::new_v5(&DEV_NAMESPACE, name.as_bytes())
}

/// Who and what a fresh local server starts with.
///
/// Listed here rather than built inline so the set is readable in one
/// screen — this table *is* the fixture a browser test writes against.
pub const DEV_PEOPLE: [(&str, &str, &str); 3] = [
    ("ada@local.test", "Ada Lovelace", "admin"),
    ("grace@local.test", "Grace Hopper", "user"),
    ("alan@local.test", "Alan Turing", "user"),
];

/// `(slug, name, owner email, second member email, their role)`
pub const DEV_ORGS: [(&str, &str, &str, &str, &str); 2] = [
    (
        "acme",
        "Acme Records",
        "ada@local.test",
        "grace@local.test",
        "admin",
    ),
    (
        "indie",
        "Indie Collective",
        "grace@local.test",
        "alan@local.test",
        "member",
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("seeding: {0}")]
    Flow(#[from] architect_auth::proto::AuthFlowError),
    #[error("hashing the development password: {0}")]
    Password(#[from] architect_auth::crypto::PasswordError),
    #[error(
        "refusing to seed {0}: seeding creates accounts with a published password, so it may \
         only run against a local database"
    )]
    NotLocal(String),
}

/// Fill an empty local database with the fixed cast.
///
/// Idempotent by way of being a no-op on a database that already has
/// people in it, so a dev server can be restarted without accumulating
/// duplicates or failing on unique constraints.
///
/// # Errors
///
/// [`SeedError::NotLocal`] if `database_url` is not a local database,
/// and whatever the engine returns while creating the fixture.
pub async fn seed<S>(auth: &ArchitectAuth<S>, database_url: &str) -> Result<bool, SeedError>
where
    S: AuthStorage,
{
    if !snapshot::is_local(database_url) {
        return Err(SeedError::NotLocal(snapshot::redact_url(database_url)));
    }

    let mut tokens = std::collections::HashMap::new();
    for (email, name, role) in DEV_PEOPLE {
        let bundle = auth
            .create_email_password_user(architect_auth::CreateEmailPasswordUser {
                email: email.to_owned(),
                password: DEV_PASSWORD.to_owned(),
                name: Some(name.to_owned()),
                username: None,
                image: None,
                metadata_json: None,
                ip_address: None,
                user_agent: None,
            })
            .await;
        match bundle {
            Ok(bundle) => {
                if role == "admin" {
                    auth.set_user_role_local_trusted(bundle.user.id, Some(role.to_owned()))
                        .await?;
                }
                tokens.insert(email, bundle.token);
            }
            // Already seeded. Nothing here is worth failing a boot over.
            Err(_) => return Ok(false),
        }
    }

    for (slug, name, owner_email, guest_email, guest_role) in DEV_ORGS {
        let Some(owner_token) = tokens.get(owner_email) else {
            continue;
        };
        let org = auth
            .create_organization(architect_auth::CreateOrganization {
                session_token: owner_token.clone(),
                name: name.to_owned(),
                slug: slug.to_owned(),
                logo: None,
                metadata_json: None,
            })
            .await?;

        // A second member, joined through a link — which also leaves a
        // spent link on the page, so the org screen has something in
        // every section on a fresh server.
        let link = auth
            .create_invite_link(architect_auth::CreateInviteLink {
                session_token: owner_token.clone(),
                organization_id: org.organization.id,
                role: guest_role.to_owned(),
                label: Some("Seeded link".to_owned()),
                expires_at: None,
                max_uses: None,
            })
            .await?;
        if let Some(guest_token) = tokens.get(guest_email) {
            auth.redeem_invite_link(architect_auth::RedeemInviteLink {
                session_token: guest_token.clone(),
                token: link.token,
            })
            .await?;
        }

        auth.create_team(architect_auth::CreateTeam {
            session_token: owner_token.clone(),
            organization_id: org.organization.id,
            name: "Mastering".to_owned(),
        })
        .await?;

        auth.create_invitation(architect_auth::CreateInvitation {
            session_token: owner_token.clone(),
            organization_id: org.organization.id,
            email: format!("pending@{slug}.local.test"),
            role: "member".to_owned(),
            expires_at: architect_auth::expiry::in_days(14),
        })
        .await?;
    }
    Ok(true)
}

/// Load a snapshot file into the database, if one was configured.
///
/// Returns what was loaded, for the log line — or `None` when there
/// was nothing to do.
///
/// # Errors
///
/// If the file cannot be read or parsed, or the import is refused —
/// see [`snapshot::import`] for the guards.
pub async fn import_file(
    db: &DatabaseConnection,
    database_url: &str,
    path: &str,
) -> Result<String, ImportError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| ImportError::Read(path.to_owned(), err.to_string()))?;
    let snapshot: snapshot::Snapshot = serde_json::from_str(&text)
        .map_err(|err| ImportError::Parse(path.to_owned(), err.to_string()))?;
    let summary = snapshot.summary();
    snapshot::import(db, database_url, &snapshot, &dev_password_hash()?).await?;
    Ok(summary)
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("reading {0}: {1}")]
    Read(String, String),
    #[error("parsing {0}: {1}")]
    Parse(String, String),
    #[error(transparent)]
    Snapshot(#[from] snapshot::SnapshotError),
    #[error("hashing the development password: {0}")]
    Password(#[from] architect_auth::crypto::PasswordError),
}

/// The hash a snapshot import should write onto every account.
///
/// # Errors
///
/// If the password hasher fails.
pub fn dev_password_hash() -> Result<String, architect_auth::crypto::PasswordError> {
    hash_password(DEV_PASSWORD)
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct SnapshotQuery {
    /// `?redact_emails=true` to rewrite addresses.
    #[serde(default)]
    pub redact_emails: bool,
}

/// `GET /admin/snapshot` — an already-sanitised copy of the database.
///
/// Admin session required, as `Authorization: Bearer` or the session
/// cookie. Answers 403 to everybody else, including a signed-in
/// non-admin: this is the whole organization graph, and "who is in
/// which company" is not public.
pub async fn snapshot_route<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Query(q): Query<SnapshotQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) =
        architect_auth::transport::axum::session_token_from_headers(&headers, &state.cookie)
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if state
        .auth
        .authorize_admin(AuthorizeAdmin {
            session_token: token,
        })
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(db) = state.db.as_ref() else {
        // A storage backend that is not SeaORM cannot be snapshotted by
        // this route, and saying so beats a confusing empty file.
        return (
            StatusCode::NOT_IMPLEMENTED,
            "snapshots require the SeaORM storage backend",
        )
            .into_response();
    };
    match snapshot::export(
        db,
        ExportOptions {
            redact_emails: q.redact_emails,
        },
    )
    .await
    {
        Ok(snapshot) => {
            tracing::info!(
                target: "auth_server::snapshot",
                summary = %snapshot.summary(),
                "snapshot exported"
            );
            Json(snapshot).into_response()
        }
        Err(err) => {
            tracing::error!(target: "auth_server::snapshot", %err, "snapshot failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "snapshot failed").into_response()
        }
    }
}

/// The connection a snapshot reads from, when there is one.
pub type SnapshotDb = Option<DatabaseConnection>;

#[cfg(test)]
mod tests {
    use super::{DEV_ORGS, DEV_PEOPLE, dev_id};

    #[test]
    fn seeded_ids_are_the_same_on_every_machine() {
        // The property a browser test depends on.
        assert_eq!(dev_id("acme"), dev_id("acme"));
        assert_ne!(dev_id("acme"), dev_id("indie"));
    }

    #[test]
    fn every_seeded_organization_names_people_who_exist() {
        for (slug, _, owner, guest, _) in DEV_ORGS {
            for email in [owner, guest] {
                assert!(
                    DEV_PEOPLE
                        .iter()
                        .any(|(candidate, _, _)| *candidate == email),
                    "{slug} names {email}, who is not in DEV_PEOPLE"
                );
            }
        }
    }

    #[test]
    fn exactly_one_seeded_person_is_an_administrator() {
        // More than one is fine in principle; zero means the snapshot
        // route cannot be reached on a fresh server, which would make
        // it untestable.
        let admins = DEV_PEOPLE
            .iter()
            .filter(|(_, _, role)| *role == "admin")
            .count();
        assert_eq!(admins, 1);
    }
}
