//! The shared clock presence is stamped in: each peer's driver estimates
//! its presence host's clock, and a playhead stamped in it is read back
//! where the leader really is, though the two peers' clocks disagree with
//! the host's by seconds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::arithmetic_side_effects
)]
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::presence::{Here, PeerState, PlayState, clock_ms};
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, PresenceHost, PresencePeer,
    doc_presence_service_descriptor,
};
use uuid::Uuid;

/// The host's clock runs this far ahead of the peers' (one process, so
/// the peers share `clock_ms`; the offset is the host's to give).
const AHEAD_MS: f64 = 5_000.0;

async fn join(local: &LocalServer, doc: Uuid) -> PresencePeer {
    let client: DocPresenceClient = local.establish().await.unwrap();
    let (peer, mut driver) = PresencePeer::new(doc, 30_000);
    tokio::spawn(async move {
        let _ = driver.run(&client).await;
    });
    peer
}

async fn eventually(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_playhead_is_stamped_and_read_in_the_hosts_clock() {
    let doc = Uuid::new_v4();
    let host = PresenceHost::new(doc, 30_000).with_clock(Arc::new(|| clock_ms() + AHEAD_MS));
    let router = LayerRouter::new().with(
        doc_presence_service_descriptor(),
        DocPresenceDispatcher::new(host),
    );
    let local = LocalServer::serve(router, Scope::new());

    // Before its driver has run, a peer has no estimate: nothing to stamp.
    let (alone, _driver) = PresencePeer::new(doc, 30_000);
    let here = Here::new(alone, "alone", PeerState::new("alone", "Alone"));
    assert!(!here.play_now(true, 0.0, 1.0));

    let lead = join(&local, doc).await;
    let follow = join(&local, doc).await;
    for peer in [&lead, &follow] {
        let clock = peer.clock().clone();
        eventually("the host's clock is estimated", || {
            clock
                .offset_ms()
                .is_some_and(|o| (o - AHEAD_MS).abs() < 2.0)
        })
        .await;
        assert!(peer.clock().round_trip_ms().unwrap() >= 0.0);
    }

    // The leader plays from 10 s, stamped in the host's clock.
    let here = Here::new(lead.clone(), "lead", PeerState::new("lead", "Lead"));
    assert!(here.play_now(true, 10.0, 1.0));
    let started = clock_ms();
    let play = {
        let follow = follow.clone();
        let mut got = None;
        eventually("the follower hears the playhead", || {
            got = follow.states().get("lead/play").and_then(PlayState::decode);
            got.is_some()
        })
        .await;
        got.unwrap()
    };
    // Its stamp is the host's clock, 5 s ahead of either peer's own.
    assert!((play.at_ms - (started + AHEAD_MS)).abs() < 50.0, "{play:?}");
    // Read on the shared clock, it is where the leader is now: 10 s plus
    // what has passed. On a peer's own clock it would be 5 s off.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let shown = play.position_at(follow.clock().now_ms().unwrap());
    let truth = 10.0 + (clock_ms() - started) / 1000.0;
    assert!(
        (shown - truth).abs() < 0.005,
        "shown {shown}, truth {truth}"
    );
    let wrong = play.position_at(clock_ms());
    assert!((wrong - truth).abs() > 4.9, "own clock: {wrong}");
}
