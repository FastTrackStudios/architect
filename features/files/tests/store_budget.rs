//! The node's blob store stays bounded (#525): a node holding its tree in
//! place keeps no second copy of it, and its store never grows past its
//! budget or into the filesystem's keep-free floor (feature `sync`).
//!
//! Sizes here are kilobytes: a test that fills a disk is the bug.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{BlobError, Fetcher, LocalFs, Space, SpaceProbe, StoreBudget};
use architect_files::sync::{Engine, EngineConfig, TreeStore};
use loro::{ExportMode, LoroDoc};
use uuid::Uuid;

type DocMap = Arc<Mutex<HashMap<Uuid, LoroDoc>>>;

struct Device {
    engine: Engine,
    docs: DocMap,
}

fn device(name: &str, tmp: &Path, files_dir: PathBuf, hold_all: bool, local: LocalFs) -> Device {
    device_on(name, tmp, files_dir, hold_all, local, Arc::default())
}

/// [`device`] over documents it already had (a restart).
fn device_on(
    name: &str,
    tmp: &Path,
    files_dir: PathBuf,
    hold_all: bool,
    local: LocalFs,
    docs: DocMap,
) -> Device {
    let store = TreeStore::new(local.clone());
    let fetcher = Fetcher::new(local);
    let source = docs.clone();
    let engine = Engine::new(
        EngineConfig {
            settle: Duration::ZERO,
            ..EngineConfig::new(name, tmp.join(name).join("data"), files_dir, hold_all)
        },
        store,
        fetcher,
        Arc::new(move |id| {
            let doc = source.lock().unwrap().entry(id).or_default().clone();
            Box::pin(async move { Some(doc) })
        }),
    );
    Device { engine, docs }
}

fn exchange(a: &Device, b: &Device) {
    let mut ids: Vec<Uuid> = a.docs.lock().unwrap().keys().copied().collect();
    ids.extend(b.docs.lock().unwrap().keys().copied());
    for id in ids {
        let da = a.docs.lock().unwrap().entry(id).or_default().clone();
        let db = b.docs.lock().unwrap().entry(id).or_default().clone();
        db.import(&da.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        da.import(&db.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        a.engine.learn(id);
        b.engine.learn(id);
    }
}

/// Every file under `dir` and its bytes.
fn du(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => du(&e.path()),
                Ok(_) => e.metadata().map_or(0, |m| m.len()),
                Err(_) => 0,
            })
            .sum()
    })
}

/// A project of `files` files of `size` bytes each, distinct content.
fn project(files_dir: &Path, name: &str, files: usize, size: usize) -> PathBuf {
    let p = files_dir.join(format!("acme/Projects/{name}"));
    std::fs::create_dir_all(p.join("Audio")).unwrap();
    for i in 0..files {
        let mut bytes = vec![u8::try_from(i % 251).unwrap(); size];
        bytes[..8].copy_from_slice(&u64::try_from(i).unwrap().to_le_bytes());
        std::fs::write(p.join(format!("Audio/take_{i}.wav")), bytes).unwrap();
    }
    p
}

async fn passes(d: &Device, n: usize) {
    for _ in 0..n {
        d.engine.pass_all().await;
    }
}

/// The cause, part one: a node that holds its tree in place copied every
/// file of it (up to `KEEP_INLINE`) into its store when it scanned it.
#[tokio::test(flavor = "multi_thread")]
async fn a_holding_node_indexes_its_tree_in_place_and_stores_none_of_it() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    project(&tree, "Album", 20, 20_000);
    let blobs = tmp.path().join("node/blobs");
    let node = device("node", tmp.path(), tree.clone(), true, LocalFs::new(&blobs));

    passes(&node, 3).await;

    let roots = node.engine.status().await;
    assert_eq!(roots.len(), 1, "the project is found");
    assert!(du(&tree) >= 400_000);
    assert_eq!(
        du(&blobs),
        0,
        "the tree's bytes are not copied into the store"
    );
}

/// The cause, part two: bytes fetched from a peer were written into the
/// tree and also left in the store.
#[tokio::test(flavor = "multi_thread")]
async fn bytes_a_holding_node_fetches_end_up_in_its_tree_only() {
    let tmp = tempfile::tempdir().unwrap();
    let laptop_files = tmp.path().join("laptop-files");
    project(&laptop_files, "Album", 20, 20_000);
    let laptop = device(
        "laptop",
        tmp.path(),
        laptop_files.clone(),
        true,
        LocalFs::new(tmp.path().join("laptop/blobs")),
    );
    let tree = tmp.path().join("nfs");
    std::fs::create_dir_all(&tree).unwrap();
    let blobs = tmp.path().join("node/blobs");
    let node = device("node", tmp.path(), tree.clone(), true, LocalFs::new(&blobs));
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));

    passes(&laptop, 2).await;
    exchange(&laptop, &node);
    let id = Uuid::parse_str(&laptop.engine.status().await[0].doc_id).unwrap();
    node.engine
        .pin(id, Some(tree.join("acme/Projects/Album")))
        .await
        .unwrap();
    passes(&node, 3).await;

    let got = tree.join("acme/Projects/Album/Audio/take_7.wav");
    assert_eq!(std::fs::metadata(&got).map(|m| m.len()).ok(), Some(20_000));
    assert_eq!(du(&blobs), 0, "fetched bytes are moved into the tree");
}

