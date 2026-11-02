//! Who a node lets in: one table every lane on the node checks.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use architect_orgs::OrgId;
use architect_permissions::{Action, Decision, PermissionEngine, Principal, Resource};

use crate::model::{Grant, Peer, Route, endpoint_key};
use crate::pairing::{PAIR_ACTION, PAIR_RESOURCE};

/// Called with every peer after each change (and once when registered).
type Observer = Arc<dyn Fn(&[Peer]) + Send + Sync>;

struct Inner {
    peers: RwLock<BTreeMap<String, Peer>>,
    /// Bumped on every change, so a connection's closer
    /// ([`Admission::ends`]) can look again.
    changed: tokio::sync::watch::Sender<u64>,
    observers: RwLock<Vec<Observer>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            peers: RwLock::default(),
            changed: tokio::sync::watch::Sender::new(0),
            observers: RwLock::default(),
        }
    }
}

/// The devices a node lets in, by iroh endpoint id.
///
/// What lets each in is a [`Grant`]: the owner's other devices (from the account's directory),
/// devices enrolled in one of the node's orgs, and devices paired with it.
///
/// Cheap to clone; every clone is the same table, so the gate built for a
/// connection sees a change made after it. Every lane on the node checks
/// it: `crdt::p2p::NodeAccess` for documents, an `EngineHost`'s iroh lane
/// as (part of) its gate's engine, and the connection itself, which
/// closes when its device is revoked ([`Self::ends`]).
///
/// As the gate's [`PermissionEngine`] it answers for `Principal::Host`
/// only: an admitted device may do what its grants allow
/// ([`Peer::allows`]), any proved endpoint may ask to pair
/// ([`PAIR_RESOURCE`]), and everyone else nothing. Compose it with the
/// engine for people (`CompositeEngine`, first allow wins).
#[derive(Clone, Default)]
pub struct Admission(Arc<Inner>);

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admission")
            .field("peers", &self.read().len())
            .finish_non_exhaustive()
    }
}

impl Admission {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Peer>> {
        self.0.peers.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Change the table with `edit`, then tell whoever waits or observes.
    /// Returns what `edit` returned.
    fn change<T>(&self, edit: impl FnOnce(&mut BTreeMap<String, Peer>) -> T) -> T {
        let (out, now) = {
            let mut peers = self.0.peers.write().unwrap_or_else(PoisonError::into_inner);
            let out = edit(&mut peers);
            peers.retain(|_, p| !p.grants.is_empty());
            (out, peers.values().cloned().collect::<Vec<_>>())
        };
        self.0.changed.send_modify(|n| *n = n.wrapping_add(1));
        let observers = self
            .0
            .observers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for observe in observers {
            observe(&now);
        }
        out
    }

    /// The owner's other devices are exactly `peers` (the account's
    /// directory, less this device): each holds [`Grant::Account`], a
    /// device off the list loses it, and every other grant stands.
    pub fn set_account<I: IntoIterator<Item = Peer>>(&self, peers: I) {
        let fresh: Vec<Peer> = peers
            .into_iter()
            .map(|p| {
                let endpoint = endpoint_key(&p.endpoint);
                Peer { endpoint, ..p }.granted(Grant::Account)
            })
            .collect();
        self.change(|table| {
            for peer in table.values_mut() {
                peer.grants.retain(|g| !matches!(g, Grant::Account));
            }
            for peer in fresh {
                upsert(table, peer);
            }
        });
    }

    /// Let `peer` in with its grants (added to any it holds).
    pub fn admit(&self, peer: Peer) {
        let peer = Peer {
            endpoint: endpoint_key(&peer.endpoint),
            ..peer
        };
        self.change(|table| upsert(table, peer));
    }

    /// Let `endpoint` in no more, by any grant: its connections close and
    /// its next call is refused. What it was.
    #[allow(clippy::must_use_candidate)]
    pub fn revoke(&self, endpoint: &str) -> Option<Peer> {
        let key = endpoint_key(endpoint);
        self.change(|table| table.remove(&key))
    }

    /// Withdraw `endpoint`'s enrolment in `org`; its other grants stand.
    /// Whether it was enrolled there.
    #[allow(clippy::must_use_candidate)]
    pub fn revoke_org(&self, endpoint: &str, org: &OrgId) -> bool {
        let key = endpoint_key(endpoint);
        self.change(|table| {
            table.get_mut(&key).is_some_and(|peer| {
                let before = peer.grants.len();
                peer.grants
                    .retain(|g| !matches!(g, Grant::Org { org: o, .. } if o == org));
                peer.grants.len() != before
            })
        })
    }

