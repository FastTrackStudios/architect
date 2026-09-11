//! Environment-driven configuration for the standalone auth server.
//!
//! The deployable instance (the private repo that ships this binary to
//! the cluster) supplies everything through env vars + mounted secret
//! files, so the same image serves dev, staging and prod without a
//! rebuild. Nothing here is FastTrackStudio-specific — the binary is a
//! generic architect-auth host, and the *deployment* decides the issuer,
//! the clients and the database.

use architect_auth::OidcClientConfig;

use crate::mail::MailConfig;

/// Anything the operator can set. Every field has an env var; only the
/// database URL and the signing secret are mandatory.
///
/// The simple fields read themselves (`#[derive(architect::Config)]`,
/// prefix `AUTH_`); the three that need parsing beyond a scalar — the
/// OIDC client list, the social providers, and the mail block's public
/// origin — are filled in by [`ServerConfig::load`].
#[derive(Clone, Debug, architect::Config)]
#[architect(prefix = "AUTH")]
pub struct ServerConfig {
    /// `host:port` the HTTP/WebSocket listener binds to.
    #[architect(default = "0.0.0.0:8080")]
    pub bind_addr: String,
    /// `SeaORM` connection string. `postgres://…` in the cluster,
    /// `sqlite://…` for local development.
    #[architect(secret)]
    pub database_url: String,
    /// Session-token signing secret. Minimum 32 bytes — architect-auth
    /// rejects anything shorter, which is the check that keeps a
    /// misconfigured deploy from issuing forgeable tokens.
    #[architect(secret)]
    pub secret: String,
    /// Public origin this server is reached at, e.g.
    /// `https://auth.fasttrackstudio.app`. Every OIDC endpoint in the
    /// discovery document is derived from it, so it must be the
    /// externally visible URL, not the pod address.
    #[architect(default = "http://localhost:8080")]
    pub base_url: String,
    /// OIDC issuer. Defaults to `base_url`; split out because an issuer
    /// is a stable identity that outlives a hostname change.
    pub oidc_issuer: Option<String>,
    /// Session lifetime in seconds.
    #[architect(default = 60 * 60 * 24 * 30)]
    pub session_ttl_seconds: i64,
    /// Whether a fresh account must verify its email before it can sign
    /// in. Off by default so a first deploy is usable before SMTP is
    /// wired up.
    #[architect(default = false)]
    pub require_email_verification: bool,
    /// Relying-party id for passkeys — the registrable domain, e.g.
    /// `fasttrackstudio.app`.
    pub passkey_rp_id: Option<String>,
    /// Origins allowed to call the browser-facing HTTP surface.
    pub cors_origins: Vec<String>,
    /// Registered OIDC clients, parsed from `AUTH_OIDC_CLIENTS` (JSON),
    /// with `AUTH_OIDC_CLIENTS_EXTRA` (or its `_FILE` form) merged over
    /// the top.
    #[architect(skip)]
    pub oidc_clients: Vec<OidcClientConfig>,
    /// Whether clients may self-register at `/oauth2/register`. Off by
    /// default: on a public issuer, dynamic registration is an open door.
    #[architect(env = "AUTH_OIDC_DYNAMIC_REGISTRATION", default = false)]
    pub oidc_allow_dynamic_client_registration: bool,
    /// Run migrations on boot. On in normal operation; an operator can
    /// turn it off to gate schema changes behind a separate job.
    #[architect(default = true)]
    pub run_migrations: bool,
    /// Load a sanitised snapshot into an empty *local* database on
    /// boot. `AUTH_IMPORT_SNAPSHOT=/path/to/auth-snapshot.json`.
    ///
    /// Take one with `cargo xtask auth-mirror`. Every imported account
    /// gets the published development password, which is why this is
    /// refused against anything but a local database.
    pub import_snapshot: Option<String>,
    /// Also serve the vox router over iroh, peer to peer. The endpoint's
    /// secret key persists at this path (a stable id across restarts);
    /// `AUTH_IROH_KEY_FILE=/var/lib/auth/iroh.key`. Unset means no p2p.
    #[architect(env = "AUTH_IROH_KEY_FILE")]
    pub iroh_key_path: Option<String>,
    /// Where to write the iroh endpoint id on boot, for other devices
    /// and agents to read. `AUTH_IROH_ID_FILE`.
    #[architect(env = "AUTH_IROH_ID_FILE")]
    pub iroh_id_path: Option<String>,
    /// Fill an empty *local* database with the fixed development cast,
    /// and serve it. `AUTH_DEV_SEED=1`.
    ///
    /// Off by default and refused against anything but a local
    /// database, because it creates accounts whose password is a
    /// published constant.
    #[architect(default = false)]
    pub dev_seed: bool,
    /// Outgoing mail. Without `AUTH_SMTP_HOST` the mailer only logs, and
    /// every flow that has to reach a person — verification, password
    /// reset — completes as far as minting a token and no further.
    #[architect(nested)]
    pub mail: MailConfig,
    /// Social providers and the linked-token policy. See the module docs.
    #[architect(skip)]
    pub social: SocialConfig,
}

