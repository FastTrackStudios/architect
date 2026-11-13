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
        "{var}: client `{client_id}` has the key `{key}`, which this issuer does not know and \
         which looks like a misspelled `{meant}`; refusing to start rather than run without \
         the `{meant}` it meant"
    )]
    OidcClientKey {
        var: &'static str,
        client_id: String,
        key: String,
        meant: &'static str,
    },
    #[error(
        "{var}: client `{client_id}` has the key `{key}` holding an object, which this issuer \
         does not read; client settings sit at the top of the client, so refusing to start \
         rather than ignore settings written inside it"
    )]
    OidcClientNested {
        var: &'static str,
        client_id: String,
        key: String,
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
/// the fields a deployment rarely sets. A key it does not know is ignored
/// with a warning at start-up naming the client and the key (a key a later
/// version adds must not stop the issuer for every app it signs in),
/// except a near-miss or synonym of a key that matters ([`GUARDED_KEYS`],
/// [`SYNONYMS`]): a misspelled `first_party`, or an `enabled: false` meant
/// as `disabled: true`, must not quietly leave the client as it was, so
/// that is an error. So is an unknown key holding an object, since settings
/// nested in it would be ignored.
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
    /// The issuer's own app: its access tokens answer `whoami` and
    /// `list_organizations`. Never set it for a third party.
    #[serde(default)]
    first_party: bool,
    /// Single-use refresh tokens (public clients only).
    #[serde(default)]
    rotate_refresh_tokens: bool,
    #[serde(default)]
    disabled: bool,
    /// Every key this version does not know; see [`unknown_key`]. A map
    /// because `flatten` needs one; the values are read only to refuse an
    /// object, never kept past parsing nor reported.
    #[serde(flatten)]
    unknown: std::collections::BTreeMap<String, serde_json::Value>,
}

/// The client keys a near-miss of which is refused rather than ignored,
/// since the deployment meant the key: every key this version reads, and
/// the session key a later issuer reads (`LATER_KEYS`). Each grants,
/// withholds or routes something, and a misspelled boolean reads `false`.
const GUARDED_KEYS: &[&str] = &[
    "client_id",
    "client_secret",
    "redirect_uris",
    "scopes",
    "public_client",
    "skip_consent",
    "first_party",
    "rotate_refresh_tokens",
    "disabled",
    "issue_sessions",
];

/// Guarded keys this version does not read yet: written exactly, they are
/// ignored like any unknown key (a config written for a newer issuer still
/// starts this one); a near-miss is still refused.
const LATER_KEYS: &[&str] = &["issue_sessions"];

/// Other words for the two keys that widen access when missing: a client
/// turned off as `enabled: false` or `revoked: true` would stay on, and
/// rotation asked for under another name would leave refresh tokens
/// reusable. `(folded word, meant)`; a folded key equal to the word (or to
/// `is` and the word), or for `rotat` containing it, is refused.
const SYNONYMS: &[(&str, &str)] = &[
    ("enabled", "disabled"),
    ("enable", "disabled"),
    ("active", "disabled"),
    ("inactive", "disabled"),
    ("revoked", "disabled"),
    ("suspended", "disabled"),
    ("blocked", "disabled"),
    ("rotat", "rotate_refresh_tokens"),
];

/// What an unknown client key is: `Some(meant)` when it is a near-miss of
/// the guarded key `meant` (the same once case, `-`, `_` and spaces are
/// ignored, or within two edits of it; one edit for a key of under eight
/// letters) or one of its [`SYNONYMS`], else `None`, an unrelated key that
/// is ignored.
fn unknown_key(key: &str) -> Option<&'static str> {
    fn folded(key: &str) -> Vec<char> {
        key.chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .flat_map(char::to_lowercase)
            .collect()
    }
    if LATER_KEYS.contains(&key) {
        return None;
    }
    let key = folded(key);
    let text: String = key.iter().collect();
    if let Some(&(_, meant)) = SYNONYMS.iter().find(|(word, _)| {
        if *word == "rotat" {
            text.contains(word)
        } else {
            text == *word || text.strip_prefix("is") == Some(*word)
        }
    }) {
        return Some(meant);
    }
    GUARDED_KEYS.iter().copied().find(|meant| {
        let meant = folded(meant);
        let near = if meant.len() < 8 { 1 } else { 2 };
        edit_distance(&key, &meant) <= near
    })
}

/// Levenshtein distance between `a` and `b`, in characters.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    // `prev`: the distance from the part of `a` read so far to each
    // prefix of `b`.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = Vec::with_capacity(prev.len());
        row.push(i.saturating_add(1));
        for ((cb, &diagonal), &above) in b.iter().zip(&prev).zip(prev.iter().skip(1)) {
            let left = row.last().copied().unwrap_or(usize::MAX);
            row.push(if ca == cb {
                diagonal
            } else {
                diagonal.min(above).min(left).saturating_add(1)
            });
        }
        prev = row;
    }
    prev.last().copied().unwrap_or(0)
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
            first_party: value.first_party,
            rotate_refresh_tokens: value.rotate_refresh_tokens,
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
    let (clients, ignored) = parse_oidc_clients(var, &raw)?;
    for (client_id, key) in ignored {
        tracing::warn!(
            var,
            client_id,
            key,
            "OIDC CLIENT CONFIG: unknown key IGNORED (this issuer does not know it; check its \
             spelling, or whether it needs a newer issuer)"
        );
    }
    Ok(clients)
}

