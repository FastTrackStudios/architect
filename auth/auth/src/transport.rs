use auth_proto::AuthFlowError;
use cookie::{Cookie, SameSite, time::Duration};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::CustomSessionBundle;

// r[impl auth.transport.domain-first]
// r[impl auth.transport.openapi]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthCommandDescriptor {
    pub plugin_id: &'static str,
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub command_type: &'static str,
    pub response_type: &'static str,
    pub requires_session: bool,
}

// r[impl auth.transport.command-metadata]
macro_rules! auth_command_catalog {
    ($(
        {
            plugin: $plugin_id:literal,
            operation: $operation_id:literal,
            method: $method:literal,
            path: $path:literal,
            command: $command_type:literal,
            response: $response_type:literal,
            requires_session: $requires_session:literal $(,)?
        }
    ),+ $(,)?) => {
        pub const AUTH_COMMAND_DESCRIPTORS: &[AuthCommandDescriptor] = &[
            $(
                AuthCommandDescriptor {
                    plugin_id: $plugin_id,
                    operation_id: $operation_id,
                    method: $method,
                    path: $path,
                    command_type: $command_type,
                    response_type: $response_type,
                    requires_session: $requires_session,
                },
            )+
        ];

        pub const AUTH_ROUTE_DESCRIPTORS: &[AuthRouteDescriptor] = &[
            $(
                AuthRouteDescriptor {
                    operation_id: $operation_id,
                    method: $method,
                    path: $path,
                    command_type: $command_type,
                    response_type: $response_type,
                    requires_session: $requires_session,
                },
            )+
        ];
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthRouteDescriptor {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub command_type: &'static str,
    pub response_type: &'static str,
    pub requires_session: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomSessionTransport<T> {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub token: String,
    pub custom: T,
}

impl<T: Clone> From<&CustomSessionBundle<T>> for CustomSessionTransport<T> {
    fn from(bundle: &CustomSessionBundle<T>) -> Self {
        Self {
            user_id: bundle.user.id,
            session_id: bundle.session.id,
            token: bundle.token.clone(),
            custom: bundle.custom.clone(),
        }
    }
}

// r[impl auth.transport.domain-first]
// r[impl auth.transport.openapi]
auth_command_catalog! {
    { plugin: "email-password", operation: "createEmailPasswordUser", method: "POST", path: "/auth/sign-up/email", command: "CreateEmailPasswordUser", response: "AuthSessionBundle", requires_session: false },
    { plugin: "email-password", operation: "signInEmailPassword", method: "POST", path: "/auth/sign-in/email", command: "SignInEmailPassword", response: "AuthSessionBundle", requires_session: false },
    { plugin: "anonymous", operation: "signInAnonymous", method: "POST", path: "/auth/sign-in/anonymous", command: "SignInAnonymous", response: "AuthSessionBundle", requires_session: false },
    { plugin: "anonymous", operation: "linkAnonymousEmailPassword", method: "POST", path: "/auth/anonymous/link-email", command: "LinkAnonymousEmailPassword", response: "AuthSessionBundle", requires_session: true },
    { plugin: "anonymous", operation: "cleanupAnonymousUsers", method: "POST", path: "/auth/anonymous/cleanup", command: "CleanupAnonymousUsers", response: "CleanupAnonymousUsersResult", requires_session: true },
    { plugin: "session-management", operation: "currentSession", method: "GET", path: "/auth/session", command: "CurrentSession", response: "AuthSessionBundle", requires_session: true },
    { plugin: "custom-session", operation: "currentCustomSession", method: "GET", path: "/auth/get-session", command: "CurrentSession", response: "CustomSessionBundle<T>", requires_session: true },
    { plugin: "additional-fields", operation: "additionalFieldsSchema", method: "GET", path: "/auth/additional-fields/schema", command: "()", response: "AdditionalFieldsSchema", requires_session: false },
    { plugin: "open-api", operation: "getOpenApiDocument", method: "GET", path: "/auth/openapi.json", command: "()", response: "OpenApiDocument", requires_session: false },
    { plugin: "session-management", operation: "signOut", method: "POST", path: "/auth/sign-out", command: "SignOut", response: "()", requires_session: true },
    { plugin: "session-management", operation: "listSessions", method: "GET", path: "/auth/sessions", command: "ListSessions", response: "Vec<AuthSession>", requires_session: true },
    { plugin: "session-management", operation: "revokeSession", method: "POST", path: "/auth/session/revoke", command: "RevokeSession", response: "()", requires_session: true },
    { plugin: "session-management", operation: "revokeOtherSessions", method: "POST", path: "/auth/session/revoke-other", command: "RevokeOtherSessions", response: "()", requires_session: true },
    { plugin: "password-management", operation: "changePassword", method: "POST", path: "/auth/change-password", command: "ChangePassword", response: "()", requires_session: true },
    { plugin: "password-management", operation: "requestPasswordReset", method: "POST", path: "/auth/request-password-reset", command: "RequestPasswordReset", response: "VerificationToken", requires_session: false },
    { plugin: "password-management", operation: "completePasswordReset", method: "POST", path: "/auth/reset-password", command: "CompletePasswordReset", response: "()", requires_session: false },
    { plugin: "email-verification", operation: "requestEmailVerification", method: "POST", path: "/auth/send-verification-email", command: "RequestEmailVerification", response: "VerificationToken", requires_session: true },
    { plugin: "email-verification", operation: "verifyEmail", method: "POST", path: "/auth/verify-email", command: "VerifyEmail", response: "()", requires_session: false },
    { plugin: "user-management", operation: "changeEmail", method: "POST", path: "/auth/change-email", command: "ChangeEmail", response: "AuthUser", requires_session: true },
    { plugin: "user-management", operation: "deleteUser", method: "POST", path: "/auth/delete-user", command: "DeleteUser", response: "()", requires_session: true },
    { plugin: "account-management", operation: "listAccounts", method: "GET", path: "/auth/list-accounts", command: "ListAccounts", response: "Vec<AuthAccount>", requires_session: true },
    { plugin: "account-management", operation: "linkOAuthAccount", method: "POST", path: "/auth/link-social", command: "LinkOAuthAccount", response: "()", requires_session: true },
    { plugin: "account-management", operation: "unlinkOAuthAccount", method: "POST", path: "/auth/unlink-account", command: "UnlinkOAuthAccount", response: "()", requires_session: true },
    { plugin: "oauth", operation: "beginOAuthAuthorization", method: "POST", path: "/auth/sign-in/social", command: "BeginOAuthAuthorization", response: "VerificationToken", requires_session: false },
    { plugin: "oauth", operation: "verifyOAuthState", method: "GET", path: "/auth/callback/{provider}", command: "VerifyOAuthState", response: "()", requires_session: false },
    { plugin: "oauth", operation: "signInOAuthAccount", method: "POST", path: "/auth/oauth/sign-in", command: "SignInOAuthAccount", response: "AuthSessionBundle", requires_session: false },
    { plugin: "oauth", operation: "getOAuthAccessToken", method: "POST", path: "/auth/get-access-token", command: "GetOAuthAccessToken", response: "OAuthAccessToken", requires_session: true },
    { plugin: "oauth", operation: "refreshOAuthToken", method: "POST", path: "/auth/refresh-token", command: "RefreshOAuthToken", response: "OAuthAccessToken", requires_session: true },
    { plugin: "oauth-proxy", operation: "getOAuthProxyMetadata", method: "GET", path: "/auth/oauth-proxy/metadata", command: "()", response: "OAuthProxyMetadata", requires_session: false },
    { plugin: "oauth-proxy", operation: "beginOAuthProxyAuthorization", method: "POST", path: "/auth/oauth-proxy/begin", command: "BeginOAuthProxyAuthorization", response: "OAuthProxyAuthorization", requires_session: false },
    { plugin: "oauth-proxy", operation: "forwardOAuthProxyCallback", method: "POST", path: "/auth/oauth-proxy/forward", command: "ForwardOAuthProxyCallback", response: "OAuthProxyForwarding", requires_session: false },
    { plugin: "oauth-proxy", operation: "consumeOAuthProxyCallback", method: "GET", path: "/auth/oauth-proxy-callback", command: "ConsumeOAuthProxyCallback", response: "OAuthProxyProfile", requires_session: false },
    { plugin: "one-tap", operation: "oneTapCallback", method: "POST", path: "/auth/one-tap/callback", command: "OneTapCallback", response: "OneTapVerification", requires_session: false },
    { plugin: "one-time-token", operation: "generateOneTimeToken", method: "POST", path: "/auth/one-time-token/generate", command: "GenerateOneTimeToken", response: "OneTimeToken", requires_session: true },
    { plugin: "one-time-token", operation: "verifyOneTimeToken", method: "POST", path: "/auth/one-time-token/verify", command: "VerifyOneTimeToken", response: "OneTimeTokenVerification", requires_session: false },
    { plugin: "one-time-token", operation: "revokeOneTimeToken", method: "POST", path: "/auth/one-time-token/revoke", command: "RevokeOneTimeToken", response: "()", requires_session: false },
    { plugin: "multi-session", operation: "listDeviceSessions", method: "POST", path: "/auth/multi-session/list-device-sessions", command: "ListDeviceSessions", response: "DeviceSessions", requires_session: false },
    { plugin: "multi-session", operation: "setActiveDeviceSession", method: "POST", path: "/auth/multi-session/set-active", command: "SetActiveDeviceSession", response: "ActiveDeviceSession", requires_session: false },
    { plugin: "multi-session", operation: "revokeDeviceSession", method: "POST", path: "/auth/multi-session/revoke", command: "RevokeDeviceSession", response: "RevokeDeviceSessionResult", requires_session: false },
    { plugin: "last-login-method", operation: "getLastLoginMethod", method: "GET", path: "/auth/last-login-method", command: "GetLastLoginMethod", response: "LastLoginMethod", requires_session: true },
    { plugin: "last-login-method", operation: "clearLastLoginMethod", method: "POST", path: "/auth/last-login-method/clear", command: "ClearLastLoginMethod", response: "LastLoginMethod", requires_session: true },
    { plugin: "username", operation: "signInUsername", method: "POST", path: "/auth/sign-in/username", command: "SignInUsername", response: "AuthSessionBundle", requires_session: false },
    { plugin: "username", operation: "updateUsername", method: "POST", path: "/auth/username/update", command: "UpdateUsername", response: "AuthUser", requires_session: true },
    { plugin: "phone-number", operation: "sendPhoneNumberOtp", method: "POST", path: "/auth/phone-number/send-otp", command: "SendPhoneNumberOtp", response: "VerificationToken", requires_session: false },
    { plugin: "phone-number", operation: "verifyPhoneNumberOtp", method: "POST", path: "/auth/phone-number/verify", command: "VerifyPhoneNumberOtp", response: "PhoneNumberVerification", requires_session: false },
    { plugin: "phone-number", operation: "updatePhoneNumber", method: "POST", path: "/auth/phone-number/update", command: "UpdatePhoneNumber", response: "AuthUser", requires_session: true },
    { plugin: "siwe", operation: "createSiweNonce", method: "POST", path: "/auth/siwe/nonce", command: "CreateSiweNonce", response: "VerificationToken", requires_session: false },
    { plugin: "siwe", operation: "verifySiweMessage", method: "POST", path: "/auth/siwe/verify", command: "VerifySiweMessage", response: "AuthSessionBundle", requires_session: false },
    { plugin: "siwe", operation: "linkSiweAddress", method: "POST", path: "/auth/siwe/link", command: "LinkSiweAddress", response: "()", requires_session: true },
    { plugin: "haveibeenpwned", operation: "checkPasswordBreach", method: "POST", path: "/auth/haveibeenpwned/check", command: "CheckPasswordBreach", response: "PasswordBreachCheck", requires_session: false },
    { plugin: "mcp", operation: "authorizeMcpRequest", method: "POST", path: "/auth/mcp/authorize", command: "AuthorizeMcpRequest", response: "McpAuthorization", requires_session: false },
    { plugin: "api-key", operation: "createApiKey", method: "POST", path: "/auth/api-key/create", command: "CreateApiKey", response: "ApiKeyBundle", requires_session: true },
    { plugin: "api-key", operation: "getApiKey", method: "GET", path: "/auth/api-key/get", command: "GetApiKey", response: "AuthApiKey", requires_session: true },
    { plugin: "api-key", operation: "listApiKeys", method: "GET", path: "/auth/api-key/list", command: "ListApiKeys", response: "Vec<AuthApiKey>", requires_session: true },
    { plugin: "api-key", operation: "updateApiKey", method: "POST", path: "/auth/api-key/update", command: "UpdateApiKey", response: "AuthApiKey", requires_session: true },
    { plugin: "api-key", operation: "deleteApiKey", method: "POST", path: "/auth/api-key/delete", command: "DeleteApiKey", response: "()", requires_session: true },
    { plugin: "api-key", operation: "verifyApiKey", method: "POST", path: "/auth/api-key/verify", command: "VerifyApiKey", response: "ApiKeyBundle", requires_session: false },
    { plugin: "bearer", operation: "authenticateBearerToken", method: "POST", path: "/auth/bearer/authenticate", command: "AuthenticateBearerToken", response: "BearerTokenBundle", requires_session: false },
    { plugin: "captcha", operation: "verifyCaptcha", method: "POST", path: "/auth/captcha/verify", command: "VerifyCaptcha", response: "CaptchaVerification", requires_session: false },
    { plugin: "email-otp", operation: "sendEmailOtp", method: "POST", path: "/auth/email-otp/send", command: "SendEmailOtp", response: "VerificationToken", requires_session: false },
    { plugin: "email-otp", operation: "verifyEmailOtp", method: "POST", path: "/auth/email-otp/verify", command: "VerifyEmailOtp", response: "EmailOtpVerification", requires_session: false },
    { plugin: "magic-link", operation: "sendMagicLink", method: "POST", path: "/auth/magic-link/send", command: "SendMagicLink", response: "MagicLinkToken", requires_session: false },
    { plugin: "magic-link", operation: "verifyMagicLink", method: "POST", path: "/auth/magic-link/verify", command: "VerifyMagicLink", response: "MagicLinkVerification", requires_session: false },
    { plugin: "jwt", operation: "issueJwt", method: "POST", path: "/auth/jwt/issue", command: "IssueJwt", response: "JwtToken", requires_session: true },
    { plugin: "jwt", operation: "verifyJwt", method: "POST", path: "/auth/jwt/verify", command: "VerifyJwt", response: "JwtVerification", requires_session: false },
    { plugin: "jwt", operation: "getJwtKeySet", method: "GET", path: "/auth/jwt/jwks", command: "()", response: "JwtKeySet", requires_session: false },
    { plugin: "oidc-provider", operation: "getOidcDiscovery", method: "GET", path: "/.well-known/openid-configuration", command: "()", response: "OidcDiscovery", requires_session: false },
    { plugin: "oidc-provider", operation: "registerOidcClient", method: "POST", path: "/oauth2/register", command: "RegisterOidcClient", response: "OidcClientRegistration", requires_session: false },
    { plugin: "oidc-provider", operation: "authorizeOidc", method: "GET", path: "/oauth2/authorize", command: "AuthorizeOidc", response: "OidcAuthorization", requires_session: true },
    { plugin: "oidc-provider", operation: "exchangeOidcToken", method: "POST", path: "/oauth2/token", command: "ExchangeOidcToken", response: "OidcTokenResponse", requires_session: false },
    { plugin: "oidc-provider", operation: "getOidcUserInfo", method: "GET", path: "/oauth2/userinfo", command: "GetOidcUserInfo", response: "OidcUserInfo", requires_session: false },
    { plugin: "passkey", operation: "beginPasskeyRegistration", method: "POST", path: "/auth/passkey/register/begin", command: "BeginPasskeyRegistration", response: "VerificationToken", requires_session: true },
    { plugin: "passkey", operation: "completePasskeyRegistration", method: "POST", path: "/auth/passkey/register/complete", command: "CompletePasskeyRegistration", response: "AuthPasskey", requires_session: true },
    { plugin: "passkey", operation: "beginPasskeyAuthentication", method: "POST", path: "/auth/passkey/authenticate/begin", command: "BeginPasskeyAuthentication", response: "VerificationToken", requires_session: false },
    { plugin: "passkey", operation: "completePasskeyAuthentication", method: "POST", path: "/auth/passkey/authenticate/complete", command: "CompletePasskeyAuthentication", response: "AuthSessionBundle", requires_session: false },
    { plugin: "passkey", operation: "listPasskeys", method: "GET", path: "/auth/passkey/list", command: "ListPasskeys", response: "Vec<AuthPasskey>", requires_session: true },
    { plugin: "passkey", operation: "deletePasskey", method: "POST", path: "/auth/passkey/delete", command: "DeletePasskey", response: "()", requires_session: true },
    { plugin: "device-authorization", operation: "createDeviceAuthorization", method: "POST", path: "/auth/device/code", command: "CreateDeviceAuthorization", response: "DeviceAuthorization", requires_session: false },
    { plugin: "device-authorization", operation: "verifyDeviceCode", method: "POST", path: "/auth/device/verify", command: "VerifyDeviceCode", response: "DeviceCodeVerification", requires_session: false },
    { plugin: "device-authorization", operation: "approveDeviceCode", method: "POST", path: "/auth/device/approve", command: "ApproveDeviceCode", response: "()", requires_session: true },
    { plugin: "device-authorization", operation: "denyDeviceCode", method: "POST", path: "/auth/device/deny", command: "DenyDeviceCode", response: "()", requires_session: false },
    { plugin: "device-authorization", operation: "pollDeviceToken", method: "POST", path: "/auth/device/token", command: "PollDeviceToken", response: "AuthSessionBundle", requires_session: false },
    { plugin: "two-factor", operation: "startTwoFactorSetup", method: "POST", path: "/auth/two-factor/start", command: "StartTwoFactorSetup", response: "()", requires_session: true },
    { plugin: "two-factor", operation: "confirmTwoFactor", method: "POST", path: "/auth/two-factor/confirm", command: "ConfirmTwoFactor", response: "()", requires_session: true },
    { plugin: "two-factor", operation: "verifyTwoFactor", method: "POST", path: "/auth/two-factor/verify", command: "VerifyTwoFactor", response: "()", requires_session: false },
    { plugin: "two-factor", operation: "disableTwoFactor", method: "POST", path: "/auth/two-factor/disable", command: "DisableTwoFactor", response: "()", requires_session: true },
    { plugin: "organization", operation: "createOrganization", method: "POST", path: "/auth/organization/create", command: "CreateOrganization", response: "OrganizationBundle", requires_session: true },
    { plugin: "organization", operation: "setActiveOrganization", method: "POST", path: "/auth/organization/set-active", command: "SetActiveOrganization", response: "()", requires_session: true },
    { plugin: "organization", operation: "requireOrganizationRole", method: "POST", path: "/auth/organization/require-role", command: "RequireOrganizationRole", response: "()", requires_session: true },
    { plugin: "organization", operation: "authorizeOrganizationAction", method: "POST", path: "/auth/organization/authorize", command: "AuthorizeOrganizationAction", response: "()", requires_session: true },
    { plugin: "organization", operation: "setMemberRole", method: "POST", path: "/auth/organization/update-member-role", command: "SetMemberRole", response: "AuthMember", requires_session: true },
    { plugin: "organization", operation: "createInvitation", method: "POST", path: "/auth/organization/invite-member", command: "CreateInvitation", response: "InvitationToken", requires_session: true },
    { plugin: "organization", operation: "acceptInvitation", method: "POST", path: "/auth/organization/accept-invitation", command: "AcceptInvitation", response: "AuthMember", requires_session: true },
    { plugin: "organization", operation: "createOrganizationRole", method: "POST", path: "/auth/organization/roles", command: "CreateOrganizationRole", response: "AuthOrganizationRole", requires_session: true },
    { plugin: "organization", operation: "updateOrganizationRole", method: "PATCH", path: "/auth/organization/roles/{role}", command: "UpdateOrganizationRole", response: "AuthOrganizationRole", requires_session: true },
    { plugin: "organization", operation: "deleteOrganizationRole", method: "DELETE", path: "/auth/organization/roles/{role}", command: "DeleteOrganizationRole", response: "()", requires_session: true },
    { plugin: "organization", operation: "listOrganizationRoles", method: "GET", path: "/auth/organization/roles", command: "ListOrganizationRoles", response: "Vec<AuthOrganizationRole>", requires_session: true },
    { plugin: "organization", operation: "createTeam", method: "POST", path: "/auth/organization/teams", command: "CreateTeam", response: "AuthTeam", requires_session: true },
    { plugin: "organization", operation: "updateTeam", method: "PATCH", path: "/auth/organization/teams/{team_id}", command: "UpdateTeam", response: "AuthTeam", requires_session: true },
    { plugin: "organization", operation: "deleteTeam", method: "DELETE", path: "/auth/organization/teams/{team_id}", command: "DeleteTeam", response: "()", requires_session: true },
    { plugin: "organization", operation: "listTeams", method: "GET", path: "/auth/organization/teams", command: "ListTeams", response: "Vec<AuthTeam>", requires_session: true },
    { plugin: "organization", operation: "addTeamMember", method: "POST", path: "/auth/organization/teams/{team_id}/members", command: "AddTeamMember", response: "AuthTeamMember", requires_session: true },
    { plugin: "organization", operation: "removeTeamMember", method: "DELETE", path: "/auth/organization/teams/{team_id}/members/{user_id}", command: "RemoveTeamMember", response: "()", requires_session: true },
    { plugin: "organization", operation: "listTeamMembers", method: "GET", path: "/auth/organization/teams/{team_id}/members", command: "ListTeamMembers", response: "Vec<AuthTeamMember>", requires_session: true },
    { plugin: "admin", operation: "listUsers", method: "GET", path: "/auth/admin/list-users", command: "ListUsers", response: "ListUsersResult", requires_session: true },
    { plugin: "admin", operation: "adminCreateUser", method: "POST", path: "/auth/admin/create-user", command: "AdminCreateUser", response: "AuthUser", requires_session: true },
    { plugin: "admin", operation: "setUserRole", method: "POST", path: "/auth/admin/set-role", command: "SetUserRole", response: "AuthUser", requires_session: true },
    { plugin: "admin", operation: "adminSetUserPassword", method: "POST", path: "/auth/admin/set-user-password", command: "AdminSetUserPassword", response: "()", requires_session: true },
    { plugin: "admin", operation: "banUser", method: "POST", path: "/auth/admin/ban-user", command: "BanUser", response: "AuthUser", requires_session: true },
    { plugin: "admin", operation: "unbanUser", method: "POST", path: "/auth/admin/unban-user", command: "UnbanUser", response: "AuthUser", requires_session: true },
    { plugin: "admin", operation: "listUserSessions", method: "POST", path: "/auth/admin/list-user-sessions", command: "ListUserSessions", response: "Vec<AuthSession>", requires_session: true },
    { plugin: "admin", operation: "revokeUserSession", method: "POST", path: "/auth/admin/revoke-user-session", command: "RevokeUserSession", response: "()", requires_session: true },
    { plugin: "admin", operation: "revokeUserSessions", method: "POST", path: "/auth/admin/revoke-user-sessions", command: "RevokeUserSessions", response: "()", requires_session: true },
    { plugin: "admin", operation: "impersonateUser", method: "POST", path: "/auth/admin/impersonate-user", command: "ImpersonateUser", response: "AuthSessionBundle", requires_session: true },
    { plugin: "admin", operation: "stopImpersonating", method: "POST", path: "/auth/admin/stop-impersonating", command: "StopImpersonating", response: "()", requires_session: true },
    { plugin: "admin", operation: "removeUser", method: "POST", path: "/auth/admin/remove-user", command: "RemoveUser", response: "()", requires_session: true },
    { plugin: "admin", operation: "adminHasPermission", method: "POST", path: "/auth/admin/has-permission", command: "AdminHasPermission", response: "HasPermissionResult", requires_session: true },
}

// r[verify auth.transport.domain-first]
// r[verify auth.transport.openapi]
pub fn auth_route_descriptors() -> &'static [AuthRouteDescriptor] {
    AUTH_ROUTE_DESCRIPTORS
}

// r[verify auth.transport.command-metadata]
pub fn auth_command_descriptors() -> &'static [AuthCommandDescriptor] {
    AUTH_COMMAND_DESCRIPTORS
}

// r[impl auth.transport.openapi]
// r[impl auth.openapi.plugin-aware]
// r[impl auth.openapi.request-schemas]
// r[impl auth.openapi.response-schemas]
// r[impl auth.openapi.error-schemas]
pub fn auth_openapi_document() -> Value {
    let mut paths = serde_json::Map::new();
    let mut schemas = serde_json::Map::new();
    schemas.insert("PublicAuthError".into(), public_error_schema());
    for command in auth_command_descriptors() {
        let method = command.method.to_ascii_lowercase();
        let path_item = paths
            .entry(command.path)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let Value::Object(methods) = path_item else {
            continue;
        };
        methods.insert(method, openapi_operation(command));
        if command.command_type != "()" {
            schemas.insert(
                schema_component_name(command.command_type),
                rust_type_schema(command.command_type),
            );
        }
        if command.response_type != "()" {
            schemas.insert(
                schema_component_name(command.response_type),
                rust_type_schema(command.response_type),
            );
        }
    }
    let enabled_plugins = enabled_openapi_plugins();

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "architect-auth",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "x-architect-enabled-plugins": enabled_plugins,
        "paths": paths,
        "components": {
            "schemas": schemas,
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                },
                "sessionCookie": {
                    "type": "apiKey",
                    "in": "cookie",
                    "name": AuthCookieConfig::default().name,
                },
            },
        },
    })
}

