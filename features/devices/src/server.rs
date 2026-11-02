//! An org host's devices (feature `server`, native): the enrolment
//! service, and enrolments as members the org's engine knows.
//!
//! An enrolled device is a `Principal::Host` member of the org with its
//! role: [`project_members`] keeps `architect_orgs::server::Members` in
//! step with the [`Admission`], so the org's `ConfinedEngine` gives it
//! the same role rules a person with that role gets, on every lane of
//! the org. Revoking it (or its enrolment) takes it out of `Members`, so
//! its next call there is refused; when it holds no grant at all, its
//! connections close too ([`Admission::ends`]).
//!
//! ```ignore
//! let admission = Admission::new();
//! let members = Members::new();
//! project_members(&admission, &members);
//! let engine = ConfinedEngine::new(members.clone()).role(&Role::Member, rules);
//! let gate = PermissionsGate::new(Arc::new(engine), identity)
//!     .permit(device_enrolment_service_descriptor(), &ENROL_PERMITS);
//! let router = LayerRouter::new().with(
//!     device_enrolment_service_descriptor(),
//!     DeviceEnrolmentDispatcher::new(EnrolmentHost::new(admission, members)),
//! );
//! // per connection at /org/{slug}/vox: confine(org, gate.wrap(router))
//! ```

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use architect_orgs::server::{Members, current_org};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::Principal;

use crate::admission::Admission;
use crate::model::{Grant, Peer, endpoint_key};
use crate::service::{DeviceEnrolment, EnrolError, Enrolment};

/// Keep `members` in step with `admission`.
///
/// Each enrolled device is a `Principal::Host` member of the orgs it is enrolled in, with its role
/// there. A device enrolled nowhere (any more) is in no org. Hosts that
/// `members` holds for other reasons are left alone.
pub fn project_members(admission: &Admission, members: &Members) {
    let members = members.clone();
    let projected: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    admission.observe(move |peers| {
        let mut held = projected.lock().unwrap_or_else(PoisonError::into_inner);
        let mut now = BTreeSet::new();
        for peer in peers {
            let orgs: Vec<Org> = peer
                .orgs()
                .into_iter()
                .map(|(org, role)| Org::new(org.0.clone(), org.0, role))
                .collect();
            if orgs.is_empty() {
                continue;
            }
            members.set(&host(&peer.endpoint), orgs);
            now.insert(peer.endpoint.clone());
        }
        for gone in held.difference(&now) {
            members.forget(&host(gone));
        }
        *held = now;
    });
}

fn host(endpoint: &str) -> Principal {
    Principal::Host {
        endpoint: endpoint.to_owned(),
    }
}

/// How much a role may do, most first: a member enrols a device with
/// their own role or a lesser one.
const fn rank(role: &Role) -> u8 {
    match role {
        Role::Owner => 0,
        Role::Admin => 1,
        Role::Member => 2,
        Role::Viewer => 3,
        Role::Other(_) => 4,
    }
}

const fn steward(role: &Role) -> bool {
    matches!(role, Role::Owner | Role::Admin)
}

/// Longest device name an enrolment keeps.
const NAME_MAX: usize = 80;

/// [`DeviceEnrolment`] for the orgs a host serves: enrolments land in the
/// host's [`Admission`] as [`Grant::Org`], and the caller's role comes
/// from `members` (the gate's `MembersResolver` fills it).
#[derive(Clone)]
pub struct EnrolmentHost {
    admission: Admission,
    members: Members,
}

impl std::fmt::Debug for EnrolmentHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnrolmentHost").finish_non_exhaustive()
    }
}

impl EnrolmentHost {
    #[must_use]
    pub const fn new(admission: Admission, members: Members) -> Self {
        Self { admission, members }
    }

