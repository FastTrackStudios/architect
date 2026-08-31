//! Environment-driven configuration for the standalone auth server.
//!
//! The deployable instance (the private repo that ships this binary to
//! the cluster) supplies everything through env vars + mounted secret
//! files, so the same image serves dev, staging and prod without a
//! rebuild. Nothing here is FastTrackStudio-specific — the binary is a
//! generic architect-auth host, and the *deployment* decides the issuer,
//! the clients and the database.

use std::env;
use std::fs;

use architect_auth::OidcClientConfig;

/// Anything the operator can set. Every field has an env var; only the
/// database URL and the signing secret are mandatory.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// `host:port` the HTTP/WebSocket listener binds to.
    pub bind_addr: String,
    /// SeaORM connection string. `postgres://…` in the cluster,
    /// `sqlite://…` for local development.
    pub database_url: String,
    /// Session-token signing secret. Minimum 32 bytes — architect-auth
    /// rejects anything shorter, which is the check that keeps a
    /// misconfigured deploy from issuing forgeable tokens.
    pub secret: String,
    /// Public origin this server is reached at, e.g.
    /// `https://auth.fasttrackstudio.app`. Every OIDC endpoint in the
    /// discovery document is derived from it, so it must be the
    /// externally visible URL, not the pod address.
    pub base_url: String,
    /// OIDC issuer. Defaults to `base_url`; split out because an issuer
    /// is a stable identity that outlives a hostname change.
    pub oidc_issuer: Option<String>,
    /// Session lifetime in seconds.
    pub session_ttl_seconds: i64,
    /// Whether a fresh account must verify its email before it can sign
    /// in. Off by default so a first deploy is usable before SMTP is
    /// wired up.
    pub require_email_verification: bool,
    /// Relying-party id for passkeys — the registrable domain, e.g.
    /// `fasttrackstudio.app`.
    pub passkey_rp_id: Option<String>,
    /// Origins allowed to call the browser-facing HTTP surface.
    pub cors_origins: Vec<String>,
    /// Registered OIDC clients, parsed from `AUTH_OIDC_CLIENTS` (JSON).
    pub oidc_clients: Vec<OidcClientConfig>,
    /// Whether clients may self-register at `/oauth2/register`. Off by
    /// default: on a public issuer, dynamic registration is an open door.
    pub oidc_allow_dynamic_client_registration: bool,
    /// Run migrations on boot. On in normal operation; an operator can
    /// turn it off to gate schema changes behind a separate job.
    pub run_migrations: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required but not set")]
    Missing(&'static str),
    #[error("{var} is set but unreadable: {source}")]
    Unreadable {
        var: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{var} must be a {expected}, got `{value}`")]
    Invalid {
        var: &'static str,
        expected: &'static str,
        value: String,
    },
    #[error("AUTH_OIDC_CLIENTS is not valid JSON: {0}")]
    OidcClients(#[source] serde_json::Error),
    #[error(
        "AUTH_SECRET must be at least 32 bytes (got {0}) — architect-auth refuses shorter keys"
    )]
    SecretTooShort(usize),
}

/// An OIDC client as written in `AUTH_OIDC_CLIENTS`.
///
/// A separate type from [`OidcClientConfig`] because the engine's config
/// struct derives no serde — and because the JSON form gets defaults for
/// the fields a deployment rarely sets.
#[derive(Debug, serde::Deserialize)]
struct OidcClientJson {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    name: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(default = "default_scopes")]
    scopes: Vec<String>,
    /// A public client (native app, SPA) holds no secret and must use
    /// PKCE.
    #[serde(default)]
    public_client: bool,
    /// First-party apps skip the consent screen — the user is not
    /// granting a third party access, they are just signing in.
    #[serde(default)]
    skip_consent: bool,
    #[serde(default)]
    disabled: bool,
}

fn default_scopes() -> Vec<String> {
    vec!["openid".into(), "profile".into(), "email".into()]
}

impl From<OidcClientJson> for OidcClientConfig {
    fn from(value: OidcClientJson) -> Self {
        let name = value.name.unwrap_or_else(|| value.client_id.clone());
        Self {
            client_id: value.client_id,
            client_secret: value.client_secret,
            name,
            redirect_uris: value.redirect_uris,
            scopes: value.scopes,
            public_client: value.public_client,
            skip_consent: value.skip_consent,
            disabled: value.disabled,
        }
    }
}

