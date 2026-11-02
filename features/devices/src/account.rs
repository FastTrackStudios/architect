//! This device on the account (feature `account`): one pass over the
//! issuer's device address book (auth's `DeviceService`), and a driver
//! that runs it on start, on every reconnect and every few minutes.
//!
//! A person signed in on a phone and a laptop with one account expects
//! their work on both. Each device runs its own node, and nodes sync
//! directly once each lets the other in; the issuer only introduces them.
//! [`pass`] is one round of a device's side of that:
//!
//! 1. register this install (`DeviceRegistration`), offering an org of its
//!    own as the account's shared org if the account has none — the first
//!    device's org is the one they all share ([`AccountNode::offer`]);
//! 2. take this device's leftover rows off the account (an id an older
//!    build registered under, a same-named row nobody refreshes);
//! 3. hold the shared org: adopt it or join it from another device
//!    ([`AccountNode::hold`]), and say this device's endpoint for it;
//! 4. let the account's other devices in: [`Admission::set_account`].
//!
//! Every step is idempotent, so the app runs it often ([`drive`]): the
//! first device learns of the second on its next pass. A device taken off
//! the account loses [`Grant::Account`](crate::Grant::Account) on every
//! other device's next pass, and its connections to them close.

use std::time::Duration;

use auth_proto::{
    AccountOrg, DeviceDirectory, DeviceRegistration, DeviceServiceClient, PeerDevice,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::admission::Admission;
use crate::model::{Peer, Route};

/// Who this device is, as it tells the issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThisDevice {
    /// Its id on the account (`auth_client::this_device::device_id`).
    pub device_id: Uuid,
    pub name: String,
    /// `ios`, `macos`, `linux`, `windows`, `server`, …
    pub platform: String,
    /// The iroh relay it is reachable through, if it pins one.
    pub relay_url: Option<String>,
    /// An id this device was registered under before (an older build's):
    /// its row is taken off the account by the pass.
    pub retire: Option<Uuid>,
}

/// The app's side of the account's shared org.
///
/// What it offers, and how it comes to hold it. Task's node adopts or
/// joins an org; an app with one endpoint and no org of its own
/// ([`FixedEndpoint`]) only says where it is.
#[allow(async_fn_in_trait)]
pub trait AccountNode {
    /// The org this device would share if the account has none yet (its
    /// own personal org), or `None`.
    fn offer(&self) -> Option<AccountOrg>;

    /// Hold `org` (the account's shared org, `None` while it has none)
    /// here — it already does, or join it from one of `peers` (the
    /// account's other devices) — and answer with this device's endpoint
    /// id. `Ok(None)`: not yet (nothing shared, no peer to join from).
    async fn hold(
        &self,
        org: Option<&AccountOrg>,
        peers: &[PeerDevice],
    ) -> Result<Option<String>, String>;
}

/// An app with one iroh endpoint and no org to offer (an engine, Studio):
/// it is at that endpoint whatever the account shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedEndpoint(pub String);

impl AccountNode for FixedEndpoint {
    fn offer(&self) -> Option<AccountOrg> {
        None
    }

    async fn hold(
        &self,
        _: Option<&AccountOrg>,
        _: &[PeerDevice],
    ) -> Result<Option<String>, String> {
        Ok(Some(self.0.clone()))
    }
}

/// What a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountReport {
    /// The org the account's devices share, once there is one.
    pub org: Option<AccountOrg>,
    /// This device's endpoint for it, once it holds it.
    pub endpoint: Option<String>,
    /// The account's other devices.
    pub others: Vec<PeerDevice>,
    /// Their endpoints, now let in here.
    pub admitted: Vec<String>,
    /// This device's leftover rows are off the account (or there were
    /// none): an app can forget [`ThisDevice::retire`].
    pub retired: bool,
}

/// Why a pass stopped. Every one is retried on the next pass.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PassError {
    /// The issuer refused or could not be reached.
    #[error("the account server: {0}")]
    Issuer(String),
    /// The app could not hold the shared org.
    #[error("holding the account's org: {0}")]
    Node(String),
}

