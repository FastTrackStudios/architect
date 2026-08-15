//! Typed auth capability modules.

pub mod admin {
    use auth_proto::{
        AuthAccountCreate, AuthFlowError, AuthSessionBundle, AuthUser, AuthUserCreate, OrgMember,
    };
    use chrono::Utc;

    use super::email_password::{
        PASSWORD_PROVIDER_ID, normalize_email, validate_metadata, validate_password_strength,
    };
    use crate::{
        AdminCreateUser, AdminHasPermission, AdminSetUserPassword, ArchitectAuth, AuthAuditEvent,
        AuthStorage, BanUser, HasPermissionResult, ImpersonateUser, ListUserSessions, ListUsers,
        ListUsersResult, RemoveUser, RevokeUserSession, RevokeUserSessions, SetUserRole,
        StopImpersonating, UnbanUser, commands::CurrentSession, crypto::hash_password,
    };

    const ADMIN_ROLE: &str = "admin";

    /// First 8 hex chars of a user id — last-resort display label when a
    /// member has neither a name nor an email.
    fn short_id(id: uuid::Uuid) -> String {
        let s = id.to_string();
        s.get(..8).unwrap_or(&s).to_string()
    }

    /// The Guest / anonymous stand-in account is not a real member.
    fn member_excluded(name: &str, email: &str) -> bool {
        name.eq_ignore_ascii_case("guest") || email.to_ascii_lowercase().starts_with("guest@")
    }

    /// Drop duplicate members — same user id, or the same display name
    /// (a person seeded under two accounts). Keeps the first seen.
    fn dedupe_members(members: Vec<OrgMember>) -> Vec<OrgMember> {
        let mut seen_id = std::collections::HashSet::new();
        let mut seen_name = std::collections::HashSet::new();
        members
            .into_iter()
            .filter(|m| {
                seen_id.insert(m.user_id) && seen_name.insert(m.name.to_ascii_lowercase())
            })
            .collect()
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.list-users]
        pub async fn list_users(&self, input: ListUsers) -> Result<ListUsersResult, AuthFlowError> {
            self.require_admin(&input.session_token).await?;
            let limit = input.limit.clamp(1, 100);
            let (users, total) = self.storage.list_users(input.offset, limit).await?;
            Ok(ListUsersResult { users, total })
        }

        /// Enumerate the members of THIS org (the store is org-scoped by
        /// the `/org/<slug>/vox` mount).
        ///
        /// Backend for `AuthService::list_org_members`. Auth is NOT
        /// required — the endpoint already scopes the org, and a session
        /// token is per-org, so a cross-org viewer's token (or none) must
        /// not gate the read. When the caller HAS a valid session for
        /// this org AND the org has real membership rows, those are used
        /// (precise roles); otherwise it enumerates the store's users
        /// with role `"member"`. Guest/anonymous accounts and duplicate
        /// people are dropped either way.
        pub(crate) async fn org_members_for_token(
            &self,
            token: String,
        ) -> Result<Vec<OrgMember>, AuthFlowError> {
            fn display(user: &AuthUser) -> (String, String) {
                let email = user
                    .email
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_default();
                let name = user
                    .name
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| user.email.clone().filter(|s| !s.trim().is_empty()))
                    .unwrap_or_else(|| short_id(user.id));
                (name, email)
            }

            // A VALID SESSION FOR THIS ORG IS REQUIRED.
            //
            // This method is reachable without a session on any lane that
            // treats `AuthService` as public (Task's org lane does, so the
            // sign-in path stays usable). The enumerate-everything
            // fallback below therefore used to answer ANONYMOUS callers:
            // every user's name, email and id, for every org, over the
            // internet, even with permission enforcement on. Verified on
            // production 2026-08-08 with a CLI holding no credentials.
            //
            // The fallback itself is still right for its actual purpose —
            // an org whose users predate membership rows — so it is kept,
            // but now only behind a session that validates HERE. A
            // foreign token doesn't (each org has its own auth store), so
            // this also stops one org's members being read with another
            // org's session.
            let Ok(bundle) = self.current_session(CurrentSession { token }).await else {
                return Err(AuthFlowError::PermissionDenied);
            };
            // Precise path: valid session for THIS org + real membership rows.
            if let Some(org_id) = bundle.session.active_organization_id {
                        let members = self.storage.list_members_by_organization(org_id).await?;
                        if !members.is_empty() {
                            let mut out = Vec::with_capacity(members.len());
                            for member in members {
                                let user = self.storage.find_user_by_id(member.user_id).await?;
                                let (name, email) = match &user {
                                    Some(user) => display(user),
                                    None => (short_id(member.user_id), String::new()),
                                };
                                if member_excluded(&name, &email) {
                                    continue;
                                }
                                out.push(OrgMember {
                                    user_id: member.user_id,
                                    name,
                                    email,
                                    role: member.role,
                                });
                            }
                            return Ok(dedupe_members(out));
                        }
                    }

            // Fallback: a validated session, but this org keeps no
            // membership rows — enumerate its users. Anonymous callers
            // never reach here.
            let (users, _total) = self.storage.list_users(0, 1000).await?;
            let out = users
                .into_iter()
                .filter_map(|user| {
                    let (name, email) = display(&user);
                    if member_excluded(&name, &email) {
                        return None;
                    }
                    Some(OrgMember {
                        user_id: user.id,
                        name,
                        email,
                        role: "member".to_string(),
                    })
                })
                .collect();
            Ok(dedupe_members(out))
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.create-user]
        // r[impl auth.admin.audit]
        pub async fn admin_create_user(
            &self,
            input: AdminCreateUser,
        ) -> Result<AuthUser, AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            let canonical_email = normalize_email(&input.email)?;
            if self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput("email already exists".into()));
            }
            validate_metadata(input.metadata_json.as_deref())?;

            let password_hash = if let Some(password) = input.password.as_deref() {
                validate_password_strength(password)?;
                self.reject_breached_password(password).await?;
                Some(
                    hash_password(password)
                        .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                )
            } else {
                None
            };

            let user = self
                .storage
                .create_user(AuthUserCreate {
                    email: Some(canonical_email),
                    name: input.name,
                    email_verified: true,
                    image: None,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: input.role,
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json: input.metadata_json.unwrap_or_else(|| "{}".into()),
                })
                .await?;

            if let Some(password_hash) = password_hash {
                self.storage
                    .create_account(AuthAccountCreate {
                        account_id: user.id.to_string(),
                        provider_id: PASSWORD_PROVIDER_ID.into(),
                        user_id: user.id,
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: None,
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: None,
                        password_hash: Some(password_hash),
                    })
                    .await?;
            }

            self.record_admin_audit(admin.id, Some(user.id), "admin.create_user")
                .await?;
            Ok(user)
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.no-self-lockout]
        // r[impl auth.admin.audit]
        pub async fn set_user_role(&self, input: SetUserRole) -> Result<AuthUser, AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            if admin.id == input.user_id && input.role.as_deref() != Some(ADMIN_ROLE) {
                return Err(AuthFlowError::PermissionDenied);
            }
            let user = self
                .storage
                .update_user_role(input.user_id, input.role)
                .await?;
            self.record_admin_audit(admin.id, Some(user.id), "admin.set_user_role")
                .await?;
            Ok(user)
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.set-user-password]
        // r[impl auth.admin.audit]
        pub async fn admin_set_user_password(
            &self,
            input: AdminSetUserPassword,
        ) -> Result<(), AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            let user = self
                .storage
                .find_user_by_id(input.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            validate_password_strength(&input.new_password)?;
            self.reject_breached_password(&input.new_password).await?;
            let password_hash = hash_password(&input.new_password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;

            if self
                .storage
                .find_password_account_by_user_id(input.user_id)
                .await?
                .is_some()
            {
                self.storage
                    .update_password_hash(input.user_id, password_hash)
                    .await?;
            } else {
                self.storage
                    .create_account(AuthAccountCreate {
                        account_id: user.id.to_string(),
                        provider_id: PASSWORD_PROVIDER_ID.into(),
                        user_id: user.id,
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: None,
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: None,
                        password_hash: Some(password_hash),
                    })
                    .await?;
            }

            self.record_admin_audit(admin.id, Some(input.user_id), "admin.set_user_password")
                .await
        }

        /// Grant or clear the `admin` role with NO admin session —
        /// operator tooling authorized by possession of the data root.
        ///
        /// This is the bootstrap for admin itself: `require_admin` needs
        /// an existing admin, so the FIRST one cannot be made through the
        /// admin flows. Something outside them has to seed it, and
        /// possession of the auth store is the only authority that
        /// predates any account.
        ///
        /// Note this sets architect-auth's `user.role`, which gates the
        /// `admin_*` flows. It is NOT the permission gate's role — that
        /// comes from `architect-permissions`' engine, which currently
        /// assigns every validated user the same default.
        pub async fn set_user_role_local_trusted(
            &self,
            user_id: uuid::Uuid,
            role: Option<String>,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            self.storage
                .find_user_by_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.update_user_role(user_id, role).await
        }

        /// Delete a user with NO admin session — operator tooling whose
        /// authorization is possession of the data root.
        ///
        /// Returns the deleted user so a caller can report what it
        /// removed; deleting a nonexistent account is an error rather
        /// than a silent success, because "did that do anything?" is the
        /// question an operator actually has.
        pub async fn delete_user_local_trusted(
            &self,
            user_id: uuid::Uuid,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            let user = self
                .storage
                .find_user_by_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.delete_user_by_id(user_id).await?;
            Ok(user)
        }

        /// Every user in this org's store, for operator tooling.
        pub async fn list_users_local_trusted(
            &self,
        ) -> Result<Vec<auth_proto::AuthUser>, AuthFlowError> {
            // 1000 is the same ceiling `org_members_for_token` uses; an
            // org with more users than that has bigger problems than this
            // listing.
            let (users, _total) = self.storage.list_users(0, 1000).await?;
            Ok(users)
        }

        /// Set a user's password with NO admin session.
        ///
        /// For operator tooling whose authorization is possession of the
        /// data root rather than a session — the same basis as
        /// `OrgManagementImpl::new_local_trusted`. It exists because the
        /// case that needs it most is precisely the one where no session
        /// can be had: the owner is locked out.
        ///
        /// Keeps every check `admin_set_user_password` applies to the new
        /// password — strength and known-breach rejection — and creates
        /// the password account when the user has none (an OAuth-only
        /// account being given a password). What it drops is only the
        /// admin lookup and the admin audit row, neither of which has a
        /// meaning without an admin.
        ///
        /// Deliberately NOT exposed over vox: a caller that can reach the
        /// network surface has not proven possession of anything.
        pub async fn set_user_password_local_trusted(
            &self,
            user_id: uuid::Uuid,
            new_password: &str,
        ) -> Result<(), AuthFlowError> {
            let user = self
                .storage
                .find_user_by_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            validate_password_strength(new_password)?;
            self.reject_breached_password(new_password).await?;
            let password_hash = hash_password(new_password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            if self
                .storage
                .find_password_account_by_user_id(user_id)
                .await?
                .is_some()
            {
                self.storage
                    .update_password_hash(user_id, password_hash)
                    .await
            } else {
                self.storage
                    .create_account(AuthAccountCreate {
                        account_id: user.id.to_string(),
                        provider_id: PASSWORD_PROVIDER_ID.into(),
                        user_id: user.id,
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: None,
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: None,
                        password_hash: Some(password_hash),
                    })
                    .await
                    .map(|_| ())
            }
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.ban]
        // r[impl auth.admin.ban-expiry]
        // r[impl auth.admin.revoke-sessions]
        // r[impl auth.admin.audit]
        pub async fn ban_user(&self, input: BanUser) -> Result<AuthUser, AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            if admin.id == input.user_id {
                return Err(AuthFlowError::PermissionDenied);
            }
            let user = self
                .storage
                .update_user_ban(input.user_id, true, input.reason, input.expires_at)
                .await?;
            self.storage.deactivate_sessions_by_user_id(user.id).await?;
            self.record_admin_audit(admin.id, Some(user.id), "admin.ban_user")
                .await?;
            Ok(user)
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.audit]
        pub async fn unban_user(&self, input: UnbanUser) -> Result<AuthUser, AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            let user = self
                .storage
                .update_user_ban(input.user_id, false, None, None)
                .await?;
            self.record_admin_audit(admin.id, Some(user.id), "admin.unban_user")
                .await?;
            Ok(user)
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.revoke-sessions]
        // r[impl auth.admin.audit]
        pub async fn revoke_user_sessions(
            &self,
            input: RevokeUserSessions,
        ) -> Result<(), AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            self.storage
                .deactivate_sessions_by_user_id(input.user_id)
                .await?;
            self.record_admin_audit(admin.id, Some(input.user_id), "admin.revoke_user_sessions")
                .await
        }

        // r[impl auth.admin.list-user-sessions]
        pub async fn list_user_sessions(
            &self,
            input: ListUserSessions,
        ) -> Result<Vec<auth_proto::AuthSession>, AuthFlowError> {
            self.require_admin(&input.session_token).await?;
            self.storage.list_sessions_by_user_id(input.user_id).await
        }

        // r[impl auth.admin.revoke-session]
        pub async fn revoke_user_session(
            &self,
            input: RevokeUserSession,
        ) -> Result<(), AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            let owns_session = self
                .storage
                .list_sessions_by_user_id(input.user_id)
                .await?
                .into_iter()
                .any(|session| session.id == input.session_id);
            if !owns_session {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage
                .deactivate_session_by_id(input.session_id)
                .await?;
            self.record_admin_audit(admin.id, Some(input.user_id), "admin.revoke_user_session")
                .await
        }

        // r[impl auth.admin.requires-role]
        // r[impl auth.admin.impersonate]
        // r[impl auth.sessions.impersonation]
        // r[impl auth.admin.audit]
        pub async fn impersonate_user(
            &self,
            input: ImpersonateUser,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            if admin.id == input.user_id {
                return Err(AuthFlowError::PermissionDenied);
            }
            let user = self
                .storage
                .find_user_by_id(input.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let session = self
                .issue_session(
                    user,
                    input.ip_address,
                    input.user_agent,
                    Some(admin.id),
                    None,
                )
                .await?;
            self.record_admin_audit(admin.id, Some(input.user_id), "admin.impersonate_user")
                .await?;
            Ok(session)
        }

        // r[impl auth.admin.stop-impersonating]
        pub async fn stop_impersonating(
            &self,
            input: StopImpersonating,
        ) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            if bundle.session.impersonated_by.is_none() {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage
                .deactivate_session_by_id(bundle.session.id)
                .await
        }

        // r[impl auth.admin.remove-user]
        pub async fn remove_user(&self, input: RemoveUser) -> Result<(), AuthFlowError> {
            let admin = self.require_admin(&input.session_token).await?;
            if admin.id == input.user_id {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage.delete_user_by_id(input.user_id).await?;
            self.record_admin_audit(admin.id, Some(input.user_id), "admin.remove_user")
                .await
        }

        // r[impl auth.admin.has-permission]
        pub async fn admin_has_permission(
            &self,
            input: AdminHasPermission,
        ) -> Result<HasPermissionResult, AuthFlowError> {
            self.require_admin(&input.session_token).await?;
            let allowed = self
                .authorize_organization_action(crate::AuthorizeOrganizationAction {
                    session_token: input.session_token,
                    organization_id: input.organization_id,
                    resource: input.resource,
                    action: input.action,
                })
                .await
                .is_ok();
            Ok(HasPermissionResult { allowed })
        }

        pub(crate) async fn require_admin(
            &self,
            session_token: &str,
        ) -> Result<AuthUser, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: session_token.to_owned(),
                })
                .await?;
            if session.user.role.as_deref() == Some(ADMIN_ROLE) {
                Ok(session.user)
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }

        async fn record_admin_audit(
            &self,
            actor_id: uuid::Uuid,
            target_id: Option<uuid::Uuid>,
            action: &str,
        ) -> Result<(), AuthFlowError> {
            self.storage
                .record_audit_event(AuthAuditEvent {
                    actor_id,
                    target_id,
                    action: action.into(),
                    created_at: Utc::now(),
                })
                .await
        }
    }
}
pub mod api_keys {
    use chrono::Utc;

    use crate::{
        ApiKeyBundle, ArchitectAuth, AuthStorage, AuthenticateApiKey, AuthorizeApiKey,
        CreateApiKey, DeleteApiKey, GetApiKey, ListApiKeys, RevokeApiKey, UpdateApiKey,
        VerifyApiKey,
        commands::CurrentSession,
        crypto::{generate_token, hash_token},
    };
    use auth_proto::{AuthApiKeyCreate, AuthFlowError};

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.apikey.random]
        // r[impl auth.apikey.hash-storage]
        // r[impl auth.apikey.prefix]
        // r[impl auth.apikey.raw-return-once]
        pub async fn create_api_key(
            &self,
            input: CreateApiKey,
        ) -> Result<ApiKeyBundle, AuthFlowError> {
            validate_json(input.permissions_json.as_deref(), "permissions_json")?;
            validate_json(input.metadata_json.as_deref(), "metadata_json")?;
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let key = format!("ak_{token}");
            let prefix = key.chars().take(12).collect::<String>();
            let key_hash = hash_token(&self.config.secret, &key);
            let rate_limit_enabled = input.rate_limit_max.is_some();
            let api_key = self
                .storage
                .create_api_key(AuthApiKeyCreate {
                    name: input.name,
                    prefix: Some(prefix),
                    key_hash,
                    user_id: session.user.id,
                    enabled: true,
                    rate_limit_enabled,
                    rate_limit_time_window: input.rate_limit_time_window,
                    rate_limit_max: input.rate_limit_max,
                    request_count: Some(0),
                    remaining: input.rate_limit_max,
                    expires_at: input.expires_at,
                    permissions_json: input.permissions_json,
                    metadata_json: input.metadata_json,
                })
                .await?;
            Ok(ApiKeyBundle {
                api_key,
                user: session.user,
                key,
            })
        }

        // r[impl auth.apikey.list]
        pub async fn list_api_keys(
            &self,
            input: ListApiKeys,
        ) -> Result<Vec<auth_proto::AuthApiKey>, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage.list_api_keys_by_user_id(session.user.id).await
        }

        // r[impl auth.apikey.get]
        pub async fn get_api_key(
            &self,
            input: GetApiKey,
        ) -> Result<auth_proto::AuthApiKey, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let api_key = self
                .storage
                .find_api_key_by_id(input.api_key_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if api_key.user_id != session.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            Ok(api_key)
        }

        // r[impl auth.apikey.update]
        pub async fn update_api_key(
            &self,
            input: UpdateApiKey,
        ) -> Result<auth_proto::AuthApiKey, AuthFlowError> {
            validate_json(input.permissions_json.as_deref(), "permissions_json")?;
            validate_json(input.metadata_json.as_deref(), "metadata_json")?;
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let api_key = self
                .storage
                .find_api_key_by_id(input.api_key_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if api_key.user_id != session.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage
                .update_api_key(
                    input.api_key_id,
                    input.name,
                    input.enabled,
                    input.expires_at,
                    input.permissions_json,
                    input.rate_limit_time_window,
                    input.rate_limit_max,
                    input.metadata_json,
                )
                .await
        }

        // r[impl auth.apikey.disabled]
        // r[impl auth.apikey.expired]
        // r[impl auth.apikey.rate-limit]
        pub async fn authenticate_api_key(
            &self,
            input: AuthenticateApiKey,
        ) -> Result<ApiKeyBundle, AuthFlowError> {
            let key_hash = hash_token(&self.config.secret, &input.key);
            let api_key = self
                .storage
                .find_api_key_by_hash(&key_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if !api_key.enabled {
                return Err(AuthFlowError::InvalidCredentials);
            }
            if api_key
                .expires_at
                .is_some_and(|expires_at| expires_at <= Utc::now())
            {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let api_key = self.apply_api_key_rate_limit(api_key).await?;
            let user = self
                .storage
                .find_user_by_id(api_key.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            Ok(ApiKeyBundle {
                api_key,
                user,
                key: input.key,
            })
        }

        // r[impl auth.apikey.verify]
        pub async fn verify_api_key(
            &self,
            input: VerifyApiKey,
        ) -> Result<ApiKeyBundle, AuthFlowError> {
            if let Some(permission) = input.permission {
                self.authorize_api_key(AuthorizeApiKey {
                    key: input.key,
                    permission,
                })
                .await
            } else {
                self.authenticate_api_key(AuthenticateApiKey { key: input.key })
                    .await
            }
        }

        // r[impl auth.apikey.permissions]
        pub async fn authorize_api_key(
            &self,
            input: AuthorizeApiKey,
        ) -> Result<ApiKeyBundle, AuthFlowError> {
            let bundle = self
                .authenticate_api_key(AuthenticateApiKey {
                    key: input.key.clone(),
                })
                .await?;
            if permission_grants(
                bundle.api_key.permissions_json.as_deref(),
                input.permission.as_str(),
            )? {
                Ok(bundle)
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }

        // r[impl auth.apikey.revoke]
        pub async fn revoke_api_key(&self, input: RevokeApiKey) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let Some(api_key) = self.storage.find_api_key_by_id(input.api_key_id).await? else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            if api_key.user_id != session.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage
                .set_api_key_enabled(input.api_key_id, false)
                .await
        }

        // r[impl auth.apikey.delete]
        pub async fn delete_api_key(&self, input: DeleteApiKey) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let Some(api_key) = self.storage.find_api_key_by_id(input.api_key_id).await? else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            if api_key.user_id != session.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage.delete_api_key(input.api_key_id).await
        }

        async fn apply_api_key_rate_limit(
            &self,
            mut api_key: auth_proto::AuthApiKey,
        ) -> Result<auth_proto::AuthApiKey, AuthFlowError> {
            if !api_key.rate_limit_enabled {
                return Ok(api_key);
            }
            let remaining = api_key
                .remaining
                .unwrap_or(api_key.rate_limit_max.unwrap_or(0));
            if remaining <= 0 {
                return Err(AuthFlowError::PermissionDenied);
            }
            let next_count = api_key.request_count.unwrap_or(0) + 1;
            let next_remaining = remaining - 1;
            self.storage
                .update_api_key_usage(api_key.id, Some(next_count), Some(next_remaining))
                .await?;
            api_key.request_count = Some(next_count);
            api_key.remaining = Some(next_remaining);
            Ok(api_key)
        }
    }

    fn validate_json(input: Option<&str>, field: &str) -> Result<(), AuthFlowError> {
        if let Some(input) = input {
            serde_json::from_str::<serde_json::Value>(input)
                .map_err(|_| AuthFlowError::InvalidInput(format!("{field} must be JSON")))?;
        }
        Ok(())
    }

    fn permission_grants(
        permissions_json: Option<&str>,
        permission: &str,
    ) -> Result<bool, AuthFlowError> {
        let Some(permissions_json) = permissions_json else {
            return Ok(false);
        };
        let Some((resource, action)) = permission.split_once(':') else {
            return Ok(false);
        };
        let permissions = serde_json::from_str::<serde_json::Value>(permissions_json)
            .map_err(|_| AuthFlowError::InvalidInput("permissions_json must be JSON".into()))?;
        let Some(grant) = permissions.get(resource) else {
            return Ok(false);
        };
        if grant == "*" || grant == action {
            return Ok(true);
        }
        Ok(grant
            .as_array()
            .is_some_and(|actions| actions.iter().any(|value| value == "*" || value == action)))
    }
}
pub mod bearer_tokens {
    use auth_proto::AuthFlowError;

    use crate::{
        ArchitectAuth, AuthStorage, AuthenticateApiKey, AuthenticateBearerToken, BearerTokenBundle,
        BearerTokenStrategy, CurrentSession,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.bearer.parse]
        // r[impl auth.bearer.session]
        // r[impl auth.bearer.api-key]
        // r[impl auth.bearer.errors]
        pub async fn authenticate_bearer_token(
            &self,
            input: AuthenticateBearerToken,
        ) -> Result<BearerTokenBundle, AuthFlowError> {
            let token = parse_authorization_header(input.authorization_header.as_deref())?;
            match self
                .current_session(CurrentSession {
                    token: token.clone(),
                })
                .await
            {
                Ok(bundle) => {
                    return Ok(BearerTokenBundle {
                        user: bundle.user,
                        token,
                        strategy: BearerTokenStrategy::Session,
                        session: Some(bundle.session),
                        api_key: None,
                    });
                }
                Err(AuthFlowError::InvalidCredentials) => {}
                Err(error) => return Err(error),
            }

            let bundle = self
                .authenticate_api_key(AuthenticateApiKey { key: token.clone() })
                .await?;
            Ok(BearerTokenBundle {
                user: bundle.user,
                token,
                strategy: BearerTokenStrategy::ApiKey,
                session: None,
                api_key: Some(bundle.api_key),
            })
        }
    }

    pub(crate) fn parse_authorization_header(
        authorization_header: Option<&str>,
    ) -> Result<String, AuthFlowError> {
        let Some(header) = authorization_header else {
            return Err(AuthFlowError::InvalidCredentials);
        };
        let Some(token) = header.strip_prefix("Bearer ") else {
            return Err(AuthFlowError::InvalidInput(
                "authorization header must use Bearer scheme".into(),
            ));
        };
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(AuthFlowError::InvalidInput(
                "bearer token is malformed".into(),
            ));
        }
        Ok(token.to_owned())
    }
}
pub mod captcha {
    use auth_proto::AuthFlowError;

    use crate::{
        ArchitectAuth, AuthStorage, CaptchaVerification, VerifyCaptcha,
        config::{CaptchaFlow, CaptchaProvider},
    };

    const CAPTCHA_METADATA_KEY: &str = "_captcha_token";

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.captcha.verify]
        // r[impl auth.captcha.providers]
        pub async fn verify_captcha(
            &self,
            input: VerifyCaptcha,
        ) -> Result<CaptchaVerification, AuthFlowError> {
            self.verify_captcha_for_flow(input.flow, input.token.as_deref())
                .await?;
            Ok(CaptchaVerification {
                flow: input.flow,
                verified: true,
            })
        }

        pub(crate) async fn verify_captcha_for_flow(
            &self,
            flow: CaptchaFlow,
            token: Option<&str>,
        ) -> Result<(), AuthFlowError> {
            if !self.config.captcha.protected_flows.contains(&flow) {
                return Ok(());
            }
            match &self.config.captcha.provider {
                CaptchaProvider::Disabled => Ok(()),
                CaptchaProvider::Bypass => Ok(()),
                CaptchaProvider::Test { valid_token } => {
                    if token == Some(valid_token.as_str()) {
                        Ok(())
                    } else {
                        Err(AuthFlowError::PermissionDenied)
                    }
                }
                CaptchaProvider::FailClosed => Err(AuthFlowError::PermissionDenied),
            }
        }
    }

    pub(crate) fn extract_captcha_token_from_metadata(
        metadata_json: Option<&str>,
    ) -> Result<(Option<String>, Option<String>), AuthFlowError> {
        let Some(metadata_json) = metadata_json else {
            return Ok((None, None));
        };
        let mut metadata = serde_json::from_str::<serde_json::Value>(metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        let serde_json::Value::Object(object) = &mut metadata else {
            return Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            ));
        };
        let token = object
            .remove(CAPTCHA_METADATA_KEY)
            .and_then(|value| value.as_str().map(str::to_owned));
        let cleaned = serde_json::to_string(&metadata)
            .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
        Ok((token, Some(cleaned)))
    }
}
pub mod last_login_method {
    use auth_proto::{AuthFlowError, AuthSessionBundle, AuthUser};
    use serde_json::{Value, json};

    use crate::{
        ArchitectAuth, AuthStorage, ClearLastLoginMethod, CurrentSession, GetLastLoginMethod,
        LastLoginMethod, LastLoginMethodCookieConfig,
    };

    pub const LAST_LOGIN_METHOD_METADATA_KEY: &str = "last_login_method";
    pub const DEFAULT_LAST_LOGIN_METHOD_COOKIE_NAME: &str = "better-auth.last_used_login_method";
    pub const DEFAULT_LAST_LOGIN_METHOD_MAX_AGE_SECONDS: i64 = 60 * 60 * 24 * 30;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.lastlogin.query]
        // r[impl auth.lastlogin.cookie-config]
        pub async fn get_last_login_method(
            &self,
            input: GetLastLoginMethod,
        ) -> Result<LastLoginMethod, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            Ok(LastLoginMethod {
                method: last_login_method_from_user(&bundle.user)?,
                cookie_name: DEFAULT_LAST_LOGIN_METHOD_COOKIE_NAME.into(),
                max_age_seconds: DEFAULT_LAST_LOGIN_METHOD_MAX_AGE_SECONDS,
            })
        }

        // r[impl auth.lastlogin.clear]
        pub async fn clear_last_login_method(
            &self,
            input: ClearLastLoginMethod,
        ) -> Result<LastLoginMethod, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let user = write_last_login_method(self, bundle.user, None).await?;
            Ok(LastLoginMethod {
                method: last_login_method_from_user(&user)?,
                cookie_name: DEFAULT_LAST_LOGIN_METHOD_COOKIE_NAME.into(),
                max_age_seconds: DEFAULT_LAST_LOGIN_METHOD_MAX_AGE_SECONDS,
            })
        }
    }

    pub fn default_cookie_config() -> LastLoginMethodCookieConfig {
        LastLoginMethodCookieConfig {
            name: DEFAULT_LAST_LOGIN_METHOD_COOKIE_NAME.into(),
            max_age_seconds: DEFAULT_LAST_LOGIN_METHOD_MAX_AGE_SECONDS,
            http_only: false,
        }
    }

    pub async fn record_last_login_method<S>(
        auth: &ArchitectAuth<S>,
        bundle: AuthSessionBundle,
        method: impl Into<String>,
    ) -> Result<AuthSessionBundle, AuthFlowError>
    where
        S: AuthStorage,
    {
        let user = write_last_login_method(auth, bundle.user, Some(method.into())).await?;
        Ok(AuthSessionBundle { user, ..bundle })
    }

    pub fn last_login_method_from_user(user: &AuthUser) -> Result<Option<String>, AuthFlowError> {
        let metadata = serde_json::from_str::<Value>(&user.metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        Ok(metadata
            .get(LAST_LOGIN_METHOD_METADATA_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    async fn write_last_login_method<S>(
        auth: &ArchitectAuth<S>,
        user: AuthUser,
        method: Option<String>,
    ) -> Result<AuthUser, AuthFlowError>
    where
        S: AuthStorage,
    {
        let mut metadata = serde_json::from_str::<Value>(&user.metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        if metadata.is_null() {
            metadata = json!({});
        }
        let Value::Object(object) = &mut metadata else {
            return Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            ));
        };
        match method {
            Some(method) => {
                object.insert(LAST_LOGIN_METHOD_METADATA_KEY.into(), Value::String(method));
            }
            None => {
                object.remove(LAST_LOGIN_METHOD_METADATA_KEY);
            }
        }
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
        auth.storage
            .update_user_profile(
                user.id,
                user.name,
                user.username,
                user.display_username,
                user.image,
                metadata_json,
            )
            .await
    }
}
pub mod username {
    use auth_proto::{AuthFlowError, AuthSessionBundle, AuthUser};

    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, SignInUsername, UpdateProfile, UpdateUsername,
        crypto::verify_password, flows::last_login_method::record_last_login_method,
    };

    const MIN_USERNAME_LEN: usize = 3;
    const MAX_USERNAME_LEN: usize = 32;
    const RESERVED_USERNAMES: &[&str] = &[
        "admin", "api", "auth", "root", "security", "support", "system", "www",
    ];

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.username.signin]
        // r[impl auth.username.case-insensitive]
        pub async fn sign_in_username(
            &self,
            input: SignInUsername,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let username = normalize_username(&input.username)?;
            let Some(user) = self.storage.find_user_by_username(&username).await? else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            if user.banned
                && user
                    .ban_expires
                    .is_none_or(|expires| expires > chrono::Utc::now())
            {
                return Err(AuthFlowError::PermissionDenied);
            }
            if self.config.require_email_verification && !user.email_verified {
                return Err(AuthFlowError::VerificationRequired);
            }
            let Some(account) = self
                .storage
                .find_password_account_by_user_id(user.id)
                .await?
            else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let Some(password_hash) = account.password_hash else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let password_ok = verify_password(&input.password, &password_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            if !password_ok {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let bundle = self
                .issue_session_with_state(
                    user.clone(),
                    input.ip_address,
                    input.user_agent,
                    None,
                    None,
                    !user.two_factor_enabled,
                )
                .await?;
            record_last_login_method(self, bundle, "username").await
        }

        // r[impl auth.username.update]
        // r[impl auth.username.unique]
        // r[impl auth.username.reserved]
        // r[impl auth.username.validation]
        /// Set the session owner's display name / avatar.
        ///
        /// Username, display_username and metadata are read back from
        /// the session's user and written unchanged: the storage
        /// primitive takes the whole profile row, so omitting them
        /// would silently blank an identifier this flow has no
        /// business touching.
        pub async fn update_profile(
            &self,
            input: UpdateProfile,
        ) -> Result<AuthUser, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            // `Some("")` clears; `None` leaves the current value.
            let merge = |new: Option<String>, current: Option<String>| match new {
                Some(v) if v.trim().is_empty() => None,
                Some(v) => Some(v),
                None => current,
            };
            self.storage
                .update_user_profile(
                    bundle.user.id,
                    merge(input.name, bundle.user.name),
                    bundle.user.username,
                    bundle.user.display_username,
                    merge(input.image, bundle.user.image),
                    bundle.user.metadata_json,
                )
                .await
        }

        pub async fn update_username(
            &self,
            input: UpdateUsername,
        ) -> Result<AuthUser, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let username = normalize_username(&input.username)?;
            if let Some(existing) = self.storage.find_user_by_username(&username).await?
                && existing.id != bundle.user.id
            {
                return Err(AuthFlowError::InvalidInput(
                    "username already exists".into(),
                ));
            }
            self.storage
                .update_user_profile(
                    bundle.user.id,
                    bundle.user.name,
                    Some(username),
                    Some(input.display_username.unwrap_or(input.username)),
                    bundle.user.image,
                    bundle.user.metadata_json,
                )
                .await
        }
    }

    pub fn normalize_optional_username(
        username: Option<String>,
    ) -> Result<(Option<String>, Option<String>), AuthFlowError> {
        match username {
            Some(username) => {
                let canonical = normalize_username(&username)?;
                Ok((Some(canonical), Some(username)))
            }
            None => Ok((None, None)),
        }
    }

    pub fn normalize_username(username: &str) -> Result<String, AuthFlowError> {
        let trimmed = username.trim();
        if trimmed.len() < MIN_USERNAME_LEN || trimmed.len() > MAX_USERNAME_LEN {
            return Err(AuthFlowError::InvalidInput(
                "username must be 3 to 32 characters".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(AuthFlowError::InvalidInput(
                "username may only contain letters, numbers, and underscore".into(),
            ));
        }
        let canonical = trimmed.to_ascii_lowercase();
        if RESERVED_USERNAMES.contains(&canonical.as_str()) {
            return Err(AuthFlowError::InvalidInput("username is reserved".into()));
        }
        Ok(canonical)
    }
}
pub mod custom_session {
    use auth_proto::AuthFlowError;

    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, CustomSessionBundle, CustomSessionEnricher,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.custom-session.typed-hook]
        // r[impl auth.custom-session.backcompat]
        pub async fn current_custom_session<E>(
            &self,
            input: CurrentSession,
            enricher: &E,
        ) -> Result<CustomSessionBundle<E::Output>, AuthFlowError>
        where
            E: CustomSessionEnricher,
        {
            let bundle = self.current_session(input).await?;
            let custom = enricher.enrich_session(&bundle).await?;
            Ok(CustomSessionBundle::from_session_bundle(bundle, custom))
        }
    }
}
pub mod additional_fields {
    use auth_proto::{AuthFlowError, AuthUser};
    use serde_json::{Map, Value};

    use crate::{
        AdditionalFieldSpec, AdditionalFieldType, AdditionalFieldsConfig, AdditionalFieldsSchema,
        AdditionalFieldsView, ArchitectAuth, AuthStorage,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.additional-fields.schema]
        pub fn additional_fields_schema(
            &self,
            config: &AdditionalFieldsConfig,
        ) -> AdditionalFieldsSchema {
            AdditionalFieldsSchema {
                user: config.user.clone(),
                session: config.session.clone(),
                account: config.account.clone(),
            }
        }
    }

    // r[impl auth.additional-fields.persist]
    pub fn validate_additional_metadata(
        metadata_json: Option<&str>,
        fields: &[AdditionalFieldSpec],
    ) -> Result<(), AuthFlowError> {
        let metadata = metadata_object(metadata_json.unwrap_or("{}"))?;
        for field in fields.iter().filter(|field| field.input) {
            match metadata.get(field.name) {
                Some(value) => validate_field_type(field, value)?,
                None if field.required && field.default_json.is_none() => {
                    return Err(AuthFlowError::InvalidInput(format!(
                        "{} is required",
                        field.name
                    )));
                }
                None => {}
            }
        }
        Ok(())
    }

    // r[impl auth.additional-fields.returned]
    // r[impl auth.additional-fields.hidden-metadata]
    pub fn project_user_additional_fields(
        user: &AuthUser,
        fields: &[AdditionalFieldSpec],
    ) -> Result<AdditionalFieldsView, AuthFlowError> {
        project_metadata_fields(&user.metadata_json, fields)
    }

    // r[impl auth.additional-fields.returned]
    // r[impl auth.additional-fields.hidden-metadata]
    pub fn project_metadata_fields(
        metadata_json: &str,
        fields: &[AdditionalFieldSpec],
    ) -> Result<AdditionalFieldsView, AuthFlowError> {
        let metadata = metadata_object(metadata_json)?;
        let mut projected = Map::new();
        for field in fields.iter().filter(|field| field.returned) {
            if let Some(value) = metadata.get(field.name) {
                validate_field_type(field, value)?;
                projected.insert(field.name.into(), value.clone());
            } else if let Some(default_json) = field.default_json {
                let value = serde_json::from_str::<Value>(default_json)
                    .map_err(|_| AuthFlowError::InvalidInput("default_json must be JSON".into()))?;
                validate_field_type(field, &value)?;
                projected.insert(field.name.into(), value);
            }
        }
        Ok(AdditionalFieldsView {
            fields: Value::Object(projected),
        })
    }

    fn metadata_object(metadata_json: &str) -> Result<Map<String, Value>, AuthFlowError> {
        let value = serde_json::from_str::<Value>(metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        match value {
            Value::Object(object) => Ok(object),
            _ => Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            )),
        }
    }

    fn validate_field_type(
        field: &AdditionalFieldSpec,
        value: &Value,
    ) -> Result<(), AuthFlowError> {
        let valid = match field.field_type {
            AdditionalFieldType::String => value.is_string(),
            AdditionalFieldType::Number => value.is_number(),
            AdditionalFieldType::Boolean => value.is_boolean(),
            AdditionalFieldType::Json => true,
        };
        if valid {
            Ok(())
        } else {
            Err(AuthFlowError::InvalidInput(format!(
                "{} has invalid type",
                field.name
            )))
        }
    }
}
pub mod email_password {
    use auth_proto::{
        AuthAccountCreate, AuthFlowError, AuthSessionBundle, AuthSessionCreate, AuthUserCreate,
    };
    use chrono::{Duration, Utc};

    use crate::{
        ArchitectAuth, AuthService, AuthStorage, ChangeEmail, ChangePassword,
        CompletePasswordReset, CreateEmailPasswordUser, DeleteUser, ListAccounts, ListSessions,
        MigrateUserEmail, RefreshSession, RequestEmailVerification, RequestPasswordReset,
        RevokeOtherSessions, RevokeSession, SignInEmailPassword, UpdateProfile, VerificationToken,
        VerifyEmail,
        commands::{CurrentSession, SignOut},
        config::CaptchaFlow,
        crypto::{generate_token, hash_password, hash_token, verify_password},
        flows::{
            captcha::extract_captcha_token_from_metadata,
            last_login_method::record_last_login_method, username::normalize_optional_username,
        },
    };

    pub(crate) const PASSWORD_PROVIDER_ID: &str = "credential";
    const PASSWORD_RESET_TTL_SECONDS: i64 = 60 * 60;
    const EMAIL_VERIFICATION_TTL_SECONDS: i64 = 60 * 60 * 24;
    const EMAIL_VERIFICATION_RESEND_SECONDS: i64 = 60;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.core.server-authoritative]
        // r[impl auth.email.signup.enabled]
        // r[impl auth.email.signup.disabled]
        // r[impl auth.email.email-normalization]
        // r[impl auth.email.email-unique]
        // r[impl auth.email.password-hash]
        // r[impl auth.email.password-never-returned]
        // r[impl auth.password.strength-policy]
        pub async fn create_email_password_user(
            &self,
            input: CreateEmailPasswordUser,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            if !self.config.email_password_enabled {
                return Err(AuthFlowError::PermissionDenied);
            }
            let canonical_email = normalize_email(&input.email)?;
            validate_password_strength(&input.password)?;
            self.reject_breached_password(&input.password).await?;
            let (captcha_token, metadata_json) =
                extract_captcha_token_from_metadata(input.metadata_json.as_deref())?;
            self.verify_captcha_for_flow(CaptchaFlow::SignUp, captcha_token.as_deref())
                .await?;
            validate_metadata(metadata_json.as_deref())?;
            let (username, display_username) = normalize_optional_username(input.username)?;
            if let Some(username) = username.as_deref()
                && self
                    .storage
                    .find_user_by_username(username)
                    .await?
                    .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "username already exists".into(),
                ));
            }

            if self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput("email already exists".into()));
            }

            let password_hash = hash_password(&input.password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let token_hash = hash_token(&self.config.secret, &token);
            let (user, session) = self
                .storage
                .create_user_account_session(
                    AuthUserCreate {
                        email: Some(canonical_email.clone()),
                        name: input.name,
                        email_verified: false,
                        image: input.image,
                        username,
                        display_username,
                        two_factor_enabled: false,
                        role: None,
                        banned: false,
                        ban_reason: None,
                        ban_expires: None,
                        metadata_json: metadata_json.unwrap_or_else(|| "{}".into()),
                    },
                    AuthAccountCreate {
                        account_id: canonical_email,
                        provider_id: PASSWORD_PROVIDER_ID.into(),
                        user_id: uuid::Uuid::nil(),
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: None,
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: None,
                        password_hash: Some(password_hash),
                    },
                    AuthSessionCreate {
                        user_id: uuid::Uuid::nil(),
                        token_hash,
                        expires_at: Utc::now() + Duration::seconds(self.config.session_ttl_seconds),
                        ip_address: input.ip_address,
                        user_agent: input.user_agent,
                        impersonated_by: None,
                        active_organization_id: None,
                        active: true,
                    },
                )
                .await?;

            record_last_login_method(
                self,
                AuthSessionBundle {
                    user,
                    session,
                    token,
                },
                "email",
            )
            .await
        }

        // r[impl auth.email.signin.invalid-generic]
        // r[impl auth.email.signin.banned]
        // r[impl auth.email.signin.verification-required]
        /// Check an email/password pair and return the user it belongs
        /// to — **without issuing a session**.
        ///
        /// This is the whole of [`Self::sign_in_email_password`]'s
        /// verification (enabled-check, normalization, ban, verification
        /// requirement, password hash) minus the session it mints,
        /// factored out for callers that authenticate *per request*
        /// rather than per login: plain-HTTP surfaces where the client
        /// re-presents the same credential every time and has nowhere to
        /// keep a token. HTTP Basic is the case that motivated it (the
        /// Files WebDAV bridge — Finder re-sends `Authorization: Basic`
        /// on every `PROPFIND`).
        ///
        /// Minting a session per such request grows the session table
        /// without bound and leaves live credentials nobody will ever
        /// present again; caching the minted session instead means a
        /// rotated password keeps working until the cache expires,
        /// because the *session* is what gets re-checked rather than the
        /// password. Verifying directly avoids both — every request is
        /// checked against the current stored hash, and no session
        /// exists to leak or to sweep (FastTrackStudio PR #287 review).
        ///
        /// Emits no audit event and does not touch `last_login_method`:
        /// this is not a login.
        pub async fn verify_email_password(
            &self,
            email: &str,
            password: &str,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            if !self.config.email_password_enabled {
                return Err(AuthFlowError::PermissionDenied);
            }
            let canonical_email = normalize_email(email)?;
            let Some(user) = self.storage.find_user_by_email(&canonical_email).await? else {
                return Err(AuthFlowError::InvalidCredentials);
            };

            if user.banned && user.ban_expires.is_none_or(|expires| expires > Utc::now()) {
                return Err(AuthFlowError::PermissionDenied);
            }
            if self.config.require_email_verification && !user.email_verified {
                return Err(AuthFlowError::VerificationRequired);
            }

            let Some(account) = self
                .storage
                .find_password_account_by_user_id(user.id)
                .await?
            else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let Some(password_hash) = account.password_hash else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let password_ok = verify_password(password, &password_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            if !password_ok {
                return Err(AuthFlowError::InvalidCredentials);
            }
            Ok(user)
        }

        // r[impl auth.email.signin.success]
        pub async fn sign_in_email_password(
            &self,
            input: SignInEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let user = self
                .verify_email_password(&input.email, &input.password)
                .await?;

            let bundle = self
                .issue_session_with_state(
                    user.clone(),
                    input.ip_address,
                    input.user_agent,
                    None,
                    None,
                    !user.two_factor_enabled,
                )
                .await?;
            record_last_login_method(self, bundle, "email").await
        }

        // r[impl auth.sessions.current.valid]
        // r[impl auth.sessions.current.missing]
        // r[impl auth.sessions.current.expired]
        pub async fn current_session(
            &self,
            input: CurrentSession,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let token_hash = hash_token(&self.config.secret, &input.token);
            let Some(session) = self.storage.find_session_by_token_hash(&token_hash).await? else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            if !session.active || session.expires_at <= Utc::now() {
                return Err(AuthFlowError::SessionExpired);
            }
            let user = self
                .storage
                .find_user_by_id(session.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;

            Ok(AuthSessionBundle {
                user,
                session,
                token: input.token,
            })
        }

        // r[impl auth.sessions.signout]
        // r[impl auth.sessions.signout-idempotence]
        pub async fn sign_out(&self, input: SignOut) -> Result<(), AuthFlowError> {
            let token_hash = hash_token(&self.config.secret, &input.token);
            self.storage
                .deactivate_session_by_token_hash(&token_hash)
                .await
        }

        // r[impl auth.sessions.refresh]
        // r[impl auth.sessions.refresh-rotation]
        // r[impl auth.sessions.refresh-invalid]
        pub async fn refresh_session(
            &self,
            input: RefreshSession,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession { token: input.token })
                .await?;
            // Issue the replacement before deactivating the old session
            // — a crash in between leaves an extra live session, never
            // a locked-out user.
            let refreshed = self
                .issue_session(
                    bundle.user,
                    bundle.session.ip_address.clone(),
                    bundle.session.user_agent.clone(),
                    bundle.session.impersonated_by,
                    bundle.session.active_organization_id,
                )
                .await?;
            self.storage
                .deactivate_session_by_id(bundle.session.id)
                .await?;
            Ok(refreshed)
        }

        // r[impl auth.sessions.list]
        pub async fn list_sessions(
            &self,
            input: ListSessions,
        ) -> Result<Vec<auth_proto::AuthSession>, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage.list_sessions_by_user_id(bundle.user.id).await
        }

        // r[impl auth.sessions.revoke]
        pub async fn revoke_session(&self, input: RevokeSession) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let sessions = self
                .storage
                .list_sessions_by_user_id(bundle.user.id)
                .await?;
            let owns_session = sessions
                .iter()
                .any(|session| session.id == input.session_id && session.user_id == bundle.user.id);
            if !owns_session {
                return Err(AuthFlowError::PermissionDenied);
            }
            self.storage
                .deactivate_session_by_id(input.session_id)
                .await
        }

        // r[impl auth.sessions.revoke-other]
        pub async fn revoke_other_sessions(
            &self,
            input: RevokeOtherSessions,
        ) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage
                .deactivate_other_sessions_by_user_id(bundle.user.id, bundle.session.id)
                .await
        }

        // r[impl auth.password.change.requires-current]
        // r[impl auth.password.change-invalidates]
        // r[impl auth.email.password-hash]
        // r[impl auth.password.strength-policy]
        pub async fn change_password(&self, input: ChangePassword) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            validate_password_strength(&input.new_password)?;
            self.reject_breached_password(&input.new_password).await?;
            let Some(account) = self
                .storage
                .find_password_account_by_user_id(bundle.user.id)
                .await?
            else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let Some(existing_hash) = account.password_hash else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let current_ok = verify_password(&input.current_password, &existing_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            if !current_ok {
                return Err(AuthFlowError::InvalidCredentials);
            }

            let new_hash = hash_password(&input.new_password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage
                .update_password_hash(bundle.user.id, new_hash)
                .await?;
            self.storage
                .deactivate_sessions_by_user_id(bundle.user.id)
                .await
        }

        // r[impl auth.password.reset-token-random]
        // r[impl auth.password.reset-token-hash]
        // r[impl auth.password.reset-expiry]
        // r[impl auth.password.reset-generic-response]
        pub async fn request_password_reset(
            &self,
            input: RequestPasswordReset,
        ) -> Result<VerificationToken, AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            let identifier = password_reset_identifier(&canonical_email);
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;

            if self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .is_some()
            {
                self.storage
                    .create_verification(auth_proto::AuthVerificationCreate {
                        identifier: identifier.clone(),
                        value_hash: hash_token(&self.config.secret, &token),
                        expires_at: Utc::now() + Duration::seconds(PASSWORD_RESET_TTL_SECONDS),
                    })
                    .await?;
            }

            Ok(VerificationToken { identifier, token })
        }

        // r[impl auth.password.reset-token-hash]
        // r[impl auth.password.reset-expiry]
        // r[impl auth.password.reset-single-use]
        // r[impl auth.email.password-hash]
        // r[impl auth.password.strength-policy]
        pub async fn complete_password_reset(
            &self,
            input: CompletePasswordReset,
        ) -> Result<(), AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            validate_password_strength(&input.new_password)?;
            self.reject_breached_password(&input.new_password).await?;
            let user = self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let identifier = password_reset_identifier(&canonical_email);
            let value_hash = hash_token(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }

            let new_hash = hash_password(&input.new_password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage.update_password_hash(user.id, new_hash).await?;
            self.storage.delete_verification(verification.id).await
        }

        // r[impl auth.verify.token-random]
        // r[impl auth.verify.token-hash]
        // r[impl auth.verify.expiry]
        // r[impl auth.verify.identifier]
        // r[impl auth.verify.resend-throttle]
        pub async fn request_email_verification(
            &self,
            input: RequestEmailVerification,
        ) -> Result<VerificationToken, AuthFlowError> {
            let user = self
                .storage
                .find_user_by_id(input.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let identifier = email_verification_identifier(user.id);
            if let Some(existing) = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                && existing.created_at + Duration::seconds(EMAIL_VERIFICATION_RESEND_SECONDS)
                    > Utc::now()
            {
                return Err(AuthFlowError::PermissionDenied);
            }
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage
                .create_verification(auth_proto::AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &token),
                    expires_at: Utc::now() + Duration::seconds(EMAIL_VERIFICATION_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken { identifier, token })
        }

        // r[impl auth.verify.token-hash]
        // r[impl auth.verify.expiry]
        // r[impl auth.verify.single-use]
        // r[impl auth.verify.success]
        pub async fn verify_email(&self, input: VerifyEmail) -> Result<(), AuthFlowError> {
            let identifier = email_verification_identifier(input.user_id);
            let value_hash = hash_token(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.storage.mark_email_verified(input.user_id).await?;
            self.storage.delete_verification(verification.id).await
        }

        // r[impl auth.verify.change-email]
        pub async fn change_email(
            &self,
            input: ChangeEmail,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let canonical_email = normalize_email(&input.new_email)?;
            if self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput("email already exists".into()));
            }
            let previous_email = bundle.user.email.clone();
            let user = self
                .storage
                .update_user_email(bundle.user.id, canonical_email.clone(), false)
                .await?;
            // Every path that changes an address appends to the trail, so
            // the history is complete rather than "whatever remembered to
            // write". `changed_by: None` = the user changed their own.
            self.storage
                .record_email_change(user.id, previous_email, canonical_email, None, None)
                .await?;
            Ok(user)
        }

        /// Migrate an account onto a different address, keeping the same
        /// user id and appending to the email trail.
        ///
        /// The id staying put is the point: it is what tasks, timers,
        /// sessions and authorship are keyed on, so renaming an address
        /// must not mint a new account. Creating one and abandoning the
        /// old would orphan all of it, which is the failure this exists to
        /// avoid.
        ///
        /// Takes a `user_id` rather than a session, so an operator can run
        /// it for someone who cannot sign in — which is the usual reason a
        /// migration is needed at all. Authorization is the CALLER's job:
        /// this is a storage-level operation, exposed only where the
        /// surface around it enforces who may ask.
        ///
        /// Sessions are deliberately left alone. They key on the user id,
        /// so they survive the rename; a migration is not a credential
        /// change and should not sign anyone out.
        pub async fn migrate_user_email(
            &self,
            input: MigrateUserEmail,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            let canonical_email = normalize_email(&input.new_email)?;
            let user = self
                .storage
                .find_user_by_id(input.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let previous_email = user.email.clone();
            if previous_email.as_deref() == Some(canonical_email.as_str()) {
                // Already there. Not an error — a re-run of a bulk
                // migration should be a no-op, not a failure — but it must
                // not append a row saying nothing changed.
                return Ok(user);
            }
            if let Some(existing) = self.storage.find_user_by_email(&canonical_email).await?
                && existing.id != user.id
            {
                return Err(AuthFlowError::InvalidInput(
                    "another account already uses that email".into(),
                ));
            }
            // Verification resets: the new address has not been proven to
            // belong to anyone yet, and inheriting the old address's
            // verified flag would be a lie the rest of the system trusts.
            let updated = self
                .storage
                .update_user_email(user.id, canonical_email.clone(), false)
                .await?;
            self.storage
                .record_email_change(
                    updated.id,
                    previous_email,
                    canonical_email,
                    input.changed_by,
                    input.reason,
                )
                .await?;
            Ok(updated)
        }

        /// Every address this account has held, oldest first.
        pub async fn list_email_history(
            &self,
            user_id: uuid::Uuid,
        ) -> Result<Vec<auth_proto::email_change::AuthEmailChange>, AuthFlowError> {
            self.storage.list_email_history(user_id).await
        }

        /// Look an account up by its CURRENT address. Sibling of
        /// [`find_user_by_previous_email`](Self::find_user_by_previous_email),
        /// which answers the same question about addresses it has left
        /// behind.
        pub async fn find_user_by_email(
            &self,
            email: &str,
        ) -> Result<Option<auth_proto::AuthUser>, AuthFlowError> {
            let canonical = normalize_email(email)?;
            self.storage.find_user_by_email(&canonical).await
        }

        /// Which account once held `email` — the reverse lookup that makes
        /// a migrated address still resolvable ("who was old@…?").
        pub async fn find_user_by_previous_email(
            &self,
            email: &str,
        ) -> Result<Option<auth_proto::AuthUser>, AuthFlowError> {
            let canonical = normalize_email(email)?;
            let Some(user_id) = self.storage.find_user_id_by_previous_email(&canonical).await?
            else {
                return Ok(None);
            };
            self.storage.find_user_by_id(user_id).await
        }

        // r[impl auth.user.delete]
        pub async fn delete_user(&self, input: DeleteUser) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage.delete_user_by_id(bundle.user.id).await
        }

        // r[impl auth.account.list]
        pub async fn list_accounts(
            &self,
            input: ListAccounts,
        ) -> Result<Vec<auth_proto::AuthAccount>, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage.list_accounts_by_user_id(bundle.user.id).await
        }

        // r[impl auth.sessions.token-random]
        // r[impl auth.sessions.token-hash-storage]
        // r[impl auth.sessions.ttl]
        // r[impl auth.sessions.context]
        // r[impl auth.core.timestamps]
        pub(crate) async fn issue_session(
            &self,
            user: auth_proto::AuthUser,
            ip_address: Option<String>,
            user_agent: Option<String>,
            impersonated_by: Option<uuid::Uuid>,
            active_organization_id: Option<uuid::Uuid>,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            self.issue_session_with_state(
                user,
                ip_address,
                user_agent,
                impersonated_by,
                active_organization_id,
                true,
            )
            .await
        }

        pub(crate) async fn issue_session_with_state(
            &self,
            user: auth_proto::AuthUser,
            ip_address: Option<String>,
            user_agent: Option<String>,
            impersonated_by: Option<uuid::Uuid>,
            active_organization_id: Option<uuid::Uuid>,
            active: bool,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let token_hash = hash_token(&self.config.secret, &token);
            let session = self
                .storage
                .create_session(auth_proto::AuthSessionCreate {
                    user_id: user.id,
                    token_hash,
                    expires_at: Utc::now() + Duration::seconds(self.config.session_ttl_seconds),
                    ip_address,
                    user_agent,
                    impersonated_by,
                    active_organization_id,
                    active,
                })
                .await?;

            Ok(AuthSessionBundle {
                user,
                session,
                token,
            })
        }
    }

    impl<S> AuthService for ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        async fn change_email(
            &self,
            input: auth_proto::service::ChangeEmailRequest,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            ArchitectAuth::change_email(
                self,
                ChangeEmail {
                    session_token: input.session_token,
                    new_email: input.new_email,
                },
            )
            .await
        }

        async fn update_profile(
            &self,
            input: auth_proto::service::UpdateProfileRequest,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            ArchitectAuth::update_profile(
                self,
                UpdateProfile {
                    session_token: input.session_token,
                    name: input.name,
                    image: input.image,
                },
            )
            .await
        }

        async fn change_password(
            &self,
            input: auth_proto::service::ChangePasswordRequest,
        ) -> Result<(), AuthFlowError> {
            ArchitectAuth::change_password(
                self,
                ChangePassword {
                    session_token: input.session_token,
                    current_password: input.current_password,
                    new_password: input.new_password,
                },
            )
            .await
        }

        async fn migrate_user_email(
            &self,
            input: auth_proto::service::MigrateUserEmailRequest,
        ) -> Result<auth_proto::AuthUser, AuthFlowError> {
            // Same contract as the vox transport: the session authorizes
            // the call and names who performed it.
            let caller = ArchitectAuth::current_session(
                self,
                CurrentSession {
                    token: input.session_token,
                },
            )
            .await?;
            ArchitectAuth::migrate_user_email(
                self,
                MigrateUserEmail {
                    user_id: input.user_id,
                    new_email: input.new_email,
                    changed_by: Some(caller.user.id),
                    reason: input.reason,
                },
            )
            .await
        }

        async fn list_email_history(
            &self,
            input: auth_proto::service::EmailHistoryRequest,
        ) -> Result<Vec<auth_proto::email_change::AuthEmailChange>, AuthFlowError> {
            ArchitectAuth::current_session(
                self,
                CurrentSession {
                    token: input.session_token,
                },
            )
            .await?;
            ArchitectAuth::list_email_history(self, input.user_id).await
        }

        async fn sign_up_email_password(
            &self,
            input: auth_proto::SignUpEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            ArchitectAuth::create_email_password_user(
                self,
                CreateEmailPasswordUser {
                    email: input.email,
                    password: input.password,
                    name: input.name,
                    username: input.username,
                    image: input.image,
                    metadata_json: input.metadata_json,
                    ip_address: input.ip_address,
                    user_agent: input.user_agent,
                },
            )
            .await
        }

        async fn sign_in_email_password(
            &self,
            input: SignInEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            ArchitectAuth::sign_in_email_password(self, input).await
        }

        async fn current_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError> {
            ArchitectAuth::current_session(self, CurrentSession { token }).await
        }

        async fn refresh_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError> {
            ArchitectAuth::refresh_session(self, RefreshSession { token }).await
        }

        async fn whoami(&self, token: String) -> Result<auth_proto::AuthUser, AuthFlowError> {
            ArchitectAuth::current_session(self, CurrentSession { token })
                .await
                .map(|bundle| bundle.user)
        }

        async fn sign_out(&self, token: String) -> Result<(), AuthFlowError> {
            ArchitectAuth::sign_out(self, SignOut { token }).await
        }

        async fn list_org_members(
            &self,
            token: String,
        ) -> Result<Vec<auth_proto::OrgMember>, AuthFlowError> {
            self.org_members_for_token(token).await
        }
    }

    pub(crate) fn normalize_email(email: &str) -> Result<String, AuthFlowError> {
        let trimmed = email.trim();
        let Some((local, domain)) = trimmed.split_once('@') else {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        };
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        }
        Ok(format!("{local}@{}", domain.to_ascii_lowercase()))
    }

    // r[impl auth.core.metadata-json]
    pub(crate) fn validate_metadata(metadata_json: Option<&str>) -> Result<(), AuthFlowError> {
        if let Some(metadata_json) = metadata_json {
            serde_json::from_str::<serde_json::Value>(metadata_json)
                .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        }
        Ok(())
    }

    pub(crate) fn validate_password_strength(password: &str) -> Result<(), AuthFlowError> {
        if password.len() >= 8 {
            Ok(())
        } else {
            Err(AuthFlowError::InvalidInput(
                "password must be at least 8 characters".into(),
            ))
        }
    }

    fn password_reset_identifier(canonical_email: &str) -> String {
        format!("password-reset:{canonical_email}")
    }

    fn email_verification_identifier(user_id: uuid::Uuid) -> String {
        format!("email-verification:{user_id}")
    }

    #[cfg(test)]
    mod tests {
        use std::{
            collections::HashMap,
            future::Future,
            pin::Pin,
            sync::{Arc, Mutex},
        };

        use async_trait::async_trait;
        use auth_proto::{
            AuthAccount, AuthAccountCreate, AuthApiKey, AuthApiKeyCreate, AuthFlowError,
            AuthInvitation, AuthInvitationCreate, AuthMember, AuthMemberCreate, AuthOrganization,
            AuthOrganizationCreate, AuthOrganizationRole, AuthOrganizationRoleCreate, AuthPasskey,
            AuthPasskeyCreate, AuthSession, AuthSessionBundle, AuthSessionCreate, AuthTeam,
            AuthTeamCreate, AuthTeamMember, AuthTeamMemberCreate, AuthTwoFactor,
            AuthTwoFactorCreate, AuthUser, AuthUserCreate, AuthVerification,
            AuthVerificationCreate,
        };
        use base64::Engine;
        use chrono::{DateTime, Duration, Utc};
        use sha2::Digest;
        use totp_rs::{Algorithm, Secret, TOTP};
        use uuid::Uuid;

        use crate::{
            AcceptInvitation, AddTeamMember, AdditionalFieldSpec, AdditionalFieldType,
            AdditionalFieldsConfig, AdminCreateUser, AdminHasPermission, AdminSetUserPassword,
            ApproveDeviceCode, ArchitectAuth, AuthAuditEvent, AuthStorage, AuthStorageCapabilities,
            AuthenticateApiKey, AuthenticateBearerToken, AuthorizeApiKey, AuthorizeMcpRequest,
            AuthorizeOidc, AuthorizeOrganizationAction, BanUser, BearerTokenStrategy,
            BeginOAuthAuthorization, BeginOAuthProxyAuthorization, BeginPasskeyAuthentication,
            BeginPasskeyRegistration, BreachedPasswordFailurePolicy, BreachedPasswordProvider,
            CaptchaFlow, ChangeEmail, ChangePassword, CheckPasswordBreach, CleanupAnonymousUsers,
            MigrateUserEmail,
            ClearLastLoginMethod, CompletePasskeyAuthentication, CompletePasskeyRegistration,
            CompletePasswordReset, ConfirmTwoFactor, ConsumeOAuthProxyCallback, CreateApiKey,
            CreateDeviceAuthorization, CreateEmailPasswordUser, CreateInvitation,
            CreateOrganization, CreateOrganizationRole, CreateSiweNonce, CreateTeam,
            CurrentSession, CustomSessionEnricher, DeleteApiKey, DeletePasskey, DeleteTeam,
            DeleteUser, DenyDeviceCode, DisableTwoFactor, ExchangeOidcToken,
            ForwardOAuthProxyCallback, GenerateOneTimeToken, GetApiKey, GetLastLoginMethod,
            GetOAuthAccessToken, GetOidcUserInfo, ImpersonateUser, IssueJwt,
            LinkAnonymousEmailPassword, LinkOAuthAccount, LinkSiweAddress, ListAccounts,
            ListApiKeys, ListDeviceSessions, ListPasskeys, ListSessions, ListTeamMembers,
            ListTeams, ListUserSessions, ListUsers, OidcClientConfig, OneTapCallback,
            PollDeviceToken, RefreshOAuthToken, RegisterOidcClient, RemoveTeamMember, RemoveUser,
            RequestEmailVerification, RequestPasswordReset, RequireOrganizationRole, RevokeApiKey,
            RevokeDeviceSession, RevokeOneTimeToken, RevokeOtherSessions, RevokeSession,
            RevokeUserSession, RevokeUserSessions, SendEmailOtp, SendMagicLink, SendPhoneNumberOtp,
            SetActiveDeviceSession, SetActiveOrganization, SetMemberRole, SetUserRole,
            SignInAnonymous, SignInEmailPassword, SignInOAuthAccount, SignInUsername, SignOut,
            SmsProvider, StartTwoFactorSetup, StopImpersonating, UnbanUser, UnlinkOAuthAccount,
            UpdateApiKey, UpdatePhoneNumber, UpdateTeam, UpdateUsername, VerifyApiKey,
            VerifyCaptcha, VerifyDeviceCode, VerifyEmail, VerifyEmailOtp, VerifyJwt,
            VerifyMagicLink, VerifyOAuthState, VerifyOneTimeToken, VerifyPhoneNumberOtp,
            VerifySiweMessage, VerifyTwoFactor,
        };

        #[derive(Clone, Default)]
        struct MemoryStorage {
            inner: Arc<Mutex<State>>,
        }

        #[derive(Default)]
        struct State {
            users: HashMap<Uuid, AuthUser>,
            audit_events: Vec<AuthAuditEvent>,
            user_ids_by_email: HashMap<String, Uuid>,
            user_ids_by_username: HashMap<String, Uuid>,
            accounts_by_user: HashMap<Uuid, Vec<AuthAccount>>,
            accounts_by_provider: HashMap<(String, String), AuthAccount>,
            sessions_by_token_hash: HashMap<String, AuthSession>,
            verifications: HashMap<Uuid, AuthVerification>,
            api_keys_by_id: HashMap<Uuid, AuthApiKey>,
            api_keys_by_hash: HashMap<String, AuthApiKey>,
            passkeys_by_credential_id: HashMap<String, AuthPasskey>,
            organizations: HashMap<Uuid, AuthOrganization>,
            organization_ids_by_slug: HashMap<String, Uuid>,
            members: HashMap<(Uuid, Uuid), AuthMember>,
            organization_roles: HashMap<(Uuid, String), AuthOrganizationRole>,
            teams: HashMap<Uuid, AuthTeam>,
            team_members: HashMap<(Uuid, Uuid), AuthTeamMember>,
            invitations: HashMap<Uuid, AuthInvitation>,
            two_factors: HashMap<Uuid, AuthTwoFactor>,
            two_factor_attempts: HashMap<Uuid, i64>,
            /// Append-only, in insertion order — mirrors the real store's
            /// "oldest first" ordering without needing timestamps to be
            /// distinct (tests move fast enough to collide on `Utc::now`).
            email_history: Vec<auth_proto::email_change::AuthEmailChange>,
        }

        #[async_trait]
        impl AuthStorage for MemoryStorage {
            fn capabilities(&self) -> AuthStorageCapabilities {
                AuthStorageCapabilities::runtime_owned("memory")
            }

            async fn record_audit_event(&self, event: AuthAuditEvent) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .audit_events
                    .push(event);
                Ok(())
            }

            async fn create_user(&self, input: AuthUserCreate) -> Result<AuthUser, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                if let Some(username) = &input.username
                    && inner.user_ids_by_username.contains_key(username)
                {
                    return Err(AuthFlowError::InvalidInput(
                        "username already exists".into(),
                    ));
                }
                let user = AuthUser {
                    id: Uuid::new_v4(),
                    email: input.email,
                    name: input.name,
                    email_verified: input.email_verified,
                    image: input.image,
                    username: input.username,
                    display_username: input.display_username,
                    two_factor_enabled: input.two_factor_enabled,
                    role: input.role,
                    banned: input.banned,
                    ban_reason: input.ban_reason,
                    ban_expires: input.ban_expires,
                    metadata_json: input.metadata_json,
                    created_at: now,
                    updated_at: now,
                };
                if let Some(email) = &user.email {
                    inner.user_ids_by_email.insert(email.clone(), user.id);
                }
                if let Some(username) = &user.username {
                    inner.user_ids_by_username.insert(username.clone(), user.id);
                }
                inner.users.insert(user.id, user.clone());
                Ok(user)
            }

            async fn find_user_by_email(
                &self,
                canonical_email: &str,
            ) -> Result<Option<AuthUser>, AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                Ok(inner
                    .user_ids_by_email
                    .get(canonical_email)
                    .and_then(|id| inner.users.get(id))
                    .cloned())
            }

            async fn find_user_by_username(
                &self,
                canonical_username: &str,
            ) -> Result<Option<AuthUser>, AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                Ok(inner
                    .user_ids_by_username
                    .get(canonical_username)
                    .and_then(|id| inner.users.get(id))
                    .cloned())
            }

            async fn find_user_by_id(
                &self,
                user_id: Uuid,
            ) -> Result<Option<AuthUser>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .users
                    .get(&user_id)
                    .cloned())
            }

            async fn list_users(
                &self,
                offset: usize,
                limit: usize,
            ) -> Result<(Vec<AuthUser>, usize), AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                let mut users = inner.users.values().cloned().collect::<Vec<_>>();
                users.sort_by_key(|user| user.created_at);
                let total = users.len();
                Ok((users.into_iter().skip(offset).take(limit).collect(), total))
            }

            async fn update_user_role(
                &self,
                user_id: Uuid,
                role: Option<String>,
            ) -> Result<AuthUser, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.role = role;
                user.updated_at = Utc::now();
                Ok(user.clone())
            }

            async fn update_user_ban(
                &self,
                user_id: Uuid,
                banned: bool,
                ban_reason: Option<String>,
                ban_expires: Option<chrono::DateTime<Utc>>,
            ) -> Result<AuthUser, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.banned = banned;
                user.ban_reason = ban_reason;
                user.ban_expires = ban_expires;
                user.updated_at = Utc::now();
                Ok(user.clone())
            }

            async fn update_user_email(
                &self,
                user_id: Uuid,
                email: String,
                email_verified: bool,
            ) -> Result<AuthUser, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let old_email = inner
                    .users
                    .get(&user_id)
                    .and_then(|user| user.email.clone());
                if let Some(existing_id) = inner.user_ids_by_email.get(&email)
                    && *existing_id != user_id
                {
                    return Err(AuthFlowError::InvalidInput("email already exists".into()));
                }
                if let Some(old_email) = old_email {
                    inner.user_ids_by_email.remove(&old_email);
                }
                inner.user_ids_by_email.insert(email.clone(), user_id);
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.email = Some(email);
                user.email_verified = email_verified;
                user.updated_at = Utc::now();
                Ok(user.clone())
            }

            async fn record_email_change(
                &self,
                user_id: Uuid,
                previous_email: Option<String>,
                new_email: String,
                changed_by: Option<Uuid>,
                reason: Option<String>,
            ) -> Result<auth_proto::email_change::AuthEmailChange, AuthFlowError> {
                let record = auth_proto::email_change::AuthEmailChange {
                    id: Uuid::new_v4(),
                    user_id,
                    previous_email,
                    new_email,
                    changed_by,
                    reason,
                    created_at: Utc::now(),
                };
                let mut inner = self.inner.lock().expect("lock memory storage");
                inner.email_history.push(record.clone());
                Ok(record)
            }

            async fn list_email_history(
                &self,
                user_id: Uuid,
            ) -> Result<Vec<auth_proto::email_change::AuthEmailChange>, AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                Ok(inner
                    .email_history
                    .iter()
                    .filter(|r| r.user_id == user_id)
                    .cloned()
                    .collect())
            }

            async fn find_user_id_by_previous_email(
                &self,
                email: &str,
            ) -> Result<Option<Uuid>, AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                // Most recent wins, matching the sea-orm impl's ordering.
                Ok(inner
                    .email_history
                    .iter()
                    .rev()
                    .find(|r| r.previous_email.as_deref() == Some(email))
                    .map(|r| r.user_id))
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
                let mut inner = self.inner.lock().expect("lock memory storage");
                let old_username = inner
                    .users
                    .get(&user_id)
                    .and_then(|user| user.username.clone());
                if let Some(new_username) = &username
                    && let Some(existing_id) = inner.user_ids_by_username.get(new_username)
                    && *existing_id != user_id
                {
                    return Err(AuthFlowError::InvalidInput(
                        "username already exists".into(),
                    ));
                }
                if old_username != username {
                    if let Some(old_username) = old_username {
                        inner.user_ids_by_username.remove(&old_username);
                    }
                    if let Some(new_username) = &username {
                        inner
                            .user_ids_by_username
                            .insert(new_username.clone(), user_id);
                    }
                }
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.name = name;
                user.username = username;
                user.display_username = display_username;
                user.image = image;
                user.metadata_json = metadata_json;
                user.updated_at = Utc::now();
                Ok(user.clone())
            }

            async fn create_account(
                &self,
                input: AuthAccountCreate,
            ) -> Result<AuthAccount, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let account = AuthAccount {
                    id: Uuid::new_v4(),
                    account_id: input.account_id,
                    provider_id: input.provider_id,
                    user_id: input.user_id,
                    access_token_ciphertext: input.access_token_ciphertext,
                    refresh_token_ciphertext: input.refresh_token_ciphertext,
                    id_token_ciphertext: input.id_token_ciphertext,
                    access_token_expires_at: input.access_token_expires_at,
                    refresh_token_expires_at: input.refresh_token_expires_at,
                    scope: input.scope,
                    password_hash: input.password_hash,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .accounts_by_user
                    .entry(account.user_id)
                    .or_default()
                    .push(account.clone());
                inner.accounts_by_provider.insert(
                    (account.provider_id.clone(), account.account_id.clone()),
                    account.clone(),
                );
                Ok(account)
            }

            async fn find_account_by_provider_account(
                &self,
                provider_id: &str,
                account_id: &str,
            ) -> Result<Option<AuthAccount>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .accounts_by_provider
                    .get(&(provider_id.to_owned(), account_id.to_owned()))
                    .cloned())
            }

            async fn find_password_account_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Option<AuthAccount>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .accounts_by_user
                    .get(&user_id)
                    .and_then(|accounts| {
                        accounts
                            .iter()
                            .find(|account| account.provider_id == super::PASSWORD_PROVIDER_ID)
                    })
                    .cloned())
            }

            async fn list_accounts_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Vec<AuthAccount>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .accounts_by_user
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_default())
            }

            async fn delete_user_by_id(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                inner.users.remove(&user_id);
                inner.user_ids_by_email.retain(|_, id| *id != user_id);
                if let Some(accounts) = inner.accounts_by_user.remove(&user_id) {
                    for account in accounts {
                        inner
                            .accounts_by_provider
                            .remove(&(account.provider_id, account.account_id));
                    }
                }
                inner
                    .sessions_by_token_hash
                    .retain(|_, session| session.user_id != user_id);
                inner.api_keys_by_id.retain(|_, key| key.user_id != user_id);
                inner
                    .api_keys_by_hash
                    .retain(|_, key| key.user_id != user_id);
                inner
                    .passkeys_by_credential_id
                    .retain(|_, passkey| passkey.user_id != user_id);
                inner
                    .members
                    .retain(|(_, member_user_id), _| *member_user_id != user_id);
                inner
                    .team_members
                    .retain(|(_, member_user_id), _| *member_user_id != user_id);
                inner.verifications.retain(|_, verification| {
                    !verification.identifier.contains(&user_id.to_string())
                });
                inner.two_factors.remove(&user_id);
                inner.two_factor_attempts.remove(&user_id);
                Ok(())
            }

            async fn delete_account_by_provider_account(
                &self,
                provider_id: &str,
                account_id: &str,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let key = (provider_id.to_owned(), account_id.to_owned());
                let Some(account) = inner.accounts_by_provider.remove(&key) else {
                    return Ok(());
                };
                if let Some(accounts) = inner.accounts_by_user.get_mut(&account.user_id) {
                    accounts.retain(|stored| {
                        stored.provider_id != provider_id || stored.account_id != account_id
                    });
                }
                Ok(())
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
                let mut inner = self.inner.lock().expect("lock memory storage");
                let key = (provider_id.to_owned(), account_id.to_owned());
                let account = inner
                    .accounts_by_provider
                    .get_mut(&key)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                account.access_token_ciphertext = access_token_ciphertext;
                if refresh_token_ciphertext.is_some() {
                    account.refresh_token_ciphertext = refresh_token_ciphertext;
                }
                if id_token_ciphertext.is_some() {
                    account.id_token_ciphertext = id_token_ciphertext;
                }
                account.access_token_expires_at = access_token_expires_at;
                if refresh_token_expires_at.is_some() {
                    account.refresh_token_expires_at = refresh_token_expires_at;
                }
                if scope.is_some() {
                    account.scope = scope;
                }
                account.updated_at = Utc::now();
                let updated = account.clone();
                if let Some(accounts) = inner.accounts_by_user.get_mut(&updated.user_id)
                    && let Some(account) = accounts.iter_mut().find(|account| {
                        account.provider_id == provider_id && account.account_id == account_id
                    })
                {
                    *account = updated.clone();
                }
                Ok(updated)
            }

            async fn update_password_hash(
                &self,
                user_id: Uuid,
                password_hash: String,
            ) -> Result<(), AuthFlowError> {
                if let Some(account) = self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .accounts_by_user
                    .get_mut(&user_id)
                    .and_then(|accounts| {
                        accounts
                            .iter_mut()
                            .find(|account| account.provider_id == super::PASSWORD_PROVIDER_ID)
                    })
                {
                    account.password_hash = Some(password_hash);
                    account.updated_at = Utc::now();
                    Ok(())
                } else {
                    Err(AuthFlowError::InvalidCredentials)
                }
            }

            async fn create_session(
                &self,
                input: AuthSessionCreate,
            ) -> Result<AuthSession, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let session = AuthSession {
                    id: Uuid::new_v4(),
                    user_id: input.user_id,
                    token_hash: input.token_hash,
                    expires_at: input.expires_at,
                    ip_address: input.ip_address,
                    user_agent: input.user_agent,
                    impersonated_by: input.impersonated_by,
                    active_organization_id: input.active_organization_id,
                    active: input.active,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .sessions_by_token_hash
                    .insert(session.token_hash.clone(), session.clone());
                Ok(session)
            }

            async fn find_session_by_token_hash(
                &self,
                token_hash: &str,
            ) -> Result<Option<AuthSession>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .sessions_by_token_hash
                    .get(token_hash)
                    .cloned())
            }

            async fn list_sessions_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Vec<AuthSession>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .sessions_by_token_hash
                    .values()
                    .filter(|session| session.user_id == user_id)
                    .cloned()
                    .collect())
            }

            async fn deactivate_session_by_token_hash(
                &self,
                token_hash: &str,
            ) -> Result<(), AuthFlowError> {
                if let Some(session) = self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .sessions_by_token_hash
                    .get_mut(token_hash)
                {
                    session.active = false;
                    session.updated_at = Utc::now();
                }
                Ok(())
            }

            async fn deactivate_session_by_id(
                &self,
                session_id: Uuid,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                for session in inner.sessions_by_token_hash.values_mut() {
                    if session.id == session_id {
                        session.active = false;
                        session.updated_at = Utc::now();
                    }
                }
                Ok(())
            }

            async fn activate_session_by_token_hash(
                &self,
                token_hash: &str,
            ) -> Result<(), AuthFlowError> {
                if let Some(session) = self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .sessions_by_token_hash
                    .get_mut(token_hash)
                {
                    session.active = true;
                    session.updated_at = Utc::now();
                }
                Ok(())
            }

            async fn deactivate_other_sessions_by_user_id(
                &self,
                user_id: Uuid,
                except_session_id: Uuid,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                for session in inner.sessions_by_token_hash.values_mut() {
                    if session.user_id == user_id && session.id != except_session_id {
                        session.active = false;
                        session.updated_at = Utc::now();
                    }
                }
                Ok(())
            }

            async fn deactivate_sessions_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                for session in inner.sessions_by_token_hash.values_mut() {
                    if session.user_id == user_id {
                        session.active = false;
                        session.updated_at = Utc::now();
                    }
                }
                Ok(())
            }

            async fn create_verification(
                &self,
                input: AuthVerificationCreate,
            ) -> Result<AuthVerification, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let verification = AuthVerification {
                    id: Uuid::new_v4(),
                    identifier: input.identifier,
                    value_hash: input.value_hash,
                    expires_at: input.expires_at,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .verifications
                    .insert(verification.id, verification.clone());
                Ok(verification)
            }

            async fn find_verification(
                &self,
                identifier: &str,
                value_hash: &str,
            ) -> Result<Option<AuthVerification>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .verifications
                    .values()
                    .find(|verification| {
                        verification.identifier == identifier
                            && verification.value_hash == value_hash
                    })
                    .cloned())
            }

            async fn find_latest_verification_by_identifier(
                &self,
                identifier: &str,
            ) -> Result<Option<AuthVerification>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .verifications
                    .values()
                    .filter(|verification| verification.identifier == identifier)
                    .max_by_key(|verification| verification.created_at)
                    .cloned())
            }

            async fn delete_verification(&self, id: Uuid) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .verifications
                    .remove(&id);
                Ok(())
            }

            async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.email_verified = true;
                user.updated_at = Utc::now();
                Ok(())
            }

            async fn create_api_key(
                &self,
                input: AuthApiKeyCreate,
            ) -> Result<AuthApiKey, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let api_key = AuthApiKey {
                    id: Uuid::new_v4(),
                    name: input.name,
                    prefix: input.prefix,
                    key_hash: input.key_hash,
                    user_id: input.user_id,
                    enabled: input.enabled,
                    rate_limit_enabled: input.rate_limit_enabled,
                    rate_limit_time_window: input.rate_limit_time_window,
                    rate_limit_max: input.rate_limit_max,
                    request_count: input.request_count,
                    remaining: input.remaining,
                    expires_at: input.expires_at,
                    permissions_json: input.permissions_json,
                    metadata_json: input.metadata_json,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .api_keys_by_hash
                    .insert(api_key.key_hash.clone(), api_key.clone());
                inner.api_keys_by_id.insert(api_key.id, api_key.clone());
                Ok(api_key)
            }

            async fn find_api_key_by_hash(
                &self,
                key_hash: &str,
            ) -> Result<Option<AuthApiKey>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .api_keys_by_hash
                    .get(key_hash)
                    .cloned())
            }

            async fn find_api_key_by_id(
                &self,
                id: Uuid,
            ) -> Result<Option<AuthApiKey>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .api_keys_by_id
                    .get(&id)
                    .cloned())
            }

            async fn list_api_keys_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Vec<AuthApiKey>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .api_keys_by_id
                    .values()
                    .filter(|api_key| api_key.user_id == user_id)
                    .cloned()
                    .collect())
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
                let mut inner = self.inner.lock().expect("lock memory storage");
                let api_key = inner
                    .api_keys_by_id
                    .get_mut(&id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                if let Some(name) = name {
                    api_key.name = Some(name);
                }
                if let Some(enabled) = enabled {
                    api_key.enabled = enabled;
                }
                if expires_at.is_some() {
                    api_key.expires_at = expires_at;
                }
                if permissions_json.is_some() {
                    api_key.permissions_json = permissions_json;
                }
                if rate_limit_time_window.is_some() {
                    api_key.rate_limit_time_window = rate_limit_time_window;
                }
                if rate_limit_max.is_some() {
                    api_key.rate_limit_enabled = true;
                    api_key.rate_limit_max = rate_limit_max;
                    api_key.remaining = rate_limit_max;
                    api_key.request_count = Some(0);
                }
                if metadata_json.is_some() {
                    api_key.metadata_json = metadata_json;
                }
                api_key.updated_at = Utc::now();
                let updated = api_key.clone();
                if let Some(by_hash) = inner.api_keys_by_hash.get_mut(&updated.key_hash) {
                    *by_hash = updated.clone();
                }
                Ok(updated)
            }

            async fn delete_api_key(&self, id: Uuid) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                if let Some(api_key) = inner.api_keys_by_id.remove(&id) {
                    inner.api_keys_by_hash.remove(&api_key.key_hash);
                }
                Ok(())
            }

            async fn update_api_key_usage(
                &self,
                id: Uuid,
                request_count: Option<i64>,
                remaining: Option<i64>,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let key_hash = {
                    let api_key = inner
                        .api_keys_by_id
                        .get_mut(&id)
                        .ok_or(AuthFlowError::InvalidCredentials)?;
                    api_key.request_count = request_count;
                    api_key.remaining = remaining;
                    api_key.updated_at = Utc::now();
                    api_key.key_hash.clone()
                };
                if let Some(api_key) = inner.api_keys_by_id.get(&id).cloned() {
                    inner.api_keys_by_hash.insert(key_hash, api_key);
                }
                Ok(())
            }

            async fn set_api_key_enabled(
                &self,
                id: Uuid,
                enabled: bool,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let key_hash = {
                    let api_key = inner
                        .api_keys_by_id
                        .get_mut(&id)
                        .ok_or(AuthFlowError::InvalidCredentials)?;
                    api_key.enabled = enabled;
                    api_key.updated_at = Utc::now();
                    api_key.key_hash.clone()
                };
                if let Some(api_key) = inner.api_keys_by_id.get(&id).cloned() {
                    inner.api_keys_by_hash.insert(key_hash, api_key);
                }
                Ok(())
            }

            async fn create_passkey(
                &self,
                input: AuthPasskeyCreate,
            ) -> Result<AuthPasskey, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                if inner
                    .passkeys_by_credential_id
                    .contains_key(&input.credential_id)
                {
                    return Err(AuthFlowError::InvalidInput(
                        "passkey credential already exists".into(),
                    ));
                }
                let passkey = AuthPasskey {
                    id: Uuid::new_v4(),
                    name: input.name,
                    user_id: input.user_id,
                    public_key: input.public_key,
                    credential_id: input.credential_id,
                    counter: input.counter,
                    device_type: input.device_type,
                    backed_up: input.backed_up,
                    transports: input.transports,
                    created_at: Utc::now(),
                };
                inner
                    .passkeys_by_credential_id
                    .insert(passkey.credential_id.clone(), passkey.clone());
                Ok(passkey)
            }

            async fn find_passkey_by_credential_id(
                &self,
                credential_id: &str,
            ) -> Result<Option<AuthPasskey>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .passkeys_by_credential_id
                    .get(credential_id)
                    .cloned())
            }

            async fn list_passkeys_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Vec<AuthPasskey>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .passkeys_by_credential_id
                    .values()
                    .filter(|passkey| passkey.user_id == user_id)
                    .cloned()
                    .collect())
            }

            async fn update_passkey_counter(
                &self,
                credential_id: &str,
                counter: i64,
            ) -> Result<AuthPasskey, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let passkey = inner
                    .passkeys_by_credential_id
                    .get_mut(credential_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                passkey.counter = counter;
                Ok(passkey.clone())
            }

            async fn delete_passkey_by_credential_id(
                &self,
                credential_id: &str,
            ) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .passkeys_by_credential_id
                    .remove(credential_id);
                Ok(())
            }

            async fn create_organization(
                &self,
                input: AuthOrganizationCreate,
            ) -> Result<AuthOrganization, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let organization = AuthOrganization {
                    id: Uuid::new_v4(),
                    name: input.name,
                    slug: input.slug,
                    logo: input.logo,
                    metadata_json: input.metadata_json,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .organization_ids_by_slug
                    .insert(organization.slug.clone(), organization.id);
                inner
                    .organizations
                    .insert(organization.id, organization.clone());
                Ok(organization)
            }

            async fn find_organization_by_slug(
                &self,
                slug: &str,
            ) -> Result<Option<AuthOrganization>, AuthFlowError> {
                let inner = self.inner.lock().expect("lock memory storage");
                Ok(inner
                    .organization_ids_by_slug
                    .get(slug)
                    .and_then(|id| inner.organizations.get(id))
                    .cloned())
            }

            async fn create_member(
                &self,
                input: AuthMemberCreate,
            ) -> Result<AuthMember, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                if inner
                    .members
                    .contains_key(&(input.organization_id, input.user_id))
                {
                    return Err(AuthFlowError::InvalidInput(
                        "organization member already exists".into(),
                    ));
                }
                let member = AuthMember {
                    id: Uuid::new_v4(),
                    organization_id: input.organization_id,
                    user_id: input.user_id,
                    role: input.role,
                    created_at: Utc::now(),
                };
                inner
                    .members
                    .insert((member.organization_id, member.user_id), member.clone());
                Ok(member)
            }

            async fn find_member(
                &self,
                organization_id: Uuid,
                user_id: Uuid,
            ) -> Result<Option<AuthMember>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .members
                    .get(&(organization_id, user_id))
                    .cloned())
            }

            async fn list_members_by_organization(
                &self,
                organization_id: Uuid,
            ) -> Result<Vec<AuthMember>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .members
                    .values()
                    .filter(|member| member.organization_id == organization_id)
                    .cloned()
                    .collect())
            }

            async fn update_member_role(
                &self,
                organization_id: Uuid,
                user_id: Uuid,
                role: String,
            ) -> Result<AuthMember, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let member = inner
                    .members
                    .get_mut(&(organization_id, user_id))
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                member.role = role;
                Ok(member.clone())
            }

            async fn create_organization_role(
                &self,
                input: AuthOrganizationRoleCreate,
            ) -> Result<AuthOrganizationRole, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let role = AuthOrganizationRole {
                    id: Uuid::new_v4(),
                    organization_id: input.organization_id,
                    role: input.role,
                    permissions_json: input.permissions_json,
                    created_at: now,
                    updated_at: now,
                };
                let key = (role.organization_id, role.role.clone());
                if inner.organization_roles.contains_key(&key) {
                    return Err(AuthFlowError::InvalidInput(
                        "organization role already exists".into(),
                    ));
                }
                inner.organization_roles.insert(key, role.clone());
                Ok(role)
            }

            async fn find_organization_role(
                &self,
                organization_id: Uuid,
                role: &str,
            ) -> Result<Option<AuthOrganizationRole>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .organization_roles
                    .get(&(organization_id, role.to_string()))
                    .cloned())
            }

            async fn list_organization_roles(
                &self,
                organization_id: Uuid,
            ) -> Result<Vec<AuthOrganizationRole>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .organization_roles
                    .values()
                    .filter(|role| role.organization_id == organization_id)
                    .cloned()
                    .collect())
            }

            async fn update_organization_role(
                &self,
                organization_id: Uuid,
                role: &str,
                permissions_json: String,
            ) -> Result<AuthOrganizationRole, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let role = inner
                    .organization_roles
                    .get_mut(&(organization_id, role.to_string()))
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                role.permissions_json = permissions_json;
                role.updated_at = Utc::now();
                Ok(role.clone())
            }

            async fn delete_organization_role(
                &self,
                organization_id: Uuid,
                role: &str,
            ) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .organization_roles
                    .remove(&(organization_id, role.to_string()));
                Ok(())
            }

            async fn create_team(&self, input: AuthTeamCreate) -> Result<AuthTeam, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let team = AuthTeam {
                    id: Uuid::new_v4(),
                    organization_id: input.organization_id,
                    name: input.name,
                    created_at: now,
                    updated_at: now,
                };
                inner.teams.insert(team.id, team.clone());
                Ok(team)
            }

            async fn find_team_by_id(&self, id: Uuid) -> Result<Option<AuthTeam>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .teams
                    .get(&id)
                    .cloned())
            }

            async fn list_teams_by_organization(
                &self,
                organization_id: Uuid,
            ) -> Result<Vec<AuthTeam>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .teams
                    .values()
                    .filter(|team| team.organization_id == organization_id)
                    .cloned()
                    .collect())
            }

            async fn update_team_name(
                &self,
                id: Uuid,
                name: String,
            ) -> Result<AuthTeam, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let team = inner
                    .teams
                    .get_mut(&id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                team.name = name;
                team.updated_at = Utc::now();
                Ok(team.clone())
            }

            async fn delete_team(&self, id: Uuid) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                inner.teams.remove(&id);
                inner.team_members.retain(|(team_id, _), _| *team_id != id);
                Ok(())
            }

            async fn create_team_member(
                &self,
                input: AuthTeamMemberCreate,
            ) -> Result<AuthTeamMember, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let key = (input.team_id, input.user_id);
                if inner.team_members.contains_key(&key) {
                    return Err(AuthFlowError::InvalidInput(
                        "team member already exists".into(),
                    ));
                }
                let team_member = AuthTeamMember {
                    id: Uuid::new_v4(),
                    team_id: input.team_id,
                    user_id: input.user_id,
                    created_at: Utc::now(),
                };
                inner.team_members.insert(key, team_member.clone());
                Ok(team_member)
            }

            async fn find_team_member(
                &self,
                team_id: Uuid,
                user_id: Uuid,
            ) -> Result<Option<AuthTeamMember>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .team_members
                    .get(&(team_id, user_id))
                    .cloned())
            }

            async fn list_team_members(
                &self,
                team_id: Uuid,
            ) -> Result<Vec<AuthTeamMember>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .team_members
                    .values()
                    .filter(|member| member.team_id == team_id)
                    .cloned()
                    .collect())
            }

            async fn delete_team_member(
                &self,
                team_id: Uuid,
                user_id: Uuid,
            ) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .team_members
                    .remove(&(team_id, user_id));
                Ok(())
            }

            async fn update_session_active_organization(
                &self,
                token_hash: &str,
                organization_id: Option<Uuid>,
            ) -> Result<(), AuthFlowError> {
                if let Some(session) = self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .sessions_by_token_hash
                    .get_mut(token_hash)
                {
                    session.active_organization_id = organization_id;
                    session.updated_at = Utc::now();
                }
                Ok(())
            }

            async fn create_invitation(
                &self,
                input: AuthInvitationCreate,
            ) -> Result<AuthInvitation, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let invitation = AuthInvitation {
                    id: Uuid::new_v4(),
                    organization_id: input.organization_id,
                    email: input.email,
                    role: input.role,
                    status: input.status,
                    inviter_id: input.inviter_id,
                    expires_at: input.expires_at,
                    created_at: Utc::now(),
                };
                inner.invitations.insert(invitation.id, invitation.clone());
                Ok(invitation)
            }

            async fn find_invitation_by_id(
                &self,
                id: Uuid,
            ) -> Result<Option<AuthInvitation>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .invitations
                    .get(&id)
                    .cloned())
            }

            async fn update_invitation_status(
                &self,
                id: Uuid,
                status: String,
            ) -> Result<(), AuthFlowError> {
                if let Some(invitation) = self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .invitations
                    .get_mut(&id)
                {
                    invitation.status = status;
                }
                Ok(())
            }

            async fn create_two_factor(
                &self,
                input: AuthTwoFactorCreate,
            ) -> Result<AuthTwoFactor, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let now = Utc::now();
                let two_factor = AuthTwoFactor {
                    id: Uuid::new_v4(),
                    user_id: input.user_id,
                    secret_ciphertext: input.secret_ciphertext,
                    backup_codes_hash: input.backup_codes_hash,
                    attempt_count: input.attempt_count,
                    created_at: now,
                    updated_at: now,
                };
                inner
                    .two_factors
                    .insert(two_factor.user_id, two_factor.clone());
                Ok(two_factor)
            }

            async fn find_two_factor_by_user_id(
                &self,
                user_id: Uuid,
            ) -> Result<Option<AuthTwoFactor>, AuthFlowError> {
                Ok(self
                    .inner
                    .lock()
                    .expect("lock memory storage")
                    .two_factors
                    .get(&user_id)
                    .cloned())
            }

            async fn delete_two_factor(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .two_factors
                    .remove(&user_id);
                Ok(())
            }

            async fn update_two_factor_backup_codes(
                &self,
                user_id: Uuid,
                backup_codes_hash: Option<String>,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let two_factor = inner
                    .two_factors
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                two_factor.backup_codes_hash = backup_codes_hash;
                two_factor.updated_at = Utc::now();
                Ok(())
            }

            async fn increment_two_factor_attempts(
                &self,
                user_id: Uuid,
            ) -> Result<i64, AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let attempts = inner.two_factor_attempts.entry(user_id).or_default();
                *attempts += 1;
                Ok(*attempts)
            }

            async fn reset_two_factor_attempts(&self, user_id: Uuid) -> Result<(), AuthFlowError> {
                self.inner
                    .lock()
                    .expect("lock memory storage")
                    .two_factor_attempts
                    .remove(&user_id);
                Ok(())
            }

            async fn set_user_two_factor_enabled(
                &self,
                user_id: Uuid,
                enabled: bool,
            ) -> Result<(), AuthFlowError> {
                let mut inner = self.inner.lock().expect("lock memory storage");
                let user = inner
                    .users
                    .get_mut(&user_id)
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                user.two_factor_enabled = enabled;
                user.updated_at = Utc::now();
                Ok(())
            }
        }

        fn auth() -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        async fn assert_last_login_method(
            auth: &ArchitectAuth<MemoryStorage>,
            session_token: &str,
            expected: Option<&str>,
        ) {
            let method = auth
                .get_last_login_method(GetLastLoginMethod {
                    session_token: session_token.into(),
                })
                .await
                .expect("get last login method");
            assert_eq!(method.method.as_deref(), expected);
            assert_eq!(method.cookie_name, "better-auth.last_used_login_method");
            assert_eq!(method.max_age_seconds, 60 * 60 * 24 * 30);
        }

        fn auth_with_config(
            email_password_enabled: bool,
            require_email_verification: bool,
        ) -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .email_password_enabled(email_password_enabled)
                .require_email_verification(require_email_verification)
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn auth_with_signup_captcha() -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .captcha_test_token("captcha-ok")
                .captcha_protected_flow(CaptchaFlow::SignUp)
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn auth_with_rotated_jwt_keys() -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .jwt_issuer("architect-auth-test")
                .jwt_audience("architect-clients")
                .jwt_signing_key("v2", "jwt-secret-v2-at-least-32-bytes")
                .jwt_fallback_key("v1", "jwt-secret-v1-at-least-32-bytes")
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn auth_with_oidc_client() -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .base_url("https://auth.example.com")
                .jwt_issuer("https://auth.example.com")
                .jwt_audience("architect-auth")
                .oidc_issuer("https://auth.example.com")
                .oidc_allow_dynamic_client_registration(true)
                .oidc_client(OidcClientConfig {
                    client_id: "dashboard".into(),
                    client_secret: Some("client-secret".into()),
                    name: "Dashboard".into(),
                    redirect_uris: vec!["https://client.example.com/callback".into()],
                    scopes: vec![
                        "openid".into(),
                        "profile".into(),
                        "email".into(),
                        "offline_access".into(),
                    ],
                    public_client: false,
                    skip_consent: true,
                    disabled: false,
                })
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn auth_with_oauth_proxy() -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .base_url("https://preview.example.com")
                .oauth_proxy_current_url("https://preview.example.com")
                .oauth_proxy_production_url("https://auth.example.com")
                .oauth_proxy_allowed_redirect_origin("https://preview.example.com")
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn auth_with_one_tap(disable_signup: bool) -> ArchitectAuth<MemoryStorage> {
            ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .one_tap_client_id("google-client")
                .one_tap_disable_signup(disable_signup)
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth")
        }

        fn one_tap_id_token(
            email: &str,
            email_verified: bool,
            sub: &str,
            audience: &str,
        ) -> String {
            let now = Utc::now().timestamp() as usize;
            let claims = serde_json::json!({
                "iss": "https://accounts.google.com",
                "aud": audience,
                "sub": sub,
                "email": email,
                "email_verified": email_verified,
                "name": "One Tap User",
                "picture": "https://example.com/photo.jpg",
                "iat": now,
                "exp": now + 300,
            });
            jsonwebtoken::encode(
                &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
                &claims,
                &jsonwebtoken::EncodingKey::from_secret(b"a-secret-at-least-32-bytes-long!!"),
            )
            .expect("sign one tap id token")
        }

        // r[verify auth.email.signup.enabled]
        // r[verify auth.email.email-normalization]
        // r[verify auth.email.email-unique]
        // r[verify auth.email.password-never-returned]
        // r[verify auth.email.signin.success]
        // r[verify auth.sessions.ttl]
        // r[verify auth.sessions.context]
        // r[verify auth.core.timestamps]
        #[tokio::test]
        async fn signup_creates_user_account_and_session() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "User@Example.COM".into(),
                    password: "correct horse battery staple".into(),
                    name: Some("User".into()),
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("test".into()),
                })
                .await
                .expect("create user");

            assert_eq!(bundle.user.email.as_deref(), Some("User@example.com"));
            assert_eq!(bundle.session.ip_address.as_deref(), Some("127.0.0.1"));
            assert_eq!(bundle.session.user_agent.as_deref(), Some("test"));
            assert!(!bundle.token.is_empty());
            assert_ne!(bundle.session.token_hash, bundle.token);

            let password_hash = auth
                .storage
                .inner
                .lock()
                .expect("lock memory storage")
                .accounts_by_user
                .get(&bundle.user.id)
                .and_then(|accounts| {
                    accounts
                        .iter()
                        .find(|account| account.provider_id == super::PASSWORD_PROVIDER_ID)
                })
                .and_then(|account| account.password_hash.clone())
                .expect("password hash");
            assert_ne!(password_hash, "correct horse battery staple");

            let duplicate = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "User@example.com".into(),
                    password: "another password".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let signin = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "User@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in");
            assert_eq!(signin.user.id, bundle.user.id);
        }

        // r[verify auth.captcha.verify]
        // r[verify auth.captcha.providers]
        // r[verify auth.captcha.signup-hook]
        // r[verify auth.captcha.errors]
        #[tokio::test]
        async fn captcha_protects_signup_with_test_provider() {
            let auth = auth_with_signup_captcha();

            let missing = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "missing-captcha@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(missing, Err(AuthFlowError::PermissionDenied)));

            let failed = auth
                .verify_captcha(VerifyCaptcha {
                    flow: CaptchaFlow::SignUp,
                    token: Some("wrong".into()),
                })
                .await;
            assert!(matches!(failed, Err(AuthFlowError::PermissionDenied)));

            auth.verify_captcha(VerifyCaptcha {
                flow: CaptchaFlow::SignUp,
                token: Some("captcha-ok".into()),
            })
            .await
            .expect("verify captcha");

            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "captcha@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: Some(
                        r#"{"_captcha_token":"captcha-ok","source":"test"}"#.into(),
                    ),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create captcha-protected user");
            let metadata = serde_json::from_str::<serde_json::Value>(&bundle.user.metadata_json)
                .expect("stored metadata");
            assert_eq!(metadata["source"], "test");
            assert!(metadata.get("_captcha_token").is_none());
        }

        // r[verify auth.email.signin.invalid-generic]
        #[tokio::test]
        async fn signin_rejects_unknown_and_wrong_password_generically() {
            let auth = auth();
            let unknown = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "missing@example.com".into(),
                    password: "wrong".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(unknown, Err(AuthFlowError::InvalidCredentials)));
        }

        // r[verify auth.email.signup.disabled]
        #[tokio::test]
        async fn email_password_disabled_rejects_signup_and_signin() {
            let auth = auth_with_config(false, false);
            let signup = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(signup, Err(AuthFlowError::PermissionDenied)));

            let signin = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(signin, Err(AuthFlowError::PermissionDenied)));
            assert_eq!(auth.storage.inner.lock().expect("lock").users.len(), 0);
        }

        // r[verify auth.email.signin.verification-required]
        #[tokio::test]
        async fn signin_requires_verified_email_when_configured() {
            let auth = auth_with_config(true, true);
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            auth.sign_out(SignOut {
                token: bundle.token,
            })
            .await
            .expect("sign out initial session");

            let unverified = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                unverified,
                Err(AuthFlowError::VerificationRequired)
            ));

            auth.storage
                .mark_email_verified(bundle.user.id)
                .await
                .expect("mark verified");
            auth.sign_in_email_password(SignInEmailPassword {
                email: "user@example.com".into(),
                password: "correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("verified sign in");
        }

        // r[verify auth.core.server-authoritative]
        // r[verify auth.sessions.current.valid]
        // r[verify auth.sessions.current.missing]
        // r[verify auth.sessions.current.expired]
        // r[verify auth.sessions.signout]
        // r[verify auth.sessions.signout-idempotence]
        #[tokio::test]
        async fn current_session_and_signout_round_trip() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let current = auth
                .current_session(CurrentSession {
                    token: bundle.token.clone(),
                })
                .await
                .expect("current session");
            assert_eq!(current.user.id, bundle.user.id);

            let missing = auth
                .current_session(CurrentSession {
                    token: "missing".into(),
                })
                .await;
            assert!(matches!(missing, Err(AuthFlowError::InvalidCredentials)));

            auth.sign_out(SignOut {
                token: bundle.token.clone(),
            })
            .await
            .expect("sign out");
            auth.sign_out(SignOut {
                token: "unknown".into(),
            })
            .await
            .expect("unknown signout is idempotent");

            let after_signout = auth
                .current_session(CurrentSession {
                    token: bundle.token,
                })
                .await;
            assert!(matches!(after_signout, Err(AuthFlowError::SessionExpired)));
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        struct ExampleCustomSession {
            display_name: String,
            active_org: Option<Uuid>,
        }

        struct ExampleCustomSessionEnricher;

        impl CustomSessionEnricher for ExampleCustomSessionEnricher {
            type Output = ExampleCustomSession;

            fn enrich_session<'a>(
                &'a self,
                bundle: &'a AuthSessionBundle,
            ) -> Pin<Box<dyn Future<Output = Result<Self::Output, AuthFlowError>> + Send + 'a>>
            {
                Box::pin(async move {
                    Ok(ExampleCustomSession {
                        display_name: bundle
                            .user
                            .name
                            .clone()
                            .unwrap_or_else(|| "Anonymous".into()),
                        active_org: bundle.session.active_organization_id,
                    })
                })
            }
        }

        // r[verify auth.custom-session.typed-hook]
        // r[verify auth.custom-session.backcompat]
        #[tokio::test]
        async fn custom_session_enrichment_preserves_standard_session_envelope() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "custom-session@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: Some("Custom User".into()),
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let custom = auth
                .current_custom_session(
                    CurrentSession {
                        token: bundle.token.clone(),
                    },
                    &ExampleCustomSessionEnricher,
                )
                .await
                .expect("custom session");

            assert_eq!(custom.user.id, bundle.user.id);
            assert_eq!(custom.session.id, bundle.session.id);
            assert_eq!(custom.token, bundle.token);
            assert_eq!(
                custom.custom,
                ExampleCustomSession {
                    display_name: "Custom User".into(),
                    active_org: None,
                }
            );
        }

        // r[verify auth.sessions.list]
        // r[verify auth.sessions.revoke]
        // r[verify auth.sessions.revoke-other]
        #[tokio::test]
        async fn list_and_revoke_sessions_are_user_scoped() {
            let auth = auth();
            let first = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let second = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("second session");
            let other = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "other@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("other user");

            let sessions = auth
                .list_sessions(ListSessions {
                    session_token: first.token.clone(),
                })
                .await
                .expect("list sessions");
            assert_eq!(sessions.len(), 2);
            assert!(
                sessions
                    .iter()
                    .all(|session| session.user_id == first.user.id)
            );

            let cross_user_revoke = auth
                .revoke_session(RevokeSession {
                    session_token: first.token.clone(),
                    session_id: other.session.id,
                })
                .await;
            assert!(matches!(
                cross_user_revoke,
                Err(AuthFlowError::PermissionDenied)
            ));

            auth.revoke_session(RevokeSession {
                session_token: first.token.clone(),
                session_id: second.session.id,
            })
            .await
            .expect("revoke owned session");
            let second_after_revoke = auth
                .current_session(CurrentSession {
                    token: second.token,
                })
                .await;
            assert!(matches!(
                second_after_revoke,
                Err(AuthFlowError::SessionExpired)
            ));

            let third = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("third session");
            auth.revoke_other_sessions(RevokeOtherSessions {
                session_token: first.token.clone(),
            })
            .await
            .expect("revoke other sessions");
            auth.current_session(CurrentSession { token: first.token })
                .await
                .expect("current preserved");
            let third_after_revoke = auth
                .current_session(CurrentSession { token: third.token })
                .await;
            assert!(matches!(
                third_after_revoke,
                Err(AuthFlowError::SessionExpired)
            ));
        }

        // r[verify auth.account.list]
        // r[verify auth.user.delete]
        #[tokio::test]
        async fn account_listing_and_delete_user_are_user_scoped() {
            let auth = auth();
            let user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let other = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "other@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create other user");

            let accounts = auth
                .list_accounts(ListAccounts {
                    session_token: user.token.clone(),
                })
                .await
                .expect("list accounts");
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].user_id, user.user.id);

            auth.delete_user(DeleteUser {
                session_token: user.token.clone(),
            })
            .await
            .expect("delete user");
            assert!(
                auth.storage
                    .find_user_by_id(user.user.id)
                    .await
                    .expect("find deleted user")
                    .is_none()
            );
            assert!(
                auth.current_session(CurrentSession { token: user.token })
                    .await
                    .is_err()
            );
            auth.current_session(CurrentSession { token: other.token })
                .await
                .expect("other user remains signed in");
        }

        // r[verify auth.core.metadata-json]
        // r[verify auth.password.strength-policy]
        #[tokio::test]
        async fn signup_rejects_invalid_metadata_json() {
            let auth = auth();
            let result = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: Some("not json".into()),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(result, Err(AuthFlowError::InvalidInput(_))));

            let weak_password = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "weak@example.com".into(),
                    password: "short".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(weak_password, Err(AuthFlowError::InvalidInput(_))));
        }

        // r[verify auth.additional-fields.persist]
        // r[verify auth.additional-fields.returned]
        // r[verify auth.additional-fields.hidden-metadata]
        // r[verify auth.additional-fields.schema]
        #[tokio::test]
        async fn additional_fields_validate_project_and_hide_metadata() {
            let auth = auth();
            let fields = vec![
                AdditionalFieldSpec {
                    name: "plan",
                    field_type: AdditionalFieldType::String,
                    required: true,
                    input: true,
                    returned: true,
                    default_json: None,
                },
                AdditionalFieldSpec {
                    name: "internal_note",
                    field_type: AdditionalFieldType::String,
                    required: false,
                    input: true,
                    returned: false,
                    default_json: None,
                },
                AdditionalFieldSpec {
                    name: "beta",
                    field_type: AdditionalFieldType::Boolean,
                    required: false,
                    input: true,
                    returned: true,
                    default_json: Some("false"),
                },
            ];

            crate::flows::additional_fields::validate_additional_metadata(
                Some(r#"{"plan":"pro","internal_note":"secret"}"#),
                &fields,
            )
            .expect("valid additional metadata");
            let invalid = crate::flows::additional_fields::validate_additional_metadata(
                Some(r#"{"internal_note":"secret"}"#),
                &fields,
            );
            assert!(matches!(invalid, Err(AuthFlowError::InvalidInput(_))));

            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "additional-fields@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: Some(r#"{"plan":"pro","internal_note":"secret"}"#.into()),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user with additional metadata");

            let view = crate::flows::additional_fields::project_user_additional_fields(
                &bundle.user,
                &fields,
            )
            .expect("project additional fields");
            assert_eq!(view.fields["plan"], "pro");
            assert_eq!(view.fields["beta"], false);
            assert!(view.fields.get("internal_note").is_none());

            let schema = auth.additional_fields_schema(&AdditionalFieldsConfig {
                user: fields.clone(),
                session: vec![],
                account: vec![],
            });
            assert_eq!(schema.user.len(), 3);
            assert!(schema.user.iter().any(|field| !field.returned));
        }

        // r[verify auth.hibp.range-check]
        // r[verify auth.hibp.reject-breached]
        // r[verify auth.hibp.failure-policy]
        // r[verify auth.hibp.password-hooks]
        #[tokio::test]
        async fn haveibeenpwned_rejects_breached_passwords_and_honors_failure_policy() {
            let auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .breached_password_provider(BreachedPasswordProvider::Test {
                    breached_passwords: vec!["correct horse battery staple".into()],
                })
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth");
            let check = auth
                .check_password_breach(CheckPasswordBreach {
                    password: "correct horse battery staple".into(),
                })
                .await
                .expect("check breach");
            assert!(check.breached);
            assert_eq!(check.count, 1);
            assert_eq!(check.prefix.len(), 5);

            let signup = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "breached@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(signup, Err(AuthFlowError::InvalidInput(_))));

            let clean = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "clean@example.com".into(),
                    password: "not breached password".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create clean user");
            let change = auth
                .change_password(ChangePassword {
                    session_token: clean.token.clone(),
                    current_password: "not breached password".into(),
                    new_password: "correct horse battery staple".into(),
                })
                .await;
            assert!(matches!(change, Err(AuthFlowError::InvalidInput(_))));

            let deny = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .breached_password_provider(BreachedPasswordProvider::Unavailable)
                .breached_password_failure_policy(BreachedPasswordFailurePolicy::Deny)
                .storage(MemoryStorage::default())
                .build()
                .expect("build deny auth");
            let denied = deny
                .check_password_breach(CheckPasswordBreach {
                    password: "any password".into(),
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            let allow = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .breached_password_provider(BreachedPasswordProvider::Unavailable)
                .breached_password_failure_policy(BreachedPasswordFailurePolicy::Allow)
                .storage(MemoryStorage::default())
                .build()
                .expect("build allow auth");
            let allowed = allow
                .check_password_breach(CheckPasswordBreach {
                    password: "any password".into(),
                })
                .await
                .expect("allow unavailable provider");
            assert!(!allowed.breached);
        }

        // r[verify auth.username.validation]
        // r[verify auth.username.reserved]
        // r[verify auth.username.unique]
        // r[verify auth.username.case-insensitive]
        // r[verify auth.username.signin]
        // r[verify auth.username.update]
        #[tokio::test]
        async fn username_signup_signin_update_and_policy_are_enforced() {
            let auth = auth();
            let first = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "username-one@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: Some("Cool_User".into()),
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user with username");
            assert_eq!(first.user.username.as_deref(), Some("cool_user"));
            assert_eq!(first.user.display_username.as_deref(), Some("Cool_User"));

            let signed_in = auth
                .sign_in_username(SignInUsername {
                    username: "COOL_USER".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: Some("username-test".into()),
                })
                .await
                .expect("sign in by case-insensitive username");
            assert_eq!(signed_in.user.id, first.user.id);
            assert_eq!(
                signed_in.session.user_agent.as_deref(),
                Some("username-test")
            );
            assert_last_login_method(&auth, &signed_in.token, Some("username")).await;

            let wrong_password = auth
                .sign_in_username(SignInUsername {
                    username: "cool_user".into(),
                    password: "wrong password".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                wrong_password,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let duplicate = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "username-two@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: Some("COOL_USER".into()),
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            for invalid in ["ad", "root", "has-dash", "space name"] {
                let result = auth
                    .update_username(UpdateUsername {
                        session_token: first.token.clone(),
                        username: invalid.into(),
                        display_username: None,
                    })
                    .await;
                assert!(matches!(result, Err(AuthFlowError::InvalidInput(_))));
            }

            let second = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "username-three@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: Some("third_user".into()),
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create second username");
            let duplicate_update = auth
                .update_username(UpdateUsername {
                    session_token: second.token.clone(),
                    username: "cool_user".into(),
                    display_username: None,
                })
                .await;
            assert!(matches!(
                duplicate_update,
                Err(AuthFlowError::InvalidInput(_))
            ));

            let updated = auth
                .update_username(UpdateUsername {
                    session_token: first.token.clone(),
                    username: "New_Name".into(),
                    display_username: Some("New Name".into()),
                })
                .await
                .expect("update username");
            assert_eq!(updated.username.as_deref(), Some("new_name"));
            assert_eq!(updated.display_username.as_deref(), Some("New Name"));
            auth.sign_in_username(SignInUsername {
                username: "cool_user".into(),
                password: "correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect_err("old username no longer signs in");
            auth.sign_in_username(SignInUsername {
                username: "new_name".into(),
                password: "correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("new username signs in");
        }

        // r[verify auth.password.change.requires-current]
        // r[verify auth.password.change-invalidates]
        // r[verify auth.password.strength-policy]
        // ── email migration ────────────────────────────────────────

        // Not a test: a helper. Named so it can't be mistaken for one.
        async fn seed_user_with_email(email: &str) -> (ArchitectAuth<MemoryStorage>, Uuid) {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: email.into(),
                    password: "correct horse battery staple".into(),
                    name: Some("Seed".into()),
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let id = bundle.user.id;
            (auth, id)
        }

        #[tokio::test]
        async fn migrating_an_email_keeps_the_user_id() {
            // THE property. The id is what tasks, timers, sessions and
            // authorship are keyed on, so a rename must not mint a new
            // account — doing that would orphan everything silently.
            let (auth, id) = seed_user_with_email("old@example.com").await;
            let moved = auth
                .migrate_user_email(MigrateUserEmail {
                    user_id: id,
                    new_email: "new@example.com".into(),
                    changed_by: None,
                    reason: None,
                })
                .await
                .expect("migrate");
            assert_eq!(moved.id, id, "migration must not change the user id");
            assert_eq!(moved.email.as_deref(), Some("new@example.com"));
        }

        #[tokio::test]
        async fn migration_records_the_trail() {
            let (auth, id) = seed_user_with_email("first@example.com").await;
            for (to, why) in [
                ("second@example.com", "domain move"),
                ("third@example.com", "consolidating"),
            ] {
                auth.migrate_user_email(MigrateUserEmail {
                    user_id: id,
                    new_email: to.into(),
                    changed_by: Some(id),
                    reason: Some(why.into()),
                })
                .await
                .expect("migrate");
            }

            let history = auth.list_email_history(id).await.expect("history");
            assert_eq!(history.len(), 2, "one row per change: {history:?}");
            // Oldest first, and each row links the pair it moved between,
            // so the chain reads end to end.
            assert_eq!(history[0].previous_email.as_deref(), Some("first@example.com"));
            assert_eq!(history[0].new_email, "second@example.com");
            assert_eq!(history[0].reason.as_deref(), Some("domain move"));
            assert_eq!(history[1].previous_email.as_deref(), Some("second@example.com"));
            assert_eq!(history[1].new_email, "third@example.com");
        }

        #[tokio::test]
        async fn a_migrated_address_is_still_resolvable() {
            // The reason the trail exists: after a migration, "who was
            // old@example.com?" must still have an answer.
            let (auth, id) = seed_user_with_email("old@example.com").await;
            auth.migrate_user_email(MigrateUserEmail {
                user_id: id,
                new_email: "new@example.com".into(),
                changed_by: None,
                reason: None,
            })
            .await
            .expect("migrate");

            let found = auth
                .find_user_by_previous_email("old@example.com")
                .await
                .expect("lookup")
                .expect("the old address should still resolve");
            assert_eq!(found.id, id);
            assert_eq!(found.email.as_deref(), Some("new@example.com"));
        }

        #[tokio::test]
        async fn migration_refuses_an_address_another_account_holds() {
            let (auth, id) = seed_user_with_email("mine@example.com").await;
            auth.create_email_password_user(CreateEmailPasswordUser {
                email: "taken@example.com".into(),
                password: "correct horse battery staple".into(),
                name: None,
                username: None,
                image: None,
                metadata_json: None,
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("second user");

            let clash = auth
                .migrate_user_email(MigrateUserEmail {
                    user_id: id,
                    new_email: "taken@example.com".into(),
                    changed_by: None,
                    reason: None,
                })
                .await;
            assert!(clash.is_err(), "must not collide two accounts onto one address");
            assert!(
                auth.list_email_history(id).await.expect("history").is_empty(),
                "a refused migration must not leave a row in the trail"
            );
        }

        #[tokio::test]
        async fn migrating_to_the_same_address_is_a_no_op() {
            // Re-running a bulk migration should be safe, and must not
            // append a row claiming a change that didn't happen.
            let (auth, id) = seed_user_with_email("same@example.com").await;
            let again = auth
                .migrate_user_email(MigrateUserEmail {
                    user_id: id,
                    new_email: "same@example.com".into(),
                    changed_by: None,
                    reason: None,
                })
                .await
                .expect("no-op migrate");
            assert_eq!(again.id, id);
            assert!(auth.list_email_history(id).await.expect("history").is_empty());
        }

        #[tokio::test]
        async fn migration_resets_verification() {
            // The new address hasn't been proven to belong to anyone;
            // inheriting the old one's verified flag would be a lie the
            // rest of the system trusts.
            let (auth, id) = seed_user_with_email("old@example.com").await;
            let moved = auth
                .migrate_user_email(MigrateUserEmail {
                    user_id: id,
                    new_email: "new@example.com".into(),
                    changed_by: None,
                    reason: None,
                })
                .await
                .expect("migrate");
            assert!(!moved.email_verified);
        }

        #[tokio::test]
        async fn self_service_change_email_also_records() {
            // Every path that changes an address appends, or the trail is
            // "whatever remembered to write" rather than a record.
            let (auth, id) = seed_user_with_email("self@example.com").await;
            let signed_in = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "self@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in");
            auth.change_email(ChangeEmail {
                session_token: signed_in.token,
                new_email: "self-new@example.com".into(),
            })
            .await
            .expect("change email");

            let history = auth.list_email_history(id).await.expect("history");
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].previous_email.as_deref(), Some("self@example.com"));
            assert_eq!(
                history[0].changed_by, None,
                "self-service changes record no operator"
            );
        }

        #[tokio::test]
        async fn change_password_requires_current_password() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let wrong = auth
                .change_password(ChangePassword {
                    session_token: bundle.token.clone(),
                    current_password: "wrong".into(),
                    new_password: "new correct horse battery staple".into(),
                })
                .await;
            assert!(matches!(wrong, Err(AuthFlowError::InvalidCredentials)));

            let weak = auth
                .change_password(ChangePassword {
                    session_token: bundle.token.clone(),
                    current_password: "correct horse battery staple".into(),
                    new_password: "short".into(),
                })
                .await;
            assert!(matches!(weak, Err(AuthFlowError::InvalidInput(_))));

            auth.change_password(ChangePassword {
                session_token: bundle.token.clone(),
                current_password: "correct horse battery staple".into(),
                new_password: "new correct horse battery staple".into(),
            })
            .await
            .expect("change password");
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: bundle.token
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));

            let old = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(old, Err(AuthFlowError::InvalidCredentials)));

            auth.sign_in_email_password(SignInEmailPassword {
                email: "user@example.com".into(),
                password: "new correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("new password works");
        }

        // r[verify auth.password.reset-token-random]
        // r[verify auth.password.reset-token-hash]
        // r[verify auth.password.reset-expiry]
        // r[verify auth.password.reset-single-use]
        // r[verify auth.password.reset-generic-response]
        // r[verify auth.password.strength-policy]
        #[tokio::test]
        async fn password_reset_uses_single_use_hashed_token() {
            let auth = auth();
            auth.create_email_password_user(CreateEmailPasswordUser {
                email: "user@example.com".into(),
                password: "correct horse battery staple".into(),
                name: None,
                username: None,
                image: None,
                metadata_json: None,
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("create user");

            let known = auth
                .request_password_reset(RequestPasswordReset {
                    email: "user@example.com".into(),
                })
                .await
                .expect("request reset");
            let unknown = auth
                .request_password_reset(RequestPasswordReset {
                    email: "missing@example.com".into(),
                })
                .await
                .expect("unknown reset request has same response shape");
            assert!(!known.token.is_empty());
            assert!(!unknown.token.is_empty());
            assert_ne!(known.token, unknown.token);

            let stored = auth
                .storage
                .inner
                .lock()
                .expect("lock memory storage")
                .verifications
                .values()
                .find(|verification| verification.identifier == known.identifier)
                .cloned()
                .expect("stored verification");
            assert_ne!(stored.value_hash, known.token);

            let weak = auth
                .complete_password_reset(CompletePasswordReset {
                    email: "user@example.com".into(),
                    token: known.token.clone(),
                    new_password: "short".into(),
                })
                .await;
            assert!(matches!(weak, Err(AuthFlowError::InvalidInput(_))));

            auth.complete_password_reset(CompletePasswordReset {
                email: "user@example.com".into(),
                token: known.token.clone(),
                new_password: "new correct horse battery staple".into(),
            })
            .await
            .expect("complete reset");

            let reused = auth
                .complete_password_reset(CompletePasswordReset {
                    email: "user@example.com".into(),
                    token: known.token,
                    new_password: "another password".into(),
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));

            auth.sign_in_email_password(SignInEmailPassword {
                email: "user@example.com".into(),
                password: "new correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("reset password works");
        }

        // r[verify auth.verify.token-random]
        // r[verify auth.verify.token-hash]
        // r[verify auth.verify.expiry]
        // r[verify auth.verify.identifier]
        // r[verify auth.verify.single-use]
        // r[verify auth.verify.success]
        // r[verify auth.verify.change-email]
        // r[verify auth.verify.resend-throttle]
        #[tokio::test]
        async fn email_verification_marks_user_verified_once() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            assert!(!bundle.user.email_verified);

            let verification = auth
                .request_email_verification(RequestEmailVerification {
                    user_id: bundle.user.id,
                })
                .await
                .expect("request verification");
            assert!(verification.identifier.starts_with("email-verification:"));

            let throttled = auth
                .request_email_verification(RequestEmailVerification {
                    user_id: bundle.user.id,
                })
                .await;
            assert!(matches!(throttled, Err(AuthFlowError::PermissionDenied)));

            auth.verify_email(VerifyEmail {
                user_id: bundle.user.id,
                token: verification.token.clone(),
            })
            .await
            .expect("verify email");

            let user = auth
                .storage
                .find_user_by_id(bundle.user.id)
                .await
                .expect("load user")
                .expect("user exists");
            assert!(user.email_verified);

            let reused = auth
                .verify_email(VerifyEmail {
                    user_id: bundle.user.id,
                    token: verification.token,
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));

            let changed = auth
                .change_email(ChangeEmail {
                    session_token: bundle.token,
                    new_email: "changed@example.com".into(),
                })
                .await
                .expect("change email");
            assert_eq!(changed.email.as_deref(), Some("changed@example.com"));
            assert!(!changed.email_verified);
        }

        // r[verify auth.emailotp.send]
        // r[verify auth.emailotp.verify]
        // r[verify auth.emailotp.expiry]
        // r[verify auth.emailotp.resend-limit]
        // r[verify auth.emailotp.single-use]
        // r[verify auth.emailotp.session]
        // r[verify auth.emailotp.test-sink]
        #[tokio::test]
        async fn email_otp_verifies_once_and_optionally_creates_session() {
            let auth = auth();
            let otp = auth
                .send_email_otp(SendEmailOtp {
                    email: "otp@example.com".into(),
                })
                .await
                .expect("send otp");
            assert!(otp.identifier.starts_with("email-otp:"));
            assert_eq!(otp.token.len(), 6);

            let throttled = auth
                .send_email_otp(SendEmailOtp {
                    email: "otp@example.com".into(),
                })
                .await;
            assert!(matches!(throttled, Err(AuthFlowError::PermissionDenied)));

            let wrong = auth
                .verify_email_otp(VerifyEmailOtp {
                    email: "otp@example.com".into(),
                    otp: "wrong".into(),
                    create_session: true,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(wrong, Err(AuthFlowError::InvalidCredentials)));

            let verified = auth
                .verify_email_otp(VerifyEmailOtp {
                    email: "otp@example.com".into(),
                    otp: otp.token.clone(),
                    create_session: true,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("email-otp-test".into()),
                })
                .await
                .expect("verify otp");
            assert_eq!(verified.user.email.as_deref(), Some("otp@example.com"));
            assert!(verified.session.is_some());
            assert!(verified.token.is_some());
            assert_eq!(
                verified
                    .session
                    .as_ref()
                    .and_then(|session| session.user_agent.as_deref()),
                Some("email-otp-test")
            );

            let reused = auth
                .verify_email_otp(VerifyEmailOtp {
                    email: "otp@example.com".into(),
                    otp: otp.token,
                    create_session: false,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));

            let expired_auth = self::auth();
            let expired = expired_auth
                .send_email_otp(SendEmailOtp {
                    email: "expired@example.com".into(),
                })
                .await
                .expect("send expired otp");
            {
                let mut inner = expired_auth
                    .storage
                    .inner
                    .lock()
                    .expect("lock memory storage");
                for verification in inner.verifications.values_mut() {
                    verification.expires_at = Utc::now() - Duration::seconds(1);
                    verification.created_at = Utc::now() - Duration::seconds(120);
                }
            }
            let expired_result = expired_auth
                .verify_email_otp(VerifyEmailOtp {
                    email: "expired@example.com".into(),
                    otp: expired.token,
                    create_session: false,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                expired_result,
                Err(AuthFlowError::InvalidCredentials)
            ));
        }

        // r[verify auth.phone.send]
        // r[verify auth.phone.verify]
        // r[verify auth.phone.expiry]
        // r[verify auth.phone.duplicate]
        // r[verify auth.phone.provider]
        // r[verify auth.phone.update]
        // r[verify auth.phone.signin]
        #[tokio::test]
        async fn phone_number_otp_update_duplicate_and_provider_failures_are_enforced() {
            let auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .sms_test_provider()
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth");

            let token = auth
                .send_phone_number_otp(SendPhoneNumberOtp {
                    phone_number: "+15555550100".into(),
                })
                .await
                .expect("send phone otp");
            assert!(token.identifier.starts_with("phone-number:+15555550100"));

            let verified = auth
                .verify_phone_number_otp(VerifyPhoneNumberOtp {
                    phone_number: "+15555550100".into(),
                    otp: token.token.clone(),
                    create_session: true,
                    ip_address: None,
                    user_agent: Some("phone-test".into()),
                })
                .await
                .expect("verify phone otp");
            assert_eq!(verified.phone_number, "+15555550100");
            assert!(verified.session.is_some());
            assert_eq!(
                verified
                    .session
                    .as_ref()
                    .and_then(|session| session.user_agent.as_deref()),
                Some("phone-test")
            );
            let metadata = serde_json::from_str::<serde_json::Value>(&verified.user.metadata_json)
                .expect("phone metadata");
            assert_eq!(metadata["phone_number"], "+15555550100");
            assert_eq!(metadata["phone_number_verified"], true);
            assert_last_login_method(
                &auth,
                verified.token.as_deref().expect("phone session token"),
                Some("phone-number"),
            )
            .await;

            let replay = auth
                .verify_phone_number_otp(VerifyPhoneNumberOtp {
                    phone_number: "+15555550100".into(),
                    otp: token.token,
                    create_session: true,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(replay, Err(AuthFlowError::InvalidCredentials)));

            let expired_auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .sms_test_provider()
                .storage(MemoryStorage::default())
                .build()
                .expect("build expired auth");
            let expired = expired_auth
                .send_phone_number_otp(SendPhoneNumberOtp {
                    phone_number: "+15555550101".into(),
                })
                .await
                .expect("send expired phone otp");
            {
                let mut inner = expired_auth
                    .storage
                    .inner
                    .lock()
                    .expect("lock memory storage");
                for verification in inner.verifications.values_mut() {
                    verification.expires_at = Utc::now() - Duration::seconds(1);
                }
            }
            let expired_result = expired_auth
                .verify_phone_number_otp(VerifyPhoneNumberOtp {
                    phone_number: "+15555550101".into(),
                    otp: expired.token,
                    create_session: false,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                expired_result,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "phone@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let updated = auth
                .update_phone_number(UpdatePhoneNumber {
                    session_token: user.token.clone(),
                    phone_number: "+15555550102".into(),
                })
                .await
                .expect("update phone number");
            let metadata = serde_json::from_str::<serde_json::Value>(&updated.metadata_json)
                .expect("updated phone metadata");
            assert_eq!(metadata["phone_number"], "+15555550102");
            assert_eq!(metadata["phone_number_verified"], false);

            let duplicate = auth
                .update_phone_number(UpdatePhoneNumber {
                    session_token: user.token,
                    phone_number: "+15555550100".into(),
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let invalid = auth
                .send_phone_number_otp(SendPhoneNumberOtp {
                    phone_number: "555-0100".into(),
                })
                .await;
            assert!(matches!(invalid, Err(AuthFlowError::InvalidInput(_))));

            let fail_closed = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .sms_provider(SmsProvider::FailClosed)
                .storage(MemoryStorage::default())
                .build()
                .expect("build fail-closed auth");
            let provider_failure = fail_closed
                .send_phone_number_otp(SendPhoneNumberOtp {
                    phone_number: "+15555550103".into(),
                })
                .await;
            assert!(matches!(
                provider_failure,
                Err(AuthFlowError::PermissionDenied)
            ));
        }

        // r[verify auth.siwe.nonce]
        // r[verify auth.siwe.verify]
        // r[verify auth.siwe.domain]
        // r[verify auth.siwe.replay]
        // r[verify auth.siwe.linked-account]
        // r[verify auth.siwe.address-link]
        #[tokio::test]
        async fn siwe_nonce_signature_domain_replay_and_linked_accounts_are_enforced() {
            let auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .siwe_domain("auth.example.com")
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth");
            let address = "0x1111111111111111111111111111111111111111";
            let nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create siwe nonce");
            let message = format!(
                "auth.example.com\nAddress: {address}\nURI: https://auth.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                nonce.token
            );
            let signature =
                crate::flows::siwe::test_siwe_signature(&auth.config.secret, &message, address);
            let session = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: message.clone(),
                    signature: signature.clone(),
                    ip_address: None,
                    user_agent: Some("siwe-test".into()),
                })
                .await
                .expect("verify siwe message");
            assert_eq!(session.session.user_agent.as_deref(), Some("siwe-test"));
            assert_last_login_method(&auth, &session.token, Some("siwe")).await;

            let replay = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: message.clone(),
                    signature,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(replay, Err(AuthFlowError::InvalidCredentials)));

            let linked_nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create linked nonce");
            let linked_message = format!(
                "auth.example.com\nAddress: {address}\nURI: https://auth.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                linked_nonce.token
            );
            let linked_signature = crate::flows::siwe::test_siwe_signature(
                &auth.config.secret,
                &linked_message,
                address,
            );
            let linked = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: linked_message,
                    signature: linked_signature,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("linked siwe signin");
            assert_eq!(linked.user.id, session.user.id);

            let wrong_domain_nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create wrong-domain nonce");
            let wrong_domain_message = format!(
                "evil.example.com\nAddress: {address}\nURI: https://evil.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                wrong_domain_nonce.token
            );
            let wrong_domain_signature = crate::flows::siwe::test_siwe_signature(
                &auth.config.secret,
                &wrong_domain_message,
                address,
            );
            let wrong_domain = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: wrong_domain_message,
                    signature: wrong_domain_signature,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(wrong_domain, Err(AuthFlowError::PermissionDenied)));

            let invalid_nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create invalid-signature nonce");
            let invalid_message = format!(
                "auth.example.com\nAddress: {address}\nURI: https://auth.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                invalid_nonce.token
            );
            let invalid = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: invalid_message,
                    signature: "test:invalid".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(invalid, Err(AuthFlowError::InvalidCredentials)));

            let password_user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "siwe-link@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create password user");
            let link_address = "0x2222222222222222222222222222222222222222";
            let link_nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create link nonce");
            let link_message = format!(
                "auth.example.com\nAddress: {link_address}\nURI: https://auth.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                link_nonce.token
            );
            let link_signature = crate::flows::siwe::test_siwe_signature(
                &auth.config.secret,
                &link_message,
                link_address,
            );
            auth.link_siwe_address(LinkSiweAddress {
                session_token: password_user.token.clone(),
                message: link_message,
                signature: link_signature,
            })
            .await
            .expect("link siwe address");

            let relogin_nonce = auth
                .create_siwe_nonce(CreateSiweNonce)
                .await
                .expect("create relogin nonce");
            let relogin_message = format!(
                "auth.example.com\nAddress: {link_address}\nURI: https://auth.example.com\nVersion: 1\nChain ID: 1\nNonce: {}",
                relogin_nonce.token
            );
            let relogin_signature = crate::flows::siwe::test_siwe_signature(
                &auth.config.secret,
                &relogin_message,
                link_address,
            );
            let relogin = auth
                .verify_siwe_message(VerifySiweMessage {
                    message: relogin_message,
                    signature: relogin_signature,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in linked siwe address");
            assert_eq!(relogin.user.id, password_user.user.id);
        }

        // r[verify auth.magic.send]
        // r[verify auth.magic.verify]
        // r[verify auth.magic.expiry]
        // r[verify auth.magic.single-use]
        // r[verify auth.magic.session]
        // r[verify auth.magic.redirect-trust]
        #[tokio::test]
        async fn magic_link_verifies_once_and_enforces_redirect_trust() {
            let auth = auth();
            let link = auth
                .send_magic_link(SendMagicLink {
                    email: "magic@example.com".into(),
                    callback_url: Some("http://localhost:3000/auth/callback".into()),
                })
                .await
                .expect("send magic link");
            assert!(link.identifier.starts_with("magic-link:"));
            assert!(link.url.contains("token="));
            assert_eq!(link.callback_url, "http://localhost:3000/auth/callback");

            let untrusted = auth
                .send_magic_link(SendMagicLink {
                    email: "magic@example.com".into(),
                    callback_url: Some("https://evil.example/callback".into()),
                })
                .await;
            assert!(matches!(untrusted, Err(AuthFlowError::PermissionDenied)));

            let mismatch = auth
                .verify_magic_link(VerifyMagicLink {
                    email: "magic@example.com".into(),
                    token: "wrong-token".into(),
                    callback_url: Some("http://localhost:3000/auth/callback".into()),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(mismatch, Err(AuthFlowError::InvalidCredentials)));

            let verified = auth
                .verify_magic_link(VerifyMagicLink {
                    email: "magic@example.com".into(),
                    token: link.token.clone(),
                    callback_url: Some("http://localhost:3000/auth/callback".into()),
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("magic-link-test".into()),
                })
                .await
                .expect("verify magic link");
            assert_eq!(verified.user.email.as_deref(), Some("magic@example.com"));
            assert_eq!(
                verified.session.user_agent.as_deref(),
                Some("magic-link-test")
            );
            assert_eq!(verified.redirect_url, "http://localhost:3000/auth/callback");

            let reused = auth
                .verify_magic_link(VerifyMagicLink {
                    email: "magic@example.com".into(),
                    token: link.token,
                    callback_url: Some("http://localhost:3000/auth/callback".into()),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));

            let expired_auth = self::auth();
            let expired = expired_auth
                .send_magic_link(SendMagicLink {
                    email: "expired-magic@example.com".into(),
                    callback_url: None,
                })
                .await
                .expect("send expired magic link");
            {
                let mut inner = expired_auth
                    .storage
                    .inner
                    .lock()
                    .expect("lock memory storage");
                for verification in inner.verifications.values_mut() {
                    verification.expires_at = Utc::now() - Duration::seconds(1);
                }
            }
            let expired_result = expired_auth
                .verify_magic_link(VerifyMagicLink {
                    email: "expired-magic@example.com".into(),
                    token: expired.token,
                    callback_url: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                expired_result,
                Err(AuthFlowError::InvalidCredentials)
            ));
        }

        // r[verify auth.jwt.sign]
        // r[verify auth.jwt.verify]
        // r[verify auth.jwt.claims]
        // r[verify auth.jwt.expiry]
        // r[verify auth.jwt.issuer-audience]
        // r[verify auth.jwt.revoked-session]
        // r[verify auth.jwt.rotation]
        // r[verify auth.jwt.jwks]
        #[tokio::test]
        async fn jwt_signing_verification_rotation_and_session_revocation() {
            let auth = auth_with_rotated_jwt_keys();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "jwt@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let token = auth
                .issue_jwt(IssueJwt {
                    session_token: bundle.token.clone(),
                    audience: None,
                    expires_in_seconds: Some(300),
                    claims_json: Some(r#"{"scope":"read"}"#.into()),
                })
                .await
                .expect("issue jwt");
            assert_eq!(token.key_id, "v2");

            let verified = auth
                .verify_jwt(VerifyJwt {
                    token: token.token.clone(),
                    audience: None,
                })
                .await
                .expect("verify jwt");
            assert_eq!(verified.claims.iss, "architect-auth-test");
            assert_eq!(verified.claims.aud, "architect-clients");
            assert_eq!(verified.claims.sub, bundle.user.id.to_string());
            assert_eq!(verified.claims.sid, bundle.session.id.to_string());
            assert_eq!(
                verified
                    .claims
                    .extra
                    .as_ref()
                    .and_then(|extra| extra.get("scope"))
                    .and_then(serde_json::Value::as_str),
                Some("read")
            );

            let wrong_audience = auth
                .verify_jwt(VerifyJwt {
                    token: token.token.clone(),
                    audience: Some("wrong".into()),
                })
                .await;
            assert!(matches!(
                wrong_audience,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let key_set = auth.jwt_key_set();
            assert_eq!(key_set.keys.len(), 2);
            assert!(key_set.keys.iter().any(|key| key.kid == "v2" && key.active));
            assert!(
                key_set
                    .keys
                    .iter()
                    .any(|key| key.kid == "v1" && !key.active)
            );

            auth.sign_out(SignOut {
                token: bundle.token.clone(),
            })
            .await
            .expect("revoke session");
            let revoked = auth
                .verify_jwt(VerifyJwt {
                    token: token.token,
                    audience: None,
                })
                .await;
            assert!(matches!(revoked, Err(AuthFlowError::SessionExpired)));

            let expired_auth = self::auth();
            let expired_bundle = expired_auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "expired-jwt@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create expired jwt user");
            let expired = expired_auth
                .issue_jwt(IssueJwt {
                    session_token: expired_bundle.token,
                    audience: None,
                    expires_in_seconds: Some(-1),
                    claims_json: None,
                })
                .await
                .expect("issue expired jwt");
            assert!(matches!(
                expired_auth
                    .verify_jwt(VerifyJwt {
                        token: expired.token,
                        audience: None,
                    })
                    .await,
                Err(AuthFlowError::InvalidCredentials)
            ));
        }

        // r[verify auth.oidc.discovery]
        // r[verify auth.oidc.client-registration]
        // r[verify auth.oidc.authorization-code]
        // r[verify auth.oidc.pkce]
        // r[verify auth.oidc.token]
        // r[verify auth.oidc.refresh-token]
        // r[verify auth.oidc.userinfo]
        // r[verify auth.oidc.jwks]
        #[tokio::test]
        async fn oidc_provider_authorization_code_pkce_tokens_and_userinfo_round_trip() {
            let auth = auth_with_oidc_client();
            let discovery = auth.oidc_discovery();
            assert_eq!(discovery.issuer, "https://auth.example.com");
            assert_eq!(
                discovery.authorization_endpoint,
                "https://auth.example.com/oauth2/authorize"
            );
            assert!(discovery.scopes_supported.contains(&"openid".into()));
            assert!(
                discovery
                    .grant_types_supported
                    .contains(&"refresh_token".into())
            );

            let registered = auth
                .register_oidc_client(RegisterOidcClient {
                    redirect_uris: vec!["https://new-client.example.com/callback".into()],
                    client_name: Some("New client".into()),
                    scope: Some("openid email".into()),
                    token_endpoint_auth_method: Some("client_secret_post".into()),
                })
                .expect("dynamic registration");
            assert_eq!(registered.client_name, "New client");
            assert!(registered.client_secret.is_some());

            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "oidc@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: Some("OIDC User".into()),
                    username: None,
                    image: Some("https://example.com/avatar.png".into()),
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let verifier = "correct-horse-battery-staple-verifier";
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(sha2::Sha256::digest(verifier.as_bytes()));
            let authorization = auth
                .authorize_oidc(AuthorizeOidc {
                    session_token: bundle.token.clone(),
                    client_id: "dashboard".into(),
                    redirect_uri: "https://client.example.com/callback".into(),
                    response_type: "code".into(),
                    scope: Some("openid profile email offline_access".into()),
                    state: Some("state-1".into()),
                    nonce: Some("nonce-1".into()),
                    code_challenge: Some(challenge),
                    code_challenge_method: Some("S256".into()),
                    prompt: None,
                })
                .await
                .expect("authorize");
            assert!(authorization.redirect_uri.contains("code="));
            assert!(authorization.redirect_uri.contains("state=state-1"));

            let wrong_verifier = auth
                .exchange_oidc_token(ExchangeOidcToken {
                    grant_type: "authorization_code".into(),
                    code: Some(authorization.code.clone()),
                    redirect_uri: Some("https://client.example.com/callback".into()),
                    client_id: "dashboard".into(),
                    client_secret: Some("client-secret".into()),
                    code_verifier: Some("wrong".into()),
                    refresh_token: None,
                })
                .await;
            assert!(matches!(
                wrong_verifier,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let tokens = auth
                .exchange_oidc_token(ExchangeOidcToken {
                    grant_type: "authorization_code".into(),
                    code: Some(authorization.code.clone()),
                    redirect_uri: Some("https://client.example.com/callback".into()),
                    client_id: "dashboard".into(),
                    client_secret: Some("client-secret".into()),
                    code_verifier: Some(verifier.into()),
                    refresh_token: None,
                })
                .await
                .expect("exchange code");
            assert_eq!(tokens.token_type, "Bearer");
            assert!(tokens.refresh_token.is_some());

            let id_token = auth
                .verify_jwt(VerifyJwt {
                    token: tokens.id_token.clone(),
                    audience: Some("dashboard".into()),
                })
                .await
                .expect("verify id token");
            assert_eq!(id_token.claims.sub, bundle.user.id.to_string());
            assert_eq!(id_token.claims.iss, "https://auth.example.com");

            let user_info = auth
                .get_oidc_user_info(GetOidcUserInfo {
                    access_token: tokens.access_token,
                })
                .await
                .expect("userinfo");
            assert_eq!(user_info.sub, bundle.user.id.to_string());
            assert_eq!(user_info.email.as_deref(), Some("oidc@example.com"));
            assert_eq!(user_info.name.as_deref(), Some("OIDC User"));

            let refreshed = auth
                .exchange_oidc_token(ExchangeOidcToken {
                    grant_type: "refresh_token".into(),
                    code: None,
                    redirect_uri: None,
                    client_id: "dashboard".into(),
                    client_secret: Some("client-secret".into()),
                    code_verifier: None,
                    refresh_token: tokens.refresh_token,
                })
                .await
                .expect("refresh token");
            assert_eq!(refreshed.scope, "openid profile email offline_access");

            let key_set = auth.jwt_key_set();
            assert_eq!(key_set.keys.len(), 1);
        }

        // r[verify auth.oauth-proxy.metadata]
        // r[verify auth.oauth-proxy.state]
        // r[verify auth.oauth-proxy.callback-forwarding]
        // r[verify auth.oauth-proxy.redirect-policy]
        // r[verify auth.oauth-proxy.max-age]
        // r[verify auth.oauth-proxy.provider-composition]
        #[tokio::test]
        async fn oauth_proxy_packages_profile_and_rejects_untrusted_redirects() {
            let auth = auth_with_oauth_proxy();
            let metadata = auth.oauth_proxy_metadata();
            assert!(metadata.should_proxy);
            assert_eq!(
                metadata.proxy_callback_url,
                "https://preview.example.com/auth/oauth-proxy-callback"
            );
            assert!(
                metadata
                    .providers
                    .iter()
                    .any(|provider| provider.id == "google")
            );

            let authorization = auth
                .begin_oauth_proxy_authorization(BeginOAuthProxyAuthorization {
                    provider_id: "google".into(),
                    callback_url: "https://preview.example.com/dashboard".into(),
                })
                .await
                .expect("begin proxy");
            assert_eq!(authorization.provider_id, "google");
            auth.verify_oauth_state(VerifyOAuthState {
                provider_id: "google".into(),
                state: authorization.state.clone(),
            })
            .await
            .expect("proxy state composes with oauth state");

            let forwarding = auth
                .forward_oauth_proxy_callback(ForwardOAuthProxyCallback {
                    provider_id: "google".into(),
                    state: authorization.state.clone(),
                    callback_url: "https://preview.example.com/dashboard".into(),
                    profile_json: r#"{"email":"proxy@example.com"}"#.into(),
                })
                .expect("forward callback");
            assert!(forwarding.redirect_url.contains("callbackURL="));
            assert!(forwarding.redirect_url.contains("profile="));

            let profile = auth
                .consume_oauth_proxy_callback(ConsumeOAuthProxyCallback {
                    callback_url: "https://preview.example.com/dashboard".into(),
                    encrypted_profile: forwarding.encrypted_profile,
                })
                .expect("consume callback");
            assert_eq!(profile.provider_id, "google");
            assert_eq!(profile.profile_json, r#"{"email":"proxy@example.com"}"#);

            let rejected_begin = auth
                .begin_oauth_proxy_authorization(BeginOAuthProxyAuthorization {
                    provider_id: "google".into(),
                    callback_url: "https://evil.example/dashboard".into(),
                })
                .await;
            assert!(matches!(
                rejected_begin,
                Err(AuthFlowError::PermissionDenied)
            ));

            let rejected_forward = auth.forward_oauth_proxy_callback(ForwardOAuthProxyCallback {
                provider_id: "google".into(),
                state: authorization.state,
                callback_url: "https://evil.example/dashboard".into(),
                profile_json: "{}".into(),
            });
            assert!(matches!(
                rejected_forward,
                Err(AuthFlowError::PermissionDenied)
            ));
        }

        // r[verify auth.onetap.token-validation]
        // r[verify auth.onetap.existing-user]
        // r[verify auth.onetap.new-user]
        // r[verify auth.onetap.disabled-signup]
        // r[verify auth.onetap.session]
        #[tokio::test]
        async fn one_tap_validates_tokens_links_existing_users_and_honors_signup_policy() {
            let auth = auth_with_one_tap(false);
            let created = auth
                .one_tap_callback(OneTapCallback {
                    id_token: one_tap_id_token(
                        "one-tap@example.com",
                        true,
                        "google-sub-1",
                        "google-client",
                    ),
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("one-tap".into()),
                })
                .await
                .expect("create one tap user");
            assert_eq!(created.user.email.as_deref(), Some("one-tap@example.com"));
            assert_eq!(created.session.user_agent.as_deref(), Some("one-tap"));

            let existing = auth
                .one_tap_callback(OneTapCallback {
                    id_token: one_tap_id_token(
                        "one-tap@example.com",
                        true,
                        "google-sub-1",
                        "google-client",
                    ),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("existing linked account");
            assert_eq!(existing.user.id, created.user.id);

            let verified_local = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "verified-local@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create local user");
            auth.storage
                .mark_email_verified(verified_local.user.id)
                .await
                .expect("mark local verified");
            let linked = auth
                .one_tap_callback(OneTapCallback {
                    id_token: one_tap_id_token(
                        "verified-local@example.com",
                        true,
                        "google-sub-2",
                        "google-client",
                    ),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("link verified local user");
            assert_eq!(linked.user.id, verified_local.user.id);

            let invalid_audience = auth
                .one_tap_callback(OneTapCallback {
                    id_token: one_tap_id_token(
                        "invalid@example.com",
                        true,
                        "google-sub-3",
                        "wrong-client",
                    ),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                invalid_audience,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let disabled = auth_with_one_tap(true)
                .one_tap_callback(OneTapCallback {
                    id_token: one_tap_id_token(
                        "new-disabled@example.com",
                        true,
                        "google-sub-4",
                        "google-client",
                    ),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(disabled, Err(AuthFlowError::InvalidCredentials)));
        }

        // r[verify auth.ott.create]
        // r[verify auth.ott.consume]
        // r[verify auth.ott.expire]
        // r[verify auth.ott.replay]
        // r[verify auth.ott.revoke]
        // r[verify auth.ott.scope]
        // r[verify auth.ott.metadata]
        #[tokio::test]
        async fn one_time_tokens_are_scoped_single_use_expiring_and_revocable() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "ott@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let token = auth
                .generate_one_time_token(GenerateOneTimeToken {
                    session_token: bundle.token.clone(),
                    expires_in_seconds: Some(300),
                    scope: Some("handoff".into()),
                    metadata_json: Some(r#"{"target":"console"}"#.into()),
                })
                .await
                .expect("generate token");
            let wrong_scope = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: token.token.clone(),
                    scope: Some("other".into()),
                })
                .await;
            assert!(matches!(wrong_scope, Err(AuthFlowError::PermissionDenied)));
            let replay = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: token.token,
                    scope: Some("handoff".into()),
                })
                .await;
            assert!(matches!(replay, Err(AuthFlowError::InvalidCredentials)));

            let token = auth
                .generate_one_time_token(GenerateOneTimeToken {
                    session_token: bundle.token.clone(),
                    expires_in_seconds: Some(300),
                    scope: Some("handoff".into()),
                    metadata_json: Some(r#"{"target":"console"}"#.into()),
                })
                .await
                .expect("generate token");
            let verified = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: token.token.clone(),
                    scope: Some("handoff".into()),
                })
                .await
                .expect("verify token");
            assert_eq!(verified.user.id, bundle.user.id);
            assert_eq!(verified.scope.as_deref(), Some("handoff"));
            assert_eq!(
                verified.metadata_json.as_deref(),
                Some(r#"{"target":"console"}"#)
            );
            let replay = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: token.token,
                    scope: Some("handoff".into()),
                })
                .await;
            assert!(matches!(replay, Err(AuthFlowError::InvalidCredentials)));

            let expired = auth
                .generate_one_time_token(GenerateOneTimeToken {
                    session_token: bundle.token.clone(),
                    expires_in_seconds: Some(-1),
                    scope: None,
                    metadata_json: None,
                })
                .await
                .expect("generate expired token");
            let expired_result = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: expired.token,
                    scope: None,
                })
                .await;
            assert!(matches!(
                expired_result,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let revoked = auth
                .generate_one_time_token(GenerateOneTimeToken {
                    session_token: bundle.token,
                    expires_in_seconds: Some(300),
                    scope: None,
                    metadata_json: None,
                })
                .await
                .expect("generate revoked token");
            auth.revoke_one_time_token(RevokeOneTimeToken {
                token: revoked.token.clone(),
            })
            .await
            .expect("revoke token");
            let revoked_result = auth
                .verify_one_time_token(VerifyOneTimeToken {
                    token: revoked.token,
                    scope: None,
                })
                .await;
            assert!(matches!(
                revoked_result,
                Err(AuthFlowError::InvalidCredentials)
            ));
        }

        // r[verify auth.multisession.list]
        // r[verify auth.multisession.set-active]
        // r[verify auth.multisession.revoke]
        // r[verify auth.multisession.current-session]
        // r[verify auth.multisession.permission-isolation]
        // r[verify auth.multisession.no-forged-sessions]
        #[tokio::test]
        async fn multi_session_switching_revocation_and_current_session_are_isolated() {
            let auth = auth();
            let first = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "multi-one@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: Some("device".into()),
                })
                .await
                .expect("create first user");
            let second = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "multi-two@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: Some("device".into()),
                })
                .await
                .expect("create second user");

            let listed = auth
                .list_device_sessions(ListDeviceSessions {
                    session_tokens: vec![
                        first.token.clone(),
                        second.token.clone(),
                        "forged".into(),
                    ],
                })
                .await
                .expect("list device sessions");
            assert_eq!(listed.sessions.len(), 2);
            assert!(
                listed
                    .sessions
                    .iter()
                    .any(|item| item.user.id == first.user.id)
            );
            assert!(
                listed
                    .sessions
                    .iter()
                    .any(|item| item.user.id == second.user.id)
            );

            let active = auth
                .set_active_device_session(SetActiveDeviceSession {
                    current_session_token: Some(first.token.clone()),
                    session_token: second.token.clone(),
                    session_tokens: vec![first.token.clone(), second.token.clone()],
                })
                .await
                .expect("set active session");
            assert_eq!(active.user.id, second.user.id);

            auth.require_admin(&second.token)
                .await
                .expect_err("second user is not admin");
            auth.storage
                .update_user_role(first.user.id, Some("admin".into()))
                .await
                .expect("make first user admin");
            auth.require_admin(&first.token)
                .await
                .expect("first user is admin");
            auth.require_admin(&second.token)
                .await
                .expect_err("switching active session does not leak admin role");

            let forged = auth
                .set_active_device_session(SetActiveDeviceSession {
                    current_session_token: Some(first.token.clone()),
                    session_token: "forged".into(),
                    session_tokens: vec![first.token.clone(), second.token.clone()],
                })
                .await;
            assert!(matches!(forged, Err(AuthFlowError::InvalidCredentials)));

            let revoked = auth
                .revoke_device_session(RevokeDeviceSession {
                    current_session_token: Some(second.token.clone()),
                    session_token: second.token.clone(),
                    session_tokens: vec![first.token.clone(), second.token.clone()],
                })
                .await
                .expect("revoke active device session");
            assert!(revoked.revoked);
            assert_eq!(
                revoked.next_active.as_ref().map(|session| session.user.id),
                Some(first.user.id)
            );
            auth.current_session(CurrentSession {
                token: second.token,
            })
            .await
            .expect_err("revoked session is inactive");
            auth.current_session(CurrentSession { token: first.token })
                .await
                .expect("next active session remains current");
        }

        // r[verify auth.mcp.session]
        // r[verify auth.mcp.permissions]
        #[tokio::test]
        async fn mcp_authorization_uses_sessions_and_service_permission_checks() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "mcp-owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let outsider = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "mcp-outsider@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create outsider");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "MCP Org".into(),
                    slug: "mcp-org".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            let allowed = auth
                .authorize_mcp_request(AuthorizeMcpRequest {
                    session_token: Some(owner.token.clone()),
                    authorization_header: None,
                    organization_id: Some(organization.organization.id),
                    resource: Some("organization".into()),
                    action: Some("update".into()),
                })
                .await
                .expect("owner mcp authorization");
            assert!(allowed.allowed);
            assert_eq!(allowed.user_id, owner.user.id);
            assert_eq!(allowed.session_id, Some(owner.session.id));
            assert_eq!(allowed.api_key_id, None);

            let denied = auth
                .authorize_mcp_request(AuthorizeMcpRequest {
                    session_token: Some(outsider.token),
                    authorization_header: None,
                    organization_id: Some(organization.organization.id),
                    resource: Some("organization".into()),
                    action: Some("update".into()),
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            let session_bearer = auth
                .authorize_mcp_request(AuthorizeMcpRequest {
                    session_token: None,
                    authorization_header: Some(format!("Bearer {}", owner.token)),
                    organization_id: Some(organization.organization.id),
                    resource: Some("organization".into()),
                    action: Some("update".into()),
                })
                .await
                .expect("session bearer mcp authorization");
            assert_eq!(session_bearer.session_id, Some(owner.session.id));

            let api_key = auth
                .create_api_key(CreateApiKey {
                    session_token: owner.token,
                    name: Some("mcp-key".into()),
                    expires_at: None,
                    permissions_json: Some(r#"{"tool":["call"]}"#.into()),
                    rate_limit_time_window: None,
                    rate_limit_max: None,
                    metadata_json: None,
                })
                .await
                .expect("create mcp api key");
            let api_key_allowed = auth
                .authorize_mcp_request(AuthorizeMcpRequest {
                    session_token: None,
                    authorization_header: Some(format!("Bearer {}", api_key.key.clone())),
                    organization_id: None,
                    resource: Some("tool".into()),
                    action: Some("call".into()),
                })
                .await
                .expect("api key mcp authorization");
            assert_eq!(api_key_allowed.session_id, None);
            assert_eq!(api_key_allowed.api_key_id, Some(api_key.api_key.id));

            let api_key_denied = auth
                .authorize_mcp_request(AuthorizeMcpRequest {
                    session_token: None,
                    authorization_header: Some(format!("Bearer {}", api_key.key)),
                    organization_id: None,
                    resource: Some("tool".into()),
                    action: Some("delete".into()),
                })
                .await;
            assert!(matches!(
                api_key_denied,
                Err(AuthFlowError::PermissionDenied)
            ));
        }

        // r[verify auth.lastlogin.track-email]
        // r[verify auth.lastlogin.track-oauth]
        // r[verify auth.lastlogin.track-passkey]
        // r[verify auth.lastlogin.track-email-otp]
        // r[verify auth.lastlogin.track-magic-link]
        // r[verify auth.lastlogin.track-anonymous-upgrade]
        // r[verify auth.lastlogin.query]
        // r[verify auth.lastlogin.clear]
        // r[verify auth.lastlogin.cookie-config]
        #[tokio::test]
        async fn last_login_method_tracks_query_and_clear_across_login_paths() {
            let auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .oauth_signup_enabled(true)
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth");

            let email = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "last-email@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create email user");
            assert_last_login_method(&auth, &email.token, Some("email")).await;

            let signed_in = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "last-email@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in email user");
            assert_last_login_method(&auth, &signed_in.token, Some("email")).await;

            let oauth = auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "github".into(),
                    account_id: "gh-last-login".into(),
                    email: Some("last-oauth@example.com".into()),
                    email_verified: true,
                    name: None,
                    image: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in oauth user");
            assert_last_login_method(&auth, &oauth.token, Some("github")).await;

            let otp = auth
                .send_email_otp(SendEmailOtp {
                    email: "last-otp@example.com".into(),
                })
                .await
                .expect("send email otp");
            let otp = auth
                .verify_email_otp(VerifyEmailOtp {
                    email: "last-otp@example.com".into(),
                    otp: otp.token,
                    create_session: true,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("verify email otp");
            assert_last_login_method(
                &auth,
                otp.token.as_deref().expect("otp session token"),
                Some("email-otp"),
            )
            .await;

            let magic = auth
                .send_magic_link(SendMagicLink {
                    email: "last-magic@example.com".into(),
                    callback_url: None,
                })
                .await
                .expect("send magic link");
            let magic = auth
                .verify_magic_link(VerifyMagicLink {
                    email: "last-magic@example.com".into(),
                    token: magic.token,
                    callback_url: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("verify magic link");
            assert_last_login_method(&auth, &magic.token, Some("magic-link")).await;

            let anonymous = auth
                .sign_in_anonymous(SignInAnonymous {
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in anonymous");
            assert_last_login_method(&auth, &anonymous.token, Some("anonymous")).await;
            let upgraded = auth
                .link_anonymous_email_password(LinkAnonymousEmailPassword {
                    session_token: anonymous.token,
                    email: "last-anonymous@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                })
                .await
                .expect("upgrade anonymous user");
            assert_last_login_method(&auth, &upgraded.token, Some("email")).await;

            let passkey_challenge = auth
                .begin_passkey_registration(BeginPasskeyRegistration {
                    session_token: email.token.clone(),
                })
                .await
                .expect("begin passkey registration");
            auth.complete_passkey_registration(CompletePasskeyRegistration {
                session_token: email.token.clone(),
                challenge: passkey_challenge.token,
                rp_id: "localhost".into(),
                origin: "http://localhost:3000".into(),
                name: "laptop".into(),
                credential_id: "last-login-passkey".into(),
                public_key: "public-key".into(),
                counter: 1,
                device_type: "platform".into(),
                backed_up: true,
                transports: Some("internal".into()),
            })
            .await
            .expect("register passkey");
            let passkey_challenge = auth
                .begin_passkey_authentication(BeginPasskeyAuthentication {
                    credential_id: "last-login-passkey".into(),
                })
                .await
                .expect("begin passkey authentication");
            let passkey = auth
                .complete_passkey_authentication(CompletePasskeyAuthentication {
                    credential_id: "last-login-passkey".into(),
                    challenge: passkey_challenge.token,
                    rp_id: "localhost".into(),
                    origin: "http://localhost:3000".into(),
                    counter: 2,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("complete passkey authentication");
            assert_last_login_method(&auth, &passkey.token, Some("passkey")).await;

            let cleared = auth
                .clear_last_login_method(ClearLastLoginMethod {
                    session_token: passkey.token.clone(),
                })
                .await
                .expect("clear last login method");
            assert_eq!(cleared.method, None);
            assert_eq!(cleared.cookie_name, "better-auth.last_used_login_method");
            assert_eq!(cleared.max_age_seconds, 60 * 60 * 24 * 30);
            assert_last_login_method(&auth, &passkey.token, None).await;
        }

        // r[verify auth.oauth.link-authenticated]
        // r[verify auth.oauth.provider-account-unique]
        // r[verify auth.oauth.signin-existing-account]
        // r[verify auth.oauth.unlink-last-credential]
        // r[verify auth.oauth.access-token]
        // r[verify auth.oauth.refresh-token]
        #[tokio::test]
        async fn oauth_link_and_signin_round_trip() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            auth.link_oauth_account(LinkOAuthAccount {
                session_token: bundle.token.clone(),
                provider_id: "github".into(),
                account_id: "123".into(),
                access_token_ciphertext: Some("access-token-one".into()),
                refresh_token_ciphertext: Some("refresh-token-one".into()),
                id_token_ciphertext: None,
                scope: Some("read:user".into()),
            })
            .await
            .expect("link oauth account");

            let duplicate_session = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in")
                .token;
            let duplicate = auth
                .link_oauth_account(LinkOAuthAccount {
                    session_token: duplicate_session,
                    provider_id: "github".into(),
                    account_id: "123".into(),
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: None,
                    scope: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let oauth = auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "github".into(),
                    account_id: "123".into(),
                    email: None,
                    email_verified: false,
                    name: None,
                    image: None,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("oauth-test".into()),
                })
                .await
                .expect("oauth sign in");
            assert_eq!(oauth.user.id, bundle.user.id);
            assert_eq!(oauth.session.user_agent.as_deref(), Some("oauth-test"));
            let access_token = auth
                .get_oauth_access_token(GetOAuthAccessToken {
                    session_token: bundle.token.clone(),
                    provider_id: "github".into(),
                    account_id: "123".into(),
                })
                .await
                .expect("get oauth access token");
            assert_eq!(
                access_token.access_token.as_deref(),
                Some("access-token-one")
            );
            assert_eq!(access_token.scope.as_deref(), Some("read:user"));

            let refreshed = auth
                .refresh_oauth_token(RefreshOAuthToken {
                    session_token: bundle.token.clone(),
                    provider_id: "github".into(),
                    account_id: "123".into(),
                    access_token: "access-token-two".into(),
                    refresh_token: None,
                    id_token: Some("id-token-two".into()),
                    access_token_expires_at: Some(Utc::now() + Duration::hours(1)),
                    refresh_token_expires_at: None,
                    scope: Some("user:email".into()),
                })
                .await
                .expect("refresh oauth token");
            assert_eq!(refreshed.access_token.as_deref(), Some("access-token-two"));
            assert_eq!(refreshed.scope.as_deref(), Some("user:email"));
            let access_token = auth
                .get_oauth_access_token(GetOAuthAccessToken {
                    session_token: bundle.token.clone(),
                    provider_id: "github".into(),
                    account_id: "123".into(),
                })
                .await
                .expect("get refreshed oauth access token");
            assert_eq!(
                access_token.access_token.as_deref(),
                Some("access-token-two")
            );

            auth.unlink_oauth_account(UnlinkOAuthAccount {
                session_token: auth
                    .sign_in_email_password(SignInEmailPassword {
                        email: "user@example.com".into(),
                        password: "correct horse battery staple".into(),
                        ip_address: None,
                        user_agent: None,
                    })
                    .await
                    .expect("password sign in")
                    .token,
                provider_id: "github".into(),
                account_id: "123".into(),
            })
            .await
            .expect("unlink oauth when password remains");
            let unlinked = auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "github".into(),
                    account_id: "123".into(),
                    email: None,
                    email_verified: false,
                    name: None,
                    image: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(unlinked, Err(AuthFlowError::InvalidCredentials)));
        }

        // r[verify auth.oauth.signin-new-account]
        // r[verify auth.oauth.email-trust]
        #[tokio::test]
        async fn oauth_signup_requires_config_and_trusts_only_verified_provider_email() {
            let disabled = auth();
            let rejected = disabled
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "github".into(),
                    account_id: "new".into(),
                    email: Some("User@Example.COM".into()),
                    email_verified: true,
                    name: Some("User".into()),
                    image: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(rejected, Err(AuthFlowError::InvalidCredentials)));

            let auth = ArchitectAuth::builder()
                .secret("a-secret-at-least-32-bytes-long!!")
                .oauth_signup_enabled(true)
                .storage(MemoryStorage::default())
                .build()
                .expect("build auth");
            let unverified = auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "github".into(),
                    account_id: "new".into(),
                    email: Some("User@Example.COM".into()),
                    email_verified: false,
                    name: Some("User".into()),
                    image: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("oauth signup");
            assert_eq!(unverified.user.email.as_deref(), Some("User@example.com"));
            assert!(!unverified.user.email_verified);

            let verified = auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: "google".into(),
                    account_id: "new-verified".into(),
                    email: Some("verified@example.com".into()),
                    email_verified: true,
                    name: Some("Verified".into()),
                    image: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("verified oauth signup");
            assert!(verified.user.email_verified);

            let unlink_last = auth
                .unlink_oauth_account(UnlinkOAuthAccount {
                    session_token: unverified.token,
                    provider_id: "github".into(),
                    account_id: "new".into(),
                })
                .await;
            assert!(matches!(unlink_last, Err(AuthFlowError::InvalidInput(_))));
        }

        // r[verify auth.oauth.token-encryption]
        #[tokio::test]
        async fn oauth_token_storage_encrypts_before_persisting() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            auth.link_oauth_account(LinkOAuthAccount {
                session_token: bundle.token,
                provider_id: "github".into(),
                account_id: "123".into(),
                access_token_ciphertext: Some("plain-access-token".into()),
                refresh_token_ciphertext: None,
                id_token_ciphertext: None,
                scope: None,
            })
            .await
            .expect("link oauth account with token storage");

            let account = auth
                .storage
                .find_account_by_provider_account("github", "123")
                .await
                .expect("load oauth account")
                .expect("oauth account exists");
            let stored = account
                .access_token_ciphertext
                .expect("stored access token ciphertext");
            assert_ne!(stored, "plain-access-token");
            assert!(!stored.contains("plain-access-token"));
        }

        // r[verify auth.oauth.state-csrf]
        // r[verify auth.oauth.provider-registry]
        // r[verify auth.oauth.generic-provider]
        #[tokio::test]
        async fn oauth_state_is_hashed_expiring_and_single_use() {
            let providers = crate::flows::oauth::built_in_oauth_providers();
            assert!(providers.iter().any(|provider| provider.id == "google"));
            assert!(providers.iter().any(|provider| provider.id == "github"));
            assert!(providers.iter().any(|provider| provider.id == "discord"));
            let generic = crate::flows::oauth::generic_oauth_provider(
                "custom",
                "https://provider.example/authorize",
                "https://provider.example/token",
                "https://provider.example/userinfo",
                &["openid", "email"],
            );
            assert_eq!(generic.id, "custom");
            assert_eq!(generic.scopes, &["openid", "email"]);

            let auth = auth();
            let state = auth
                .begin_oauth_authorization(BeginOAuthAuthorization {
                    provider_id: "github".into(),
                })
                .await
                .expect("begin oauth authorization");
            assert_eq!(state.identifier, "oauth-state:github");
            assert!(!state.token.is_empty());

            let stored = auth
                .storage
                .inner
                .lock()
                .expect("lock memory storage")
                .verifications
                .values()
                .find(|verification| verification.identifier == state.identifier)
                .cloned()
                .expect("stored oauth state");
            assert_ne!(stored.value_hash, state.token);
            assert!(stored.expires_at > Utc::now());

            let wrong_provider = auth
                .verify_oauth_state(VerifyOAuthState {
                    provider_id: "google".into(),
                    state: state.token.clone(),
                })
                .await;
            assert!(matches!(
                wrong_provider,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let wrong_state = auth
                .verify_oauth_state(VerifyOAuthState {
                    provider_id: "github".into(),
                    state: "wrong-state".into(),
                })
                .await;
            assert!(matches!(
                wrong_state,
                Err(AuthFlowError::InvalidCredentials)
            ));

            auth.verify_oauth_state(VerifyOAuthState {
                provider_id: "github".into(),
                state: state.token.clone(),
            })
            .await
            .expect("verify oauth state");

            let reused = auth
                .verify_oauth_state(VerifyOAuthState {
                    provider_id: "github".into(),
                    state: state.token,
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));
        }

        // r[verify auth.passkey.challenge-random]
        // r[verify auth.passkey.challenge-expiry]
        // r[verify auth.passkey.rp-origin]
        // r[verify auth.passkey.credential-unique]
        // r[verify auth.passkey.user-match]
        // r[verify auth.passkey.counter]
        // r[verify auth.passkey.transports]
        // r[verify auth.passkey.list]
        #[tokio::test]
        async fn passkey_registration_and_authentication_enforce_domain_rules() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let challenge = auth
                .begin_passkey_registration(BeginPasskeyRegistration {
                    session_token: bundle.token.clone(),
                })
                .await
                .expect("begin passkey registration");
            assert!(challenge.identifier.starts_with("passkey-registration:"));
            assert!(!challenge.token.is_empty());

            let bad_origin = auth
                .complete_passkey_registration(CompletePasskeyRegistration {
                    session_token: bundle.token.clone(),
                    challenge: challenge.token.clone(),
                    rp_id: "localhost".into(),
                    origin: "https://evil.example".into(),
                    name: "laptop".into(),
                    credential_id: "credential-1".into(),
                    public_key: "public-key".into(),
                    counter: 1,
                    device_type: "platform".into(),
                    backed_up: true,
                    transports: Some("internal,hybrid".into()),
                })
                .await;
            assert!(matches!(bad_origin, Err(AuthFlowError::PermissionDenied)));

            let passkey = auth
                .complete_passkey_registration(CompletePasskeyRegistration {
                    session_token: bundle.token.clone(),
                    challenge: challenge.token,
                    rp_id: "localhost".into(),
                    origin: "http://localhost:3000".into(),
                    name: "laptop".into(),
                    credential_id: "credential-1".into(),
                    public_key: "public-key".into(),
                    counter: 1,
                    device_type: "platform".into(),
                    backed_up: true,
                    transports: Some("internal,hybrid".into()),
                })
                .await
                .expect("complete passkey registration");
            assert_eq!(passkey.user_id, bundle.user.id);
            assert_eq!(passkey.transports.as_deref(), Some("internal,hybrid"));
            let passkeys = auth
                .list_passkeys(ListPasskeys {
                    session_token: bundle.token.clone(),
                })
                .await
                .expect("list passkeys");
            assert_eq!(passkeys.len(), 1);
            assert_eq!(passkeys[0].credential_id, "credential-1");

            let duplicate_challenge = auth
                .begin_passkey_registration(BeginPasskeyRegistration {
                    session_token: bundle.token.clone(),
                })
                .await
                .expect("begin duplicate passkey registration");
            let duplicate = auth
                .complete_passkey_registration(CompletePasskeyRegistration {
                    session_token: bundle.token.clone(),
                    challenge: duplicate_challenge.token,
                    rp_id: "localhost".into(),
                    origin: "http://localhost:3000".into(),
                    name: "duplicate".into(),
                    credential_id: "credential-1".into(),
                    public_key: "public-key".into(),
                    counter: 1,
                    device_type: "platform".into(),
                    backed_up: true,
                    transports: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let auth_challenge = auth
                .begin_passkey_authentication(BeginPasskeyAuthentication {
                    credential_id: "credential-1".into(),
                })
                .await
                .expect("begin passkey authentication");
            let stale_counter = auth
                .complete_passkey_authentication(CompletePasskeyAuthentication {
                    credential_id: "credential-1".into(),
                    challenge: auth_challenge.token.clone(),
                    rp_id: "localhost".into(),
                    origin: "http://localhost:3000".into(),
                    counter: 1,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                stale_counter,
                Err(AuthFlowError::InvalidCredentials)
            ));

            let auth_challenge = auth
                .begin_passkey_authentication(BeginPasskeyAuthentication {
                    credential_id: "credential-1".into(),
                })
                .await
                .expect("begin second passkey authentication");
            let session = auth
                .complete_passkey_authentication(CompletePasskeyAuthentication {
                    credential_id: "credential-1".into(),
                    challenge: auth_challenge.token,
                    rp_id: "localhost".into(),
                    origin: "http://localhost:3000".into(),
                    counter: 2,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("passkey-test".into()),
                })
                .await
                .expect("complete passkey authentication");
            assert_eq!(session.user.id, bundle.user.id);
            assert_eq!(session.session.user_agent.as_deref(), Some("passkey-test"));
        }

        // r[verify auth.passkey.delete-last-credential]
        #[tokio::test]
        async fn passkey_delete_rejects_last_signin_credential() {
            let auth = auth();
            let user = auth
                .storage
                .create_user(AuthUserCreate {
                    email: Some("passkey@example.com".into()),
                    name: None,
                    email_verified: true,
                    image: None,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: None,
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json: "{}".into(),
                })
                .await
                .expect("create passkey-only user");
            let session = auth
                .issue_session(user.clone(), None, None, None, None)
                .await
                .expect("issue passkey-only setup session");
            let challenge = auth
                .begin_passkey_registration(BeginPasskeyRegistration {
                    session_token: session.token.clone(),
                })
                .await
                .expect("begin registration");
            auth.complete_passkey_registration(CompletePasskeyRegistration {
                session_token: session.token.clone(),
                challenge: challenge.token,
                rp_id: "localhost".into(),
                origin: "http://localhost:3000".into(),
                name: "security key".into(),
                credential_id: "only-passkey".into(),
                public_key: "public-key".into(),
                counter: 1,
                device_type: "cross-platform".into(),
                backed_up: false,
                transports: Some("usb".into()),
            })
            .await
            .expect("complete registration");

            let rejected = auth
                .delete_passkey(DeletePasskey {
                    session_token: session.token.clone(),
                    credential_id: "only-passkey".into(),
                })
                .await;
            assert!(matches!(rejected, Err(AuthFlowError::InvalidInput(_))));

            auth.storage
                .create_account(AuthAccountCreate {
                    account_id: "passkey@example.com".into(),
                    provider_id: super::PASSWORD_PROVIDER_ID.into(),
                    user_id: user.id,
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: None,
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: None,
                    password_hash: Some("existing-password-hash".into()),
                })
                .await
                .expect("add password credential");
            auth.delete_passkey(DeletePasskey {
                session_token: session.token,
                credential_id: "only-passkey".into(),
            })
            .await
            .expect("delete passkey when password remains");
        }

        // r[verify auth.device.create]
        // r[verify auth.device.verify]
        // r[verify auth.device.approve-deny]
        // r[verify auth.device.polling]
        // r[verify auth.device.expiry]
        #[tokio::test]
        async fn device_authorization_handles_polling_approval_denial_and_expiry() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "device@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let pending = auth
                .create_device_authorization(CreateDeviceAuthorization {
                    client_id: "television".into(),
                    scope: Some("openid profile".into()),
                    expires_in_seconds: Some(60),
                    interval_seconds: Some(5),
                })
                .await
                .expect("create device authorization");
            assert_eq!(pending.expires_in_seconds, 60);
            assert_eq!(pending.interval_seconds, 5);
            assert!(
                pending
                    .verification_uri_complete
                    .contains(&pending.user_code)
            );

            let verification = auth
                .verify_device_code(VerifyDeviceCode {
                    user_code: pending.user_code.clone(),
                })
                .await
                .expect("verify user code");
            assert_eq!(verification.client_id, "television");
            assert_eq!(verification.scope.as_deref(), Some("openid profile"));

            let authorization_pending = auth
                .poll_device_token(PollDeviceToken {
                    device_code: pending.device_code.clone(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                authorization_pending,
                Err(AuthFlowError::VerificationRequired)
            ));
            let slow_down = auth
                .poll_device_token(PollDeviceToken {
                    device_code: pending.device_code,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(
                slow_down,
                Err(AuthFlowError::InvalidInput(message)) if message == "slow_down"
            ));

            let approved = auth
                .create_device_authorization(CreateDeviceAuthorization {
                    client_id: "cli".into(),
                    scope: None,
                    expires_in_seconds: Some(60),
                    interval_seconds: Some(5),
                })
                .await
                .expect("create approval device authorization");
            auth.approve_device_code(ApproveDeviceCode {
                session_token: bundle.token.clone(),
                user_code: approved.user_code,
            })
            .await
            .expect("approve device code");
            let session = auth
                .poll_device_token(PollDeviceToken {
                    device_code: approved.device_code,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("device-client".into()),
                })
                .await
                .expect("poll approved token");
            assert_eq!(session.user.id, bundle.user.id);
            assert_eq!(session.session.user_agent.as_deref(), Some("device-client"));

            let denied = auth
                .create_device_authorization(CreateDeviceAuthorization {
                    client_id: "denied-client".into(),
                    scope: None,
                    expires_in_seconds: Some(60),
                    interval_seconds: Some(5),
                })
                .await
                .expect("create denied device authorization");
            auth.deny_device_code(DenyDeviceCode {
                user_code: denied.user_code,
            })
            .await
            .expect("deny device code");
            let denied_poll = auth
                .poll_device_token(PollDeviceToken {
                    device_code: denied.device_code,
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(denied_poll, Err(AuthFlowError::PermissionDenied)));

            let expired_auth = self::auth();
            let expired = expired_auth
                .create_device_authorization(CreateDeviceAuthorization {
                    client_id: "expired-client".into(),
                    scope: None,
                    expires_in_seconds: Some(60),
                    interval_seconds: Some(5),
                })
                .await
                .expect("create expired device authorization");
            {
                let mut inner = expired_auth
                    .storage
                    .inner
                    .lock()
                    .expect("lock memory storage");
                for verification in inner.verifications.values_mut() {
                    verification.expires_at = Utc::now() - Duration::seconds(1);
                }
            }
            assert!(matches!(
                expired_auth
                    .verify_device_code(VerifyDeviceCode {
                        user_code: expired.user_code
                    })
                    .await,
                Err(AuthFlowError::InvalidCredentials)
            ));
            assert!(matches!(
                expired_auth
                    .poll_device_token(PollDeviceToken {
                        device_code: expired.device_code,
                        ip_address: None,
                        user_agent: None,
                    })
                    .await,
                Err(AuthFlowError::InvalidCredentials)
            ));
        }

        // r[verify auth.anonymous.signin]
        // r[verify auth.anonymous.policy]
        // r[verify auth.anonymous.link]
        // r[verify auth.anonymous.revoke-obsolete]
        // r[verify auth.anonymous.cleanup]
        #[tokio::test]
        async fn anonymous_user_upgrades_without_bypassing_policy() {
            let auth = auth();
            let anonymous = auth
                .sign_in_anonymous(SignInAnonymous {
                    metadata_json: Some(r#"{"cart_id":"cart-1"}"#.into()),
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("anon-test".into()),
                })
                .await
                .expect("sign in anonymous");
            assert!(anonymous.user.email.is_none());
            assert_eq!(anonymous.user.role.as_deref(), Some("anonymous"));
            let metadata = serde_json::from_str::<serde_json::Value>(&anonymous.user.metadata_json)
                .expect("anonymous metadata");
            assert_eq!(metadata["anonymous"], true);
            assert_eq!(metadata["cart_id"], "cart-1");

            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token,
                    name: "Owner Org".into(),
                    slug: "owner-org".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create owner org");
            let denied = auth
                .authorize_organization_action(AuthorizeOrganizationAction {
                    session_token: anonymous.token.clone(),
                    organization_id: organization.organization.id,
                    resource: "organization".into(),
                    action: "update".into(),
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            let upgraded = auth
                .link_anonymous_email_password(LinkAnonymousEmailPassword {
                    session_token: anonymous.token.clone(),
                    email: "upgraded@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: Some("Upgraded".into()),
                    username: Some("upgraded".into()),
                    image: Some("https://example.com/avatar.png".into()),
                })
                .await
                .expect("upgrade anonymous");
            assert_eq!(upgraded.user.id, anonymous.user.id);
            assert_eq!(upgraded.user.email.as_deref(), Some("upgraded@example.com"));
            let upgraded_metadata =
                serde_json::from_str::<serde_json::Value>(&upgraded.user.metadata_json)
                    .expect("upgraded metadata");
            assert_eq!(upgraded_metadata["anonymous"], false);
            assert_eq!(upgraded_metadata["upgraded_from_anonymous"], true);
            assert_eq!(upgraded_metadata["cart_id"], "cart-1");
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: anonymous.token
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));
            auth.sign_in_email_password(SignInEmailPassword {
                email: "upgraded@example.com".into(),
                password: "correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("upgraded credential signs in");

            let stale = auth
                .sign_in_anonymous(SignInAnonymous {
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("stale anonymous");
            let recent = auth
                .sign_in_anonymous(SignInAnonymous {
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("recent anonymous");
            {
                let mut inner = auth.storage.inner.lock().expect("lock memory storage");
                inner
                    .users
                    .get_mut(&stale.user.id)
                    .expect("stale user")
                    .created_at = Utc::now() - Duration::days(30);
            }
            let admin = auth
                .storage
                .create_user(AuthUserCreate {
                    email: Some("admin@example.com".into()),
                    name: None,
                    email_verified: true,
                    image: None,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: Some("admin".into()),
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json: "{}".into(),
                })
                .await
                .expect("create admin");
            let admin_session = auth
                .issue_session(admin, None, None, None, None)
                .await
                .expect("issue admin session");
            let cleanup = auth
                .cleanup_anonymous_users(CleanupAnonymousUsers {
                    session_token: admin_session.token,
                    older_than_seconds: 60 * 60 * 24,
                })
                .await
                .expect("cleanup anonymous");
            assert_eq!(cleanup.deleted, 1);
            assert!(
                auth.storage
                    .find_user_by_id(stale.user.id)
                    .await
                    .expect("find stale")
                    .is_none()
            );
            assert!(
                auth.storage
                    .find_user_by_id(recent.user.id)
                    .await
                    .expect("find recent")
                    .is_some()
            );
        }

        // r[verify auth.apikey.random]
        // r[verify auth.apikey.hash-storage]
        // r[verify auth.apikey.prefix]
        // r[verify auth.apikey.raw-return-once]
        // r[verify auth.apikey.list]
        // r[verify auth.apikey.get]
        // r[verify auth.apikey.update]
        // r[verify auth.apikey.delete]
        // r[verify auth.apikey.verify]
        // r[verify auth.apikey.disabled]
        // r[verify auth.apikey.expired]
        // r[verify auth.apikey.permissions]
        // r[verify auth.apikey.rate-limit]
        // r[verify auth.apikey.revoke]
        #[tokio::test]
        async fn api_key_create_and_authenticate_checks_state() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let created = auth
                .create_api_key(CreateApiKey {
                    session_token: bundle.token.clone(),
                    name: Some("ci".into()),
                    expires_at: Some(Utc::now() + Duration::hours(1)),
                    permissions_json: Some(r#"{"repo":"read"}"#.into()),
                    rate_limit_time_window: None,
                    rate_limit_max: None,
                    metadata_json: None,
                })
                .await
                .expect("create api key");
            assert!(created.key.starts_with("ak_"));
            assert_ne!(created.api_key.key_hash, created.key);
            assert_eq!(created.api_key.prefix.as_deref(), Some(&created.key[..12]));

            let listed = auth
                .list_api_keys(ListApiKeys {
                    session_token: bundle.token.clone(),
                })
                .await
                .expect("list api keys");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, created.api_key.id);

            let fetched = auth
                .get_api_key(GetApiKey {
                    session_token: bundle.token.clone(),
                    api_key_id: created.api_key.id,
                })
                .await
                .expect("get api key");
            assert_eq!(fetched.id, created.api_key.id);

            let updated = auth
                .update_api_key(UpdateApiKey {
                    session_token: bundle.token.clone(),
                    api_key_id: created.api_key.id,
                    name: Some("renamed".into()),
                    enabled: Some(true),
                    expires_at: None,
                    permissions_json: Some(r#"{"repo":["read","write"]}"#.into()),
                    rate_limit_time_window: None,
                    rate_limit_max: None,
                    metadata_json: Some(r#"{"env":"ci"}"#.into()),
                })
                .await
                .expect("update api key");
            assert_eq!(updated.name.as_deref(), Some("renamed"));
            assert_eq!(
                updated.permissions_json.as_deref(),
                Some(r#"{"repo":["read","write"]}"#)
            );
            assert_eq!(updated.key_hash, created.api_key.key_hash);

            let authenticated = auth
                .authenticate_api_key(AuthenticateApiKey {
                    key: created.key.clone(),
                })
                .await
                .expect("authenticate api key");
            assert_eq!(authenticated.user.id, created.user.id);
            auth.authorize_api_key(AuthorizeApiKey {
                key: created.key.clone(),
                permission: "repo:read".into(),
            })
            .await
            .expect("authorized api key");
            let denied_permission = auth
                .authorize_api_key(AuthorizeApiKey {
                    key: created.key.clone(),
                    permission: "repo:delete".into(),
                })
                .await;
            assert!(matches!(
                denied_permission,
                Err(AuthFlowError::PermissionDenied)
            ));
            auth.verify_api_key(VerifyApiKey {
                key: created.key.clone(),
                permission: Some("repo:write".into()),
            })
            .await
            .expect("verify api key with updated permission");

            auth.revoke_api_key(RevokeApiKey {
                session_token: bundle.token.clone(),
                api_key_id: created.api_key.id,
            })
            .await
            .expect("revoke api key");
            let disabled = auth
                .authenticate_api_key(AuthenticateApiKey {
                    key: created.key.clone(),
                })
                .await;
            assert!(matches!(disabled, Err(AuthFlowError::InvalidCredentials)));

            let delete_me = auth
                .create_api_key(CreateApiKey {
                    session_token: bundle.token.clone(),
                    name: Some("delete-me".into()),
                    expires_at: None,
                    permissions_json: None,
                    rate_limit_time_window: None,
                    rate_limit_max: None,
                    metadata_json: None,
                })
                .await
                .expect("create api key to delete");
            auth.delete_api_key(DeleteApiKey {
                session_token: bundle.token.clone(),
                api_key_id: delete_me.api_key.id,
            })
            .await
            .expect("delete api key");
            let deleted = auth
                .get_api_key(GetApiKey {
                    session_token: bundle.token.clone(),
                    api_key_id: delete_me.api_key.id,
                })
                .await;
            assert!(matches!(deleted, Err(AuthFlowError::InvalidCredentials)));

            {
                let mut inner = auth.storage.inner.lock().expect("lock memory storage");
                let by_hash = inner
                    .api_keys_by_hash
                    .get_mut(&created.api_key.key_hash)
                    .expect("stored api key");
                by_hash.enabled = true;
                by_hash.expires_at = Some(Utc::now() - Duration::seconds(1));
                let by_id = inner
                    .api_keys_by_id
                    .get_mut(&created.api_key.id)
                    .expect("stored api key by id");
                by_id.enabled = true;
                by_id.expires_at = Some(Utc::now() - Duration::seconds(1));
            }
            let expired = auth
                .authenticate_api_key(AuthenticateApiKey { key: created.key })
                .await;
            assert!(matches!(expired, Err(AuthFlowError::InvalidCredentials)));

            let limited = auth
                .create_api_key(CreateApiKey {
                    session_token: bundle.token,
                    name: Some("limited".into()),
                    expires_at: None,
                    permissions_json: None,
                    rate_limit_time_window: Some(60),
                    rate_limit_max: Some(1),
                    metadata_json: None,
                })
                .await
                .expect("create limited api key");
            auth.authenticate_api_key(AuthenticateApiKey {
                key: limited.key.clone(),
            })
            .await
            .expect("first limited request");
            let limited_again = auth
                .authenticate_api_key(AuthenticateApiKey { key: limited.key })
                .await;
            assert!(matches!(
                limited_again,
                Err(AuthFlowError::PermissionDenied)
            ));
        }

        // r[verify auth.bearer.parse]
        // r[verify auth.bearer.session]
        // r[verify auth.bearer.api-key]
        // r[verify auth.bearer.errors]
        #[tokio::test]
        async fn bearer_authenticates_sessions_and_api_keys_with_stable_errors() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "bearer@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let session_bearer = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: Some(format!("Bearer {}", bundle.token)),
                })
                .await
                .expect("authenticate session bearer");
            assert_eq!(session_bearer.user.id, bundle.user.id);
            assert_eq!(session_bearer.strategy, BearerTokenStrategy::Session);
            assert!(session_bearer.session.is_some());
            assert!(session_bearer.api_key.is_none());

            let api_key = auth
                .create_api_key(CreateApiKey {
                    session_token: bundle.token.clone(),
                    name: Some("bearer-key".into()),
                    expires_at: None,
                    permissions_json: None,
                    rate_limit_time_window: None,
                    rate_limit_max: None,
                    metadata_json: None,
                })
                .await
                .expect("create api key");
            let api_key_bearer = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: Some(format!("Bearer {}", api_key.key.clone())),
                })
                .await
                .expect("authenticate api key bearer");
            assert_eq!(api_key_bearer.user.id, bundle.user.id);
            assert_eq!(api_key_bearer.strategy, BearerTokenStrategy::ApiKey);
            assert!(api_key_bearer.session.is_none());
            assert_eq!(
                api_key_bearer.api_key.as_ref().map(|key| key.id),
                Some(api_key.api_key.id)
            );

            let missing = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: None,
                })
                .await;
            assert_eq!(
                crate::transport::map_auth_error(&missing.expect_err("missing bearer")).code,
                "invalid_credentials"
            );

            let malformed = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: Some("Basic abc".into()),
                })
                .await;
            assert_eq!(
                crate::transport::map_auth_error(&malformed.expect_err("malformed bearer")).code,
                "invalid_input"
            );

            auth.revoke_api_key(RevokeApiKey {
                session_token: bundle.token.clone(),
                api_key_id: api_key.api_key.id,
            })
            .await
            .expect("revoke api key");
            let revoked_api_key = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: Some(format!("Bearer {}", api_key.key)),
                })
                .await;
            assert_eq!(
                crate::transport::map_auth_error(
                    &revoked_api_key.expect_err("revoked api key bearer")
                )
                .code,
                "invalid_credentials"
            );

            auth.sign_out(SignOut {
                token: bundle.token.clone(),
            })
            .await
            .expect("revoke session");
            let revoked_session = auth
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: Some(format!("Bearer {}", bundle.token)),
                })
                .await;
            assert_eq!(
                crate::transport::map_auth_error(
                    &revoked_session.expect_err("revoked session bearer")
                )
                .code,
                "session_expired"
            );
        }

        // r[verify auth.org.slug-unique]
        // r[verify auth.org.create-owner]
        // r[verify auth.org.member-unique]
        // r[verify auth.org.active-session]
        // r[verify auth.org.role-authoritative]
        // r[verify auth.org.rbac-deny-default]
        #[tokio::test]
        async fn organization_create_active_and_rbac_round_trip() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: bundle.token.clone(),
                    name: "Acme".into(),
                    slug: "ACME".into(),
                    logo: None,
                    metadata_json: Some(r#"{"tier":"pro"}"#.into()),
                })
                .await
                .expect("create org");

            assert_eq!(organization.organization.slug, "acme");
            assert_eq!(organization.membership.role, "owner");
            assert_eq!(organization.membership.user_id, bundle.user.id);

            let duplicate = auth
                .create_organization(CreateOrganization {
                    session_token: bundle.token.clone(),
                    name: "Acme 2".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            auth.set_active_organization(SetActiveOrganization {
                session_token: bundle.token.clone(),
                organization_id: organization.organization.id,
            })
            .await
            .expect("set active org");
            let current = auth
                .current_session(CurrentSession {
                    token: bundle.token.clone(),
                })
                .await
                .expect("current session");
            assert_eq!(
                current.session.active_organization_id,
                Some(organization.organization.id)
            );

            auth.require_organization_role(RequireOrganizationRole {
                session_token: bundle.token.clone(),
                organization_id: organization.organization.id,
                allowed_roles: vec!["owner".into()],
            })
            .await
            .expect("owner authorized");

            let denied = auth
                .require_organization_role(RequireOrganizationRole {
                    session_token: bundle.token,
                    organization_id: organization.organization.id,
                    allowed_roles: vec!["member".into()],
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));
        }

        // r[verify auth.org.invite-token]
        // r[verify auth.org.invite-status]
        #[tokio::test]
        async fn organization_invitation_acceptance_is_single_use() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let member = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "member@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create member");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token,
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");

            let invitation = auth
                .create_invitation(CreateInvitation {
                    session_token: auth
                        .sign_in_email_password(SignInEmailPassword {
                            email: "owner@example.com".into(),
                            password: "correct horse battery staple".into(),
                            ip_address: None,
                            user_agent: None,
                        })
                        .await
                        .expect("owner sign in")
                        .token,
                    organization_id: organization.organization.id,
                    email: "Member@Example.COM".into(),
                    role: "member".into(),
                    expires_at: Utc::now() + Duration::hours(1),
                })
                .await
                .expect("create invitation");
            assert_eq!(invitation.invitation.email, "Member@example.com");
            assert_ne!(invitation.invitation.status, invitation.token);

            auth.accept_invitation(AcceptInvitation {
                session_token: member.token,
                invitation_id: invitation.invitation.id,
                token: invitation.token.clone(),
            })
            .await
            .expect("accept invitation");

            let accepted = auth
                .storage
                .find_invitation_by_id(invitation.invitation.id)
                .await
                .expect("load invitation")
                .expect("invitation exists");
            assert_eq!(
                accepted.status,
                auth_proto::InvitationStatus::Accepted.as_str()
            );
            let member_record = auth
                .storage
                .find_member(organization.organization.id, member.user.id)
                .await
                .expect("load membership")
                .expect("membership exists");
            assert_eq!(member_record.role, "member");

            let reused = auth
                .accept_invitation(AcceptInvitation {
                    session_token: auth
                        .sign_in_email_password(SignInEmailPassword {
                            email: "member@example.com".into(),
                            password: "correct horse battery staple".into(),
                            ip_address: None,
                            user_agent: None,
                        })
                        .await
                        .expect("member sign in")
                        .token,
                    invitation_id: invitation.invitation.id,
                    token: invitation.token,
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidInput(_))));
        }

        // r[verify auth.org.remove-last-owner]
        #[tokio::test]
        async fn organization_rejects_demoting_last_owner() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let member = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "member@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create member");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: member.user.id,
                    role: "member".into(),
                })
                .await
                .expect("add member");

            let demote_last_owner = auth
                .set_member_role(SetMemberRole {
                    session_token: owner.token.clone(),
                    organization_id: organization.organization.id,
                    user_id: owner.user.id,
                    role: "admin".into(),
                })
                .await;
            assert!(matches!(
                demote_last_owner,
                Err(AuthFlowError::InvalidInput(_))
            ));

            auth.set_member_role(SetMemberRole {
                session_token: owner.token.clone(),
                organization_id: organization.organization.id,
                user_id: member.user.id,
                role: "owner".into(),
            })
            .await
            .expect("promote second owner");
            let demoted = auth
                .set_member_role(SetMemberRole {
                    session_token: owner.token,
                    organization_id: organization.organization.id,
                    user_id: owner.user.id,
                    role: "admin".into(),
                })
                .await
                .expect("demote when another owner exists");
            assert_eq!(demoted.role, "admin");
        }

        // r[verify auth.org.permission-resources]
        // r[verify auth.org.default-permission-roles]
        // r[verify auth.org.rbac-deny-default]
        #[tokio::test]
        async fn organization_permissions_match_better_auth_defaults() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
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
            let member = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "member@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create member");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: admin.user.id,
                    role: "admin".into(),
                })
                .await
                .expect("add admin");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: member.user.id,
                    role: "member".into(),
                })
                .await
                .expect("add member");

            auth.authorize_organization_action(AuthorizeOrganizationAction {
                session_token: owner.token,
                organization_id: organization.organization.id,
                resource: "organization".into(),
                action: "delete".into(),
            })
            .await
            .expect("owner can delete organization");
            auth.authorize_organization_action(AuthorizeOrganizationAction {
                session_token: admin.token.clone(),
                organization_id: organization.organization.id,
                resource: "team".into(),
                action: "create".into(),
            })
            .await
            .expect("admin can create teams");
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
                Err(AuthFlowError::PermissionDenied)
            ));
            auth.authorize_organization_action(AuthorizeOrganizationAction {
                session_token: member.token.clone(),
                organization_id: organization.organization.id,
                resource: "ac".into(),
                action: "read".into(),
            })
            .await
            .expect("member can read ac");
            let member_create_team = auth
                .authorize_organization_action(AuthorizeOrganizationAction {
                    session_token: member.token,
                    organization_id: organization.organization.id,
                    resource: "team".into(),
                    action: "create".into(),
                })
                .await;
            assert!(matches!(
                member_create_team,
                Err(AuthFlowError::PermissionDenied)
            ));
        }

        // r[verify auth.org.permission-resources]
        // r[verify auth.org.default-permission-roles]
        // r[verify auth.org.rbac-deny-default]
        #[tokio::test]
        async fn organization_permissions_match_better_auth_role_matrix_fixture() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
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
            let member = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "member@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create member");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: admin.user.id,
                    role: "admin".into(),
                })
                .await
                .expect("add admin");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: member.user.id,
                    role: "member".into(),
                })
                .await
                .expect("add member");

            let cases = [
                ("owner", &owner.token, "organization", "update", true),
                ("owner", &owner.token, "organization", "delete", true),
                ("owner", &owner.token, "member", "create", true),
                ("owner", &owner.token, "invitation", "cancel", true),
                ("owner", &owner.token, "team", "delete", true),
                ("owner", &owner.token, "ac", "delete", true),
                ("admin", &admin.token, "organization", "update", true),
                ("admin", &admin.token, "organization", "delete", false),
                ("admin", &admin.token, "member", "delete", true),
                ("admin", &admin.token, "invitation", "create", true),
                ("admin", &admin.token, "team", "update", true),
                ("admin", &admin.token, "ac", "update", true),
                ("member", &member.token, "organization", "update", false),
                ("member", &member.token, "member", "create", false),
                ("member", &member.token, "invitation", "create", false),
                ("member", &member.token, "team", "create", false),
                ("member", &member.token, "ac", "read", true),
                ("member", &member.token, "ac", "create", false),
            ];

            for (role, token, resource, action, allowed) in cases {
                let result = auth
                    .authorize_organization_action(AuthorizeOrganizationAction {
                        session_token: token.clone(),
                        organization_id: organization.organization.id,
                        resource: resource.into(),
                        action: action.into(),
                    })
                    .await;
                assert_eq!(
                    result.is_ok(),
                    allowed,
                    "{role} {resource}:{action} expected allowed={allowed}"
                );
            }
        }

        // r[verify auth.org.composite-roles]
        #[tokio::test]
        async fn organization_permissions_allow_composite_roles() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token,
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: user.user.id,
                    role: "member,admin".into(),
                })
                .await
                .expect("add composite member");

            auth.authorize_organization_action(AuthorizeOrganizationAction {
                session_token: user.token,
                organization_id: organization.organization.id,
                resource: "organization".into(),
                action: "update".into(),
            })
            .await
            .expect("admin half of composite role grants update");
        }

        // r[verify auth.org.dynamic-access-control]
        #[tokio::test]
        async fn organization_dynamic_roles_extend_default_permissions() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let billing = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "billing@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create billing user");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: billing.user.id,
                    role: "billing".into(),
                })
                .await
                .expect("add billing member");

            let denied = auth
                .create_team(CreateTeam {
                    session_token: billing.token.clone(),
                    organization_id: organization.organization.id,
                    name: "Finance".into(),
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            let role = auth
                .create_organization_role(CreateOrganizationRole {
                    session_token: owner.token,
                    organization_id: organization.organization.id,
                    role: "billing".into(),
                    permissions_json: r#"{"team":["create"]}"#.into(),
                })
                .await
                .expect("create dynamic role");
            assert_eq!(role.role, "billing");

            let team = auth
                .create_team(CreateTeam {
                    session_token: billing.token,
                    organization_id: organization.organization.id,
                    name: "Finance".into(),
                })
                .await
                .expect("dynamic role can create team");
            assert_eq!(team.name, "Finance");
        }

        // r[verify auth.org.teams]
        #[tokio::test]
        async fn organization_team_crud_and_membership_round_trip() {
            let auth = auth();
            let owner = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "owner@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create owner");
            let member = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "member@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create member");
            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: owner.token.clone(),
                    name: "Acme".into(),
                    slug: "acme".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create organization");
            auth.storage
                .create_member(AuthMemberCreate {
                    organization_id: organization.organization.id,
                    user_id: member.user.id,
                    role: "member".into(),
                })
                .await
                .expect("add org member");

            let team = auth
                .create_team(CreateTeam {
                    session_token: owner.token.clone(),
                    organization_id: organization.organization.id,
                    name: "Platform".into(),
                })
                .await
                .expect("create team");
            let teams = auth
                .list_teams(ListTeams {
                    session_token: member.token.clone(),
                    organization_id: organization.organization.id,
                })
                .await
                .expect("member can list teams");
            assert_eq!(teams.len(), 1);

            let team_member = auth
                .add_team_member(AddTeamMember {
                    session_token: owner.token.clone(),
                    organization_id: organization.organization.id,
                    team_id: team.id,
                    user_id: member.user.id,
                })
                .await
                .expect("add team member");
            assert_eq!(team_member.user_id, member.user.id);
            let duplicate = auth
                .add_team_member(AddTeamMember {
                    session_token: owner.token.clone(),
                    organization_id: organization.organization.id,
                    team_id: team.id,
                    user_id: member.user.id,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let listed = auth
                .list_team_members(ListTeamMembers {
                    session_token: member.token,
                    organization_id: organization.organization.id,
                    team_id: team.id,
                })
                .await
                .expect("list team members");
            assert_eq!(listed.len(), 1);

            let updated = auth
                .update_team(UpdateTeam {
                    session_token: owner.token.clone(),
                    organization_id: organization.organization.id,
                    team_id: team.id,
                    name: "Core Platform".into(),
                })
                .await
                .expect("update team");
            assert_eq!(updated.name, "Core Platform");

            auth.remove_team_member(RemoveTeamMember {
                session_token: owner.token.clone(),
                organization_id: organization.organization.id,
                team_id: team.id,
                user_id: member.user.id,
            })
            .await
            .expect("remove team member");
            auth.delete_team(DeleteTeam {
                session_token: owner.token,
                organization_id: organization.organization.id,
                team_id: team.id,
            })
            .await
            .expect("delete team");
            let teams = auth
                .storage
                .list_teams_by_organization(organization.organization.id)
                .await
                .expect("list teams");
            assert!(teams.is_empty());
        }

        // r[verify auth.admin.requires-role]
        // r[verify auth.admin.list-users]
        // r[verify auth.admin.create-user]
        // r[verify auth.admin.set-user-password]
        // r[verify auth.admin.no-self-lockout]
        #[tokio::test]
        async fn admin_list_users_and_role_changes_require_admin() {
            let auth = auth();
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
            auth.storage
                .update_user_role(admin.user.id, Some("admin".into()))
                .await
                .expect("promote admin");
            let user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let denied = auth
                .list_users(ListUsers {
                    session_token: user.token.clone(),
                    offset: 0,
                    limit: 10,
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            let listed = auth
                .list_users(ListUsers {
                    session_token: admin.token.clone(),
                    offset: 0,
                    limit: 10,
                })
                .await
                .expect("list users");
            assert_eq!(listed.total, 2);
            assert_eq!(listed.users.len(), 2);

            let admin_created = auth
                .admin_create_user(AdminCreateUser {
                    session_token: admin.token.clone(),
                    email: "created@example.com".into(),
                    password: Some("temporary password".into()),
                    name: Some("Created User".into()),
                    role: Some("user".into()),
                    metadata_json: Some(r#"{"source":"admin"}"#.into()),
                })
                .await
                .expect("admin create user");
            assert_eq!(admin_created.email.as_deref(), Some("created@example.com"));
            assert!(admin_created.email_verified);
            assert_eq!(admin_created.role.as_deref(), Some("user"));
            auth.sign_in_email_password(SignInEmailPassword {
                email: "created@example.com".into(),
                password: "temporary password".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("admin created credential signs in");

            let duplicate = auth
                .admin_create_user(AdminCreateUser {
                    session_token: admin.token.clone(),
                    email: "created@EXAMPLE.com".into(),
                    password: None,
                    name: Some("Duplicate".into()),
                    role: None,
                    metadata_json: None,
                })
                .await;
            assert!(matches!(duplicate, Err(AuthFlowError::InvalidInput(_))));

            let passwordless = auth
                .admin_create_user(AdminCreateUser {
                    session_token: admin.token.clone(),
                    email: "passwordless@example.com".into(),
                    password: None,
                    name: Some("Passwordless".into()),
                    role: None,
                    metadata_json: None,
                })
                .await
                .expect("admin create passwordless user");
            assert!(
                auth.storage
                    .find_password_account_by_user_id(passwordless.id)
                    .await
                    .expect("find password account")
                    .is_none()
            );
            auth.admin_set_user_password(AdminSetUserPassword {
                session_token: admin.token.clone(),
                user_id: passwordless.id,
                new_password: "new password value".into(),
            })
            .await
            .expect("admin set user password");
            auth.sign_in_email_password(SignInEmailPassword {
                email: "passwordless@example.com".into(),
                password: "new password value".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("admin set password signs in");

            let promoted = auth
                .set_user_role(SetUserRole {
                    session_token: admin.token.clone(),
                    user_id: user.user.id,
                    role: Some("admin".into()),
                })
                .await
                .expect("set user role");
            assert_eq!(promoted.role.as_deref(), Some("admin"));

            let self_lockout = auth
                .set_user_role(SetUserRole {
                    session_token: admin.token,
                    user_id: admin.user.id,
                    role: Some("user".into()),
                })
                .await;
            assert!(matches!(self_lockout, Err(AuthFlowError::PermissionDenied)));
        }

        // r[verify auth.admin.ban]
        // r[verify auth.admin.ban-expiry]
        // r[verify auth.admin.revoke-sessions]
        // r[verify auth.admin.list-user-sessions]
        // r[verify auth.admin.revoke-session]
        // r[verify auth.admin.impersonate]
        // r[verify auth.admin.stop-impersonating]
        // r[verify auth.admin.remove-user]
        // r[verify auth.admin.has-permission]
        // r[verify auth.admin.audit]
        // r[verify auth.email.signin.banned]
        // r[verify auth.sessions.current.expired]
        // r[verify auth.sessions.impersonation]
        #[tokio::test]
        async fn admin_ban_revoke_and_impersonate_flows() {
            let auth = auth();
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
            auth.storage
                .update_user_role(admin.user.id, Some("admin".into()))
                .await
                .expect("promote admin");
            let user = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");

            let impersonated = auth
                .impersonate_user(ImpersonateUser {
                    session_token: admin.token.clone(),
                    user_id: user.user.id,
                    ip_address: Some("127.0.0.1".into()),
                    user_agent: Some("admin-test".into()),
                })
                .await
                .expect("impersonate user");
            assert_eq!(impersonated.user.id, user.user.id);
            assert_eq!(impersonated.session.impersonated_by, Some(admin.user.id));

            let user_sessions = auth
                .list_user_sessions(ListUserSessions {
                    session_token: admin.token.clone(),
                    user_id: user.user.id,
                })
                .await
                .expect("list user sessions");
            assert!(
                user_sessions
                    .iter()
                    .any(|session| session.id == impersonated.session.id)
            );

            auth.stop_impersonating(StopImpersonating {
                session_token: impersonated.token.clone(),
            })
            .await
            .expect("stop impersonating");
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: impersonated.token
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));

            let second_user_session = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("second user session");

            auth.revoke_user_session(RevokeUserSession {
                session_token: admin.token.clone(),
                user_id: user.user.id,
                session_id: second_user_session.session.id,
            })
            .await
            .expect("revoke one user session");
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: second_user_session.token
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));

            let third_user_session = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("third user session");

            auth.revoke_user_sessions(RevokeUserSessions {
                session_token: admin.token.clone(),
                user_id: user.user.id,
            })
            .await
            .expect("revoke user sessions");
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: third_user_session.token
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));

            let banned = auth
                .ban_user(BanUser {
                    session_token: admin.token.clone(),
                    user_id: user.user.id,
                    reason: Some("policy".into()),
                    expires_at: Some(Utc::now() + Duration::hours(1)),
                })
                .await
                .expect("ban user");
            assert!(banned.banned);
            assert_eq!(banned.ban_reason.as_deref(), Some("policy"));
            let denied = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await;
            assert!(matches!(denied, Err(AuthFlowError::PermissionDenied)));

            auth.storage
                .update_user_ban(
                    user.user.id,
                    true,
                    Some("expired".into()),
                    Some(Utc::now() - Duration::seconds(1)),
                )
                .await
                .expect("expire ban");
            auth.sign_in_email_password(SignInEmailPassword {
                email: "user@example.com".into(),
                password: "correct horse battery staple".into(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("expired ban allows sign in");

            let unbanned = auth
                .unban_user(UnbanUser {
                    session_token: admin.token.clone(),
                    user_id: user.user.id,
                })
                .await
                .expect("unban user");
            assert!(!unbanned.banned);
            assert!(unbanned.ban_reason.is_none());
            assert!(unbanned.ban_expires.is_none());

            let organization = auth
                .create_organization(CreateOrganization {
                    session_token: admin.token.clone(),
                    name: "Admin Org".into(),
                    slug: "admin-org".into(),
                    logo: None,
                    metadata_json: None,
                })
                .await
                .expect("create admin org");
            let permission = auth
                .admin_has_permission(AdminHasPermission {
                    session_token: admin.token.clone(),
                    organization_id: organization.organization.id,
                    resource: "organization".into(),
                    action: "update".into(),
                })
                .await
                .expect("check admin permission");
            assert!(permission.allowed);

            auth.remove_user(RemoveUser {
                session_token: admin.token.clone(),
                user_id: user.user.id,
            })
            .await
            .expect("remove user");
            assert!(
                auth.storage
                    .find_user_by_id(user.user.id)
                    .await
                    .expect("find removed user")
                    .is_none()
            );

            let audit_events = auth
                .storage
                .inner
                .lock()
                .expect("lock memory storage")
                .audit_events
                .clone();
            assert!(audit_events.iter().any(|event| {
                event.actor_id == admin.user.id
                    && event.target_id == Some(user.user.id)
                    && event.action == "admin.impersonate_user"
            }));
            assert!(audit_events.iter().any(|event| {
                event.actor_id == admin.user.id
                    && event.target_id == Some(user.user.id)
                    && event.action == "admin.ban_user"
            }));
            assert!(
                audit_events
                    .iter()
                    .all(|event| event.created_at <= Utc::now())
            );
        }

        // r[verify auth.twofactor.enable-requires-session]
        // r[verify auth.twofactor.secret-encryption]
        // r[verify auth.twofactor.confirm-before-enabled]
        // r[verify auth.twofactor.backup-codes-hash]
        // r[verify auth.twofactor.disable-requires-proof]
        #[tokio::test]
        async fn two_factor_setup_confirm_verify_and_disable_round_trip() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let secret = "OBWGC2LOFVZXI4TJNZTS243FMNZGK5BNGEZDG";
            let totp = test_totp(secret);

            auth.start_two_factor_setup(StartTwoFactorSetup {
                session_token: bundle.token.clone(),
                secret_ciphertext: secret.into(),
                backup_codes: vec!["recovery-code".into()],
            })
            .await
            .expect("start two factor setup");

            let pending_user = auth
                .storage
                .find_user_by_id(bundle.user.id)
                .await
                .expect("load user")
                .expect("user exists");
            assert!(!pending_user.two_factor_enabled);
            let stored = auth
                .storage
                .find_two_factor_by_user_id(bundle.user.id)
                .await
                .expect("load two factor")
                .expect("two factor exists");
            assert_ne!(stored.secret_ciphertext, secret);
            assert!(!stored.secret_ciphertext.contains(secret));
            assert!(
                !stored
                    .backup_codes_hash
                    .as_deref()
                    .expect("backup hashes")
                    .contains("recovery-code")
            );

            let code = totp.generate_current().expect("generate totp");
            auth.confirm_two_factor(ConfirmTwoFactor {
                session_token: bundle.token.clone(),
                code: code.clone(),
            })
            .await
            .expect("confirm two factor");
            let enabled_user = auth
                .storage
                .find_user_by_id(bundle.user.id)
                .await
                .expect("load enabled user")
                .expect("user exists");
            assert!(enabled_user.two_factor_enabled);

            auth.verify_two_factor(VerifyTwoFactor {
                session_token: bundle.token.clone(),
                code: code.clone(),
            })
            .await
            .expect("verify two factor");

            auth.disable_two_factor(DisableTwoFactor {
                session_token: bundle.token,
                code,
            })
            .await
            .expect("disable two factor");
            let disabled_user = auth
                .storage
                .find_user_by_id(bundle.user.id)
                .await
                .expect("load disabled user")
                .expect("user exists");
            assert!(!disabled_user.two_factor_enabled);
            assert!(
                auth.storage
                    .find_two_factor_by_user_id(bundle.user.id)
                    .await
                    .expect("load deleted two factor")
                    .is_none()
            );
        }

        // r[verify auth.twofactor.signin-required]
        // r[verify auth.twofactor.backup-codes-single-use]
        // r[verify auth.twofactor.rate-limit]
        #[tokio::test]
        async fn two_factor_signin_requires_second_factor_and_consumes_backup_code() {
            let auth = auth();
            let bundle = auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    name: None,
                    username: None,
                    image: None,
                    metadata_json: None,
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("create user");
            let secret = "OBWGC2LOFVZXI4TJNZTS243FMNZGK5BNGEZDG";
            let code = test_totp(secret).generate_current().expect("generate totp");

            auth.start_two_factor_setup(StartTwoFactorSetup {
                session_token: bundle.token.clone(),
                secret_ciphertext: secret.into(),
                backup_codes: vec!["first-backup".into(), "second-backup".into()],
            })
            .await
            .expect("start two factor setup");
            auth.confirm_two_factor(ConfirmTwoFactor {
                session_token: bundle.token,
                code,
            })
            .await
            .expect("confirm two factor");

            let pending = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in with 2fa enabled");
            assert!(!pending.session.active);
            assert!(matches!(
                auth.current_session(CurrentSession {
                    token: pending.token.clone()
                })
                .await,
                Err(AuthFlowError::SessionExpired)
            ));

            for _ in 0..2 {
                let wrong = auth
                    .verify_two_factor(VerifyTwoFactor {
                        session_token: pending.token.clone(),
                        code: "wrong-code".into(),
                    })
                    .await;
                assert!(matches!(wrong, Err(AuthFlowError::InvalidCredentials)));
            }
            auth.verify_two_factor(VerifyTwoFactor {
                session_token: pending.token.clone(),
                code: "first-backup".into(),
            })
            .await
            .expect("verify backup code");
            auth.current_session(CurrentSession {
                token: pending.token.clone(),
            })
            .await
            .expect("backup code activates session");

            let remaining_hashes = auth
                .storage
                .find_two_factor_by_user_id(pending.user.id)
                .await
                .expect("load two factor")
                .expect("two factor exists")
                .backup_codes_hash
                .expect("remaining backup code");
            assert_eq!(remaining_hashes.lines().count(), 1);

            let second_pending = auth
                .sign_in_email_password(SignInEmailPassword {
                    email: "user@example.com".into(),
                    password: "correct horse battery staple".into(),
                    ip_address: None,
                    user_agent: None,
                })
                .await
                .expect("sign in again with 2fa enabled");
            let reused = auth
                .verify_two_factor(VerifyTwoFactor {
                    session_token: second_pending.token.clone(),
                    code: "first-backup".into(),
                })
                .await;
            assert!(matches!(reused, Err(AuthFlowError::InvalidCredentials)));

            for _ in 0..4 {
                let wrong = auth
                    .verify_two_factor(VerifyTwoFactor {
                        session_token: second_pending.token.clone(),
                        code: "wrong-code".into(),
                    })
                    .await;
                assert!(matches!(wrong, Err(AuthFlowError::InvalidCredentials)));
            }
            let blocked = auth
                .verify_two_factor(VerifyTwoFactor {
                    session_token: second_pending.token,
                    code: "wrong-code".into(),
                })
                .await;
            assert!(matches!(blocked, Err(AuthFlowError::PermissionDenied)));
        }

        fn test_totp(secret: &str) -> TOTP {
            TOTP::new(
                Algorithm::SHA1,
                6,
                1,
                30,
                Secret::Encoded(secret.to_owned())
                    .to_bytes()
                    .expect("decode secret"),
                None,
                "architect-auth".into(),
            )
            .expect("build totp")
        }
    }
}
pub mod email_otp {
    use auth_proto::{AuthFlowError, AuthUserCreate, AuthVerificationCreate};
    use chrono::{Duration, Utc};

    use super::email_password::normalize_email;
    use crate::{
        ArchitectAuth, AuthStorage, EmailOtpVerification, SendEmailOtp, VerificationToken,
        VerifyEmailOtp,
        crypto::{generate_token, hash_token},
        flows::last_login_method::record_last_login_method,
    };

    const EMAIL_OTP_TTL_SECONDS: i64 = 300;
    const EMAIL_OTP_RESEND_SECONDS: i64 = 60;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.emailotp.send]
        // r[impl auth.emailotp.expiry]
        // r[impl auth.emailotp.resend-limit]
        // r[impl auth.emailotp.test-sink]
        pub async fn send_email_otp(
            &self,
            input: SendEmailOtp,
        ) -> Result<VerificationToken, AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            let identifier = email_otp_identifier(&canonical_email);
            if let Some(existing) = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                && existing.created_at + Duration::seconds(EMAIL_OTP_RESEND_SECONDS) > Utc::now()
            {
                return Err(AuthFlowError::PermissionDenied);
            }
            let otp = generate_otp()?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &otp),
                    expires_at: Utc::now() + Duration::seconds(EMAIL_OTP_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken {
                identifier,
                token: otp,
            })
        }

        // r[impl auth.emailotp.verify]
        // r[impl auth.emailotp.single-use]
        // r[impl auth.emailotp.session]
        pub async fn verify_email_otp(
            &self,
            input: VerifyEmailOtp,
        ) -> Result<EmailOtpVerification, AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            let identifier = email_otp_identifier(&canonical_email);
            let value_hash = hash_token(&self.config.secret, &input.otp);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }

            let user = if let Some(user) = self.storage.find_user_by_email(&canonical_email).await?
            {
                user
            } else {
                self.storage
                    .create_user(AuthUserCreate {
                        email: Some(canonical_email),
                        name: None,
                        email_verified: true,
                        image: None,
                        username: None,
                        display_username: None,
                        two_factor_enabled: false,
                        role: None,
                        banned: false,
                        ban_reason: None,
                        ban_expires: None,
                        metadata_json: "{}".into(),
                    })
                    .await?
            };
            self.storage.delete_verification(verification.id).await?;

            if input.create_session {
                let bundle = self
                    .issue_session(user, input.ip_address, input.user_agent, None, None)
                    .await?;
                let bundle = record_last_login_method(self, bundle, "email-otp").await?;
                Ok(EmailOtpVerification {
                    user: bundle.user,
                    session: Some(bundle.session),
                    token: Some(bundle.token),
                })
            } else {
                Ok(EmailOtpVerification {
                    user,
                    session: None,
                    token: None,
                })
            }
        }
    }

    fn generate_otp() -> Result<String, AuthFlowError> {
        let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
        Ok(token
            .chars()
            .take(6)
            .collect::<String>()
            .to_ascii_uppercase())
    }

    fn email_otp_identifier(canonical_email: &str) -> String {
        format!("email-otp:{canonical_email}")
    }
}
pub mod phone_number {
    use auth_proto::{AuthFlowError, AuthUser, AuthUserCreate, AuthVerificationCreate};
    use chrono::{Duration, Utc};
    use serde_json::{Value, json};

    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, PhoneNumberVerification, SendPhoneNumberOtp,
        UpdatePhoneNumber, VerificationToken, VerifyPhoneNumberOtp,
        config::SmsProvider,
        crypto::{generate_token, hash_token},
        flows::last_login_method::record_last_login_method,
    };

    const PHONE_OTP_TTL_SECONDS: i64 = 300;
    const PHONE_METADATA_KEY: &str = "phone_number";
    const PHONE_VERIFIED_METADATA_KEY: &str = "phone_number_verified";

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.phone.send]
        // r[impl auth.phone.provider]
        pub async fn send_phone_number_otp(
            &self,
            input: SendPhoneNumberOtp,
        ) -> Result<VerificationToken, AuthFlowError> {
            let phone_number = normalize_phone_number(&input.phone_number)?;
            self.deliver_phone_otp(&phone_number).await?;
            let otp = generate_phone_otp()?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: phone_identifier(&phone_number),
                    value_hash: hash_token(&self.config.secret, &otp),
                    expires_at: Utc::now() + Duration::seconds(PHONE_OTP_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken {
                identifier: phone_identifier(&phone_number),
                token: otp,
            })
        }

        // r[impl auth.phone.verify]
        // r[impl auth.phone.expiry]
        // r[impl auth.phone.signin]
        pub async fn verify_phone_number_otp(
            &self,
            input: VerifyPhoneNumberOtp,
        ) -> Result<PhoneNumberVerification, AuthFlowError> {
            let phone_number = normalize_phone_number(&input.phone_number)?;
            let identifier = phone_identifier(&phone_number);
            let value_hash = hash_token(&self.config.secret, &input.otp);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.storage.delete_verification(verification.id).await?;

            let user = if let Some(user) = self.find_user_by_phone_number(&phone_number).await? {
                set_user_phone_number(self, user, &phone_number, true).await?
            } else {
                self.storage
                    .create_user(AuthUserCreate {
                        email: None,
                        name: None,
                        email_verified: false,
                        image: None,
                        username: None,
                        display_username: None,
                        two_factor_enabled: false,
                        role: None,
                        banned: false,
                        ban_reason: None,
                        ban_expires: None,
                        metadata_json: phone_metadata_json(&phone_number, true)?,
                    })
                    .await?
            };

            if input.create_session {
                let bundle = self
                    .issue_session(user, input.ip_address, input.user_agent, None, None)
                    .await?;
                let bundle = record_last_login_method(self, bundle, "phone-number").await?;
                Ok(PhoneNumberVerification {
                    user: bundle.user,
                    session: Some(bundle.session),
                    token: Some(bundle.token),
                    phone_number,
                })
            } else {
                Ok(PhoneNumberVerification {
                    user,
                    session: None,
                    token: None,
                    phone_number,
                })
            }
        }

        // r[impl auth.phone.update]
        // r[impl auth.phone.duplicate]
        pub async fn update_phone_number(
            &self,
            input: UpdatePhoneNumber,
        ) -> Result<AuthUser, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let phone_number = normalize_phone_number(&input.phone_number)?;
            if let Some(existing) = self.find_user_by_phone_number(&phone_number).await?
                && existing.id != bundle.user.id
            {
                return Err(AuthFlowError::InvalidInput(
                    "phone number already exists".into(),
                ));
            }
            set_user_phone_number(self, bundle.user, &phone_number, false).await
        }

        async fn deliver_phone_otp(&self, _phone_number: &str) -> Result<(), AuthFlowError> {
            match self.config.sms.provider {
                SmsProvider::Disabled | SmsProvider::Test => Ok(()),
                SmsProvider::FailClosed => Err(AuthFlowError::PermissionDenied),
            }
        }

        async fn find_user_by_phone_number(
            &self,
            phone_number: &str,
        ) -> Result<Option<AuthUser>, AuthFlowError> {
            let (users, _) = self.storage.list_users(0, 10_000).await?;
            for user in users {
                if user_phone_number(&user)?.as_deref() == Some(phone_number) {
                    return Ok(Some(user));
                }
            }
            Ok(None)
        }
    }

    pub fn normalize_phone_number(phone_number: &str) -> Result<String, AuthFlowError> {
        let trimmed = phone_number.trim();
        if !trimmed.starts_with('+') {
            return Err(AuthFlowError::InvalidInput(
                "phone number must be E.164".into(),
            ));
        }
        let digits = &trimmed[1..];
        if digits.len() < 8 || digits.len() > 15 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(AuthFlowError::InvalidInput(
                "phone number must be E.164".into(),
            ));
        }
        Ok(trimmed.to_owned())
    }

    fn generate_phone_otp() -> Result<String, AuthFlowError> {
        let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
        Ok(token
            .chars()
            .take(6)
            .collect::<String>()
            .to_ascii_uppercase())
    }

    async fn set_user_phone_number<S>(
        auth: &ArchitectAuth<S>,
        user: AuthUser,
        phone_number: &str,
        verified: bool,
    ) -> Result<AuthUser, AuthFlowError>
    where
        S: AuthStorage,
    {
        let metadata_json = update_phone_metadata(&user.metadata_json, phone_number, verified)?;
        auth.storage
            .update_user_profile(
                user.id,
                user.name,
                user.username,
                user.display_username,
                user.image,
                metadata_json,
            )
            .await
    }

    fn user_phone_number(user: &AuthUser) -> Result<Option<String>, AuthFlowError> {
        let metadata = serde_json::from_str::<Value>(&user.metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        Ok(metadata
            .get(PHONE_METADATA_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    fn phone_metadata_json(phone_number: &str, verified: bool) -> Result<String, AuthFlowError> {
        update_phone_metadata("{}", phone_number, verified)
    }

    fn update_phone_metadata(
        metadata_json: &str,
        phone_number: &str,
        verified: bool,
    ) -> Result<String, AuthFlowError> {
        let mut metadata = serde_json::from_str::<Value>(metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        if metadata.is_null() {
            metadata = json!({});
        }
        let Value::Object(object) = &mut metadata else {
            return Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            ));
        };
        object.insert(
            PHONE_METADATA_KEY.into(),
            Value::String(phone_number.into()),
        );
        object.insert(PHONE_VERIFIED_METADATA_KEY.into(), Value::Bool(verified));
        serde_json::to_string(&metadata).map_err(|err| AuthFlowError::Internal(err.to_string()))
    }

    fn phone_identifier(phone_number: &str) -> String {
        format!("phone-number:{phone_number}")
    }
}
pub mod siwe {
    use auth_proto::{AuthAccountCreate, AuthFlowError, AuthSessionBundle, AuthUserCreate};
    use chrono::{Duration, Utc};

    use crate::{
        ArchitectAuth, AuthStorage, CreateSiweNonce, CurrentSession, LinkSiweAddress,
        VerificationToken, VerifySiweMessage,
        crypto::{generate_token, hash_token},
        flows::last_login_method::record_last_login_method,
    };

    const SIWE_PROVIDER_ID: &str = "siwe";
    const SIWE_NONCE_TTL_SECONDS: i64 = 300;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ParsedSiweMessage {
        domain: String,
        address: String,
        nonce: String,
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.siwe.nonce]
        pub async fn create_siwe_nonce(
            &self,
            _input: CreateSiweNonce,
        ) -> Result<VerificationToken, AuthFlowError> {
            let nonce = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage
                .create_verification(auth_proto::AuthVerificationCreate {
                    identifier: siwe_nonce_identifier(&nonce),
                    value_hash: hash_token(&self.config.secret, &nonce),
                    expires_at: Utc::now() + Duration::seconds(SIWE_NONCE_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken {
                identifier: siwe_nonce_identifier(&nonce),
                token: nonce,
            })
        }

        // r[impl auth.siwe.verify]
        // r[impl auth.siwe.domain]
        // r[impl auth.siwe.replay]
        // r[impl auth.siwe.linked-account]
        // r[impl auth.siwe.signup]
        pub async fn verify_siwe_message(
            &self,
            input: VerifySiweMessage,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let parsed = self
                .consume_valid_siwe_message(&input.message, &input.signature)
                .await?;
            let address = parsed.address;
            let user = if let Some(account) = self
                .storage
                .find_account_by_provider_account(SIWE_PROVIDER_ID, &address)
                .await?
            {
                self.storage
                    .find_user_by_id(account.user_id)
                    .await?
                    .ok_or(AuthFlowError::InvalidCredentials)?
            } else {
                if !self.config.siwe.signup_enabled {
                    return Err(AuthFlowError::InvalidCredentials);
                }
                let user = self
                    .storage
                    .create_user(AuthUserCreate {
                        email: None,
                        name: None,
                        email_verified: false,
                        image: None,
                        username: None,
                        display_username: None,
                        two_factor_enabled: false,
                        role: None,
                        banned: false,
                        ban_reason: None,
                        ban_expires: None,
                        metadata_json: format!(r#"{{"siwe_address":"{address}"}}"#),
                    })
                    .await?;
                self.storage
                    .create_account(AuthAccountCreate {
                        account_id: address.clone(),
                        provider_id: SIWE_PROVIDER_ID.into(),
                        user_id: user.id,
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: None,
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: None,
                        password_hash: None,
                    })
                    .await?;
                user
            };
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            record_last_login_method(self, bundle, "siwe").await
        }

        // r[impl auth.siwe.address-link]
        pub async fn link_siwe_address(&self, input: LinkSiweAddress) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let parsed = self
                .consume_valid_siwe_message(&input.message, &input.signature)
                .await?;
            if self
                .storage
                .find_account_by_provider_account(SIWE_PROVIDER_ID, &parsed.address)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "siwe address already linked".into(),
                ));
            }
            self.storage
                .create_account(AuthAccountCreate {
                    account_id: parsed.address,
                    provider_id: SIWE_PROVIDER_ID.into(),
                    user_id: bundle.user.id,
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: None,
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: None,
                    password_hash: None,
                })
                .await?;
            Ok(())
        }

        async fn consume_valid_siwe_message(
            &self,
            message: &str,
            signature: &str,
        ) -> Result<ParsedSiweMessage, AuthFlowError> {
            let parsed = parse_siwe_message(message)?;
            if parsed.domain != self.config.siwe.domain {
                return Err(AuthFlowError::PermissionDenied);
            }
            verify_test_signature(&self.config.secret, message, &parsed.address, signature)?;
            let identifier = siwe_nonce_identifier(&parsed.nonce);
            let value_hash = hash_token(&self.config.secret, &parsed.nonce);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.storage.delete_verification(verification.id).await?;
            Ok(parsed)
        }
    }

    pub fn test_siwe_signature(secret: &str, message: &str, address: &str) -> String {
        format!(
            "test:{}",
            hash_token(secret, &format!("{message}:{address}"))
        )
    }

    fn verify_test_signature(
        secret: &str,
        message: &str,
        address: &str,
        signature: &str,
    ) -> Result<(), AuthFlowError> {
        if signature == test_siwe_signature(secret, message, address) {
            Ok(())
        } else {
            Err(AuthFlowError::InvalidCredentials)
        }
    }

    fn parse_siwe_message(message: &str) -> Result<ParsedSiweMessage, AuthFlowError> {
        let mut lines = message.lines();
        let domain = lines
            .next()
            .ok_or_else(|| AuthFlowError::InvalidInput("siwe domain is required".into()))?
            .trim()
            .to_owned();
        let address = lines
            .find_map(|line| line.strip_prefix("Address: "))
            .ok_or_else(|| AuthFlowError::InvalidInput("siwe address is required".into()))?
            .trim()
            .to_ascii_lowercase();
        if !is_ethereum_address(&address) {
            return Err(AuthFlowError::InvalidInput(
                "siwe address is invalid".into(),
            ));
        }
        let nonce = message
            .lines()
            .find_map(|line| line.strip_prefix("Nonce: "))
            .ok_or_else(|| AuthFlowError::InvalidInput("siwe nonce is required".into()))?
            .trim()
            .to_owned();
        Ok(ParsedSiweMessage {
            domain,
            address,
            nonce,
        })
    }

    fn is_ethereum_address(address: &str) -> bool {
        address.len() == 42
            && address.starts_with("0x")
            && address[2..].chars().all(|ch| ch.is_ascii_hexdigit())
    }

    fn siwe_nonce_identifier(nonce: &str) -> String {
        format!("siwe-nonce:{nonce}")
    }
}
pub mod haveibeenpwned {
    use auth_proto::AuthFlowError;
    use sha1::{Digest, Sha1};

    use crate::{
        ArchitectAuth, AuthStorage, CheckPasswordBreach, PasswordBreachCheck,
        config::{BreachedPasswordFailurePolicy, BreachedPasswordProvider},
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.hibp.range-check]
        // r[impl auth.hibp.failure-policy]
        pub async fn check_password_breach(
            &self,
            input: CheckPasswordBreach,
        ) -> Result<PasswordBreachCheck, AuthFlowError> {
            check_password_breach_with_config(&self.config.breached_passwords, &input.password)
        }

        pub(crate) async fn reject_breached_password(
            &self,
            password: &str,
        ) -> Result<(), AuthFlowError> {
            let check =
                check_password_breach_with_config(&self.config.breached_passwords, password)?;
            if check.breached {
                Err(AuthFlowError::InvalidInput(
                    "password has been breached".into(),
                ))
            } else {
                Ok(())
            }
        }
    }

    pub fn sha1_prefix_suffix(password: &str) -> (String, String) {
        let mut hasher = Sha1::new();
        hasher.update(password.as_bytes());
        let digest = hasher.finalize();
        let hash = digest
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        (hash[..5].to_owned(), hash[5..].to_owned())
    }

    fn check_password_breach_with_config(
        config: &crate::config::BreachedPasswordConfig,
        password: &str,
    ) -> Result<PasswordBreachCheck, AuthFlowError> {
        let (prefix, suffix) = sha1_prefix_suffix(password);
        match &config.provider {
            BreachedPasswordProvider::Disabled => Ok(PasswordBreachCheck {
                breached: false,
                count: 0,
                prefix,
            }),
            BreachedPasswordProvider::Unavailable => match config.failure_policy {
                BreachedPasswordFailurePolicy::Allow => Ok(PasswordBreachCheck {
                    breached: false,
                    count: 0,
                    prefix,
                }),
                BreachedPasswordFailurePolicy::Deny => Err(AuthFlowError::PermissionDenied),
            },
            BreachedPasswordProvider::Test { breached_passwords } => {
                let count = breached_passwords
                    .iter()
                    .filter(|candidate| {
                        let (candidate_prefix, candidate_suffix) = sha1_prefix_suffix(candidate);
                        candidate_prefix == prefix && candidate_suffix == suffix
                    })
                    .count() as u64;
                Ok(PasswordBreachCheck {
                    breached: count > 0,
                    count,
                    prefix,
                })
            }
        }
    }
}
pub mod mcp {
    use auth_proto::AuthFlowError;

    use crate::{
        ArchitectAuth, AuthStorage, AuthenticateBearerToken, AuthorizeMcpRequest,
        AuthorizeOrganizationAction, BearerTokenStrategy, CurrentSession, McpAuthorization,
        VerifyApiKey,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.mcp.session]
        // r[impl auth.mcp.permissions]
        pub async fn authorize_mcp_request(
            &self,
            input: AuthorizeMcpRequest,
        ) -> Result<McpAuthorization, AuthFlowError> {
            if let Some(session_token) = input.session_token {
                let bundle = self
                    .current_session(CurrentSession {
                        token: session_token.clone(),
                    })
                    .await?;
                if let (Some(organization_id), Some(resource), Some(action)) =
                    (input.organization_id, input.resource, input.action)
                {
                    self.authorize_organization_action(AuthorizeOrganizationAction {
                        session_token,
                        organization_id,
                        resource,
                        action,
                    })
                    .await?;
                }
                return Ok(McpAuthorization {
                    allowed: true,
                    user_id: bundle.user.id,
                    session_id: Some(bundle.session.id),
                    api_key_id: None,
                });
            }

            let bundle = self
                .authenticate_bearer_token(AuthenticateBearerToken {
                    authorization_header: input.authorization_header,
                })
                .await?;
            match bundle.strategy {
                BearerTokenStrategy::Session => {
                    if let (Some(organization_id), Some(resource), Some(action)) =
                        (input.organization_id, input.resource, input.action)
                    {
                        self.authorize_organization_action(AuthorizeOrganizationAction {
                            session_token: bundle.token,
                            organization_id,
                            resource,
                            action,
                        })
                        .await?;
                    }
                }
                BearerTokenStrategy::ApiKey => {
                    if let (Some(resource), Some(action)) = (input.resource, input.action) {
                        self.verify_api_key(VerifyApiKey {
                            key: bundle.token,
                            permission: Some(format!("{resource}:{action}")),
                        })
                        .await?;
                    }
                }
            }

            Ok(McpAuthorization {
                allowed: true,
                user_id: bundle.user.id,
                session_id: bundle.session.map(|session| session.id),
                api_key_id: bundle.api_key.map(|api_key| api_key.id),
            })
        }
    }
}
pub mod magic_link {
    use auth_proto::{AuthFlowError, AuthUserCreate, AuthVerificationCreate};
    use chrono::{Duration, Utc};

    use super::email_password::normalize_email;
    use crate::{
        ArchitectAuth, AuthStorage, MagicLinkToken, MagicLinkVerification, SendMagicLink,
        VerifyMagicLink,
        crypto::{generate_token, hash_token},
        flows::last_login_method::record_last_login_method,
    };

    const MAGIC_LINK_TTL_SECONDS: i64 = 600;
    const MAGIC_LINK_RESEND_SECONDS: i64 = 60;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.magic.send]
        // r[impl auth.magic.expiry]
        // r[impl auth.magic.redirect-trust]
        pub async fn send_magic_link(
            &self,
            input: SendMagicLink,
        ) -> Result<MagicLinkToken, AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            let callback_url = self.trusted_magic_link_callback(input.callback_url.as_deref())?;
            let identifier = magic_link_identifier(&canonical_email);
            if let Some(existing) = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                && existing.created_at + Duration::seconds(MAGIC_LINK_RESEND_SECONDS) > Utc::now()
            {
                return Err(AuthFlowError::PermissionDenied);
            }

            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &token),
                    expires_at: Utc::now() + Duration::seconds(MAGIC_LINK_TTL_SECONDS),
                })
                .await?;
            let url = format!(
                "{}?token={}&email={}",
                callback_url,
                token,
                canonical_email.replace('@', "%40")
            );
            Ok(MagicLinkToken {
                identifier,
                token,
                url,
                callback_url,
            })
        }

        // r[impl auth.magic.verify]
        // r[impl auth.magic.single-use]
        // r[impl auth.magic.session]
        // r[impl auth.magic.redirect-trust]
        pub async fn verify_magic_link(
            &self,
            input: VerifyMagicLink,
        ) -> Result<MagicLinkVerification, AuthFlowError> {
            let canonical_email = normalize_email(&input.email)?;
            let redirect_url = self.trusted_magic_link_callback(input.callback_url.as_deref())?;
            let identifier = magic_link_identifier(&canonical_email);
            let value_hash = hash_token(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }

            let user = if let Some(user) = self.storage.find_user_by_email(&canonical_email).await?
            {
                user
            } else {
                self.storage
                    .create_user(AuthUserCreate {
                        email: Some(canonical_email),
                        name: None,
                        email_verified: true,
                        image: None,
                        username: None,
                        display_username: None,
                        two_factor_enabled: false,
                        role: None,
                        banned: false,
                        ban_reason: None,
                        ban_expires: None,
                        metadata_json: "{}".into(),
                    })
                    .await?
            };
            self.storage.delete_verification(verification.id).await?;
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            let bundle = record_last_login_method(self, bundle, "magic-link").await?;
            Ok(MagicLinkVerification {
                user: bundle.user,
                session: bundle.session,
                token: bundle.token,
                redirect_url,
            })
        }

        fn trusted_magic_link_callback(
            &self,
            callback_url: Option<&str>,
        ) -> Result<String, AuthFlowError> {
            let callback_url = callback_url.unwrap_or(self.config.base_url.as_str());
            if callback_url == self.config.base_url
                || callback_url
                    .starts_with(&format!("{}/", self.config.base_url.trim_end_matches('/')))
            {
                Ok(callback_url.to_owned())
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }
    }

    fn magic_link_identifier(canonical_email: &str) -> String {
        format!("magic-link:{canonical_email}")
    }
}
pub mod jwt {
    use auth_proto::AuthFlowError;
    use chrono::{Duration, Utc};
    use jsonwebtoken::{
        Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
    };

    use crate::{
        ArchitectAuth, AuthStorage, IssueJwt, JwtClaims, JwtKeyDescriptor, JwtKeySet, JwtToken,
        JwtVerification, VerifyJwt, commands::CurrentSession,
    };

    const DEFAULT_JWT_TTL_SECONDS: i64 = 900;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.jwt.sign]
        // r[impl auth.jwt.claims]
        // r[impl auth.jwt.rotation]
        pub async fn issue_jwt(&self, input: IssueJwt) -> Result<JwtToken, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let key = self
                .config
                .jwt
                .keys
                .iter()
                .find(|key| key.id == self.config.jwt.active_key_id)
                .ok_or_else(|| AuthFlowError::Internal("active JWT key missing".into()))?;
            let now = Utc::now();
            let expires_at = now
                + Duration::seconds(input.expires_in_seconds.unwrap_or(DEFAULT_JWT_TTL_SECONDS));
            let extra = input
                .claims_json
                .map(|claims| {
                    serde_json::from_str::<serde_json::Value>(&claims)
                        .map_err(|_| AuthFlowError::InvalidInput("claims_json must be JSON".into()))
                })
                .transpose()?;
            let claims = JwtClaims {
                iss: self.config.jwt.issuer.clone(),
                aud: input
                    .audience
                    .unwrap_or_else(|| self.config.jwt.audience.clone()),
                sub: bundle.user.id.to_string(),
                sid: bundle.session.id.to_string(),
                iat: now.timestamp(),
                exp: expires_at.timestamp(),
                extra,
            };
            let mut header = Header::new(Algorithm::HS256);
            header.kid = Some(key.id.clone());
            let token = encode(
                &header,
                &claims,
                &EncodingKey::from_secret(key.secret.as_bytes()),
            )
            .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            Ok(JwtToken {
                token,
                key_id: key.id.clone(),
                expires_at,
            })
        }

        // r[impl auth.jwt.verify]
        // r[impl auth.jwt.expiry]
        // r[impl auth.jwt.issuer-audience]
        // r[impl auth.jwt.revoked-session]
        // r[impl auth.jwt.rotation]
        pub async fn verify_jwt(&self, input: VerifyJwt) -> Result<JwtVerification, AuthFlowError> {
            let header =
                decode_header(&input.token).map_err(|_| AuthFlowError::InvalidCredentials)?;
            let key_id = header.kid.ok_or(AuthFlowError::InvalidCredentials)?;
            let key = self
                .config
                .jwt
                .keys
                .iter()
                .find(|key| key.id == key_id)
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let mut validation = Validation::new(Algorithm::HS256);
            validation.leeway = 0;
            validation.set_issuer(&[self.config.jwt.issuer.as_str()]);
            let audience = input
                .audience
                .as_deref()
                .unwrap_or(self.config.jwt.audience.as_str());
            validation.set_audience(&[audience]);
            let token_data = decode::<JwtClaims>(
                &input.token,
                &DecodingKey::from_secret(key.secret.as_bytes()),
                &validation,
            )
            .map_err(|_| AuthFlowError::InvalidCredentials)?;

            let user_id: uuid::Uuid = token_data
                .claims
                .sub
                .parse()
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let session_id: uuid::Uuid = token_data
                .claims
                .sid
                .parse()
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let sessions = self.storage.list_sessions_by_user_id(user_id).await?;
            let session = sessions
                .into_iter()
                .find(|session| session.id == session_id)
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if !session.active || session.expires_at <= Utc::now() {
                return Err(AuthFlowError::SessionExpired);
            }
            Ok(JwtVerification {
                claims: token_data.claims,
            })
        }

        // r[impl auth.jwt.jwks]
        pub fn jwt_key_set(&self) -> JwtKeySet {
            JwtKeySet {
                keys: self
                    .config
                    .jwt
                    .keys
                    .iter()
                    .map(|key| JwtKeyDescriptor {
                        kid: key.id.clone(),
                        alg: "HS256".into(),
                        active: key.id == self.config.jwt.active_key_id,
                    })
                    .collect(),
            }
        }
    }
}
pub mod oidc_provider {
    use auth_proto::{AuthFlowError, AuthVerificationCreate};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::{Duration, Utc};
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, IssueJwt, VerifyJwt,
        commands::{
            AuthorizeOidc, ExchangeOidcToken, GetOidcUserInfo, OidcAuthorization,
            OidcClientRegistration, OidcDiscovery, OidcTokenResponse, OidcUserInfo,
            RegisterOidcClient,
        },
        config::OidcClientConfig,
        crypto::{generate_token, hash_token},
    };

    const SUPPORTED_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access"];

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct OidcCodeState {
        session_token: String,
        client_id: String,
        redirect_uri: String,
        scope: String,
        nonce: Option<String>,
        code_challenge: Option<String>,
        code_challenge_method: Option<String>,
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.oidc.discovery]
        pub fn oidc_discovery(&self) -> OidcDiscovery {
            let base_url = self.config.base_url.trim_end_matches('/');
            OidcDiscovery {
                issuer: self.config.oidc.issuer.clone(),
                authorization_endpoint: format!("{base_url}/oauth2/authorize"),
                token_endpoint: format!("{base_url}/oauth2/token"),
                userinfo_endpoint: format!("{base_url}/oauth2/userinfo"),
                jwks_uri: format!("{base_url}/auth/jwt/jwks"),
                registration_endpoint: format!("{base_url}/oauth2/register"),
                scopes_supported: SUPPORTED_SCOPES
                    .iter()
                    .map(|scope| (*scope).into())
                    .collect(),
                response_types_supported: vec!["code".into()],
                grant_types_supported: vec!["authorization_code".into(), "refresh_token".into()],
                token_endpoint_auth_methods_supported: vec![
                    "client_secret_post".into(),
                    "none".into(),
                ],
                id_token_signing_alg_values_supported: vec!["HS256".into()],
                code_challenge_methods_supported: vec!["S256".into()],
                claims_supported: vec![
                    "sub".into(),
                    "iss".into(),
                    "aud".into(),
                    "exp".into(),
                    "iat".into(),
                    "email".into(),
                    "email_verified".into(),
                    "name".into(),
                    "picture".into(),
                ],
            }
        }

        // r[impl auth.oidc.client-registration]
        pub fn register_oidc_client(
            &self,
            input: RegisterOidcClient,
        ) -> Result<OidcClientRegistration, AuthFlowError> {
            if !self.config.oidc.allow_dynamic_client_registration {
                return Err(AuthFlowError::PermissionDenied);
            }
            if input.redirect_uris.is_empty()
                || input.redirect_uris.iter().any(|uri| {
                    !(uri.starts_with("https://") || uri.starts_with("http://localhost"))
                })
            {
                return Err(AuthFlowError::InvalidInput(
                    "redirect_uris must contain registered http localhost or https URLs".into(),
                ));
            }
            let public_client = input
                .token_endpoint_auth_method
                .as_deref()
                .is_some_and(|method| method == "none");
            Ok(OidcClientRegistration {
                client_id: generate_token()
                    .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                client_secret: if public_client {
                    None
                } else {
                    Some(generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?)
                },
                client_name: input.client_name.unwrap_or_else(|| "OIDC client".into()),
                redirect_uris: input.redirect_uris,
                scope: normalize_scope(input.scope.as_deref())?,
                token_endpoint_auth_method: input
                    .token_endpoint_auth_method
                    .unwrap_or_else(|| "client_secret_post".into()),
            })
        }

        // r[impl auth.oidc.authorization-code]
        // r[impl auth.oidc.pkce]
        // r[impl auth.oidc.consent]
        pub async fn authorize_oidc(
            &self,
            input: AuthorizeOidc,
        ) -> Result<OidcAuthorization, AuthFlowError> {
            self.current_session(CurrentSession {
                token: input.session_token.clone(),
            })
            .await?;
            let client = self.oidc_client(&input.client_id)?;
            if client.disabled {
                return Err(AuthFlowError::PermissionDenied);
            }
            if input.response_type != "code" {
                return Err(AuthFlowError::InvalidInput(
                    "response_type must be code".into(),
                ));
            }
            if !client
                .redirect_uris
                .iter()
                .any(|uri| uri == &input.redirect_uri)
            {
                return Err(AuthFlowError::InvalidInput(
                    "redirect_uri is not registered".into(),
                ));
            }
            let scope = normalize_scope(input.scope.as_deref())?;
            ensure_client_scopes(client, &scope)?;
            if self.config.oidc.require_pkce && input.code_challenge.is_none() {
                return Err(AuthFlowError::InvalidInput("pkce is required".into()));
            }
            if input.code_challenge.is_some()
                && input.code_challenge_method.as_deref() != Some("S256")
            {
                return Err(AuthFlowError::InvalidInput(
                    "code_challenge_method must be S256".into(),
                ));
            }
            if input.prompt.as_deref() == Some("consent") && !client.skip_consent {
                return Err(AuthFlowError::VerificationRequired);
            }

            let code = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let state = OidcCodeState {
                session_token: input.session_token,
                client_id: client.client_id.clone(),
                redirect_uri: input.redirect_uri.clone(),
                scope: scope.clone(),
                nonce: input.nonce,
                code_challenge: input.code_challenge,
                code_challenge_method: input.code_challenge_method,
            };
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: oidc_code_identifier(&self.config.secret, &code),
                    value_hash: serde_json::to_string(&state)
                        .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                    expires_at: Utc::now() + Duration::seconds(self.config.oidc.code_ttl_seconds),
                })
                .await?;

            Ok(OidcAuthorization {
                redirect_uri: append_query(&input.redirect_uri, &code, input.state.as_deref()),
                code,
                state: input.state,
                scope,
            })
        }

        // r[impl auth.oidc.token]
        // r[impl auth.oidc.refresh-token]
        // r[impl auth.oidc.pkce]
        pub async fn exchange_oidc_token(
            &self,
            input: ExchangeOidcToken,
        ) -> Result<OidcTokenResponse, AuthFlowError> {
            match input.grant_type.as_str() {
                "authorization_code" => self.exchange_oidc_authorization_code(input).await,
                "refresh_token" => self.exchange_oidc_refresh_token(input).await,
                _ => Err(AuthFlowError::InvalidInput("unsupported grant_type".into())),
            }
        }

        // r[impl auth.oidc.userinfo]
        pub async fn get_oidc_user_info(
            &self,
            input: GetOidcUserInfo,
        ) -> Result<OidcUserInfo, AuthFlowError> {
            let verification = self
                .verify_jwt(VerifyJwt {
                    token: input.access_token,
                    audience: None,
                })
                .await?;
            let user_id = verification
                .claims
                .sub
                .parse()
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let user = self
                .storage
                .find_user_by_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let scopes = verification
                .claims
                .extra
                .as_ref()
                .and_then(|extra| extra.get("scope"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("openid");
            Ok(OidcUserInfo {
                sub: user.id.to_string(),
                email: scopes.contains("email").then_some(user.email).flatten(),
                email_verified: scopes.contains("email").then_some(user.email_verified),
                name: scopes.contains("profile").then_some(user.name).flatten(),
                picture: scopes.contains("profile").then_some(user.image).flatten(),
            })
        }

        fn oidc_client(&self, client_id: &str) -> Result<&OidcClientConfig, AuthFlowError> {
            self.config
                .oidc
                .clients
                .iter()
                .find(|client| client.client_id == client_id)
                .ok_or(AuthFlowError::InvalidCredentials)
        }

        async fn exchange_oidc_authorization_code(
            &self,
            input: ExchangeOidcToken,
        ) -> Result<OidcTokenResponse, AuthFlowError> {
            let code = input
                .code
                .as_deref()
                .ok_or_else(|| AuthFlowError::InvalidInput("code is required".into()))?;
            let identifier = oidc_code_identifier(&self.config.secret, code);
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state: OidcCodeState = serde_json::from_str(&verification.value_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            self.validate_oidc_client_secret(&input.client_id, input.client_secret.as_deref())?;
            if state.client_id != input.client_id
                || input.redirect_uri.as_deref() != Some(&state.redirect_uri)
            {
                return Err(AuthFlowError::InvalidCredentials);
            }
            validate_pkce(&state, input.code_verifier.as_deref())?;
            self.storage.delete_verification(verification.id).await?;
            self.issue_oidc_tokens(state).await
        }

        async fn exchange_oidc_refresh_token(
            &self,
            input: ExchangeOidcToken,
        ) -> Result<OidcTokenResponse, AuthFlowError> {
            self.validate_oidc_client_secret(&input.client_id, input.client_secret.as_deref())?;
            let refresh_token = input
                .refresh_token
                .as_deref()
                .ok_or_else(|| AuthFlowError::InvalidInput("refresh_token is required".into()))?;
            let identifier = oidc_refresh_identifier(&self.config.secret, refresh_token);
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state: OidcCodeState = serde_json::from_str(&verification.value_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            if state.client_id != input.client_id {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.issue_oidc_tokens(state).await
        }

        fn validate_oidc_client_secret(
            &self,
            client_id: &str,
            client_secret: Option<&str>,
        ) -> Result<(), AuthFlowError> {
            let client = self.oidc_client(client_id)?;
            if client.public_client {
                return Ok(());
            }
            if client.client_secret.as_deref() != client_secret {
                return Err(AuthFlowError::InvalidCredentials);
            }
            Ok(())
        }

        async fn issue_oidc_tokens(
            &self,
            state: OidcCodeState,
        ) -> Result<OidcTokenResponse, AuthFlowError> {
            let extra = json!({
                "scope": state.scope.clone(),
                "client_id": state.client_id.clone(),
                "nonce": state.nonce.clone(),
            });
            let access_token = self
                .issue_jwt(IssueJwt {
                    session_token: state.session_token.clone(),
                    audience: None,
                    expires_in_seconds: Some(self.config.oidc.access_token_ttl_seconds),
                    claims_json: Some(extra.to_string()),
                })
                .await?;
            let id_token = self
                .issue_jwt(IssueJwt {
                    session_token: state.session_token.clone(),
                    audience: Some(state.client_id.clone()),
                    expires_in_seconds: Some(self.config.oidc.access_token_ttl_seconds),
                    claims_json: Some(extra.to_string()),
                })
                .await?;
            let refresh_token = if state
                .scope
                .split_whitespace()
                .any(|scope| scope == "offline_access")
            {
                let token =
                    generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
                self.storage
                    .create_verification(AuthVerificationCreate {
                        identifier: oidc_refresh_identifier(&self.config.secret, &token),
                        value_hash: serde_json::to_string(&state)
                            .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                        expires_at: Utc::now()
                            + Duration::seconds(self.config.oidc.refresh_token_ttl_seconds),
                    })
                    .await?;
                Some(token)
            } else {
                None
            };
            Ok(OidcTokenResponse {
                access_token: access_token.token,
                id_token: id_token.token,
                refresh_token,
                token_type: "Bearer".into(),
                expires_in: self.config.oidc.access_token_ttl_seconds,
                scope: state.scope,
            })
        }
    }

    fn normalize_scope(scope: Option<&str>) -> Result<String, AuthFlowError> {
        let scopes = scope.unwrap_or("openid");
        let mut normalized = Vec::new();
        for scope in scopes.split_whitespace() {
            if !SUPPORTED_SCOPES.contains(&scope) {
                return Err(AuthFlowError::InvalidInput(format!(
                    "unsupported scope: {scope}"
                )));
            }
            if !normalized.contains(&scope) {
                normalized.push(scope);
            }
        }
        if !normalized.contains(&"openid") {
            normalized.insert(0, "openid");
        }
        Ok(normalized.join(" "))
    }

    fn ensure_client_scopes(client: &OidcClientConfig, scope: &str) -> Result<(), AuthFlowError> {
        for requested in scope.split_whitespace() {
            if !client.scopes.iter().any(|allowed| allowed == requested) {
                return Err(AuthFlowError::PermissionDenied);
            }
        }
        Ok(())
    }

    fn validate_pkce(state: &OidcCodeState, verifier: Option<&str>) -> Result<(), AuthFlowError> {
        let Some(challenge) = state.code_challenge.as_deref() else {
            return Ok(());
        };
        let verifier = verifier.ok_or(AuthFlowError::InvalidCredentials)?;
        let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        if state.code_challenge_method.as_deref() == Some("S256") && computed == challenge {
            Ok(())
        } else {
            Err(AuthFlowError::InvalidCredentials)
        }
    }

    fn append_query(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
        let separator = if redirect_uri.contains('?') { '&' } else { '?' };
        let mut url = format!("{redirect_uri}{separator}code={code}");
        if let Some(state) = state {
            url.push_str("&state=");
            url.push_str(state);
        }
        url
    }

    fn oidc_code_identifier(secret: &str, code: &str) -> String {
        format!("oidc-code:{}", hash_token(secret, code))
    }

    fn oidc_refresh_identifier(secret: &str, refresh_token: &str) -> String {
        format!("oidc-refresh:{}", hash_token(secret, refresh_token))
    }
}
pub mod anonymous {
    use auth_proto::{
        AuthAccountCreate, AuthFlowError, AuthSessionBundle, AuthUser, AuthUserCreate,
    };
    use chrono::{Duration, Utc};
    use serde_json::{Value, json};

    use super::email_password::{
        PASSWORD_PROVIDER_ID, normalize_email, validate_metadata, validate_password_strength,
    };
    use crate::{
        ArchitectAuth, AuthStorage, CleanupAnonymousUsers, CleanupAnonymousUsersResult,
        LinkAnonymousEmailPassword, SignInAnonymous,
        commands::CurrentSession,
        crypto::hash_password,
        flows::{
            last_login_method::record_last_login_method, username::normalize_optional_username,
        },
    };

    const ANONYMOUS_ROLE: &str = "anonymous";

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.anonymous.signin]
        // r[impl auth.anonymous.policy]
        pub async fn sign_in_anonymous(
            &self,
            input: SignInAnonymous,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let metadata_json = anonymous_metadata(input.metadata_json.as_deref())?;
            let user = self
                .storage
                .create_user(AuthUserCreate {
                    email: None,
                    name: None,
                    email_verified: false,
                    image: None,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: Some(ANONYMOUS_ROLE.into()),
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json,
                })
                .await?;
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            record_last_login_method(self, bundle, "anonymous").await
        }

        // r[impl auth.anonymous.link]
        // r[impl auth.anonymous.revoke-obsolete]
        pub async fn link_anonymous_email_password(
            &self,
            input: LinkAnonymousEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            require_anonymous_user(&bundle.user)?;
            let canonical_email = normalize_email(&input.email)?;
            validate_password_strength(&input.password)?;
            self.reject_breached_password(&input.password).await?;
            let (username, display_username) = normalize_optional_username(input.username)?;
            if let Some(username) = username.as_deref()
                && self
                    .storage
                    .find_user_by_username(username)
                    .await?
                    .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "username already exists".into(),
                ));
            }

            if self
                .storage
                .find_user_by_email(&canonical_email)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput("email already exists".into()));
            }
            if self
                .storage
                .find_password_account_by_user_id(bundle.user.id)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "anonymous user already has a password credential".into(),
                ));
            }

            let password_hash = hash_password(&input.password)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            self.storage
                .update_user_profile(
                    bundle.user.id,
                    input.name,
                    username,
                    display_username,
                    input.image,
                    mark_permanent_metadata(&bundle.user.metadata_json)?,
                )
                .await?;
            let user = self
                .storage
                .update_user_email(bundle.user.id, canonical_email.clone(), false)
                .await?;
            let user = self.storage.update_user_role(user.id, None).await?;
            self.storage
                .create_account(AuthAccountCreate {
                    account_id: canonical_email,
                    provider_id: PASSWORD_PROVIDER_ID.into(),
                    user_id: user.id,
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: None,
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: None,
                    password_hash: Some(password_hash),
                })
                .await?;
            self.storage.deactivate_sessions_by_user_id(user.id).await?;
            let bundle = self.issue_session(user, None, None, None, None).await?;
            record_last_login_method(self, bundle, "email").await
        }

        // r[impl auth.anonymous.cleanup]
        pub async fn cleanup_anonymous_users(
            &self,
            input: CleanupAnonymousUsers,
        ) -> Result<CleanupAnonymousUsersResult, AuthFlowError> {
            self.require_admin(&input.session_token).await?;
            let older_than = Utc::now() - Duration::seconds(input.older_than_seconds.max(0));
            let (users, _) = self.storage.list_users(0, 10_000).await?;
            let mut deleted = 0;
            for user in users {
                if user.created_at < older_than && is_anonymous_user(&user) {
                    self.storage.delete_user_by_id(user.id).await?;
                    deleted += 1;
                }
            }
            Ok(CleanupAnonymousUsersResult { deleted })
        }
    }

    fn anonymous_metadata(metadata_json: Option<&str>) -> Result<String, AuthFlowError> {
        validate_metadata(metadata_json)?;
        let mut metadata = metadata_json
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?
            .unwrap_or_else(|| json!({}));
        let Value::Object(object) = &mut metadata else {
            return Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            ));
        };
        object.insert("anonymous".into(), Value::Bool(true));
        serde_json::to_string(&metadata).map_err(|err| AuthFlowError::Internal(err.to_string()))
    }

    fn mark_permanent_metadata(metadata_json: &str) -> Result<String, AuthFlowError> {
        let mut metadata = serde_json::from_str::<Value>(metadata_json)
            .map_err(|_| AuthFlowError::InvalidInput("metadata_json must be JSON".into()))?;
        let Value::Object(object) = &mut metadata else {
            return Err(AuthFlowError::InvalidInput(
                "metadata_json must be a JSON object".into(),
            ));
        };
        object.insert("anonymous".into(), Value::Bool(false));
        object.insert("upgraded_from_anonymous".into(), Value::Bool(true));
        serde_json::to_string(&metadata).map_err(|err| AuthFlowError::Internal(err.to_string()))
    }

    fn require_anonymous_user(user: &AuthUser) -> Result<(), AuthFlowError> {
        if is_anonymous_user(user) {
            Ok(())
        } else {
            Err(AuthFlowError::PermissionDenied)
        }
    }

    fn is_anonymous_user(user: &AuthUser) -> bool {
        user.email.is_none()
            && user.role.as_deref() == Some(ANONYMOUS_ROLE)
            && serde_json::from_str::<Value>(&user.metadata_json)
                .ok()
                .and_then(|metadata| metadata.get("anonymous").and_then(Value::as_bool))
                .unwrap_or(false)
    }
}
pub mod email_verification {}
pub mod oauth {
    use auth_proto::{
        AuthAccountCreate, AuthFlowError, AuthSessionBundle, AuthUserCreate, AuthVerificationCreate,
    };
    use chrono::{Duration, Utc};

    use crate::{
        ArchitectAuth, AuthStorage, BeginOAuthAuthorization, GetOAuthAccessToken, LinkOAuthAccount,
        OAuthAccessToken, OAuthProviderDescriptor, RefreshOAuthToken, SignInOAuthAccount,
        UnlinkOAuthAccount, VerificationToken, VerifyOAuthState,
        commands::CurrentSession,
        crypto::{decrypt_secret, encrypt_secret, generate_token, hash_token},
        flows::last_login_method::record_last_login_method,
    };

    const OAUTH_STATE_TTL_SECONDS: i64 = 600;
    pub const GOOGLE_OAUTH_PROVIDER: OAuthProviderDescriptor = OAuthProviderDescriptor {
        id: "google",
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        user_info_url: "https://www.googleapis.com/oauth2/v3/userinfo",
        scopes: &["openid", "email", "profile"],
    };
    pub const GITHUB_OAUTH_PROVIDER: OAuthProviderDescriptor = OAuthProviderDescriptor {
        id: "github",
        auth_url: "https://github.com/login/oauth/authorize",
        token_url: "https://github.com/login/oauth/access_token",
        user_info_url: "https://api.github.com/user",
        scopes: &["user:email"],
    };
    pub const DISCORD_OAUTH_PROVIDER: OAuthProviderDescriptor = OAuthProviderDescriptor {
        id: "discord",
        auth_url: "https://discord.com/api/oauth2/authorize",
        token_url: "https://discord.com/api/oauth2/token",
        user_info_url: "https://discord.com/api/users/@me",
        scopes: &["identify", "email"],
    };
    pub const BUILT_IN_OAUTH_PROVIDERS: &[OAuthProviderDescriptor] = &[
        GOOGLE_OAUTH_PROVIDER,
        GITHUB_OAUTH_PROVIDER,
        DISCORD_OAUTH_PROVIDER,
    ];

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.oauth.state-csrf]
        pub async fn begin_oauth_authorization(
            &self,
            input: BeginOAuthAuthorization,
        ) -> Result<VerificationToken, AuthFlowError> {
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let identifier = oauth_state_identifier(&input.provider_id);
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &token),
                    expires_at: Utc::now() + Duration::seconds(OAUTH_STATE_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken { identifier, token })
        }

        // r[impl auth.oauth.state-csrf]
        pub async fn verify_oauth_state(
            &self,
            input: VerifyOAuthState,
        ) -> Result<(), AuthFlowError> {
            let identifier = oauth_state_identifier(&input.provider_id);
            let value_hash = hash_token(&self.config.secret, &input.state);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.storage.delete_verification(verification.id).await
        }

        // r[impl auth.oauth.link-authenticated]
        // r[impl auth.oauth.provider-account-unique]
        pub async fn link_oauth_account(
            &self,
            input: LinkOAuthAccount,
        ) -> Result<(), AuthFlowError> {
            let access_token_ciphertext =
                encrypt_optional_secret(&self.config.secret, input.access_token_ciphertext)?;
            let refresh_token_ciphertext =
                encrypt_optional_secret(&self.config.secret, input.refresh_token_ciphertext)?;
            let id_token_ciphertext =
                encrypt_optional_secret(&self.config.secret, input.id_token_ciphertext)?;
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            if self
                .storage
                .find_account_by_provider_account(&input.provider_id, &input.account_id)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "provider account already linked".into(),
                ));
            }

            self.storage
                .create_account(AuthAccountCreate {
                    account_id: input.account_id,
                    provider_id: input.provider_id,
                    user_id: bundle.user.id,
                    access_token_ciphertext,
                    refresh_token_ciphertext,
                    id_token_ciphertext,
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: input.scope,
                    password_hash: None,
                })
                .await?;
            Ok(())
        }

        // r[impl auth.oauth.unlink-last-credential]
        pub async fn unlink_oauth_account(
            &self,
            input: UnlinkOAuthAccount,
        ) -> Result<(), AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let account = self
                .storage
                .find_account_by_provider_account(&input.provider_id, &input.account_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if account.user_id != bundle.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            let has_password = self
                .storage
                .find_password_account_by_user_id(bundle.user.id)
                .await?
                .is_some();
            let oauth_account_count = self
                .storage
                .list_accounts_by_user_id(bundle.user.id)
                .await?
                .into_iter()
                .filter(|account| account.provider_id != "credential")
                .count();
            if !has_password && oauth_account_count <= 1 {
                return Err(AuthFlowError::InvalidInput(
                    "cannot unlink the last sign-in credential".into(),
                ));
            }
            self.storage
                .delete_account_by_provider_account(&input.provider_id, &input.account_id)
                .await
        }

        // r[impl auth.oauth.signin-existing-account]
        // r[impl auth.oauth.signin-new-account]
        // r[impl auth.oauth.email-trust]
        pub async fn sign_in_oauth_account(
            &self,
            input: SignInOAuthAccount,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let method = input.provider_id.clone();
            if let Some(account) = self
                .storage
                .find_account_by_provider_account(&input.provider_id, &input.account_id)
                .await?
            {
                let user = self
                    .storage
                    .find_user_by_id(account.user_id)
                    .await?
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                let bundle = self
                    .issue_session(user, input.ip_address, input.user_agent, None, None)
                    .await?;
                return record_last_login_method(self, bundle, method).await;
            }
            if !self.config.oauth_signup_enabled {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let email = normalize_email(
                input
                    .email
                    .as_deref()
                    .ok_or_else(|| AuthFlowError::InvalidInput("email is required".into()))?,
            )?;
            if self.storage.find_user_by_email(&email).await?.is_some() {
                return Err(AuthFlowError::InvalidInput("email already exists".into()));
            }
            let user = self
                .storage
                .create_user(AuthUserCreate {
                    email: Some(email),
                    name: input.name,
                    email_verified: input.email_verified,
                    image: input.image,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: None,
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json: "{}".into(),
                })
                .await?;
            self.storage
                .create_account(AuthAccountCreate {
                    account_id: input.account_id,
                    provider_id: input.provider_id,
                    user_id: user.id,
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: None,
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: None,
                    password_hash: None,
                })
                .await?;
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            record_last_login_method(self, bundle, method).await
        }

        // r[impl auth.oauth.access-token]
        pub async fn get_oauth_access_token(
            &self,
            input: GetOAuthAccessToken,
        ) -> Result<OAuthAccessToken, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let account = self
                .storage
                .find_account_by_provider_account(&input.provider_id, &input.account_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if account.user_id != bundle.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            let access_token = account
                .access_token_ciphertext
                .as_deref()
                .map(|ciphertext| decrypt_secret(&self.config.secret, ciphertext))
                .transpose()
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            Ok(OAuthAccessToken {
                access_token,
                access_token_expires_at: account.access_token_expires_at,
                scope: account.scope,
            })
        }

        // r[impl auth.oauth.refresh-token]
        pub async fn refresh_oauth_token(
            &self,
            input: RefreshOAuthToken,
        ) -> Result<OAuthAccessToken, AuthFlowError> {
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let account = self
                .storage
                .find_account_by_provider_account(&input.provider_id, &input.account_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if account.user_id != bundle.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            if account.refresh_token_ciphertext.is_none() && input.refresh_token.is_none() {
                return Err(AuthFlowError::InvalidInput(
                    "refresh token is required".into(),
                ));
            }
            let access_token_ciphertext =
                encrypt_optional_secret(&self.config.secret, Some(input.access_token.clone()))?;
            let refresh_token_ciphertext =
                encrypt_optional_secret(&self.config.secret, input.refresh_token)?;
            let id_token_ciphertext = encrypt_optional_secret(&self.config.secret, input.id_token)?;
            let updated = self
                .storage
                .update_oauth_account_tokens(
                    &input.provider_id,
                    &input.account_id,
                    access_token_ciphertext,
                    refresh_token_ciphertext,
                    id_token_ciphertext,
                    input.access_token_expires_at,
                    input.refresh_token_expires_at,
                    input.scope.clone(),
                )
                .await?;
            Ok(OAuthAccessToken {
                access_token: Some(input.access_token),
                access_token_expires_at: updated.access_token_expires_at,
                scope: updated.scope,
            })
        }
    }

    // r[impl auth.oauth.provider-registry]
    // r[impl auth.oauth.generic-provider]
    pub fn built_in_oauth_providers() -> &'static [OAuthProviderDescriptor] {
        BUILT_IN_OAUTH_PROVIDERS
    }

    pub fn generic_oauth_provider(
        id: &'static str,
        auth_url: &'static str,
        token_url: &'static str,
        user_info_url: &'static str,
        scopes: &'static [&'static str],
    ) -> OAuthProviderDescriptor {
        OAuthProviderDescriptor {
            id,
            auth_url,
            token_url,
            user_info_url,
            scopes,
        }
    }

    fn normalize_email(email: &str) -> Result<String, AuthFlowError> {
        let trimmed = email.trim();
        let Some((local, domain)) = trimmed.split_once('@') else {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        };
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        }
        Ok(format!("{local}@{}", domain.to_ascii_lowercase()))
    }

    fn oauth_state_identifier(provider_id: &str) -> String {
        format!("oauth-state:{provider_id}")
    }

    // r[impl auth.oauth.token-encryption]
    fn encrypt_optional_secret(
        secret: &str,
        value: Option<String>,
    ) -> Result<Option<String>, AuthFlowError> {
        value
            .map(|value| {
                encrypt_secret(secret, &value)
                    .map_err(|err| AuthFlowError::Internal(err.to_string()))
            })
            .transpose()
    }
}
pub mod oauth_proxy {
    use auth_proto::AuthFlowError;
    use chrono::Utc;
    use serde::{Deserialize, Serialize};

    use super::oauth::built_in_oauth_providers;
    use crate::{
        ArchitectAuth, AuthStorage, BeginOAuthAuthorization, BeginOAuthProxyAuthorization,
        ConsumeOAuthProxyCallback, ForwardOAuthProxyCallback, OAuthProxyAuthorization,
        OAuthProxyForwarding, OAuthProxyMetadata, OAuthProxyProfile,
        crypto::{decrypt_secret, encrypt_secret},
    };

    #[derive(Debug, Serialize, Deserialize)]
    struct OAuthProxyPayload {
        provider_id: String,
        state: String,
        callback_url: String,
        profile_json: String,
        timestamp: i64,
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.oauth-proxy.metadata]
        pub fn oauth_proxy_metadata(&self) -> OAuthProxyMetadata {
            OAuthProxyMetadata {
                current_url: self.config.oauth_proxy.current_url.clone(),
                production_url: self.config.oauth_proxy.production_url.clone(),
                proxy_callback_url: self.oauth_proxy_callback_url(),
                should_proxy: normalized_origin(&self.config.oauth_proxy.current_url)
                    != normalized_origin(&self.config.oauth_proxy.production_url),
                providers: built_in_oauth_providers().to_vec(),
            }
        }

        // r[impl auth.oauth-proxy.state]
        // r[impl auth.oauth-proxy.provider-composition]
        // r[impl auth.oauth-proxy.redirect-policy]
        pub async fn begin_oauth_proxy_authorization(
            &self,
            input: BeginOAuthProxyAuthorization,
        ) -> Result<OAuthProxyAuthorization, AuthFlowError> {
            self.ensure_oauth_proxy_redirect_allowed(&input.callback_url)?;
            let state = self
                .begin_oauth_authorization(BeginOAuthAuthorization {
                    provider_id: input.provider_id.clone(),
                })
                .await?;
            let production = self.config.oauth_proxy.production_url.trim_end_matches('/');
            Ok(OAuthProxyAuthorization {
                provider_id: input.provider_id,
                state: state.token,
                production_callback_url: format!("{production}/auth/callback"),
                proxy_callback_url: self.oauth_proxy_callback_url(),
            })
        }

        // r[impl auth.oauth-proxy.callback-forwarding]
        // r[impl auth.oauth-proxy.redirect-policy]
        pub fn forward_oauth_proxy_callback(
            &self,
            input: ForwardOAuthProxyCallback,
        ) -> Result<OAuthProxyForwarding, AuthFlowError> {
            self.ensure_oauth_proxy_redirect_allowed(&input.callback_url)?;
            let payload = OAuthProxyPayload {
                provider_id: input.provider_id,
                state: input.state,
                callback_url: input.callback_url.clone(),
                profile_json: input.profile_json,
                timestamp: Utc::now().timestamp(),
            };
            let encrypted_profile = encrypt_secret(
                &self.config.secret,
                &serde_json::to_string(&payload)
                    .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
            )
            .map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let redirect_url = format!(
                "{}?callbackURL={}&profile={}",
                self.oauth_proxy_callback_url(),
                input.callback_url,
                encrypted_profile
            );
            Ok(OAuthProxyForwarding {
                redirect_url,
                encrypted_profile,
            })
        }

        // r[impl auth.oauth-proxy.callback-forwarding]
        // r[impl auth.oauth-proxy.max-age]
        // r[impl auth.oauth-proxy.redirect-policy]
        pub fn consume_oauth_proxy_callback(
            &self,
            input: ConsumeOAuthProxyCallback,
        ) -> Result<OAuthProxyProfile, AuthFlowError> {
            self.ensure_oauth_proxy_redirect_allowed(&input.callback_url)?;
            let plaintext = decrypt_secret(&self.config.secret, &input.encrypted_profile)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let payload: OAuthProxyPayload =
                serde_json::from_str(&plaintext).map_err(|_| AuthFlowError::InvalidCredentials)?;
            if payload.callback_url != input.callback_url {
                return Err(AuthFlowError::PermissionDenied);
            }
            let age = Utc::now().timestamp() - payload.timestamp;
            if age > self.config.oauth_proxy.max_age_seconds || age < -10 {
                return Err(AuthFlowError::InvalidCredentials);
            }
            Ok(OAuthProxyProfile {
                provider_id: payload.provider_id,
                state: payload.state,
                callback_url: payload.callback_url,
                profile_json: payload.profile_json,
            })
        }

        fn oauth_proxy_callback_url(&self) -> String {
            format!(
                "{}{}",
                self.config.oauth_proxy.current_url.trim_end_matches('/'),
                self.config.oauth_proxy.callback_path
            )
        }

        fn ensure_oauth_proxy_redirect_allowed(
            &self,
            callback_url: &str,
        ) -> Result<(), AuthFlowError> {
            let origin = normalized_origin(callback_url);
            if origin.is_empty()
                || !self
                    .config
                    .oauth_proxy
                    .allowed_redirect_origins
                    .iter()
                    .any(|allowed| origin == normalized_origin(allowed))
            {
                return Err(AuthFlowError::PermissionDenied);
            }
            Ok(())
        }
    }

    fn normalized_origin(url: &str) -> String {
        let trimmed = url.trim_end_matches('/');
        let Some((scheme, rest)) = trimmed.split_once("://") else {
            return String::new();
        };
        let host = rest.split('/').next().unwrap_or_default();
        if scheme.is_empty() || host.is_empty() {
            String::new()
        } else {
            format!(
                "{}://{}",
                scheme.to_ascii_lowercase(),
                host.to_ascii_lowercase()
            )
        }
    }

    #[cfg(test)]
    mod boundary_tests {
        use proptest::prelude::*;

        proptest! {
            // r[verify auth.boundary.property-tests]
            #[test]
            fn normalized_origin_is_case_insensitive_and_path_independent(
                scheme in "(https|http|HTTPS|HTTP)",
                host in "[A-Za-z0-9][A-Za-z0-9.-]{0,48}",
                path in "[A-Za-z0-9/_.,~-]{0,48}",
            ) {
                let input = format!("{scheme}://{host}/{path}");
                let expected = format!(
                    "{}://{}",
                    scheme.to_ascii_lowercase(),
                    host.to_ascii_lowercase()
                );

                prop_assert_eq!(super::normalized_origin(&input), expected.clone());
                prop_assert_eq!(super::normalized_origin(&format!("{input}/")), expected);
            }

            // r[verify auth.boundary.property-tests]
            #[test]
            fn normalized_origin_rejects_strings_without_scheme(input in "[^:]{0,128}") {
                prop_assert_eq!(super::normalized_origin(&input), "");
            }
        }
    }
}
pub mod one_tap {
    use auth_proto::{AuthAccountCreate, AuthFlowError, AuthUserCreate};
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use serde::{Deserialize, Serialize};

    use super::email_password::normalize_email;
    use crate::{
        ArchitectAuth, AuthStorage, OneTapCallback, OneTapVerification, crypto::encrypt_secret,
        flows::last_login_method::record_last_login_method,
    };

    const GOOGLE_PROVIDER_ID: &str = "google";

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct OneTapClaims {
        iss: String,
        aud: String,
        sub: String,
        email: Option<String>,
        email_verified: Option<bool>,
        name: Option<String>,
        picture: Option<String>,
        exp: usize,
        iat: Option<usize>,
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.onetap.token-validation]
        // r[impl auth.onetap.existing-user]
        // r[impl auth.onetap.new-user]
        // r[impl auth.onetap.disabled-signup]
        pub async fn one_tap_callback(
            &self,
            input: OneTapCallback,
        ) -> Result<OneTapVerification, AuthFlowError> {
            let claims = self.verify_one_tap_token(&input.id_token)?;
            let raw_email = claims
                .email
                .as_deref()
                .ok_or_else(|| AuthFlowError::InvalidInput("email is required".into()))?;
            let email = normalize_email(raw_email)?;
            let provider_email_verified = claims.email_verified.unwrap_or(false);

            if let Some(account) = self
                .storage
                .find_account_by_provider_account(GOOGLE_PROVIDER_ID, &claims.sub)
                .await?
            {
                let user = self
                    .storage
                    .find_user_by_id(account.user_id)
                    .await?
                    .ok_or(AuthFlowError::InvalidCredentials)?;
                let bundle = self
                    .issue_session(user, input.ip_address, input.user_agent, None, None)
                    .await?;
                let bundle = record_last_login_method(self, bundle, GOOGLE_PROVIDER_ID).await?;
                return Ok(OneTapVerification {
                    user: bundle.user,
                    session: bundle.session,
                    token: bundle.token,
                });
            }

            if let Some(user) = self.storage.find_user_by_email(&email).await? {
                if !user.email_verified || !provider_email_verified {
                    return Err(AuthFlowError::PermissionDenied);
                }
                self.storage
                    .create_account(AuthAccountCreate {
                        account_id: claims.sub,
                        provider_id: GOOGLE_PROVIDER_ID.into(),
                        user_id: user.id,
                        access_token_ciphertext: None,
                        refresh_token_ciphertext: None,
                        id_token_ciphertext: Some(
                            encrypt_secret(&self.config.secret, &input.id_token)
                                .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                        ),
                        access_token_expires_at: None,
                        refresh_token_expires_at: None,
                        scope: Some("openid profile email".into()),
                        password_hash: None,
                    })
                    .await?;
                let bundle = self
                    .issue_session(user, input.ip_address, input.user_agent, None, None)
                    .await?;
                let bundle = record_last_login_method(self, bundle, GOOGLE_PROVIDER_ID).await?;
                return Ok(OneTapVerification {
                    user: bundle.user,
                    session: bundle.session,
                    token: bundle.token,
                });
            }

            if self.config.one_tap.disable_signup {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let user = self
                .storage
                .create_user(AuthUserCreate {
                    email: Some(email),
                    name: claims.name,
                    email_verified: provider_email_verified,
                    image: claims.picture,
                    username: None,
                    display_username: None,
                    two_factor_enabled: false,
                    role: None,
                    banned: false,
                    ban_reason: None,
                    ban_expires: None,
                    metadata_json: "{}".into(),
                })
                .await?;
            self.storage
                .create_account(AuthAccountCreate {
                    account_id: claims.sub,
                    provider_id: GOOGLE_PROVIDER_ID.into(),
                    user_id: user.id,
                    access_token_ciphertext: None,
                    refresh_token_ciphertext: None,
                    id_token_ciphertext: Some(
                        encrypt_secret(&self.config.secret, &input.id_token)
                            .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                    ),
                    access_token_expires_at: None,
                    refresh_token_expires_at: None,
                    scope: Some("openid profile email".into()),
                    password_hash: None,
                })
                .await?;
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            let bundle = record_last_login_method(self, bundle, GOOGLE_PROVIDER_ID).await?;
            Ok(OneTapVerification {
                user: bundle.user,
                session: bundle.session,
                token: bundle.token,
            })
        }

        fn verify_one_tap_token(&self, id_token: &str) -> Result<OneTapClaims, AuthFlowError> {
            let mut validation = Validation::new(Algorithm::HS256);
            validation.leeway = 0;
            validation.set_issuer(&[self.config.one_tap.issuer.as_str()]);
            validation.set_audience(&[self.config.one_tap.client_id.as_str()]);
            decode::<OneTapClaims>(
                id_token,
                &DecodingKey::from_secret(self.config.secret.as_bytes()),
                &validation,
            )
            .map(|token| token.claims)
            .map_err(|_| AuthFlowError::InvalidCredentials)
        }
    }
}
pub mod one_time_token {
    use auth_proto::{AuthFlowError, AuthVerificationCreate};
    use chrono::{Duration, Utc};
    use serde::{Deserialize, Serialize};

    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, GenerateOneTimeToken, OneTimeToken,
        OneTimeTokenVerification, RevokeOneTimeToken, VerifyOneTimeToken,
        crypto::{generate_token, hash_token},
    };

    const DEFAULT_ONE_TIME_TOKEN_TTL_SECONDS: i64 = 180;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct OneTimeTokenState {
        session_token: String,
        scope: Option<String>,
        metadata_json: Option<String>,
    }

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.ott.create]
        // r[impl auth.ott.scope]
        // r[impl auth.ott.metadata]
        pub async fn generate_one_time_token(
            &self,
            input: GenerateOneTimeToken,
        ) -> Result<OneTimeToken, AuthFlowError> {
            self.current_session(CurrentSession {
                token: input.session_token.clone(),
            })
            .await?;
            if let Some(metadata) = input.metadata_json.as_deref() {
                serde_json::from_str::<serde_json::Value>(metadata).map_err(|_| {
                    AuthFlowError::InvalidInput("metadata_json must be JSON".into())
                })?;
            }
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let expires_at = Utc::now()
                + Duration::seconds(
                    input
                        .expires_in_seconds
                        .unwrap_or(DEFAULT_ONE_TIME_TOKEN_TTL_SECONDS),
                );
            let state = OneTimeTokenState {
                session_token: input.session_token,
                scope: input.scope.clone(),
                metadata_json: input.metadata_json.clone(),
            };
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: one_time_token_identifier(&self.config.secret, &token),
                    value_hash: serde_json::to_string(&state)
                        .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                    expires_at,
                })
                .await?;
            Ok(OneTimeToken {
                token,
                expires_at,
                scope: input.scope,
                metadata_json: input.metadata_json,
            })
        }

        // r[impl auth.ott.consume]
        // r[impl auth.ott.expire]
        // r[impl auth.ott.scope]
        pub async fn verify_one_time_token(
            &self,
            input: VerifyOneTimeToken,
        ) -> Result<OneTimeTokenVerification, AuthFlowError> {
            let identifier = one_time_token_identifier(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.delete_verification(verification.id).await?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state: OneTimeTokenState = serde_json::from_str(&verification.value_hash)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            if input.scope.is_some() && input.scope != state.scope {
                return Err(AuthFlowError::PermissionDenied);
            }
            let bundle = self
                .current_session(CurrentSession {
                    token: state.session_token,
                })
                .await?;
            Ok(OneTimeTokenVerification {
                user: bundle.user,
                session: bundle.session,
                token: bundle.token,
                scope: state.scope,
                metadata_json: state.metadata_json,
            })
        }

        // r[impl auth.ott.revoke]
        pub async fn revoke_one_time_token(
            &self,
            input: RevokeOneTimeToken,
        ) -> Result<(), AuthFlowError> {
            let identifier = one_time_token_identifier(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.delete_verification(verification.id).await
        }
    }

    fn one_time_token_identifier(secret: &str, token: &str) -> String {
        format!("one-time-token:{}", hash_token(secret, token))
    }
}
pub mod multi_session {
    use auth_proto::{AuthFlowError, AuthSessionBundle};

    use crate::{
        ActiveDeviceSession, ArchitectAuth, AuthStorage, CurrentSession, DeviceSession,
        DeviceSessions, ListDeviceSessions, RevokeDeviceSession, RevokeDeviceSessionResult,
        SetActiveDeviceSession, SignOut,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.multisession.list]
        // r[impl auth.multisession.no-forged-sessions]
        pub async fn list_device_sessions(
            &self,
            input: ListDeviceSessions,
        ) -> Result<DeviceSessions, AuthFlowError> {
            let mut sessions = Vec::new();
            let mut seen_users = Vec::new();
            for token in input.session_tokens {
                let Ok(bundle) = self
                    .current_session(CurrentSession {
                        token: token.clone(),
                    })
                    .await
                else {
                    continue;
                };
                if seen_users.contains(&bundle.user.id) {
                    continue;
                }
                seen_users.push(bundle.user.id);
                sessions.push(DeviceSession {
                    user: bundle.user,
                    session: bundle.session,
                    token,
                    active: sessions.is_empty(),
                });
            }
            Ok(DeviceSessions { sessions })
        }

        // r[impl auth.multisession.set-active]
        // r[impl auth.multisession.permission-isolation]
        pub async fn set_active_device_session(
            &self,
            input: SetActiveDeviceSession,
        ) -> Result<ActiveDeviceSession, AuthFlowError> {
            if !input
                .session_tokens
                .iter()
                .any(|token| token == &input.session_token)
            {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let bundle = self
                .current_session(CurrentSession {
                    token: input.session_token.clone(),
                })
                .await?;
            Ok(ActiveDeviceSession {
                user: bundle.user,
                session: bundle.session,
                token: input.session_token,
            })
        }

        // r[impl auth.multisession.revoke]
        // r[impl auth.multisession.current-session]
        pub async fn revoke_device_session(
            &self,
            input: RevokeDeviceSession,
        ) -> Result<RevokeDeviceSessionResult, AuthFlowError> {
            if !input
                .session_tokens
                .iter()
                .any(|token| token == &input.session_token)
            {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let active_before = input
                .current_session_token
                .as_deref()
                .is_some_and(|token| token == input.session_token);
            self.sign_out(SignOut {
                token: input.session_token.clone(),
            })
            .await?;

            let next_active = if active_before {
                first_valid_session(self, input.session_tokens, Some(&input.session_token)).await?
            } else {
                None
            };
            Ok(RevokeDeviceSessionResult {
                revoked: true,
                next_active,
            })
        }
    }

    async fn first_valid_session<S>(
        auth: &ArchitectAuth<S>,
        session_tokens: Vec<String>,
        except: Option<&str>,
    ) -> Result<Option<ActiveDeviceSession>, AuthFlowError>
    where
        S: AuthStorage,
    {
        for token in session_tokens {
            if except.is_some_and(|except| except == token) {
                continue;
            }
            if let Ok(AuthSessionBundle {
                user,
                session,
                token,
            }) = auth.current_session(CurrentSession { token }).await
            {
                return Ok(Some(ActiveDeviceSession {
                    user,
                    session,
                    token,
                }));
            }
        }
        Ok(None)
    }
}
pub mod organizations {
    use chrono::Utc;

    use crate::{
        AcceptInvitation, AddTeamMember, ArchitectAuth, AuthStorage, AuthorizeOrganizationAction,
        CreateInvitation, CreateOrganization, CreateOrganizationRole, CreateTeam,
        DeleteOrganizationRole, DeleteTeam, InvitationToken, ListOrganizationRoles,
        ListTeamMembers, ListTeams, OrganizationBundle, RemoveTeamMember, RequireOrganizationRole,
        SetActiveOrganization, SetMemberRole, UpdateOrganizationRole, UpdateTeam,
        commands::CurrentSession,
        crypto::{generate_token, hash_token},
    };
    use auth_proto::{
        AuthFlowError, AuthInvitationCreate, AuthMember, AuthMemberCreate, AuthOrganizationCreate,
        AuthOrganizationRole, AuthOrganizationRoleCreate, AuthTeam, AuthTeamCreate, AuthTeamMember,
        AuthTeamMemberCreate,
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.org.slug-unique]
        // r[impl auth.org.create-owner]
        // r[impl auth.org.member-unique]
        pub async fn create_organization(
            &self,
            input: CreateOrganization,
        ) -> Result<OrganizationBundle, AuthFlowError> {
            validate_json(input.metadata_json.as_deref(), "metadata_json")?;
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let slug = normalize_slug(&input.slug)?;
            if self
                .storage
                .find_organization_by_slug(&slug)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "organization slug already exists".into(),
                ));
            }

            let (organization, membership) = self
                .storage
                .create_organization_with_owner(
                    AuthOrganizationCreate {
                        name: input.name,
                        slug,
                        logo: input.logo,
                        metadata_json: input.metadata_json,
                    },
                    session.user.id,
                )
                .await?;

            Ok(OrganizationBundle {
                organization,
                membership,
            })
        }

        // r[impl auth.org.active-session]
        pub async fn set_active_organization(
            &self,
            input: SetActiveOrganization,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token.clone(),
                })
                .await?;
            self.storage
                .find_member(input.organization_id, session.user.id)
                .await?
                .ok_or(AuthFlowError::PermissionDenied)?;
            let token_hash = hash_token(&self.config.secret, &input.session_token);
            self.storage
                .update_session_active_organization(&token_hash, Some(input.organization_id))
                .await
        }
        // r[impl auth.org.role-authoritative]
        // r[impl auth.org.rbac-deny-default]
        pub async fn require_organization_role(
            &self,
            input: RequireOrganizationRole,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let member = self
                .storage
                .find_member(input.organization_id, session.user.id)
                .await?
                .ok_or(AuthFlowError::PermissionDenied)?;
            if input.allowed_roles.iter().any(|role| role == &member.role) {
                Ok(())
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }

        // r[impl auth.org.permission-resources]
        // r[impl auth.org.default-permission-roles]
        // r[impl auth.org.composite-roles]
        // r[impl auth.org.dynamic-access-control]
        // r[impl auth.org.rbac-deny-default]
        pub async fn authorize_organization_action(
            &self,
            input: AuthorizeOrganizationAction,
        ) -> Result<(), AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                &input.resource,
                &input.action,
            )
            .await
            .map(|_| ())
        }

        // r[impl auth.org.remove-last-owner]
        pub async fn set_member_role(
            &self,
            input: SetMemberRole,
        ) -> Result<AuthMember, AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "member",
                "update",
            )
            .await?;
            let member = self
                .storage
                .find_member(input.organization_id, input.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if member.role == "owner" && input.role != "owner" {
                reject_if_last_owner(&self.storage, input.organization_id).await?;
            }
            self.storage
                .update_member_role(input.organization_id, input.user_id, input.role)
                .await
        }

        // r[impl auth.org.invite-token]
        pub async fn create_invitation(
            &self,
            input: CreateInvitation,
        ) -> Result<InvitationToken, AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "invitation",
                "create",
            )
            .await?;
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let canonical_email = normalize_email(&input.email)?;
            let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let invitation = self
                .storage
                .create_invitation(AuthInvitationCreate {
                    organization_id: input.organization_id,
                    email: canonical_email,
                    role: input.role,
                    status: auth_proto::InvitationStatus::Pending.as_str().into(),
                    inviter_id: session.user.id,
                    expires_at: input.expires_at,
                })
                .await?;
            self.storage
                .create_verification(auth_proto::AuthVerificationCreate {
                    identifier: invitation_identifier(invitation.id),
                    value_hash: hash_token(&self.config.secret, &token),
                    expires_at: invitation.expires_at,
                })
                .await?;
            Ok(InvitationToken { invitation, token })
        }

        // r[impl auth.org.invite-token]
        // r[impl auth.org.invite-status]
        // r[impl auth.org.member-unique]
        pub async fn accept_invitation(
            &self,
            input: AcceptInvitation,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let invitation = self
                .storage
                .find_invitation_by_id(input.invitation_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if invitation.status != auth_proto::InvitationStatus::Pending.as_str() {
                return Err(AuthFlowError::InvalidInput(
                    "invitation is not pending".into(),
                ));
            }
            if invitation.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let identifier = invitation_identifier(invitation.id);
            let value_hash = hash_token(&self.config.secret, &input.token);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            if self
                .storage
                .find_member(invitation.organization_id, session.user.id)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "organization member already exists".into(),
                ));
            }
            self.storage
                .accept_invitation_membership(
                    AuthMemberCreate {
                        organization_id: invitation.organization_id,
                        user_id: session.user.id,
                        role: invitation.role,
                    },
                    invitation.id,
                    auth_proto::InvitationStatus::Accepted.as_str().into(),
                    verification.id,
                )
                .await
        }

        // r[impl auth.org.dynamic-access-control]
        pub async fn create_organization_role(
            &self,
            input: CreateOrganizationRole,
        ) -> Result<AuthOrganizationRole, AuthFlowError> {
            validate_permissions_json(&input.permissions_json)?;
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "ac",
                "create",
            )
            .await?;
            self.storage
                .create_organization_role(AuthOrganizationRoleCreate {
                    organization_id: input.organization_id,
                    role: input.role,
                    permissions_json: input.permissions_json,
                })
                .await
        }

        // r[impl auth.org.dynamic-access-control]
        pub async fn update_organization_role(
            &self,
            input: UpdateOrganizationRole,
        ) -> Result<AuthOrganizationRole, AuthFlowError> {
            validate_permissions_json(&input.permissions_json)?;
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "ac",
                "update",
            )
            .await?;
            self.storage
                .update_organization_role(
                    input.organization_id,
                    &input.role,
                    input.permissions_json,
                )
                .await
        }

        // r[impl auth.org.dynamic-access-control]
        pub async fn delete_organization_role(
            &self,
            input: DeleteOrganizationRole,
        ) -> Result<(), AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "ac",
                "delete",
            )
            .await?;
            self.storage
                .delete_organization_role(input.organization_id, &input.role)
                .await
        }

        // r[impl auth.org.dynamic-access-control]
        pub async fn list_organization_roles(
            &self,
            input: ListOrganizationRoles,
        ) -> Result<Vec<AuthOrganizationRole>, AuthFlowError> {
            self.authorize_member_action(&input.session_token, input.organization_id, "ac", "read")
                .await?;
            self.storage
                .list_organization_roles(input.organization_id)
                .await
        }

        // r[impl auth.org.teams]
        pub async fn create_team(&self, input: CreateTeam) -> Result<AuthTeam, AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "team",
                "create",
            )
            .await?;
            self.storage
                .create_team(AuthTeamCreate {
                    organization_id: input.organization_id,
                    name: input.name,
                })
                .await
        }

        // r[impl auth.org.teams]
        pub async fn update_team(&self, input: UpdateTeam) -> Result<AuthTeam, AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "team",
                "update",
            )
            .await?;
            self.require_team_in_organization(input.team_id, input.organization_id)
                .await?;
            self.storage
                .update_team_name(input.team_id, input.name)
                .await
        }

        // r[impl auth.org.teams]
        pub async fn delete_team(&self, input: DeleteTeam) -> Result<(), AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "team",
                "delete",
            )
            .await?;
            self.require_team_in_organization(input.team_id, input.organization_id)
                .await?;
            self.storage.delete_team(input.team_id).await
        }

        // r[impl auth.org.teams]
        pub async fn list_teams(&self, input: ListTeams) -> Result<Vec<AuthTeam>, AuthFlowError> {
            self.require_member(&input.session_token, input.organization_id)
                .await?;
            self.storage
                .list_teams_by_organization(input.organization_id)
                .await
        }

        // r[impl auth.org.teams]
        pub async fn add_team_member(
            &self,
            input: AddTeamMember,
        ) -> Result<AuthTeamMember, AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "team",
                "update",
            )
            .await?;
            self.require_team_in_organization(input.team_id, input.organization_id)
                .await?;
            self.storage
                .find_member(input.organization_id, input.user_id)
                .await?
                .ok_or(AuthFlowError::PermissionDenied)?;
            if self
                .storage
                .find_team_member(input.team_id, input.user_id)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "team member already exists".into(),
                ));
            }
            self.storage
                .create_team_member(AuthTeamMemberCreate {
                    team_id: input.team_id,
                    user_id: input.user_id,
                })
                .await
        }

        // r[impl auth.org.teams]
        pub async fn remove_team_member(
            &self,
            input: RemoveTeamMember,
        ) -> Result<(), AuthFlowError> {
            self.authorize_member_action(
                &input.session_token,
                input.organization_id,
                "team",
                "update",
            )
            .await?;
            self.require_team_in_organization(input.team_id, input.organization_id)
                .await?;
            self.storage
                .delete_team_member(input.team_id, input.user_id)
                .await
        }

        // r[impl auth.org.teams]
        pub async fn list_team_members(
            &self,
            input: ListTeamMembers,
        ) -> Result<Vec<AuthTeamMember>, AuthFlowError> {
            self.require_member(&input.session_token, input.organization_id)
                .await?;
            self.require_team_in_organization(input.team_id, input.organization_id)
                .await?;
            self.storage.list_team_members(input.team_id).await
        }

        async fn require_member(
            &self,
            session_token: &str,
            organization_id: uuid::Uuid,
        ) -> Result<AuthMember, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: session_token.to_owned(),
                })
                .await?;
            self.storage
                .find_member(organization_id, session.user.id)
                .await?
                .ok_or(AuthFlowError::PermissionDenied)
        }

        async fn authorize_member_action(
            &self,
            session_token: &str,
            organization_id: uuid::Uuid,
            resource: &str,
            action: &str,
        ) -> Result<AuthMember, AuthFlowError> {
            let member = self.require_member(session_token, organization_id).await?;
            for role in member
                .role
                .split(',')
                .map(str::trim)
                .filter(|role| !role.is_empty())
            {
                if default_role_grants(role, resource, action)
                    || self
                        .dynamic_role_grants(organization_id, role, resource, action)
                        .await?
                {
                    return Ok(member);
                }
            }
            Err(AuthFlowError::PermissionDenied)
        }

        async fn dynamic_role_grants(
            &self,
            organization_id: uuid::Uuid,
            role: &str,
            resource: &str,
            action: &str,
        ) -> Result<bool, AuthFlowError> {
            let Some(role) = self
                .storage
                .find_organization_role(organization_id, role)
                .await?
            else {
                return Ok(false);
            };
            permissions_json_grants(&role.permissions_json, resource, action)
        }

        async fn require_team_in_organization(
            &self,
            team_id: uuid::Uuid,
            organization_id: uuid::Uuid,
        ) -> Result<AuthTeam, AuthFlowError> {
            let team = self
                .storage
                .find_team_by_id(team_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if team.organization_id == organization_id {
                Ok(team)
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }
    }

    fn normalize_slug(slug: &str) -> Result<String, AuthFlowError> {
        let slug = slug.trim().to_ascii_lowercase();
        let valid = !slug.is_empty()
            && slug
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if valid {
            Ok(slug)
        } else {
            Err(AuthFlowError::InvalidInput(
                "organization slug is invalid".into(),
            ))
        }
    }

    fn validate_json(input: Option<&str>, field: &str) -> Result<(), AuthFlowError> {
        if let Some(input) = input {
            serde_json::from_str::<serde_json::Value>(input)
                .map_err(|_| AuthFlowError::InvalidInput(format!("{field} must be JSON")))?;
        }
        Ok(())
    }

    fn validate_permissions_json(input: &str) -> Result<(), AuthFlowError> {
        permissions_json_grants(input, "ac", "read").map(|_| ())
    }

    fn permissions_json_grants(
        input: &str,
        resource: &str,
        action: &str,
    ) -> Result<bool, AuthFlowError> {
        let value = serde_json::from_str::<serde_json::Value>(input)
            .map_err(|_| AuthFlowError::InvalidInput("permissions_json must be JSON".into()))?;
        let object = value.as_object().ok_or_else(|| {
            AuthFlowError::InvalidInput("permissions_json must be a JSON object".into())
        })?;
        let Some(actions) = object.get(resource) else {
            return Ok(false);
        };
        let actions = actions.as_array().ok_or_else(|| {
            AuthFlowError::InvalidInput("permissions_json values must be arrays".into())
        })?;
        Ok(actions
            .iter()
            .any(|value| value.as_str().is_some_and(|candidate| candidate == action)))
    }

    fn default_role_grants(role: &str, resource: &str, action: &str) -> bool {
        match role {
            "owner" => matches!(
                (resource, action),
                ("organization", "update" | "delete")
                    | ("member", "create" | "update" | "delete")
                    | ("invitation", "create" | "cancel")
                    | ("team", "create" | "update" | "delete")
                    | ("ac", "create" | "read" | "update" | "delete")
            ),
            "admin" => matches!(
                (resource, action),
                ("organization", "update")
                    | ("member", "create" | "update" | "delete")
                    | ("invitation", "create" | "cancel")
                    | ("team", "create" | "update" | "delete")
                    | ("ac", "create" | "read" | "update" | "delete")
            ),
            "member" => matches!((resource, action), ("ac", "read")),
            _ => false,
        }
    }

    fn normalize_email(email: &str) -> Result<String, AuthFlowError> {
        let trimmed = email.trim();
        let Some((local, domain)) = trimmed.split_once('@') else {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        };
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return Err(AuthFlowError::InvalidInput("email is invalid".into()));
        }
        Ok(format!("{local}@{}", domain.to_ascii_lowercase()))
    }

    fn invitation_identifier(invitation_id: uuid::Uuid) -> String {
        format!("organization-invitation:{invitation_id}")
    }

    async fn reject_if_last_owner<S>(
        storage: &S,
        organization_id: uuid::Uuid,
    ) -> Result<(), AuthFlowError>
    where
        S: AuthStorage,
    {
        let owner_count = storage
            .list_members_by_organization(organization_id)
            .await?
            .into_iter()
            .filter(|member| member.role == "owner")
            .count();
        if owner_count <= 1 {
            Err(AuthFlowError::InvalidInput(
                "cannot demote the last organization owner".into(),
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    mod boundary_tests {
        use proptest::prelude::*;
        use serde_json::json;

        proptest! {
            // r[verify auth.boundary.property-tests]
            #[test]
            fn permissions_json_grants_only_matching_string_actions(
                resource in "[a-z][a-z0-9_-]{0,24}",
                action in "[a-z][a-z0-9_.:-]{0,24}",
                other_action in "[A-Z][A-Za-z0-9_.:-]{0,24}",
            ) {
                let grants = json!({ resource.clone(): [action.clone(), other_action] }).to_string();

                prop_assert!(
                    super::permissions_json_grants(&grants, &resource, &action)
                        .expect("valid permissions json grants")
                );
                prop_assert!(
                    !super::permissions_json_grants(&grants, &resource, "missing")
                        .expect("valid permissions json rejects missing action")
                );
                prop_assert!(
                    !super::permissions_json_grants(&grants, "missing", &action)
                        .expect("valid permissions json rejects missing resource")
                );
            }

            // r[verify auth.boundary.property-tests]
            #[test]
            fn permissions_json_rejects_non_object_or_non_array_values(input in "\\PC{0,128}") {
                let arbitrary_json_string = serde_json::to_string(&input).expect("json string");
                prop_assert!(super::permissions_json_grants(&arbitrary_json_string, "ac", "read").is_err());

                let object_with_string = json!({ "ac": input }).to_string();
                prop_assert!(super::permissions_json_grants(&object_with_string, "ac", "read").is_err());
            }
        }
    }
}
pub mod passkeys {
    use auth_proto::{AuthFlowError, AuthPasskey, AuthPasskeyCreate, AuthSessionBundle};
    use chrono::{Duration, Utc};

    use crate::{
        ArchitectAuth, AuthStorage, BeginPasskeyAuthentication, BeginPasskeyRegistration,
        CompletePasskeyAuthentication, CompletePasskeyRegistration, DeletePasskey, ListPasskeys,
        VerificationToken, commands::CurrentSession, crypto::generate_token, crypto::hash_token,
        flows::last_login_method::record_last_login_method,
    };

    const PASSKEY_CHALLENGE_TTL_SECONDS: i64 = 300;

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.passkey.challenge-random]
        // r[impl auth.passkey.challenge-expiry]
        // r[impl auth.passkey.user-match]
        pub async fn begin_passkey_registration(
            &self,
            input: BeginPasskeyRegistration,
        ) -> Result<VerificationToken, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let challenge =
                generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let identifier = registration_identifier(session.user.id);
            self.storage
                .create_verification(auth_proto::AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &challenge),
                    expires_at: Utc::now() + Duration::seconds(PASSKEY_CHALLENGE_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken {
                identifier,
                token: challenge,
            })
        }

        // r[impl auth.passkey.challenge-expiry]
        // r[impl auth.passkey.rp-origin]
        // r[impl auth.passkey.credential-unique]
        // r[impl auth.passkey.user-match]
        // r[impl auth.passkey.transports]
        pub async fn complete_passkey_registration(
            &self,
            input: CompletePasskeyRegistration,
        ) -> Result<AuthPasskey, AuthFlowError> {
            self.validate_passkey_relying_party(&input.rp_id, &input.origin)?;
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            if self
                .storage
                .find_passkey_by_credential_id(&input.credential_id)
                .await?
                .is_some()
            {
                return Err(AuthFlowError::InvalidInput(
                    "passkey credential already exists".into(),
                ));
            }
            let identifier = registration_identifier(session.user.id);
            let value_hash = hash_token(&self.config.secret, &input.challenge);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let passkey = self
                .storage
                .create_passkey(AuthPasskeyCreate {
                    name: input.name,
                    user_id: session.user.id,
                    public_key: input.public_key,
                    credential_id: input.credential_id,
                    counter: input.counter,
                    device_type: input.device_type,
                    backed_up: input.backed_up,
                    transports: input.transports,
                })
                .await?;
            self.storage.delete_verification(verification.id).await?;
            Ok(passkey)
        }

        // r[impl auth.passkey.challenge-random]
        // r[impl auth.passkey.challenge-expiry]
        pub async fn begin_passkey_authentication(
            &self,
            input: BeginPasskeyAuthentication,
        ) -> Result<VerificationToken, AuthFlowError> {
            self.storage
                .find_passkey_by_credential_id(&input.credential_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let challenge =
                generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let identifier = authentication_identifier(&input.credential_id);
            self.storage
                .create_verification(auth_proto::AuthVerificationCreate {
                    identifier: identifier.clone(),
                    value_hash: hash_token(&self.config.secret, &challenge),
                    expires_at: Utc::now() + Duration::seconds(PASSKEY_CHALLENGE_TTL_SECONDS),
                })
                .await?;
            Ok(VerificationToken {
                identifier,
                token: challenge,
            })
        }

        // r[impl auth.passkey.challenge-expiry]
        // r[impl auth.passkey.rp-origin]
        // r[impl auth.passkey.counter]
        pub async fn complete_passkey_authentication(
            &self,
            input: CompletePasskeyAuthentication,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            self.validate_passkey_relying_party(&input.rp_id, &input.origin)?;
            let passkey = self
                .storage
                .find_passkey_by_credential_id(&input.credential_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if input.counter <= passkey.counter {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let identifier = authentication_identifier(&input.credential_id);
            let value_hash = hash_token(&self.config.secret, &input.challenge);
            let verification = self
                .storage
                .find_verification(&identifier, &value_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let passkey = self
                .storage
                .update_passkey_counter(&input.credential_id, input.counter)
                .await?;
            let user = self
                .storage
                .find_user_by_id(passkey.user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.delete_verification(verification.id).await?;
            let bundle = self
                .issue_session(user, input.ip_address, input.user_agent, None, None)
                .await?;
            record_last_login_method(self, bundle, "passkey").await
        }

        pub async fn list_passkeys(
            &self,
            input: ListPasskeys,
        ) -> Result<Vec<AuthPasskey>, AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.storage.list_passkeys_by_user_id(session.user.id).await
        }

        // r[impl auth.passkey.delete-last-credential]
        pub async fn delete_passkey(&self, input: DeletePasskey) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let passkey = self
                .storage
                .find_passkey_by_credential_id(&input.credential_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if passkey.user_id != session.user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
            let has_password = self
                .storage
                .find_password_account_by_user_id(session.user.id)
                .await?
                .is_some();
            let oauth_count = self
                .storage
                .list_accounts_by_user_id(session.user.id)
                .await?
                .into_iter()
                .filter(|account| account.provider_id != "credential")
                .count();
            let passkey_count = self
                .storage
                .list_passkeys_by_user_id(session.user.id)
                .await?
                .len();
            if !has_password && oauth_count == 0 && passkey_count <= 1 {
                return Err(AuthFlowError::InvalidInput(
                    "cannot delete the last sign-in credential".into(),
                ));
            }
            self.storage
                .delete_passkey_by_credential_id(&input.credential_id)
                .await
        }

        fn validate_passkey_relying_party(
            &self,
            rp_id: &str,
            origin: &str,
        ) -> Result<(), AuthFlowError> {
            if rp_id != self.config.passkey_rp_id {
                return Err(AuthFlowError::PermissionDenied);
            }
            if self
                .config
                .passkey_allowed_origins
                .iter()
                .any(|allowed| allowed == origin)
            {
                Ok(())
            } else {
                Err(AuthFlowError::PermissionDenied)
            }
        }
    }

    fn registration_identifier(user_id: uuid::Uuid) -> String {
        format!("passkey-registration:{user_id}")
    }

    fn authentication_identifier(credential_id: &str) -> String {
        format!("passkey-authentication:{credential_id}")
    }
}
pub mod device_authorization {
    use auth_proto::{AuthFlowError, AuthSessionBundle, AuthVerificationCreate};
    use chrono::{Duration, Utc};

    use crate::{
        ApproveDeviceCode, ArchitectAuth, AuthStorage, CreateDeviceAuthorization, DenyDeviceCode,
        DeviceAuthorization, DeviceCodeVerification, PollDeviceToken, VerifyDeviceCode,
        commands::CurrentSession,
        crypto::{generate_token, hash_token},
    };

    const DEFAULT_DEVICE_CODE_TTL_SECONDS: i64 = 600;
    const DEFAULT_DEVICE_CODE_INTERVAL_SECONDS: i64 = 5;
    const DEVICE_VERIFICATION_URI: &str = "/auth/device";

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.device.create]
        pub async fn create_device_authorization(
            &self,
            input: CreateDeviceAuthorization,
        ) -> Result<DeviceAuthorization, AuthFlowError> {
            if input.client_id.trim().is_empty() {
                return Err(AuthFlowError::InvalidInput("client_id is required".into()));
            }
            let expires_in_seconds = input
                .expires_in_seconds
                .unwrap_or(DEFAULT_DEVICE_CODE_TTL_SECONDS)
                .clamp(60, 1800);
            let interval_seconds = input
                .interval_seconds
                .unwrap_or(DEFAULT_DEVICE_CODE_INTERVAL_SECONDS)
                .clamp(1, 60);
            let device_code =
                generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
            let user_code = generate_user_code()?;
            let device_hash = hash_token(&self.config.secret, &device_code);
            let expires_at = Utc::now() + Duration::seconds(expires_in_seconds);
            let value = encode_device_value(
                &device_hash,
                &input.client_id,
                input.scope.as_deref(),
                interval_seconds,
            );

            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: device_identifier(&device_hash),
                    value_hash: value.clone(),
                    expires_at,
                })
                .await?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: device_user_identifier(&user_code),
                    value_hash: value,
                    expires_at,
                })
                .await?;

            Ok(DeviceAuthorization {
                verification_uri_complete: format!(
                    "{DEVICE_VERIFICATION_URI}?user_code={user_code}"
                ),
                verification_uri: DEVICE_VERIFICATION_URI.into(),
                device_code,
                user_code,
                expires_in_seconds,
                interval_seconds,
            })
        }

        // r[impl auth.device.verify]
        pub async fn verify_device_code(
            &self,
            input: VerifyDeviceCode,
        ) -> Result<DeviceCodeVerification, AuthFlowError> {
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&device_user_identifier(&input.user_code))
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state = decode_device_value(&verification.value_hash)?;
            Ok(DeviceCodeVerification {
                user_code: input.user_code,
                client_id: state.client_id,
                scope: state.scope,
            })
        }

        // r[impl auth.device.approve-deny]
        pub async fn approve_device_code(
            &self,
            input: ApproveDeviceCode,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&device_user_identifier(&input.user_code))
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state = decode_device_value(&verification.value_hash)?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: device_status_identifier(&state.device_hash),
                    value_hash: format!("approved:{}", session.user.id),
                    expires_at: verification.expires_at,
                })
                .await
                .map(|_| ())
        }

        // r[impl auth.device.approve-deny]
        pub async fn deny_device_code(&self, input: DenyDeviceCode) -> Result<(), AuthFlowError> {
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&device_user_identifier(&input.user_code))
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state = decode_device_value(&verification.value_hash)?;
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier: device_status_identifier(&state.device_hash),
                    value_hash: "denied".into(),
                    expires_at: verification.expires_at,
                })
                .await
                .map(|_| ())
        }

        // r[impl auth.device.polling]
        // r[impl auth.device.expiry]
        pub async fn poll_device_token(
            &self,
            input: PollDeviceToken,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            let device_hash = hash_token(&self.config.secret, &input.device_code);
            let verification = self
                .storage
                .find_latest_verification_by_identifier(&device_identifier(&device_hash))
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if verification.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            let state = decode_device_value(&verification.value_hash)?;
            self.enforce_device_poll_interval(&state).await?;

            let status = self
                .storage
                .find_latest_verification_by_identifier(&device_status_identifier(&device_hash))
                .await?;
            let Some(status) = status else {
                return Err(AuthFlowError::VerificationRequired);
            };
            if status.expires_at <= Utc::now() {
                return Err(AuthFlowError::InvalidCredentials);
            }
            if status.value_hash == "denied" {
                return Err(AuthFlowError::PermissionDenied);
            }
            let Some(user_id) = status.value_hash.strip_prefix("approved:") else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let user_id = user_id
                .parse()
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let user = self
                .storage
                .find_user_by_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            self.storage.delete_verification(status.id).await?;
            self.storage.delete_verification(verification.id).await?;
            self.issue_session(user, input.ip_address, input.user_agent, None, None)
                .await
        }

        async fn enforce_device_poll_interval(
            &self,
            state: &DeviceGrantState,
        ) -> Result<(), AuthFlowError> {
            let identifier = device_poll_identifier(&state.device_hash);
            if let Some(last_poll) = self
                .storage
                .find_latest_verification_by_identifier(&identifier)
                .await?
                && last_poll.created_at + Duration::seconds(state.interval_seconds) > Utc::now()
            {
                return Err(AuthFlowError::InvalidInput("slow_down".into()));
            }
            self.storage
                .create_verification(AuthVerificationCreate {
                    identifier,
                    value_hash: "poll".into(),
                    expires_at: Utc::now() + Duration::seconds(state.interval_seconds),
                })
                .await
                .map(|_| ())
        }
    }

    struct DeviceGrantState {
        device_hash: String,
        client_id: String,
        scope: Option<String>,
        interval_seconds: i64,
    }

    fn generate_user_code() -> Result<String, AuthFlowError> {
        let token = generate_token().map_err(|err| AuthFlowError::Internal(err.to_string()))?;
        Ok(token
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>()
            .to_ascii_uppercase())
    }

    fn encode_device_value(
        device_hash: &str,
        client_id: &str,
        scope: Option<&str>,
        interval_seconds: i64,
    ) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            device_hash,
            client_id,
            scope.unwrap_or_default(),
            interval_seconds
        )
    }

    fn decode_device_value(value: &str) -> Result<DeviceGrantState, AuthFlowError> {
        let mut parts = value.splitn(4, '\n');
        let device_hash = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or(AuthFlowError::InvalidCredentials)?
            .to_owned();
        let client_id = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or(AuthFlowError::InvalidCredentials)?
            .to_owned();
        let scope = parts
            .next()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let interval_seconds = parts
            .next()
            .ok_or(AuthFlowError::InvalidCredentials)?
            .parse()
            .map_err(|_| AuthFlowError::InvalidCredentials)?;
        Ok(DeviceGrantState {
            device_hash,
            client_id,
            scope,
            interval_seconds,
        })
    }

    fn device_identifier(device_hash: &str) -> String {
        format!("device-authorization:{device_hash}")
    }

    fn device_user_identifier(user_code: &str) -> String {
        format!(
            "device-authorization-user:{}",
            user_code.trim().to_ascii_uppercase()
        )
    }

    fn device_status_identifier(device_hash: &str) -> String {
        format!("device-authorization-status:{device_hash}")
    }

    fn device_poll_identifier(device_hash: &str) -> String {
        format!("device-authorization-poll:{device_hash}")
    }

    #[cfg(test)]
    mod boundary_tests {
        use proptest::prelude::*;

        proptest! {
            // r[verify auth.boundary.property-tests]
            #[test]
            fn device_authorization_value_round_trips_without_newlines(
                device_hash in "[A-Za-z0-9_-]{1,64}",
                client_id in "[A-Za-z0-9_.:-]{1,64}",
                scope in proptest::option::of("[A-Za-z0-9 _:.-]{1,64}"),
                interval_seconds in 1_i64..3600,
            ) {
                let encoded = super::encode_device_value(
                    &device_hash,
                    &client_id,
                    scope.as_deref(),
                    interval_seconds,
                );
                let decoded = super::decode_device_value(&encoded)
                    .expect("encoded device grant value decodes");

                prop_assert_eq!(decoded.device_hash, device_hash);
                prop_assert_eq!(decoded.client_id, client_id);
                prop_assert_eq!(decoded.scope, scope);
                prop_assert_eq!(decoded.interval_seconds, interval_seconds);
            }

            // r[verify auth.boundary.fixtures]
            #[test]
            fn device_authorization_value_rejects_minimized_malformed_fixtures(
                prefix in "[A-Za-z0-9_-]{0,16}",
            ) {
                for malformed in [
                    "",
                    "hash",
                    "hash\nclient",
                    "hash\nclient\nscope\nnot-a-number",
                    "\nclient\nscope\n1",
                    "hash\n\nscope\n1",
                    prefix.as_str(),
                ] {
                    prop_assert!(super::decode_device_value(malformed).is_err());
                }
            }
        }
    }
}

pub mod passwords {}
pub mod sessions {}
pub mod two_factor {
    use auth_proto::{AuthFlowError, AuthTwoFactorCreate};
    use totp_rs::{Algorithm, Secret, TOTP};

    use crate::{
        ArchitectAuth, AuthStorage, ConfirmTwoFactor, DisableTwoFactor, StartTwoFactorSetup,
        VerifyTwoFactor,
        commands::CurrentSession,
        crypto::{decrypt_secret, encrypt_secret, hash_token},
    };

    impl<S> ArchitectAuth<S>
    where
        S: AuthStorage,
    {
        // r[impl auth.twofactor.enable-requires-session]
        // r[impl auth.twofactor.secret-encryption]
        // r[impl auth.twofactor.backup-codes-hash]
        // r[impl auth.twofactor.confirm-before-enabled]
        pub async fn start_two_factor_setup(
            &self,
            input: StartTwoFactorSetup,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            let backup_codes_hash = if input.backup_codes.is_empty() {
                None
            } else {
                Some(
                    input
                        .backup_codes
                        .iter()
                        .map(|code| hash_token(&self.config.secret, code))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            };
            self.storage
                .create_two_factor(AuthTwoFactorCreate {
                    user_id: session.user.id,
                    secret_ciphertext: encrypt_secret(
                        &self.config.secret,
                        &input.secret_ciphertext,
                    )
                    .map_err(|err| AuthFlowError::Internal(err.to_string()))?,
                    backup_codes_hash,
                    attempt_count: 0,
                })
                .await?;
            self.storage
                .set_user_two_factor_enabled(session.user.id, false)
                .await
        }

        // r[impl auth.twofactor.confirm-before-enabled]
        pub async fn confirm_two_factor(
            &self,
            input: ConfirmTwoFactor,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.verify_totp_for_user(session.user.id, &input.code)
                .await?;
            self.storage
                .set_user_two_factor_enabled(session.user.id, true)
                .await
        }

        // r[impl auth.twofactor.signin-required]
        // r[impl auth.twofactor.backup-codes-single-use]
        // r[impl auth.twofactor.rate-limit]
        pub async fn verify_two_factor(&self, input: VerifyTwoFactor) -> Result<(), AuthFlowError> {
            let token_hash = hash_token(&self.config.secret, &input.session_token);
            let session = self
                .storage
                .find_session_by_token_hash(&token_hash)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            if session.expires_at <= chrono::Utc::now() {
                return Err(AuthFlowError::SessionExpired);
            }
            self.verify_second_factor_for_user(session.user_id, &input.code)
                .await?;
            self.storage
                .activate_session_by_token_hash(&token_hash)
                .await
        }

        // r[impl auth.twofactor.disable-requires-proof]
        // r[impl auth.twofactor.backup-codes-single-use]
        // r[impl auth.twofactor.rate-limit]
        pub async fn disable_two_factor(
            &self,
            input: DisableTwoFactor,
        ) -> Result<(), AuthFlowError> {
            let session = self
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.verify_second_factor_for_user(session.user.id, &input.code)
                .await?;
            self.storage.delete_two_factor(session.user.id).await?;
            self.storage
                .set_user_two_factor_enabled(session.user.id, false)
                .await
        }

        async fn verify_totp_for_user(
            &self,
            user_id: uuid::Uuid,
            code: &str,
        ) -> Result<(), AuthFlowError> {
            let two_factor = self
                .storage
                .find_two_factor_by_user_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let secret = decrypt_secret(&self.config.secret, &two_factor.secret_ciphertext)
                .map_err(|_| AuthFlowError::InvalidCredentials)?;
            let totp = totp_from_encoded_secret(&secret)?;
            if totp
                .check_current(code)
                .map_err(|err| AuthFlowError::Internal(err.to_string()))?
            {
                Ok(())
            } else {
                Err(AuthFlowError::InvalidCredentials)
            }
        }

        async fn verify_second_factor_for_user(
            &self,
            user_id: uuid::Uuid,
            code: &str,
        ) -> Result<(), AuthFlowError> {
            let attempts = self.storage.increment_two_factor_attempts(user_id).await?;
            if attempts > 5 {
                return Err(AuthFlowError::PermissionDenied);
            }
            if self.verify_totp_for_user(user_id, code).await.is_ok() {
                self.storage.reset_two_factor_attempts(user_id).await?;
                return Ok(());
            }
            match self.verify_and_consume_backup_code(user_id, code).await {
                Ok(()) => {
                    self.storage.reset_two_factor_attempts(user_id).await?;
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }

        async fn verify_and_consume_backup_code(
            &self,
            user_id: uuid::Uuid,
            code: &str,
        ) -> Result<(), AuthFlowError> {
            let two_factor = self
                .storage
                .find_two_factor_by_user_id(user_id)
                .await?
                .ok_or(AuthFlowError::InvalidCredentials)?;
            let Some(backup_codes_hash) = two_factor.backup_codes_hash else {
                return Err(AuthFlowError::InvalidCredentials);
            };
            let code_hash = hash_token(&self.config.secret, code);
            let mut remaining = Vec::new();
            let mut matched = false;
            for stored_hash in backup_codes_hash.lines().filter(|hash| !hash.is_empty()) {
                if stored_hash == code_hash && !matched {
                    matched = true;
                } else {
                    remaining.push(stored_hash.to_owned());
                }
            }
            if !matched {
                return Err(AuthFlowError::InvalidCredentials);
            }
            self.storage
                .update_two_factor_backup_codes(
                    user_id,
                    (!remaining.is_empty()).then(|| remaining.join("\n")),
                )
                .await
        }
    }

    fn totp_from_encoded_secret(secret: &str) -> Result<TOTP, AuthFlowError> {
        TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            Secret::Encoded(secret.to_owned())
                .to_bytes()
                .map_err(|err| AuthFlowError::InvalidInput(err.to_string()))?,
            None,
            "architect-auth".into(),
        )
        .map_err(|err| AuthFlowError::InvalidInput(err.to_string()))
    }
}