/// One upstream OAuth provider this server may sign people in with and
/// link accounts against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocialProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Scopes requested at the provider, space-separated on the wire.
    pub scopes: Vec<String>,
}

/// Which providers are on, and what a relying party needs in order to be
/// handed a linked token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocialConfig {
    pub github: Option<SocialProviderConfig>,
    /// The NAM capture library. A PUBLIC client — it authenticates with
    /// PKCE and has no secret, which is why `client_secret` is empty here
    /// and why nothing in a deployment needs to hold one.
    pub tone3000: Option<SocialProviderConfig>,
    pub google: Option<SocialProviderConfig>,
    /// The OIDC scope an access token must carry for
    /// `GET /oauth2/linked-token`. Registered as an extra grantable
    /// scope with the OIDC provider.
    pub linked_token_scope: String,
    /// Send every provider to a mock server at this origin instead of
    /// the real one. `AUTH_SOCIAL_MOCK_URL`.
    ///
    /// All three or none: a deployment pointing some calls at a mock
    /// and others at the real provider fails in ways that look like the
    /// provider misbehaving.
    pub mock_url: Option<String>,
}

impl SocialConfig {
    /// The OIDC scope a bearer must carry to be handed `provider`'s token.
    ///
    /// GitHub reads the configured value so an existing deployment keeps the
    /// scope it already grants; every other provider carries its own, so a
    /// client trusted to act as someone on TONE3000 does not thereby reach
    /// their GitHub token.
    /// This deployment's settings for `provider`, if it is switched on.
    #[must_use]
    pub const fn provider_config_for(
        &self,
        provider: crate::social::Provider,
    ) -> Option<&SocialProviderConfig> {
        match provider {
            crate::social::Provider::GitHub => self.github.as_ref(),
            crate::social::Provider::Google => self.google.as_ref(),
            crate::social::Provider::Tone3000 => self.tone3000.as_ref(),
        }
    }

    #[must_use]
    pub const fn required_scope(&self, provider: crate::social::Provider) -> &str {
        match provider {
            crate::social::Provider::GitHub => self.linked_token_scope.as_str(),
            other => other.linked_token_scope(),
        }
    }

    pub const DEFAULT_LINKED_TOKEN_SCOPE: &'static str = "forge:github";
    /// TONE3000 documents no `scope` parameter on its authorize endpoint,
    /// and an empty scope is not the same as no scope.
    pub const DEFAULT_TONE3000_SCOPES: &'static str = "";
    pub const DEFAULT_GITHUB_SCOPES: &'static str = "repo read:user user:email";
    pub const DEFAULT_GOOGLE_SCOPES: &'static str = "openid email profile";

    /// Nothing configured — the routes 404 and the pages show no buttons.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            github: None,
            tone3000: None,
            google: None,
            linked_token_scope: Self::DEFAULT_LINKED_TOKEN_SCOPE.to_owned(),
            mock_url: None,
        }
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.github.is_some() || self.google.is_some() || self.tone3000.is_some()
    }
}

