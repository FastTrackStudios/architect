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
//!
//! With a bucket ([`Config::blob_s3_bucket`](super::Config::blob_s3_bucket))
//! the bytes are also in S3 ([`backend`], [`replicate_forever`]): every
//! hash a manifest names is copied there from this node, and a hash this
//! node lacks is fetched from there as from any peer. Nodes that share
//! the bucket share its bytes, deduped by hash. With
//! [`Placement::Primary`] the bucket is where the bytes live and this
//! node's store is a cache.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use architect::blob::{
    BlobTransferClient, BlobTransferDispatcher, Fetcher, LocalFs, Remote, S3Config, S3Store,
    Sha256, VoxRemote, blob_transfer_service_descriptor,
};
use architect::host::{DavRequest, EngineHost};
use architect::iroh_link;
use architect::layer::Layer as _;
use architect::layer::LayerRouter;
use architect_devices::Admission;
use architect_files::sync::dav::Dav;
use architect_files::sync::verify::{PossessionDispatcher, possession_service_descriptor};
use architect_files::sync::{
    BLOB_PERMITS, Engine, EngineConfig, POSSESSION_PERMITS, PossessionClient, PossessionHost,
    Prover, TreeStore, VoxProver,
};
use crdt::DocRegistry;
use crdt::p2p::More;

