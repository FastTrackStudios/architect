//! Runtime storage boundary for auth flows.

use async_trait::async_trait;
use auth_proto::{
    AuthAccount, AuthAccountCreate, AuthApiKey, AuthApiKeyCreate, AuthFlowError, AuthInvitation,
    email_change::AuthEmailChange,
    AuthInvitationCreate, AuthMember, AuthMemberCreate, AuthOrganization, AuthOrganizationCreate,
    AuthOrganizationRole, AuthOrganizationRoleCreate, AuthPasskey, AuthPasskeyCreate, AuthSession,
    AuthSessionCreate, AuthTeam, AuthTeamCreate, AuthTeamMember, AuthTeamMemberCreate,
    AuthTwoFactor, AuthTwoFactorCreate, AuthUser, AuthUserCreate, AuthVerification,
    AuthVerificationCreate,
};
use chrono::{DateTime, Utc};

// r[impl auth.storage.clock]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStorageClock {
    RuntimeUtc,
    BackendGeneratedUtc,
}

// r[impl auth.storage.backend-parity]
// r[impl auth.storage.transactions]
// r[impl auth.storage.clock]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthStorageCapabilities {
    pub backend: &'static str,
    pub transactions: bool,
    pub clock: AuthStorageClock,
}

impl AuthStorageCapabilities {
    pub const fn runtime_owned(backend: &'static str) -> Self {
        Self {
            backend,
            transactions: false,
            clock: AuthStorageClock::RuntimeUtc,
        }
    }

    pub const fn transactional(backend: &'static str, clock: AuthStorageClock) -> Self {
        Self {
            backend,
            transactions: true,
            clock,
        }
    }
}

