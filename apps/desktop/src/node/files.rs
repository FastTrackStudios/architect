//! Projects and asset libraries on the node (`architect_files::sync`).
//!
//! With [`Config::files_dir`](super::Config::files_dir) the node runs the
//! files engine: on starcommand it holds every root in the tree in place
//! ([`Config::files_hold`](super::Config::files_hold)); on a laptop it is
//! the device's agent, keeping a working copy of each pinned root. Each
//! root's manifest is a plain document in the node's registry, so it
//! travels the same lanes as the notes (the catalogue lists it, a live
//! bridge carries it); its bytes travel `BlobTransfer`, on the same
//! connections, from whichever peer has them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use architect::blob::{
    BlobTransferClient, BlobTransferDispatcher, Fetcher, LocalFs, Remote, VoxRemote,
    blob_transfer_service_descriptor,
};
use architect::host::{DavRequest, EngineHost};
use architect::iroh_link;
use architect::layer::Layer as _;
use architect::layer::LayerRouter;
use architect_devices::Admission;
use architect_files::sync::dav::Dav;
use architect_files::sync::{BLOB_PERMITS, Engine, EngineConfig, TreeStore};
use crdt::DocRegistry;
use crdt::p2p::More;

use super::{Config, Dial, IrohMode, targets};

/// How often the blob connections are looked at.
const DIAL_EVERY: Duration = Duration::from_secs(3);

/// The engine for `config`, whose documents come from `registry` once set.
pub(super) fn engine(
    config: &Config,
    files_dir: &Path,
    registry: Arc<OnceLock<DocRegistry>>,
) -> Engine {
    let data = config.data_dir.join("files");
    let local = LocalFs::new(data.join("blobs"));
    Engine::new(
        EngineConfig::new(&config.name, data, files_dir.to_owned(), config.files_hold),
        TreeStore::new(local.clone()),
        Fetcher::new(local),
        Arc::new(move |doc_id| {
            let registry = registry.clone();
            Box::pin(async move {
                let doc = registry.get()?.doc(doc_id).await.ok()?;
                Some(doc.loro().clone())
            })
        }),
    )
    .with_transcoder(architect_files::server::FfmpegTranscoder::new())
}

/// The agent on the node's HTTP side, when it listens on loopback only (a
/// laptop's agent: what the window and the device's `WebDAV` mount talk
/// to). A node bound wider serves `FILES_*` only over `/mcp`, behind its
/// bearer.
pub(super) fn mount(host: EngineHost, engine: &Engine, bind: &str) -> EngineHost {
    let loopback = bind
        .parse::<std::net::SocketAddr>()
        .is_ok_and(|a| a.ip().is_loopback());
    if !loopback {
        return host;
    }
    let agent = architect::layers![architect_files::agent::files::Service].provide_http(engine);
    let (dav, engine) = (Dav::default(), engine.clone());
    host.plugin(agent).dav("/dav", move |req: DavRequest| {
        let (dav, engine) = (dav.clone(), engine.clone());
        async move {
            let method = req.request.method().clone();
            let headers = req.request.headers().clone();
            let body = req.request.into_body();
            dav.handle(&engine, &method, &req.mount, &req.rel, &headers, body)
                .await
        }
    })
}

/// `BlobTransfer` on the node's peer lane, behind its gate.
pub(super) fn more(engine: &Engine) -> More {
    More {
        router: Some(LayerRouter::new().with(
            blob_transfer_service_descriptor(),
            BlobTransferDispatcher::new(engine.blob_host()),
        )),
        permits: vec![(blob_transfer_service_descriptor(), &BLOB_PERMITS)],
    }
}

/// Keep a `BlobTransfer` connection to every peer the node can dial, the
/// engine's fetcher reading from them in turn.
pub(super) async fn dial_forever(
    engine: Engine,
    endpoint: iroh::Endpoint,
    configured: Vec<Dial>,
    admission: Admission,
    mode: IrohMode,
) {
    let me = architect_devices::endpoint_key(&endpoint.id().to_string());
    let mut peers: BTreeMap<String, BlobTransferClient> = BTreeMap::new();
    loop {
        let mut changed = false;
        let wanted = targets(&configured, &admission, mode);
        for peer in &wanted {
            if peer.endpoint == me {
                continue;
            }
            if let Some(client) = peers.get(&peer.endpoint)
                && client.size("0".repeat(64)).await.is_ok()
            {
                continue;
            }
            peers.remove(&peer.endpoint);
            changed = true;
            let Ok(link) = iroh_link::connect(&endpoint, peer.addr.clone()).await else {
                continue;
            };
            if let Ok(client) = vox::initiator_on(link)
                .establish::<BlobTransferClient>()
                .await
            {
                peers.insert(peer.endpoint.clone(), client);
            }
        }
        if changed {
            let remotes: Vec<Arc<dyn Remote>> = peers
                .values()
                .map(|c| -> Arc<dyn Remote> { Arc::new(VoxRemote::new(c.clone())) })
                .collect();
            tracing::debug!(peers = remotes.len(), "files: blob peers");
            engine.fetcher.set_remotes(remotes);
        }
        tokio::time::sleep(DIAL_EVERY).await;
    }
}