use super::config::Placement;
use super::{Config, Dial, IrohMode, read_secret, targets};

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
    let engine_config = EngineConfig {
        site: config.files_site.clone().unwrap_or_default(),
        verify_window: Duration::from_secs(config.files_verify_secs.max(1)),
        scrub_budget: config.files_scrub_mib.saturating_mul(1 << 20),
        ..EngineConfig::new(&config.name, data, files_dir.to_owned(), config.files_hold)
    };
    let engine = Engine::new(
        engine_config,
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
    .with_transcoder(architect_files::server::FfmpegTranscoder::new());
    // Its attestations are signed with its iroh key: its holder id is its endpoint id.
    match iroh_link::load_or_create_secret_key(&config.data_dir.join("iroh.key")) {
        Ok(key) => engine.with_identity(key.to_bytes()),
        Err(error) => {
            tracing::warn!(%error, "files: the iroh key is not readable; attestations use a key of their own");
            engine
        }
    }
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

/// Mount the drive at `at` (Linux, macOS): every root `engine` knows, as a
/// real folder; a guard process unmounts it if the node dies. A mount
/// that fails is logged, and the node runs without it.
#[cfg(unix)]
pub(super) fn mount_drive(
    engine: &Engine,
    config: &Config,
    at: &Path,
) -> Option<architect_files::drive::Mounted> {
    engine.set_drive(Some(at.to_owned()));
    let tmp = config.data_dir.join("files").join("drive-tmp");
    let handle = tokio::runtime::Handle::current();
    match architect_files::drive::mount(engine.clone(), handle, at, tmp, true) {
        Ok(mounted) => Some(mounted),
        Err(error) => {
            tracing::warn!(%error, "the drive is not mounted");
            None
        }
    }
}

/// `BlobTransfer` on the node's peer lane, behind its gate.
pub(super) fn more(engine: &Engine) -> More {
    More {
        router: Some(
            LayerRouter::new()
                .with(
                    blob_transfer_service_descriptor(),
                    BlobTransferDispatcher::new(engine.blob_host()),
                )
                .with(
                    possession_service_descriptor(),
                    PossessionDispatcher::new(PossessionHost(engine.clone())),
                ),
        ),
        permits: vec![
            (blob_transfer_service_descriptor(), &BLOB_PERMITS),
            (possession_service_descriptor(), &POSSESSION_PERMITS),
        ],
    }
}

/// The bucket of `config`, when it names one. The secret comes from
/// `blob_s3_secret_file`, else `ARCHITECT_BLOB_S3_SECRET_ACCESS_KEY`;
/// it is never logged.
pub(super) async fn backend(config: &Config) -> Result<Option<S3Store>, String> {
    let Some(bucket) = &config.blob_s3_bucket else {
        return Ok(None);
    };
    let secret = config
        .blob_s3_secret_file
        .as_deref()
        .and_then(read_secret)
        .or_else(|| {
            std::env::var("ARCHITECT_BLOB_S3_SECRET_ACCESS_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or("blob_s3_bucket is set but no secret: set blob_s3_secret_file")?;
    let s3 = S3Config {
        endpoint: config.blob_s3_endpoint.clone(),
        region: config.blob_s3_region.clone(),
        bucket: bucket.clone(),
        prefix: config.blob_s3_prefix.clone(),
        access_key_id: config.blob_s3_access_key_id.clone().unwrap_or_default(),
        secret_access_key: secret,
        virtual_hosted: false,
    };
    let store = S3Store::open(&s3)
        .await
        .map_err(|e| format!("the blob bucket {bucket}: {e}"))?;
    tracing::info!(bucket = %bucket, holder = store.holder_id(), "files: bytes in S3 too");
    Ok(Some(store))
}

/// Copy every hash a manifest names into the bucket, from this node;
/// with [`Placement::Primary`], then keep the local store to its budget.
pub(super) async fn replicate_forever(
    engine: Engine,
    s3: S3Store,
    placement: Placement,
    cache_bytes: u64,
    every: Duration,
) {
    let mut there: Option<std::collections::HashSet<Sha256>> = None;
    loop {
        if there.is_none() {
            match s3.list().await {
                Ok(list) => there = Some(list.into_iter().map(|(h, _)| h).collect()),
                Err(e) => tracing::warn!(%e, "files: the blob bucket does not list"),
            }
        }
        if let Some(there) = there.as_mut() {
            for hash in engine.named() {
                if there.contains(&hash) {
                    continue;
                }
                let Some(file) = engine.store.file_for(&hash) else {
                    continue;
                };
                let Ok(body) = tokio::fs::File::open(&file).await else {
                    continue;
                };
                match s3.put_reader(Some(hash), Box::pin(body)).await {
                    Ok(_) => {
                        there.insert(hash);
                    }
                    // A working file changed under us: the next scan has it.
                    Err(e) => tracing::debug!(%hash, %e, "files: not replicated yet"),
                }
            }
            if placement == Placement::Primary {
                evict(engine.store.local.clone(), cache_bytes, there).await;
            }
        }
        tokio::time::sleep(every).await;
    }
}

/// Remove the oldest blobs from `local` that the bucket holds, until it
/// is within `budget` bytes. Pinned blobs and working copies stay.
async fn evict(local: LocalFs, budget: u64, there: &std::collections::HashSet<Sha256>) {
    let (Ok(all), Ok(pinned)) = (local.list().await, local.pinned().await) else {
        return;
    };
    let mut held = Vec::new();
    let mut total = 0u64;
    for hash in all {
        let path = local.path_for(&hash);
        let Ok(meta) = tokio::fs::metadata(&path).await else {
            continue;
        };
        total = total.saturating_add(meta.len());
        let at = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if there.contains(&hash) && !pinned.contains(&hash) {
            held.push((at, meta.len(), path));
        }
    }
    held.sort();
    for (_, len, path) in held {
        if total <= budget {
            break;
        }
        if tokio::fs::remove_file(&path).await.is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Keep a `BlobTransfer` connection to every peer the node can dial, the
/// engine's fetcher reading from them in turn, then from `fixed` (the
/// node's bucket), and a `Possession` one, so the engine can challenge
/// what each peer says it holds (and knows which holders are reachable now).
pub(super) async fn dial_forever(
    engine: Engine,
    endpoint: iroh::Endpoint,
    configured: Vec<Dial>,
    admission: Admission,
    mode: IrohMode,
    fixed: Vec<Arc<dyn Remote>>,
) {
    engine.fetcher.set_remotes(fixed.clone());
    let me = architect_devices::endpoint_key(&endpoint.id().to_string());
    let mut peers: BTreeMap<String, (BlobTransferClient, PossessionClient)> = BTreeMap::new();
    loop {
        let mut changed = false;
        let wanted = targets(&configured, &admission, mode);
        let wanted_ids: std::collections::BTreeSet<&str> =
            wanted.iter().map(|p| p.endpoint.as_str()).collect();
        let before = peers.len();
        peers.retain(|id, _| wanted_ids.contains(id.as_str()));
        changed |= peers.len() != before;
        for peer in &wanted {
            if peer.endpoint == me {
                continue;
            }
            if let Some((client, _)) = peers.get(&peer.endpoint)
                && client.size("0".repeat(64)).await.is_ok()
            {
                continue;
            }
            changed |= peers.remove(&peer.endpoint).is_some();
            let Some(blob) = establish::<BlobTransferClient>(&endpoint, peer).await else {
                continue;
            };
            let Some(proofs) = establish::<PossessionClient>(&endpoint, peer).await else {
                continue;
            };
            peers.insert(peer.endpoint.clone(), (blob, proofs));
            changed = true;
        }
        if changed {
            let remotes: Vec<Arc<dyn Remote>> = peers
                .values()
                .map(|(c, _)| -> Arc<dyn Remote> { Arc::new(VoxRemote::new(c.clone())) })
                .chain(fixed.iter().cloned())
                .collect();
            tracing::debug!(peers = remotes.len(), "files: blob peers");
            engine.set_online(!remotes.is_empty());
            engine.fetcher.set_remotes(remotes);
            engine.set_provers(
                peers
                    .iter()
                    .map(|(id, (_, p))| -> (String, Arc<dyn Prover>) {
                        (id.clone(), Arc::new(VoxProver(p.clone())))
                    })
                    .collect(),
            );
        }
        tokio::time::sleep(DIAL_EVERY).await;
    }
}

async fn establish<C: vox::FromVoxLane>(endpoint: &iroh::Endpoint, peer: &Dial) -> Option<C> {
    let link = iroh_link::connect(endpoint, peer.addr.clone()).await.ok()?;
    vox::initiator_on(link).establish::<C>().await.ok()
}
