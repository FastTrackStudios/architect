//! Runtime facade for architect-auth.

pub use auth_proto::*;

// r[impl auth.core.feature-flags]
// r[verify auth.core.feature-flags]
#[cfg(feature = "backend-db")]
pub mod backend_db {
    pub use auth_db::*;

    mod storage;

    pub use storage::AuthSeaOrmStorage;
}

pub mod commands;
pub mod config;
pub mod crypto;
pub mod flows;
pub mod identity;
pub mod plugins;
pub mod storage;
pub mod test_utils;
pub mod transport;

pub use commands::{
    AcceptInvitation, ActiveDeviceSession, AddTeamMember, AdditionalFieldSpec, AdditionalFieldType,
    AdditionalFieldsConfig, AdditionalFieldsSchema, AdditionalFieldsView, AdminCreateUser,
    AdminHasPermission, AdminSetUserPassword, ApiKeyBundle, ApproveDeviceCode, AuthAuditEvent,
    AuthenticateApiKey, AuthenticateBearerToken, AuthorizeApiKey, AuthorizeMcpRequest,
    AuthorizeOidc, AuthorizeOrganizationAction, BanUser, BearerTokenBundle, BearerTokenStrategy,
    BeginOAuthAuthorization, BeginOAuthProxyAuthorization, BeginPasskeyAuthentication,
    BeginPasskeyRegistration, CaptchaVerification, ChangeEmail, ChangePassword, MigrateUserEmail,
    CheckPasswordBreach, CleanupAnonymousUsers, CleanupAnonymousUsersResult, ClearLastLoginMethod,
    CompletePasskeyAuthentication, CompletePasskeyRegistration, CompletePasswordReset,
    ConfirmTwoFactor, ConsumeOAuthProxyCallback, CreateApiKey, CreateDeviceAuthorization,
    CreateEmailPasswordUser, CreateInvitation, CreateOrganization, CreateOrganizationRole,
    CreateSession, CreateSiweNonce, CreateTeam, CurrentSession, CustomSessionBundle,
    CustomSessionEnricher, DeleteApiKey, DeleteOrganizationRole, DeletePasskey, DeleteTeam,
    DeleteUser, DenyDeviceCode, DeviceAuthorization, DeviceCodeVerification, DeviceSession,
    DeviceSessions, DisableTwoFactor, EmailOtpVerification, ExchangeOidcToken,
    ForwardOAuthProxyCallback, GenerateOneTimeToken, GetApiKey, GetLastLoginMethod,
    GetOAuthAccessToken, GetOidcUserInfo, HasPermissionResult, ImpersonateUser, InvitationToken,
    IssueJwt, JwtClaims, JwtKeyDescriptor, JwtKeySet, JwtToken, JwtVerification, LastLoginMethod,
    LastLoginMethodCookieConfig, LinkAnonymousEmailPassword, LinkOAuthAccount, LinkSiweAddress,
    ListAccounts, ListApiKeys, ListDeviceSessions, ListOrganizationRoles, ListPasskeys,
    ListSessions, ListTeamMembers, ListTeams, ListUserSessions, ListUsers, ListUsersResult,
    MagicLinkToken, MagicLinkVerification, McpAuthorization, OAuthAccessToken,
    OAuthProviderDescriptor, OAuthProxyAuthorization, OAuthProxyForwarding, OAuthProxyMetadata,
    OAuthProxyProfile, OidcAuthorization, OidcClientRegistration, OidcDiscovery, OidcTokenResponse,
    OidcUserInfo, OneTapCallback, OneTapVerification, OneTimeToken, OneTimeTokenVerification,
    OrganizationBundle, PasswordBreachCheck, PhoneNumberVerification, PollDeviceToken,
    RefreshOAuthToken, RefreshSession, RegisterOidcClient, RemoveTeamMember, RemoveUser,
    RequestEmailVerification, RequestPasswordReset, RequireOrganizationRole, RevokeApiKey,
    RevokeDeviceSession, RevokeDeviceSessionResult, RevokeOneTimeToken, RevokeOtherSessions,
    RevokeSession, RevokeUserSession, RevokeUserSessions, SendEmailOtp, SendMagicLink,
    SendPhoneNumberOtp, SetActiveDeviceSession, SetActiveOrganization, SetMemberRole, SetUserRole,
    SignInAnonymous, SignInOAuthAccount, SignInUsername, SignOut, StartTwoFactorSetup,
    StopImpersonating, UnbanUser, UnlinkOAuthAccount, UpdateApiKey, UpdateOrganizationRole,
    UpdatePhoneNumber, UpdateProfile, UpdateTeam, UpdateUsername, VerificationToken, VerifyApiKey,
    VerifyCaptcha,
    VerifyDeviceCode, VerifyEmail, VerifyEmailOtp, VerifyJwt, VerifyMagicLink, VerifyOAuthState,
    VerifyOneTimeToken, VerifyPhoneNumberOtp, VerifySiweMessage, VerifyTwoFactor,
};
pub use config::{
    ArchitectAuthBuilder, ArchitectAuthConfig, BreachedPasswordConfig,
    BreachedPasswordFailurePolicy, BreachedPasswordProvider, CaptchaConfig, CaptchaFlow,
    CaptchaProvider, JwtConfig, JwtSigningKey, OAuthProxyConfig, OidcClientConfig,
    OidcProviderConfig, OneTapConfig, SiweConfig, SmsConfig, SmsProvider,
};
pub use plugins::{
    ACCOUNT_MANAGEMENT_PLUGIN, ADDITIONAL_FIELDS_PLUGIN, ADMIN_PLUGIN, AUTH_PLUGIN_DESCRIPTORS,
    AUTH_PLUGIN_STORAGE_REQUIREMENTS, AuthPluginDescriptor, AuthPluginStorageRequirement,
    BEARER_PLUGIN, CAPTCHA_PLUGIN, CUSTOM_SESSION_PLUGIN, DEVICE_AUTHORIZATION_PLUGIN,
    EMAIL_OTP_PLUGIN, EMAIL_PASSWORD_PLUGIN, EMAIL_VERIFICATION_PLUGIN, HAVEIBEENPWNED_PLUGIN,
    JWT_PLUGIN, LAST_LOGIN_METHOD_PLUGIN, MAGIC_LINK_PLUGIN, MCP_PLUGIN, MULTI_SESSION_PLUGIN,
    OAUTH_PROXY_PLUGIN, OIDC_PROVIDER_PLUGIN, ONE_TAP_PLUGIN, ONE_TIME_TOKEN_PLUGIN,
    OPEN_API_PLUGIN, ORGANIZATION_PLUGIN, PASSKEY_PLUGIN, PASSWORD_MANAGEMENT_PLUGIN,
    PHONE_NUMBER_PLUGIN, SESSION_MANAGEMENT_PLUGIN, SIWE_PLUGIN, TWO_FACTOR_PLUGIN,
    USER_MANAGEMENT_PLUGIN, USERNAME_PLUGIN, auth_plugin_descriptor, auth_plugin_descriptors,
    auth_plugin_storage_requirement, auth_plugin_storage_requirements,
};
pub use storage::{AuthStorage, AuthStorageCapabilities, AuthStorageClock};
pub use transport::{
    AuthCommandDescriptor, AuthCookieConfig, AuthErrorTaxonomyEntry, AuthHookContext,
    AuthHookOutcome, AuthRouteDescriptor, AuthVoxErrorStatus, CustomSessionTransport,
    PublicAuthError, SameSitePolicy, auth_command_descriptors, auth_error_taxonomy,
    auth_openapi_document, auth_route_descriptors, map_auth_error, map_auth_error_to_vox_status,
};

#[derive(Clone)]
pub struct ArchitectAuth<S> {
    pub config: ArchitectAuthConfig,
    pub storage: S,
}

impl ArchitectAuth<()> {
    pub fn builder() -> ArchitectAuthBuilder<()> {
        ArchitectAuthBuilder::new(())
    }
}
