//! No byte is lost to keep the store bounded (#527's review). Each test
//! here failed on #527's first version; most began as the reviewer's
//! probes. Sizes are kilobytes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{BlobStore as _, Fetcher, LocalFs, Sha256, Space, StoreBudget};
use architect_files::sync::{Engine, EngineConfig, TreeStore, layout};
use loro::{ExportMode, LoroDoc};
use uuid::Uuid;

type DocMap = Arc<Mutex<HashMap<Uuid, LoroDoc>>>;

struct Device {
    engine: Engine,
    docs: DocMap,
}

struct Setup<'a> {
    name: &'a str,
    tmp: &'a Path,
    files_dir: PathBuf,
    hold: bool,
    store: TreeStore,
    keep_inline: Option<u64>,
    peer: Option<&'a TreeStore>,
}

fn device(s: Setup<'_>) -> Device {
    let docs: DocMap = Arc::default();
    let fetcher = Fetcher::new(s.store.inbox.clone());
    if let Some(p) = s.peer {
        fetcher.add_remote(Arc::new(p.clone()));
    }
    let source = docs.clone();
    let mut config = EngineConfig {
        settle: Duration::ZERO,
        site: s.name.into(),
        verify_window: Duration::from_secs(600),
        scrub_budget: 0,
        challenges: 0,
        ..EngineConfig::new(s.name, s.tmp.join(s.name).join("data"), s.files_dir, s.hold)
    };
    if let Some(k) = s.keep_inline {
        config.keep_inline = k;
    }
    let engine = Engine::new(
        config,
        s.store,
        fetcher,
        Arc::new(move |id| {
            let doc = source.lock().unwrap().entry(id).or_default().clone();
            Box::pin(async move { Some(doc) })
        }),
    );
    Device { engine, docs }
}

fn plain(
    name: &str,
    tmp: &Path,
    files_dir: PathBuf,
    hold: bool,
    peer: Option<&TreeStore>,
) -> Device {
    device(Setup {
        name,
        tmp,
        files_dir,
        hold,
        store: TreeStore::new(LocalFs::new(tmp.join(name).join("blobs"))),
        keep_inline: None,
        peer,
    })
}

/// A holding node as the node runs it: incoming bytes and kept versions
/// on the tree's own filesystem, under `history` (a budget for them).
fn holding_node(tmp: &Path, tree: &Path, history: Option<StoreBudget>) -> Device {
    let (incoming, versions) = layout::areas(tree);
    let mut kept = LocalFs::new(&versions);
    if let Some(b) = history {
        kept = kept.with_budget(Arc::new(b));
    }
    let store = TreeStore::new(LocalFs::new(tmp.join("node/blobs")))
        .with_areas(LocalFs::new(incoming), kept);
    device(Setup {
        name: "node",
        tmp,
        files_dir: tree.to_owned(),
        hold: true,
        store,
        keep_inline: None,
        peer: None,
    })
}

