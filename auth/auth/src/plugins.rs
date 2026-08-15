use crate::transport::{AuthRouteDescriptor, auth_route_descriptors};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthPluginDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub upstream: &'static str,
    pub feature: Option<&'static str>,
    pub dependencies: &'static [&'static str],
    pub capabilities: &'static [&'static str],
    pub command_ids: &'static [&'static str],
}

// r[impl auth.storage.plugin-migrations]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthPluginStorageRequirement {
    pub plugin_id: &'static str,
    pub tables: &'static [&'static str],
    pub indexes: &'static [&'static str],
    pub migrations: &'static [&'static str],
}

impl AuthPluginDescriptor {
    pub const fn command_count(&self) -> usize {
        self.command_ids.len()
    }
}

const USERS: &[&str] = &["auth_users"];
const SESSIONS: &[&str] = &["auth_sessions"];
const ACCOUNTS: &[&str] = &["auth_accounts"];
const VERIFICATIONS: &[&str] = &["auth_verifications"];
const API_KEYS: &[&str] = &["auth_api_keys"];
const TWO_FACTORS: &[&str] = &["auth_two_factors"];
const PASSKEYS: &[&str] = &["auth_passkeys"];
const ORGANIZATION_TABLES: &[&str] = &[
    "auth_organizations",
    "auth_members",
    "auth_invitations",
    "auth_organization_roles",
    "auth_teams",
    "auth_team_members",
];
const USERS_SESSIONS_ACCOUNTS: &[&str] = &["auth_users", "auth_sessions", "auth_accounts"];
const USERS_SESSIONS_VERIFICATIONS: &[&str] =
    &["auth_users", "auth_sessions", "auth_verifications"];
const USERS_SESSIONS_ACCOUNTS_VERIFICATIONS: &[&str] = &[
    "auth_users",
    "auth_sessions",
    "auth_accounts",
    "auth_verifications",
];
const MIGRATION_V1: &[&str] = &["m20260513_000001_create_auth_tables"];
const NO_STORAGE: &[&str] = &[];

// r[impl auth.email.plugin-descriptor]
// r[impl auth.email.plugin-routes]
pub const EMAIL_PASSWORD_COMMANDS: &[&str] = &["createEmailPasswordUser", "signInEmailPassword"];

pub fn email_password_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&EMAIL_PASSWORD_PLUGIN)
}

// r[impl auth.email.plugin-descriptor]
pub const EMAIL_PASSWORD_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "email-password",
    display_name: "Email/password",
    upstream: "better-auth-rs:email_password; better-auth:email-password",
    feature: None,
    dependencies: &["sessions", "accounts"],
    capabilities: &["signup", "signin", "password-hashing", "session-creation"],
    command_ids: EMAIL_PASSWORD_COMMANDS,
};

// r[impl auth.anonymous.plugin-descriptor]
// r[impl auth.anonymous.plugin-routes]
pub const ANONYMOUS_COMMANDS: &[&str] = &[
    "signInAnonymous",
    "linkAnonymousEmailPassword",
    "cleanupAnonymousUsers",
];

pub fn anonymous_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ANONYMOUS_PLUGIN)
}

// r[impl auth.anonymous.plugin-descriptor]
pub const ANONYMOUS_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "anonymous",
    display_name: "Anonymous users",
    upstream: "better-auth-rs:anonymous; better-auth:anonymous",
    feature: None,
    dependencies: &["users", "sessions", "email-password"],
    capabilities: &[
        "anonymous-session",
        "account-upgrade",
        "email-password-link",
        "policy-role",
        "cleanup",
        "session-revocation",
    ],
    command_ids: ANONYMOUS_COMMANDS,
};

// r[impl auth.sessions.plugin-descriptor]
// r[impl auth.sessions.plugin-routes]
pub const SESSION_MANAGEMENT_COMMANDS: &[&str] = &[
    "currentSession",
    "signOut",
    "listSessions",
    "revokeSession",
    "revokeOtherSessions",
];

pub fn session_management_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&SESSION_MANAGEMENT_PLUGIN)
}

// r[impl auth.sessions.plugin-descriptor]
pub const SESSION_MANAGEMENT_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "session-management",
    display_name: "Session management",
    upstream: "better-auth-rs:session_management; better-auth:session",
    feature: None,
    dependencies: &["sessions"],
    capabilities: &[
        "get-session",
        "sign-out",
        "list-sessions",
        "revoke-session",
        "revoke-other-sessions",
    ],
    command_ids: SESSION_MANAGEMENT_COMMANDS,
};

// r[impl auth.custom-session.plugin-descriptor]
// r[impl auth.custom-session.plugin-routes]
pub const CUSTOM_SESSION_COMMANDS: &[&str] = &["currentCustomSession"];

pub fn custom_session_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&CUSTOM_SESSION_PLUGIN)
}

// r[impl auth.custom-session.plugin-descriptor]
pub const CUSTOM_SESSION_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "custom-session",
    display_name: "Custom session",
    upstream: "better-auth:custom-session",
    feature: None,
    dependencies: &["session-management"],
    capabilities: &[
        "typed-enrichment-hook",
        "axum-extension",
        "vox-extension",
        "schema-extension",
        "backwards-compatible-session",
    ],
    command_ids: CUSTOM_SESSION_COMMANDS,
};

// r[impl auth.additional-fields.plugin-descriptor]
// r[impl auth.additional-fields.plugin-routes]
pub const ADDITIONAL_FIELDS_COMMANDS: &[&str] = &["additionalFieldsSchema"];

pub fn additional_fields_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ADDITIONAL_FIELDS_PLUGIN)
}

// r[impl auth.additional-fields.plugin-descriptor]
pub const ADDITIONAL_FIELDS_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "additional-fields",
    display_name: "Additional fields",
    upstream: "better-auth:additional-fields; better-auth:hide-metadata",
    feature: None,
    dependencies: &["users", "sessions", "accounts"],
    capabilities: &[
        "typed-field-spec",
        "metadata-persistence",
        "returned-field-filter",
        "hidden-metadata",
        "schema-metadata",
        "migration-contract",
    ],
    command_ids: ADDITIONAL_FIELDS_COMMANDS,
};

// r[impl auth.openapi.plugin-descriptor]
// r[impl auth.openapi.plugin-routes]
pub const OPEN_API_COMMANDS: &[&str] = &["getOpenApiDocument"];

pub fn open_api_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&OPEN_API_PLUGIN)
}

// r[impl auth.openapi.plugin-descriptor]
pub const OPEN_API_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "open-api",
    display_name: "OpenAPI",
    upstream: "better-auth:open-api",
    feature: None,
    dependencies: &[],
    capabilities: &[
        "plugin-aware-document",
        "request-schemas",
        "response-schemas",
        "error-schemas",
        "snapshot-tested-output",
    ],
    command_ids: OPEN_API_COMMANDS,
};

// r[impl auth.password.plugin-descriptor]
// r[impl auth.password.plugin-routes]
pub const PASSWORD_MANAGEMENT_COMMANDS: &[&str] = &[
    "changePassword",
    "requestPasswordReset",
    "completePasswordReset",
];

pub fn password_management_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&PASSWORD_MANAGEMENT_PLUGIN)
}

// r[impl auth.password.plugin-descriptor]
pub const PASSWORD_MANAGEMENT_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "password-management",
    display_name: "Password management",
    upstream: "better-auth-rs:password_management; better-auth:password",
    feature: None,
    dependencies: &["email-password", "sessions", "verification-tokens"],
    capabilities: &[
        "change-password",
        "request-password-reset",
        "complete-password-reset",
    ],
    command_ids: PASSWORD_MANAGEMENT_COMMANDS,
};

// r[impl auth.verify.plugin-descriptor]
// r[impl auth.verify.plugin-routes]
pub const EMAIL_VERIFICATION_COMMANDS: &[&str] = &["requestEmailVerification", "verifyEmail"];

pub fn email_verification_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&EMAIL_VERIFICATION_PLUGIN)
}

// r[impl auth.verify.plugin-descriptor]
pub const EMAIL_VERIFICATION_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "email-verification",
    display_name: "Email verification",
    upstream: "better-auth-rs:email_verification; better-auth:email-verification",
    feature: None,
    dependencies: &["users", "verification-tokens"],
    capabilities: &["send-verification-email", "verify-email"],
    command_ids: EMAIL_VERIFICATION_COMMANDS,
};

// r[impl auth.user.plugin-descriptor]
// r[impl auth.user.plugin-routes]
pub const USER_MANAGEMENT_COMMANDS: &[&str] = &["changeEmail", "deleteUser"];

pub fn user_management_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&USER_MANAGEMENT_PLUGIN)
}