fn issuer(e: impl std::fmt::Debug) -> PassError {
    PassError::Issuer(format!("{e:?}"))
}

/// One pass of this device's side of the account's address book.
///
/// With `token` (the account's session or access token), letting the
/// others into `admission`. Send when `node`'s `hold` is (the future's
/// Send-ness is the app's node's).
///
/// # Errors
///
/// [`PassError`]: what stopped it. What it did before stopping stands.
#[allow(clippy::future_not_send)]
pub async fn pass(
    devices: &DeviceServiceClient,
    token: &str,
    me: &ThisDevice,
    node: &impl AccountNode,
    admission: &Admission,
) -> Result<AccountReport, PassError> {
    let register =
        |endpoint_id: Option<String>, offer_org: Option<AccountOrg>| DeviceRegistration {
            device_id: me.device_id,
            name: me.name.clone(),
            platform: me.platform.clone(),
            endpoint_id,
            relay_url: me.relay_url.clone(),
            offer_org,
        };

    // 1. Here I am, and here is an org if the account has none.
    let first = devices
        .register_device(token.to_owned(), register(None, node.offer()))
        .await
        .map_err(issuer)?;
    // 2. Not twice.
    let stale = stale_rows(me, &first.devices, Utc::now());
    let retired = retire(devices, token, &stale).await;
    let others = |dir: &DeviceDirectory| -> Vec<PeerDevice> {
        dir.devices
            .iter()
            .filter(|d| d.device_id != me.device_id && !stale.contains(&d.device_id))
            .cloned()
            .collect()
    };

    // 3. Hold the shared org, and say where.
    let mut dir = first;
    let endpoint = node
        .hold(dir.account_org.as_ref(), &others(&dir))
        .await
        .map_err(PassError::Node)?
        .map(|e| crate::model::endpoint_key(&e));
    if endpoint.is_some() && listed_endpoint(&dir, me.device_id) != endpoint {
        dir = devices
            .register_device(token.to_owned(), register(endpoint.clone(), None))
            .await
            .map_err(issuer)?;
    }

    // 4. Let the others in.
    let others = others(&dir);
    let peers: Vec<Peer> = others.iter().filter_map(peer_of).collect();
    let admitted = peers.iter().map(|p| p.endpoint.clone()).collect();
    admission.set_account(peers);
    Ok(AccountReport {
        org: dir.account_org,
        endpoint,
        others,
        admitted,
        retired,
    })
}

/// One of the account's devices as this node lets it in: by its endpoint,
/// if it has one, reachable through its relay, if it pins one.
#[must_use]
pub fn peer_of(device: &PeerDevice) -> Option<Peer> {
    let endpoint = device.endpoint_id.as_deref()?;
    let mut peer =
        Peer::new(endpoint, device.name.clone(), device.platform.clone()).device(device.device_id);
    if let Some(url) = &device.relay_url {
        peer = peer.route(Route::Relay { url: url.clone() });
    }
    Some(peer)
}

fn listed_endpoint(dir: &DeviceDirectory, me: Uuid) -> Option<String> {
    dir.devices
        .iter()
        .find(|d| d.device_id == me)
        .and_then(|d| d.endpoint_id.as_deref())
        .map(crate::model::endpoint_key)
}

/// How long a same-named row goes unrefreshed before it is a leftover.
///
/// A live device refreshes its row every pass
/// (minutes); a leftover is never touched again.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(24);

/// The account's other rows that are this device.
///
/// [`ThisDevice::retire`], and a row with its name and platform under
/// another id that nobody has refreshed for [`STALE_AFTER`] (made before
/// every app took its id from `auth_client::this_device`, or by a
/// reinstall). The age tells a leftover from another device that shares
/// the name (two iPads called "Cody's iPad"): that one refreshes its row
/// too. A `server` never matches by name: servers' names are often alike.
#[must_use]
pub fn stale_rows(me: &ThisDevice, devices: &[PeerDevice], now: DateTime<Utc>) -> Vec<Uuid> {
    let name = me.name.trim();
    devices
        .iter()
        .filter(|d| d.device_id != me.device_id)
        .filter(|d| {
            Some(d.device_id) == me.retire
                || (me.platform != "server"
                    && d.platform == me.platform
                    && d.name.trim().eq_ignore_ascii_case(name)
                    && now.signed_duration_since(d.last_seen_at) > STALE_AFTER)
        })
        .map(|d| d.device_id)
        .collect()
}

