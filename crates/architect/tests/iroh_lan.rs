//! LAN peer discovery for the iroh transport (feature `iroh-lan`).
//!
//! Two endpoints bound with [`PeerLookup::Lan`] — relays off, nothing
//! published to n0, no internet needed — dial each other by **bare**
//! [`iroh::EndpointId`]: the only way the client learns the server's
//! addresses is the mDNS-style lookup on the local network. The control
//! test shows the same dial fails with no lookup at all, so the first
//! test passing is the LAN lookup's doing.
//!
//! Needs a network interface that carries multicast (any real LAN
//! interface; the loopback-only sandbox some CI containers run in does
//! not). The manual two-machine check is in `examples/iroh_lan.rs`.
// Integration-test crate: see the matching block in `tests/iroh_link.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::future_not_send
)]
#![cfg(feature = "iroh-lan")]

use std::time::Duration;

use architect::iroh_link::{self, PeerLookup, VOX_ALPN};

/// mDNS lookups wait at least 10 s for an answer; leave room for that
/// plus the QUIC handshake.
const DIAL_DEADLINE: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread")]
async fn lan_peers_dial_by_bare_id() {
    let server = iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), PeerLookup::Lan)
        .await
        .expect("bind server");
    let client = iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), PeerLookup::Lan)
        .await
        .expect("bind client");

    // Serve an acceptor that rejects every lane: completing the vox
    // handshake over the dialled link is the point.
    let serve_task = tokio::spawn({
        let server = server.clone();
        async move {
            let acceptor = iroh_link::lane_acceptor_fn(|_req, _connection| {
                Err(vox_core::LaneRejection::new(
                    vox_core::LaneRejectReason::UnknownService,
                ))
            });
            iroh_link::serve_endpoint(&server, acceptor).await;
        }
    });

    // A bare id: no addresses, no relay URL.
    let link = tokio::time::timeout(DIAL_DEADLINE, iroh_link::connect(&client, server.id()))
        .await
        .expect("LAN lookup + dial timed out")
        .expect("dial by bare id over the LAN");
    assert_eq!(link.remote_id(), server.id());
    let connection = vox_core::initiator_on(link)
        .establish_connection()
        .await
        .expect("vox handshake over a LAN-discovered link");
    drop(connection);

    client.close().await;
    server.close().await;
    serve_task.await.expect("serve task");
}

/// Control: with no address lookup at all, the same bare-id dial has
/// nowhere to learn addresses from and fails.
#[tokio::test]
async fn bare_id_needs_a_lookup() {
    let server = iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), PeerLookup::Lan)
        .await
        .expect("bind server");
    let client = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .alpns(vec![VOX_ALPN.to_vec()])
        .bind()
        .await
        .expect("bind lookup-less client");

    let dialled = tokio::time::timeout(DIAL_DEADLINE, iroh_link::connect(&client, server.id()))
        .await
        .expect("a dial with no lookup should fail, not hang");
    assert!(dialled.is_err(), "dialled a bare id with no address lookup");

    client.close().await;
    server.close().await;
}

#[test]
fn n0_stays_the_default() {
    assert_eq!(PeerLookup::default(), PeerLookup::N0);
}