fn openapi_operation(command: &AuthCommandDescriptor) -> Value {
    let mut operation = json!({
        "operationId": command.operation_id,
        "x-architect-plugin": command.plugin_id,
        "x-architect-command": command.command_type,
        "x-architect-response": command.response_type,
        "responses": {
            "200": {
                "description": command.response_type,
                "content": {
                    "application/json": {
                        "schema": schema_ref(command.response_type),
                    },
                },
            },
            "400": {
                "description": "invalid input",
                "content": public_error_content(),
            },
            "401": {
                "description": "invalid credentials or expired session",
                "content": public_error_content(),
            },
            "403": {
                "description": "permission, verification, or two-factor challenge required",
                "content": public_error_content(),
            },
            "500": {
                "description": "internal error",
                "content": public_error_content(),
            },
        },
    });

    if command.method != "GET" && command.method != "DELETE" && command.command_type != "()" {
        operation["requestBody"] = json!({
            "required": true,
            "content": {
                "application/json": {
                    "schema": schema_ref(command.command_type),
                },
            },
        });
    }

    if command.requires_session {
        operation["security"] = json!([
            { "bearerAuth": [] },
            { "sessionCookie": [] },
        ]);
    }

    operation
}

fn enabled_openapi_plugins() -> Vec<&'static str> {
    let mut plugins = auth_command_descriptors()
        .iter()
        .map(|command| command.plugin_id)
        .collect::<Vec<_>>();
    plugins.sort_unstable();
    plugins.dedup();
    plugins
}