#[async_trait]
pub trait AuthStorage: Clone + Send + Sync + 'static {
    // r[impl auth.storage.backend-parity]
    // r[impl auth.storage.transactions]
    // r[impl auth.storage.clock]
    fn capabilities(&self) -> AuthStorageCapabilities {
        AuthStorageCapabilities::runtime_owned("custom")
    }

    // r[impl auth.core.server-authoritative]
    async fn create_user(&self, input: AuthUserCreate) -> Result<AuthUser, AuthFlowError>;

    // r[impl auth.storage.transactions]
    async fn create_user_account_session(
        &self,
        user_input: AuthUserCreate,
        mut account_input: AuthAccountCreate,
        mut session_input: AuthSessionCreate,
    ) -> Result<(AuthUser, AuthSession), AuthFlowError> {
        let user = self.create_user(user_input).await?;
        account_input.user_id = user.id;
        session_input.user_id = user.id;
        self.create_account(account_input).await?;
        let session = self.create_session(session_input).await?;
        Ok((user, session))
    }

    async fn record_audit_event(&self, event: crate::AuthAuditEvent) -> Result<(), AuthFlowError>;

    async fn find_user_by_email(
        &self,
        canonical_email: &str,
    ) -> Result<Option<AuthUser>, AuthFlowError>;

    async fn find_user_by_username(
        &self,
        canonical_username: &str,
    ) -> Result<Option<AuthUser>, AuthFlowError>;

    async fn find_user_by_id(&self, user_id: uuid::Uuid)
    -> Result<Option<AuthUser>, AuthFlowError>;

    async fn list_users(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<AuthUser>, usize), AuthFlowError>;

    async fn update_user_role(
        &self,
        user_id: uuid::Uuid,
        role: Option<String>,
    ) -> Result<AuthUser, AuthFlowError>;

    async fn update_user_ban(
        &self,
        user_id: uuid::Uuid,
        banned: bool,
        ban_reason: Option<String>,
        ban_expires: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AuthUser, AuthFlowError>;

    async fn update_user_email(
        &self,
        user_id: uuid::Uuid,
        email: String,
        email_verified: bool,
    ) -> Result<AuthUser, AuthFlowError>;

    /// Append an entry to the account's email trail. Called by every path
    /// that changes an address, so the history is complete rather than
    /// best-effort — see `auth_proto::email_change`.
    async fn record_email_change(
        &self,
        user_id: uuid::Uuid,
        previous_email: Option<String>,
        new_email: String,
        changed_by: Option<uuid::Uuid>,
        reason: Option<String>,
    ) -> Result<AuthEmailChange, AuthFlowError>;

    /// Every address this account has held, oldest first.
    async fn list_email_history(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<AuthEmailChange>, AuthFlowError>;

    /// The account that once held `email`, if any — the reverse lookup
    /// ("who was old@example.com?"). Most recent change wins when an
    /// address has been used more than once.
    async fn find_user_id_by_previous_email(
        &self,
        email: &str,
    ) -> Result<Option<uuid::Uuid>, AuthFlowError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_user_profile(
        &self,
        user_id: uuid::Uuid,
        name: Option<String>,
        username: Option<String>,
        display_username: Option<String>,
        image: Option<String>,
        metadata_json: String,
    ) -> Result<AuthUser, AuthFlowError>;

    async fn create_account(&self, input: AuthAccountCreate) -> Result<AuthAccount, AuthFlowError>;

    async fn find_account_by_provider_account(
        &self,
        provider_id: &str,
        account_id: &str,
    ) -> Result<Option<AuthAccount>, AuthFlowError>;

    async fn find_password_account_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Option<AuthAccount>, AuthFlowError>;

    async fn list_accounts_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<AuthAccount>, AuthFlowError>;

    async fn delete_user_by_id(&self, user_id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn delete_account_by_provider_account(
        &self,
        provider_id: &str,
        account_id: &str,
    ) -> Result<(), AuthFlowError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_oauth_account_tokens(
        &self,
        provider_id: &str,
        account_id: &str,
        access_token_ciphertext: Option<String>,
        refresh_token_ciphertext: Option<String>,
        id_token_ciphertext: Option<String>,
        access_token_expires_at: Option<DateTime<Utc>>,
        refresh_token_expires_at: Option<DateTime<Utc>>,
        scope: Option<String>,
    ) -> Result<AuthAccount, AuthFlowError>;

    async fn update_password_hash(
        &self,
        user_id: uuid::Uuid,
        password_hash: String,
    ) -> Result<(), AuthFlowError>;

    async fn create_session(&self, input: AuthSessionCreate) -> Result<AuthSession, AuthFlowError>;

    async fn find_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<AuthSession>, AuthFlowError>;

    async fn list_sessions_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<AuthSession>, AuthFlowError>;

    async fn deactivate_session_by_token_hash(&self, token_hash: &str)
    -> Result<(), AuthFlowError>;

    async fn deactivate_session_by_id(&self, session_id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn activate_session_by_token_hash(&self, token_hash: &str) -> Result<(), AuthFlowError>;

    async fn deactivate_sessions_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<(), AuthFlowError>;

    async fn deactivate_other_sessions_by_user_id(
        &self,
        user_id: uuid::Uuid,
        except_session_id: uuid::Uuid,
    ) -> Result<(), AuthFlowError>;

    async fn create_verification(
        &self,
        input: AuthVerificationCreate,
    ) -> Result<AuthVerification, AuthFlowError>;

    async fn find_verification(
        &self,
        identifier: &str,
        value_hash: &str,
    ) -> Result<Option<AuthVerification>, AuthFlowError>;

    async fn find_latest_verification_by_identifier(
        &self,
        identifier: &str,
    ) -> Result<Option<AuthVerification>, AuthFlowError>;

    async fn delete_verification(&self, id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn mark_email_verified(&self, user_id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn create_api_key(&self, input: AuthApiKeyCreate) -> Result<AuthApiKey, AuthFlowError>;

    async fn find_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<AuthApiKey>, AuthFlowError>;

    async fn find_api_key_by_id(&self, id: uuid::Uuid)
    -> Result<Option<AuthApiKey>, AuthFlowError>;

    async fn list_api_keys_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<AuthApiKey>, AuthFlowError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_api_key(
        &self,
        id: uuid::Uuid,
        name: Option<String>,
        enabled: Option<bool>,
        expires_at: Option<DateTime<Utc>>,
        permissions_json: Option<String>,
        rate_limit_time_window: Option<i64>,
        rate_limit_max: Option<i64>,
        metadata_json: Option<String>,
    ) -> Result<AuthApiKey, AuthFlowError>;

    async fn delete_api_key(&self, id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn update_api_key_usage(
        &self,
        id: uuid::Uuid,
        request_count: Option<i64>,
        remaining: Option<i64>,
    ) -> Result<(), AuthFlowError>;

    async fn set_api_key_enabled(&self, id: uuid::Uuid, enabled: bool)
    -> Result<(), AuthFlowError>;

    async fn create_passkey(&self, input: AuthPasskeyCreate) -> Result<AuthPasskey, AuthFlowError>;

    async fn find_passkey_by_credential_id(
        &self,
        credential_id: &str,
    ) -> Result<Option<AuthPasskey>, AuthFlowError>;

    async fn list_passkeys_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<AuthPasskey>, AuthFlowError>;

    async fn update_passkey_counter(
        &self,
        credential_id: &str,
        counter: i64,
    ) -> Result<AuthPasskey, AuthFlowError>;

    async fn delete_passkey_by_credential_id(
        &self,
        credential_id: &str,
    ) -> Result<(), AuthFlowError>;

    async fn create_organization(
        &self,
        input: AuthOrganizationCreate,
    ) -> Result<AuthOrganization, AuthFlowError>;

    // r[impl auth.storage.transactions]
    async fn create_organization_with_owner(
        &self,
        organization_input: AuthOrganizationCreate,
        owner_user_id: uuid::Uuid,
    ) -> Result<(AuthOrganization, AuthMember), AuthFlowError> {
        let organization = self.create_organization(organization_input).await?;
        let member = self
            .create_member(AuthMemberCreate {
                organization_id: organization.id,
                user_id: owner_user_id,
                role: "owner".into(),
            })
            .await?;
        Ok((organization, member))
    }

    async fn find_organization_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<AuthOrganization>, AuthFlowError>;

    async fn create_member(&self, input: AuthMemberCreate) -> Result<AuthMember, AuthFlowError>;

    async fn find_member(
        &self,
        organization_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<Option<AuthMember>, AuthFlowError>;

    async fn list_members_by_organization(
        &self,
        organization_id: uuid::Uuid,
    ) -> Result<Vec<AuthMember>, AuthFlowError>;

    async fn update_member_role(
        &self,
        organization_id: uuid::Uuid,
        user_id: uuid::Uuid,
        role: String,
    ) -> Result<AuthMember, AuthFlowError>;

    async fn create_organization_role(
        &self,
        input: AuthOrganizationRoleCreate,
    ) -> Result<AuthOrganizationRole, AuthFlowError>;

    async fn find_organization_role(
        &self,
        organization_id: uuid::Uuid,
        role: &str,
    ) -> Result<Option<AuthOrganizationRole>, AuthFlowError>;

    async fn list_organization_roles(
        &self,
        organization_id: uuid::Uuid,
    ) -> Result<Vec<AuthOrganizationRole>, AuthFlowError>;

    async fn update_organization_role(
        &self,
        organization_id: uuid::Uuid,
        role: &str,
        permissions_json: String,
    ) -> Result<AuthOrganizationRole, AuthFlowError>;

    async fn delete_organization_role(
        &self,
        organization_id: uuid::Uuid,
        role: &str,
    ) -> Result<(), AuthFlowError>;

    async fn create_team(&self, input: AuthTeamCreate) -> Result<AuthTeam, AuthFlowError>;

    async fn find_team_by_id(&self, id: uuid::Uuid) -> Result<Option<AuthTeam>, AuthFlowError>;

    async fn list_teams_by_organization(
        &self,
        organization_id: uuid::Uuid,
    ) -> Result<Vec<AuthTeam>, AuthFlowError>;

    async fn update_team_name(
        &self,
        id: uuid::Uuid,
        name: String,
    ) -> Result<AuthTeam, AuthFlowError>;

    async fn delete_team(&self, id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn create_team_member(
        &self,
        input: AuthTeamMemberCreate,
    ) -> Result<AuthTeamMember, AuthFlowError>;

    async fn find_team_member(
        &self,
        team_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<Option<AuthTeamMember>, AuthFlowError>;

    async fn list_team_members(
        &self,
        team_id: uuid::Uuid,
    ) -> Result<Vec<AuthTeamMember>, AuthFlowError>;

    async fn delete_team_member(
        &self,
        team_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), AuthFlowError>;

    async fn update_session_active_organization(
        &self,
        token_hash: &str,
        organization_id: Option<uuid::Uuid>,
    ) -> Result<(), AuthFlowError>;

    async fn create_invitation(
        &self,
        input: AuthInvitationCreate,
    ) -> Result<AuthInvitation, AuthFlowError>;

    async fn find_invitation_by_id(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<AuthInvitation>, AuthFlowError>;

    async fn update_invitation_status(
        &self,
        id: uuid::Uuid,
        status: String,
    ) -> Result<(), AuthFlowError>;

    // r[impl auth.storage.transactions]
    async fn accept_invitation_membership(
        &self,
        member_input: AuthMemberCreate,
        invitation_id: uuid::Uuid,
        accepted_status: String,
        verification_id: uuid::Uuid,
    ) -> Result<(), AuthFlowError> {
        self.create_member(member_input).await?;
        self.update_invitation_status(invitation_id, accepted_status)
            .await?;
        self.delete_verification(verification_id).await
    }

    async fn create_two_factor(
        &self,
        input: AuthTwoFactorCreate,
    ) -> Result<AuthTwoFactor, AuthFlowError>;

    async fn find_two_factor_by_user_id(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Option<AuthTwoFactor>, AuthFlowError>;

    async fn delete_two_factor(&self, user_id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn update_two_factor_backup_codes(
        &self,
        user_id: uuid::Uuid,
        backup_codes_hash: Option<String>,
    ) -> Result<(), AuthFlowError>;

    async fn increment_two_factor_attempts(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<i64, AuthFlowError>;

    async fn reset_two_factor_attempts(&self, user_id: uuid::Uuid) -> Result<(), AuthFlowError>;

    async fn set_user_two_factor_enabled(
        &self,
        user_id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), AuthFlowError>;
}

#[cfg(test)]
mod tests {
    use super::{AuthStorageCapabilities, AuthStorageClock};

    // r[verify auth.storage.backend-parity]
    // r[verify auth.storage.transactions]
    // r[verify auth.storage.clock]
    #[test]
    fn storage_capabilities_describe_backend_parity_contract() {
        let memory = AuthStorageCapabilities::runtime_owned("memory");
        assert_eq!(memory.backend, "memory");
        assert!(!memory.transactions);
        assert_eq!(memory.clock, AuthStorageClock::RuntimeUtc);

        let db = AuthStorageCapabilities::transactional(
            "sea-orm-sqlite",
            AuthStorageClock::BackendGeneratedUtc,
        );
        assert_eq!(db.backend, "sea-orm-sqlite");
        assert!(db.transactions);
        assert_eq!(db.clock, AuthStorageClock::BackendGeneratedUtc);
    }
}
