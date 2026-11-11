//! Filling the stack: decided once, when the auth server starts.
//!
//! A credentialed machine (an administrator's session token in
//! `AUTH_ADMIN_TOKEN`, or a file named by `AUTH_ADMIN_TOKEN_FILE`) mirrors
//! production's organization graph; a failed mirror falls back to the last
//! snapshot, else the demo seed; everyone else gets the demo seed. Never
//! blocks on production: the mirror is bounded.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The token's environment variable (`cargo xtask auth-mirror` reads it too).
pub const TOKEN_VAR: &str = crate::mirror::TOKEN_VAR;
/// A file holding the token: where a sops-nix secret lands on a fleet machine.
pub const TOKEN_FILE_VAR: &str = "AUTH_ADMIN_TOKEN_FILE";
/// How long the mirror may take before the stack gives up on it.
pub const MIRROR_TIMEOUT: Duration = Duration::from_secs(30);

/// The demo seed's node account and org (`apps/auth-server/src/dev.rs`
/// `DEV_ORGS`: Ada owns `acme`).
pub const DEMO_NODE_EMAIL: &str = "ada@local.test";
pub const DEMO_NODE_ORG: &str = "acme";

/// How the stack is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// A mirror of production taken at this start.
    Mirror,
    /// The last mirror on this machine: this start's failed.
    LastSnapshot,
    /// The demo seed.
    Demo,
}

impl Mode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mirror => "a mirror of production, taken now",
            Self::LastSnapshot => "the last mirror of production on this machine",
            Self::Demo => "the demo seed",
        }
    }
}

/// What was decided, kept in `<state>/fill.json` for the node and `dev status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub mode: Mode,
    pub why: String,
    /// Unix seconds of the decision.
    pub at: u64,
    /// The snapshot's own `taken_at`, on a mirror.
    pub snapshot_taken_at: Option<String>,
    pub emails_redacted: Option<bool>,
    /// Who the node signs in as, and the org it hosts.
    pub node_email: String,
    pub node_org: String,
    /// What the auth database was made from (`demo`, `snapshot:<taken_at>`):
    /// a different source starts it again.
    pub db_source: String,
}

/// Where the admin token came from (never the token in a message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub source: String,
    pub value: String,
}

/// The admin token, if this machine is credentialed: `AUTH_ADMIN_TOKEN`,
/// then the file `AUTH_ADMIN_TOKEN_FILE` names.
///
/// # Errors
///
/// The file is named but unreadable or empty (said, not hidden: a fleet
/// machine whose secret did not land should hear about it).
pub fn admin_token(env: &dyn Fn(&str) -> Option<String>) -> Result<Option<Token>, String> {
    if let Some(value) = env(TOKEN_VAR).filter(|v| !v.trim().is_empty()) {
        return Ok(Some(Token {
            source: format!("${TOKEN_VAR}"),
            value: value.trim().to_owned(),
        }));
    }
    let Some(file) = env(TOKEN_FILE_VAR).filter(|v| !v.trim().is_empty()) else {
        return Ok(None);
    };
    let value = std::fs::read_to_string(file.trim())
        .map_err(|e| format!("{TOKEN_FILE_VAR} names {file}, which cannot be read: {e}"))?;
    if value.trim().is_empty() {
        return Err(format!("{TOKEN_FILE_VAR} names {file}, which is empty"));
    }
    Ok(Some(Token {
        source: format!("${TOKEN_FILE_VAR} ({})", file.trim()),
        value: value.trim().to_owned(),
    }))
}

/// The decision, before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// A fresh snapshot (its bytes).
    Mirror(Vec<u8>, String),
    LastSnapshot(String),
    Demo(String),
}

/// Decide how to fill: `forced_demo` (`just dev-seed`) wins; a token
/// mirrors (with `mirror`, bounded by the caller); a failed mirror falls
/// back to `have_snapshot`, else the seed.
pub fn decide(
    forced_demo: bool,
    token: Result<Option<Token>, String>,
    have_snapshot: bool,
    mirror: impl FnOnce(&Token) -> Result<Vec<u8>, String>,
) -> Decision {
    if forced_demo {
        return Decision::Demo(
            "`just dev-seed` chose the demo seed (`just dev-refresh` mirrors again)".to_owned(),
        );
    }
    let token = match token {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Decision::Demo(format!(
                "no administrator token on this machine (${TOKEN_VAR} or ${TOKEN_FILE_VAR})"
            ));
        }
        Err(problem) => return fallback(problem, have_snapshot),
    };
    match mirror(&token) {
        Ok(bytes) => Decision::Mirror(
            bytes,
            format!("an administrator token from {}", token.source),
        ),
        Err(problem) => fallback(format!("the mirror failed: {problem}"), have_snapshot),
    }
}

fn fallback(problem: String, have_snapshot: bool) -> Decision {
    if have_snapshot {
        Decision::LastSnapshot(problem)
    } else {
        Decision::Demo(format!("{problem}; no earlier mirror on this machine"))
    }
}