/// `raw` (a JSON client list from `var`) as clients, and the unknown keys
/// it ignored as `(client_id, key)`. A near-miss of one of the
/// [`GUARDED_KEYS`] is an error instead.
fn parse_oidc_clients(
    var: &'static str,
    raw: &str,
) -> Result<(Vec<OidcClientConfig>, Vec<(String, String)>), ConfigError> {
    let parsed: Vec<OidcClientJson> =
        serde_json::from_str(raw).map_err(|source| ConfigError::OidcClients { var, source })?;
    let mut ignored = Vec::new();
    for client in &parsed {
        for (key, value) in &client.unknown {
            if value.is_object() {
                return Err(ConfigError::OidcClientNested {
                    var,
                    client_id: client.client_id.clone(),
                    key: key.clone(),
                });
            }
            if let Some(meant) = unknown_key(key) {
                return Err(ConfigError::OidcClientKey {
                    var,
                    client_id: client.client_id.clone(),
                    key: key.clone(),
                    meant,
                });
            }
            ignored.push((client.client_id.clone(), key.clone()));
        }
    }
    Ok((
        parsed.into_iter().map(OidcClientConfig::from).collect(),
        ignored,
    ))
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

#[cfg(test)]
mod oidc_client_json_tests {
    use super::{ConfigError, parse_oidc_clients};

    fn client_with(extra: &str) -> String {
        format!(
            r#"[{{"client_id":"architect","redirect_uris":["https://a.example/cb"],"first_party":true{extra}}}]"#
        )
    }

    /// A key this version does not know (a later version's, a note) is
    /// ignored and reported with its client; the client is read as usual.
    /// Its value is never reported.
    #[test]
    fn an_unknown_client_key_is_ignored_and_named() {
        for key in ["issue_sessions", "comment", "note", "logo_uri", "contacts"] {
            let raw = client_with(&format!(r#","{key}":"secret-looking-value""#));
            let (clients, ignored) =
                parse_oidc_clients("AUTH_OIDC_CLIENTS", &raw).expect("an unknown key starts");
            assert!(clients[0].first_party, "{key}");
            assert_eq!(ignored, vec![("architect".to_owned(), key.to_owned())]);
            assert!(!format!("{ignored:?}").contains("secret-looking-value"));
        }
    }

    /// A key that looks like one that grants something is refused, not
    /// read as `false`: misspelled, in another case, or with `-` for `_`.
    #[test]
    fn a_near_miss_of_a_granting_key_is_an_error() {
        for (key, meant) in [
            ("first_pary", "first_party"),
            ("firstParty", "first_party"),
            ("First-Party", "first_party"),
            ("rotate_refresh_token", "rotate_refresh_tokens"),
            ("rotate-refresh-tokens", "rotate_refresh_tokens"),
            ("skip_consnt", "skip_consent"),
            ("SkipConsent", "skip_consent"),
            ("public-client", "public_client"),
            ("disable", "disabled"),
            ("redirect_uri", "redirect_uris"),
            ("redirectUris", "redirect_uris"),
            ("redirect_urls", "redirect_uris"),
            ("issue_session", "issue_sessions"),
            ("issueSessions", "issue_sessions"),
            ("issue-sessions", "issue_sessions"),
            ("isue_sessions", "issue_sessions"),
            ("scope", "scopes"),
            ("client_secrets", "client_secret"),
            ("enabled", "disabled"),
            ("Active", "disabled"),
            ("revoked", "disabled"),
            ("is-suspended", "disabled"),
            ("refresh_token_rotation", "rotate_refresh_tokens"),
            ("rotate_tokens", "rotate_refresh_tokens"),
        ] {
            let raw = client_with(&format!(r#","{key}":true"#));
            match parse_oidc_clients("AUTH_OIDC_CLIENTS_EXTRA", &raw) {
                Err(ConfigError::OidcClientKey {
                    client_id,
                    key: k,
                    meant: m,
                    ..
                }) => {
                    assert_eq!(
                        (client_id.as_str(), k.as_str(), m),
                        ("architect", key, meant)
                    );
                }
                other => panic!("{key}: {other:?}"),
            }
        }
    }

    /// Settings nested in an unknown object would be ignored, so an
    /// unknown key holding an object is refused; its contents are not
    /// reported.
    #[test]
    fn an_unknown_key_holding_an_object_is_an_error() {
        let raw = client_with(r#","settings":{"disabled":true,"rotate_refresh_tokens":true}"#);
        match parse_oidc_clients("AUTH_OIDC_CLIENTS", &raw) {
            Err(e @ ConfigError::OidcClientNested { .. }) => {
                let shown = e.to_string();
                assert!(
                    shown.contains("`settings`") && !shown.contains("rotate"),
                    "{shown}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// The keys this version knows read as before.
    #[test]
    fn known_keys_read_with_nothing_ignored() {
        let raw = client_with(r#","rotate_refresh_tokens":true,"public_client":true"#);
        let (clients, ignored) = parse_oidc_clients("AUTH_OIDC_CLIENTS", &raw).unwrap();
        assert!(ignored.is_empty());
        assert!(clients[0].rotate_refresh_tokens && clients[0].public_client);
    }
}
