//! What a node knows about another device: who it is, what lets it in,
//! what it may do, and how to reach it.

use architect_orgs::{OrgId, Role};
use architect_permissions::{Action, Resource, Rule};
use facet::Facet;
use uuid::Uuid;

/// The resource a document is checked as: `docs/<id>`, the shape
/// `crdt`'s permit tables use (`docs/**`).
#[must_use]
pub fn doc_resource(doc_id: Uuid) -> Resource {
    Resource::new(format!("docs/{doc_id}"))
}

/// An endpoint id as every table here keys it: trimmed, lowercase.
#[must_use]
pub fn endpoint_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// What lets a device in. A device may hold several: one of the owner's
/// own devices may also be enrolled in an org.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Grant {
    /// One of the owner's own devices, from the account's device
    /// directory: whatever the owner may do on this node.
    Account,
    /// Enrolled in `org` with `role`, by `by` (the member who enrolled
    /// it, as `Principal::describe` says). What the role may do there is
    /// the org's engine's to decide
    /// (`architect_orgs::server::ConfinedEngine`, through `Members`).
    Org { org: OrgId, role: Role, by: String },
    /// Paired: exactly `scopes`, approved on the device named `by`.
    Paired { scopes: Vec<Scope>, by: String },
}

impl Grant {
    /// Whether this grant lets its device do `action` on `what` on this
    /// node, without asking an org's engine. An org's role is not
    /// answered here (`false`): it is what the org's lanes decide.
    #[must_use]
    pub fn allows(&self, what: &Resource, action: &Action) -> bool {
        match self {
            Self::Account => true,
            Self::Org { .. } => false,
            Self::Paired { scopes, .. } => scopes.iter().any(|s| s.permits(what, action)),
        }
    }
}

/// One thing a paired device may do: these actions on resources matching
/// this pattern (`docs/<id>`, `docs/**`, `action/SESSION_*`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct Scope {
    pub resource: String,
    pub actions: Vec<String>,
}

impl Scope {
    #[must_use]
    pub fn new(resource: impl Into<String>, actions: &[&str]) -> Self {
        Self {
            resource: resource.into(),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    /// `actions` on one document.
    #[must_use]
    pub fn doc(doc_id: Uuid, actions: &[&str]) -> Self {
        Self::new(doc_resource(doc_id).0, actions)
    }

    #[must_use]
    pub fn permits(&self, what: &Resource, action: &Action) -> bool {
        Rule {
            resource: self.resource.clone(),
            actions: self.actions.clone(),
        }
        .permits(what, action)
    }

    /// Whether this scope names something rather than everything: some
    /// actions, and a resource whose first segment is a literal
    /// (`docs/**` is every document, never every resource). A pairing
    /// grants only narrow scopes.
    #[must_use]
    pub fn is_narrow(&self) -> bool {
        let first = self.resource.split('/').next().unwrap_or("");
        !self.actions.is_empty()
            && !self.actions.iter().any(|a| a == "*" || a.is_empty())
            && !first.is_empty()
            && !first.contains('*')
    }
}

/// One way to reach a device, best first in [`Peer::routes`]. AR1's
/// route list (direct, LAN, relay, learnt) builds on this.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Route {
    /// Through an iroh relay (`https://…`).
    Relay { url: String },
    /// A direct socket address (`192.168.1.20:4433`), learnt from a
    /// connection or the LAN.
    Direct { addr: String },
}

/// A device a node lets in.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Peer {
    /// Its iroh endpoint id ([`endpoint_key`]): what the transport
    /// proves, so what a gate checks.
    pub endpoint: String,
    /// Its install id on the account (`auth_client::this_device`), when
    /// it is one of the account's devices.
    pub device_id: Option<Uuid>,
    /// What a person calls it ("Cody's iPhone").
    pub name: String,
    /// `ios`, `macos`, `linux`, …
    pub platform: String,
    pub grants: Vec<Grant>,
    pub routes: Vec<Route>,
}