// r[impl auth.user.plugin-descriptor]
pub const USER_MANAGEMENT_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "user-management",
    display_name: "User management",
    upstream: "better-auth-rs:user_management; better-auth:user-management",
    feature: None,
    dependencies: &["users", "sessions", "email-verification"],
    capabilities: &["change-email", "delete-user"],
    command_ids: USER_MANAGEMENT_COMMANDS,
};

// r[impl auth.account.plugin-descriptor]
// r[impl auth.account.plugin-routes]
pub const ACCOUNT_MANAGEMENT_COMMANDS: &[&str] =
    &["listAccounts", "linkOAuthAccount", "unlinkOAuthAccount"];

pub fn account_management_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ACCOUNT_MANAGEMENT_PLUGIN)
}

// r[impl auth.account.plugin-descriptor]
pub const ACCOUNT_MANAGEMENT_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "account-management",
    display_name: "Account management",
    upstream: "better-auth-rs:account_management; better-auth:account-management",
    feature: None,
    dependencies: &["sessions", "accounts", "oauth"],
    capabilities: &["list-accounts", "link-account", "unlink-account"],
    command_ids: ACCOUNT_MANAGEMENT_COMMANDS,
};

// r[impl auth.oauth.plugin-descriptor]
// r[impl auth.oauth.plugin-routes]
pub const OAUTH_COMMANDS: &[&str] = &[
    "beginOAuthAuthorization",
    "verifyOAuthState",
    "signInOAuthAccount",
    "getOAuthAccessToken",
    "refreshOAuthToken",
    "linkOAuthAccount",
    "unlinkOAuthAccount",
];

pub fn oauth_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&OAUTH_PLUGIN)
}

// r[impl auth.oauth.plugin-descriptor]
pub const OAUTH_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "oauth",
    display_name: "OAuth and generic OAuth",
    upstream: "better-auth-rs:oauth; better-auth:oauth/generic-oauth",
    feature: None,
    dependencies: &["users", "sessions", "accounts", "verification-tokens"],
    capabilities: &[
        "social-sign-in",
        "callback-state",
        "account-linking",
        "account-unlinking",
        "access-token",
        "refresh-token",
        "provider-registry",
        "generic-provider",
        "token-encryption",
    ],
    command_ids: OAUTH_COMMANDS,
};

// r[impl auth.oauth-proxy.plugin-descriptor]
// r[impl auth.oauth-proxy.plugin-routes]
pub const OAUTH_PROXY_COMMANDS: &[&str] = &[
    "getOAuthProxyMetadata",
    "beginOAuthProxyAuthorization",
    "forwardOAuthProxyCallback",
    "consumeOAuthProxyCallback",
];

pub fn oauth_proxy_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&OAUTH_PROXY_PLUGIN)
}

// r[impl auth.oauth-proxy.plugin-descriptor]
pub const OAUTH_PROXY_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "oauth-proxy",
    display_name: "OAuth proxy",
    upstream: "better-auth:oauth-proxy",
    feature: None,
    dependencies: &["oauth"],
    capabilities: &[
        "proxy-state",
        "callback-forwarding",
        "encrypted-profile",
        "redirect-policy",
        "provider-metadata",
        "max-age",
    ],
    command_ids: OAUTH_PROXY_COMMANDS,
};

// r[impl auth.onetap.plugin-descriptor]
// r[impl auth.onetap.plugin-routes]
pub const ONE_TAP_COMMANDS: &[&str] = &["oneTapCallback"];

pub fn one_tap_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ONE_TAP_PLUGIN)
}

// r[impl auth.onetap.plugin-descriptor]
pub const ONE_TAP_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "one-tap",
    display_name: "One Tap",
    upstream: "better-auth:one-tap",
    feature: None,
    dependencies: &["users", "sessions", "accounts"],
    capabilities: &[
        "google-id-token",
        "token-validation",
        "existing-account",
        "implicit-linking-gate",
        "auto-signup",
        "session-creation",
    ],
    command_ids: ONE_TAP_COMMANDS,
};

// r[impl auth.ott.plugin-descriptor]
// r[impl auth.ott.plugin-routes]
pub const ONE_TIME_TOKEN_COMMANDS: &[&str] = &[
    "generateOneTimeToken",
    "verifyOneTimeToken",
    "revokeOneTimeToken",
];

pub fn one_time_token_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ONE_TIME_TOKEN_PLUGIN)
}

// r[impl auth.ott.plugin-descriptor]
pub const ONE_TIME_TOKEN_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "one-time-token",
    display_name: "One-time token",
    upstream: "better-auth:one-time-token",
    feature: None,
    dependencies: &["sessions", "verification-tokens"],
    capabilities: &["create", "consume", "expire", "revoke", "scope", "metadata"],
    command_ids: ONE_TIME_TOKEN_COMMANDS,
};

// r[impl auth.multisession.plugin-descriptor]
// r[impl auth.multisession.plugin-routes]
pub const MULTI_SESSION_COMMANDS: &[&str] = &[
    "listDeviceSessions",
    "setActiveDeviceSession",
    "revokeDeviceSession",
];

pub fn multi_session_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&MULTI_SESSION_PLUGIN)
}

// r[impl auth.multisession.plugin-descriptor]
pub const MULTI_SESSION_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "multi-session",
    display_name: "Multi-session",
    upstream: "better-auth:multi-session",
    feature: None,
    dependencies: &["sessions"],
    capabilities: &[
        "device-session-list",
        "active-session-selection",
        "session-revocation",
        "forgery-rejection",
        "permission-isolation",
    ],
    command_ids: MULTI_SESSION_COMMANDS,
};

// r[impl auth.lastlogin.plugin-descriptor]
// r[impl auth.lastlogin.plugin-routes]
pub const LAST_LOGIN_METHOD_COMMANDS: &[&str] = &["getLastLoginMethod", "clearLastLoginMethod"];

pub fn last_login_method_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&LAST_LOGIN_METHOD_PLUGIN)
}

// r[impl auth.lastlogin.plugin-descriptor]
pub const LAST_LOGIN_METHOD_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "last-login-method",
    display_name: "Last login method",
    upstream: "better-auth:last-login-method",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "method-tracking",
        "client-readable-cookie",
        "metadata-persistence",
        "query-last-method",
        "clear-last-method",
    ],
    command_ids: LAST_LOGIN_METHOD_COMMANDS,
};

// r[impl auth.username.plugin-descriptor]
// r[impl auth.username.plugin-routes]
pub const USERNAME_COMMANDS: &[&str] = &["signInUsername", "updateUsername"];

pub fn username_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&USERNAME_PLUGIN)
}

// r[impl auth.username.plugin-descriptor]
pub const USERNAME_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "username",
    display_name: "Username",
    upstream: "better-auth:username",
    feature: None,
    dependencies: &["users", "sessions", "email-password"],
    capabilities: &[
        "username-validation",
        "case-insensitive-lookup",
        "reserved-name-policy",
        "username-sign-in",
        "username-update",
    ],
    command_ids: USERNAME_COMMANDS,
};

// r[impl auth.phone.plugin-descriptor]
// r[impl auth.phone.plugin-routes]
pub const PHONE_NUMBER_COMMANDS: &[&str] = &[
    "sendPhoneNumberOtp",
    "verifyPhoneNumberOtp",
    "updatePhoneNumber",
];

pub fn phone_number_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&PHONE_NUMBER_PLUGIN)
}

// r[impl auth.phone.plugin-descriptor]
pub const PHONE_NUMBER_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "phone-number",
    display_name: "Phone number",
    upstream: "better-auth:phone-number",
    feature: None,
    dependencies: &["users", "sessions", "verification-tokens"],
    capabilities: &[
        "phone-schema",
        "sms-provider",
        "send-code",
        "verify-code",
        "update-phone-number",
        "phone-sign-in",
    ],
    command_ids: PHONE_NUMBER_COMMANDS,
};

// r[impl auth.siwe.plugin-descriptor]
// r[impl auth.siwe.plugin-routes]
pub const SIWE_COMMANDS: &[&str] = &["createSiweNonce", "verifySiweMessage", "linkSiweAddress"];

pub fn siwe_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&SIWE_PLUGIN)
}

// r[impl auth.siwe.plugin-descriptor]
pub const SIWE_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "siwe",
    display_name: "Sign-In with Ethereum",
    upstream: "better-auth:siwe",
    feature: None,
    dependencies: &["users", "sessions", "accounts", "verification-tokens"],
    capabilities: &[
        "nonce",
        "message-verification",
        "domain-check",
        "replay-protection",
        "address-linking",
        "session-creation",
    ],
    command_ids: SIWE_COMMANDS,
};

// r[impl auth.hibp.plugin-descriptor]
// r[impl auth.hibp.plugin-routes]
pub const HAVEIBEENPWNED_COMMANDS: &[&str] = &["checkPasswordBreach"];

pub fn haveibeenpwned_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&HAVEIBEENPWNED_PLUGIN)
}