impl ServerConfig {
    /// Read the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let secret = read_secret("AUTH_SECRET")?;
        if secret.len() < 32 {
            return Err(ConfigError::SecretTooShort(secret.len()));
        }

        let database_url = read_secret("AUTH_DATABASE_URL")?;

        let base_url = env::var("AUTH_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();

        let oidc_clients = match env::var("AUTH_OIDC_CLIENTS") {
            Ok(raw) if !raw.trim().is_empty() => {
                let parsed: Vec<OidcClientJson> =
                    serde_json::from_str(&raw).map_err(ConfigError::OidcClients)?;
                parsed.into_iter().map(OidcClientConfig::from).collect()
            }
            _ => Vec::new(),
        };

        Ok(Self {
            bind_addr: env::var("AUTH_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            database_url,
            secret,
            base_url,
            oidc_issuer: optional("AUTH_OIDC_ISSUER"),
            session_ttl_seconds: parse_or("AUTH_SESSION_TTL_SECONDS", 60 * 60 * 24 * 30)?,
            require_email_verification: flag("AUTH_REQUIRE_EMAIL_VERIFICATION", false)?,
            passkey_rp_id: optional("AUTH_PASSKEY_RP_ID"),
            cors_origins: list("AUTH_CORS_ORIGINS"),
            oidc_clients,
            oidc_allow_dynamic_client_registration: flag("AUTH_OIDC_DYNAMIC_REGISTRATION", false)?,
            run_migrations: flag("AUTH_RUN_MIGRATIONS", true)?,
        })
    }

    /// The issuer, falling back to `base_url`.
    pub fn issuer(&self) -> &str {
        self.oidc_issuer.as_deref().unwrap_or(&self.base_url)
    }
}

/// Read a secret from `<VAR>_FILE` if set, else `<VAR>`.
///
/// The `_FILE` indirection is how Kubernetes secrets should be
/// consumed: a mounted file never shows up in `/proc/<pid>/environ`, in
/// a crash dump, or in `kubectl describe pod`.
fn read_secret(var: &'static str) -> Result<String, ConfigError> {
    let file_var = format!("{var}_FILE");
    if let Ok(path) = env::var(&file_var) {
        let contents =
            fs::read_to_string(&path).map_err(|source| ConfigError::Unreadable { var, source })?;
        return Ok(contents.trim().to_owned());
    }
    env::var(var).map_err(|_| ConfigError::Missing(var))
}

fn optional(var: &str) -> Option<String> {
    env::var(var).ok().filter(|value| !value.trim().is_empty())
}

fn list(var: &str) -> Vec<String> {
    env::var(var)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn flag(var: &'static str, default: bool) -> Result<bool, ConfigError> {
    match env::var(var) {
        Err(_) => Ok(default),
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(ConfigError::Invalid {
                var,
                expected: "boolean",
                value: raw,
            }),
        },
    }
}

fn parse_or(var: &'static str, default: i64) -> Result<i64, ConfigError> {
    match env::var(var) {
        Err(_) => Ok(default),
        Ok(raw) => raw.trim().parse().map_err(|_| ConfigError::Invalid {
            var,
            expected: "integer",
            value: raw,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_client_json_defaults_to_openid_profile_email() {
        let parsed: Vec<OidcClientJson> = serde_json::from_str(
            r#"[{"client_id":"task","redirect_uris":["https://task.fasttrackstudio.app/callback"]}]"#,
        )
        .expect("parse");
        let client = OidcClientConfig::from(parsed.into_iter().next().expect("one client"));
        assert_eq!(client.scopes, vec!["openid", "profile", "email"]);
        // The display name falls back to the id rather than being empty.
        assert_eq!(client.name, "task");
        assert!(!client.public_client);
    }

    #[test]
    fn flag_rejects_nonsense_rather_than_defaulting() {
        // Safety-relevant: silently treating `AUTH_REQUIRE_EMAIL_VERIFICATION=maybe`
        // as `false` would weaken the deploy without saying so.
        unsafe { env::set_var("AUTH_TEST_FLAG", "maybe") };
        assert!(flag("AUTH_TEST_FLAG", true).is_err());
        unsafe { env::remove_var("AUTH_TEST_FLAG") };
    }
}