/// Documents and ledgers both ways, as the peer lanes carry them.
fn exchange(a: &Device, b: &Device) {
    let mut ledgers = a.engine.ledgers();
    ledgers.extend(b.engine.ledgers());
    let ledger_ids: Vec<Uuid> = ledgers.iter().map(|(id, _)| *id).collect();
    for (_, which) in ledgers {
        a.engine.learn_ledger(which.clone());
        b.engine.learn_ledger(which);
    }
    let mut ids: Vec<Uuid> = a.docs.lock().unwrap().keys().copied().collect();
    ids.extend(b.docs.lock().unwrap().keys().copied());
    for id in ids {
        let da = a.docs.lock().unwrap().entry(id).or_default().clone();
        let db = b.docs.lock().unwrap().entry(id).or_default().clone();
        db.import(&da.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        da.import(&db.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        if !ledger_ids.contains(&id) {
            a.engine.learn(id);
            b.engine.learn(id);
        }
    }
}

fn project(files: &Path) -> (PathBuf, Vec<u8>) {
    let p = files.join("acme/Projects/Album");
    std::fs::create_dir_all(p.join("Audio")).unwrap();
    let bytes: Vec<u8> = (0..10_000u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    std::fs::write(p.join("Audio/take.wav"), &bytes).unwrap();
    std::fs::write(p.join("song.rpp"), b"<REAPER_PROJECT\n>\n").unwrap();
    (p, bytes)
}

/// The same size, other bytes: an edit made on the tree itself.
fn edit_in_place(file: &Path, old: &[u8]) -> Vec<u8> {
    std::thread::sleep(Duration::from_millis(20));
    let mut new = old.to_vec();
    new[0] ^= 0xff;
    std::fs::write(file, &new).unwrap();
    new
}

async fn passes(d: &Device, n: usize) {
    for _ in 0..n {
        d.engine.pass_all().await;
    }
}

fn all_files(dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_type().unwrap().is_dir() {
            all_files(&e.path(), out);
        } else {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
    }
}

/// Two devices that fetched an old version, then evict it for space:
/// in the review both succeeded and the version was gone everywhere.
async fn race(order: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    let (p, old) = project(&tree);
    let node = plain("node", tmp.path(), tree, true, None);
    passes(&node, 2).await;
    let a = plain(
        "a",
        tmp.path(),
        tmp.path().join("a-files"),
        false,
        Some(&node.engine.store),
    );
    let b = plain(
        "b",
        tmp.path(),
        tmp.path().join("b-files"),
        false,
        Some(&node.engine.store),
    );
    exchange(&node, &a);
    exchange(&node, &b);
    a.engine.pass_all().await;
    b.engine.pass_all().await;
    let x = Sha256::of(&old);
    for d in [&a, &b] {
        d.engine.fetcher.fetch(x).await.unwrap();
        d.engine.note_verified(&x.to_hex());
        d.engine.attest(&[x]).await;
    }
    exchange(&a, &b);
    exchange(&node, &a);
    exchange(&node, &b);
    // The node's user edits the take in place: X is an old version now,
    // held only by a and b, each verified in the other's eyes.
    edit_in_place(&p.join("Audio/take.wav"), &old);
    passes(&node, 2).await;
    exchange(&node, &a);
    exchange(&node, &b);
    a.engine.pass_all().await;
    b.engine.pass_all().await;
    exchange(&a, &b);

    let (ra, rb) = match order {
        "together" => tokio::join!(a.engine.evict_for_space(x), b.engine.evict_for_space(x)),
        "a-then-b" => {
            let ra = a.engine.evict_for_space(x).await;
            (ra, b.engine.evict_for_space(x).await)
        }
        _ => {
            let rb = b.engine.evict_for_space(x).await;
            (a.engine.evict_for_space(x).await, rb)
        }
    };
    let held = |d: &Device| d.engine.store.local.path_for(&x).is_file();
    assert!(ra.is_err() && rb.is_err(), "{order}: a={ra:?} b={rb:?}");
    assert!(
        held(&a) && held(&b),
        "{order}: the old version is kept by both"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_holders_evicting_one_old_version_never_drop_it_everywhere() {
    for order in ["together", "a-then-b", "b-then-a"] {
        race(order).await;
    }
}

/// Space eviction drops a store copy only while a working file holds the
/// same bytes, checked by hash: an edit that kept size and mtime (so the
/// index still says "held") does not count.
#[tokio::test(flavor = "multi_thread")]
async fn eviction_rehashes_the_working_file_before_dropping_its_store_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let files = tmp.path().join("files");
    let (p, old) = project(&files);
    let laptop = plain("laptop", tmp.path(), files.clone(), true, None);
    passes(&laptop, 2).await;
    let x = Sha256::of(&old);
    laptop.engine.store.local.put_bytes(&old).await.unwrap();

    // Same size, same mtime, other bytes: the stat says nothing changed.
    let file = p.join("Audio/take.wav");
    let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
    let mut new = old.clone();
    new[0] ^= 0xff;
    std::fs::write(&file, &new).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let refused = laptop.engine.evict_for_space(x).await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(
        laptop.engine.store.local.path_for(&x).is_file(),
        "the only copy stays"
    );
}

/// M2: a holding node's store copy of a file it did not write itself is
/// kept, so an edit made directly on the tree still has its previous
/// version. #527's first version deleted it as a duplicate.
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_edit_on_the_tree_keeps_the_previous_version_the_node_had() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    let (p, old) = project(&tree);
    let node = holding_node(tmp.path(), &tree, None);
    // A copy from before (main kept one of every small file).
    node.engine.store.local.put_bytes(&old).await.unwrap();
    passes(&node, 3).await;
    let x = Sha256::of(&old);
    assert!(
        node.engine.store.local.path_for(&x).is_file(),
        "not dropped as a duplicate"
    );

    edit_in_place(&p.join("Audio/take.wav"), &old);
    passes(&node, 3).await;
    assert!(
        node.engine.store.has(x).await.unwrap(),
        "the previous version is still served"
    );
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();
    let versions = node.engine.tree(id).await.unwrap();
    let previous = versions
        .history("Audio/take.wav")
        .iter()
        .find(|v| v.content.as_deref() == Some(&*x.to_hex()))
        .cloned()
        .unwrap();
    let bytes = node.engine.version_bytes(previous.id, &x.to_hex()).await;
    assert!(bytes.here && !bytes.lost(), "{bytes:?}");
}

/// M2: an edit made on the tree behind the node's back, of a file it had
/// no other copy of, cannot keep its previous bytes (they are already
/// overwritten when the scan sees it). The version list says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_version_whose_bytes_no_one_holds_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    let (p, old) = project(&tree);
    let node = holding_node(tmp.path(), &tree, None);
    passes(&node, 2).await;
    edit_in_place(&p.join("Audio/take.wav"), &old);
    passes(&node, 3).await;
    let x = Sha256::of(&old);
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();
    let tree_doc = node.engine.tree(id).await.unwrap();
    let previous = tree_doc
        .history("Audio/take.wav")
        .iter()
        .find(|v| v.content.as_deref() == Some(&*x.to_hex()))
        .cloned()
        .unwrap();
    let bytes = node.engine.version_bytes(previous.id, &x.to_hex()).await;
    assert!(!bytes.here, "{bytes:?}");
    assert!(bytes.lost(), "no holder on record: {bytes:?}");
}

/// M2: a version the node replaces itself (a peer's newer version) is
/// kept on the tree's filesystem, served and attested; nothing of it is
/// in the node's store, and the scan never sees the versions area.
#[tokio::test(flavor = "multi_thread")]
async fn a_version_the_node_replaces_is_kept_beside_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    let (_, old) = project(&tree);
    let node = holding_node(tmp.path(), &tree, None);
    passes(&node, 2).await;
    let laptop_files = tmp.path().join("laptop-files");
    let laptop = plain(
        "laptop",
        tmp.path(),
        laptop_files.clone(),
        false,
        Some(&node.engine.store),
    );
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));
    exchange(&node, &laptop);
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();
    let copy = laptop.engine.pin(id, None).await.unwrap();
    passes(&laptop, 2).await;
    let new = edit_in_place(&copy.join("Audio/take.wav"), &old);
    passes(&laptop, 2).await;
    exchange(&node, &laptop);
    passes(&node, 2).await;

    let held = tree.join("acme/Projects/Album/Audio/take.wav");
    assert_eq!(
        std::fs::read(&held).unwrap(),
        new,
        "the node has the laptop's edit"
    );
    let x = Sha256::of(&old);
    let kept = node.engine.store.file_for(&x).unwrap();
    assert!(
        kept.starts_with(tree.join(layout::AREA)),
        "{}",
        kept.display()
    );
    assert_eq!(std::fs::read(&kept).unwrap(), old);
    assert!(
        !node.engine.store.local.path_for(&x).is_file(),
        "not in the node's store"
    );
    let paths: Vec<String> = node
        .engine
        .tree(id)
        .await
        .unwrap()
        .paths()
        .map(str::to_owned)
        .collect();
    assert!(paths.iter().all(|p| !p.contains(layout::AREA)), "{paths:?}");
}