// r[impl auth.hibp.plugin-descriptor]
pub const HAVEIBEENPWNED_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "haveibeenpwned",
    display_name: "HaveIBeenPwned",
    upstream: "better-auth:haveIBeenPwned",
    feature: None,
    dependencies: &["email-password"],
    capabilities: &[
        "k-anonymity-range",
        "breached-password-rejection",
        "failure-policy",
        "test-provider",
        "password-flow-hooks",
    ],
    command_ids: HAVEIBEENPWNED_COMMANDS,
};

// r[impl auth.mcp.plugin-descriptor]
// r[impl auth.mcp.plugin-routes]
pub const MCP_COMMANDS: &[&str] = &["authorizeMcpRequest"];

pub fn mcp_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&MCP_PLUGIN)
}

// r[impl auth.mcp.plugin-descriptor]
pub const MCP_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "mcp",
    display_name: "MCP authorization",
    upstream: "better-auth:mcp",
    feature: None,
    dependencies: &["sessions", "organization"],
    capabilities: &[
        "session-validation",
        "api-key-token",
        "service-permission-check",
        "organization-action-policy",
    ],
    command_ids: MCP_COMMANDS,
};

// r[impl auth.apikey.plugin-descriptor]
// r[impl auth.apikey.plugin-routes]
pub const API_KEY_COMMANDS: &[&str] = &[
    "createApiKey",
    "getApiKey",
    "listApiKeys",
    "updateApiKey",
    "deleteApiKey",
    "verifyApiKey",
];

pub fn api_key_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&API_KEY_PLUGIN)
}

// r[impl auth.apikey.plugin-descriptor]
pub const API_KEY_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "api-key",
    display_name: "API key",
    upstream: "better-auth-rs:api_key; better-auth:api-key",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "create",
        "get",
        "list",
        "update",
        "delete",
        "verify",
        "permissions",
        "rate-limit",
    ],
    command_ids: API_KEY_COMMANDS,
};

// r[impl auth.bearer.plugin-descriptor]
// r[impl auth.bearer.plugin-routes]
pub const BEARER_COMMANDS: &[&str] = &["authenticateBearerToken"];

pub fn bearer_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&BEARER_PLUGIN)
}

// r[impl auth.bearer.plugin-descriptor]
pub const BEARER_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "bearer",
    display_name: "Bearer token",
    upstream: "better-auth-rs:bearer; better-auth:bearer",
    feature: None,
    dependencies: &["sessions", "api-key"],
    capabilities: &[
        "authorization-header",
        "session-token",
        "api-key-token",
        "axum-middleware",
        "vox-metadata",
        "stable-errors",
    ],
    command_ids: BEARER_COMMANDS,
};

// r[impl auth.captcha.plugin-descriptor]
// r[impl auth.captcha.plugin-routes]
pub const CAPTCHA_COMMANDS: &[&str] = &["verifyCaptcha"];

pub fn captcha_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&CAPTCHA_PLUGIN)
}

// r[impl auth.captcha.plugin-descriptor]
pub const CAPTCHA_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "captcha",
    display_name: "CAPTCHA",
    upstream: "better-auth-rs:captcha; better-auth:captcha",
    feature: None,
    dependencies: &["email-password", "verification-tokens"],
    capabilities: &[
        "provider-config",
        "test-provider",
        "bypass-mode",
        "fail-closed",
        "signup-hook",
        "stable-errors",
    ],
    command_ids: CAPTCHA_COMMANDS,
};

// r[impl auth.emailotp.plugin-descriptor]
// r[impl auth.emailotp.plugin-routes]
pub const EMAIL_OTP_COMMANDS: &[&str] = &["sendEmailOtp", "verifyEmailOtp"];

pub fn email_otp_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&EMAIL_OTP_PLUGIN)
}

// r[impl auth.emailotp.plugin-descriptor]
pub const EMAIL_OTP_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "email-otp",
    display_name: "Email OTP",
    upstream: "better-auth-rs:email_otp; better-auth:email-otp",
    feature: None,
    dependencies: &["users", "sessions", "verification-tokens"],
    capabilities: &[
        "send-otp",
        "verify-otp",
        "test-sink",
        "expiration",
        "resend-rate-limit",
        "single-use",
        "session-creation",
    ],
    command_ids: EMAIL_OTP_COMMANDS,
};

// r[impl auth.magic.plugin-descriptor]
// r[impl auth.magic.plugin-routes]
pub const MAGIC_LINK_COMMANDS: &[&str] = &["sendMagicLink", "verifyMagicLink"];

pub fn magic_link_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&MAGIC_LINK_PLUGIN)
}

// r[impl auth.magic.plugin-descriptor]
pub const MAGIC_LINK_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "magic-link",
    display_name: "Magic link",
    upstream: "better-auth-rs:magic_link; better-auth:magic-link",
    feature: None,
    dependencies: &["users", "sessions", "verification-tokens"],
    capabilities: &[
        "link-generation",
        "test-sink",
        "token-hash-storage",
        "expiration",
        "single-use",
        "session-creation",
        "redirect-trust",
    ],
    command_ids: MAGIC_LINK_COMMANDS,
};

// r[impl auth.jwt.plugin-descriptor]
// r[impl auth.jwt.plugin-routes]
pub const JWT_COMMANDS: &[&str] = &["issueJwt", "verifyJwt", "getJwtKeySet"];

pub fn jwt_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&JWT_PLUGIN)
}

// r[impl auth.jwt.plugin-descriptor]
pub const JWT_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "jwt",
    display_name: "JWT",
    upstream: "better-auth-rs:jwt; better-auth:jwt",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "hs256-signing",
        "verification",
        "issuer-audience",
        "claims-json",
        "session-backed",
        "rotation",
        "jwks-metadata",
    ],
    command_ids: JWT_COMMANDS,
};

// r[impl auth.oidc.plugin-descriptor]
// r[impl auth.oidc.plugin-routes]
pub const OIDC_PROVIDER_COMMANDS: &[&str] = &[
    "getOidcDiscovery",
    "registerOidcClient",
    "authorizeOidc",
    "exchangeOidcToken",
    "getOidcUserInfo",
];

pub fn oidc_provider_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&OIDC_PROVIDER_PLUGIN)
}

// r[impl auth.oidc.plugin-descriptor]
pub const OIDC_PROVIDER_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "oidc-provider",
    display_name: "OIDC provider",
    upstream: "better-auth:oidc-provider",
    feature: None,
    dependencies: &["users", "sessions", "jwt"],
    capabilities: &[
        "discovery",
        "trusted-clients",
        "dynamic-client-registration",
        "authorization-code",
        "pkce-s256",
        "token-endpoint",
        "refresh-token",
        "userinfo",
        "jwks",
    ],
    command_ids: OIDC_PROVIDER_COMMANDS,
};

// r[impl auth.twofactor.plugin-descriptor]
// r[impl auth.twofactor.plugin-routes]
pub const TWO_FACTOR_COMMANDS: &[&str] = &[
    "startTwoFactorSetup",
    "confirmTwoFactor",
    "verifyTwoFactor",
    "disableTwoFactor",
];

pub fn two_factor_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&TWO_FACTOR_PLUGIN)
}

// r[impl auth.twofactor.plugin-descriptor]
pub const TWO_FACTOR_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "two-factor",
    display_name: "Two-factor authentication",
    upstream: "better-auth-rs:two_factor; better-auth:two-factor",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "totp-setup",
        "totp-confirm",
        "pending-session",
        "backup-codes",
        "single-use-recovery",
        "rate-limit",
        "secret-encryption",
    ],
    command_ids: TWO_FACTOR_COMMANDS,
};

// r[impl auth.passkey.plugin-descriptor]
// r[impl auth.passkey.plugin-routes]
pub const PASSKEY_COMMANDS: &[&str] = &[
    "beginPasskeyRegistration",
    "completePasskeyRegistration",
    "beginPasskeyAuthentication",
    "completePasskeyAuthentication",
    "listPasskeys",
    "deletePasskey",
];

pub fn passkey_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&PASSKEY_PLUGIN)
}

// r[impl auth.passkey.plugin-descriptor]
pub const PASSKEY_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "passkey",
    display_name: "Passkeys and WebAuthn",
    upstream: "better-auth-rs:passkey; better-auth:passkey",
    feature: None,
    dependencies: &["users", "sessions", "verification-tokens"],
    capabilities: &[
        "registration-options",
        "registration-verification",
        "authentication-options",
        "authentication-verification",
        "list",
        "delete",
        "rp-origin-validation",
        "challenge-lifecycle",
        "credential-counters",
    ],
    command_ids: PASSKEY_COMMANDS,
};

// r[impl auth.device.plugin-descriptor]
// r[impl auth.device.plugin-routes]
pub const DEVICE_AUTHORIZATION_COMMANDS: &[&str] = &[
    "createDeviceAuthorization",
    "verifyDeviceCode",
    "approveDeviceCode",
    "denyDeviceCode",
    "pollDeviceToken",
];