/// A filesystem that says it has `free` bytes free, whatever the disk has.
fn probe(free: &Arc<AtomicU64>) -> SpaceProbe {
    let free = free.clone();
    Arc::new(move |_| {
        Some(Space {
            total: 1 << 40,
            free: free.load(Ordering::SeqCst),
        })
    })
}

fn budgeted(blobs: &Path, max: u64, keep_free: u64, free: &Arc<AtomicU64>) -> LocalFs {
    let budget = StoreBudget::new(blobs, max, keep_free).with_probe(probe(free));
    LocalFs::new(blobs).with_budget(Arc::new(budget))
}

/// The largest the store at `blobs` was while it ran, sampled.
struct Sampler {
    stop: Arc<AtomicBool>,
    max: Arc<AtomicU64>,
    thread: std::thread::JoinHandle<()>,
}

impl Sampler {
    fn start(blobs: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let max = Arc::new(AtomicU64::new(0));
        let (s, m) = (stop.clone(), max.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                m.fetch_max(du(&blobs), Ordering::SeqCst);
                std::thread::sleep(Duration::from_micros(200));
            }
        });
        Self { stop, max, thread }
    }

    fn finish(self) -> u64 {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().unwrap();
        self.max.load(Ordering::SeqCst)
    }
}

/// The regression (#525): a holding node syncs a project several times
/// larger than its store's budget. The store never passes the budget
/// (sampled throughout), fetches wait while the filesystem is at its
/// floor and the node says so, and the tree ends up complete.
#[tokio::test(flavor = "multi_thread")]
async fn a_holding_node_syncs_far_more_than_its_budget_and_its_store_stays_under_it() {
    const BUDGET: u64 = 50_000;
    let tmp = tempfile::tempdir().unwrap();
    let laptop_files = tmp.path().join("laptop-files");
    project(&laptop_files, "Album", 20, 20_000); // 400 kB: eight budgets
    let laptop = device(
        "laptop",
        tmp.path(),
        laptop_files,
        true,
        LocalFs::new(tmp.path().join("laptop/blobs")),
    );
    let tree = tmp.path().join("nfs");
    std::fs::create_dir_all(&tree).unwrap();
    let blobs = tmp.path().join("node/blobs");
    // The filesystem starts at its floor: nothing may be fetched.
    let free = Arc::new(AtomicU64::new(1_000_000));
    let node = device(
        "node",
        tmp.path(),
        tree.clone(),
        true,
        budgeted(&blobs, BUDGET, 1_000_000, &free),
    );
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));
    passes(&laptop, 2).await;
    exchange(&laptop, &node);
    let id = Uuid::parse_str(&laptop.engine.status().await[0].doc_id).unwrap();
    let album = tree.join("acme/Projects/Album");
    node.engine.pin(id, Some(album.clone())).await.unwrap();

    let sampler = Sampler::start(blobs.clone());
    passes(&node, 2).await;
    let waiting = node.engine.status().await;
    assert_eq!(waiting[0].deferred, 20, "every fetch waits for room");
    assert_eq!(node.engine.deferred(), 20);
    assert!(du(&album) < 20_000, "nothing was fetched past the floor");
    let usage = node.engine.store_usage().unwrap();
    assert_eq!(
        (usage.max, usage.space.map(|s| s.free)),
        (BUDGET, Some(1_000_000))
    );

    // Space appears: the fetches go ahead, one budget's worth at a time.
    free.store(10_000_000, Ordering::SeqCst);
    passes(&node, 3).await;
    let largest = sampler.finish();

    assert!(largest <= BUDGET, "the store reached {largest} of {BUDGET}");
    assert_eq!(du(&album), 400_000, "the tree is complete");
    let done = node.engine.status().await;
    assert_eq!((done[0].missing, done[0].deferred), (0, 0));
    assert_eq!(du(&blobs), 0, "and nothing is stored twice");
}