impl Peer {
    #[must_use]
    pub fn new(endpoint: &str, name: impl Into<String>, platform: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint_key(endpoint),
            device_id: None,
            name: name.into(),
            platform: platform.into(),
            grants: Vec::new(),
            routes: Vec::new(),
        }
    }

    #[must_use]
    pub fn granted(mut self, grant: Grant) -> Self {
        self.grant(grant);
        self
    }

    #[must_use]
    pub const fn device(mut self, device_id: Uuid) -> Self {
        self.device_id = Some(device_id);
        self
    }

    #[must_use]
    pub fn route(mut self, route: Route) -> Self {
        if !self.routes.contains(&route) {
            self.routes.push(route);
        }
        self
    }

    /// Add `grant`: an org enrolment replaces the same org's, an account
    /// grant is held once, pairings add up.
    pub fn grant(&mut self, grant: Grant) {
        match &grant {
            Grant::Account if self.is_account() => return,
            Grant::Org { org, .. } => self
                .grants
                .retain(|g| !matches!(g, Grant::Org { org: o, .. } if o == org)),
            _ => {}
        }
        self.grants.push(grant);
    }

    /// One of the owner's own devices.
    #[must_use]
    pub fn is_account(&self) -> bool {
        self.grants.iter().any(|g| matches!(g, Grant::Account))
    }

    /// Whether any grant lets it do `action` on `what` here
    /// ([`Grant::allows`]).
    #[must_use]
    pub fn allows(&self, what: &Resource, action: &Action) -> bool {
        self.grants.iter().any(|g| g.allows(what, action))
    }

    /// The orgs it is enrolled in, with its role in each.
    #[must_use]
    pub fn orgs(&self) -> Vec<(OrgId, Role)> {
        self.grants
            .iter()
            .filter_map(|g| match g {
                Grant::Org { org, role, .. } => Some((org.clone(), role.clone())),
                _ => None,
            })
            .collect()
    }

    /// The same device with what `newer` says about it (its name, its
    /// routes), keeping every grant either holds.
    #[must_use]
    pub fn merged(mut self, newer: Self) -> Self {
        self.name = newer.name;
        self.platform = newer.platform;
        self.device_id = newer.device_id.or(self.device_id);
        for route in newer.routes {
            if !self.routes.contains(&route) {
                self.routes.push(route);
            }
        }
        for grant in newer.grants {
            self.grant(grant);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_scope_names_something() {
        let doc = Uuid::from_u128(7);
        assert!(Scope::doc(doc, &["read", "write"]).is_narrow());
        assert!(Scope::new("docs/**", &["read"]).is_narrow());
        assert!(!Scope::new("**", &["read"]).is_narrow(), "everything");
        assert!(!Scope::new("*/notes", &["read"]).is_narrow());
        assert!(!Scope::new("docs/**", &["*"]).is_narrow(), "any action");
        assert!(!Scope::new("docs/**", &[]).is_narrow(), "no action");
    }

    #[test]
    fn a_scope_permits_its_resource_and_actions_only() {
        let (doc, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let read = Scope::doc(doc, &["read"]);
        assert!(read.permits(&doc_resource(doc), &Action::read()));
        assert!(!read.permits(&doc_resource(doc), &Action::write()));
        assert!(!read.permits(&doc_resource(other), &Action::read()));
    }

    #[test]
    fn grants_merge_one_per_org_and_one_account() {
        let org = OrgId::new("o1");
        let peer = Peer::new(" AB ", "laptop", "linux")
            .granted(Grant::Account)
            .granted(Grant::Account)
            .granted(Grant::Org {
                org: org.clone(),
                role: Role::Viewer,
                by: "user:u".into(),
            })
            .granted(Grant::Org {
                org: org.clone(),
                role: Role::Member,
                by: "user:u".into(),
            });
        assert_eq!(peer.endpoint, "ab");
        assert_eq!(peer.grants.len(), 2);
        assert_eq!(peer.orgs(), vec![(org, Role::Member)]);
        assert!(peer.allows(&Resource::new("anything"), &Action::admin()));
    }
}
