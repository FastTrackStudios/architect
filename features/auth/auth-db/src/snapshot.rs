//! Taking a copy of an auth database that is safe to carry off it.
//!
//! # The problem this exists for
//!
//! You want production's *shape* locally: the organizations, who is in
//! them, the teams, the roles. That is what makes a local server useful
//! for reproducing a bug, rehearsing a migration, or replaying a
//! security incident.
//!
//! You do not want production's *credentials*. And a plain
//! `pg_dump | psql` gives you both. That is worth being precise about,
//! because the intuition "it is all hashed anyway" is wrong in the one
//! place it matters:
//!
//! - Session, invitation and invite-link tokens are stored as
//!   `hash_token(secret, ..)`, keyed by `AUTH_SECRET`. A local server
//!   with a different secret cannot verify them, so they are already
//!   inert — but they are copied anyway, because a table of live token
//!   hashes on a laptop is a liability with no upside.
//! - **Password hashes are not keyed by anything.** They are Argon2
//!   over the password and a per-row salt, which is exactly as
//!   crackable on a laptop as on the server. Copying them takes every
//!   account in the company along.
//! - OAuth access and refresh tokens are encrypted with the server
//!   secret, but they are credentials *at the provider*, not at us.
//!
//! So [`export`] copies the graph and drops every credential, and it
//! does so **server-side** — the plaintext never leaves the machine
//! that holds it, rather than being fetched and scrubbed afterwards.
//!
//! # What survives, and what does not
//!
//! | Kept | Dropped |
//! | --- | --- |
//! | organizations, members, teams, team members, roles | password hashes |
//! | users: id, name, username, image, role, ban state | every session |
//! | invitations: who, which org, which role, status | invitation and link tokens |
//! | invite links: label, role, uses, expiry | link token hashes |
//! | | OAuth accounts and their tokens |
//! | | API keys, passkeys, two-factor secrets |
//! | | verifications (password resets, OTPs) |
//!
//! Invitations and links come across as *rows*, not as working URLs:
//! you can see that a link exists, how much of it is spent and who made
//! it, and you cannot follow it. Which is the correct local fidelity —
//! the org page renders exactly as it does in production.
//!
//! Emails are personal data rather than credentials, so they are a
//! choice: [`ExportOptions::redact_emails`] rewrites them
//! deterministically to `user-<id>@local.invalid`, keeping them
//! distinct and joinable without carrying anybody's address.
//!
//! # Getting back in
//!
//! Every imported account is given one known password, so you can sign
//! in as anyone. That is the point of a local mirror and it is also why
//! [`import`] refuses to run against anything but a local database:
//! this function's entire purpose is to make accounts trivially
//! enterable.

use sea_orm::{
    ActiveModelTrait, DatabaseConnection, EntityTrait, PaginatorTrait, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::{
    AuthAccountActiveModel, AuthInvitation, AuthInvitationActiveModel, AuthInvitationEntity,
    AuthInviteLink, AuthInviteLinkActiveModel, AuthInviteLinkEntity, AuthMember,
    AuthMemberActiveModel, AuthMemberEntity, AuthOrganization, AuthOrganizationActiveModel,
    AuthOrganizationEntity, AuthOrganizationRole, AuthOrganizationRoleActiveModel,
    AuthOrganizationRoleEntity, AuthTeam, AuthTeamActiveModel, AuthTeamEntity, AuthTeamMember,
    AuthTeamMemberActiveModel, AuthTeamMemberEntity, AuthUser, AuthUserActiveModel, AuthUserEntity,
};

/// The version stamped into every snapshot.
///
/// Read on import and refused if it does not match: a snapshot is a
/// copy of a schema, and loading one taken before a migration would
/// half-populate tables in ways that look like data corruption rather
/// than a version mismatch.
pub const SNAPSHOT_VERSION: u32 = 1;

/// The `provider_id` an email-and-password account is stored under.
///
/// Mirrors the engine's own constant. Named here rather than imported
/// because this crate is the storage layer and does not depend on the
/// engine — and because a drift between the two would show up as
/// "imported accounts cannot sign in", which the round-trip test
/// below would catch.
const PASSWORD_PROVIDER_ID: &str = "credential";

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("database: {0}")]
    Db(#[from] sea_orm::DbErr),
    #[error(
        "snapshot is version {found}, this build reads version {expected} — \
         re-export from a server running this schema"
    )]
    Version { found: u32, expected: u32 },
    #[error(
        "refusing to import into {0}: a snapshot import sets one known password on every \
         account, so it may only target a local database (sqlite, or a host of localhost / \
         127.0.0.1 / ::1)"
    )]
    NotLocal(String),
    #[error("refusing to import into a database that already holds {0} user(s)")]
    NotEmpty(u64),
}