/// The parts of a snapshot the stack reads (`auth_db::snapshot::Snapshot`).
#[derive(Debug, Deserialize)]
pub struct SnapshotView {
    pub taken_at: String,
    pub emails_redacted: bool,
    pub users: Vec<SnapshotUser>,
    pub organizations: Vec<SnapshotOrg>,
    pub members: Vec<SnapshotMember>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotUser {
    pub id: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotOrg {
    pub id: String,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotMember {
    pub organization_id: String,
    pub user_id: String,
    pub role: String,
}

impl SnapshotView {
    /// # Errors
    ///
    /// Not a snapshot.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a snapshot: {e}", path.display()))
    }

    /// The org the node hosts (`wanted`, else the first by slug with an
    /// owner) and its owner's address.
    ///
    /// # Errors
    ///
    /// No such org, or it has no owner with an address.
    pub fn node_account(&self, wanted: &str) -> Result<(String, String), String> {
        let mut orgs: Vec<&SnapshotOrg> = self
            .organizations
            .iter()
            .filter(|o| wanted.is_empty() || o.slug == wanted)
            .collect();
        orgs.sort_by(|a, b| a.slug.cmp(&b.slug));
        for org in orgs {
            let owner = self
                .members
                .iter()
                .filter(|m| m.organization_id == org.id && m.role == "owner")
                .find_map(|m| {
                    self.users
                        .iter()
                        .find(|u| u.id == m.user_id)
                        .and_then(|u| u.email.clone())
                });
            if let Some(email) = owner {
                return Ok((email, org.slug.clone()));
            }
        }
        Err(if wanted.is_empty() {
            "the snapshot has no org with an owner".to_owned()
        } else {
            format!("the snapshot has no org `{wanted}` with an owner (dev-stack.toml `node_org`)")
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[allow(clippy::unnecessary_wraps)]
    fn token() -> Result<Option<Token>, String> {
        Ok(Some(Token {
            source: "$AUTH_ADMIN_TOKEN".into(),
            value: "t".into(),
        }))
    }

    #[test]
    fn no_token_is_the_demo_seed_and_production_is_never_asked() {
        let d = decide(false, Ok(None), true, |_| panic!("asked production"));
        assert!(matches!(d, Decision::Demo(why) if why.contains("no administrator token")));
    }

    #[test]
    fn a_token_mirrors() {
        let d = decide(false, token(), false, |t| {
            assert_eq!(t.value, "t");
            Ok(b"{}".to_vec())
        });
        assert!(matches!(d, Decision::Mirror(_, why) if why.contains("AUTH_ADMIN_TOKEN")));
    }

    #[test]
    fn a_failed_mirror_falls_back_to_the_last_snapshot_else_the_seed() {
        let fail = |_: &Token| Err("401 Unauthorized".to_owned());
        assert!(matches!(
            decide(false, token(), true, fail),
            Decision::LastSnapshot(why) if why.contains("401")
        ));
        assert!(matches!(
            decide(false, token(), false, fail),
            Decision::Demo(why) if why.contains("401") && why.contains("no earlier mirror")
        ));
    }

    #[test]
    fn dev_seed_wins_over_a_token() {
        let d = decide(true, token(), true, |_| panic!("asked production"));
        assert!(matches!(d, Decision::Demo(_)));
    }

    #[test]
    fn an_unreadable_token_file_is_said_and_falls_back() {
        let env = |k: &str| (k == TOKEN_FILE_VAR).then(|| "/nonexistent/token".to_owned());
        let token = admin_token(&env);
        assert!(
            token
                .as_ref()
                .is_err_and(|e| e.contains("/nonexistent/token"))
        );
        assert!(matches!(
            decide(false, token, false, |_| panic!("asked production")),
            Decision::Demo(_)
        ));
    }

    #[test]
    fn the_token_comes_from_the_environment_then_the_file() {
        let dir = std::env::temp_dir().join(format!("xtask-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("token");
        std::fs::write(&file, "from-file\n").unwrap();
        let path = file.display().to_string();
        let both = |k: &str| match k {
            TOKEN_VAR => Some("from-env".to_owned()),
            TOKEN_FILE_VAR => Some(path.clone()),
            _ => None,
        };
        assert_eq!(admin_token(&both).unwrap().unwrap().value, "from-env");
        let file_only = |k: &str| (k == TOKEN_FILE_VAR).then(|| path.clone());
        let found = admin_token(&file_only).unwrap().unwrap();
        assert_eq!(found.value, "from-file");
        assert!(!found.source.contains("from-file"), "{}", found.source);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_node_hosts_the_first_org_with_an_owner() {
        let snap: SnapshotView = serde_json::from_str(
            r#"{"taken_at":"2026-11-11T00:00:00Z","emails_redacted":true,
                "users":[{"id":"u1","email":"user-u1@local.invalid"},{"id":"u2","email":"b@x"}],
                "organizations":[{"id":"o2","slug":"zeta"},{"id":"o1","slug":"alpha"},{"id":"o3","slug":"beta"}],
                "members":[{"organization_id":"o1","user_id":"u2","role":"member"},
                           {"organization_id":"o3","user_id":"u1","role":"owner"},
                           {"organization_id":"o2","user_id":"u2","role":"owner"}]}"#,
        )
        .unwrap();
        assert_eq!(
            snap.node_account("").unwrap(),
            ("user-u1@local.invalid".to_owned(), "beta".to_owned())
        );
        assert_eq!(snap.node_account("zeta").unwrap().1, "zeta");
        assert!(snap.node_account("alpha").is_err());
    }
}
