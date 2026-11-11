//! The isolation guard: run before any process of the stack starts.
//!
//! Every URL, issuer, bind address and endpoint the stack hands a process
//! must be this machine (loopback) or a private dev address, and none may
//! name production. The inherited environment must carry no production
//! credential (an `AUTH_SECRET`, a database URL, a bucket's keys). The one
//! call off the machine, the mirror's authenticated GET of a snapshot, is
//! not a process's setting and is not checked here.

/// Production's registrable domain: any host under it is production.
pub const PRODUCTION_DOMAIN: &str = "fasttrackstudio.app";

/// Variables a dev shell must not carry: they would be production's.
const FORBIDDEN_INHERITED: [&str; 4] = [
    "AUTH_SECRET",
    "AUTH_SECRET_FILE",
    "AUTH_DATABASE_URL",
    "AUTH_DATABASE_URL_FILE",
];

/// Prefixes no inherited variable of a dev shell may have.
const FORBIDDEN_INHERITED_PREFIXES: [&str; 1] = ["ARCHITECT_BLOB_S3_"];

/// Why the guard refused one setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub key: String,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.why)
    }
}

/// The hosts that count as production: [`PRODUCTION_DOMAIN`], plus
/// `extra` (a test names a loopback port standing in for production).
#[derive(Debug, Clone, Default)]
pub struct Production {
    extra: Vec<String>,
}

impl Production {
    #[must_use]
    pub const fn with_extra(extra: Vec<String>) -> Self {
        Self { extra }
    }

    /// Whether `authority` (`host` or `host:port`) is production.
    #[must_use]
    pub fn is(&self, authority: &str) -> bool {
        let authority = authority.to_ascii_lowercase();
        let host = split_host(&authority).0;
        let host = host.trim_end_matches('.');
        host == PRODUCTION_DOMAIN
            || host.ends_with(&format!(".{PRODUCTION_DOMAIN}"))
            || self
                .extra
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&authority) || p.eq_ignore_ascii_case(host))
    }
}

/// Check the settings the stack gives a process: every URL in a value
/// (anywhere in it: a JSON list of redirect URIs too) and every bind
/// address. Empty when all is well.
#[must_use]
pub fn check_settings(vars: &[(String, String)], production: &Production) -> Vec<Refusal> {
    let mut refused = Vec::new();
    for (key, value) in vars {
        if value.to_ascii_lowercase().contains(PRODUCTION_DOMAIN) {
            refused.push(Refusal {
                key: key.clone(),
                why: format!("`{value}` names production ({PRODUCTION_DOMAIN})"),
            });
            continue;
        }
        if is_bind_key(key) {
            if let Some(why) = bind_problem(value, production) {
                refused.push(Refusal {
                    key: key.clone(),
                    why,
                });
            }
            continue;
        }
        for authority in authorities(value) {
            if let Some(why) = authority_problem(&authority, production) {
                refused.push(Refusal {
                    key: key.clone(),
                    why,
                });
            }
        }
    }
    refused
}

/// Check what the dev shell itself carries: no production secret, no
/// bucket credentials.
pub fn check_inherited<'a>(vars: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<Refusal> {
    vars.into_iter()
        .filter(|(key, value)| {
            !value.is_empty()
                && (FORBIDDEN_INHERITED.contains(key)
                    || FORBIDDEN_INHERITED_PREFIXES
                        .iter()
                        .any(|p| key.starts_with(p)))
        })
        .map(|(key, _)| Refusal {
            key: key.to_owned(),
            why: "the dev shell must not carry production credentials (the stack makes its own \
                  secret and uses no bucket); unset it before `devenv up`"
                .to_owned(),
        })
        .collect()
}

fn is_bind_key(key: &str) -> bool {
    key.ends_with("_BIND") || key.ends_with("_BIND_ADDR")
}

fn bind_problem(value: &str, production: &Production) -> Option<String> {
    if production.is(value) {
        return Some(format!("`{value}` is production"));
    }
    let host = split_host(value).0;
    (!is_loopback(host)).then(|| {
        format!("`{value}` is not loopback: the stack listens on this machine only (127.0.0.1)")
    })
}

fn authority_problem(authority: &str, production: &Production) -> Option<String> {
    if production.is(authority) {
        return Some(format!("`{authority}` is production"));
    }
    let host = split_host(authority).0;
    (!is_loopback(host) && !is_private(host))
        .then(|| format!("`{authority}` is neither this machine nor a private dev address"))
}

/// The authority (`host[:port]`, userinfo dropped) of every `scheme://…`
/// in `value`.
fn authorities(value: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find("://") {
        let after = rest.get(at.saturating_add(3)..).unwrap_or_default();
        let end = after
            .find(['/', '?', '#', '"', '\'', ',', ' ', '\n', '\t'])
            .unwrap_or(after.len());
        let authority = after.get(..end).unwrap_or_default();
        let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
        if !authority.is_empty() {
            found.push(authority.to_ascii_lowercase());
        }
        rest = after.get(end..).unwrap_or_default();
    }
    found
}