/// The keep-free floor: with a filesystem that says it is nearly full, a
/// put and a fetch are refused before anything is written.
#[tokio::test(flavor = "multi_thread")]
async fn writes_that_would_cross_the_keep_free_floor_are_refused_before_a_byte_lands() {
    let tmp = tempfile::tempdir().unwrap();
    let free = Arc::new(AtomicU64::new(100_000));
    let blobs = tmp.path().join("blobs");
    let store = budgeted(&blobs, u64::MAX, 90_000, &free);

    assert!(
        store.put_bytes(&vec![1u8; 5_000]).await.is_ok(),
        "95 kB stays free"
    );
    let refused = store.put_bytes(&vec![2u8; 20_000]).await;
    assert!(matches!(refused, Err(BlobError::NoRoom(_))), "{refused:?}");

    let remote = LocalFs::new(tmp.path().join("remote"));
    let hash = remote.put_bytes(&vec![3u8; 20_000]).await.unwrap();
    let fetcher = Fetcher::new(store.clone()).with_remote(Arc::new(remote));
    let deferred = fetcher.fetch(hash).await;
    assert!(
        matches!(deferred, Err(BlobError::NoRoom(_))),
        "{deferred:?}"
    );
    assert_eq!(du(&blobs), 5_000, "only the first put is in the store");

    // Room again: the same fetch goes through.
    free.store(1_000_000, Ordering::SeqCst);
    fetcher.fetch(hash).await.unwrap();
    assert_eq!(du(&blobs), 25_000);
    assert!(store.budget().unwrap().usage().refused >= 2);
}

/// N fetches at once against one budget: together they never pass it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_fetches_cannot_jointly_exceed_the_budget() {
    const BLOB: usize = 10_000;
    const BUDGET: u64 = 35_000; // room for three
    let tmp = tempfile::tempdir().unwrap();
    let remote = LocalFs::new(tmp.path().join("remote"));
    let mut hashes = Vec::new();
    for i in 0..12u8 {
        hashes.push(remote.put_bytes(&vec![i; BLOB]).await.unwrap());
    }
    let free = Arc::new(AtomicU64::new(1 << 30));
    let blobs = tmp.path().join("blobs");
    let fetcher = Fetcher::new(budgeted(&blobs, BUDGET, 0, &free)).with_remote(Arc::new(remote));

    let sampler = Sampler::start(blobs.clone());
    let tasks: Vec<_> = hashes
        .iter()
        .map(|h| {
            let (f, h) = (fetcher.clone(), *h);
            tokio::spawn(async move { f.fetch(h).await })
        })
        .collect();
    let mut done = 0;
    for t in tasks {
        match t.await.unwrap() {
            Ok(()) => done += 1,
            Err(BlobError::NoRoom(_)) => {}
            Err(e) => panic!("{e}"),
        }
    }
    let largest = sampler.finish();
    assert_eq!(done, 3, "three fit");
    assert!(largest <= BUDGET, "the store reached {largest} of {BUDGET}");
    let usage = fetcher.store().budget().unwrap().usage();
    assert_eq!((usage.used, usage.reserved), (30_000, 0));
}

/// Over its soft threshold the store drops what is safe: bytes a working
/// file also holds go, a blob no one else has a verified copy of stays.
#[tokio::test(flavor = "multi_thread")]
async fn eviction_drops_duplicates_and_never_a_last_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let held = tmp.path().join("nfs");
    project(&held, "Album", 4, 10_000);
    let node = device(
        "node",
        tmp.path(),
        held,
        true,
        LocalFs::new(tmp.path().join("node/blobs")),
    );
    passes(&node, 2).await;
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();

    let free = Arc::new(AtomicU64::new(1 << 30));
    let files = tmp.path().join("laptop-files");
    let blobs = tmp.path().join("laptop/blobs");
    // Not holding: small files it fetches are kept in the store too (its
    // history), until the store is over its soft threshold.
    let laptop = device(
        "laptop",
        tmp.path(),
        files.clone(),
        false,
        budgeted(&blobs, 1_000_000, 0, &free),
    );
    laptop
        .engine
        .fetcher
        .add_remote(Arc::new(node.engine.store.clone()));
    // Its only copy of an old version: nobody else holds these bytes.
    let last = laptop
        .engine
        .store
        .local
        .put_bytes(&vec![9u8; 30_000])
        .await
        .unwrap();
    exchange(&node, &laptop);
    let copy = files.join("Album");
    laptop.engine.pin(id, Some(copy.clone())).await.unwrap();
    passes(&laptop, 2).await;
    assert_eq!(du(&copy), 40_000);
    assert_eq!(
        du(&blobs),
        70_000,
        "four files kept as history, and the old version"
    );

    // A smaller budget: 70 kB is over 90% of 72 kB.
    let tight = budgeted(&blobs, 72_000, 0, &free);
    let laptop = device_on("laptop", tmp.path(), files, false, tight, laptop.docs);
    laptop.engine.learn(id);
    laptop.engine.pass_all().await;

    assert!(
        laptop.engine.store.local.path_for(&last).is_file(),
        "the last copy stays"
    );
    assert_eq!(du(&blobs), 30_000, "the working files' duplicates went");
}