/// M2: the kept versions have their own bound; under it (here the tree's
/// filesystem at its floor) the replacement waits and says so. Nothing
/// is deleted and the file stays as it was.
#[tokio::test(flavor = "multi_thread")]
async fn kept_versions_at_their_floor_defer_the_replacement_and_delete_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("nfs");
    let (_, old) = project(&tree);
    let free = Arc::new(AtomicU64::new(1_000));
    let probe_free = free.clone();
    let budget = StoreBudget::new(layout::areas(&tree).1, u64::MAX, 100_000).with_probe(Arc::new(
        move |_| {
            Some(Space {
                total: 1 << 30,
                free: probe_free.load(Ordering::SeqCst),
            })
        },
    ));
    let node = holding_node(tmp.path(), &tree, Some(budget));
    passes(&node, 2).await;
    let laptop = plain(
        "laptop",
        tmp.path(),
        tmp.path().join("laptop-files"),
        false,
        Some(&node.engine.store),
    );
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));
    exchange(&node, &laptop);
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();
    let copy = laptop.engine.pin(id, None).await.unwrap();
    passes(&laptop, 2).await;
    let new = edit_in_place(&copy.join("Audio/take.wav"), &old);
    passes(&laptop, 2).await;
    exchange(&node, &laptop);
    passes(&node, 2).await;

    let held = tree.join("acme/Projects/Album/Audio/take.wav");
    assert_eq!(
        std::fs::read(&held).unwrap(),
        old,
        "not replaced while its version cannot be kept"
    );
    assert_eq!(node.engine.status().await[0].deferred, 1);

    free.store(10_000_000, Ordering::SeqCst);
    passes(&node, 2).await;
    assert_eq!(std::fs::read(&held).unwrap(), new);
    assert_eq!(
        node.engine
            .store
            .file_for(&Sha256::of(&old))
            .map(|p| std::fs::read(p).unwrap()),
        Some(old)
    );
    assert_eq!(node.engine.status().await[0].deferred, 0);
}