/// `host` and the port, from `host`, `host:port` or `[v6]:port`.
fn split_host(authority: &str) -> (&str, Option<&str>) {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split_once(']').map_or((rest, None), |(host, after)| {
            (host, after.strip_prefix(':'))
        });
    }
    match authority.rsplit_once(':') {
        // An IPv6 literal without brackets has more than one colon.
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        _ => (authority, None),
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// RFC 1918 and unique-local IPv6: a dev box on a LAN, never the internet.
fn is_private(host: &str) -> bool {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private(),
        Ok(std::net::IpAddr::V6(v6)) => v6.segments().first().is_some_and(|s| s & 0xfe00 == 0xfc00),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn refused(pairs: &[(&str, &str)]) -> Vec<String> {
        check_settings(&vars(pairs), &Production::default())
            .into_iter()
            .map(|r| r.key)
            .collect()
    }

    #[test]
    fn loopback_and_private_addresses_pass() {
        assert!(
            refused(&[
                ("AUTH_BASE_URL", "http://127.0.0.1:7411"),
                ("ARCHITECT_ISSUER", "http://localhost:7411/"),
                ("ARCHITECT_ORG_HOST", "http://[::1]:7412"),
                ("ARCHITECT_BLOB_S3_ENDPOINT", "http://192.168.1.20:3900"),
                ("ARCHITECT_BIND", "127.0.0.1:7412"),
                ("ARCHITECT_IROH", "local"),
                (
                    "AUTH_OIDC_CLIENTS",
                    r#"[{"redirect_uris":["http://127.0.0.1:7413/auth/callback"]}]"#
                ),
            ])
            .is_empty()
        );
    }

    #[test]
    fn production_hosts_are_refused_wherever_they_appear() {
        assert_eq!(
            refused(&[
                ("ARCHITECT_ISSUER", "https://auth.fasttrackstudio.app"),
                ("AUTH_BASE_URL", "https://FastTrackStudio.app/"),
                (
                    "AUTH_OIDC_CLIENTS",
                    r#"[{"redirect_uris":["https://architect.fasttrackstudio.app/cb"]}]"#
                ),
                ("ARCHITECT_ENROL_AT", "fasttrackstudio.app."),
            ]),
            [
                "ARCHITECT_ISSUER",
                "AUTH_BASE_URL",
                "AUTH_OIDC_CLIENTS",
                "ARCHITECT_ENROL_AT"
            ]
        );
    }

    #[test]
    fn a_public_host_or_bucket_is_refused() {
        assert_eq!(
            refused(&[
                ("ARCHITECT_BLOB_S3_ENDPOINT", "https://s3.amazonaws.com"),
                ("ARCHITECT_ISSUER", "https://auth.example.com"),
                ("ARCHITECT_PEERS", "abc@http://8.8.8.8:7777"),
            ]),
            [
                "ARCHITECT_BLOB_S3_ENDPOINT",
                "ARCHITECT_ISSUER",
                "ARCHITECT_PEERS"
            ]
        );
    }

    #[test]
    fn a_bind_off_loopback_is_refused() {
        assert_eq!(
            refused(&[
                ("ARCHITECT_BIND", "0.0.0.0:7412"),
                ("AUTH_BIND_ADDR", "192.168.1.2:7411"),
                ("MOCK_OAUTH_BIND_ADDR", "127.0.0.1:7410"),
            ]),
            ["ARCHITECT_BIND", "AUTH_BIND_ADDR"]
        );
    }

    #[test]
    fn a_stand_in_for_production_is_refused_like_production() {
        // The tests' override: a loopback port playing production.
        let production = Production::with_extra(vec!["127.0.0.1:7999".to_owned()]);
        let refused = check_settings(
            &vars(&[
                ("ARCHITECT_ISSUER", "http://127.0.0.1:7999"),
                ("AUTH_BASE_URL", "http://127.0.0.1:7411"),
            ]),
            &production,
        );
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused.first().unwrap().key, "ARCHITECT_ISSUER");
    }

    #[test]
    fn production_credentials_in_the_shell_are_refused() {
        let shell = [
            ("AUTH_SECRET", "x"),
            ("ARCHITECT_BLOB_S3_ACCESS_KEY_ID", "AKIA…"),
            ("AUTH_ADMIN_TOKEN", "the mirror's, allowed"),
            ("AUTH_DATABASE_URL_FILE", ""),
            ("PATH", "/bin"),
        ];
        let keys: Vec<_> = check_inherited(shell).into_iter().map(|r| r.key).collect();
        assert_eq!(keys, ["AUTH_SECRET", "ARCHITECT_BLOB_S3_ACCESS_KEY_ID"]);
    }

    #[test]
    fn authorities_are_found_inside_any_value() {
        assert_eq!(
            authorities("a http://u:p@Host:1/x, then wss://h2?q"),
            ["host:1", "h2"]
        );
        assert_eq!(split_host("[::1]:80"), ("::1", Some("80")));
        assert_eq!(split_host("::1"), ("::1", None));
    }
}
