//! What a person said yes to (#178, #181): an extension's capabilities,
//! granted on this device or in one organization, revocable.
//!
//! An installed extension runs with what its manifest asks for *and* a
//! person granted ([`Capabilities::within`]): nothing past its own
//! actions, settings and data until they do. A built-in one (the app
//! ships it) and a development copy (the person put it there) run with
//! what they ask. The prompt — on install, and on an update whose
//! capabilities grow — shows [`Capabilities::grows_from`] the grant.
//!
//! A grant is kept for one [`Scope`]: the **device** (wherever this app
//! runs here) or one **organization** (only while it is the current one,
//! [`Grants::set_org`]). [`Grants::revoke`] takes it back at once: the
//! next host-service call, hook or script run is refused what it no
//! longer has. [`Grants::open`] keeps them in a file
//! (`<app dir>/extensions/grants.json` in the lab).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::manifest::{Capabilities, Reach};

/// Where a grant holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// This device, whichever organization is current.
    Device,
    /// Only while organization `org` is the current one.
    Org(String),
}

impl Scope {
    /// `device`, or `org:<id>`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Device => "device".to_owned(),
            Self::Org(org) => format!("org:{org}"),
        }
    }

    /// `device`, `org` (the current organization, `current`), or
    /// `org:<id>`.
    ///
    /// # Errors
    ///
    /// None of those, or `org` with no current organization.
    pub fn parse(text: &str, current: Option<&str>) -> Result<Self, String> {
        match text.trim() {
            "" | "device" => Ok(Self::Device),
            "org" => current
                .map(|o| Self::Org(o.to_owned()))
                .ok_or_else(|| "no organization is current: grant it to the device".to_owned()),
            other => other
                .strip_prefix("org:")
                .filter(|o| !o.is_empty())
                .map(|o| Self::Org(o.to_owned()))
                .ok_or_else(|| format!("scope {other:?}: device, org or org:<id>")),
        }
    }
}

/// One yes: what extension `id` may reach, where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    /// The version it was granted at (an update that asks for no more
    /// keeps it).
    pub version: String,
    pub capabilities: Capabilities,
    pub scope: Scope,
    /// When, in ms since the epoch.
    pub granted_ms: u64,
    /// Who said yes (`person`, an agent's principal after its ticket).
    pub by: String,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    grants: Vec<Grant>,
}

struct Inner {
    path: Option<PathBuf>,
    grants: Vec<Grant>,
    org: Option<String>,
    revision: u64,
}

/// Every grant on this device. Cheap to clone: every clone is the same.
#[derive(Clone)]
pub struct Grants {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for Grants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Grants")
            .field("path", &inner.path)
            .field("grants", &inner.grants.len())
            .field("org", &inner.org)
            .finish()
    }
}