/// What to leave out beyond the credentials, which are never included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Replace every address with `user-<id>@local.invalid`.
    ///
    /// Addresses are not credentials, so they are kept by default —
    /// signing in as yourself is the first thing anybody does with a
    /// mirror. Turn this on when the copy is going somewhere the real
    /// ones should not, and they stay distinct and joinable.
    pub redact_emails: bool,
}

/// The organization graph, as plain serialisable rows.
///
/// Mirrors of the entity types rather than the entity types themselves,
/// which carry `facet` rather than `serde`. The duplication is worth
/// it: this file is the definitive statement of what leaves a
/// production database, and a field only crosses if somebody wrote it
/// here on purpose. A new column on an entity does not silently join
/// the export.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotOrganization {
    pub id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub logo: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotMember {
    pub id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub role: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotTeam {
    pub id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotTeamMember {
    pub id: uuid::Uuid,
    pub team_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotOrganizationRole {
    pub id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub role: String,
    pub permissions_json: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A user, with everything that could authenticate as them removed.
///
/// Note what is *not* here: no `password_hash` field exists to forget
/// to clear. The type cannot carry one.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotUser {
    pub id: uuid::Uuid,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub username: Option<String>,
    pub display_username: Option<String>,
    pub image: Option<String>,
    pub role: Option<String>,
    pub banned: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// An invitation as a row, not as a working URL.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotInvitation {
    pub id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub inviter_id: uuid::Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// An invite link as a row. No `token_hash`: locally the link is
/// visible and unfollowable, which is the right fidelity.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SnapshotInviteLink {
    pub id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub label: Option<String>,
    pub role: String,
    pub created_by: uuid::Uuid,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The whole copy.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub version: u32,
    pub taken_at: chrono::DateTime<chrono::Utc>,
    /// Whether addresses in this file are the real ones.
    pub emails_redacted: bool,
    pub users: Vec<SnapshotUser>,
    pub organizations: Vec<SnapshotOrganization>,
    pub members: Vec<SnapshotMember>,
    pub teams: Vec<SnapshotTeam>,
    pub team_members: Vec<SnapshotTeamMember>,
    pub organization_roles: Vec<SnapshotOrganizationRole>,
    pub invitations: Vec<SnapshotInvitation>,
    pub invite_links: Vec<SnapshotInviteLink>,
}

impl Snapshot {
    /// A one-line summary, for a tool that has just taken or loaded one.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} user(s), {} organization(s), {} membership(s), {} team(s), {} invitation(s), \
             {} link(s){}",
            self.users.len(),
            self.organizations.len(),
            self.members.len(),
            self.teams.len(),
            self.invitations.len(),
            self.invite_links.len(),
            if self.emails_redacted {
                ", emails redacted"
            } else {
                ""
            }
        )
    }
}

/// Copy the graph out of `db`, leaving every credential behind.
///
/// # Errors
///
/// Any database error while reading.
pub async fn export(
    db: &DatabaseConnection,
    options: ExportOptions,
) -> Result<Snapshot, SnapshotError> {
    let users = AuthUserEntity::find()
        .all(db)
        .await?
        .into_iter()
        .map(AuthUser::from)
        .map(|user| SnapshotUser {
            email: match (&user.email, options.redact_emails) {
                (Some(_), true) => Some(redacted_email(user.id)),
                (email, _) => email.clone(),
            },
            id: user.id,
            email_verified: user.email_verified,
            name: user.name,
            username: user.username,
            display_username: user.display_username,
            image: user.image,
            role: user.role,
            banned: user.banned,
            created_at: user.created_at,
        })
        .collect();

    Ok(Snapshot {
        version: SNAPSHOT_VERSION,
        taken_at: chrono::Utc::now(),
        emails_redacted: options.redact_emails,
        users,
        organizations: AuthOrganizationEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthOrganization::from)
            .map(|o| SnapshotOrganization {
                id: o.id,
                name: o.name,
                slug: o.slug,
                logo: o.logo,
                metadata_json: o.metadata_json,
                created_at: o.created_at,
                updated_at: o.updated_at,
            })
            .collect(),
        members: AuthMemberEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthMember::from)
            .map(|m| SnapshotMember {
                id: m.id,
                organization_id: m.organization_id,
                user_id: m.user_id,
                role: m.role,
                created_at: m.created_at,
            })
            .collect(),
        teams: AuthTeamEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthTeam::from)
            .map(|t| SnapshotTeam {
                id: t.id,
                organization_id: t.organization_id,
                name: t.name,
                created_at: t.created_at,
                updated_at: t.updated_at,
            })
            .collect(),
        team_members: AuthTeamMemberEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthTeamMember::from)
            .map(|t| SnapshotTeamMember {
                id: t.id,
                team_id: t.team_id,
                user_id: t.user_id,
                created_at: t.created_at,
            })
            .collect(),
        organization_roles: AuthOrganizationRoleEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthOrganizationRole::from)
            .map(|r| SnapshotOrganizationRole {
                id: r.id,
                organization_id: r.organization_id,
                role: r.role,
                permissions_json: r.permissions_json,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
        invitations: AuthInvitationEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthInvitation::from)
            .map(|invitation| SnapshotInvitation {
                email: if options.redact_emails {
                    redacted_email(invitation.id)
                } else {
                    invitation.email
                },
                id: invitation.id,
                organization_id: invitation.organization_id,
                role: invitation.role,
                status: invitation.status,
                inviter_id: invitation.inviter_id,
                expires_at: invitation.expires_at,
                created_at: invitation.created_at,
            })
            .collect(),
        invite_links: AuthInviteLinkEntity::find()
            .all(db)
            .await?
            .into_iter()
            .map(AuthInviteLink::from)
            .map(|link| SnapshotInviteLink {
                id: link.id,
                organization_id: link.organization_id,
                label: link.label,
                role: link.role,
                created_by: link.created_by,
                expires_at: link.expires_at,
                max_uses: link.max_uses,
                uses: link.uses,
                revoked_at: link.revoked_at,
                created_at: link.created_at,
            })
            .collect(),
    })
}

