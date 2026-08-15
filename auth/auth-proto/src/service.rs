//! The mountable RPC surface for architect-auth sessions.
//!
//! One `#[architect::rpc]` trait covering the session lifecycle:
//! sign-up, password sign-in, validate (`current_session`), refresh,
//! `whoami`, and sign-out. The trait is all-async, so `vox::service`
//! applies to it directly — the macro additionally emits the `serve` /
//! `layer` verbs, the deferred-bind `Service` token, and a glob-safe
//! [`prelude`] so the service mounts like any other architect service:
//!
//! ```ignore
//! use architect_auth::{AuthVoxService, auth_service_layer};
//!
//! let router = architect::LayerRouter::new()
//!     .merge(auth_service_layer(AuthVoxService::new(auth)));
//! ```
//!
//! Engine-only capabilities (TOTP, API keys, invitations, …) stay off
//! this surface on purpose: they remain reachable through
//! `ArchitectAuth` directly, and adding them later is purely additive —
//! new trait methods, no re-mount.

use crate::email_change::AuthEmailChange;
use crate::{AuthFlowError, AuthSessionBundle, AuthUser, SignInEmailPassword, SignUpEmailPassword};
use uuid::Uuid;

/// Flattened membership row for the org-members enumeration RPC: the
/// member's `role` joined with the user's display `name` + `email`, so
/// clients (rates editor, owner dashboard) get a ready-to-render list
/// without a second per-user round-trip.
#[derive(Clone, Debug, PartialEq, ::facet::Facet)]
pub struct OrgMember {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
}

/// Metadata key carrying the session token on vox calls.
///
/// Clients attach `Bearer <token>` under this key (see
/// `AuthClientMiddleware` / `auth-client`'s `TokenStoreMiddleware`);
/// `AuthServerMiddleware` parses it back out on the server side.
pub const AUTHORIZATION_METADATA_KEY: &str = "authorization";

/// Wire form of a self-service email change.
#[derive(Clone, Debug, PartialEq, Eq, ::facet::Facet)]
pub struct ChangeEmailRequest {
    /// Identifies the account; the change always applies to the
    /// session's own user.
    pub session_token: String,
    pub new_email: String,
}

/// Wire form of a self-service password change.
#[derive(Clone, Debug, PartialEq, Eq, ::facet::Facet)]
pub struct ChangePasswordRequest {
    /// Identifies the account. The change always applies to the session's
    /// own user — there is no target parameter, by design.
    pub session_token: String,
    /// Proof of possession. Required even with a valid session, so a
    /// stolen token alone cannot take an account over.
    pub current_password: String,
    pub new_password: String,
}

/// Wire form of an operator-performed email migration.
#[derive(Clone, Debug, PartialEq, Eq, ::facet::Facet)]
pub struct MigrateUserEmailRequest {
    /// Authorizes the call AND identifies who to record as `changed_by`.
    pub session_token: String,
    /// The account being moved. Not the caller's own id — an operator
    /// migrates someone else.
    pub user_id: uuid::Uuid,
    pub new_email: String,
    /// Free text for the trail; worth filling in on bulk migrations.
    pub reason: Option<String>,
}

/// Wire form of a history read.
#[derive(Clone, Debug, PartialEq, Eq, ::facet::Facet)]
pub struct EmailHistoryRequest {
    pub session_token: String,
    pub user_id: uuid::Uuid,
}

// r[impl auth.transport.vox-schema]
#[architect::rpc]
pub trait AuthService {
    /// Create an email/password user and sign them in. Returns the
    /// freshly issued session bundle (the raw token is only returned
    /// here — only its hash is stored).
    async fn sign_up_email_password(
        &self,
        input: SignUpEmailPassword,
    ) -> Result<AuthSessionBundle, AuthFlowError>;

    /// Password sign-in for an existing user.
    async fn sign_in_email_password(
        &self,
        input: SignInEmailPassword,
    ) -> Result<AuthSessionBundle, AuthFlowError>;

    /// Validate a session token, returning the matching user + session.
    async fn current_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError>;