fn schema_ref(rust_type: &str) -> Value {
    if rust_type == "()" {
        json!({ "type": "null" })
    } else {
        json!({ "$ref": format!("#/components/schemas/{}", schema_component_name(rust_type)) })
    }
}

fn rust_type_schema(rust_type: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "x-rust-type": rust_type,
    })
}

fn schema_component_name(rust_type: &str) -> String {
    let mut output = String::with_capacity(rust_type.len());
    for ch in rust_type.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch);
        } else if !output.ends_with('_') {
            output.push('_');
        }
    }
    output.trim_matches('_').to_owned()
}

// r[impl auth.errors.openapi]
fn public_error_schema() -> Value {
    let codes = auth_error_taxonomy()
        .iter()
        .map(|entry| Value::String(entry.code.into()))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "required": ["code", "message", "status"],
        "properties": {
            "code": { "type": "string", "enum": codes },
            "message": { "type": "string" },
            "status": { "type": "integer" },
        },
    })
}

fn public_error_content() -> Value {
    json!({
        "application/json": {
            "schema": {
                "$ref": "#/components/schemas/PublicAuthError",
            },
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSitePolicy {
    Strict,
    Lax,
    None,
}

impl From<SameSitePolicy> for SameSite {
    fn from(value: SameSitePolicy) -> Self {
        match value {
            SameSitePolicy::Strict => Self::Strict,
            SameSitePolicy::Lax => Self::Lax,
            SameSitePolicy::None => Self::None,
        }
    }
}

// r[impl auth.transport.cookie-security]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthCookieConfig {
    pub name: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSitePolicy,
    pub domain: Option<String>,
    pub path: String,
    pub max_age_seconds: Option<i64>,
}

impl Default for AuthCookieConfig {
    fn default() -> Self {
        Self {
            name: "architect-auth.session".into(),
            secure: true,
            http_only: true,
            same_site: SameSitePolicy::Lax,
            domain: None,
            path: "/".into(),
            max_age_seconds: None,
        }
    }
}

impl AuthCookieConfig {
    // r[verify auth.transport.cookie-security]
    pub fn session_cookie(&self, token: impl Into<String>) -> Cookie<'static> {
        let mut cookie = Cookie::build((self.name.clone(), token.into()))
            .secure(self.secure)
            .http_only(self.http_only)
            .same_site(SameSite::from(self.same_site))
            .path(self.path.clone());
        if let Some(domain) = self.domain.clone() {
            cookie = cookie.domain(domain);
        }
        if let Some(max_age_seconds) = self.max_age_seconds {
            cookie = cookie.max_age(Duration::seconds(max_age_seconds));
        }
        cookie.build()
    }
}

// r[impl auth.transport.error-mapping]
// r[impl auth.errors.taxonomy]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicAuthError {
    pub status: u16,
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthVoxErrorStatus {
    InvalidArgument,
    Unauthenticated,
    PermissionDenied,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthErrorTaxonomyEntry {
    pub rust_variant: &'static str,
    pub code: &'static str,
    pub message: &'static str,
    pub http_status: u16,
    pub vox_status: AuthVoxErrorStatus,
}

pub const AUTH_ERROR_TAXONOMY: &[AuthErrorTaxonomyEntry] = &[
    AuthErrorTaxonomyEntry {
        rust_variant: "InvalidCredentials",
        code: "invalid_credentials",
        message: "invalid credentials",
        http_status: 401,
        vox_status: AuthVoxErrorStatus::Unauthenticated,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "SessionExpired",
        code: "session_expired",
        message: "session expired",
        http_status: 401,
        vox_status: AuthVoxErrorStatus::Unauthenticated,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "VerificationRequired",
        code: "verification_required",
        message: "verification required",
        http_status: 403,
        vox_status: AuthVoxErrorStatus::PermissionDenied,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "TwoFactorRequired",
        code: "two_factor_required",
        message: "two-factor required",
        http_status: 403,
        vox_status: AuthVoxErrorStatus::PermissionDenied,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "PermissionDenied",
        code: "permission_denied",
        message: "permission denied",
        http_status: 403,
        vox_status: AuthVoxErrorStatus::PermissionDenied,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "InvalidInput",
        code: "invalid_input",
        message: "invalid input",
        http_status: 400,
        vox_status: AuthVoxErrorStatus::InvalidArgument,
    },
    AuthErrorTaxonomyEntry {
        rust_variant: "Internal",
        code: "internal",
        message: "internal error",
        http_status: 500,
        vox_status: AuthVoxErrorStatus::Internal,
    },
];

pub fn auth_error_taxonomy() -> &'static [AuthErrorTaxonomyEntry] {
    AUTH_ERROR_TAXONOMY
}

// r[verify auth.transport.error-mapping]
// r[impl auth.errors.axum]
pub fn map_auth_error(error: &AuthFlowError) -> PublicAuthError {
    let entry = auth_error_taxonomy_entry(error);
    PublicAuthError {
        status: entry.http_status,
        code: entry.code,
        message: entry.message,
    }
}

// r[impl auth.errors.vox]
pub fn map_auth_error_to_vox_status(error: &AuthFlowError) -> AuthVoxErrorStatus {
    auth_error_taxonomy_entry(error).vox_status
}

pub fn auth_error_taxonomy_entry(error: &AuthFlowError) -> &'static AuthErrorTaxonomyEntry {
    let rust_variant = match error {
        AuthFlowError::InvalidCredentials => "InvalidCredentials",
        AuthFlowError::SessionExpired => "SessionExpired",
        AuthFlowError::VerificationRequired => "VerificationRequired",
        AuthFlowError::TwoFactorRequired => "TwoFactorRequired",
        AuthFlowError::PermissionDenied => "PermissionDenied",
        AuthFlowError::InvalidInput(_) => "InvalidInput",
        AuthFlowError::Internal(_) => "Internal",
    };
    auth_error_taxonomy()
        .iter()
        .find(|entry| entry.rust_variant == rust_variant)
        .expect("auth error taxonomy covers every AuthFlowError variant")
}

// r[impl auth.transport.hooks-typed]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthHookContext<C> {
    pub command: C,
    pub route: Option<AuthRouteDescriptor>,
}