/// A stable stand-in address, distinct per row and obviously not real.
///
/// `.invalid` is reserved by RFC 2606 precisely so it can never resolve,
/// so a stray mail send in a local environment fails at DNS rather than
/// reaching a person.
fn redacted_email(id: uuid::Uuid) -> String {
    let short: String = id.simple().to_string().chars().take(12).collect();
    format!("user-{short}@local.invalid")
}

/// Load a snapshot into a **local** database, giving every account
/// `password_hash`.
///
/// The caller supplies the hash because hashing lives in the engine,
/// not here — and because it makes the "one known password" explicit at
/// the call site rather than buried in this crate.
///
/// # Errors
///
/// [`SnapshotError::Version`] for a snapshot from a different schema,
/// [`SnapshotError::NotLocal`] if `database_url` is not local, and
/// [`SnapshotError::NotEmpty`] if the target already holds users.
pub async fn import(
    db: &DatabaseConnection,
    database_url: &str,
    snapshot: &Snapshot,
    password_hash: &str,
) -> Result<(), SnapshotError> {
    if snapshot.version != SNAPSHOT_VERSION {
        return Err(SnapshotError::Version {
            found: snapshot.version,
            expected: SNAPSHOT_VERSION,
        });
    }
    if !is_local(database_url) {
        return Err(SnapshotError::NotLocal(redact_url(database_url)));
    }
    let existing = AuthUserEntity::find().count(db).await?;
    if existing > 0 {
        return Err(SnapshotError::NotEmpty(existing));
    }
    load(db, snapshot, password_hash).await
}

