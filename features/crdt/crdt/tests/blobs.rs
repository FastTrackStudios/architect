//! `crdt::blobs` + `architect::blob`'s transfer by hash
//! (`documents.md`, "The file": large media are blobs).
//!
//! A song recorded on one device opens on another; its audio is fetched
//! by hash when it is played — streamed as it arrives, resumed after the
//! link drops — over vox in process, over iroh, and from a server's HTTP
//! route. Pinning keeps a blob through collection; a blob nothing points
//! at is collected.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::literal_string_with_formatting_args,
    clippy::panic
)]
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashSet;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use architect::blob::{
    Blob, BlobError, BlobStore, BlobTransferClient, BlobTransferDispatcher, BlobTransferHost,
    Fetcher, HttpRemote, LocalFs, Progress, Reader, Remote, Sha256, VoxRemote,
    blob_transfer_service_descriptor, serve_http,
};
use architect::layer::{LayerRouter, handler_acceptor};
use crdt::blobs;
use crdt::{DocKind, DocKinds};
use futures::future::BoxFuture;
use loro::LoroDoc;
use tokio::io::{AsyncRead, AsyncReadExt as _, ReadBuf};

const SIZE: usize = 1_000_000;
/// Where the gated host stops sending until the link is dropped.
const STALL: u64 = 300_000;
const WAIT: Duration = Duration::from_secs(10);

fn audio(seed: u8) -> Vec<u8> {
    (0..SIZE).map(|i| (i * 31 % 251) as u8 ^ seed).collect()
}

/// A song points at its audio by hash, in its `audio` list.
fn song_audio(doc: &LoroDoc) -> Vec<String> {
    let list = doc.get_list("audio");
    (0..list.len())
        .filter_map(|i| {
            let v = list.get(i)?.into_value().ok()?;
            v.into_string().ok().map(|s| s.to_string())
        })
        .collect()
}

fn song_kind() -> DocKind {
    DocKind::new("test.song", "Song")
        .extension("song")
        .blobs(song_audio)
}

fn kinds() -> DocKinds {
    let mut kinds = DocKinds::new();
    kinds.register(song_kind());
    kinds
}

/// A song on the recording device that points at `takes`.
fn song(takes: &[Sha256]) -> LoroDoc {
    let doc = LoroDoc::new();
    song_kind().stamp(&doc);
    let list = doc.get_list("audio");
    for take in takes {
        list.push(take.to_hex().as_str()).unwrap();
    }
    doc.commit();
    doc
}

/// The song as it opens on another device.
fn open_elsewhere(doc: &LoroDoc) -> LoroDoc {
    let there = LoroDoc::new();
    there
        .import(&doc.export(loro::ExportMode::Snapshot).unwrap())
        .unwrap();
    there
}

/// A reader that never yields: the host stalled mid-send.
struct Stalled;

impl AsyncRead for Stalled {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

/// The recording device's store, recording where each read starts and
/// (optionally) stalling once `stall` bytes are out.
struct Recorded {
    inner: LocalFs,
    stall: Option<u64>,
    offsets: Arc<Mutex<Vec<u64>>>,
}

impl BlobStore for Recorded {
    fn put(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> BoxFuture<'_, Result<Sha256, BlobError>> {
        self.inner.put(expected, body)
    }

    /// What `size` asks; only reads (`get_from`) are recorded.
    fn get(&self, hash: Sha256) -> BoxFuture<'_, Result<Blob, BlobError>> {
        self.inner.get(hash)
    }

    fn has(&self, hash: Sha256) -> BoxFuture<'_, Result<bool, BlobError>> {
        self.inner.has(hash)
    }

    fn get_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, Result<Blob, BlobError>> {
        Box::pin(async move {
            self.offsets.lock().unwrap().push(offset);
            let blob = self.inner.get_from(hash, offset).await?;
            let reader: Reader = match self.stall {
                Some(stall) => Box::pin(
                    blob.reader
                        .take(stall.saturating_sub(offset))
                        .chain(Stalled),
                ),
                None => blob.reader,
            };
            Ok(Blob {
                len: blob.len,
                reader,
            })
        })
    }
}

/// The client's half of an in-process link that a test can cut, as a
/// network going away does: receiving ends, sending fails.
struct CutTx {
    inner: vox::MemoryLinkTx,
    cut: tokio::sync::watch::Receiver<bool>,
}

struct CutRx {
    inner: vox::MemoryLinkRx,
    cut: tokio::sync::watch::Receiver<bool>,
}

impl vox_types::LinkTx for CutTx {
    async fn send(&self, bytes: Vec<u8>) -> io::Result<()> {
        if *self.cut.borrow() {
            return Err(io::ErrorKind::ConnectionReset.into());
        }
        self.inner.send(bytes).await
    }