// r[impl auth.transport.hooks-typed]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthHookOutcome<R> {
    pub result: Result<R, AuthFlowError>,
}

#[cfg(feature = "axum")]
pub mod axum {
    use super::{
        AuthCookieConfig, AuthRouteDescriptor, CustomSessionTransport, PublicAuthError,
        auth_route_descriptors, map_auth_error,
    };
    use crate::{
        ArchitectAuth, AuthStorage, CurrentSession, CustomSessionBundle, flows::bearer_tokens,
    };
    use ::axum::{
        extract::{Request, State},
        http::{HeaderMap, StatusCode, header},
        middleware::Next,
        response::{IntoResponse, Response},
    };
    use auth_proto::AuthFlowError;
    use uuid::Uuid;

    // r[impl auth.transport.axum-feature]
    pub use architect::axum_ws;

    // r[impl auth.transport.axum-feature]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct AxumAuthRoutes {
        pub descriptors: &'static [AuthRouteDescriptor],
    }

    // r[verify auth.transport.axum-feature]
    pub fn routes() -> AxumAuthRoutes {
        AxumAuthRoutes {
            descriptors: auth_route_descriptors(),
        }
    }

    // r[impl auth.transport.axum-feature]
    // r[impl auth.transport.error-mapping]
    #[derive(Clone)]
    pub struct AxumAuthState<S> {
        pub auth: ArchitectAuth<S>,
        pub cookie: AuthCookieConfig,
    }

    impl<S> AxumAuthState<S> {
        pub fn new(auth: ArchitectAuth<S>) -> Self {
            Self {
                auth,
                cookie: AuthCookieConfig::default(),
            }
        }

        pub fn with_cookie(mut self, cookie: AuthCookieConfig) -> Self {
            self.cookie = cookie;
            self
        }
    }

    // r[impl auth.transport.axum-feature]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct AuthenticatedSession {
        pub user_id: Uuid,
        pub session_id: Uuid,
        pub token: String,
    }

    // r[impl auth.custom-session.axum]
    #[derive(Clone, Debug, PartialEq)]
    pub struct AuthenticatedCustomSession<T> {
        pub user_id: Uuid,
        pub session_id: Uuid,
        pub token: String,
        pub custom: T,
    }

    impl<T: Clone> From<&CustomSessionBundle<T>> for AuthenticatedCustomSession<T> {
        fn from(bundle: &CustomSessionBundle<T>) -> Self {
            let transport = CustomSessionTransport::from(bundle);
            Self {
                user_id: transport.user_id,
                session_id: transport.session_id,
                token: transport.token,
                custom: transport.custom,
            }
        }
    }

    // r[impl auth.custom-session.axum]
    pub fn insert_custom_session_extension<T>(
        request: &mut Request,
        bundle: &CustomSessionBundle<T>,
    ) where
        T: Clone + Send + Sync + 'static,
    {
        request
            .extensions_mut()
            .insert(AuthenticatedCustomSession::from(bundle));
    }

    // r[impl auth.transport.error-mapping]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AxumAuthError {
        pub public: PublicAuthError,
    }

    impl From<AuthFlowError> for AxumAuthError {
        fn from(error: AuthFlowError) -> Self {
            Self {
                public: map_auth_error(&error),
            }
        }
    }

    impl IntoResponse for AxumAuthError {
        fn into_response(self) -> Response {
            let status = StatusCode::from_u16(self.public.status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, self.public.message).into_response()
        }
    }

    // r[impl auth.transport.axum-feature]
    pub fn session_token_from_headers(
        headers: &HeaderMap,
        cookie: &AuthCookieConfig,
    ) -> Option<String> {
        bearer_token(headers).or_else(|| session_cookie_token(headers, cookie))
    }

    fn bearer_token(headers: &HeaderMap) -> Option<String> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| bearer_tokens::parse_authorization_header(Some(value)).ok())
    }

    fn session_cookie_token(headers: &HeaderMap, cookie: &AuthCookieConfig) -> Option<String> {
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookies| {
                cookies.split(';').map(str::trim).find_map(|pair| {
                    pair.strip_prefix(cookie.name.as_str())
                        .and_then(|value| value.strip_prefix('='))
                })
            })
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
    }

    // r[impl auth.transport.axum-feature]
    // r[impl auth.transport.domain-first]
    pub async fn require_session<S>(
        State(state): State<AxumAuthState<S>>,
        mut request: Request,
        next: Next,
    ) -> Result<Response, AxumAuthError>
    where
        S: AuthStorage,
    {
        let token =
            session_token_from_headers(request.headers(), &state.cookie).ok_or(AxumAuthError {
                public: map_auth_error(&AuthFlowError::InvalidCredentials),
            })?;
        let bundle = state
            .auth
            .current_session(CurrentSession {
                token: token.clone(),
            })
            .await?;

        request.extensions_mut().insert(AuthenticatedSession {
            user_id: bundle.user.id,
            session_id: bundle.session.id,
            token,
        });

        Ok(next.run(request).await)
    }
}