impl Default for SocialConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Env(#[from] architect::config::ConfigError),
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
    #[error("{var} is not valid JSON: {source}")]
    OidcClients {
        var: &'static str,
        #[source]
        source: serde_json::Error,
    },
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
    /// A configuration for a server on this machine: in-memory `SQLite`,
    /// no OIDC clients, no social providers, mail in log mode.
    ///
    /// Exists so that tests and local runs name only what they care
    /// about (`ServerConfig { dev_seed: true, ..ServerConfig::local() }`)
    /// rather than every field. There is deliberately no `Default`:
    /// this carries a fixed development secret, and a config that could
    /// be reached by accident in production is a config that will be.
    #[must_use]
    pub fn local() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".into(),
            database_url: "sqlite::memory:".into(),
            secret: "a-secret-at-least-32-bytes-long!!".into(),
            base_url: "http://localhost:8080".into(),
            oidc_issuer: None,
            session_ttl_seconds: 60 * 60 * 24 * 30,
            require_email_verification: false,
            passkey_rp_id: None,
            cors_origins: Vec::new(),
            oidc_clients: Vec::new(),
            oidc_allow_dynamic_client_registration: false,
            run_migrations: true,
            import_snapshot: None,
            iroh_key_path: None,
            iroh_id_path: None,
            dev_seed: false,
            social: SocialConfig::disabled(),
            mail: crate::mail::MailConfig {
                host: None,
                port: 587,
                username: None,
                password: None,
                from: "noreply@localhost".into(),
                base_url: "http://localhost:8080".into(),
            },
        }
    }

    /// Read the configuration from the process environment.
    ///
    /// The derived `from_env` reads every scalar; this finishes the job:
    /// the secret length check, the OIDC client lists, the social
    /// providers, and the mail block's public origin.
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Self::from_env()?;
        if config.secret.len() < 32 {
            return Err(ConfigError::SecretTooShort(config.secret.len()));
        }
        config.base_url = config.base_url.trim_end_matches('/').to_owned();

        // Two sources, because they have different secrecy. The public
        // clients (native apps, SPAs) hold nothing secret and belong in
        // the deployment's declarative config, in git. A CONFIDENTIAL
        // client's entry contains its `client_secret`, which must not be
        // — so it arrives through the same `_FILE` indirection as
        // AUTH_SECRET, out of a mounted Kubernetes secret.
        //
        // Extras are merged by `client_id`, last writer wins, so a
        // deployment can also override one declared client without
        // restating the rest.
        let mut oidc_clients = read_oidc_clients("AUTH_OIDC_CLIENTS")?;
        for extra in read_oidc_clients("AUTH_OIDC_CLIENTS_EXTRA")? {
            match oidc_clients
                .iter_mut()
                .find(|client| client.client_id == extra.client_id)
            {
                Some(existing) => *existing = extra,
                None => oidc_clients.push(extra),
            }
        }
        config.oidc_clients = oidc_clients;

        config.social = SocialConfig {
            github: read_social_provider(
                "AUTH_GITHUB_CLIENT_ID",
                "AUTH_GITHUB_CLIENT_SECRET",
                "AUTH_GITHUB_SCOPES",
                SocialConfig::DEFAULT_GITHUB_SCOPES,
            )?,
            google: read_social_provider(
                "AUTH_GOOGLE_CLIENT_ID",
                "AUTH_GOOGLE_CLIENT_SECRET",
                "AUTH_GOOGLE_SCOPES",
                SocialConfig::DEFAULT_GOOGLE_SCOPES,
            )?,
            tone3000: read_public_social_provider(
                "AUTH_TONE3000_CLIENT_ID",
                "AUTH_TONE3000_SCOPES",
                SocialConfig::DEFAULT_TONE3000_SCOPES,
            ),
            linked_token_scope: optional("AUTH_LINKED_TOKEN_SCOPE")
                .unwrap_or_else(|| SocialConfig::DEFAULT_LINKED_TOKEN_SCOPE.to_owned()),
            mock_url: optional("AUTH_SOCIAL_MOCK_URL"),
        };
        config.mail.base_url = config.base_url.clone();
        Ok(config)
    }

    /// The issuer, falling back to `base_url`.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.oidc_issuer.as_deref().unwrap_or(&self.base_url)
    }
}

/// A provider is present exactly when its client id is set. A client id
/// without a secret is a broken deployment, not a disabled provider —
/// it would send people to GitHub and fail on the way back — so it is
/// refused at boot.
fn read_social_provider(
    id_var: &'static str,
    secret_var: &'static str,
    scopes_var: &'static str,
    default_scopes: &str,
) -> Result<Option<SocialProviderConfig>, ConfigError> {
    let Some(client_id) = optional(id_var) else {
        return Ok(None);
    };
    let client_secret = read_secret(secret_var)?;
    let scopes = optional(scopes_var)
        .unwrap_or_else(|| default_scopes.to_owned())
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    Ok(Some(SocialProviderConfig {
        client_id,
        client_secret,
        scopes,
    }))
}

/// A provider that authenticates with PKCE rather than a secret.
///
/// Deliberately a different function from [`read_social_provider`] rather
/// than an optional secret on that one: "this deployment forgot to mount
/// GitHub's secret" and "this provider has no secret by design" are
/// different situations, and only one of them should start the server.
fn read_public_social_provider(
    id_var: &'static str,
    scopes_var: &'static str,
    default_scopes: &str,
) -> Option<SocialProviderConfig> {
    let client_id = optional(id_var)?;
    let scopes = optional(scopes_var)
        .unwrap_or_else(|| default_scopes.to_owned())
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    Some(SocialProviderConfig {
        client_id,
        client_secret: String::new(),
        scopes,
    })
}

/// Read and parse a JSON client list from `<VAR>_FILE` if set, else
/// `<VAR>`. An unset or empty variable is not an error — a deployment
/// with only public clients sets no extras, and vice versa.
fn read_oidc_clients(var: &'static str) -> Result<Vec<OidcClientConfig>, ConfigError> {
    let Some(raw) = optional_secret(var)? else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Vec<OidcClientJson> =
        serde_json::from_str(&raw).map_err(|source| ConfigError::OidcClients { var, source })?;
    Ok(parsed.into_iter().map(OidcClientConfig::from).collect())
}

/// Read a secret from `<VAR>_FILE` if set, else `<VAR>` — the
/// framework's reader, with this crate's error type.
fn read_secret(var: &'static str) -> Result<String, ConfigError> {
    optional_secret(var)?.ok_or(ConfigError::Missing(var))
}

fn optional_secret(var: &'static str) -> Result<Option<String>, ConfigError> {
    Ok(architect::config::secret(var)?)
}

fn optional(var: &str) -> Option<String> {
    architect::config::var(var)
}