    /// Remember another way to reach `endpoint` (no-op for a device not
    /// let in).
    pub fn learn_route(&self, endpoint: &str, route: Route) {
        let key = endpoint_key(endpoint);
        if self
            .read()
            .get(&key)
            .is_some_and(|p| p.routes.contains(&route))
        {
            return;
        }
        self.change(|table| {
            if let Some(peer) = table.get_mut(&key) {
                peer.routes.push(route);
            }
        });
    }

    /// Make the table exactly `peers`: what [`Self::to_json`] saved, read
    /// back on start so the node admits its devices before it reaches
    /// anything.
    pub fn restore(&self, peers: Vec<Peer>) {
        self.change(|table| {
            table.clear();
            for peer in peers {
                let peer = Peer {
                    endpoint: endpoint_key(&peer.endpoint),
                    ..peer
                };
                upsert(table, peer);
            }
        });
    }

    #[must_use]
    pub fn get(&self, endpoint: &str) -> Option<Peer> {
        self.read().get(&endpoint_key(endpoint)).cloned()
    }

    /// Every device let in, by endpoint id: what a devices page lists.
    #[must_use]
    pub fn list(&self) -> Vec<Peer> {
        self.read().values().cloned().collect()
    }

    /// Whether `endpoint` holds any grant.
    #[must_use]
    pub fn admits(&self, endpoint: &str) -> bool {
        self.read().contains_key(&endpoint_key(endpoint))
    }

    /// Whether `endpoint` is one of the owner's own devices.
    #[must_use]
    pub fn is_account(&self, endpoint: &str) -> bool {
        self.read()
            .get(&endpoint_key(endpoint))
            .is_some_and(Peer::is_account)
    }

    /// Whether `endpoint`'s grants let it do `action` on `what` here
    /// ([`Peer::allows`]).
    #[must_use]
    pub fn allows(&self, endpoint: &str, what: &Resource, action: &Action) -> bool {
        self.read()
            .get(&endpoint_key(endpoint))
            .is_some_and(|p| p.allows(what, action))
    }

    /// Call `observe` with every peer now, and again after every change
    /// (outside the table's lock): how an app saves the table, or keeps
    /// another one in step (`crate::server::project_members`).
    pub fn observe(&self, observe: impl Fn(&[Peer]) + Send + Sync + 'static) {
        let observe: Observer = Arc::new(observe);
        self.0
            .observers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(&observe));
        observe(&self.list());
    }

    /// A receiver that changes whenever the table does.
    #[must_use]
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.changed.subscribe()
    }

    /// Resolves once `endpoint`, having been let in, is let in no more:
    /// what closes a connection when its device is revoked. A connection
    /// from an endpoint not let in yet (one that is about to pair) waits
    /// for it to be let in first.
    pub async fn ends(&self, endpoint: &str) {
        let key = endpoint_key(endpoint);
        let mut changes = self.changes();
        let mut was_in = false;
        loop {
            let is_in = self.admits(&key);
            if was_in && !is_in {
                return;
            }
            was_in = is_in;
            if changes.changed().await.is_err() {
                // The sender lives as long as the table: never close a
                // connection for it.
                std::future::pending::<()>().await;
            }
        }
    }

    /// The table, as JSON (facet), for the app to keep.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(&self.list()).map_err(|e| e.to_string())
    }

    /// [`Self::restore`] from what [`Self::to_json`] wrote.
    ///
    /// # Errors
    ///
    /// The decoder's message; the table is unchanged.
    pub fn restore_json(&self, json: &str) -> Result<(), String> {
        let peers: Vec<Peer> = facet_json::from_str(json).map_err(|e| e.to_string())?;
        self.restore(peers);
        Ok(())
    }
}

fn upsert(table: &mut BTreeMap<String, Peer>, peer: Peer) {
    let merged = match table.remove(&peer.endpoint) {
        Some(held) => held.merged(peer),
        None => peer,
    };
    table.insert(merged.endpoint.clone(), merged);
}

