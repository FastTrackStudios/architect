//! FTS nodes on the local network: advertised under one service name,
//! listed by a [`LanBrowser`].
//!
//! [`PeerLookup::Lan`](super::PeerLookup::Lan) and
//! [`PeerLookup::N0AndLan`](super::PeerLookup::N0AndLan) advertise the
//! endpoint as `<endpoint id>._fts._udp.local.` (swarm-discovery's
//! mDNS-like records; iroh's QUIC is UDP, hence `_udp`). Advertising is
//! what lets a peer dial the endpoint by bare id; browsing is the other
//! half: which nodes are here at all, so an app can offer "devices
//! nearby" before it knows any id.

use std::collections::HashMap;

use futures::StreamExt as _;
use iroh::EndpointId;
use iroh::address_lookup::AddressLookupBuilderError;
use iroh::endpoint_info::UserData;
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup, MdnsAddressLookupBuilder};

/// The service name FTS nodes advertise on the local network: records
/// read `<endpoint id>._fts._udp.local.`. One name for every FTS app, so
/// a phone running one app finds a laptop running another.
pub const LAN_SERVICE: &str = "fts";

/// The address lookup an endpoint built for the LAN carries: advertises
/// this endpoint and resolves others under [`LAN_SERVICE`].
pub(super) fn advertiser() -> MdnsAddressLookupBuilder {
    MdnsAddressLookup::builder().service_name(LAN_SERVICE)
}

/// Put `name` ("Cody's iPhone") beside this endpoint's id in what it
/// advertises, so a [`LanBrowser`] elsewhere can show it. Longer names
/// are cut at the record's limit.
pub fn advertise_name(endpoint: &iroh::Endpoint, name: &str) {
    let mut cut = name.len().min(UserData::MAX_LENGTH);
    while !name.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    let data = name
        .get(..cut)
        .and_then(|n| UserData::try_from(n.to_owned()).ok());
    endpoint.set_user_data_for_address_lookup(data);
}

/// What a [`LanBrowser`] saw change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanEvent {
    /// A node appeared on the network, or its name changed.
    Found {
        endpoint: EndpointId,
        /// What it advertises itself as ([`advertise_name`]), if anything.
        name: Option<String>,
    },
    /// A node stopped answering (left the network, closed, asleep).
    Lost { endpoint: EndpointId },
}

/// Lists the FTS nodes on the local network.
///
/// A listen-only participant in the [`LAN_SERVICE`] swarm: it asks who
/// is there and advertises nothing (the endpoint's own lookup does
/// that). Must be made inside a tokio runtime.
#[derive(Debug, Clone)]
pub struct LanBrowser {
    mdns: MdnsAddressLookup,
    own: EndpointId,
}

impl LanBrowser {
    /// Browse for the nodes other than `own` (this node's endpoint id).
    ///
    /// # Errors
    ///
    /// The network allows neither IPv4 nor IPv6 multicast.
    pub fn new(own: EndpointId) -> Result<Self, AddressLookupBuilderError> {
        // A throwaway id of its own: the browser is not this endpoint
        // and must not answer for it.
        let browser_id = iroh::SecretKey::generate().public();
        let mdns = MdnsAddressLookup::builder()
            .service_name(LAN_SERVICE)
            .advertise(false)
            .build(browser_id)?;
        Ok(Self { mdns, own })
    }

    /// The nodes seen from now on: each one [`LanEvent::Found`] once
    /// (again when its name changes), [`LanEvent::Lost`] when it goes.
    pub async fn events(&self) -> LanEvents {
        LanEvents {
            inner: Box::new(self.mdns.subscribe().await),
            own: self.own,
            seen: HashMap::new(),
            _mdns: self.mdns.clone(),
        }
    }
}

/// The stream of a [`LanBrowser`]'s [`LanEvent`]s.
pub struct LanEvents {
    inner: Box<dyn futures::Stream<Item = DiscoveryEvent> + Send + Unpin>,
    own: EndpointId,
    seen: HashMap<EndpointId, Option<String>>,
    /// Keeps the discoverer running while the stream is held.
    _mdns: MdnsAddressLookup,
}

impl LanEvents {
    /// The next change; `None` once the browser has stopped.
    pub async fn next(&mut self) -> Option<LanEvent> {
        while let Some(event) = self.inner.next().await {
            match event {
                DiscoveryEvent::Discovered { endpoint_info, .. } => {
                    let endpoint = endpoint_info.endpoint_id;
                    if endpoint == self.own {
                        continue;
                    }
                    let name = endpoint_info.data.user_data().map(ToString::to_string);
                    if self.seen.get(&endpoint) == Some(&name) {
                        continue;
                    }
                    self.seen.insert(endpoint, name.clone());
                    return Some(LanEvent::Found { endpoint, name });
                }
                DiscoveryEvent::Expired { endpoint_id } => {
                    if self.seen.remove(&endpoint_id).is_some() {
                        return Some(LanEvent::Lost {
                            endpoint: endpoint_id,
                        });
                    }
                }
                _ => {}
            }
        }
        None
    }
}

impl std::fmt::Debug for LanEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanEvents")
            .field("own", &self.own)
            .field("seen", &self.seen.len())
            .finish_non_exhaustive()
    }
}