pub fn device_authorization_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&DEVICE_AUTHORIZATION_PLUGIN)
}

// r[impl auth.device.plugin-descriptor]
pub const DEVICE_AUTHORIZATION_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "device-authorization",
    display_name: "Device authorization",
    upstream: "better-auth-rs:device_authorization; better-auth:device-authorization",
    feature: None,
    dependencies: &["users", "sessions", "verification-tokens"],
    capabilities: &[
        "device-code",
        "user-code",
        "verification-ui",
        "polling",
        "slow-down",
        "approval",
        "denial",
        "expiry",
    ],
    command_ids: DEVICE_AUTHORIZATION_COMMANDS,
};

// r[impl auth.org.plugin-descriptor]
// r[impl auth.org.plugin-routes]
pub const ORGANIZATION_COMMANDS: &[&str] = &[
    "createOrganization",
    "setActiveOrganization",
    "requireOrganizationRole",
    "authorizeOrganizationAction",
    "setMemberRole",
    "createInvitation",
    "acceptInvitation",
    "createOrganizationRole",
    "updateOrganizationRole",
    "deleteOrganizationRole",
    "listOrganizationRoles",
    "createTeam",
    "updateTeam",
    "deleteTeam",
    "listTeams",
    "addTeamMember",
    "removeTeamMember",
    "listTeamMembers",
];

pub fn organization_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ORGANIZATION_PLUGIN)
}

// r[impl auth.org.plugin-descriptor]
pub const ORGANIZATION_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "organization",
    display_name: "Organization, teams, and access control",
    upstream: "better-auth-rs:organization; better-auth:organization/access",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "organizations",
        "members",
        "invitations",
        "roles",
        "permissions",
        "teams",
    ],
    command_ids: ORGANIZATION_COMMANDS,
};

// r[impl auth.admin.plugin-descriptor]
// r[impl auth.admin.plugin-routes]
pub const ADMIN_COMMANDS: &[&str] = &[
    "listUsers",
    "adminCreateUser",
    "setUserRole",
    "adminSetUserPassword",
    "banUser",
    "unbanUser",
    "listUserSessions",
    "revokeUserSession",
    "revokeUserSessions",
    "impersonateUser",
    "stopImpersonating",
    "removeUser",
    "adminHasPermission",
];

pub fn admin_routes() -> Vec<&'static AuthRouteDescriptor> {
    plugin_routes(&ADMIN_PLUGIN)
}

// r[impl auth.admin.plugin-descriptor]
pub const ADMIN_PLUGIN: AuthPluginDescriptor = AuthPluginDescriptor {
    id: "admin",
    display_name: "Admin",
    upstream: "better-auth-rs:admin; better-auth:admin",
    feature: None,
    dependencies: &["users", "sessions"],
    capabilities: &[
        "list-users",
        "create-user",
        "set-role",
        "set-user-password",
        "ban-user",
        "unban-user",
        "list-user-sessions",
        "revoke-user-session",
        "revoke-user-sessions",
        "impersonate-user",
        "stop-impersonating",
        "remove-user",
        "has-permission",
        "audit-events",
    ],
    command_ids: ADMIN_COMMANDS,
};

pub const AUTH_PLUGIN_DESCRIPTORS: &[AuthPluginDescriptor] = &[
    EMAIL_PASSWORD_PLUGIN,
    ANONYMOUS_PLUGIN,
    SESSION_MANAGEMENT_PLUGIN,
    CUSTOM_SESSION_PLUGIN,
    ADDITIONAL_FIELDS_PLUGIN,
    OPEN_API_PLUGIN,
    PASSWORD_MANAGEMENT_PLUGIN,
    EMAIL_VERIFICATION_PLUGIN,
    USER_MANAGEMENT_PLUGIN,
    ACCOUNT_MANAGEMENT_PLUGIN,
    OAUTH_PLUGIN,
    OAUTH_PROXY_PLUGIN,
    ONE_TAP_PLUGIN,
    ONE_TIME_TOKEN_PLUGIN,
    MULTI_SESSION_PLUGIN,
    LAST_LOGIN_METHOD_PLUGIN,
    USERNAME_PLUGIN,
    PHONE_NUMBER_PLUGIN,
    SIWE_PLUGIN,
    HAVEIBEENPWNED_PLUGIN,
    MCP_PLUGIN,
    API_KEY_PLUGIN,
    BEARER_PLUGIN,
    CAPTCHA_PLUGIN,
    EMAIL_OTP_PLUGIN,
    MAGIC_LINK_PLUGIN,
    JWT_PLUGIN,
    OIDC_PROVIDER_PLUGIN,
    PASSKEY_PLUGIN,
    DEVICE_AUTHORIZATION_PLUGIN,
    TWO_FACTOR_PLUGIN,
    ORGANIZATION_PLUGIN,
    ADMIN_PLUGIN,
];