/// The gate's engine for `Principal::Host`: what an admitted device's
/// grants allow, asking to pair for anyone, nothing else.
impl PermissionEngine for Admission {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        let Principal::Host { endpoint } = who else {
            return Decision::deny(format!("{} is not a device", who.describe()));
        };
        if what.as_str() == PAIR_RESOURCE && action.as_str() == PAIR_ACTION {
            return Decision::Allow;
        }
        match self.get(endpoint) {
            Some(peer) if peer.allows(what, action) => Decision::Allow,
            Some(peer) => Decision::deny(format!(
                "{} may not {} {}",
                peer.name,
                action.as_str(),
                what.as_str()
            )),
            None => Decision::deny("not a device this node lets in"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use architect_orgs::Role;
    use uuid::Uuid;

    use super::*;
    use crate::model::{Scope, doc_resource};

    fn host(endpoint: &str) -> Principal {
        Principal::Host {
            endpoint: endpoint.into(),
        }
    }

    #[test]
    fn the_account_list_replaces_only_account_grants() {
        let admission = Admission::new();
        let doc = Uuid::from_u128(1);
        admission.admit(
            Peer::new("cc", "guest's ipad", "ios").granted(Grant::Paired {
                scopes: vec![Scope::doc(doc, &["read"])],
                by: "laptop".into(),
            }),
        );
        admission.set_account([
            Peer::new("AA", "phone", "ios"),
            Peer::new("cc", "ipad", "ios"),
        ]);
        assert!(admission.is_account("aa"));
        assert!(admission.is_account("cc"));

        // The phone leaves the account; the iPad's pairing outlives its
        // account grant.
        admission.set_account([]);
        assert!(!admission.admits("aa"));
        assert!(admission.admits("cc"));
        assert!(!admission.is_account("cc"));
        assert_eq!(admission.get("cc").map(|p| p.name).as_deref(), Some("ipad"));
    }

    #[test]
    fn the_engine_answers_for_devices_by_their_grants() {
        let admission = Admission::new();
        let (doc, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        admission.set_account([Peer::new("aa", "phone", "ios")]);
        admission.admit(Peer::new("bb", "ipad", "ios").granted(Grant::Paired {
            scopes: vec![Scope::doc(doc, &["write"])],
            by: "laptop".into(),
        }));
        let write = Action::write();
        assert!(
            admission
                .check(&host("aa"), &doc_resource(other), &write)
                .allowed()
        );
        assert!(
            admission
                .check(&host("bb"), &doc_resource(doc), &write)
                .allowed()
        );
        assert!(
            !admission
                .check(&host("bb"), &doc_resource(other), &write)
                .allowed()
        );
        assert!(
            !admission
                .check(&host("zz"), &doc_resource(doc), &write)
                .allowed()
        );
        let pair = (Resource::new(PAIR_RESOURCE), Action::new(PAIR_ACTION));
        assert!(admission.check(&host("zz"), &pair.0, &pair.1).allowed());
        let user = Principal::User {
            user_id: "u".into(),
        };
        assert!(!admission.check(&user, &pair.0, &pair.1).allowed());
    }

    #[test]
    fn an_org_enrolment_is_withdrawn_alone() {
        let admission = Admission::new();
        let (a, b) = (OrgId::new("a"), OrgId::new("b"));
        for org in [&a, &b] {
            admission.admit(Peer::new("aa", "phone", "ios").granted(Grant::Org {
                org: org.clone(),
                role: Role::Member,
                by: "user:u".into(),
            }));
        }
        assert!(admission.revoke_org("aa", &a));
        assert!(!admission.revoke_org("aa", &a), "once");
        assert_eq!(
            admission.get("aa").map(|p| p.orgs()),
            Some(vec![(b.clone(), Role::Member)])
        );
        assert!(admission.revoke_org("aa", &b));
        assert!(!admission.admits("aa"), "no grant left, not let in");
    }

    #[test]
    fn the_table_saves_and_restores() {
        let admission = Admission::new();
        admission.set_account([Peer::new("aa", "phone", "ios")
            .device(Uuid::from_u128(9))
            .route(Route::Relay {
                url: "https://relay.example".into(),
            })]);
        admission.admit(Peer::new("bb", "ipad", "ios").granted(Grant::Paired {
            scopes: vec![Scope::new("docs/**", &["read"])],
            by: "laptop".into(),
        }));
        let saved = admission.to_json().unwrap();
        let back = Admission::new();
        back.restore_json(&saved).unwrap();
        assert_eq!(back.list(), admission.list());
        assert!(back.restore_json("not json").is_err());
        assert_eq!(back.list().len(), 2, "unchanged by a bad file");
    }

    #[test]
    fn observers_see_every_change() {
        let admission = Admission::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        admission.observe({
            let seen = Arc::clone(&seen);
            move |peers| seen.lock().unwrap().push(peers.len())
        });
        admission.set_account([Peer::new("aa", "phone", "ios")]);
        admission.revoke("aa");
        assert_eq!(*seen.lock().unwrap(), vec![0, 1, 0]);
    }

    #[tokio::test]
    async fn ends_fires_when_a_device_is_let_in_and_then_revoked() {
        let admission = Admission::new();
        let ends = tokio::spawn({
            let admission = admission.clone();
            async move { admission.ends("aa").await }
        });
        tokio::task::yield_now().await;
        admission.set_account([Peer::new("aa", "phone", "ios")]);
        tokio::task::yield_now().await;
        assert!(!ends.is_finished(), "let in: still open");
        admission.revoke("aa");
        tokio::time::timeout(std::time::Duration::from_secs(5), ends)
            .await
            .expect("ends once revoked")
            .unwrap();
    }
}
