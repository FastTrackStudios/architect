//! The wire (feature `vox`): pairing, served by a device to the devices
//! that would pair with it, and per-org enrolment, served by an org's
//! host to its members.

use architect_orgs::Role;
use architect_permissions::{MethodPermit, ServicePermits};
use facet::Facet;

use crate::model::Peer;
use crate::pairing::{PAIR_ACTION, PAIR_RESOURCE, PairError, PairRequest, Paired};

/// Asking a device to let this one in. Served on the device's iroh
/// endpoint to every proved endpoint ([`PAIR_PERMITS`]): the caller is
/// the endpoint its connection proved, and that is what is let in.
#[vox::service]
pub trait DevicePairing {
    /// Redeem one of the device's offers with its code or its ticket's
    /// secret: the caller is let in with the offer's scopes.
    async fn pair(&self, request: PairRequest) -> Result<Paired, PairError>;
}

/// The gate's table for [`DevicePairing`]: anyone may ask
/// (`crate::Admission` allows [`PAIR_RESOURCE`] to every endpoint).
pub const PAIR_PERMITS: ServicePermits = ServicePermits {
    service: "DevicePairing",
    methods: &[MethodPermit::new("pair", PAIR_ACTION, PAIR_RESOURCE).audited()],
};

/// A device to enrol in the lane's org.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Enrolment {
    /// Its iroh endpoint id.
    pub endpoint: String,
    pub name: String,
    pub platform: String,
    /// What it may do there: at most the enrolling member's own role.
    pub role: Role,
}

/// Why an enrolment call was refused.
#[derive(Debug, Clone, PartialEq, Eq, Facet, thiserror::Error)]
#[repr(u8)]
pub enum EnrolError {
    /// The call is not in an org (enrolment is per org: an org's lane).
    #[error("enrolment is per org: call it on an org's lane")]
    NoOrg,
    /// The caller is not a member of the org, or not signed in.
    #[error("only a member of the org enrols devices in it")]
    NotAMember,
    /// A role above the caller's own, or a device that is not theirs to
    /// revoke.
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Invalid(String),
    /// No such enrolment in this org.
    #[error("no device with that endpoint is enrolled here")]
    NotEnrolled,
}

/// An org's enrolled devices: a member admits their device with a role,
/// lists what is enrolled, and revokes. Every call is in the lane's org
/// (`architect_orgs::server::current_org`) as the caller the gate
/// resolved; the org is never an argument.
#[vox::service]
pub trait DeviceEnrolment {
    /// Admit a device in this org with a role no higher than the
    /// caller's own. Enrolling it again changes its role.
    async fn enrol(&self, device: Enrolment) -> Result<Peer, EnrolError>;

    /// The devices enrolled here: every one for an owner or admin, the
    /// caller's own otherwise.
    async fn enrolments(&self) -> Result<Vec<Peer>, EnrolError>;

    /// Withdraw a device's enrolment here: the one who enrolled it, or an
    /// owner or admin. Its calls in the org are refused from the next on.
    async fn revoke(&self, endpoint: String) -> Result<(), EnrolError>;
}

/// The gate's table for [`DeviceEnrolment`]. Membership and role are the
/// service's to check (the caller's role caps the device's); the gate's
/// part is that the caller is someone in the org.
pub const ENROL_PERMITS: ServicePermits = ServicePermits {
    service: "DeviceEnrolment",
    methods: &[
        MethodPermit::new("enrol", "read", "devices/enrolments").audited(),
        MethodPermit::new("enrolments", "read", "devices/enrolments"),
        MethodPermit::new("revoke", "read", "devices/enrolments").audited(),
    ],
};

/// Milliseconds since the Unix epoch, for the pairing model's clock.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// [`DevicePairing`] over a node's [`Pairings`](crate::Pairings): the
/// caller must be `Principal::Host`, the endpoint its connection proved
/// (`architect::iroh_link::serve_peers`, `EngineHost`'s iroh lane).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct PairingHost(pub crate::Pairings);

#[cfg(not(target_arch = "wasm32"))]
impl DevicePairing for PairingHost {
    async fn pair(&self, request: PairRequest) -> Result<Paired, PairError> {
        let Some(architect_permissions::Principal::Host { endpoint }) =
            architect::permissions_gate::caller()
        else {
            return Err(PairError::NotADevice);
        };
        // The gate audits the call (`PAIR_PERMITS` is audited); `crdt`
        // serves this, so it stays off the telemetry stack.
        self.0.redeem(&endpoint, request, now_ms())
    }
}