impl PartialEq for Grants {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Both together: each part the wider of the two.
fn union(a: &Capabilities, b: &Capabilities) -> Capabilities {
    let mut out = a.clone();
    for (into, from) in [
        (&mut out.actions, &b.actions),
        (&mut out.hosts, &b.hosts),
        (&mut out.folders, &b.folders),
        (&mut out.reads, &b.reads),
        (&mut out.writes, &b.writes),
        (&mut out.ui, &b.ui),
        (&mut out.events, &b.events),
        (&mut out.ports, &b.ports),
    ] {
        for x in from {
            if !into.contains(x) {
                into.push(x.clone());
            }
        }
    }
    out.effect = a.effect.max(b.effect);
    out
}

impl Grants {
    /// Grants kept in memory, for the run.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path: None,
                grants: Vec::new(),
                org: None,
                revision: 0,
            })),
        }
    }

    /// Grants kept in `path` (JSON), read now; a file that does not read
    /// is none (and is replaced on the next grant).
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let grants = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<File>(&t).ok())
            .map(|f| f.grants)
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path: Some(path),
                grants,
                org: None,
                revision: 0,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save(inner: &mut Inner) {
        inner.revision = inner.revision.wrapping_add(1);
        let Some(path) = &inner.path else { return };
        let file = File {
            grants: inner.grants.clone(),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&file) {
            let _ = std::fs::write(path, text);
        }
    }

    /// The organization working in now: whose grants hold besides the
    /// device's.
    pub fn set_org(&self, org: Option<String>) {
        let mut inner = self.lock();
        if inner.org != org {
            inner.org = org;
            inner.revision = inner.revision.wrapping_add(1);
        }
    }

    /// The current organization.
    #[must_use]
    pub fn org(&self) -> Option<String> {
        self.lock().org.clone()
    }

    /// Changes so far: a page re-reads on a new one.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Grants `capabilities` to `id` (at `version`) in `scope`, in place of
    /// what it had there.
    // Kept and written under the lock, so two grants never cross.
    #[allow(clippy::must_use_candidate, clippy::significant_drop_tightening)]
    pub fn grant(
        &self,
        id: &str,
        version: &str,
        capabilities: Capabilities,
        scope: Scope,
        by: &str,
    ) -> Grant {
        let grant = Grant {
            id: id.to_owned(),
            version: version.to_owned(),
            capabilities,
            scope,
            granted_ms: crate::logs::now_ms(),
            by: by.to_owned(),
        };
        {
            let mut inner = self.lock();
            inner
                .grants
                .retain(|g| !(g.id == grant.id && g.scope == grant.scope));
            inner.grants.push(grant.clone());
            Self::save(&mut inner);
        }
        grant
    }

    /// Takes back `id`'s grant in `scope` (every scope when `None`): how
    /// many there were.
    #[allow(clippy::must_use_candidate)]
    pub fn revoke(&self, id: &str, scope: Option<&Scope>) -> usize {
        let mut inner = self.lock();
        let before = inner.grants.len();
        inner
            .grants
            .retain(|g| !(g.id == id && scope.is_none_or(|s| &g.scope == s)));
        let gone = before.saturating_sub(inner.grants.len());
        if gone > 0 {
            Self::save(&mut inner);
        }
        drop(inner);
        gone
    }

    /// Every grant, by id.
    #[must_use]
    pub fn list(&self) -> Vec<Grant> {
        let mut out = self.lock().grants.clone();
        out.sort_by(|a, b| {
            a.id.cmp(&b.id)
                .then(a.scope.describe().cmp(&b.scope.describe()))
        });
        out
    }

    /// `id`'s grants, as kept.
    #[must_use]
    pub fn of(&self, id: &str) -> Vec<Grant> {
        self.list().into_iter().filter(|g| g.id == id).collect()
    }

    /// What `id` was granted that holds now: the device's and the current
    /// organization's together. `None` when nothing was.
    #[must_use]
    pub fn granted(&self, id: &str) -> Option<Capabilities> {
        let inner = self.lock();
        inner
            .grants
            .iter()
            .filter(|g| g.id == id)
            .filter(|g| match &g.scope {
                Scope::Device => true,
                Scope::Org(o) => inner.org.as_deref() == Some(o.as_str()),
            })
            .map(|g| g.capabilities.clone())
            .reduce(|a, b| union(&a, &b))
    }

    /// What `asks` (a manifest's capabilities) needs a person's yes for
    /// past what is granted now: empty when nothing.
    #[must_use]
    pub fn needs(&self, id: &str, asks: &Capabilities) -> Vec<crate::capabilities::Ask> {
        let granted = self.granted(id).unwrap_or_else(|| Capabilities {
            effect: Reach::ReadOnly,
            ..Capabilities::default()
        });
        asks.grows_from(&granted)
    }
}

#[cfg(test)]
mod tests {
    use super::{Grants, Scope};
    use crate::manifest::{Capabilities, Reach};

    fn asks() -> Capabilities {
        Capabilities {
            actions: vec!["TRANSPORT_".to_owned()],
            effect: Reach::Mutating,
            reads: vec!["selection".to_owned()],
            ..Capabilities::default()
        }
    }

    #[test]
    fn a_grant_holds_where_it_was_given_and_is_taken_back() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("grants.json");
        let grants = Grants::open(&path);
        assert_eq!(grants.granted("fts.m"), None);
        assert_eq!(grants.needs("fts.m", &asks()).len(), 3);

        grants.grant(
            "fts.m",
            "1.0.0",
            asks(),
            Scope::Org("acme".to_owned()),
            "person",
        );
        assert_eq!(
            grants.granted("fts.m"),
            None,
            "not while acme is not current"
        );
        grants.set_org(Some("acme".to_owned()));
        assert_eq!(grants.granted("fts.m"), Some(asks()));
        assert!(grants.needs("fts.m", &asks()).is_empty());

        grants.set_org(None);
        grants.grant("fts.m", "1.0.0", asks(), Scope::Device, "person");
        let again = Grants::open(&path);
        assert_eq!(again.granted("fts.m"), Some(asks()), "kept in its file");
        assert_eq!(again.of("fts.m").len(), 2);

        assert_eq!(again.revoke("fts.m", Some(&Scope::Device)), 1);
        assert_eq!(again.granted("fts.m"), None);
        assert_eq!(again.revoke("fts.m", None), 1);
        assert!(again.list().is_empty());
    }

    #[test]
    fn a_scope_reads_from_words() {
        assert_eq!(Scope::parse("device", None).unwrap(), Scope::Device);
        assert_eq!(
            Scope::parse("org", Some("acme")).unwrap(),
            Scope::Org("acme".to_owned())
        );
        assert!(Scope::parse("org", None).is_err());
        assert_eq!(
            Scope::parse("org:b", None).unwrap(),
            Scope::Org("b".to_owned())
        );
        assert!(Scope::parse("planet", None).is_err());
    }
}