// r[impl auth.storage.plugin-migrations]
pub const AUTH_PLUGIN_STORAGE_REQUIREMENTS: &[AuthPluginStorageRequirement] = &[
    AuthPluginStorageRequirement {
        plugin_id: "email-password",
        tables: USERS_SESSIONS_ACCOUNTS,
        indexes: &[
            "idx_auth_users_email",
            "idx_auth_accounts_provider_account",
            "idx_auth_sessions_token_hash",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "anonymous",
        tables: USERS_SESSIONS_ACCOUNTS,
        indexes: &["idx_auth_users_email", "idx_auth_sessions_token_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "session-management",
        tables: SESSIONS,
        indexes: &[
            "idx_auth_sessions_user_id",
            "idx_auth_sessions_token_hash",
            "idx_auth_sessions_expires_at",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "custom-session",
        tables: SESSIONS,
        indexes: &["idx_auth_sessions_token_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "additional-fields",
        tables: USERS_SESSIONS_ACCOUNTS,
        indexes: &["idx_auth_users_email"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "open-api",
        tables: NO_STORAGE,
        indexes: NO_STORAGE,
        migrations: NO_STORAGE,
    },
    AuthPluginStorageRequirement {
        plugin_id: "password-management",
        tables: USERS_SESSIONS_ACCOUNTS_VERIFICATIONS,
        indexes: &[
            "idx_auth_accounts_user_id",
            "idx_auth_verifications_identifier",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "email-verification",
        tables: VERIFICATIONS,
        indexes: &[
            "idx_auth_verifications_identifier",
            "idx_auth_verifications_expires_at",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "user-management",
        tables: USERS,
        indexes: &["idx_auth_users_email"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "account-management",
        tables: ACCOUNTS,
        indexes: &[
            "idx_auth_accounts_provider_account",
            "idx_auth_accounts_user_id",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "oauth",
        tables: USERS_SESSIONS_ACCOUNTS_VERIFICATIONS,
        indexes: &[
            "idx_auth_accounts_provider_account",
            "idx_auth_sessions_token_hash",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "oauth-proxy",
        tables: VERIFICATIONS,
        indexes: &["idx_auth_verifications_identifier"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "one-tap",
        tables: USERS_SESSIONS_ACCOUNTS,
        indexes: &["idx_auth_accounts_provider_account"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "one-time-token",
        tables: VERIFICATIONS,
        indexes: &[
            "idx_auth_verifications_identifier",
            "idx_auth_verifications_expires_at",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "multi-session",
        tables: SESSIONS,
        indexes: &["idx_auth_sessions_user_id", "idx_auth_sessions_token_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "last-login-method",
        tables: USERS,
        indexes: &["idx_auth_users_email"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "username",
        tables: USERS,
        indexes: &["idx_auth_users_username"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "phone-number",
        tables: USERS_SESSIONS_VERIFICATIONS,
        indexes: &["idx_auth_verifications_identifier"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "siwe",
        tables: USERS_SESSIONS_ACCOUNTS_VERIFICATIONS,
        indexes: &[
            "idx_auth_accounts_provider_account",
            "idx_auth_verifications_identifier",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "haveibeenpwned",
        tables: NO_STORAGE,
        indexes: NO_STORAGE,
        migrations: NO_STORAGE,
    },
    AuthPluginStorageRequirement {
        plugin_id: "mcp",
        tables: &[
            "auth_users",
            "auth_sessions",
            "auth_api_keys",
            "auth_organizations",
            "auth_members",
            "auth_invitations",
            "auth_organization_roles",
            "auth_teams",
            "auth_team_members",
        ],
        indexes: &["idx_auth_sessions_token_hash", "idx_auth_api_keys_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "api-key",
        tables: API_KEYS,
        indexes: &[
            "idx_auth_api_keys_hash",
            "idx_auth_api_keys_user_id",
            "idx_auth_api_keys_expires_at",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "bearer",
        tables: &["auth_sessions", "auth_api_keys"],
        indexes: &["idx_auth_sessions_token_hash", "idx_auth_api_keys_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "captcha",
        tables: VERIFICATIONS,
        indexes: &["idx_auth_verifications_identifier"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "email-otp",
        tables: USERS_SESSIONS_VERIFICATIONS,
        indexes: &["idx_auth_verifications_identifier"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "magic-link",
        tables: USERS_SESSIONS_VERIFICATIONS,
        indexes: &["idx_auth_verifications_identifier"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "jwt",
        tables: SESSIONS,
        indexes: &["idx_auth_sessions_token_hash"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "oidc-provider",
        tables: USERS_SESSIONS_ACCOUNTS_VERIFICATIONS,
        indexes: &[
            "idx_auth_accounts_provider_account",
            "idx_auth_verifications_identifier",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "passkey",
        tables: PASSKEYS,
        indexes: &[
            "idx_auth_passkeys_credential_id",
            "idx_auth_passkeys_user_id",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "device-authorization",
        tables: USERS_SESSIONS_VERIFICATIONS,
        indexes: &[
            "idx_auth_verifications_identifier",
            "idx_auth_verifications_expires_at",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "two-factor",
        tables: TWO_FACTORS,
        indexes: &["idx_auth_two_factors_user_id"],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "organization",
        tables: ORGANIZATION_TABLES,
        indexes: &[
            "idx_auth_organizations_slug",
            "idx_auth_members_org_user",
            "idx_auth_org_roles_org_role",
            "idx_auth_team_members_team_user",
        ],
        migrations: MIGRATION_V1,
    },
    AuthPluginStorageRequirement {
        plugin_id: "admin",
        tables: &["auth_users", "auth_sessions", "auth_audit_events"],
        indexes: &[
            "idx_auth_users_email",
            "idx_auth_sessions_user_id",
            "idx_auth_audit_events_actor",
        ],
        migrations: MIGRATION_V1,
    },
];

pub fn auth_plugin_descriptors() -> &'static [AuthPluginDescriptor] {
    AUTH_PLUGIN_DESCRIPTORS
}

pub fn auth_plugin_storage_requirements() -> &'static [AuthPluginStorageRequirement] {
    AUTH_PLUGIN_STORAGE_REQUIREMENTS
}

pub fn auth_plugin_storage_requirement(id: &str) -> Option<&'static AuthPluginStorageRequirement> {
    auth_plugin_storage_requirements()
        .iter()
        .find(|requirement| requirement.plugin_id == id)
}

pub fn auth_plugin_descriptor(id: &str) -> Option<&'static AuthPluginDescriptor> {
    auth_plugin_descriptors()
        .iter()
        .find(|plugin| plugin.id == id)
}

pub fn plugin_routes(plugin: &AuthPluginDescriptor) -> Vec<&'static AuthRouteDescriptor> {
    auth_route_descriptors()
        .iter()
        .filter(|route| plugin.command_ids.contains(&route.operation_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ACCOUNT_MANAGEMENT_COMMANDS, ACCOUNT_MANAGEMENT_PLUGIN, ADDITIONAL_FIELDS_COMMANDS,
        ADDITIONAL_FIELDS_PLUGIN, ADMIN_COMMANDS, ADMIN_PLUGIN, ANONYMOUS_COMMANDS,
        ANONYMOUS_PLUGIN, API_KEY_COMMANDS, API_KEY_PLUGIN, BEARER_COMMANDS, BEARER_PLUGIN,
        CAPTCHA_COMMANDS, CAPTCHA_PLUGIN, CUSTOM_SESSION_COMMANDS, CUSTOM_SESSION_PLUGIN,
        DEVICE_AUTHORIZATION_COMMANDS, DEVICE_AUTHORIZATION_PLUGIN, EMAIL_OTP_COMMANDS,
        EMAIL_OTP_PLUGIN, EMAIL_PASSWORD_COMMANDS, EMAIL_PASSWORD_PLUGIN,
        EMAIL_VERIFICATION_COMMANDS, EMAIL_VERIFICATION_PLUGIN, HAVEIBEENPWNED_COMMANDS,
        HAVEIBEENPWNED_PLUGIN, JWT_COMMANDS, JWT_PLUGIN, LAST_LOGIN_METHOD_COMMANDS,
        LAST_LOGIN_METHOD_PLUGIN, MAGIC_LINK_COMMANDS, MAGIC_LINK_PLUGIN, MCP_COMMANDS, MCP_PLUGIN,
        MULTI_SESSION_COMMANDS, MULTI_SESSION_PLUGIN, OAUTH_COMMANDS, OAUTH_PLUGIN,
        OAUTH_PROXY_COMMANDS, OAUTH_PROXY_PLUGIN, OIDC_PROVIDER_COMMANDS, OIDC_PROVIDER_PLUGIN,
        ONE_TAP_COMMANDS, ONE_TAP_PLUGIN, ONE_TIME_TOKEN_COMMANDS, ONE_TIME_TOKEN_PLUGIN,
        OPEN_API_COMMANDS, OPEN_API_PLUGIN, ORGANIZATION_COMMANDS, ORGANIZATION_PLUGIN,
        PASSKEY_COMMANDS, PASSKEY_PLUGIN, PASSWORD_MANAGEMENT_COMMANDS, PASSWORD_MANAGEMENT_PLUGIN,
        PHONE_NUMBER_COMMANDS, PHONE_NUMBER_PLUGIN, SESSION_MANAGEMENT_COMMANDS,
        SESSION_MANAGEMENT_PLUGIN, SIWE_COMMANDS, SIWE_PLUGIN, TWO_FACTOR_COMMANDS,
        TWO_FACTOR_PLUGIN, USER_MANAGEMENT_COMMANDS, USER_MANAGEMENT_PLUGIN, USERNAME_COMMANDS,
        USERNAME_PLUGIN, account_management_routes, additional_fields_routes, admin_routes,
        anonymous_routes, api_key_routes, auth_plugin_descriptor, auth_plugin_descriptors,
        auth_plugin_storage_requirement, auth_plugin_storage_requirements, bearer_routes,
        captcha_routes, custom_session_routes, device_authorization_routes, email_otp_routes,
        email_password_routes, email_verification_routes, haveibeenpwned_routes, jwt_routes,
        last_login_method_routes, magic_link_routes, mcp_routes, multi_session_routes,
        oauth_proxy_routes, oauth_routes, oidc_provider_routes, one_tap_routes,
        one_time_token_routes, open_api_routes, organization_routes, passkey_routes,
        password_management_routes, phone_number_routes, session_management_routes, siwe_routes,
        two_factor_routes, user_management_routes, username_routes,
    };

    #[test]
    // r[verify auth.email.plugin-descriptor]
    fn email_password_plugin_declares_identity_and_dependencies() {
        let plugin = auth_plugin_descriptor("email-password").expect("email/password plugin");

        assert_eq!(plugin, &EMAIL_PASSWORD_PLUGIN);
        assert_eq!(plugin.display_name, "Email/password");
        assert!(plugin.upstream.contains("better-auth-rs:email_password"));
        assert_eq!(plugin.dependencies, &["sessions", "accounts"]);
        assert!(plugin.capabilities.contains(&"signup"));
        assert!(plugin.capabilities.contains(&"signin"));
        assert_eq!(plugin.command_ids, EMAIL_PASSWORD_COMMANDS);
    }

    #[test]
    // r[verify auth.email.plugin-routes]
    fn email_password_plugin_owns_signup_and_signin_routes() {
        let plugin = auth_plugin_descriptor("email-password").expect("email/password plugin");
        assert_eq!(plugin.command_ids, EMAIL_PASSWORD_COMMANDS);
        let operation_ids = email_password_routes()
            .iter()
            .map(|route| route.operation_id)
            .collect::<Vec<_>>();

        assert_eq!(
            operation_ids,
            vec!["createEmailPasswordUser", "signInEmailPassword"]
        );
        assert!(
            email_password_routes()
                .iter()
                .all(|route| !route.requires_session)
        );
    }

    #[test]
    // r[verify auth.anonymous.plugin-descriptor]
    // r[verify auth.anonymous.plugin-routes]
    fn anonymous_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("anonymous").expect("anonymous plugin");

        assert_eq!(plugin, &ANONYMOUS_PLUGIN);
        assert_eq!(plugin.command_ids, ANONYMOUS_COMMANDS);
        assert!(plugin.capabilities.contains(&"anonymous-session"));
        assert!(plugin.capabilities.contains(&"account-upgrade"));
        assert!(plugin.capabilities.contains(&"cleanup"));

        let routes = anonymous_routes();
        assert_eq!(routes.len(), ANONYMOUS_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "signInAnonymous",
                "linkAnonymousEmailPassword",
                "cleanupAnonymousUsers",
            ]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "signInAnonymous" && !route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "linkAnonymousEmailPassword"
                    && route.requires_session)
        );
    }

    #[test]
    fn plugin_registry_starts_with_email_password() {
        let plugins = auth_plugin_descriptors();

        assert_eq!(plugins.len(), 33);
        assert_eq!(plugins[0].id, "email-password");
        assert_eq!(plugins[1].id, "anonymous");
        assert_eq!(plugins[2].id, "session-management");
        assert_eq!(plugins[3].id, "custom-session");
        assert_eq!(plugins[4].id, "additional-fields");
        assert_eq!(plugins[5].id, "open-api");
        assert_eq!(plugins[6].id, "password-management");
        assert_eq!(plugins[7].id, "email-verification");
        assert_eq!(plugins[8].id, "user-management");
        assert_eq!(plugins[9].id, "account-management");
        assert_eq!(plugins[10].id, "oauth");
        assert_eq!(plugins[11].id, "oauth-proxy");
        assert_eq!(plugins[12].id, "one-tap");
        assert_eq!(plugins[13].id, "one-time-token");
        assert_eq!(plugins[14].id, "multi-session");
        assert_eq!(plugins[15].id, "last-login-method");
        assert_eq!(plugins[16].id, "username");
        assert_eq!(plugins[17].id, "phone-number");
        assert_eq!(plugins[18].id, "siwe");
        assert_eq!(plugins[19].id, "haveibeenpwned");
        assert_eq!(plugins[20].id, "mcp");
        assert_eq!(plugins[21].id, "api-key");
        assert_eq!(plugins[22].id, "bearer");
        assert_eq!(plugins[23].id, "captcha");
        assert_eq!(plugins[24].id, "email-otp");
        assert_eq!(plugins[25].id, "magic-link");
        assert_eq!(plugins[26].id, "jwt");
        assert_eq!(plugins[27].id, "oidc-provider");
        assert_eq!(plugins[28].id, "passkey");
        assert_eq!(plugins[29].id, "device-authorization");
        assert_eq!(plugins[30].id, "two-factor");
        assert_eq!(plugins[31].id, "organization");
        assert_eq!(plugins[32].id, "admin");
    }

    // r[verify auth.storage.plugin-migrations]
    #[test]
    fn every_plugin_declares_storage_and_migration_requirements() {
        let requirements = auth_plugin_storage_requirements();

        assert_eq!(requirements.len(), auth_plugin_descriptors().len());
        for plugin in auth_plugin_descriptors() {
            let requirement = auth_plugin_storage_requirement(plugin.id)
                .unwrap_or_else(|| panic!("{} storage requirement", plugin.id));
            assert_eq!(requirement.plugin_id, plugin.id);
            if !requirement.tables.is_empty() {
                assert_eq!(
                    requirement.migrations,
                    &["m20260513_000001_create_auth_tables"]
                );
            }
        }

        let organization =
            auth_plugin_storage_requirement("organization").expect("organization storage");
        assert!(organization.tables.contains(&"auth_organizations"));
        assert!(organization.tables.contains(&"auth_teams"));
        assert!(
            organization
                .indexes
                .contains(&"idx_auth_team_members_team_user")
        );

        let open_api = auth_plugin_storage_requirement("open-api").expect("open api storage");
        assert!(open_api.tables.is_empty());
        assert!(open_api.migrations.is_empty());
    }

    #[test]
    // r[verify auth.sessions.plugin-descriptor]
    fn session_management_plugin_declares_identity_and_capabilities() {
        let plugin = auth_plugin_descriptor("session-management").expect("session plugin");

        assert_eq!(plugin, &SESSION_MANAGEMENT_PLUGIN);
        assert_eq!(plugin.display_name, "Session management");
        assert!(
            plugin
                .upstream
                .contains("better-auth-rs:session_management")
        );
        assert!(plugin.capabilities.contains(&"get-session"));
        assert!(plugin.capabilities.contains(&"revoke-other-sessions"));
        assert_eq!(plugin.command_ids, SESSION_MANAGEMENT_COMMANDS);
    }

    #[test]
    // r[verify auth.custom-session.plugin-descriptor]
    // r[verify auth.custom-session.plugin-routes]
    fn custom_session_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("custom-session").expect("custom session plugin");

        assert_eq!(plugin, &CUSTOM_SESSION_PLUGIN);
        assert_eq!(plugin.dependencies, &["session-management"]);
        assert_eq!(plugin.command_ids, CUSTOM_SESSION_COMMANDS);
        assert!(plugin.capabilities.contains(&"typed-enrichment-hook"));
        assert!(plugin.capabilities.contains(&"schema-extension"));

        let routes = custom_session_routes();
        assert_eq!(routes.len(), CUSTOM_SESSION_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "currentCustomSession");
        assert!(routes[0].requires_session);
    }

    #[test]
    // r[verify auth.additional-fields.plugin-descriptor]
    // r[verify auth.additional-fields.plugin-routes]
    fn additional_fields_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("additional-fields").expect("additional fields plugin");

        assert_eq!(plugin, &ADDITIONAL_FIELDS_PLUGIN);
        assert_eq!(plugin.dependencies, &["users", "sessions", "accounts"]);
        assert_eq!(plugin.command_ids, ADDITIONAL_FIELDS_COMMANDS);
        assert!(plugin.capabilities.contains(&"metadata-persistence"));
        assert!(plugin.capabilities.contains(&"hidden-metadata"));
        assert!(plugin.capabilities.contains(&"schema-metadata"));

        let routes = additional_fields_routes();
        assert_eq!(routes.len(), ADDITIONAL_FIELDS_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "additionalFieldsSchema");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.openapi.plugin-descriptor]
    // r[verify auth.openapi.plugin-routes]
    fn open_api_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("open-api").expect("open api plugin");

        assert_eq!(plugin, &OPEN_API_PLUGIN);
        assert_eq!(plugin.command_ids, OPEN_API_COMMANDS);
        assert!(plugin.capabilities.contains(&"request-schemas"));
        assert!(plugin.capabilities.contains(&"response-schemas"));
        assert!(plugin.capabilities.contains(&"error-schemas"));

        let routes = open_api_routes();
        assert_eq!(routes.len(), OPEN_API_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "getOpenApiDocument");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.sessions.plugin-routes]
    fn session_management_plugin_owns_session_routes() {
        let plugin = auth_plugin_descriptor("session-management").expect("session plugin");
        assert_eq!(plugin.command_ids, SESSION_MANAGEMENT_COMMANDS);
        let operation_ids = session_management_routes()
            .iter()
            .map(|route| route.operation_id)
            .collect::<Vec<_>>();

        assert_eq!(
            operation_ids,
            vec![
                "currentSession",
                "signOut",
                "listSessions",
                "revokeSession",
                "revokeOtherSessions"
            ]
        );
        assert!(
            session_management_routes()
                .iter()
                .all(|route| route.requires_session)
        );
    }

    #[test]
    // r[verify auth.multisession.plugin-descriptor]
    // r[verify auth.multisession.plugin-routes]
    fn multi_session_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("multi-session").expect("multi-session plugin");

        assert_eq!(plugin, &MULTI_SESSION_PLUGIN);
        assert_eq!(plugin.command_ids, MULTI_SESSION_COMMANDS);
        assert_eq!(plugin.dependencies, &["sessions"]);
        assert!(plugin.capabilities.contains(&"device-session-list"));
        assert!(plugin.capabilities.contains(&"permission-isolation"));

        let routes = multi_session_routes();
        assert_eq!(routes.len(), MULTI_SESSION_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "listDeviceSessions",
                "setActiveDeviceSession",
                "revokeDeviceSession",
            ]
        );
        assert!(routes.iter().all(|route| !route.requires_session));
    }

    #[test]
    // r[verify auth.lastlogin.plugin-descriptor]
    // r[verify auth.lastlogin.plugin-routes]
    fn last_login_method_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("last-login-method").expect("last login method plugin");

        assert_eq!(plugin, &LAST_LOGIN_METHOD_PLUGIN);
        assert_eq!(plugin.command_ids, LAST_LOGIN_METHOD_COMMANDS);
        assert_eq!(plugin.dependencies, &["users", "sessions"]);
        assert!(plugin.capabilities.contains(&"method-tracking"));
        assert!(plugin.capabilities.contains(&"client-readable-cookie"));

        let routes = last_login_method_routes();
        assert_eq!(routes.len(), LAST_LOGIN_METHOD_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["getLastLoginMethod", "clearLastLoginMethod"]
        );
        assert!(routes.iter().all(|route| route.requires_session));
    }

    #[test]
    // r[verify auth.username.plugin-descriptor]
    // r[verify auth.username.plugin-routes]
    fn username_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("username").expect("username plugin");

        assert_eq!(plugin, &USERNAME_PLUGIN);
        assert_eq!(plugin.command_ids, USERNAME_COMMANDS);
        assert_eq!(
            plugin.dependencies,
            &["users", "sessions", "email-password"]
        );
        assert!(plugin.capabilities.contains(&"username-validation"));
        assert!(plugin.capabilities.contains(&"case-insensitive-lookup"));

        let routes = username_routes();
        assert_eq!(routes.len(), USERNAME_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["signInUsername", "updateUsername"]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "signInUsername" && !route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "updateUsername" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.phone.plugin-descriptor]
    // r[verify auth.phone.plugin-routes]
    fn phone_number_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("phone-number").expect("phone-number plugin");

        assert_eq!(plugin, &PHONE_NUMBER_PLUGIN);
        assert_eq!(plugin.command_ids, PHONE_NUMBER_COMMANDS);
        assert_eq!(
            plugin.dependencies,
            &["users", "sessions", "verification-tokens"]
        );
        assert!(plugin.capabilities.contains(&"sms-provider"));
        assert!(plugin.capabilities.contains(&"phone-sign-in"));

        let routes = phone_number_routes();
        assert_eq!(routes.len(), PHONE_NUMBER_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "sendPhoneNumberOtp",
                "verifyPhoneNumberOtp",
                "updatePhoneNumber",
            ]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "updatePhoneNumber" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.siwe.plugin-descriptor]
    // r[verify auth.siwe.plugin-routes]
    fn siwe_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("siwe").expect("siwe plugin");

        assert_eq!(plugin, &SIWE_PLUGIN);
        assert_eq!(plugin.command_ids, SIWE_COMMANDS);
        assert_eq!(
            plugin.dependencies,
            &["users", "sessions", "accounts", "verification-tokens"]
        );
        assert!(plugin.capabilities.contains(&"nonce"));
        assert!(plugin.capabilities.contains(&"address-linking"));

        let routes = siwe_routes();
        assert_eq!(routes.len(), SIWE_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["createSiweNonce", "verifySiweMessage", "linkSiweAddress"]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "linkSiweAddress" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.hibp.plugin-descriptor]
    // r[verify auth.hibp.plugin-routes]
    fn haveibeenpwned_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("haveibeenpwned").expect("haveibeenpwned plugin");

        assert_eq!(plugin, &HAVEIBEENPWNED_PLUGIN);
        assert_eq!(plugin.command_ids, HAVEIBEENPWNED_COMMANDS);
        assert_eq!(plugin.dependencies, &["email-password"]);
        assert!(plugin.capabilities.contains(&"k-anonymity-range"));
        assert!(plugin.capabilities.contains(&"failure-policy"));

        let routes = haveibeenpwned_routes();
        assert_eq!(routes.len(), HAVEIBEENPWNED_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "checkPasswordBreach");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.mcp.plugin-descriptor]
    // r[verify auth.mcp.plugin-routes]
    fn mcp_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("mcp").expect("mcp plugin");

        assert_eq!(plugin, &MCP_PLUGIN);
        assert_eq!(plugin.command_ids, MCP_COMMANDS);
        assert_eq!(plugin.dependencies, &["sessions", "organization"]);
        assert!(plugin.capabilities.contains(&"session-validation"));
        assert!(plugin.capabilities.contains(&"api-key-token"));
        assert!(plugin.capabilities.contains(&"service-permission-check"));

        let routes = mcp_routes();
        assert_eq!(routes.len(), MCP_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "authorizeMcpRequest");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.password.plugin-descriptor]
    fn password_management_plugin_declares_identity_and_dependencies() {
        let plugin = auth_plugin_descriptor("password-management").expect("password plugin");

        assert_eq!(plugin, &PASSWORD_MANAGEMENT_PLUGIN);
        assert_eq!(plugin.display_name, "Password management");
        assert!(
            plugin
                .upstream
                .contains("better-auth-rs:password_management")
        );
        assert!(plugin.dependencies.contains(&"email-password"));
        assert!(plugin.capabilities.contains(&"change-password"));
        assert!(plugin.capabilities.contains(&"complete-password-reset"));
        assert_eq!(plugin.command_ids, PASSWORD_MANAGEMENT_COMMANDS);
    }

    #[test]
    // r[verify auth.password.plugin-routes]
    fn password_management_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("password-management").expect("password plugin");
        assert_eq!(plugin.command_ids, PASSWORD_MANAGEMENT_COMMANDS);
        let operation_ids = password_management_routes()
            .iter()
            .map(|route| route.operation_id)
            .collect::<Vec<_>>();

        assert_eq!(
            operation_ids,
            vec![
                "changePassword",
                "requestPasswordReset",
                "completePasswordReset"
            ]
        );
        assert!(
            password_management_routes()
                .iter()
                .any(|route| route.operation_id == "changePassword" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.verify.plugin-descriptor]
    // r[verify auth.verify.plugin-routes]
    fn email_verification_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("email-verification").expect("verify plugin");

        assert_eq!(plugin, &EMAIL_VERIFICATION_PLUGIN);
        assert_eq!(plugin.command_ids, EMAIL_VERIFICATION_COMMANDS);
        assert_eq!(
            email_verification_routes()
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["requestEmailVerification", "verifyEmail"]
        );
    }

    #[test]
    // r[verify auth.user.plugin-descriptor]
    // r[verify auth.user.plugin-routes]
    fn user_management_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("user-management").expect("user plugin");

        assert_eq!(plugin, &USER_MANAGEMENT_PLUGIN);
        assert_eq!(plugin.command_ids, USER_MANAGEMENT_COMMANDS);
        assert_eq!(
            user_management_routes()
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["changeEmail", "deleteUser"]
        );
    }

    #[test]
    // r[verify auth.account.plugin-descriptor]
    // r[verify auth.account.plugin-routes]
    fn account_management_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("account-management").expect("account plugin");

        assert_eq!(plugin, &ACCOUNT_MANAGEMENT_PLUGIN);
        assert_eq!(plugin.command_ids, ACCOUNT_MANAGEMENT_COMMANDS);
        assert_eq!(
            account_management_routes()
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["listAccounts", "linkOAuthAccount", "unlinkOAuthAccount"]
        );
    }

    #[test]
    // r[verify auth.oauth.plugin-descriptor]
    // r[verify auth.oauth.plugin-routes]
    fn oauth_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("oauth").expect("oauth plugin");

        assert_eq!(plugin, &OAUTH_PLUGIN);
        assert_eq!(plugin.command_ids, OAUTH_COMMANDS);
        assert!(plugin.capabilities.contains(&"social-sign-in"));
        let routes = oauth_routes();
        assert_eq!(routes.len(), OAUTH_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "beginOAuthAuthorization")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "linkOAuthAccount" && route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "getOAuthAccessToken" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.oauth-proxy.plugin-descriptor]
    // r[verify auth.oauth-proxy.plugin-routes]
    fn oauth_proxy_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("oauth-proxy").expect("oauth proxy plugin");

        assert_eq!(plugin, &OAUTH_PROXY_PLUGIN);
        assert_eq!(plugin.command_ids, OAUTH_PROXY_COMMANDS);
        assert!(plugin.capabilities.contains(&"callback-forwarding"));
        assert!(plugin.capabilities.contains(&"redirect-policy"));

        let routes = oauth_proxy_routes();
        assert_eq!(routes.len(), OAUTH_PROXY_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "beginOAuthProxyAuthorization")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "consumeOAuthProxyCallback")
        );
    }

    #[test]
    // r[verify auth.onetap.plugin-descriptor]
    // r[verify auth.onetap.plugin-routes]
    fn one_tap_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("one-tap").expect("one tap plugin");

        assert_eq!(plugin, &ONE_TAP_PLUGIN);
        assert_eq!(plugin.command_ids, ONE_TAP_COMMANDS);
        assert!(plugin.capabilities.contains(&"google-id-token"));
        assert!(plugin.capabilities.contains(&"auto-signup"));

        let routes = one_tap_routes();
        assert_eq!(routes.len(), ONE_TAP_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "oneTapCallback");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.ott.plugin-descriptor]
    // r[verify auth.ott.plugin-routes]
    fn one_time_token_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("one-time-token").expect("one-time token plugin");

        assert_eq!(plugin, &ONE_TIME_TOKEN_PLUGIN);
        assert_eq!(plugin.command_ids, ONE_TIME_TOKEN_COMMANDS);
        assert!(plugin.capabilities.contains(&"consume"));
        assert!(plugin.capabilities.contains(&"metadata"));

        let routes = one_time_token_routes();
        assert_eq!(routes.len(), ONE_TIME_TOKEN_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "generateOneTimeToken" && route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "verifyOneTimeToken" && !route.requires_session)
        );
    }

    #[test]
    // r[verify auth.apikey.plugin-descriptor]
    // r[verify auth.apikey.plugin-routes]
    fn api_key_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("api-key").expect("api key plugin");

        assert_eq!(plugin, &API_KEY_PLUGIN);
        assert_eq!(plugin.command_ids, API_KEY_COMMANDS);
        assert!(plugin.capabilities.contains(&"verify"));
        let routes = api_key_routes();
        assert_eq!(routes.len(), API_KEY_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "verifyApiKey" && !route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "createApiKey" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.bearer.plugin-descriptor]
    // r[verify auth.bearer.plugin-routes]
    fn bearer_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("bearer").expect("bearer plugin");

        assert_eq!(plugin, &BEARER_PLUGIN);
        assert_eq!(plugin.command_ids, BEARER_COMMANDS);
        assert!(plugin.capabilities.contains(&"authorization-header"));
        assert!(plugin.capabilities.contains(&"session-token"));
        assert!(plugin.capabilities.contains(&"api-key-token"));

        let routes = bearer_routes();
        assert_eq!(routes.len(), BEARER_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "authenticateBearerToken");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.captcha.plugin-descriptor]
    // r[verify auth.captcha.plugin-routes]
    fn captcha_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("captcha").expect("captcha plugin");

        assert_eq!(plugin, &CAPTCHA_PLUGIN);
        assert_eq!(plugin.command_ids, CAPTCHA_COMMANDS);
        assert!(plugin.capabilities.contains(&"provider-config"));
        assert!(plugin.capabilities.contains(&"test-provider"));
        assert!(plugin.capabilities.contains(&"signup-hook"));

        let routes = captcha_routes();
        assert_eq!(routes.len(), CAPTCHA_COMMANDS.len());
        assert_eq!(routes[0].operation_id, "verifyCaptcha");
        assert!(!routes[0].requires_session);
    }

    #[test]
    // r[verify auth.emailotp.plugin-descriptor]
    // r[verify auth.emailotp.plugin-routes]
    fn email_otp_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("email-otp").expect("email otp plugin");

        assert_eq!(plugin, &EMAIL_OTP_PLUGIN);
        assert_eq!(plugin.command_ids, EMAIL_OTP_COMMANDS);
        assert!(plugin.capabilities.contains(&"send-otp"));
        assert!(plugin.capabilities.contains(&"single-use"));
        assert!(plugin.capabilities.contains(&"session-creation"));

        let routes = email_otp_routes();
        assert_eq!(routes.len(), EMAIL_OTP_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["sendEmailOtp", "verifyEmailOtp"]
        );
        assert!(routes.iter().all(|route| !route.requires_session));
    }

    #[test]
    // r[verify auth.magic.plugin-descriptor]
    // r[verify auth.magic.plugin-routes]
    fn magic_link_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("magic-link").expect("magic link plugin");

        assert_eq!(plugin, &MAGIC_LINK_PLUGIN);
        assert_eq!(plugin.command_ids, MAGIC_LINK_COMMANDS);
        assert!(plugin.capabilities.contains(&"link-generation"));
        assert!(plugin.capabilities.contains(&"single-use"));
        assert!(plugin.capabilities.contains(&"redirect-trust"));

        let routes = magic_link_routes();
        assert_eq!(routes.len(), MAGIC_LINK_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["sendMagicLink", "verifyMagicLink"]
        );
        assert!(routes.iter().all(|route| !route.requires_session));
    }

    #[test]
    // r[verify auth.jwt.plugin-descriptor]
    // r[verify auth.jwt.plugin-routes]
    fn jwt_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("jwt").expect("jwt plugin");

        assert_eq!(plugin, &JWT_PLUGIN);
        assert_eq!(plugin.command_ids, JWT_COMMANDS);
        assert!(plugin.capabilities.contains(&"hs256-signing"));
        assert!(plugin.capabilities.contains(&"rotation"));
        assert!(plugin.capabilities.contains(&"session-backed"));

        let routes = jwt_routes();
        assert_eq!(routes.len(), JWT_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec!["issueJwt", "verifyJwt", "getJwtKeySet"]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "issueJwt" && route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "verifyJwt" && !route.requires_session)
        );
    }

    #[test]
    // r[verify auth.oidc.plugin-descriptor]
    // r[verify auth.oidc.plugin-routes]
    fn oidc_provider_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("oidc-provider").expect("oidc provider plugin");

        assert_eq!(plugin, &OIDC_PROVIDER_PLUGIN);
        assert_eq!(plugin.command_ids, OIDC_PROVIDER_COMMANDS);
        assert!(plugin.capabilities.contains(&"authorization-code"));
        assert!(plugin.capabilities.contains(&"pkce-s256"));
        assert!(plugin.capabilities.contains(&"userinfo"));

        let routes = oidc_provider_routes();
        assert_eq!(routes.len(), OIDC_PROVIDER_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "authorizeOidc" && route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "exchangeOidcToken" && !route.requires_session)
        );
    }

    #[test]
    // r[verify auth.passkey.plugin-descriptor]
    // r[verify auth.passkey.plugin-routes]
    fn passkey_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("passkey").expect("passkey plugin");

        assert_eq!(plugin, &PASSKEY_PLUGIN);
        assert_eq!(plugin.command_ids, PASSKEY_COMMANDS);
        assert!(plugin.capabilities.contains(&"registration-options"));
        assert!(plugin.capabilities.contains(&"authentication-verification"));
        assert!(plugin.capabilities.contains(&"rp-origin-validation"));

        let routes = passkey_routes();
        assert_eq!(routes.len(), PASSKEY_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "beginPasskeyRegistration",
                "completePasskeyRegistration",
                "beginPasskeyAuthentication",
                "completePasskeyAuthentication",
                "listPasskeys",
                "deletePasskey",
            ]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "beginPasskeyAuthentication"
                    && !route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "listPasskeys" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.device.plugin-descriptor]
    // r[verify auth.device.plugin-routes]
    fn device_authorization_plugin_resolves_generated_routes() {
        let plugin =
            auth_plugin_descriptor("device-authorization").expect("device authorization plugin");

        assert_eq!(plugin, &DEVICE_AUTHORIZATION_PLUGIN);
        assert_eq!(plugin.command_ids, DEVICE_AUTHORIZATION_COMMANDS);
        assert!(plugin.capabilities.contains(&"device-code"));
        assert!(plugin.capabilities.contains(&"slow-down"));
        assert!(plugin.capabilities.contains(&"approval"));

        let routes = device_authorization_routes();
        assert_eq!(routes.len(), DEVICE_AUTHORIZATION_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "createDeviceAuthorization",
                "verifyDeviceCode",
                "approveDeviceCode",
                "denyDeviceCode",
                "pollDeviceToken",
            ]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "approveDeviceCode" && route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "pollDeviceToken" && !route.requires_session)
        );
    }

    #[test]
    // r[verify auth.twofactor.plugin-descriptor]
    // r[verify auth.twofactor.plugin-routes]
    fn two_factor_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("two-factor").expect("two factor plugin");

        assert_eq!(plugin, &TWO_FACTOR_PLUGIN);
        assert_eq!(plugin.command_ids, TWO_FACTOR_COMMANDS);
        assert!(plugin.capabilities.contains(&"totp-setup"));
        assert!(plugin.capabilities.contains(&"pending-session"));
        assert!(plugin.capabilities.contains(&"backup-codes"));

        let routes = two_factor_routes();
        assert_eq!(routes.len(), TWO_FACTOR_COMMANDS.len());
        assert_eq!(
            routes
                .iter()
                .map(|route| route.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "startTwoFactorSetup",
                "confirmTwoFactor",
                "verifyTwoFactor",
                "disableTwoFactor",
            ]
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "verifyTwoFactor" && !route.requires_session)
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "disableTwoFactor" && route.requires_session)
        );
    }

    #[test]
    // r[verify auth.org.plugin-descriptor]
    // r[verify auth.org.plugin-routes]
    fn organization_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("organization").expect("organization plugin");

        assert_eq!(plugin, &ORGANIZATION_PLUGIN);
        assert_eq!(plugin.command_ids, ORGANIZATION_COMMANDS);
        let routes = organization_routes();
        assert_eq!(routes.len(), ORGANIZATION_COMMANDS.len());
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "createOrganization")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "authorizeOrganizationAction")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "addTeamMember")
        );
        assert!(routes.iter().all(|route| route.requires_session));
    }

    #[test]
    // r[verify auth.admin.plugin-descriptor]
    // r[verify auth.admin.plugin-routes]
    fn admin_plugin_resolves_generated_routes() {
        let plugin = auth_plugin_descriptor("admin").expect("admin plugin");

        assert_eq!(plugin, &ADMIN_PLUGIN);
        assert_eq!(plugin.command_ids, ADMIN_COMMANDS);
        let routes = admin_routes();
        assert_eq!(routes.len(), ADMIN_COMMANDS.len());
        assert!(routes.iter().any(|route| route.operation_id == "listUsers"));
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "adminCreateUser")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "impersonateUser")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "adminSetUserPassword")
        );
        assert!(
            routes
                .iter()
                .any(|route| route.operation_id == "adminHasPermission")
        );
        assert!(routes.iter().all(|route| route.requires_session));
    }
}
