use async_trait::async_trait;
use auth_proto::{
    AuthAccount, AuthAccountCreate, AuthApiKey, AuthApiKeyCreate, AuthFlowError, AuthInvitation,
    AuthInvitationCreate, AuthMember, AuthMemberCreate, AuthOrganization, AuthOrganizationCreate,
    AuthOrganizationRole, AuthOrganizationRoleCreate, AuthPasskey, AuthPasskeyCreate, AuthSession,
    AuthSessionCreate, AuthTeam, AuthTeamCreate, AuthTeamMember, AuthTeamMemberCreate,
    AuthTwoFactor, AuthTwoFactorCreate, AuthUser, AuthUserCreate, AuthVerification,
    AuthVerificationCreate,
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionError, TransactionTrait,
};
use uuid::Uuid;

use crate::{
    AuthAuditEvent, AuthStorage, AuthStorageCapabilities, AuthStorageClock,
    backend_db::{
        AuthAccountActiveModel, AuthAccountColumn, AuthAccountEntity, AuthApiKeyActiveModel,
        AuthEmailChange, AuthEmailChangeActiveModel, AuthEmailChangeColumn, AuthEmailChangeEntity,
        AuthApiKeyColumn, AuthApiKeyEntity, AuthAuditEventRecordActiveModel,
        AuthInvitationActiveModel, AuthInvitationEntity, AuthMemberActiveModel, AuthMemberColumn,
        AuthMemberEntity, AuthOrganizationActiveModel, AuthOrganizationColumn,
        AuthOrganizationEntity, AuthOrganizationRoleActiveModel, AuthOrganizationRoleColumn,
        AuthOrganizationRoleEntity, AuthPasskeyActiveModel, AuthPasskeyColumn, AuthPasskeyEntity,
        AuthSessionActiveModel, AuthSessionColumn, AuthSessionEntity, AuthTeamActiveModel,
        AuthTeamColumn, AuthTeamEntity, AuthTeamMemberActiveModel, AuthTeamMemberColumn,
        AuthTeamMemberEntity, AuthTwoFactorActiveModel, AuthTwoFactorColumn, AuthTwoFactorEntity,
        AuthUserActiveModel, AuthUserColumn, AuthUserEntity, AuthVerificationActiveModel,
        AuthVerificationColumn, AuthVerificationEntity,
    },
};

#[derive(Clone)]
pub struct AuthSeaOrmStorage {
    db: DatabaseConnection,
}

impl AuthSeaOrmStorage {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }
}

fn map_db_err(err: sea_orm::DbErr) -> AuthFlowError {
    AuthFlowError::Internal(err.to_string())
}

fn map_txn_err(err: TransactionError<sea_orm::DbErr>) -> AuthFlowError {
    AuthFlowError::Internal(err.to_string())
}

// r[impl auth.storage.backend-parity]
// r[impl auth.storage.transactions]
// r[impl auth.storage.clock]
#[async_trait]
impl AuthStorage for AuthSeaOrmStorage {
    fn capabilities(&self) -> AuthStorageCapabilities {
        AuthStorageCapabilities::transactional("sea-orm", AuthStorageClock::BackendGeneratedUtc)
    }