    async fn close(self) -> io::Result<()> {
        self.inner.close().await
    }
}

impl vox_types::LinkRx for CutRx {
    type Error = vox::MemoryLinkRxError;

    async fn recv(&mut self) -> Result<Option<vox_types::Backing>, Self::Error> {
        let mut cut = self.cut.clone();
        let frame = tokio::select! {
            frame = self.inner.recv() => frame,
            _ = cut.wait_for(|c| *c) => return Ok(None),
        };
        if *self.cut.borrow() { Ok(None) } else { frame }
    }
}

/// A vox lane to `host` over an in-process link; the returned sender
/// cuts it.
async fn lane(host: BlobTransferHost) -> (BlobTransferClient, tokio::sync::watch::Sender<bool>) {
    let router = LayerRouter::new().with(
        blob_transfer_service_descriptor(),
        BlobTransferDispatcher::new(host),
    );
    let (client_link, server_link) = vox::memory_link_pair(16);
    let server = tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(router))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    let (cut_tx, cut) = tokio::sync::watch::channel(false);
    let (tx, rx) = vox_types::Link::split(client_link);
    let client_link = vox_types::SplitLink {
        tx: CutTx {
            inner: tx,
            cut: cut.clone(),
        },
        rx: CutRx { inner: rx, cut },
    };
    let client = vox::initiator_on(client_link)
        .establish::<BlobTransferClient>()
        .await
        .expect("establish blob lane");
    // The server outlives the test's interest in it; cutting the link is
    // what ends a lane here.
    drop(server);
    (client, cut_tx)
}

fn remote(client: BlobTransferClient) -> Arc<dyn Remote> {
    Arc::new(VoxRemote::new(client))
}

async fn read_all(mut blob: Blob) -> Vec<u8> {
    let mut out = Vec::new();
    tokio::time::timeout(WAIT, blob.reader.read_to_end(&mut out))
        .await
        .expect("read in time")
        .expect("read");
    out
}

/// Wait until the transfer of `hash` has `at_least` bytes in.
async fn wait_for(fetcher: &Fetcher, hash: Sha256, at_least: u64) {
    let mut rx = fetcher.progress(hash).expect("a transfer is running");
    tokio::time::timeout(
        WAIT,
        rx.wait_for(|p| matches!(p, Progress::Fetching { have, .. } if *have >= at_least)),
    )
    .await
    .expect("progress in time")
    .expect("transfer running");
}

#[tokio::test]
async fn a_songs_audio_arrives_on_demand_streamed_and_resumes_after_a_dropped_link() {
    let dir = tempfile::tempdir().unwrap();
    // The recording device: the take in its store, the song pointing at it.
    let studio = LocalFs::new(dir.path().join("studio"));
    let take = audio(0);
    let hash = studio.put_bytes(&take).await.unwrap();
    let recorded = song(&[hash]);

    // Another device opens the song: it points at audio this device lacks.
    let laptop = Fetcher::new(LocalFs::new(dir.path().join("laptop")));
    let opened = open_elsewhere(&recorded);
    assert_eq!(blobs::referenced(&song_kind(), &opened), vec![hash]);
    assert_eq!(
        blobs::missing(&song_kind(), &opened, laptop.store())
            .await
            .unwrap(),
        vec![hash]
    );

    // The studio serves its store; the link stalls partway through.
    let offsets = Arc::new(Mutex::new(Vec::new()));
    let stalling = BlobTransferHost::new(Arc::new(Recorded {
        inner: studio.clone(),
        stall: Some(STALL),
        offsets: offsets.clone(),
    }));
    let (client, link) = lane(stalling).await;
    laptop.set_remotes(vec![remote(client)]);

    // Played: the reader yields the first bytes before the rest exists.
    let mut playing = laptop.open(hash).await.unwrap();
    assert_eq!(playing.len, SIZE as u64);
    let mut head = vec![0u8; STALL as usize];
    tokio::time::timeout(WAIT, playing.reader.read_exact(&mut head))
        .await
        .expect("the head streams before the end arrives")
        .unwrap();
    assert_eq!(head, take[..STALL as usize]);
    assert!(!laptop.store().has(hash).await.unwrap(), "not whole yet");
    wait_for(&laptop, hash, STALL).await;

    // The link drops mid-transfer: the player's read fails, the fetch
    // fails as unavailable, and what arrived is kept.
    link.send_replace(true);
    let mut rest = Vec::new();
    let broken = tokio::time::timeout(WAIT, playing.reader.read_to_end(&mut rest))
        .await
        .expect("the player hears about the drop");
    assert!(broken.is_err(), "{broken:?}");
    let err = tokio::time::timeout(WAIT, laptop.fetch(hash))
        .await
        .expect("fetch settles")
        .unwrap_err();
    assert!(
        matches!(err, BlobError::Unavailable { hash: h, .. } if h == hash),
        "{err:?}"
    );
    let part = architect::blob::part_path(&laptop.store().partial_path(&hash));
    assert_eq!(std::fs::metadata(&part).unwrap().len(), STALL);

    // Reconnected: the fetch asks only for the rest, and lands whole.
    let healthy = BlobTransferHost::new(Arc::new(Recorded {
        inner: studio.clone(),
        stall: None,
        offsets: offsets.clone(),
    }));
    let (client, _link) = lane(healthy).await;
    laptop.set_remotes(vec![remote(client)]);
    let played = read_all(laptop.open(hash).await.unwrap()).await;
    assert_eq!(played, take);
    tokio::time::timeout(WAIT, laptop.fetch(hash))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*offsets.lock().unwrap(), vec![0, STALL]);
    assert!(laptop.store().has(hash).await.unwrap());
    assert!(!part.exists());
    assert!(
        blobs::missing(&song_kind(), &opened, laptop.store())
            .await
            .unwrap()
            .is_empty()
    );

    // Seeking reads from the store now it is here.
    let tail = read_all(laptop.open_at(hash, 999_000).await.unwrap()).await;
    assert_eq!(tail, take[999_000..]);
}