/// Take `stale` off the account. `true` when none is left; a refusal is
/// retried on the next pass, never failing this one.
async fn retire(devices: &DeviceServiceClient, token: &str, stale: &[Uuid]) -> bool {
    let mut all_gone = true;
    for &old in stale {
        if let Err(e) = devices.forget_device(token.to_owned(), old).await {
            tracing::debug!(retired = %old, error = ?e, "devices: a leftover row is still listed");
            all_gone = false;
        }
    }
    all_gone
}

/// Run [`pass`] now, on every reconnect and every `every`.
///
/// Again whenever `reconnects` changes (the connection to the issuer
/// came back), until the future is dropped. `token` is asked before each pass (`None`: signed out, skip
/// it); `report` hears each outcome.
#[allow(clippy::future_not_send)]
pub async fn drive(
    devices: DeviceServiceClient,
    token: impl Fn() -> Option<String>,
    me: ThisDevice,
    node: impl AccountNode,
    admission: Admission,
    every: Duration,
    mut reconnects: tokio::sync::watch::Receiver<u64>,
    mut report: impl FnMut(&Result<AccountReport, PassError>),
) {
    let mut watching = true;
    loop {
        if let Some(token) = token() {
            let outcome = pass(&devices, &token, &me, &node, &admission).await;
            report(&outcome);
        }
        if watching {
            tokio::select! {
                () = architect::platform::sleep(every) => {}
                changed = reconnects.changed() => watching = changed.is_ok(),
            }
        } else {
            architect::platform::sleep(every).await;
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    fn row(id: Uuid, name: &str, platform: &str, hours_ago: i64) -> PeerDevice {
        PeerDevice {
            device_id: id,
            name: name.into(),
            platform: platform.into(),
            endpoint_id: None,
            relay_url: None,
            last_seen_at: Utc::now() - chrono::Duration::hours(hours_ago),
        }
    }

    fn me(name: &str, platform: &str, retire: Option<Uuid>) -> ThisDevice {
        ThisDevice {
            device_id: Uuid::new_v4(),
            name: name.into(),
            platform: platform.into(),
            relay_url: None,
            retire,
        }
    }

    #[test]
    fn leftover_rows_are_an_old_id_and_an_unrefreshed_name() {
        let (old, stray, twin, laptop) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let this = me("Cody's iPhone", "ios", Some(old));
        let listed = [
            row(this.device_id, "Cody's iPhone", "ios", 0),
            row(old, "iPhone", "ios", 0),
            row(stray, " cody's iphone ", "ios", 72),
            row(twin, "Cody's iPhone", "ios", 1),
            row(laptop, "airlock", "macos", 72),
        ];
        let mut stale = stale_rows(&this, &listed, Utc::now());
        stale.sort();
        let mut want = vec![old, stray];
        want.sort();
        assert_eq!(stale, want);
    }

    #[test]
    fn a_server_retires_only_its_old_id() {
        let (old, other) = (Uuid::new_v4(), Uuid::new_v4());
        let this = me("node", "server", Some(old));
        let listed = [
            row(old, "node", "server", 72),
            row(other, "node", "server", 72),
        ];
        assert_eq!(stale_rows(&this, &listed, Utc::now()), vec![old]);
    }

    #[test]
    fn a_device_without_an_endpoint_is_not_let_in() {
        let mut device = row(Uuid::new_v4(), "phone", "ios", 0);
        assert!(peer_of(&device).is_none());
        device.endpoint_id = Some("AB".into());
        device.relay_url = Some("https://relay.example".into());
        let peer = peer_of(&device).unwrap();
        assert_eq!(peer.endpoint, "ab");
        assert_eq!(peer.device_id, Some(device.device_id));
        assert_eq!(
            peer.routes,
            vec![Route::Relay {
                url: "https://relay.example".into()
            }]
        );
    }
}