#[cfg(feature = "vox")]
pub mod vox {
    use super::CustomSessionTransport;
    use crate::{
        ArchitectAuth, AuthStorage, CreateEmailPasswordUser, CurrentSession, CustomSessionBundle,
        ChangeEmail, ChangePassword, MigrateUserEmail, RefreshSession, SignOut, UpdateProfile,
        flows::bearer_tokens,
    };
    use auth_proto::{
        AuthFlowError, AuthService, AuthSessionBundle, AuthUser, OrgMember, SignInEmailPassword,
        SignUpEmailPassword,
    };
    use uuid::Uuid;

    pub use auth_proto::AUTHORIZATION_METADATA_KEY;

    // r[impl auth.transport.vox-schema]
    #[derive(Clone)]
    pub struct AuthVoxService<S> {
        pub auth: ArchitectAuth<S>,
    }

    impl<S> AuthVoxService<S> {
        pub fn new(auth: ArchitectAuth<S>) -> Self {
            Self { auth }
        }
    }

    impl<S> AuthService for AuthVoxService<S>
    where
        S: AuthStorage,
    {
        async fn sign_up_email_password(
            &self,
            input: SignUpEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            self.auth
                .create_email_password_user(CreateEmailPasswordUser {
                    email: input.email,
                    password: input.password,
                    name: input.name,
                    username: input.username,
                    image: input.image,
                    metadata_json: input.metadata_json,
                    ip_address: input.ip_address,
                    user_agent: input.user_agent,
                })
                .await
        }

        async fn sign_in_email_password(
            &self,
            input: SignInEmailPassword,
        ) -> Result<AuthSessionBundle, AuthFlowError> {
            self.auth.sign_in_email_password(input).await
        }

        async fn current_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError> {
            self.auth.current_session(CurrentSession { token }).await
        }

        async fn refresh_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError> {
            self.auth.refresh_session(RefreshSession { token }).await
        }

        async fn whoami(&self, token: String) -> Result<AuthUser, AuthFlowError> {
            self.auth
                .current_session(CurrentSession { token })
                .await
                .map(|bundle| bundle.user)
        }

        async fn sign_out(&self, token: String) -> Result<(), AuthFlowError> {
            self.auth.sign_out(SignOut { token }).await
        }

        async fn list_org_members(&self, token: String) -> Result<Vec<OrgMember>, AuthFlowError> {
            self.auth.org_members_for_token(token).await
        }

        async fn migrate_user_email(
            &self,
            input: auth_proto::service::MigrateUserEmailRequest,
        ) -> Result<AuthUser, AuthFlowError> {
            // The session is the authorization: it must validate against
            // THIS org's store. `migrate_user_email` itself takes no
            // session — it is a storage operation — so the check has to
            // happen here, at the network edge.
            let caller = self
                .auth
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.auth
                .migrate_user_email(MigrateUserEmail {
                    user_id: input.user_id,
                    new_email: input.new_email,
                    // Who did it, from the validated session rather than
                    // anything the caller supplied.
                    changed_by: Some(caller.user.id),
                    reason: input.reason,
                })
                .await
        }

        async fn list_email_history(
            &self,
            input: auth_proto::service::EmailHistoryRequest,
        ) -> Result<Vec<auth_proto::email_change::AuthEmailChange>, AuthFlowError> {
            self.auth
                .current_session(CurrentSession {
                    token: input.session_token,
                })
                .await?;
            self.auth.list_email_history(input.user_id).await
        }

        async fn update_profile(
            &self,
            input: auth_proto::service::UpdateProfileRequest,
        ) -> Result<AuthUser, AuthFlowError> {
            self.auth
                .update_profile(UpdateProfile {
                    session_token: input.session_token,
                    name: input.name,
                    image: input.image,
                })
                .await
        }

        async fn change_email(
            &self,
            input: auth_proto::service::ChangeEmailRequest,
        ) -> Result<AuthUser, AuthFlowError> {
            // `change_email` validates the session, rejects an address
            // another account holds, and appends to the history trail.
            self.auth
                .change_email(ChangeEmail {
                    session_token: input.session_token,
                    new_email: input.new_email,
                })
                .await
        }

        async fn change_password(
            &self,
            input: auth_proto::service::ChangePasswordRequest,
        ) -> Result<(), AuthFlowError> {
            // No extra checks here: `change_password` already validates
            // the session, verifies the current password, enforces
            // strength and rejects breached passwords.
            self.auth
                .change_password(ChangePassword {
                    session_token: input.session_token,
                    current_password: input.current_password,
                    new_password: input.new_password,
                })
                .await
        }
    }