/// M4: with the store on another filesystem (tmpfs) than the tree, a
/// fetched file arrives through a verified copy, and no temp name of it
/// is ever committed or left in the tree.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_across_filesystems_never_commits_its_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let shm = tempfile::tempdir_in("/dev/shm").unwrap();
    let laptop_files = tmp.path().join("laptop-files");
    let (_, bytes) = project(&laptop_files);
    let laptop = plain("laptop", tmp.path(), laptop_files, true, None);
    passes(&laptop, 2).await;
    let tree = tmp.path().join("nfs");
    std::fs::create_dir_all(&tree).unwrap();
    let node = device(Setup {
        name: "node",
        tmp: tmp.path(),
        files_dir: tree.clone(),
        hold: true,
        store: TreeStore::new(LocalFs::new(shm.path().join("blobs"))),
        keep_inline: None,
        peer: Some(&laptop.engine.store),
    });
    exchange(&laptop, &node);
    let id = Uuid::parse_str(&laptop.engine.status().await[0].doc_id).unwrap();
    let album = tree.join("acme/Projects/Album");
    // What a copy that died half-way left behind (the old double suffix).
    std::fs::create_dir_all(album.join("Audio")).unwrap();
    let leftover = album.join("Audio/gtr.wav.architect-part.part");
    std::fs::write(&leftover, &bytes[..4_000]).unwrap();
    node.engine.pin(id, Some(album.clone())).await.unwrap();
    passes(&node, 4).await;

    assert_eq!(std::fs::read(album.join("Audio/take.wav")).unwrap(), bytes);
    let mut names = Vec::new();
    all_files(&album, &mut names);
    names.sort();
    assert_eq!(
        names,
        ["song.rpp", "take.wav"],
        "no temp file left in the tree"
    );
    let paths: Vec<String> = node
        .engine
        .tree(id)
        .await
        .unwrap()
        .paths()
        .map(str::to_owned)
        .collect();
    assert!(paths.iter().all(|p| !layout::is_temp(p)), "{paths:?}");
    let mut stored = Vec::new();
    all_files(&shm.path().join("blobs"), &mut stored);
    assert!(
        stored.is_empty(),
        "the store's copies left once in the tree: {stored:?}"
    );
}