#[tokio::test]
async fn a_blob_no_remote_has_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let empty = LocalFs::new(dir.path().join("empty"));
    let (client, _link) = lane(BlobTransferHost::new(Arc::new(empty))).await;
    let fetcher = Fetcher::new(LocalFs::new(dir.path().join("here"))).with_remote(remote(client));
    let hash = Sha256::of(b"never recorded");
    assert!(matches!(
        fetcher.fetch(hash).await.unwrap_err(),
        BlobError::NotFound(h) if h == hash
    ));
    assert!(matches!(
        fetcher.open(hash).await.unwrap_err(),
        BlobError::NotFound(_)
    ));
}

#[tokio::test]
async fn a_host_refuses_blobs_it_does_not_allow() {
    let dir = tempfile::tempdir().unwrap();
    let studio = LocalFs::new(dir.path().join("studio"));
    let private = studio.put_bytes(b"private take").await.unwrap();
    let shared = studio.put_bytes(b"shared take").await.unwrap();
    let host = BlobTransferHost::new(Arc::new(studio)).allow(move |hash| hash == shared);
    let (client, _link) = lane(host).await;
    let fetcher = Fetcher::new(LocalFs::new(dir.path().join("here"))).with_remote(remote(client));
    fetcher.fetch(shared).await.unwrap();
    assert!(matches!(
        fetcher.fetch(private).await.unwrap_err(),
        BlobError::Unavailable { .. }
    ));
}

#[tokio::test]
async fn blobs_travel_over_iroh() {
    let bind = || async {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(vec![architect::iroh_link::VOX_ALPN.to_vec()])
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .bind()
            .await
            .unwrap()
    };
    let (studio_ep, laptop_ep) = (bind().await, bind().await);
    let addr = iroh::EndpointAddr::from_parts(
        studio_ep.id(),
        studio_ep
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );

    let dir = tempfile::tempdir().unwrap();
    let studio = LocalFs::new(dir.path().join("studio"));
    let take = audio(7);
    let hash = studio.put_bytes(&take).await.unwrap();
    let router = LayerRouter::new().with(
        blob_transfer_service_descriptor(),
        BlobTransferDispatcher::new(BlobTransferHost::new(Arc::new(studio))),
    );
    let serving = tokio::spawn({
        let ep = studio_ep.clone();
        async move { architect::iroh_link::serve_router(&ep, router).await }
    });

    let link = architect::iroh_link::connect(&laptop_ep, addr)
        .await
        .unwrap();
    let client = vox::initiator_on(link)
        .establish::<BlobTransferClient>()
        .await
        .unwrap();
    let laptop = Fetcher::new(LocalFs::new(dir.path().join("laptop"))).with_remote(remote(client));
    let played = read_all(laptop.open(hash).await.unwrap()).await;
    assert_eq!(played, take);
    tokio::time::timeout(WAIT, laptop.fetch(hash))
        .await
        .unwrap()
        .unwrap();
    assert!(laptop.store().has(hash).await.unwrap());

    studio_ep.close().await;
    laptop_ep.close().await;
    serving.abort();
}