    /// The lane's org, the caller, and the caller's role there. A device
    /// does not enrol devices: only a person (or the host itself).
    fn caller(&self) -> Result<(OrgId, Principal, Role), EnrolError> {
        let org = current_org().ok_or(EnrolError::NoOrg)?;
        let who = architect::permissions_gate::caller().ok_or(EnrolError::NotAMember)?;
        if !matches!(who, Principal::User { .. } | Principal::Service { .. }) {
            return Err(EnrolError::NotAMember);
        }
        let role = match &who {
            Principal::Service { .. } => Role::Owner,
            _ => self
                .members
                .role(&who, &org)
                .ok_or(EnrolError::NotAMember)?,
        };
        Ok((org, who, role))
    }
}

impl DeviceEnrolment for EnrolmentHost {
    async fn enrol(&self, device: Enrolment) -> Result<Peer, EnrolError> {
        let (org, who, role) = self.caller()?;
        let endpoint = endpoint_key(&device.endpoint);
        if endpoint.is_empty()
            || endpoint.len() > 128
            || !endpoint.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(EnrolError::Invalid(
                "endpoint is an endpoint's id, letters and digits".into(),
            ));
        }
        let name = device.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            return Err(EnrolError::Invalid(
                "a device needs a name of at most 80 characters".into(),
            ));
        }
        if rank(&device.role) < rank(&role) {
            return Err(EnrolError::Denied(format!(
                "a {} enrols a device as {} at most",
                role.label(),
                role.label()
            )));
        }
        if let Some(held) = self.held(&endpoint, &org)
            && held != who.describe()
            && !steward(&role)
        {
            return Err(EnrolError::Denied(
                "that device was enrolled by someone else".into(),
            ));
        }
        self.admission.admit(
            Peer::new(&endpoint, name, device.platform.trim().to_ascii_lowercase()).granted(
                Grant::Org {
                    org: org.clone(),
                    role: device.role.clone(),
                    by: who.describe(),
                },
            ),
        );
        architect_telemetry::wide::set("devices.enrol.org", org.0.clone());
        architect_telemetry::wide::set("devices.enrol.endpoint", endpoint.clone());
        architect_telemetry::wide::set("devices.enrol.role", device.role.label());
        self.admission
            .get(&endpoint)
            .map(|p| in_org(p, &org))
            .ok_or(EnrolError::NotEnrolled)
    }

    async fn enrolments(&self) -> Result<Vec<Peer>, EnrolError> {
        let (org, who, role) = self.caller()?;
        let me = who.describe();
        Ok(self
            .admission
            .list()
            .into_iter()
            .filter(|p| {
                p.grants.iter().any(|g| {
                    matches!(g, Grant::Org { org: o, by, .. } if *o == org && (steward(&role) || *by == me))
                })
            })
            .map(|p| in_org(p, &org))
            .collect())
    }

    async fn revoke(&self, endpoint: String) -> Result<(), EnrolError> {
        let (org, who, role) = self.caller()?;
        let endpoint = endpoint_key(&endpoint);
        let by = self.held(&endpoint, &org).ok_or(EnrolError::NotEnrolled)?;
        if by != who.describe() && !steward(&role) {
            return Err(EnrolError::Denied(
                "only who enrolled a device, or an owner or admin, revokes it".into(),
            ));
        }
        self.admission.revoke_org(&endpoint, &org);
        architect_telemetry::wide::set("devices.revoke.org", org.0);
        architect_telemetry::wide::set("devices.revoke.endpoint", endpoint);
        Ok(())
    }
}

impl EnrolmentHost {
    /// Who enrolled `endpoint` in `org`, if it is enrolled there.
    fn held(&self, endpoint: &str, org: &OrgId) -> Option<String> {
        self.admission.get(endpoint).and_then(|p| {
            p.grants.into_iter().find_map(|g| match g {
                Grant::Org { org: o, by, .. } if o == *org => Some(by),
                _ => None,
            })
        })
    }
}

/// `peer` as this org sees it: its grant here only, not what else lets
/// it in.
fn in_org(mut peer: Peer, org: &OrgId) -> Peer {
    peer.grants
        .retain(|g| matches!(g, Grant::Org { org: o, .. } if o == org));
    peer
}