    // r[impl auth.transport.vox-schema]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct AuthVoxContext {
        pub token: Option<String>,
    }

    // r[impl auth.custom-session.vox]
    #[derive(Clone, Debug, PartialEq)]
    pub struct AuthVoxCustomSession<T> {
        pub user_id: Uuid,
        pub session_id: Uuid,
        pub token: String,
        pub custom: T,
    }

    impl<T: Clone> From<&CustomSessionBundle<T>> for AuthVoxCustomSession<T> {
        fn from(bundle: &CustomSessionBundle<T>) -> Self {
            let transport = CustomSessionTransport::from(bundle);
            Self {
                user_id: transport.user_id,
                session_id: transport.session_id,
                token: transport.token,
                custom: transport.custom,
            }
        }
    }

    // r[impl auth.custom-session.vox]
    pub fn insert_custom_session_extension<T>(
        extensions: &::vox::Extensions,
        bundle: &CustomSessionBundle<T>,
    ) where
        T: Clone + Send + Sync + 'static,
    {
        extensions.insert(AuthVoxCustomSession::from(bundle));
    }

    // r[impl auth.transport.vox-schema]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct AuthClientMiddleware {
        token: String,
    }

    impl AuthClientMiddleware {
        pub fn bearer(token: impl Into<String>) -> Self {
            Self {
                token: token.into(),
            }
        }
    }

    impl ::vox::ClientMiddleware for AuthClientMiddleware {
        fn pre<'a, 'call>(
            &'a self,
            _context: &'a ::vox::ClientContext<'a>,
            request: &'a mut ::vox::ClientRequest<'call, 'a>,
        ) -> ::vox::BoxMiddlewareFuture<'a> {
            Box::pin(async move {
                request.push_string_metadata(
                    AUTHORIZATION_METADATA_KEY,
                    format!("Bearer {}", self.token),
                );
            })
        }
    }

    // r[impl auth.transport.vox-schema]
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct AuthServerMiddleware;

    impl ::vox::ServerMiddleware for AuthServerMiddleware {
        fn pre<'a>(&'a self, request: ::vox::ServerRequest<'_>) -> ::vox::BoxMiddlewareFuture<'a> {
            let token = authorization_token_from_metadata(request.metadata());
            request.extensions().insert(AuthVoxContext { token });
            Box::pin(async {})
        }
    }

    // r[impl auth.transport.vox-schema]
    pub fn authorization_token_from_metadata(metadata: &::vox::Metadata) -> Option<String> {
        use ::vox::MetadataExt;
        metadata
            .meta_str(AUTHORIZATION_METADATA_KEY)
            .and_then(|value| bearer_tokens::parse_authorization_header(Some(value)).ok())
    }
}

// r[impl auth.transport.vox-schema]
// r[verify auth.transport.vox-schema]
pub type VoxSignInEmailPassword = auth_proto::SignInEmailPassword;

#[cfg(test)]
mod tests {
    use super::{
        AuthCookieConfig, AuthHookContext, AuthVoxErrorStatus, CustomSessionTransport,
        SameSitePolicy, auth_command_descriptors, auth_error_taxonomy, auth_openapi_document,
        auth_route_descriptors, map_auth_error, map_auth_error_to_vox_status,
    };
    use crate::CreateEmailPasswordUser;
    use auth_proto::{AuthFlowError, AuthSession, AuthSessionBundle, AuthUser};
    use chrono::{Duration, Utc};
    use proptest::prelude::*;
    use uuid::Uuid;

    #[test]
    fn route_descriptors_name_domain_commands() {
        let descriptors = auth_route_descriptors();
        assert!(descriptors.iter().any(|route| {
            route.operation_id == "createEmailPasswordUser"
                && route.command_type == "CreateEmailPasswordUser"
                && route.response_type == "AuthSessionBundle"
        }));
        assert!(descriptors.iter().any(|route| route.requires_session));
    }

    #[test]
    // r[verify auth.transport.command-metadata]
    fn command_metadata_is_the_transport_source_of_truth() {
        let commands = auth_command_descriptors();

        assert_eq!(commands.len(), auth_route_descriptors().len());
        assert!(commands.iter().any(|command| {
            command.plugin_id == "email-password"
                && command.operation_id == "signInEmailPassword"
                && command.command_type == "SignInEmailPassword"
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "session-management"
                && command.operation_id == "revokeOtherSessions"
                && command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "two-factor"
                && command.operation_id == "verifyTwoFactor"
                && command.command_type == "VerifyTwoFactor"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "passkey"
                && command.operation_id == "completePasskeyAuthentication"
                && command.response_type == "AuthSessionBundle"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "device-authorization"
                && command.operation_id == "pollDeviceToken"
                && command.response_type == "AuthSessionBundle"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "anonymous"
                && command.operation_id == "linkAnonymousEmailPassword"
                && command.command_type == "LinkAnonymousEmailPassword"
                && command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "bearer"
                && command.operation_id == "authenticateBearerToken"
                && command.command_type == "AuthenticateBearerToken"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "captcha"
                && command.operation_id == "verifyCaptcha"
                && command.command_type == "VerifyCaptcha"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "email-otp"
                && command.operation_id == "verifyEmailOtp"
                && command.response_type == "EmailOtpVerification"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "magic-link"
                && command.operation_id == "verifyMagicLink"
                && command.response_type == "MagicLinkVerification"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "jwt"
                && command.operation_id == "issueJwt"
                && command.response_type == "JwtToken"
                && command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "custom-session"
                && command.operation_id == "currentCustomSession"
                && command.response_type == "CustomSessionBundle<T>"
                && command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "additional-fields"
                && command.operation_id == "additionalFieldsSchema"
                && command.response_type == "AdditionalFieldsSchema"
                && !command.requires_session
        }));
        assert!(commands.iter().any(|command| {
            command.plugin_id == "open-api"
                && command.operation_id == "getOpenApiDocument"
                && command.response_type == "OpenApiDocument"
                && !command.requires_session
        }));
    }

    // r[verify auth.boundary.property-tests]
    #[test]
    fn plugin_registry_openapi_generation_has_unique_command_boundaries() {
        let commands = auth_command_descriptors();
        let document = auth_openapi_document();

        for (index, command) in commands.iter().enumerate() {
            assert!(
                commands[index + 1..]
                    .iter()
                    .all(|other| other.operation_id != command.operation_id),
                "{} operation id is unique",
                command.operation_id
            );
            assert_eq!(
                document["paths"][command.path][command.method.to_ascii_lowercase()]["operationId"],
                command.operation_id
            );
        }
    }

