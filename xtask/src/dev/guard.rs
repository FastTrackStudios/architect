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
    let mut refused = check_auto_sign_in(vars, production);
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

/// The dev stack's auto sign-in settings, each with the issuer settings
/// of its process that must be loopback for it to start: the auth
/// server's (`AUTH_DEV_AUTO_SIGN_IN`, `apps/auth-server/src/dev.rs`) and
/// the window's (`ARCHITECT_DEV_SIGN_IN`, `apps/desktop/src/dev.rs`).
pub const AUTO_SIGN_IN: [(&str, &[&str]); 2] = [
    (
        "AUTH_DEV_AUTO_SIGN_IN",
        &["AUTH_BASE_URL", "AUTH_BIND_ADDR"],
    ),
    ("ARCHITECT_DEV_SIGN_IN", &["ARCHITECT_AUTH_ISSUER"]),
];

/// The domain of the demo seed's accounts: the only ones an auto sign-in
/// may name (never a real address, never a mirrored one).
pub const DEMO_DOMAIN: &str = "local.test";

/// Whether `email` is one of the demo seed's (`…@local.test`).
#[must_use]
pub fn is_demo_account(email: &str) -> bool {
    email
        .trim()
        .rsplit_once('@')
        .is_some_and(|(name, domain)| !name.is_empty() && domain.eq_ignore_ascii_case(DEMO_DOMAIN))
}

/// An auto sign-in only with a demo account and an issuer on this machine
/// (loopback, never merely a private address, never production), set
/// beside it.
fn check_auto_sign_in(vars: &[(String, String)], production: &Production) -> Vec<Refusal> {
    let get = |key: &str| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let mut refused = Vec::new();
    for (key, issuers) in AUTO_SIGN_IN {
        let Some(email) = get(key).filter(|v| !v.trim().is_empty()) else {
            continue;
        };
        if !is_demo_account(email) {
            refused.push(Refusal {
                key: key.to_owned(),
                why: format!(
                    "`{email}` is not a demo account (…@{DEMO_DOMAIN}): an auto sign-in is the demo \
                     seed's account, never a real one"
                ),
            });
        }
        for issuer in issuers {
            let Some(value) = get(issuer) else {
                refused.push(Refusal {
                    key: key.to_owned(),
                    why: format!(
                        "{issuer} is unset: an auto sign-in needs an issuer on this machine"
                    ),
                });
                continue;
            };
            let authority = if is_bind_key(issuer) {
                value.to_ascii_lowercase()
            } else {
                authorities(value).into_iter().next().unwrap_or_default()
            };
            if production.is(&authority) || !is_loopback(split_host(&authority).0) {
                refused.push(Refusal {
                    key: key.to_owned(),
                    why: format!(
                        "{issuer}=`{value}` is not loopback: an auto sign-in signs in on this machine only"
                    ),
                });
            }
        }
    }
    refused
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
    fn an_auto_sign_in_with_the_demo_account_on_loopback_passes() {
        assert!(
            refused(&[
                ("AUTH_DEV_AUTO_SIGN_IN", "ada@local.test"),
                ("AUTH_BASE_URL", "http://127.0.0.1:7411"),
                ("AUTH_BIND_ADDR", "127.0.0.1:7411"),
            ])
            .is_empty()
        );
        assert!(
            refused(&[
                ("ARCHITECT_DEV_SIGN_IN", "ada@local.test"),
                ("ARCHITECT_AUTH_ISSUER", "http://localhost:7411/"),
            ])
            .is_empty()
        );
    }

    #[test]
    fn an_auto_sign_in_off_loopback_or_at_production_is_refused() {
        // A private dev address passes the guard for anything else, but
        // never for an auto sign-in.
        assert_eq!(
            refused(&[
                ("ARCHITECT_DEV_SIGN_IN", "ada@local.test"),
                ("ARCHITECT_AUTH_ISSUER", "http://192.168.1.20:7411"),
            ]),
            ["ARCHITECT_DEV_SIGN_IN"]
        );
        assert_eq!(
            refused(&[
                ("AUTH_DEV_AUTO_SIGN_IN", "ada@local.test"),
                ("AUTH_BASE_URL", "http://127.0.0.1:7411"),
                ("AUTH_BIND_ADDR", "0.0.0.0:7411"),
            ]),
            ["AUTH_DEV_AUTO_SIGN_IN", "AUTH_BIND_ADDR"]
        );
        // Production by name, and a loopback port standing in for it.
        let named = refused(&[
            ("ARCHITECT_DEV_SIGN_IN", "ada@local.test"),
            ("ARCHITECT_AUTH_ISSUER", "https://auth.fasttrackstudio.app"),
        ]);
        assert!(
            named
                .iter()
                .filter(|k| *k == "ARCHITECT_DEV_SIGN_IN")
                .count()
                == 1,
            "{named:?}"
        );
        let production = Production::with_extra(vec!["127.0.0.1:7999".to_owned()]);
        let stand_in = check_settings(
            &vars(&[
                ("ARCHITECT_DEV_SIGN_IN", "ada@local.test"),
                ("ARCHITECT_AUTH_ISSUER", "http://127.0.0.1:7999"),
            ]),
            &production,
        );
        assert!(
            stand_in.iter().any(|r| r.key == "ARCHITECT_DEV_SIGN_IN"),
            "{stand_in:?}"
        );
        // No issuer beside it.
        assert_eq!(
            refused(&[("ARCHITECT_DEV_SIGN_IN", "ada@local.test")]),
            ["ARCHITECT_DEV_SIGN_IN"]
        );
    }

    #[test]
    fn an_auto_sign_in_names_only_a_demo_account() {
        for email in [
            "cody@example.com",
            "user-7@local.invalid",
            "ada@local.test.evil.com",
        ] {
            assert_eq!(
                refused(&[
                    ("ARCHITECT_DEV_SIGN_IN", email),
                    ("ARCHITECT_AUTH_ISSUER", "http://127.0.0.1:7411"),
                ]),
                ["ARCHITECT_DEV_SIGN_IN"],
                "{email}"
            );
        }
        assert!(is_demo_account(" ada@LOCAL.test "));
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