    async fn create_user_account_session(
        &self,
        user_input: AuthUserCreate,
        account_input: AuthAccountCreate,
        session_input: AuthSessionCreate,
    ) -> Result<(AuthUser, AuthSession), AuthFlowError> {
        self.db
            .transaction::<_, (AuthUser, AuthSession), sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    let now = Utc::now();
                    let user = AuthUserActiveModel {
                        id: Set(Uuid::new_v4()),
                        email: Set(user_input.email),
                        name: Set(user_input.name),
                        email_verified: Set(user_input.email_verified),
                        image: Set(user_input.image),
                        username: Set(user_input.username),
                        display_username: Set(user_input.display_username),
                        two_factor_enabled: Set(user_input.two_factor_enabled),
                        role: Set(user_input.role),
                        banned: Set(user_input.banned),
                        ban_reason: Set(user_input.ban_reason),
                        ban_expires: Set(user_input.ban_expires),
                        metadata_json: Set(user_input.metadata_json),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    AuthAccountActiveModel {
                        id: Set(Uuid::new_v4()),
                        account_id: Set(account_input.account_id),
                        provider_id: Set(account_input.provider_id),
                        user_id: Set(user.id),
                        access_token_ciphertext: Set(account_input.access_token_ciphertext),
                        refresh_token_ciphertext: Set(account_input.refresh_token_ciphertext),
                        id_token_ciphertext: Set(account_input.id_token_ciphertext),
                        access_token_expires_at: Set(account_input.access_token_expires_at),
                        refresh_token_expires_at: Set(account_input.refresh_token_expires_at),
                        scope: Set(account_input.scope),
                        password_hash: Set(account_input.password_hash),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    let session = AuthSessionActiveModel {
                        id: Set(Uuid::new_v4()),
                        user_id: Set(user.id),
                        token_hash: Set(session_input.token_hash),
                        expires_at: Set(session_input.expires_at),
                        ip_address: Set(session_input.ip_address),
                        user_agent: Set(session_input.user_agent),
                        active_organization_id: Set(session_input.active_organization_id),
                        impersonated_by: Set(session_input.impersonated_by),
                        active: Set(session_input.active),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    Ok((AuthUser::from(user), AuthSession::from(session)))
                })
            })
            .await
            .map_err(map_txn_err)
    }

    async fn create_user(&self, input: AuthUserCreate) -> Result<AuthUser, AuthFlowError> {
        let now = Utc::now();
        AuthUserActiveModel {
            id: Set(Uuid::new_v4()),
            email: Set(input.email),
            name: Set(input.name),
            email_verified: Set(input.email_verified),
            image: Set(input.image),
            username: Set(input.username),
            display_username: Set(input.display_username),
            two_factor_enabled: Set(input.two_factor_enabled),
            role: Set(input.role),
            banned: Set(input.banned),
            ban_reason: Set(input.ban_reason),
            ban_expires: Set(input.ban_expires),
            metadata_json: Set(input.metadata_json),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthUser::from)
        .map_err(map_db_err)
    }

    async fn record_audit_event(&self, event: AuthAuditEvent) -> Result<(), AuthFlowError> {
        AuthAuditEventRecordActiveModel {
            id: Set(Uuid::new_v4()),
            actor_id: Set(event.actor_id),
            target_id: Set(event.target_id),
            action: Set(event.action),
            created_at: Set(event.created_at),
        }
        .insert(&self.db)
        .await
        .map(|_| ())
        .map_err(map_db_err)
    }

    async fn find_user_by_email(
        &self,
        canonical_email: &str,
    ) -> Result<Option<AuthUser>, AuthFlowError> {
        AuthUserEntity::find()
            .filter(AuthUserColumn::Email.eq(canonical_email))
            .one(&self.db)
            .await
            .map(|user| user.map(AuthUser::from))
            .map_err(map_db_err)
    }

    async fn find_user_by_username(
        &self,
        canonical_username: &str,
    ) -> Result<Option<AuthUser>, AuthFlowError> {
        AuthUserEntity::find()
            .filter(AuthUserColumn::Username.eq(canonical_username))
            .one(&self.db)
            .await
            .map(|user| user.map(AuthUser::from))
            .map_err(map_db_err)
    }

    async fn find_user_by_id(&self, user_id: Uuid) -> Result<Option<AuthUser>, AuthFlowError> {
        AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map(|user| user.map(AuthUser::from))
            .map_err(map_db_err)
    }

    async fn list_users(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<AuthUser>, usize), AuthFlowError> {
        let total = AuthUserEntity::find()
            .count(&self.db)
            .await
            .map_err(map_db_err)? as usize;
        let users = AuthUserEntity::find()
            .order_by_asc(AuthUserColumn::CreatedAt)
            .offset(offset as u64)
            .limit(limit as u64)
            .all(&self.db)
            .await
            .map_err(map_db_err)?
            .into_iter()
            .map(AuthUser::from)
            .collect();
        Ok((users, total))
    }

    async fn update_user_role(
        &self,
        user_id: Uuid,
        role: Option<String>,
    ) -> Result<AuthUser, AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.role = Set(role);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthUser::from)
            .map_err(map_db_err)
    }

    async fn update_user_ban(
        &self,
        user_id: Uuid,
        banned: bool,
        ban_reason: Option<String>,
        ban_expires: Option<chrono::DateTime<Utc>>,
    ) -> Result<AuthUser, AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.banned = Set(banned);
        active.ban_reason = Set(ban_reason);
        active.ban_expires = Set(ban_expires);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthUser::from)
            .map_err(map_db_err)
    }

    async fn update_user_email(
        &self,
        user_id: Uuid,
        email: String,
        email_verified: bool,
    ) -> Result<AuthUser, AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.email = Set(Some(email));
        active.email_verified = Set(email_verified);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthUser::from)
            .map_err(map_db_err)
    }

    async fn record_email_change(
        &self,
        user_id: Uuid,
        previous_email: Option<String>,
        new_email: String,
        changed_by: Option<Uuid>,
        reason: Option<String>,
    ) -> Result<AuthEmailChange, AuthFlowError> {
        let row = AuthEmailChangeActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            previous_email: Set(previous_email),
            new_email: Set(new_email),
            changed_by: Set(changed_by),
            reason: Set(reason),
            created_at: Set(Utc::now()),
        };
        row.insert(&self.db)
            .await
            .map(AuthEmailChange::from)
            .map_err(map_db_err)
    }

    async fn list_email_history(&self, user_id: Uuid) -> Result<Vec<AuthEmailChange>, AuthFlowError> {
        AuthEmailChangeEntity::find()
            .filter(AuthEmailChangeColumn::UserId.eq(user_id))
            // Oldest first: the trail reads as the chain of addresses in
            // the order they were held.
            .order_by_asc(AuthEmailChangeColumn::CreatedAt)
            .all(&self.db)
            .await
            .map(|rows| rows.into_iter().map(AuthEmailChange::from).collect())
            .map_err(map_db_err)
    }

    async fn find_user_id_by_previous_email(
        &self,
        email: &str,
    ) -> Result<Option<Uuid>, AuthFlowError> {
        AuthEmailChangeEntity::find()
            .filter(AuthEmailChangeColumn::PreviousEmail.eq(email.to_owned()))
            // An address can be held, released and held again; the most
            // recent record is the one that answers "who was this".
            .order_by_desc(AuthEmailChangeColumn::CreatedAt)
            .one(&self.db)
            .await
            .map(|row| row.map(|r| r.user_id))
            .map_err(map_db_err)
    }

    async fn update_user_profile(
        &self,
        user_id: Uuid,
        name: Option<String>,
        username: Option<String>,
        display_username: Option<String>,
        image: Option<String>,
        metadata_json: String,
    ) -> Result<AuthUser, AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.name = Set(name);
        active.username = Set(username);
        active.display_username = Set(display_username);
        active.image = Set(image);
        active.metadata_json = Set(metadata_json);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthUser::from)
            .map_err(map_db_err)
    }

    async fn create_account(&self, input: AuthAccountCreate) -> Result<AuthAccount, AuthFlowError> {
        let now = Utc::now();
        AuthAccountActiveModel {
            id: Set(Uuid::new_v4()),
            account_id: Set(input.account_id),
            provider_id: Set(input.provider_id),
            user_id: Set(input.user_id),
            access_token_ciphertext: Set(input.access_token_ciphertext),
            refresh_token_ciphertext: Set(input.refresh_token_ciphertext),
            id_token_ciphertext: Set(input.id_token_ciphertext),
            access_token_expires_at: Set(input.access_token_expires_at),
            refresh_token_expires_at: Set(input.refresh_token_expires_at),
            scope: Set(input.scope),
            password_hash: Set(input.password_hash),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthAccount::from)
        .map_err(map_db_err)
    }

    async fn find_account_by_provider_account(
        &self,
        provider_id: &str,
        account_id: &str,
    ) -> Result<Option<AuthAccount>, AuthFlowError> {
        AuthAccountEntity::find()
            .filter(AuthAccountColumn::ProviderId.eq(provider_id))
            .filter(AuthAccountColumn::AccountId.eq(account_id))
            .one(&self.db)
            .await
            .map(|account| account.map(AuthAccount::from))
            .map_err(map_db_err)
    }

    async fn find_password_account_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Option<AuthAccount>, AuthFlowError> {
        AuthAccountEntity::find()
            .filter(AuthAccountColumn::UserId.eq(user_id))
            .filter(AuthAccountColumn::ProviderId.eq("credential"))
            .one(&self.db)
            .await
            .map(|account| account.map(AuthAccount::from))
            .map_err(map_db_err)
    }

    async fn list_accounts_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<AuthAccount>, AuthFlowError> {
        AuthAccountEntity::find()
            .filter(AuthAccountColumn::UserId.eq(user_id))
            .all(&self.db)
            .await
            .map(|accounts| accounts.into_iter().map(AuthAccount::from).collect())
            .map_err(map_db_err)
    }

    async fn delete_user_by_id(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
        self.db
            .transaction::<_, (), sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    AuthSessionEntity::delete_many()
                        .filter(AuthSessionColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthAccountEntity::delete_many()
                        .filter(AuthAccountColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthApiKeyEntity::delete_many()
                        .filter(AuthApiKeyColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthPasskeyEntity::delete_many()
                        .filter(AuthPasskeyColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthTwoFactorEntity::delete_many()
                        .filter(AuthTwoFactorColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthTeamMemberEntity::delete_many()
                        .filter(AuthTeamMemberColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthMemberEntity::delete_many()
                        .filter(AuthMemberColumn::UserId.eq(user_id))
                        .exec(txn)
                        .await?;
                    AuthVerificationEntity::delete_many()
                        .filter(AuthVerificationColumn::Identifier.contains(user_id.to_string()))
                        .exec(txn)
                        .await?;
                    AuthUserEntity::delete_by_id(user_id).exec(txn).await?;
                    Ok(())
                })
            })
            .await
            .map_err(map_txn_err)
    }

    async fn delete_account_by_provider_account(
        &self,
        provider_id: &str,
        account_id: &str,
    ) -> Result<(), AuthFlowError> {
        AuthAccountEntity::delete_many()
            .filter(AuthAccountColumn::ProviderId.eq(provider_id))
            .filter(AuthAccountColumn::AccountId.eq(account_id))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

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
    ) -> Result<AuthAccount, AuthFlowError> {
        let account = AuthAccountEntity::find()
            .filter(AuthAccountColumn::ProviderId.eq(provider_id))
            .filter(AuthAccountColumn::AccountId.eq(account_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthAccountActiveModel = account.into();
        active.access_token_ciphertext = Set(access_token_ciphertext);
        if refresh_token_ciphertext.is_some() {
            active.refresh_token_ciphertext = Set(refresh_token_ciphertext);
        }
        if id_token_ciphertext.is_some() {
            active.id_token_ciphertext = Set(id_token_ciphertext);
        }
        active.access_token_expires_at = Set(access_token_expires_at);
        if refresh_token_expires_at.is_some() {
            active.refresh_token_expires_at = Set(refresh_token_expires_at);
        }
        if scope.is_some() {
            active.scope = Set(scope);
        }
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthAccount::from)
            .map_err(map_db_err)
    }

    async fn update_password_hash(
        &self,
        user_id: Uuid,
        password_hash: String,
    ) -> Result<(), AuthFlowError> {
        if let Some(account) = self.find_password_account_by_user_id(user_id).await? {
            let model = AuthAccountEntity::find_by_id(account.id)
                .one(&self.db)
                .await
                .map_err(map_db_err)?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let mut active: AuthAccountActiveModel = model.into();
            active.password_hash = Set(Some(password_hash));
            active.updated_at = Set(Utc::now());
            active
                .update(&self.db)
                .await
                .map(|_| ())
                .map_err(map_db_err)
        } else {
            Err(AuthFlowError::InvalidCredentials)
        }
    }

    async fn create_session(&self, input: AuthSessionCreate) -> Result<AuthSession, AuthFlowError> {
        let now = Utc::now();
        AuthSessionActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(input.user_id),
            token_hash: Set(input.token_hash),
            expires_at: Set(input.expires_at),
            ip_address: Set(input.ip_address),
            user_agent: Set(input.user_agent),
            impersonated_by: Set(input.impersonated_by),
            active_organization_id: Set(input.active_organization_id),
            active: Set(input.active),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthSession::from)
        .map_err(map_db_err)
    }

    async fn find_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<AuthSession>, AuthFlowError> {
        AuthSessionEntity::find()
            .filter(AuthSessionColumn::TokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .map(|session| session.map(AuthSession::from))
            .map_err(map_db_err)
    }

    async fn list_sessions_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<AuthSession>, AuthFlowError> {
        AuthSessionEntity::find()
            .filter(AuthSessionColumn::UserId.eq(user_id))
            .all(&self.db)
            .await
            .map(|sessions| sessions.into_iter().map(AuthSession::from).collect())
            .map_err(map_db_err)
    }

    async fn deactivate_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(), AuthFlowError> {
        let Some(session) = AuthSessionEntity::find()
            .filter(AuthSessionColumn::TokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
        else {
            return Ok(());
        };
        let mut active: AuthSessionActiveModel = session.into();
        active.active = Set(false);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn deactivate_session_by_id(&self, session_id: Uuid) -> Result<(), AuthFlowError> {
        let Some(session) = AuthSessionEntity::find_by_id(session_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
        else {
            return Ok(());
        };
        let mut active: AuthSessionActiveModel = session.into();
        active.active = Set(false);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn activate_session_by_token_hash(&self, token_hash: &str) -> Result<(), AuthFlowError> {
        let session = AuthSessionEntity::find()
            .filter(AuthSessionColumn::TokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthSessionActiveModel = session.into();
        active.active = Set(true);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn deactivate_sessions_by_user_id(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
        let sessions = AuthSessionEntity::find()
            .filter(AuthSessionColumn::UserId.eq(user_id))
            .all(&self.db)
            .await
            .map_err(map_db_err)?;
        for session in sessions {
            let mut active: AuthSessionActiveModel = session.into();
            active.active = Set(false);
            active.updated_at = Set(Utc::now());
            active.update(&self.db).await.map_err(map_db_err)?;
        }
        Ok(())
    }

    async fn deactivate_other_sessions_by_user_id(
        &self,
        user_id: Uuid,
        except_session_id: Uuid,
    ) -> Result<(), AuthFlowError> {
        let sessions = AuthSessionEntity::find()
            .filter(AuthSessionColumn::UserId.eq(user_id))
            .filter(AuthSessionColumn::Id.ne(except_session_id))
            .all(&self.db)
            .await
            .map_err(map_db_err)?;
        for session in sessions {
            let mut active: AuthSessionActiveModel = session.into();
            active.active = Set(false);
            active.updated_at = Set(Utc::now());
            active.update(&self.db).await.map_err(map_db_err)?;
        }
        Ok(())
    }

    async fn create_verification(
        &self,
        input: AuthVerificationCreate,
    ) -> Result<AuthVerification, AuthFlowError> {
        let now = Utc::now();
        AuthVerificationActiveModel {
            id: Set(Uuid::new_v4()),
            identifier: Set(input.identifier),
            value_hash: Set(input.value_hash),
            expires_at: Set(input.expires_at),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthVerification::from)
        .map_err(map_db_err)
    }

    async fn find_verification(
        &self,
        identifier: &str,
        value_hash: &str,
    ) -> Result<Option<AuthVerification>, AuthFlowError> {
        AuthVerificationEntity::find()
            .filter(AuthVerificationColumn::Identifier.eq(identifier))
            .filter(AuthVerificationColumn::ValueHash.eq(value_hash))
            .one(&self.db)
            .await
            .map(|verification| verification.map(AuthVerification::from))
            .map_err(map_db_err)
    }

    async fn find_latest_verification_by_identifier(
        &self,
        identifier: &str,
    ) -> Result<Option<AuthVerification>, AuthFlowError> {
        AuthVerificationEntity::find()
            .filter(AuthVerificationColumn::Identifier.eq(identifier))
            .order_by_desc(AuthVerificationColumn::CreatedAt)
            .one(&self.db)
            .await
            .map(|verification| verification.map(AuthVerification::from))
            .map_err(map_db_err)
    }

    async fn delete_verification(&self, id: Uuid) -> Result<(), AuthFlowError> {
        AuthVerificationEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.email_verified = Set(true);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_api_key(&self, input: AuthApiKeyCreate) -> Result<AuthApiKey, AuthFlowError> {
        let now = Utc::now();
        AuthApiKeyActiveModel {
            id: Set(Uuid::new_v4()),
            name: Set(input.name),
            prefix: Set(input.prefix),
            key_hash: Set(input.key_hash),
            user_id: Set(input.user_id),
            enabled: Set(input.enabled),
            rate_limit_enabled: Set(input.rate_limit_enabled),
            rate_limit_time_window: Set(input.rate_limit_time_window),
            rate_limit_max: Set(input.rate_limit_max),
            request_count: Set(input.request_count),
            remaining: Set(input.remaining),
            expires_at: Set(input.expires_at),
            permissions_json: Set(input.permissions_json),
            metadata_json: Set(input.metadata_json),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthApiKey::from)
        .map_err(map_db_err)
    }

    async fn find_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<AuthApiKey>, AuthFlowError> {
        AuthApiKeyEntity::find()
            .filter(AuthApiKeyColumn::KeyHash.eq(key_hash))
            .one(&self.db)
            .await
            .map(|key| key.map(AuthApiKey::from))
            .map_err(map_db_err)
    }

    async fn find_api_key_by_id(&self, id: Uuid) -> Result<Option<AuthApiKey>, AuthFlowError> {
        AuthApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map(|key| key.map(AuthApiKey::from))
            .map_err(map_db_err)
    }

    async fn list_api_keys_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<AuthApiKey>, AuthFlowError> {
        AuthApiKeyEntity::find()
            .filter(AuthApiKeyColumn::UserId.eq(user_id))
            .all(&self.db)
            .await
            .map(|keys| keys.into_iter().map(AuthApiKey::from).collect())
            .map_err(map_db_err)
    }

    async fn update_api_key(
        &self,
        id: Uuid,
        name: Option<String>,
        enabled: Option<bool>,
        expires_at: Option<DateTime<Utc>>,
        permissions_json: Option<String>,
        rate_limit_time_window: Option<i64>,
        rate_limit_max: Option<i64>,
        metadata_json: Option<String>,
    ) -> Result<AuthApiKey, AuthFlowError> {
        let key = AuthApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthApiKeyActiveModel = key.into();
        if name.is_some() {
            active.name = Set(name);
        }
        if let Some(enabled) = enabled {
            active.enabled = Set(enabled);
        }
        if expires_at.is_some() {
            active.expires_at = Set(expires_at);
        }
        if permissions_json.is_some() {
            active.permissions_json = Set(permissions_json);
        }
        if rate_limit_time_window.is_some() {
            active.rate_limit_time_window = Set(rate_limit_time_window);
        }
        if rate_limit_max.is_some() {
            active.rate_limit_enabled = Set(true);
            active.rate_limit_max = Set(rate_limit_max);
            active.remaining = Set(rate_limit_max);
            active.request_count = Set(Some(0));
        }
        if metadata_json.is_some() {
            active.metadata_json = Set(metadata_json);
        }
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthApiKey::from)
            .map_err(map_db_err)
    }

    async fn delete_api_key(&self, id: Uuid) -> Result<(), AuthFlowError> {
        AuthApiKeyEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn update_api_key_usage(
        &self,
        id: Uuid,
        request_count: Option<i64>,
        remaining: Option<i64>,
    ) -> Result<(), AuthFlowError> {
        let key = AuthApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthApiKeyActiveModel = key.into();
        active.request_count = Set(request_count);
        active.remaining = Set(remaining);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn set_api_key_enabled(&self, id: Uuid, enabled: bool) -> Result<(), AuthFlowError> {
        let key = AuthApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthApiKeyActiveModel = key.into();
        active.enabled = Set(enabled);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_passkey(&self, input: AuthPasskeyCreate) -> Result<AuthPasskey, AuthFlowError> {
        AuthPasskeyActiveModel {
            id: Set(Uuid::new_v4()),
            name: Set(input.name),
            user_id: Set(input.user_id),
            public_key: Set(input.public_key),
            credential_id: Set(input.credential_id),
            counter: Set(input.counter),
            device_type: Set(input.device_type),
            backed_up: Set(input.backed_up),
            transports: Set(input.transports),
            created_at: Set(Utc::now()),
        }
        .insert(&self.db)
        .await
        .map(AuthPasskey::from)
        .map_err(map_db_err)
    }

    async fn find_passkey_by_credential_id(
        &self,
        credential_id: &str,
    ) -> Result<Option<AuthPasskey>, AuthFlowError> {
        AuthPasskeyEntity::find()
            .filter(AuthPasskeyColumn::CredentialId.eq(credential_id))
            .one(&self.db)
            .await
            .map(|passkey| passkey.map(AuthPasskey::from))
            .map_err(map_db_err)
    }

    async fn list_passkeys_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<AuthPasskey>, AuthFlowError> {
        AuthPasskeyEntity::find()
            .filter(AuthPasskeyColumn::UserId.eq(user_id))
            .all(&self.db)
            .await
            .map(|passkeys| passkeys.into_iter().map(AuthPasskey::from).collect())
            .map_err(map_db_err)
    }

    async fn update_passkey_counter(
        &self,
        credential_id: &str,
        counter: i64,
    ) -> Result<AuthPasskey, AuthFlowError> {
        let passkey = AuthPasskeyEntity::find()
            .filter(AuthPasskeyColumn::CredentialId.eq(credential_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthPasskeyActiveModel = passkey.into();
        active.counter = Set(counter);
        active
            .update(&self.db)
            .await
            .map(AuthPasskey::from)
            .map_err(map_db_err)
    }

    async fn delete_passkey_by_credential_id(
        &self,
        credential_id: &str,
    ) -> Result<(), AuthFlowError> {
        AuthPasskeyEntity::delete_many()
            .filter(AuthPasskeyColumn::CredentialId.eq(credential_id))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_organization(
        &self,
        input: AuthOrganizationCreate,
    ) -> Result<AuthOrganization, AuthFlowError> {
        let now = Utc::now();
        AuthOrganizationActiveModel {
            id: Set(Uuid::new_v4()),
            name: Set(input.name),
            slug: Set(input.slug),
            logo: Set(input.logo),
            metadata_json: Set(input.metadata_json),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthOrganization::from)
        .map_err(map_db_err)
    }

    async fn create_organization_with_owner(
        &self,
        organization_input: AuthOrganizationCreate,
        owner_user_id: Uuid,
    ) -> Result<(AuthOrganization, AuthMember), AuthFlowError> {
        self.db
            .transaction::<_, (AuthOrganization, AuthMember), sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    let now = Utc::now();
                    let organization = AuthOrganizationActiveModel {
                        id: Set(Uuid::new_v4()),
                        name: Set(organization_input.name),
                        slug: Set(organization_input.slug),
                        logo: Set(organization_input.logo),
                        metadata_json: Set(organization_input.metadata_json),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    let member = AuthMemberActiveModel {
                        id: Set(Uuid::new_v4()),
                        organization_id: Set(organization.id),
                        user_id: Set(owner_user_id),
                        role: Set("owner".into()),
                        created_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    Ok((
                        AuthOrganization::from(organization),
                        AuthMember::from(member),
                    ))
                })
            })
            .await
            .map_err(map_txn_err)
    }

    async fn find_organization_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<AuthOrganization>, AuthFlowError> {
        AuthOrganizationEntity::find()
            .filter(AuthOrganizationColumn::Slug.eq(slug))
            .one(&self.db)
            .await
            .map(|org| org.map(AuthOrganization::from))
            .map_err(map_db_err)
    }

    async fn create_member(&self, input: AuthMemberCreate) -> Result<AuthMember, AuthFlowError> {
        AuthMemberActiveModel {
            id: Set(Uuid::new_v4()),
            organization_id: Set(input.organization_id),
            user_id: Set(input.user_id),
            role: Set(input.role),
            created_at: Set(Utc::now()),
        }
        .insert(&self.db)
        .await
        .map(AuthMember::from)
        .map_err(map_db_err)
    }

    async fn find_member(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<AuthMember>, AuthFlowError> {
        AuthMemberEntity::find()
            .filter(AuthMemberColumn::OrganizationId.eq(organization_id))
            .filter(AuthMemberColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map(|member| member.map(AuthMember::from))
            .map_err(map_db_err)
    }

    async fn list_members_by_organization(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<AuthMember>, AuthFlowError> {
        AuthMemberEntity::find()
            .filter(AuthMemberColumn::OrganizationId.eq(organization_id))
            .all(&self.db)
            .await
            .map(|members| members.into_iter().map(AuthMember::from).collect())
            .map_err(map_db_err)
    }

    async fn update_member_role(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        role: String,
    ) -> Result<AuthMember, AuthFlowError> {
        let member = AuthMemberEntity::find()
            .filter(AuthMemberColumn::OrganizationId.eq(organization_id))
            .filter(AuthMemberColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::PermissionDenied)?;
        let mut active: AuthMemberActiveModel = member.into();
        active.role = Set(role);
        active
            .update(&self.db)
            .await
            .map(AuthMember::from)
            .map_err(map_db_err)
    }

    async fn create_organization_role(
        &self,
        input: AuthOrganizationRoleCreate,
    ) -> Result<AuthOrganizationRole, AuthFlowError> {
        let now = Utc::now();
        AuthOrganizationRoleActiveModel {
            id: Set(Uuid::new_v4()),
            organization_id: Set(input.organization_id),
            role: Set(input.role),
            permissions_json: Set(input.permissions_json),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthOrganizationRole::from)
        .map_err(map_db_err)
    }

    async fn find_organization_role(
        &self,
        organization_id: Uuid,
        role: &str,
    ) -> Result<Option<AuthOrganizationRole>, AuthFlowError> {
        AuthOrganizationRoleEntity::find()
            .filter(AuthOrganizationRoleColumn::OrganizationId.eq(organization_id))
            .filter(AuthOrganizationRoleColumn::Role.eq(role))
            .one(&self.db)
            .await
            .map(|role| role.map(AuthOrganizationRole::from))
            .map_err(map_db_err)
    }

    async fn list_organization_roles(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<AuthOrganizationRole>, AuthFlowError> {
        AuthOrganizationRoleEntity::find()
            .filter(AuthOrganizationRoleColumn::OrganizationId.eq(organization_id))
            .all(&self.db)
            .await
            .map(|roles| roles.into_iter().map(AuthOrganizationRole::from).collect())
            .map_err(map_db_err)
    }

    async fn update_organization_role(
        &self,
        organization_id: Uuid,
        role: &str,
        permissions_json: String,
    ) -> Result<AuthOrganizationRole, AuthFlowError> {
        let role = AuthOrganizationRoleEntity::find()
            .filter(AuthOrganizationRoleColumn::OrganizationId.eq(organization_id))
            .filter(AuthOrganizationRoleColumn::Role.eq(role))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthOrganizationRoleActiveModel = role.into();
        active.permissions_json = Set(permissions_json);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthOrganizationRole::from)
            .map_err(map_db_err)
    }

    async fn delete_organization_role(
        &self,
        organization_id: Uuid,
        role: &str,
    ) -> Result<(), AuthFlowError> {
        AuthOrganizationRoleEntity::delete_many()
            .filter(AuthOrganizationRoleColumn::OrganizationId.eq(organization_id))
            .filter(AuthOrganizationRoleColumn::Role.eq(role))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_team(&self, input: AuthTeamCreate) -> Result<AuthTeam, AuthFlowError> {
        let now = Utc::now();
        AuthTeamActiveModel {
            id: Set(Uuid::new_v4()),
            organization_id: Set(input.organization_id),
            name: Set(input.name),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthTeam::from)
        .map_err(map_db_err)
    }

    async fn find_team_by_id(&self, id: Uuid) -> Result<Option<AuthTeam>, AuthFlowError> {
        AuthTeamEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map(|team| team.map(AuthTeam::from))
            .map_err(map_db_err)
    }

    async fn list_teams_by_organization(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<AuthTeam>, AuthFlowError> {
        AuthTeamEntity::find()
            .filter(AuthTeamColumn::OrganizationId.eq(organization_id))
            .all(&self.db)
            .await
            .map(|teams| teams.into_iter().map(AuthTeam::from).collect())
            .map_err(map_db_err)
    }

    async fn update_team_name(&self, id: Uuid, name: String) -> Result<AuthTeam, AuthFlowError> {
        let team = AuthTeamEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthTeamActiveModel = team.into();
        active.name = Set(name);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(AuthTeam::from)
            .map_err(map_db_err)
    }

    async fn delete_team(&self, id: Uuid) -> Result<(), AuthFlowError> {
        AuthTeamMemberEntity::delete_many()
            .filter(AuthTeamMemberColumn::TeamId.eq(id))
            .exec(&self.db)
            .await
            .map_err(map_db_err)?;
        AuthTeamEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_team_member(
        &self,
        input: AuthTeamMemberCreate,
    ) -> Result<AuthTeamMember, AuthFlowError> {
        AuthTeamMemberActiveModel {
            id: Set(Uuid::new_v4()),
            team_id: Set(input.team_id),
            user_id: Set(input.user_id),
            created_at: Set(Utc::now()),
        }
        .insert(&self.db)
        .await
        .map(AuthTeamMember::from)
        .map_err(map_db_err)
    }

    async fn find_team_member(
        &self,
        team_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<AuthTeamMember>, AuthFlowError> {
        AuthTeamMemberEntity::find()
            .filter(AuthTeamMemberColumn::TeamId.eq(team_id))
            .filter(AuthTeamMemberColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map(|member| member.map(AuthTeamMember::from))
            .map_err(map_db_err)
    }

    async fn list_team_members(&self, team_id: Uuid) -> Result<Vec<AuthTeamMember>, AuthFlowError> {
        AuthTeamMemberEntity::find()
            .filter(AuthTeamMemberColumn::TeamId.eq(team_id))
            .all(&self.db)
            .await
            .map(|members| members.into_iter().map(AuthTeamMember::from).collect())
            .map_err(map_db_err)
    }

    async fn delete_team_member(&self, team_id: Uuid, user_id: Uuid) -> Result<(), AuthFlowError> {
        AuthTeamMemberEntity::delete_many()
            .filter(AuthTeamMemberColumn::TeamId.eq(team_id))
            .filter(AuthTeamMemberColumn::UserId.eq(user_id))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn update_session_active_organization(
        &self,
        token_hash: &str,
        organization_id: Option<Uuid>,
    ) -> Result<(), AuthFlowError> {
        let session = AuthSessionEntity::find()
            .filter(AuthSessionColumn::TokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthSessionActiveModel = session.into();
        active.active_organization_id = Set(organization_id);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn create_invitation(
        &self,
        input: AuthInvitationCreate,
    ) -> Result<AuthInvitation, AuthFlowError> {
        AuthInvitationActiveModel {
            id: Set(Uuid::new_v4()),
            organization_id: Set(input.organization_id),
            email: Set(input.email),
            role: Set(input.role),
            status: Set(input.status),
            inviter_id: Set(input.inviter_id),
            expires_at: Set(input.expires_at),
            created_at: Set(Utc::now()),
        }
        .insert(&self.db)
        .await
        .map(AuthInvitation::from)
        .map_err(map_db_err)
    }

    async fn find_invitation_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<AuthInvitation>, AuthFlowError> {
        AuthInvitationEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map(|invite| invite.map(AuthInvitation::from))
            .map_err(map_db_err)
    }

    async fn update_invitation_status(
        &self,
        id: Uuid,
        status: String,
    ) -> Result<(), AuthFlowError> {
        let invitation = AuthInvitationEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthInvitationActiveModel = invitation.into();
        active.status = Set(status);
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn accept_invitation_membership(
        &self,
        member_input: AuthMemberCreate,
        invitation_id: Uuid,
        accepted_status: String,
        verification_id: Uuid,
    ) -> Result<(), AuthFlowError> {
        self.db
            .transaction::<_, (), sea_orm::DbErr>(|txn| {
                Box::pin(async move {
                    AuthMemberActiveModel {
                        id: Set(Uuid::new_v4()),
                        organization_id: Set(member_input.organization_id),
                        user_id: Set(member_input.user_id),
                        role: Set(member_input.role),
                        created_at: Set(Utc::now()),
                    }
                    .insert(txn)
                    .await?;

                    let invitation = AuthInvitationEntity::find_by_id(invitation_id)
                        .one(txn)
                        .await?
                        .ok_or_else(|| sea_orm::DbErr::RecordNotFound(invitation_id.to_string()))?;
                    let mut active: AuthInvitationActiveModel = invitation.into();
                    active.status = Set(accepted_status);
                    active.update(txn).await?;

                    AuthVerificationEntity::delete_by_id(verification_id)
                        .exec(txn)
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(map_txn_err)
    }

    async fn create_two_factor(
        &self,
        input: AuthTwoFactorCreate,
    ) -> Result<AuthTwoFactor, AuthFlowError> {
        let now = Utc::now();
        AuthTwoFactorActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(input.user_id),
            secret_ciphertext: Set(input.secret_ciphertext),
            backup_codes_hash: Set(input.backup_codes_hash),
            attempt_count: Set(input.attempt_count),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(AuthTwoFactor::from)
        .map_err(map_db_err)
    }

    async fn find_two_factor_by_user_id(
        &self,
        user_id: Uuid,
    ) -> Result<Option<AuthTwoFactor>, AuthFlowError> {
        AuthTwoFactorEntity::find()
            .filter(AuthTwoFactorColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map(|two_factor| two_factor.map(AuthTwoFactor::from))
            .map_err(map_db_err)
    }

    async fn delete_two_factor(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
        AuthTwoFactorEntity::delete_many()
            .filter(AuthTwoFactorColumn::UserId.eq(user_id))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn update_two_factor_backup_codes(
        &self,
        user_id: Uuid,
        backup_codes_hash: Option<String>,
    ) -> Result<(), AuthFlowError> {
        let two_factor = AuthTwoFactorEntity::find()
            .filter(AuthTwoFactorColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthTwoFactorActiveModel = two_factor.into();
        active.backup_codes_hash = Set(backup_codes_hash);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn increment_two_factor_attempts(&self, user_id: Uuid) -> Result<i64, AuthFlowError> {
        let two_factor = AuthTwoFactorEntity::find()
            .filter(AuthTwoFactorColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let next = two_factor.attempt_count + 1;
        let mut active: AuthTwoFactorActiveModel = two_factor.into();
        active.attempt_count = Set(next);
        active.updated_at = Set(Utc::now());
        active.update(&self.db).await.map_err(map_db_err)?;
        Ok(next)
    }

    async fn reset_two_factor_attempts(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
        let two_factor = AuthTwoFactorEntity::find()
            .filter(AuthTwoFactorColumn::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthTwoFactorActiveModel = two_factor.into();
        active.attempt_count = Set(0);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }

    async fn set_user_two_factor_enabled(
        &self,
        user_id: Uuid,
        enabled: bool,
    ) -> Result<(), AuthFlowError> {
        let user = AuthUserEntity::find_by_id(user_id)
            .one(&self.db)
            .await
            .map_err(map_db_err)?
            .ok_or(AuthFlowError::InvalidCredentials)?;
        let mut active: AuthUserActiveModel = user.into();
        active.two_factor_enabled = Set(enabled);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.db)
            .await
            .map(|_| ())
            .map_err(map_db_err)
    }
}