    /// Rotate a valid session: issue a fresh token with a new expiry
    /// and deactivate the old one.
    async fn refresh_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError>;

    /// Resolve a session token to its user — `current_session` minus
    /// the session details.
    async fn whoami(&self, token: String) -> Result<AuthUser, AuthFlowError>;

    /// Revoke the session behind a token. Idempotent: unknown or
    /// already-revoked tokens succeed without revealing existence.
    async fn sign_out(&self, token: String) -> Result<(), AuthFlowError>;

    /// List the members of the caller's active organization.
    ///
    /// No `org_id` parameter: this service is mounted per-org (the
    /// `/org/<slug>/vox` route binds an org-scoped `ArchitectAuth`
    /// backed by that org's own auth store), and the target org is
    /// derived from the caller's session (`active_organization_id`).
    /// Each row carries `role` from the membership plus the user's
    /// display `name` + `email`. If the org has no membership rows yet,
    /// implementations fall back to enumerating the org store's users
    /// with role `"member"` so the list is never spuriously empty.
    async fn list_org_members(&self, token: String) -> Result<Vec<OrgMember>, AuthFlowError>;

    /// Move an account onto a different email, keeping its user id, and
    /// append to its history trail (`auth_proto::email_change`).
    ///
    /// Takes the TARGET's `user_id` rather than deriving it from the
    /// session, because the common case is an operator migrating someone
    /// who can no longer sign in with the old address. The session is the
    /// AUTHORIZATION — the caller must hold a valid one for this org —
    /// and is recorded as `changed_by`, so the trail says who did it.
    async fn migrate_user_email(
        &self,
        input: MigrateUserEmailRequest,
    ) -> Result<AuthUser, AuthFlowError>;

    /// Every address an account has held, oldest first. Same
    /// authorization as the migration itself.
    async fn list_email_history(
        &self,
        input: EmailHistoryRequest,
    ) -> Result<Vec<AuthEmailChange>, AuthFlowError>;

    /// Change your OWN password.
    ///
    /// Self-service by construction: the session names the account, and
    /// the current password must be supplied — so holding a stolen
    /// session is not enough to lock the owner out, and knowing the
    /// password is not enough without a session. The flow also enforces
    /// strength and rejects known-breached passwords.
    ///
    /// Distinct from an operator reset, which needs neither and is
    /// therefore not exposed here at all.
    async fn change_password(&self, input: ChangePasswordRequest) -> Result<(), AuthFlowError>;

    /// Change your OWN email.
    ///
    /// Self-service counterpart to the operator migration: the session
    /// names the account, so there is no target parameter and no way to
    /// move someone else's address. Appends to the same history trail,
    /// recorded with no `changed_by` because the owner did it.
    ///
    /// The new address starts unverified — it has not been proven to
    /// belong to anyone yet.
    async fn change_email(&self, input: ChangeEmailRequest) -> Result<AuthUser, AuthFlowError>;

    /// Set your OWN display profile — the name and avatar other people
    /// see. Self-service like the two above: the session names the
    /// account, so there is no target parameter.
    ///
    /// `None` leaves a field alone; `Some("")` clears it. That
    /// distinction matters for a federated fan-out, where "the home
    /// didn't mention the avatar" and "the home cleared the avatar"
    /// must not collapse into the same write.
    ///
    /// Username is deliberately NOT here — it is an identifier with a
    /// uniqueness constraint, not display text, and it has its own
    /// flow.
    async fn update_profile(&self, input: UpdateProfileRequest) -> Result<AuthUser, AuthFlowError>;
}

/// Wire form of a self-service profile change.
#[derive(Clone, Debug, PartialEq, Eq, ::facet::Facet)]
#[repr(C)]
pub struct UpdateProfileRequest {
    /// Identifies the account; the change always applies to the
    /// session's own user.
    pub session_token: String,
    /// Display name. `None` = leave unchanged, `Some("")` = clear.
    pub name: Option<String>,
    /// Avatar URL. `None` = leave unchanged, `Some("")` = clear.
    pub image: Option<String>,
}