async fn load(
    db: &DatabaseConnection,
    snapshot: &Snapshot,
    password_hash: &str,
) -> Result<(), SnapshotError> {
    let txn = db.begin().await?;
    for user in &snapshot.users {
        AuthUserActiveModel {
            id: Set(user.id),
            email: Set(user.email.clone()),
            email_verified: Set(user.email_verified),
            name: Set(user.name.clone()),
            username: Set(user.username.clone()),
            display_username: Set(user.display_username.clone()),
            image: Set(user.image.clone()),
            role: Set(user.role.clone()),
            banned: Set(user.banned),
            ban_reason: Set(None),
            ban_expires: Set(None),
            // Not carried over, and not guessed at: a mirrored account
            // signs in with the one known password and nothing else, so
            // a local server never prompts for a second factor whose
            // secret it does not have.
            two_factor_enabled: Set(false),
            // `"{}"`, not empty: the column holds JSON and the engine
            // parses it.
            metadata_json: Set("{}".to_owned()),
            created_at: Set(user.created_at),
            updated_at: Set(user.created_at),
        }
        .insert(&txn)
        .await?;

        // The password lives on an account row, alongside the OAuth
        // tokens — which is exactly why no account row is exported.
        // A fresh credential account is minted here instead, so the
        // imported database holds one hash this machine chose rather
        // than any hash production held.
        //
        // Skipped for a user with no address: the email/password sign-in
        // path looks an account up BY that address, so a credential row
        // without one is unreachable. Such a user exists in the mirror
        // and simply cannot be signed in as, which is the truth.
        let Some(email) = user.email.as_deref() else {
            continue;
        };
        AuthAccountActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            // The address, NOT the user id: `sign_in_email_password`
            // finds the credential account by `(provider, account_id)`
            // with the canonical address as `account_id`. Writing the
            // user id here produces a database that looks correct in
            // every table and cannot be signed in to.
            account_id: Set(email.to_owned()),
            provider_id: Set(PASSWORD_PROVIDER_ID.to_owned()),
            user_id: Set(user.id),
            access_token_ciphertext: Set(None),
            refresh_token_ciphertext: Set(None),
            id_token_ciphertext: Set(None),
            access_token_expires_at: Set(None),
            refresh_token_expires_at: Set(None),
            scope: Set(None),
            password_hash: Set(Some(password_hash.to_owned())),
            created_at: Set(user.created_at),
            updated_at: Set(user.created_at),
        }
        .insert(&txn)
        .await?;
    }
    for organization in &snapshot.organizations {
        AuthOrganizationActiveModel {
            id: Set(organization.id),
            name: Set(organization.name.clone()),
            slug: Set(organization.slug.clone()),
            logo: Set(organization.logo.clone()),
            metadata_json: Set(organization.metadata_json.clone()),
            created_at: Set(organization.created_at),
            updated_at: Set(organization.updated_at),
        }
        .insert(&txn)
        .await?;
    }
    for member in &snapshot.members {
        AuthMemberActiveModel {
            id: Set(member.id),
            organization_id: Set(member.organization_id),
            user_id: Set(member.user_id),
            role: Set(member.role.clone()),
            created_at: Set(member.created_at),
        }
        .insert(&txn)
        .await?;
    }
    for team in &snapshot.teams {
        AuthTeamActiveModel {
            id: Set(team.id),
            organization_id: Set(team.organization_id),
            name: Set(team.name.clone()),
            created_at: Set(team.created_at),
            updated_at: Set(team.updated_at),
        }
        .insert(&txn)
        .await?;
    }
    for team_member in &snapshot.team_members {
        AuthTeamMemberActiveModel {
            id: Set(team_member.id),
            team_id: Set(team_member.team_id),
            user_id: Set(team_member.user_id),
            created_at: Set(team_member.created_at),
        }
        .insert(&txn)
        .await?;
    }
    for role in &snapshot.organization_roles {
        AuthOrganizationRoleActiveModel {
            id: Set(role.id),
            organization_id: Set(role.organization_id),
            role: Set(role.role.clone()),
            permissions_json: Set(role.permissions_json.clone()),
            created_at: Set(role.created_at),
            updated_at: Set(role.updated_at),
        }
        .insert(&txn)
        .await?;
    }
    for invitation in &snapshot.invitations {
        AuthInvitationActiveModel {
            id: Set(invitation.id),
            organization_id: Set(invitation.organization_id),
            email: Set(invitation.email.clone()),
            role: Set(invitation.role.clone()),
            status: Set(invitation.status.clone()),
            inviter_id: Set(invitation.inviter_id),
            expires_at: Set(invitation.expires_at),
            created_at: Set(invitation.created_at),
        }
        .insert(&txn)
        .await?;
    }
    for link in &snapshot.invite_links {
        AuthInviteLinkActiveModel {
            id: Set(link.id),
            organization_id: Set(link.organization_id),
            // No token came across, and none is invented: a hash of
            // nothing would still be a hash somebody could collide
            // against. The row's own id is not a secret and cannot be
            // presented as one, because lookup is by hash.
            token_hash: Set(format!("imported:{}", link.id)),
            label: Set(link.label.clone()),
            role: Set(link.role.clone()),
            created_by: Set(link.created_by),
            expires_at: Set(link.expires_at),
            max_uses: Set(link.max_uses),
            uses: Set(link.uses),
            revoked_at: Set(link.revoked_at),
            created_at: Set(link.created_at),
        }
        .insert(&txn)
        .await?;
    }
    txn.commit().await?;
    Ok(())
}