    #[test]
    fn openapi_document_derives_from_route_descriptors() {
        let document = auth_openapi_document();

        assert_eq!(document["openapi"], "3.1.0");
        assert_eq!(
            document["paths"]["/auth/sign-in/email"]["post"]["operationId"],
            "signInEmailPassword"
        );
        assert_eq!(
            document["paths"]["/auth/sign-in/email"]["post"]["x-architect-command"],
            "SignInEmailPassword"
        );
        assert_eq!(
            document["paths"]["/auth/sign-in/email"]["post"]["x-architect-plugin"],
            "email-password"
        );
        assert_eq!(
            document["paths"]["/auth/session"]["get"]["security"][0]["bearerAuth"],
            serde_json::json!([])
        );
        assert_eq!(
            document["paths"]["/auth/get-session"]["get"]["operationId"],
            "currentCustomSession"
        );
        assert_eq!(
            document["paths"]["/auth/get-session"]["get"]["x-architect-plugin"],
            "custom-session"
        );
        assert_eq!(
            document["paths"]["/auth/get-session"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/CustomSessionBundle_T"
        );
        assert_eq!(
            document["paths"]["/auth/additional-fields/schema"]["get"]["operationId"],
            "additionalFieldsSchema"
        );
        assert_eq!(
            document["paths"]["/auth/additional-fields/schema"]["get"]["responses"]["200"]["content"]
                ["application/json"]["schema"]["$ref"],
            "#/components/schemas/AdditionalFieldsSchema"
        );
        assert_eq!(
            document["paths"]["/auth/openapi.json"]["get"]["operationId"],
            "getOpenApiDocument"
        );
        assert_eq!(
            document["paths"]["/auth/openapi.json"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/OpenApiDocument"
        );
        assert_eq!(
            document["components"]["schemas"]["SignInEmailPassword"]["x-rust-type"],
            "SignInEmailPassword"
        );
        assert_eq!(
            document["components"]["schemas"]["AuthSessionBundle"]["x-rust-type"],
            "AuthSessionBundle"
        );
        assert_eq!(
            document["components"]["schemas"]["PublicAuthError"]["required"],
            serde_json::json!(["code", "message", "status"])
        );
        assert_eq!(
            document["components"]["securitySchemes"]["sessionCookie"]["name"],
            "architect-auth.session"
        );
        assert_eq!(
            document["paths"]["/auth/two-factor/verify"]["post"]["operationId"],
            "verifyTwoFactor"
        );
        assert_eq!(
            document["paths"]["/auth/two-factor/verify"]["post"]["x-architect-plugin"],
            "two-factor"
        );
        assert_eq!(
            document["paths"]["/auth/passkey/authenticate/complete"]["post"]["operationId"],
            "completePasskeyAuthentication"
        );
        assert_eq!(
            document["paths"]["/auth/passkey/authenticate/complete"]["post"]["x-architect-plugin"],
            "passkey"
        );
        assert_eq!(
            document["paths"]["/auth/device/token"]["post"]["operationId"],
            "pollDeviceToken"
        );
        assert_eq!(
            document["paths"]["/auth/device/token"]["post"]["x-architect-plugin"],
            "device-authorization"
        );
        assert_eq!(
            document["paths"]["/auth/sign-in/anonymous"]["post"]["operationId"],
            "signInAnonymous"
        );
        assert_eq!(
            document["paths"]["/auth/sign-in/anonymous"]["post"]["x-architect-plugin"],
            "anonymous"
        );
        assert_eq!(
            document["paths"]["/auth/bearer/authenticate"]["post"]["operationId"],
            "authenticateBearerToken"
        );
        assert_eq!(
            document["paths"]["/auth/bearer/authenticate"]["post"]["x-architect-plugin"],
            "bearer"
        );
        assert_eq!(
            document["paths"]["/auth/captcha/verify"]["post"]["operationId"],
            "verifyCaptcha"
        );
        assert_eq!(
            document["paths"]["/auth/captcha/verify"]["post"]["x-architect-plugin"],
            "captcha"
        );
        assert_eq!(
            document["paths"]["/auth/email-otp/verify"]["post"]["operationId"],
            "verifyEmailOtp"
        );
        assert_eq!(
            document["paths"]["/auth/email-otp/verify"]["post"]["x-architect-plugin"],
            "email-otp"
        );
        assert_eq!(
            document["paths"]["/auth/magic-link/verify"]["post"]["operationId"],
            "verifyMagicLink"
        );
        assert_eq!(
            document["paths"]["/auth/magic-link/verify"]["post"]["x-architect-plugin"],
            "magic-link"
        );
        assert_eq!(
            document["paths"]["/auth/jwt/issue"]["post"]["operationId"],
            "issueJwt"
        );
        assert_eq!(
            document["paths"]["/auth/jwt/issue"]["post"]["x-architect-plugin"],
            "jwt"
        );
    }

    // r[verify auth.openapi.plugin-aware]
    // r[verify auth.openapi.request-schemas]
    // r[verify auth.openapi.response-schemas]
    // r[verify auth.openapi.error-schemas]
    // r[verify auth.openapi.snapshot]
    #[test]
    fn openapi_document_snapshot_covers_plugins_paths_and_components() {
        let document = auth_openapi_document();
        let snapshot = serde_json::json!({
            "openapi": document["openapi"].clone(),
            "plugin_count": document["x-architect-enabled-plugins"].as_array().unwrap().len(),
            "path_count": document["paths"].as_object().unwrap().len(),
            "schema_count": document["components"]["schemas"].as_object().unwrap().len(),
            "selected_plugins": [
                document["paths"]["/auth/sign-up/email"]["post"]["x-architect-plugin"].clone(),
                document["paths"]["/auth/get-session"]["get"]["x-architect-plugin"].clone(),
                document["paths"]["/auth/additional-fields/schema"]["get"]["x-architect-plugin"].clone(),
                document["paths"]["/auth/openapi.json"]["get"]["x-architect-plugin"].clone(),
                document["paths"]["/auth/mcp/authorize"]["post"]["x-architect-plugin"].clone(),
            ],
            "selected_refs": {
                "signup_request": document["paths"]["/auth/sign-up/email"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"].clone(),
                "signup_response": document["paths"]["/auth/sign-up/email"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"].clone(),
                "error": document["paths"]["/auth/sign-up/email"]["post"]["responses"]["400"]["content"]["application/json"]["schema"]["$ref"].clone(),
                "openapi_response": document["paths"]["/auth/openapi.json"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"].clone(),
            },
            "security": document["paths"]["/auth/session"]["get"]["security"].clone(),
        });

        assert_eq!(
            snapshot,
            serde_json::json!({
                "openapi": "3.1.0",
                "plugin_count": 33,
                "path_count": 112,
                "schema_count": 167,
                "selected_plugins": [
                    "email-password",
                    "custom-session",
                    "additional-fields",
                    "open-api",
                    "mcp",
                ],
                "selected_refs": {
                    "signup_request": "#/components/schemas/CreateEmailPasswordUser",
                    "signup_response": "#/components/schemas/AuthSessionBundle",
                    "error": "#/components/schemas/PublicAuthError",
                    "openapi_response": "#/components/schemas/OpenApiDocument",
                },
                "security": [
                    { "bearerAuth": [] },
                    { "sessionCookie": [] },
                ],
            })
        );
    }

    #[test]
    fn session_cookie_applies_security_attributes() {
        let cookie = AuthCookieConfig {
            name: "sid".into(),
            secure: true,
            http_only: true,
            same_site: SameSitePolicy::Strict,
            domain: Some("example.com".into()),
            path: "/auth".into(),
            max_age_seconds: Some(60),
        }
        .session_cookie("secret-session");

        assert_eq!(cookie.name(), "sid");
        assert_eq!(cookie.value(), "secret-session");
        assert_eq!(cookie.domain(), Some("example.com"));
        assert_eq!(cookie.path(), Some("/auth"));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(cookie::SameSite::Strict));
        assert_eq!(cookie.max_age().map(|age| age.whole_seconds()), Some(60));
    }

    #[test]
    fn public_error_mapping_does_not_leak_internal_details() {
        let public = map_auth_error(&AuthFlowError::Internal("database password leaked".into()));

        assert_eq!(public.status, 500);
        assert_eq!(public.code, "internal");
        assert_eq!(public.message, "internal error");
    }

    // r[verify auth.errors.taxonomy]
    // r[verify auth.errors.axum]
    // r[verify auth.errors.vox]
    // r[verify auth.errors.openapi]
    // r[verify auth.errors.coverage]
    #[test]
    fn stable_error_taxonomy_covers_rust_axum_vox_and_openapi() {
        let cases = [
            (
                AuthFlowError::InvalidCredentials,
                "InvalidCredentials",
                "invalid_credentials",
                401,
                AuthVoxErrorStatus::Unauthenticated,
            ),
            (
                AuthFlowError::SessionExpired,
                "SessionExpired",
                "session_expired",
                401,
                AuthVoxErrorStatus::Unauthenticated,
            ),
            (
                AuthFlowError::VerificationRequired,
                "VerificationRequired",
                "verification_required",
                403,
                AuthVoxErrorStatus::PermissionDenied,
            ),
            (
                AuthFlowError::TwoFactorRequired,
                "TwoFactorRequired",
                "two_factor_required",
                403,
                AuthVoxErrorStatus::PermissionDenied,
            ),
            (
                AuthFlowError::PermissionDenied,
                "PermissionDenied",
                "permission_denied",
                403,
                AuthVoxErrorStatus::PermissionDenied,
            ),
            (
                AuthFlowError::InvalidInput("bad email".into()),
                "InvalidInput",
                "invalid_input",
                400,
                AuthVoxErrorStatus::InvalidArgument,
            ),
            (
                AuthFlowError::Internal("database password leaked".into()),
                "Internal",
                "internal",
                500,
                AuthVoxErrorStatus::Internal,
            ),
        ];

        assert_eq!(auth_error_taxonomy().len(), cases.len());
        for (error, variant, code, status, vox_status) in cases {
            let public = map_auth_error(&error);
            assert_eq!(public.code, code);
            assert_eq!(public.status, status);
            assert_eq!(map_auth_error_to_vox_status(&error), vox_status);
            assert!(
                auth_error_taxonomy()
                    .iter()
                    .any(|entry| entry.rust_variant == variant
                        && entry.code == code
                        && entry.http_status == status
                        && entry.vox_status == vox_status)
            );
        }

        let plugin_family_cases = [
            (
                "email-password",
                AuthFlowError::InvalidCredentials,
                "invalid_credentials",
            ),
            (
                "session-management",
                AuthFlowError::SessionExpired,
                "session_expired",
            ),
            (
                "email-verification",
                AuthFlowError::VerificationRequired,
                "verification_required",
            ),
            (
                "two-factor",
                AuthFlowError::TwoFactorRequired,
                "two_factor_required",
            ),
            (
                "organization",
                AuthFlowError::PermissionDenied,
                "permission_denied",
            ),
            (
                "additional-fields",
                AuthFlowError::InvalidInput("unknown field".into()),
                "invalid_input",
            ),
            (
                "open-api",
                AuthFlowError::Internal("schema generation failed".into()),
                "internal",
            ),
        ];
        for (plugin_id, error, code) in plugin_family_cases {
            assert!(
                auth_command_descriptors()
                    .iter()
                    .any(|command| command.plugin_id == plugin_id),
                "{plugin_id} should have representative command metadata"
            );
            assert_eq!(map_auth_error(&error).code, code);
        }

        let document = auth_openapi_document();
        assert_eq!(
            document["components"]["schemas"]["PublicAuthError"]["properties"]["code"]["enum"],
            serde_json::json!([
                "invalid_credentials",
                "session_expired",
                "verification_required",
                "two_factor_required",
                "permission_denied",
                "invalid_input",
                "internal",
            ])
        );
    }

    fn example_session_bundle() -> AuthSessionBundle {
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        AuthSessionBundle {
            user: AuthUser {
                id: user_id,
                email: Some("custom@example.com".into()),
                name: Some("Custom User".into()),
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
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            session: AuthSession {
                id: session_id,
                user_id,
                token_hash: "hash".into(),
                expires_at: Utc::now() + Duration::hours(1),
                ip_address: None,
                user_agent: None,
                impersonated_by: None,
                active_organization_id: None,
                active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            token: "session-token".into(),
        }
    }

    // r[verify auth.custom-session.serialization]
    #[test]
    fn custom_session_transport_preserves_stable_envelope_identifiers() {
        let bundle =
            crate::CustomSessionBundle::from_session_bundle(example_session_bundle(), "custom");
        let transport = CustomSessionTransport::from(&bundle);

        assert_eq!(transport.user_id, bundle.user.id);
        assert_eq!(transport.session_id, bundle.session.id);
        assert_eq!(transport.token, "session-token");
        assert_eq!(transport.custom, "custom");
    }

    #[cfg(feature = "axum")]
    // r[verify auth.custom-session.axum]
    #[test]
    fn axum_custom_session_extension_is_typed() {
        let bundle =
            crate::CustomSessionBundle::from_session_bundle(example_session_bundle(), 42u64);
        let mut request = ::axum::http::Request::builder()
            .body(::axum::body::Body::empty())
            .expect("request");

        super::axum::insert_custom_session_extension(&mut request, &bundle);
        let extension = request
            .extensions()
            .get::<super::axum::AuthenticatedCustomSession<u64>>()
            .expect("typed custom session extension");

        assert_eq!(extension.user_id, bundle.user.id);
        assert_eq!(extension.custom, 42);
    }

    #[cfg(feature = "vox")]
    // r[verify auth.custom-session.vox]
    #[test]
    fn vox_custom_session_extension_is_typed() {
        let bundle =
            crate::CustomSessionBundle::from_session_bundle(example_session_bundle(), 7u64);
        let extensions = ::vox::Extensions::new();

        super::vox::insert_custom_session_extension(&extensions, &bundle);
        let extension = extensions
            .get_cloned::<super::vox::AuthVoxCustomSession<u64>>()
            .expect("typed custom session extension");

        assert_eq!(extension.session_id, bundle.session.id);
        assert_eq!(extension.custom, 7);
    }

    // r[verify auth.transport.hooks-typed]
    #[test]
    fn hook_context_is_typed_by_command() {
        let command = CreateEmailPasswordUser {
            email: "user@example.com".into(),
            password: "correct horse battery staple".into(),
            name: None,
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: None,
        };
        let context = AuthHookContext {
            command,
            route: auth_route_descriptors().first().copied(),
        };

        assert_eq!(context.command.email, "user@example.com");
        assert_eq!(
            context.route.map(|route| route.command_type),
            Some("CreateEmailPasswordUser")
        );
    }

    #[cfg(feature = "axum")]
    #[test]
    fn axum_feature_exposes_architect_adapter_descriptors() {
        let routes = super::axum::routes();

        assert!(!routes.descriptors.is_empty());
        assert_eq!(
            routes.descriptors[0].command_type,
            "CreateEmailPasswordUser"
        );
    }

    #[cfg(feature = "axum")]
    #[test]
    fn axum_extracts_bearer_token_before_cookie() {
        let mut headers = ::axum::http::HeaderMap::new();
        headers.insert(
            ::axum::http::header::AUTHORIZATION,
            ::axum::http::HeaderValue::from_static("Bearer bearer-token"),
        );
        headers.insert(
            ::axum::http::header::COOKIE,
            ::axum::http::HeaderValue::from_static("architect-auth.session=cookie-token"),
        );

        let token = super::axum::session_token_from_headers(&headers, &AuthCookieConfig::default());

        assert_eq!(token.as_deref(), Some("bearer-token"));
    }

    proptest! {
        // r[verify auth.boundary.property-tests]
        #[test]
        fn bearer_authorization_parser_accepts_only_single_nonempty_token(token in "[A-Za-z0-9._~+/-]{1,96}") {
            let header = format!("Bearer {token}");
            let parsed = crate::flows::bearer_tokens::parse_authorization_header(Some(&header));
            prop_assert_eq!(parsed.as_deref(), Ok(token.as_str()));

            for malformed in [
                token.clone(),
                format!("bearer {token}"),
                format!("Bearer {token} suffix"),
                "Bearer ".to_owned(),
                format!("Bearer {token}\n"),
            ] {
                prop_assert!(
                    crate::flows::bearer_tokens::parse_authorization_header(Some(&malformed)).is_err()
                );
            }
        }
    }

    #[cfg(feature = "axum")]
    proptest! {
        // r[verify auth.boundary.property-tests]
        #[test]
        fn axum_cookie_parser_extracts_configured_cookie_token(
            token in "[A-Za-z0-9._~+/-]{1,96}",
            prefix in "[A-Za-z0-9_=-]{0,24}",
            suffix in "[A-Za-z0-9_=-]{0,24}",
        ) {
            let mut headers = ::axum::http::HeaderMap::new();
            let cookie = AuthCookieConfig {
                name: "sid".into(),
                ..AuthCookieConfig::default()
            };
            let cookie_header = format!("other={prefix}; sid={token}; trailing={suffix}");
            headers.insert(
                ::axum::http::header::COOKIE,
                ::axum::http::HeaderValue::from_str(&cookie_header).expect("valid cookie header"),
            );

            let parsed = super::axum::session_token_from_headers(&headers, &cookie);
            prop_assert_eq!(parsed.as_deref(), Some(token.as_str()));
        }
    }

    #[cfg(feature = "axum")]
    #[test]
    fn axum_extracts_configured_session_cookie() {
        let mut headers = ::axum::http::HeaderMap::new();
        headers.insert(
            ::axum::http::header::COOKIE,
            ::axum::http::HeaderValue::from_static("other=1; sid=cookie-token"),
        );
        let cookie = AuthCookieConfig {
            name: "sid".into(),
            ..AuthCookieConfig::default()
        };

        let token = super::axum::session_token_from_headers(&headers, &cookie);

        assert_eq!(token.as_deref(), Some("cookie-token"));
    }

    #[cfg(feature = "axum")]
    #[test]
    fn axum_rejects_malformed_bearer_token() {
        let mut headers = ::axum::http::HeaderMap::new();
        headers.insert(
            ::axum::http::header::AUTHORIZATION,
            ::axum::http::HeaderValue::from_static("Bearer has space"),
        );

        let token = super::axum::session_token_from_headers(&headers, &AuthCookieConfig::default());

        assert_eq!(token, None);
    }

    #[cfg(feature = "axum")]
    #[test]
    fn axum_error_uses_public_mapping() {
        let error = super::axum::AxumAuthError::from(AuthFlowError::Internal(
            "database password leaked".into(),
        ));

        assert_eq!(error.public.status, 500);
        assert_eq!(error.public.message, "internal error");
    }

    #[cfg(feature = "vox")]
    #[test]
    fn vox_extracts_bearer_metadata() {
        let metadata = ::vox::metadata()
            .str("authorization", "Bearer session-token")
            .build();

        let token = super::vox::authorization_token_from_metadata(&metadata);

        assert_eq!(token.as_deref(), Some("session-token"));
    }

    #[cfg(feature = "vox")]
    #[test]
    fn vox_ignores_non_bearer_metadata() {
        let metadata = ::vox::metadata().str("authorization", "Basic no").build();

        let token = super::vox::authorization_token_from_metadata(&metadata);

        assert_eq!(token, None);
    }
}
