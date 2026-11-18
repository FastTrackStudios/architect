//! How a document's presence and clock travel between two nodes over the
//! org's iroh lane (session#192): the round trip of the host's clock and
//! how long a published move takes to arrive, dialled by endpoint id alone.
//!
//! ```text
//! presence_probe id KEYFILE                       # the follower's endpoint id (makes KEYFILE)
//! presence_probe host FOLLOWER_ID                 # serves, leads; prints its id
//! presence_probe follow KEYFILE HOST_ID SECONDS   # measures, then reports
//! ```
#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing
)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;
use std::time::Duration;

use architect::iroh_link::{self, PeerLookup, VOX_ALPN};
use architect::layer::handler_acceptor;
use architect::permissions_gate::{PermissionsGate, StaticPrincipal, UnlistedPolicy};
use architect_permissions::{AllowAll, Principal};
use crdt::CrdtDoc;
use crdt::p2p::{Admission, NodeAccess, Shares, node_router};
use crdt::peer::OpenDocs;
use crdt::registry::DocRegistry;
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, PresencePeer, doc_presence_service_descriptor,
};
use uuid::Uuid;

/// The one document both sides use.
const DOC: Uuid = Uuid::from_u128(0x5e55_1011_0000_4000_8000_0000_0000_0192);

async fn endpoint(key: Option<iroh::SecretKey>) -> iroh::Endpoint {
    let builder = iroh_link::endpoint_builder(PeerLookup::N0).alpns(vec![VOX_ALPN.to_vec()]);
    let builder = match key {
        Some(key) => builder.secret_key(key),
        None => builder,
    };
    builder.bind().await.expect("bind")
}

fn key(path: &str) -> iroh::SecretKey {
    if let Ok(bytes) = std::fs::read(path) {
        let bytes: [u8; 32] = bytes.try_into().expect("a 32-byte key");
        return iroh::SecretKey::from_bytes(&bytes);
    }
    let key = iroh::SecretKey::generate();
    std::fs::write(path, key.to_bytes()).expect("write key");
    key
}

/// A presence peer on `registry`, in-process, calling as the node itself.
async fn own_presence(registry: DocRegistry) -> PresencePeer {
    let router = architect::LayerRouter::new().with(
        doc_presence_service_descriptor(),
        DocPresenceDispatcher::new(registry),
    );
    let me = Principal::Service {
        name: "self".to_owned(),
    };
    let gated = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(me)))
        .unlisted(UnlistedPolicy::Allow)
        .wrap(router);
    let (client_link, server_link) = vox::memory_link_pair(64);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(gated))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    let client: DocPresenceClient = vox::initiator_on(client_link).establish().await.unwrap();
    let (peer, mut driver) = PresencePeer::new(DOC, 30_000);
    tokio::spawn(async move {
        let _ = driver.run(&client).await;
    });
    peer
}

async fn host(follower: &str) {
    let ep = endpoint(None).await;
    println!("host {}", ep.id());
    let devices = Admission::new();
    devices.set_account([architect_devices::Peer::new(follower, "probe", "probe")]);
    let access = NodeAccess::new(devices, Shares::new());
    let registry = DocRegistry::new(|_| Box::pin(async { Ok(CrdtDoc::ephemeral()) }))
        .with_authorizer({
            let access = access.clone();
            move |who, doc_id| {
                matches!(who, Some(Principal::Service { name }) if name == "self")
                    || access.admits(who, doc_id)
            }
        });
    let serving = ep.clone();
    let (r, a) = (registry.clone(), access.clone());
    tokio::spawn(async move {
        iroh_link::serve_peers(&serving, move |remote| {
            node_router(
                r.clone(),
                OpenDocs(r.clone()),
                a.clone(),
                &remote.to_string(),
            )
        })
        .await;
    });
    // The leader: a move a second, stamped in the host's own clock.
    let lead = own_presence(registry).await;
    loop {
        lead.set("lead/play", crdt::presence::clock_ms());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn stats(name: &str, mut v: Vec<f64>) {
    if v.is_empty() {
        println!("{name}: no samples");
        return;
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    println!(
        "{name}: n {} min {:.2} p50 {:.2} p90 {:.2} p99 {:.2} max {:.2} ms",
        v.len(),
        v[0],
        at(0.5),
        at(0.9),
        at(0.99),
        v[v.len() - 1]
    );
}

async fn follow(keyfile: &str, host: &str, seconds: f64) {
    let ep = endpoint(Some(key(keyfile))).await;
    let host_id: iroh::EndpointId = host.parse().expect("an endpoint id");
    let dial = std::time::Instant::now();
    let link = iroh_link::connect(&ep, host_id).await.expect("dial");
    println!(
        "dialled {host} by id in {:.0} ms",
        dial.elapsed().as_secs_f64() * 1e3
    );
    let client: DocPresenceClient = vox::initiator_on(link).establish().await.unwrap();
    let (peer, mut driver) = PresencePeer::new(DOC, 30_000);
    let pinger = client.clone();
    tokio::spawn(async move {
        let _ = driver.run(&client).await;
    });
    let clock = peer.clock().clone();
    let (mut rtts, mut arrivals, mut offsets) = (Vec::new(), Vec::new(), Vec::new());
    let mut last = None;
    let end = std::time::Instant::now() + Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < end {
        // A raw ping of the host's clock: its round trip.
        let t0 = crdt::presence::clock_ms();
        if pinger.now().await.is_ok() {
            rtts.push(crdt::presence::clock_ms() - t0);
        }
        if let Some(o) = clock.offset_ms() {
            offsets.push(o);
        }
        // A move: when it arrived, in the host's clock, after it was stamped.
        let stamp = peer
            .states()
            .get("lead/play")
            .and_then(|v| v.clone().into_double().ok());
        if let (Some(stamp), Some(now)) = (stamp, clock.now_ms())
            && last != Some(stamp.to_bits())
        {
            last = Some(stamp.to_bits());
            arrivals.push(now - stamp);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    stats("clock round trip", rtts);
    stats("move arrival (one way, by the shared clock)", arrivals);
    if let (Some(lo), Some(hi)) = (
        offsets.iter().copied().reduce(f64::min),
        offsets.iter().copied().reduce(f64::max),
    ) {
        println!(
            "offset estimate spread: {:.3} ms ({} samples)",
            hi - lo,
            offsets.len()
        );
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["id", keyfile] => {
            let key = key(keyfile);
            println!("{}", key.public());
        }
        ["host", follower] => host(follower).await,
        ["follow", keyfile, host_id, secs] => follow(keyfile, host_id, secs.parse().unwrap()).await,
        _ => println!(
            "presence_probe id KEYFILE | host FOLLOWER_ID | follow KEYFILE HOST_ID SECONDS"
        ),
    }
}