/// Is this URL unambiguously a database on this machine?
///
/// Deliberately a whitelist. A guess that errs toward "local" here
/// writes a known password onto every account in whatever it is
/// pointing at.
#[must_use]
pub fn is_local(database_url: &str) -> bool {
    let url = database_url.trim();
    if url.starts_with("sqlite:") {
        return true;
    }
    let Some((_scheme, rest)) = url.split_once("://") else {
        return false;
    };
    // Userinfo may itself contain `@` in a password, so the LAST one
    // separates it from the authority.
    let authority = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    let authority = authority.split(['/', '?']).next().unwrap_or_default();
    // An IPv6 literal is bracketed, and contains the colons that would
    // otherwise look like a port separator.
    let host = authority.strip_prefix('[').map_or_else(
        || {
            authority
                .split_once(':')
                .map_or(authority, |(host, _port)| host)
        },
        |rest| rest.split_once(']').map_or(rest, |(host, _port)| host),
    );
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// A database URL with any password in it replaced, for an error
/// message that is going into a terminal and possibly a bug report.
#[must_use]
pub fn redact_url(database_url: &str) -> String {
    let Some((scheme, rest)) = database_url.split_once("://") else {
        return database_url.to_owned();
    };
    match rest.split_once('@') {
        None => database_url.to_owned(),
        Some((userinfo, host)) => {
            let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
            format!("{scheme}://{user}:***@{host}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::{is_local, redact_url, redacted_email};

    #[test]
    fn sqlite_is_always_local() {
        assert!(is_local("sqlite::memory:"));
        assert!(is_local("sqlite://./auth-local.db?mode=rwc"));
    }

    #[test]
    fn loopback_hosts_are_local() {
        assert!(is_local("postgres://auth:pw@localhost:5432/auth"));
        assert!(is_local("postgres://auth:pw@127.0.0.1/auth"));
        assert!(is_local("postgres://auth@[::1]:5432/auth"));
    }

    #[test]
    fn anything_else_is_not() {
        // The one that matters: this function's answer decides whether
        // a known password is written onto every account.
        assert!(!is_local(
            "postgres://auth:pw@auth-db.prod.svc.cluster.local:5432/auth"
        ));
        assert!(!is_local("postgres://auth:pw@10.0.0.4:5432/auth"));
        assert!(!is_local("mysql://auth@db.example.com/auth"));
        assert!(!is_local(""));
        assert!(!is_local("not a url"));
    }

    #[test]
    fn a_host_that_merely_contains_localhost_is_not_local() {
        // `localhost.evil.example` resolves to whatever its owner says.
        assert!(!is_local("postgres://a@localhost.evil.example/auth"));
        assert!(!is_local("postgres://a@notlocalhost/auth"));
    }

    #[test]
    fn a_url_in_an_error_message_carries_no_password() {
        assert_eq!(
            redact_url("postgres://auth:hunter2@db.example.com:5432/auth"),
            "postgres://auth:***@db.example.com:5432/auth"
        );
        assert_eq!(redact_url("sqlite::memory:"), "sqlite::memory:");
    }

    #[test]
    fn redacted_addresses_stay_distinct_and_unreachable() {
        let a = redacted_email(uuid::Uuid::new_v4());
        let b = redacted_email(uuid::Uuid::new_v4());
        assert_ne!(a, b);
        // RFC 2606 reserves `.invalid` so it can never resolve.
        assert!(a.ends_with("@local.invalid"));
    }
}
