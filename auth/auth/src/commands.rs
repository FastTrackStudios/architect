//! Transport-agnostic auth commands.

use crate::config::CaptchaFlow;
use auth_proto::AuthSessionBundle;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{future::Future, pin::Pin};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateEmailPasswordUser {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub image: Option<String>,
    pub metadata_json: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInAnonymous {
    pub metadata_json: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInUsername {
    pub username: String,
    pub password: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateUsername {
    pub session_token: String,
    pub username: String,
    pub display_username: Option<String>,
}

/// Set the session owner's display name / avatar. `None` leaves a
/// field untouched; `Some("")` clears it — a distinction federated
/// callers depend on, since "not mentioned" and "deliberately cleared"
/// must not become the same write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateProfile {
    pub session_token: String,
    pub name: Option<String>,
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkAnonymousEmailPassword {
    pub session_token: String,
    pub email: String,
    pub password: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupAnonymousUsers {
    pub session_token: String,
    pub older_than_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupAnonymousUsersResult {
    pub deleted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSession {
    pub user_id: Uuid,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub impersonated_by: Option<Uuid>,
    pub active_organization_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentSession {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomSessionBundle<T> {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
    pub custom: T,
}

impl<T> CustomSessionBundle<T> {
    pub fn from_session_bundle(bundle: AuthSessionBundle, custom: T) -> Self {
        Self {
            user: bundle.user,
            session: bundle.session,
            token: bundle.token,
            custom,
        }
    }
}

pub trait CustomSessionEnricher: Send + Sync {
    type Output: Clone + Send + Sync + 'static;

    fn enrich_session<'a>(
        &'a self,
        bundle: &'a AuthSessionBundle,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, auth_proto::AuthFlowError>> + Send + 'a>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignOut {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshSession {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSessions {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeSession {
    pub session_token: String,
    pub session_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeOtherSessions {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangePassword {
    pub session_token: String,
    pub current_password: String,
    pub new_password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPasswordReset {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletePasswordReset {
    pub email: String,
    pub token: String,
    pub new_password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestEmailVerification {
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyEmail {
    pub user_id: Uuid,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendEmailOtp {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyEmailOtp {
    pub email: String,
    pub otp: String,
    pub create_session: bool,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendPhoneNumberOtp {
    pub phone_number: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyPhoneNumberOtp {
    pub phone_number: String,
    pub otp: String,
    pub create_session: bool,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePhoneNumber {
    pub session_token: String,
    pub phone_number: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhoneNumberVerification {
    pub user: auth_proto::AuthUser,
    pub session: Option<auth_proto::AuthSession>,
    pub token: Option<String>,
    pub phone_number: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSiweNonce;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifySiweMessage {
    pub message: String,
    pub signature: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckPasswordBreach {
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordBreachCheck {
    pub breached: bool,
    pub count: u64,
    pub prefix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdditionalFieldType {
    String,
    Number,
    Boolean,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdditionalFieldSpec {
    pub name: &'static str,
    pub field_type: AdditionalFieldType,
    pub required: bool,
    pub input: bool,
    pub returned: bool,
    pub default_json: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdditionalFieldsConfig {
    pub user: Vec<AdditionalFieldSpec>,
    pub session: Vec<AdditionalFieldSpec>,
    pub account: Vec<AdditionalFieldSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdditionalFieldsView {
    pub fields: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdditionalFieldsSchema {
    pub user: Vec<AdditionalFieldSpec>,
    pub session: Vec<AdditionalFieldSpec>,
    pub account: Vec<AdditionalFieldSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizeMcpRequest {
    pub session_token: Option<String>,
    pub authorization_header: Option<String>,
    pub organization_id: Option<Uuid>,
    pub resource: Option<String>,
    pub action: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAuthorization {
    pub allowed: bool,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSiweAddress {
    pub session_token: String,
    pub message: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmailOtpVerification {
    pub user: auth_proto::AuthUser,
    pub session: Option<auth_proto::AuthSession>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMagicLink {
    pub email: String,
    pub callback_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagicLinkToken {
    pub identifier: String,
    pub token: String,
    pub url: String,
    pub callback_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyMagicLink {
    pub email: String,
    pub token: String,
    pub callback_url: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicLinkVerification {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
    pub redirect_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueJwt {
    pub session_token: String,
    pub audience: Option<String>,
    pub expires_in_seconds: Option<i64>,
    pub claims_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyJwt {
    pub token: String,
    pub audience: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtToken {
    pub token: String,
    pub key_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtVerification {
    pub claims: JwtClaims,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JwtClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub sid: String,
    pub iat: i64,
    pub exp: i64,
    pub extra: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtKeySet {
    pub keys: Vec<JwtKeyDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtKeyDescriptor {
    pub kid: String,
    pub alg: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
    pub registration_endpoint: String,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub claims_supported: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterOidcClient {
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
    pub scope: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcClientRegistration {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub scope: String,
    pub token_endpoint_auth_method: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizeOidc {
    pub session_token: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcAuthorization {
    pub redirect_uri: String,
    pub code: String,
    pub state: Option<String>,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeOidcToken {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetOidcUserInfo {
    pub access_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcUserInfo {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeEmail {
    pub session_token: String,
    pub new_email: String,
}

/// Move an account onto a different address, keeping its user id.
///
/// Identified by `user_id`, not a session: the usual reason to migrate an
/// address is that the person cannot sign in with the old one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateUserEmail {
    pub user_id: uuid::Uuid,
    pub new_email: String,
    /// The operator performing it. `None` records the change as
    /// self-service.
    pub changed_by: Option<uuid::Uuid>,
    /// Free text for the trail — worth filling in for bulk migrations, so
    /// the record explains itself months later.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteUser {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationToken {
    pub identifier: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListAccounts {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOAuthAccount {
    pub session_token: String,
    pub provider_id: String,
    pub account_id: String,
    pub access_token_ciphertext: Option<String>,
    pub refresh_token_ciphertext: Option<String>,
    pub id_token_ciphertext: Option<String>,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlinkOAuthAccount {
    pub session_token: String,
    pub provider_id: String,
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginOAuthAuthorization {
    pub provider_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyOAuthState {
    pub provider_id: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInOAuthAccount {
    pub provider_id: String,
    pub account_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub image: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetOAuthAccessToken {
    pub session_token: String,
    pub provider_id: String,
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshOAuthToken {
    pub session_token: String,
    pub provider_id: String,
    pub account_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub refresh_token_expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthAccessToken {
    pub access_token: Option<String>,
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProviderDescriptor {
    pub id: &'static str,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub user_info_url: &'static str,
    pub scopes: &'static [&'static str],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginOAuthProxyAuthorization {
    pub provider_id: String,
    pub callback_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProxyAuthorization {
    pub provider_id: String,
    pub state: String,
    pub production_callback_url: String,
    pub proxy_callback_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardOAuthProxyCallback {
    pub provider_id: String,
    pub state: String,
    pub callback_url: String,
    pub profile_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProxyForwarding {
    pub redirect_url: String,
    pub encrypted_profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsumeOAuthProxyCallback {
    pub callback_url: String,
    pub encrypted_profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProxyProfile {
    pub provider_id: String,
    pub state: String,
    pub callback_url: String,
    pub profile_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProxyMetadata {
    pub current_url: String,
    pub production_url: String,
    pub proxy_callback_url: String,
    pub should_proxy: bool,
    pub providers: Vec<OAuthProviderDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneTapCallback {
    pub id_token: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OneTapVerification {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateOneTimeToken {
    pub session_token: String,
    pub expires_in_seconds: Option<i64>,
    pub scope: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneTimeToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub scope: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyOneTimeToken {
    pub token: String,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OneTimeTokenVerification {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
    pub scope: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeOneTimeToken {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListDeviceSessions {
    pub session_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceSession {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceSessions {
    pub sessions: Vec<DeviceSession>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetActiveDeviceSession {
    pub current_session_token: Option<String>,
    pub session_token: String,
    pub session_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveDeviceSession {
    pub user: auth_proto::AuthUser,
    pub session: auth_proto::AuthSession,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeDeviceSession {
    pub current_session_token: Option<String>,
    pub session_token: String,
    pub session_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RevokeDeviceSessionResult {
    pub revoked: bool,
    pub next_active: Option<ActiveDeviceSession>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetLastLoginMethod {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearLastLoginMethod {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastLoginMethod {
    pub method: Option<String>,
    pub cookie_name: String,
    pub max_age_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastLoginMethodCookieConfig {
    pub name: String,
    pub max_age_seconds: i64,
    pub http_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateApiKey {
    pub session_token: String,
    pub name: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions_json: Option<String>,
    pub rate_limit_time_window: Option<i64>,
    pub rate_limit_max: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListApiKeys {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetApiKey {
    pub session_token: String,
    pub api_key_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateApiKey {
    pub session_token: String,
    pub api_key_id: Uuid,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions_json: Option<String>,
    pub rate_limit_time_window: Option<i64>,
    pub rate_limit_max: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteApiKey {
    pub session_token: String,
    pub api_key_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticateApiKey {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyApiKey {
    pub key: String,
    pub permission: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizeApiKey {
    pub key: String,
    pub permission: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeApiKey {
    pub session_token: String,
    pub api_key_id: Uuid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApiKeyBundle {
    pub api_key: auth_proto::AuthApiKey,
    pub user: auth_proto::AuthUser,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticateBearerToken {
    pub authorization_header: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BearerTokenBundle {
    pub user: auth_proto::AuthUser,
    pub token: String,
    pub strategy: BearerTokenStrategy,
    pub session: Option<auth_proto::AuthSession>,
    pub api_key: Option<auth_proto::AuthApiKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BearerTokenStrategy {
    Session,
    ApiKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyCaptcha {
    pub flow: CaptchaFlow,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptchaVerification {
    pub flow: CaptchaFlow,
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOrganization {
    pub session_token: String,
    pub name: String,
    pub slug: String,
    pub logo: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrganizationBundle {
    pub organization: auth_proto::AuthOrganization,
    pub membership: auth_proto::AuthMember,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetActiveOrganization {
    pub session_token: String,
    pub organization_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequireOrganizationRole {
    pub session_token: String,
    pub organization_id: Uuid,
    pub allowed_roles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizeOrganizationAction {
    pub session_token: String,
    pub organization_id: Uuid,
    pub resource: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOrganizationRole {
    pub session_token: String,
    pub organization_id: Uuid,
    pub role: String,
    pub permissions_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateOrganizationRole {
    pub session_token: String,
    pub organization_id: Uuid,
    pub role: String,
    pub permissions_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteOrganizationRole {
    pub session_token: String,
    pub organization_id: Uuid,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListOrganizationRoles {
    pub session_token: String,
    pub organization_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetMemberRole {
    pub session_token: String,
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateInvitation {
    pub session_token: String,
    pub organization_id: Uuid,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InvitationToken {
    pub invitation: auth_proto::AuthInvitation,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateTeam {
    pub session_token: String,
    pub organization_id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateTeam {
    pub session_token: String,
    pub organization_id: Uuid,
    pub team_id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTeam {
    pub session_token: String,
    pub organization_id: Uuid,
    pub team_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListTeams {
    pub session_token: String,
    pub organization_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddTeamMember {
    pub session_token: String,
    pub organization_id: Uuid,
    pub team_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveTeamMember {
    pub session_token: String,
    pub organization_id: Uuid,
    pub team_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListTeamMembers {
    pub session_token: String,
    pub organization_id: Uuid,
    pub team_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPasskeyRegistration {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletePasskeyRegistration {
    pub session_token: String,
    pub challenge: String,
    pub rp_id: String,
    pub origin: String,
    pub name: String,
    pub credential_id: String,
    pub public_key: String,
    pub counter: i64,
    pub device_type: String,
    pub backed_up: bool,
    pub transports: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPasskeyAuthentication {
    pub credential_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletePasskeyAuthentication {
    pub credential_id: String,
    pub challenge: String,
    pub rp_id: String,
    pub origin: String,
    pub counter: i64,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPasskeys {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletePasskey {
    pub session_token: String,
    pub credential_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateDeviceAuthorization {
    pub client_id: String,
    pub scope: Option<String>,
    pub expires_in_seconds: Option<i64>,
    pub interval_seconds: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in_seconds: i64,
    pub interval_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyDeviceCode {
    pub user_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCodeVerification {
    pub user_code: String,
    pub client_id: String,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApproveDeviceCode {
    pub session_token: String,
    pub user_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DenyDeviceCode {
    pub user_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollDeviceToken {
    pub device_code: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptInvitation {
    pub session_token: String,
    pub invitation_id: Uuid,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartTwoFactorSetup {
    pub session_token: String,
    pub secret_ciphertext: String,
    pub backup_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmTwoFactor {
    pub session_token: String,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyTwoFactor {
    pub session_token: String,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisableTwoFactor {
    pub session_token: String,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListUsers {
    pub session_token: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListUsersResult {
    pub users: Vec<auth_proto::AuthUser>,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminCreateUser {
    pub session_token: String,
    pub email: String,
    pub password: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetUserRole {
    pub session_token: String,
    pub user_id: Uuid,
    pub role: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminSetUserPassword {
    pub session_token: String,
    pub user_id: Uuid,
    pub new_password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BanUser {
    pub session_token: String,
    pub user_id: Uuid,
    pub reason: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnbanUser {
    pub session_token: String,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeUserSessions {
    pub session_token: String,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListUserSessions {
    pub session_token: String,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeUserSession {
    pub session_token: String,
    pub user_id: Uuid,
    pub session_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpersonateUser {
    pub session_token: String,
    pub user_id: Uuid,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopImpersonating {
    pub session_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveUser {
    pub session_token: String,
    pub user_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminHasPermission {
    pub session_token: String,
    pub organization_id: Uuid,
    pub resource: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HasPermissionResult {
    pub allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthAuditEvent {
    pub actor_id: Uuid,
    pub target_id: Option<Uuid>,
    pub action: String,
    pub created_at: DateTime<Utc>,
}