#[tokio::test]
async fn a_servers_copy_over_http_resumes_with_a_range() {
    let dir = tempfile::tempdir().unwrap();
    let server_store = LocalFs::new(dir.path().join("server"));
    let take = audio(3);
    let hash = server_store.put_bytes(&take).await.unwrap();
    let ranges = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = axum::Router::new().route(
        "/blobs/{hash}",
        axum::routing::get({
            let (store, ranges) = (server_store.clone(), ranges.clone());
            move |axum::extract::Path(hash): axum::extract::Path<String>,
                  headers: axum::http::HeaderMap| async move {
                if let Some(range) = headers.get(axum::http::header::RANGE) {
                    ranges
                        .lock()
                        .unwrap()
                        .push(range.to_str().unwrap().to_owned());
                }
                serve_http(&store, &headers, &hash).await
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });

    // A peer that has nothing comes first; the server's copy is the fallback.
    let (empty, _link) = lane(BlobTransferHost::new(Arc::new(LocalFs::new(
        dir.path().join("empty"),
    ))))
    .await;
    let here = LocalFs::new(dir.path().join("here"));
    let fetcher = Fetcher::new(here.clone())
        .with_remote(remote(empty))
        .with_remote(Arc::new(HttpRemote::new(format!("http://{addr}/blobs/"))));

    // An earlier, interrupted fetch left the first 400 KB.
    let part = architect::blob::part_path(&here.partial_path(&hash));
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, &take[..400_000]).unwrap();

    fetcher.fetch(hash).await.unwrap();
    assert_eq!(
        read_all(here.get(hash).await.unwrap()).await,
        take,
        "resumed and verified"
    );
    assert_eq!(
        *ranges.lock().unwrap(),
        vec!["bytes=0-0".to_owned(), "bytes=400000-".to_owned()]
    );

    let remote = HttpRemote::new(format!("http://{addr}/blobs"));
    assert_eq!(
        remote.size(Sha256::of(b"absent")).await.unwrap(),
        None,
        "404 is not having it"
    );
}

#[tokio::test]
async fn a_pinned_blob_survives_collection_and_an_unreferenced_one_is_collected() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalFs::new(dir.path().join("blobs"));
    let used = store.put_bytes(b"the take the song uses").await.unwrap();
    let pinned = store.put_bytes(b"a take kept offline").await.unwrap();
    let dropped = store.put_bytes(b"a take nobody uses").await.unwrap();
    let abandoned = Sha256::of(b"a fetch nobody finished");
    let stale = architect::blob::part_path(&store.partial_path(&abandoned));
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"half").unwrap();
    store.pin(pinned).await.unwrap();
    assert!(store.is_pinned(pinned).await.unwrap());

    let doc = song(&[used]);
    let live = blobs::live(&kinds(), [&doc]);
    assert_eq!(live, HashSet::from([used]));

    let report = store.collect(&live, Duration::ZERO).await.unwrap();
    assert_eq!(report.removed, vec![dropped]);
    assert_eq!(report.pinned, 1);
    assert_eq!(report.freed, 18 + 4);
    assert!(store.has(used).await.unwrap());
    assert!(store.has(pinned).await.unwrap());
    assert!(!store.has(dropped).await.unwrap());
    assert!(!stale.exists());

    // A grace period spares what just arrived.
    store.unpin(pinned).await.unwrap();
    let young = store
        .collect(&live, Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(young.removed.is_empty());
    let report = store.collect(&live, Duration::ZERO).await.unwrap();
    assert_eq!(report.removed, vec![pinned]);

    // A song nothing opens any more points at nothing.
    let report = store
        .collect(&blobs::live(&kinds(), []), Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(report.removed, vec![used]);
    assert!(store.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn keep_offline_fetches_and_pins_a_songs_audio() {
    let dir = tempfile::tempdir().unwrap();
    let studio = LocalFs::new(dir.path().join("studio"));
    let takes = [
        studio.put_bytes(&audio(1)).await.unwrap(),
        studio.put_bytes(&audio(2)).await.unwrap(),
    ];
    let opened = open_elsewhere(&song(&takes));
    let (client, link) = lane(BlobTransferHost::new(Arc::new(studio))).await;
    let laptop = Fetcher::new(LocalFs::new(dir.path().join("laptop"))).with_remote(remote(client));

    let kept = blobs::keep_offline(&song_kind(), &opened, &laptop)
        .await
        .unwrap();
    assert_eq!(kept, takes);

    // Offline: the studio is gone, the song is closed, collection runs.
    link.send_replace(true);
    laptop.set_remotes(Vec::new());
    let report = laptop
        .store()
        .collect(&HashSet::new(), Duration::ZERO)
        .await
        .unwrap();
    assert!(report.removed.is_empty());
    assert_eq!(report.pinned, 2);
    assert_eq!(
        read_all(laptop.open(takes[1]).await.unwrap()).await,
        audio(2)
    );

    // Released, nothing keeps them.
    blobs::release(&song_kind(), &opened, laptop.store())
        .await
        .unwrap();
    let report = laptop
        .store()
        .collect(&HashSet::new(), Duration::ZERO)
        .await
        .unwrap();
    let mut expected = takes.to_vec();
    expected.sort();
    assert_eq!(report.removed, expected);
}
