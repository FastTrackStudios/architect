use crate::ArchitectAuth;

#[derive(Clone, Debug)]
pub struct ArchitectAuthConfig {
    pub secret: String,
    pub base_url: String,
    pub session_ttl_seconds: i64,
    pub email_password_enabled: bool,
    pub require_email_verification: bool,
    pub oauth_signup_enabled: bool,
    pub oauth_token_storage_enabled: bool,
    pub passkey_rp_id: String,
    pub passkey_allowed_origins: Vec<String>,
    pub captcha: CaptchaConfig,
    pub sms: SmsConfig,
    pub jwt: JwtConfig,
    pub oidc: OidcProviderConfig,
    pub oauth_proxy: OAuthProxyConfig,
    pub one_tap: OneTapConfig,
    pub siwe: SiweConfig,
    pub breached_passwords: BreachedPasswordConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptchaConfig {
    pub provider: CaptchaProvider,
    pub protected_flows: Vec<CaptchaFlow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptchaProvider {
    Disabled,
    Bypass,
    Test { valid_token: String },
    FailClosed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmsConfig {
    pub provider: SmsProvider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmsProvider {
    Disabled,
    Test,
    FailClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptchaFlow {
    SignUp,
    PasswordReset,
    EmailOtp,
    MagicLink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub active_key_id: String,
    pub keys: Vec<JwtSigningKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtSigningKey {
    pub id: String,
    pub secret: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcProviderConfig {
    pub issuer: String,
    pub code_ttl_seconds: i64,
    pub access_token_ttl_seconds: i64,
    pub refresh_token_ttl_seconds: i64,
    pub require_pkce: bool,
    pub allow_dynamic_client_registration: bool,
    pub clients: Vec<OidcClientConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub scopes: Vec<String>,
    pub public_client: bool,
    pub skip_consent: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthProxyConfig {
    pub current_url: String,
    pub production_url: String,
    pub callback_path: String,
    pub max_age_seconds: i64,
    pub allowed_redirect_origins: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneTapConfig {
    pub client_id: String,
    pub issuer: String,
    pub disable_signup: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiweConfig {
    pub domain: String,
    pub signup_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreachedPasswordConfig {
    pub provider: BreachedPasswordProvider,
    pub failure_policy: BreachedPasswordFailurePolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BreachedPasswordProvider {
    Disabled,
    Test { breached_passwords: Vec<String> },
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreachedPasswordFailurePolicy {
    Allow,
    Deny,
}

pub struct ArchitectAuthBuilder<S> {
    storage: S,
    secret: Option<String>,
    base_url: Option<String>,
    session_ttl_seconds: i64,
    email_password_enabled: bool,
    require_email_verification: bool,
    oauth_signup_enabled: bool,
    oauth_token_storage_enabled: bool,
    passkey_rp_id: Option<String>,
    passkey_allowed_origins: Vec<String>,
    captcha: CaptchaConfig,
    sms: SmsConfig,
    jwt_issuer: Option<String>,
    jwt_audience: Option<String>,
    jwt_active_key_id: Option<String>,
    jwt_keys: Vec<JwtSigningKey>,
    oidc_issuer: Option<String>,
    oidc_code_ttl_seconds: i64,
    oidc_access_token_ttl_seconds: i64,
    oidc_refresh_token_ttl_seconds: i64,
    oidc_require_pkce: bool,
    oidc_allow_dynamic_client_registration: bool,
    oidc_clients: Vec<OidcClientConfig>,
    oauth_proxy_current_url: Option<String>,
    oauth_proxy_production_url: Option<String>,
    oauth_proxy_callback_path: String,
    oauth_proxy_max_age_seconds: i64,
    oauth_proxy_allowed_redirect_origins: Vec<String>,
    one_tap_client_id: Option<String>,
    one_tap_issuer: String,
    one_tap_disable_signup: bool,
    siwe_domain: Option<String>,
    siwe_signup_enabled: bool,
    breached_passwords: BreachedPasswordConfig,
}

impl<S> ArchitectAuthBuilder<S> {
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            secret: None,
            base_url: None,
            session_ttl_seconds: 60 * 60 * 24 * 7,
            email_password_enabled: true,
            require_email_verification: false,
            oauth_signup_enabled: false,
            oauth_token_storage_enabled: false,
            passkey_rp_id: None,
            passkey_allowed_origins: vec!["http://localhost:3000".into()],
            captcha: CaptchaConfig {
                provider: CaptchaProvider::Disabled,
                protected_flows: Vec::new(),
            },
            sms: SmsConfig {
                provider: SmsProvider::Disabled,
            },
            jwt_issuer: None,
            jwt_audience: None,
            jwt_active_key_id: None,
            jwt_keys: Vec::new(),
            oidc_issuer: None,
            oidc_code_ttl_seconds: 600,
            oidc_access_token_ttl_seconds: 3600,
            oidc_refresh_token_ttl_seconds: 60 * 60 * 24 * 7,
            oidc_require_pkce: true,
            oidc_allow_dynamic_client_registration: false,
            oidc_clients: Vec::new(),
            oauth_proxy_current_url: None,
            oauth_proxy_production_url: None,
            oauth_proxy_callback_path: "/auth/oauth-proxy-callback".into(),
            oauth_proxy_max_age_seconds: 60,
            oauth_proxy_allowed_redirect_origins: Vec::new(),
            one_tap_client_id: None,
            one_tap_issuer: "https://accounts.google.com".into(),
            one_tap_disable_signup: false,
            siwe_domain: None,
            siwe_signup_enabled: true,
            breached_passwords: BreachedPasswordConfig {
                provider: BreachedPasswordProvider::Disabled,
                failure_policy: BreachedPasswordFailurePolicy::Deny,
            },
        }
    }

    pub fn storage<T>(self, storage: T) -> ArchitectAuthBuilder<T> {
        ArchitectAuthBuilder {
            storage,
            secret: self.secret,
            base_url: self.base_url,
            session_ttl_seconds: self.session_ttl_seconds,
            email_password_enabled: self.email_password_enabled,
            require_email_verification: self.require_email_verification,
            oauth_signup_enabled: self.oauth_signup_enabled,
            oauth_token_storage_enabled: self.oauth_token_storage_enabled,
            passkey_rp_id: self.passkey_rp_id,
            passkey_allowed_origins: self.passkey_allowed_origins,
            captcha: self.captcha,
            sms: self.sms,
            jwt_issuer: self.jwt_issuer,
            jwt_audience: self.jwt_audience,
            jwt_active_key_id: self.jwt_active_key_id,
            jwt_keys: self.jwt_keys,
            oidc_issuer: self.oidc_issuer,
            oidc_code_ttl_seconds: self.oidc_code_ttl_seconds,
            oidc_access_token_ttl_seconds: self.oidc_access_token_ttl_seconds,
            oidc_refresh_token_ttl_seconds: self.oidc_refresh_token_ttl_seconds,
            oidc_require_pkce: self.oidc_require_pkce,
            oidc_allow_dynamic_client_registration: self.oidc_allow_dynamic_client_registration,
            oidc_clients: self.oidc_clients,
            oauth_proxy_current_url: self.oauth_proxy_current_url,
            oauth_proxy_production_url: self.oauth_proxy_production_url,
            oauth_proxy_callback_path: self.oauth_proxy_callback_path,
            oauth_proxy_max_age_seconds: self.oauth_proxy_max_age_seconds,
            oauth_proxy_allowed_redirect_origins: self.oauth_proxy_allowed_redirect_origins,
            one_tap_client_id: self.one_tap_client_id,
            one_tap_issuer: self.one_tap_issuer,
            one_tap_disable_signup: self.one_tap_disable_signup,
            siwe_domain: self.siwe_domain,
            siwe_signup_enabled: self.siwe_signup_enabled,
            breached_passwords: self.breached_passwords,
        }
    }

    pub fn secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn session_ttl_seconds(mut self, seconds: i64) -> Self {
        self.session_ttl_seconds = seconds;
        self
    }

    pub fn email_password_enabled(mut self, enabled: bool) -> Self {
        self.email_password_enabled = enabled;
        self
    }

    pub fn require_email_verification(mut self, required: bool) -> Self {
        self.require_email_verification = required;
        self
    }

    pub fn oauth_signup_enabled(mut self, enabled: bool) -> Self {
        self.oauth_signup_enabled = enabled;
        self
    }

    // r[impl auth.oauth.token-encryption]
    pub fn oauth_token_storage_enabled(mut self, enabled: bool) -> Self {
        self.oauth_token_storage_enabled = enabled;
        self
    }

    pub fn passkey_rp_id(mut self, rp_id: impl Into<String>) -> Self {
        self.passkey_rp_id = Some(rp_id.into());
        self
    }

    pub fn passkey_allowed_origin(mut self, origin: impl Into<String>) -> Self {
        self.passkey_allowed_origins.push(origin.into());
        self
    }

    pub fn captcha_provider(mut self, provider: CaptchaProvider) -> Self {
        self.captcha.provider = provider;
        self
    }

    pub fn captcha_test_token(mut self, token: impl Into<String>) -> Self {
        self.captcha.provider = CaptchaProvider::Test {
            valid_token: token.into(),
        };
        self
    }

    pub fn captcha_bypass(mut self) -> Self {
        self.captcha.provider = CaptchaProvider::Bypass;
        self
    }

    pub fn captcha_protected_flow(mut self, flow: CaptchaFlow) -> Self {
        if !self.captcha.protected_flows.contains(&flow) {
            self.captcha.protected_flows.push(flow);
        }
        self
    }

    pub fn sms_provider(mut self, provider: SmsProvider) -> Self {
        self.sms.provider = provider;
        self
    }

    pub fn sms_test_provider(mut self) -> Self {
        self.sms.provider = SmsProvider::Test;
        self
    }

    pub fn siwe_domain(mut self, domain: impl Into<String>) -> Self {
        self.siwe_domain = Some(domain.into());
        self
    }

    pub fn siwe_signup_enabled(mut self, enabled: bool) -> Self {
        self.siwe_signup_enabled = enabled;
        self
    }

    pub fn breached_password_provider(mut self, provider: BreachedPasswordProvider) -> Self {
        self.breached_passwords.provider = provider;
        self
    }

    pub fn breached_password_failure_policy(
        mut self,
        policy: BreachedPasswordFailurePolicy,
    ) -> Self {
        self.breached_passwords.failure_policy = policy;
        self
    }

    pub fn jwt_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.jwt_issuer = Some(issuer.into());
        self
    }

    pub fn jwt_audience(mut self, audience: impl Into<String>) -> Self {
        self.jwt_audience = Some(audience.into());
        self
    }

    pub fn jwt_signing_key(mut self, id: impl Into<String>, secret: impl Into<String>) -> Self {
        let id = id.into();
        self.jwt_active_key_id = Some(id.clone());
        self.jwt_keys.push(JwtSigningKey {
            id,
            secret: secret.into(),
        });
        self
    }

    pub fn jwt_fallback_key(mut self, id: impl Into<String>, secret: impl Into<String>) -> Self {
        self.jwt_keys.push(JwtSigningKey {
            id: id.into(),
            secret: secret.into(),
        });
        self
    }

    pub fn oidc_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.oidc_issuer = Some(issuer.into());
        self
    }

    pub fn oidc_allow_dynamic_client_registration(mut self, enabled: bool) -> Self {
        self.oidc_allow_dynamic_client_registration = enabled;
        self
    }

    pub fn oidc_require_pkce(mut self, required: bool) -> Self {
        self.oidc_require_pkce = required;
        self
    }

    pub fn oidc_client(mut self, client: OidcClientConfig) -> Self {
        self.oidc_clients.push(client);
        self
    }

    pub fn oauth_proxy_current_url(mut self, url: impl Into<String>) -> Self {
        self.oauth_proxy_current_url = Some(url.into());
        self
    }

    pub fn oauth_proxy_production_url(mut self, url: impl Into<String>) -> Self {
        self.oauth_proxy_production_url = Some(url.into());
        self
    }

    pub fn oauth_proxy_allowed_redirect_origin(mut self, origin: impl Into<String>) -> Self {
        self.oauth_proxy_allowed_redirect_origins
            .push(origin.into());
        self
    }

    pub fn one_tap_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.one_tap_client_id = Some(client_id.into());
        self
    }

    pub fn one_tap_disable_signup(mut self, disabled: bool) -> Self {
        self.one_tap_disable_signup = disabled;
        self
    }

    pub fn build(self) -> Result<ArchitectAuth<S>, ConfigError> {
        let secret = self.secret.ok_or(ConfigError::MissingSecret)?;
        // r[impl auth.core.secret-minimum]
        if secret.len() < 32 {
            return Err(ConfigError::SecretTooShort);
        }

        let jwt_active_key_id = self.jwt_active_key_id.unwrap_or_else(|| "primary".into());
        let mut jwt_keys = self.jwt_keys;
        if !jwt_keys.iter().any(|key| key.id == jwt_active_key_id) {
            jwt_keys.push(JwtSigningKey {
                id: jwt_active_key_id.clone(),
                secret: secret.clone(),
            });
        }

        let base_url = self
            .base_url
            .unwrap_or_else(|| "http://localhost:3000".into());
        let oidc_issuer = self.oidc_issuer.unwrap_or_else(|| base_url.clone());
        let oauth_proxy_current_url = self
            .oauth_proxy_current_url
            .unwrap_or_else(|| base_url.clone());
        let oauth_proxy_production_url = self
            .oauth_proxy_production_url
            .unwrap_or_else(|| base_url.clone());
        let mut oauth_proxy_allowed_redirect_origins = self.oauth_proxy_allowed_redirect_origins;
        if oauth_proxy_allowed_redirect_origins.is_empty() {
            oauth_proxy_allowed_redirect_origins.push(oauth_proxy_current_url.clone());
        }

        Ok(ArchitectAuth {
            config: ArchitectAuthConfig {
                secret,
                base_url,
                session_ttl_seconds: self.session_ttl_seconds,
                email_password_enabled: self.email_password_enabled,
                require_email_verification: self.require_email_verification,
                oauth_signup_enabled: self.oauth_signup_enabled,
                oauth_token_storage_enabled: self.oauth_token_storage_enabled,
                passkey_rp_id: self.passkey_rp_id.unwrap_or_else(|| "localhost".into()),
                passkey_allowed_origins: self.passkey_allowed_origins,
                captcha: self.captcha,
                sms: self.sms,
                jwt: JwtConfig {
                    issuer: self.jwt_issuer.unwrap_or_else(|| "architect-auth".into()),
                    audience: self.jwt_audience.unwrap_or_else(|| "architect-auth".into()),
                    active_key_id: jwt_active_key_id,
                    keys: jwt_keys,
                },
                oidc: OidcProviderConfig {
                    issuer: oidc_issuer,
                    code_ttl_seconds: self.oidc_code_ttl_seconds,
                    access_token_ttl_seconds: self.oidc_access_token_ttl_seconds,
                    refresh_token_ttl_seconds: self.oidc_refresh_token_ttl_seconds,
                    require_pkce: self.oidc_require_pkce,
                    allow_dynamic_client_registration: self.oidc_allow_dynamic_client_registration,
                    clients: self.oidc_clients,
                },
                oauth_proxy: OAuthProxyConfig {
                    current_url: oauth_proxy_current_url,
                    production_url: oauth_proxy_production_url,
                    callback_path: self.oauth_proxy_callback_path,
                    max_age_seconds: self.oauth_proxy_max_age_seconds,
                    allowed_redirect_origins: oauth_proxy_allowed_redirect_origins,
                },
                one_tap: OneTapConfig {
                    client_id: self
                        .one_tap_client_id
                        .unwrap_or_else(|| "google-client".into()),
                    issuer: self.one_tap_issuer,
                    disable_signup: self.one_tap_disable_signup,
                },
                siwe: SiweConfig {
                    domain: self.siwe_domain.unwrap_or_else(|| "localhost".into()),
                    signup_enabled: self.siwe_signup_enabled,
                },
                breached_passwords: self.breached_passwords,
            },
            storage: self.storage,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("auth secret is required")]
    MissingSecret,
    #[error("auth secret must be at least 32 bytes")]
    SecretTooShort,
}