/// S1: a nearly full store never leaves a path empty: the new version is
/// staged first, and with no room for it nothing changes.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_store_never_leaves_a_path_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let node_tree = tmp.path().join("nfs");
    let (_, old) = project(&node_tree);
    let node = plain("node", tmp.path(), node_tree.clone(), true, None);
    passes(&node, 2).await;
    // The laptop's store has room for the old version (10 kB) but not
    // for it and the new one too.
    let files = tmp.path().join("laptop-files");
    let blobs = tmp.path().join("laptop/blobs");
    let laptop = device(Setup {
        name: "laptop",
        tmp: tmp.path(),
        files_dir: files,
        hold: false,
        store: TreeStore::new(
            LocalFs::new(&blobs).with_budget(Arc::new(StoreBudget::new(&blobs, 15_000, 0))),
        ),
        keep_inline: Some(0),
        peer: Some(&node.engine.store),
    });
    exchange(&node, &laptop);
    let id = Uuid::parse_str(&node.engine.status().await[0].doc_id).unwrap();
    let copy = laptop.engine.pin(id, None).await.unwrap();
    passes(&laptop, 3).await;
    let take = copy.join("Audio/take.wav");
    assert_eq!(std::fs::read(&take).unwrap(), old);

    edit_in_place(&node_tree.join("acme/Projects/Album/Audio/take.wav"), &old);
    passes(&node, 2).await;
    exchange(&node, &laptop);
    passes(&laptop, 2).await;

    assert_eq!(
        std::fs::read(&take).ok(),
        Some(old),
        "the path keeps its file"
    );
    let status = laptop.engine.status().await;
    assert_eq!((status[0].missing, status[0].deferred), (1, 1));
}

/// S2: a file larger than the whole store limit is "too large", not
/// merely deferred; on a holding node, whose fetches land on the tree's
/// filesystem, it arrives anyway.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_larger_than_the_store_is_too_large_or_lands_beside_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    let (_, bytes) = project(&source);
    let origin = plain("origin", tmp.path(), source, true, None);
    passes(&origin, 2).await;
    let small = |name: &str| {
        let blobs = tmp.path().join(name).join("blobs");
        LocalFs::new(&blobs).with_budget(Arc::new(StoreBudget::new(&blobs, 5_000, 0)))
    };
    let laptop = device(Setup {
        name: "laptop",
        tmp: tmp.path(),
        files_dir: tmp.path().join("laptop-files"),
        hold: false,
        store: TreeStore::new(small("laptop")),
        keep_inline: None,
        peer: Some(&origin.engine.store),
    });
    exchange(&origin, &laptop);
    let id = Uuid::parse_str(&origin.engine.status().await[0].doc_id).unwrap();
    laptop.engine.pin(id, None).await.unwrap();
    passes(&laptop, 2).await;
    let s = &laptop.engine.status().await[0];
    assert_eq!((s.too_large, s.deferred), (1, 0), "{s:?}");

    // The node's store is as small, but its fetches land beside the tree.
    let tree = tmp.path().join("nfs");
    std::fs::create_dir_all(&tree).unwrap();
    let (incoming, versions) = layout::areas(&tree);
    let node = device(Setup {
        name: "node",
        tmp: tmp.path(),
        files_dir: tree.clone(),
        hold: true,
        store: TreeStore::new(small("node"))
            .with_areas(LocalFs::new(incoming), LocalFs::new(versions)),
        keep_inline: None,
        peer: Some(&origin.engine.store),
    });
    exchange(&origin, &node);
    let album = tree.join("acme/Projects/Album");
    node.engine.pin(id, Some(album.clone())).await.unwrap();
    passes(&node, 3).await;
    assert_eq!(std::fs::read(album.join("Audio/take.wav")).unwrap(), bytes);
    assert_eq!(node.engine.status().await[0].too_large, 0);
}

/// S2: a partial fetch no transfer resumes is removed once it is old.
#[tokio::test(flavor = "multi_thread")]
async fn orphaned_partial_fetches_are_cleaned() {
    let tmp = tempfile::tempdir().unwrap();
    let files = tmp.path().join("files");
    project(&files);
    let d = plain("d", tmp.path(), files, true, None);
    let partial = d
        .engine
        .store
        .local
        .partial_path(&Sha256::of(b"never"))
        .with_extension("part");
    std::fs::create_dir_all(partial.parent().unwrap()).unwrap();
    std::fs::write(&partial, vec![1u8; 4_000]).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_hours(2);
    std::fs::File::options()
        .write(true)
        .open(&partial)
        .unwrap()
        .set_modified(old)
        .unwrap();
    d.engine.pass_all().await;
    assert!(!partial.exists());
    assert!(d.engine.store.has(Sha256::of(b"never")).await.is_ok());
}
