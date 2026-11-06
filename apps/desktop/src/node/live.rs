//! Live sync: a `DocSync` bridge per note with each peer, so an edit
//! reaches the others within a round trip rather than at the next sweep.
//!
//! For each peer it dials, the node keeps one iroh connection and, on it,
//! one [`crdt::peer::bridge`] per note either side holds (up to
//! [`Config::live_max`](super::Config::live_max)): the note's document in
//! this node's registry and the peer's, kept in step both ways. A note
//! the peer holds and this node does not is registered first, so its file
//! is made here. A bridge that meets a second seed of its note
//! ([`crdt::peer::SessionEnd::ForeignLineage`]) settles it
//! ([`crdt::peer::adopt_lineage`]) and starts again.
//!
//! The sweep stays (`super::sweep_forever`): it reaches a peer this node
//! does not bridge (one it only learnt through the lookup, whose id is
//! lower; notes past the cap) and catches up whatever a dropped
//! connection missed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::iroh_link;
use architect_devices::Admission;
use crdt::peer::{DocAt, DocPeerClient, PeerLane, SelfLane, SessionEnd};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::catalog::VaultCatalog;
use super::{Dial, IrohMode, targets};

/// How often the supervisor looks at its peers.
const TICK: Duration = Duration::from_secs(1);
/// The longest a peer's notes go unlisted (new notes get a bridge).
const RELIST_MAX: Duration = Duration::from_secs(5);

/// What the live side shares between its tasks.
#[derive(Clone)]
pub(super) struct Live {
    pub endpoint: iroh::Endpoint,
    pub lane: Arc<SelfLane>,
    pub catalog: VaultCatalog,
    pub admission: Admission,
    pub configured: Vec<Dial>,
    pub mode: IrohMode,
    pub every: Duration,
    pub max: usize,
}

struct Peer {
    task: JoinHandle<()>,
    ended: Instant,
}

/// Bridge every peer this node dials: the configured ones, and those the
/// lookup reaches whose endpoint id is higher than this node's (so two
/// nodes that each know the other open one set of bridges, not two).
pub(super) async fn live_forever(live: Live) {
    if live.max == 0 {
        return;
    }
    let me = architect_devices::endpoint_key(&live.endpoint.id().to_string());
    let retry = live.every.min(RELIST_MAX);
    let mut peers: HashMap<String, Peer> = HashMap::new();
    loop {
        let configured: HashSet<&str> = live
            .configured
            .iter()
            .map(|d| d.endpoint.as_str())
            .collect();
        for peer in targets(&live.configured, &live.admission, live.mode) {
            let ours = configured.contains(peer.endpoint.as_str()) || me < peer.endpoint;
            if peer.endpoint == me || !ours {
                continue;
            }
            let due = peers
                .get(&peer.endpoint)
                .is_none_or(|p| p.task.is_finished() && p.ended.elapsed() >= retry);
            if due {
                let task = tokio::spawn(live_peer(live.clone(), peer.clone()));
                peers.insert(
                    peer.endpoint.clone(),
                    Peer {
                        task,
                        ended: Instant::now(),
                    },
                );
            }
        }
        for peer in peers.values_mut() {
            if !peer.task.is_finished() {
                peer.ended = Instant::now();
            }
        }
        tokio::time::sleep(TICK).await;
    }
}

/// One peer: a connection, its notes listed now and again, a bridge per
/// note. Returns when the connection fails (the supervisor dials again).
async fn live_peer(live: Live, peer: Dial) {
    let Ok(link) = iroh_link::connect(&live.endpoint, peer.addr.clone()).await else {
        return;
    };
    let Ok(client) = vox::initiator_on(link).establish::<DocPeerClient>().await else {
        return;
    };
    let mut bridges: HashMap<Uuid, JoinHandle<()>> = HashMap::new();
    let refused: Arc<Mutex<HashSet<Uuid>>> = Arc::default();
    let relist = live.every.min(RELIST_MAX);
    loop {
        let theirs = match client.list().await {
            Ok(theirs) => theirs,
            Err(why) => {
                tracing::debug!(peer = %peer.endpoint, ?why, "live: the peer stopped answering");
                break;
            }
        };
        let mut notes: BTreeMap<Uuid, String> = BTreeMap::new();
        for held in theirs {
            let at = DocAt {
                doc_id: held.doc_id,
                name: held.name.clone(),
            };
            if live.catalog.register(&at).is_ok() {
                notes.insert(held.doc_id, held.name);
            }
        }
        for held in live.catalog.notes() {
            notes.entry(held.doc_id).or_insert(held.name);
        }
        let skip = refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for (doc_id, name) in notes.into_iter().take(live.max) {
            if skip.contains(&doc_id) || bridges.get(&doc_id).is_some_and(|b| !b.is_finished()) {
                continue;
            }
            let (lane, client, refused) =
                (Arc::clone(&live.lane), client.clone(), Arc::clone(&refused));
            bridges.insert(
                doc_id,
                tokio::spawn(async move {
                    keep_live(doc_id, &lane, &client, &name, &refused).await;
                }),
            );
        }
        tokio::time::sleep(relist).await;
    }
    for bridge in bridges.into_values() {
        bridge.abort();
    }
}

/// One note's bridge, settling a second seed and starting again; a
/// refusal is remembered for the connection.
async fn keep_live(
    doc_id: Uuid,
    lane: &SelfLane,
    client: &DocPeerClient,
    name: &str,
    refused: &Mutex<HashSet<Uuid>>,
) {
    let remote = PeerLane { client, name };
    loop {
        match crdt::peer::bridge(doc_id, lane, &remote).await {
            SessionEnd::ForeignLineage => {
                if let Err(end) = crdt::peer::adopt_lineage(doc_id, lane, &remote).await {
                    tracing::debug!(%name, ?end, "live: the second seed was not settled");
                    return;
                }
            }
            SessionEnd::Refused(why) => {
                tracing::debug!(%name, %why, "live: refused");
                refused
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(doc_id);
                return;
            }
            SessionEnd::Dropped => return,
        }
    }
}
