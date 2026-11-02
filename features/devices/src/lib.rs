// `#[vox::service]` writes `async fn` into its trait.
#![allow(async_fn_in_trait)]

//! Devices (`docs/content/architecture/devices.md`): which devices a node
//! lets in, and how a device comes to be one of them.
//!
//! Every node (a phone, a laptop, an org's server, an engine) serves its
//! lanes over iroh, where the connection proves the caller's endpoint id
//! (`Principal::Host`). [`Admission`] is the one table every lane on the
//! node checks that id against. A device gets into it three ways, each a
//! [`Grant`]:
//!
//! - **The account** ([`Grant::Account`]): the owner's other devices, from
//!   the issuer's device address book. Feature `account`: [`account::pass`]
//!   registers this device, holds the account's shared org and lets the
//!   others in; [`account::drive`] runs it on start, on every reconnect
//!   and every few minutes.
//! - **An org** ([`Grant::Org`]): a member enrols their device in an org
//!   with a role at most their own, listed and revocable. Feature
//!   `server`: [`server::EnrolmentHost`] serves [`DeviceEnrolment`] on the
//!   org's lanes, and [`server::project_members`] makes each enrolled
//!   device a member the org's `ConfinedEngine` gives the role's rules.
//! - **Pairing** ([`Grant::Paired`]): a device that is not signed in, or
//!   is somebody else's, redeems an offer made on a signed-in device — a
//!   six-digit code, or a QR code or link whose secret rides the URL
//!   fragment ([`PairTicket`]) — and gets the offer's narrow [`Scope`]s
//!   ([`Pairings`], served as [`DevicePairing`] by [`PairingHost`]).
//!
//! Revoking ([`Admission::revoke`], [`Admission::revoke_org`], an account
//! pass that no longer lists it) refuses the device's next call, ends
//! the sessions it holds where a lane waits on [`Admission::ends`]
//! (`crdt::p2p`), and closes its connections ([`closer`], feature `iroh`;
//! `EngineHost::iroh_until`). Each [`Peer`] also carries its [`Route`]s:
//! how to reach it.
//!
//! - default (wasm-clean): the model, [`Admission`] (the gate's
//!   `PermissionEngine` for devices) and [`Pairings`].
//! - `vox`: the two services, their clients and permit tables; natively
//!   [`PairingHost`].
//! - `server` (native): [`server`].
//! - `account`: [`account`], over auth's `DeviceService`.
//! - `iroh` (native): [`serve`], [`closer`].

#[cfg(feature = "account")]
pub mod account;
mod admission;
#[cfg(all(feature = "iroh", not(target_arch = "wasm32")))]
mod iroh;
mod model;
mod pairing;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "vox")]
mod service;

pub use admission::Admission;
#[cfg(all(feature = "iroh", not(target_arch = "wasm32")))]
pub use iroh::{closer, serve};
pub use model::{Grant, Peer, Route, Scope, doc_resource, endpoint_key};
pub use pairing::{
    CODE_TRIES, OFFER_TTL_MS, PAIR_ACTION, PAIR_RESOURCE, PairError, PairOffer, PairProof,
    PairRequest, PairTicket, Paired, Pairings, TICKET_SCHEME, TicketError,
};
#[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
pub use service::PairingHost;
#[cfg(feature = "vox")]
pub use service::{
    DeviceEnrolment, DeviceEnrolmentClient, DeviceEnrolmentDispatcher, DevicePairing,
    DevicePairingClient, DevicePairingDispatcher, ENROL_PERMITS, EnrolError, Enrolment,
    PAIR_PERMITS, device_enrolment_service_descriptor, device_pairing_service_descriptor,
};
