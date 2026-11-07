//! The registry and copy accounting between a node and two laptops
//! (feature `sync`): offline search with locations, a pin taking a file
//! from one verified copy to two, a corrupted copy caught by a challenge
//! and dropped by a scrub, evidence going stale, and an unpin refused
//! under policy. Documents travel as Loro updates, as on the peer lanes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::string_slice
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{Fetcher, LocalFs};
use architect_files::copies::{Confidence, Policy};
use architect_files::sync::{Engine, EngineConfig, EngineError, TreeStore};
use loro::{ExportMode, LoroDoc};
use uuid::Uuid;

type DocMap = Arc<Mutex<HashMap<Uuid, LoroDoc>>>;

struct Device {
    engine: Engine,
    docs: DocMap,
}

fn device(
    name: &str,
    tmp: &Path,
    files_dir: PathBuf,
    hold_all: bool,
    peer: Option<&TreeStore>,
    window: Duration,
) -> Device {
    let docs: DocMap = Arc::default();
    let local = LocalFs::new(tmp.join(name).join("blobs"));
    let store = TreeStore::new(local.clone());
    let fetcher = Fetcher::new(local);
    if let Some(peer) = peer {
        fetcher.add_remote(Arc::new(peer.clone()));
    }
    let source = docs.clone();
    let engine = Engine::new(
        EngineConfig {
            settle: Duration::ZERO,
            site: if hold_all { "starcommand" } else { "home" }.into(),
            verify_window: window,
            scrub_budget: 0,
            challenges: 0,
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

/// Every document both ways between `a` and `b`, ledgers named as such.
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

fn project(files: &Path) -> PathBuf {
    let p = files.join("tombrooksmusic/Projects/Crescendum Album");
    std::fs::create_dir_all(p.join("Crescendum/Audio Files")).unwrap();
    std::fs::write(
        p.join("Crescendum/Crescendum.rpp"),
        "<REAPER_PROJECT 0.1\n>\n",
    )
    .unwrap();
    let take: Vec<u8> = (0..(3 << 20) + 5).map(|i: u32| (i % 249) as u8).collect();
    std::fs::write(
        p.join("Crescendum/Audio Files/148-V LEAD-241011_2006.wav"),
        take,
    )
    .unwrap();
    p
}

const TAKE: &str = "Crescendum/Audio Files/148-V LEAD-241011_2006.wav";

async fn passes(d: &Device) {
    d.engine.pass_all().await;
    d.engine.pass_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_are_counted_on_evidence_from_one_to_two_and_back() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    project(&tree);
    let window = Duration::from_secs(3);
    let node = device("node", tmp.path(), tree, true, None, window);
    passes(&node).await;
    let a = device(
        "laptop",
        tmp.path(),
        tmp.path().join("laptop-files"),
        false,
        Some(&node.engine.store),
        window,
    );
    let b = device(
        "desk",
        tmp.path(),
        tmp.path().join("desk-files"),
        false,
        Some(&node.engine.store),
        window,
    );
    exchange(&node, &a);
    exchange(&node, &b);
    a.engine.pass_all().await;
    b.engine.pass_all().await;

    // Offline search on a device that holds none of it: where it lives.
    let hits = b.engine.search("crescendum lead").await;
    let hit = hits.iter().find(|h| h.path == TAKE).expect("the take");
    assert_eq!(hit.org, "tombrooksmusic");
    assert_eq!(hit.verified, 1, "{hit:?}");
    assert!(hit.holders[0].starts_with("node"), "{hit:?}");
    let hash = hit.content.clone().unwrap();
    let by_hash = b.engine.search(&format!("sha:{}", &hash[..12])).await;
    assert_eq!(by_hash.len(), 1);

    // A pin on the laptop: two verified copies, at two sites.
    let root = a
        .engine
        .resolve_root("tombrooksmusic/Projects/Crescendum Album")
        .unwrap();
    a.engine.pin(root, None).await.unwrap();
    a.engine.pass_all().await;
    exchange(&a, &b);
    exchange(&node, &b);
    b.engine.refresh_ledgers().await;
    let loc = b.engine.locations(&hash).await;
    assert_eq!(loc.verified, 2, "{loc:#?}");
    assert_eq!(loc.sites, 2);
    assert_eq!(loc.places[0].path, TAKE);

    // The laptop scrubs (its Merkle root), then its bytes rot, in the
    // blob store and in the working copy: same size, same mtime.
    let s = a.engine.scrub(Some(&hash), u64::MAX).await;
    assert_eq!((s.scrubbed, s.corrupt.len()), (1, 0));
    exchange(&a, &b);
    let file = a.engine.working_dir(root).unwrap().join(TAKE);
    let blob = a
        .engine
        .store
        .local
        .path_for(&architect::blob::Sha256::from_hex(&hash).unwrap());
    for f in [&file, &blob] {
        let mtime = std::fs::metadata(f).unwrap().modified().unwrap();
        let mut bytes = std::fs::read(f).unwrap();
        bytes[(2 << 20) + 9] ^= 0xff;
        std::fs::write(f, &bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(f)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }

    // The desk challenges the laptop: the proof fails, the copy is not counted.
    b.engine
        .add_prover(a.engine.holder_id(), Arc::new(a.engine.clone()));
    // Three chunks: challenge until the bad one is drawn.
    let mut failed = 0;
    for _ in 0..40 {
        let c = b.engine.challenge(4, Some(&hash)).await;
        failed += c.failed.len();
        if failed > 0 {
            break;
        }
    }
    assert!(failed > 0, "a challenge of the rotten chunk fails");
    let loc = b.engine.locations(&hash).await;
    assert_eq!(loc.verified, 1, "{loc:#?}");
    let laptop = loc
        .copies
        .iter()
        .find(|c| c.holder.name == "laptop")
        .unwrap();
    assert_eq!(laptop.confidence, Confidence::Failed);
    assert!(laptop.reachable);

    // The laptop's own scrub finds the rot and withdraws its claim.
    let s = a.engine.scrub(Some(&hash), u64::MAX).await;
    assert_eq!(s.corrupt, vec![hash.clone()]);
    a.engine.pass_all().await;
    exchange(&a, &b);
    b.engine.refresh_ledgers().await;
    let loc = b.engine.locations(&hash).await;
    assert!(
        loc.copies.iter().all(|c| c.holder.name != "laptop"),
        "{loc:#?}"
    );
    assert_eq!(loc.verified, 1);
    assert!(a.engine.config().data.join("corrupt").join(&hash).exists());

    // Two copies required: unpinning the laptop is refused while the
    // node's is the only other one.
    std::fs::write(
        &file,
        (0..(3u32 << 20) + 5)
            .map(|i| (i % 249) as u8)
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    a.engine
        .set_policy(
            "tombrooksmusic",
            Some(Policy {
                min_verified: 2,
                min_sites: 1,
            }),
        )
        .await
        .unwrap();
    passes(&a).await;
    let refused = a.engine.unpin(root).await;
    assert!(
        matches!(refused, Err(EngineError::BelowPolicy(_))),
        "{refused:?}"
    );

    // Nobody re-verifies for a window: every copy is stale, none counted.
    tokio::time::sleep(window + Duration::from_millis(300)).await;
    let loc = b.engine.locations(&hash).await;
    assert_eq!(loc.verified, 0, "{loc:#?}");
    assert!(
        loc.copies
            .iter()
            .all(|c| c.confidence == Confidence::Stale && c.verified_at_us > 0)
    );
    let report = b.engine.copy_report("tombrooksmusic").await;
    assert_eq!(report.files, 2);
    assert_eq!(report.unverified_bytes, report.bytes);
    assert_eq!(report.violation_count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_no_peer_lists_any_more_is_forgotten() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    project(&tree);
    let node = device(
        "node",
        tmp.path(),
        tree,
        true,
        None,
        Duration::from_secs(60),
    );
    passes(&node).await;
    let b = device(
        "desk",
        tmp.path(),
        tmp.path().join("desk-files"),
        false,
        Some(&node.engine.store),
        Duration::from_secs(60),
    );
    exchange(&node, &b);
    b.engine.pass_all().await;
    assert!(!b.engine.search("crescendum").await.is_empty());
    let all: Vec<Uuid> = node.docs.lock().unwrap().keys().copied().collect();
    assert!(b.engine.listed_by("node", &all).is_empty());
    // Access revoked: the node lists nothing to this device.
    let forgotten = b.engine.listed_by("node", &[]);
    assert!(!forgotten.is_empty());
    assert!(b.engine.search("crescendum").await.is_empty());
    assert_eq!(b.engine.registry_size().1, 0);
}

/// The registry at the scale of starcommand: 7 orgs, 210 roots, a
/// million files. `cargo test -p architect-files --features sync --test
/// copies -- --ignored --nocapture registry_at_scale`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: run by hand"]
async fn registry_at_scale() {
    use architect_files::sync::registry::{RootIndex, Row};
    let tmp = tempfile::tempdir().unwrap();
    let d = device(
        "bench",
        tmp.path(),
        tmp.path().join("f"),
        false,
        None,
        Duration::from_secs(60),
    );
    let orgs = [
        "tombrooksmusic",
        "codywright",
        "cbu",
        "days-to-praise",
        "fasttrackaudio",
        "fasttrackstudio",
        "rockstars-of-tomorrow",
    ];
    let t = std::time::Instant::now();
    let mut n = 0u64;
    for r in 0..210u32 {
        let org = orgs[(r % 7) as usize];
        let name = format!("Project {r} - Artist {}", r % 13);
        let place = format!("{org}/Projects/{name}");
        let mut rows = vec![Row {
            path: String::new(),
            size: 0,
            at_us: 0,
            content: String::new(),
            versions: 0,
            what: "project".into(),
            props: String::new(),
            name: String::new(),
        }];
        for f in 0..4762u32 {
            n += 1;
            let song = f / 300;
            let path = if f % 3 == 0 {
                format!("Song {song}/Song {song}-2024-10-11_{f:04}.rpp-bak")
            } else {
                format!(
                    "Song {song}/Audio Files/{}-V LEAD-241011_{f:04}.wav",
                    f % 200
                )
            };
            let content = architect::blob::Sha256::of(format!("{r}/{f}").as_bytes()).to_hex();
            rows.push(Row {
                path,
                size: 30 << 20,
                at_us: 1,
                content,
                versions: 1,
                what: "file".into(),
                props: String::new(),
                name: String::new(),
            });
        }
        let hashes = rows
            .iter()
            .filter(|r| !r.content.is_empty())
            .map(|r| r.content.clone())
            .collect();
        d.engine.put_index(RootIndex {
            doc_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, place.as_bytes()).to_string(),
            org: org.into(),
            name,
            place,
            section: architect_files::sync::Section::Projects,
            digest: "x".into(),
            rows,
            hashes,
        });
    }
    let built = t.elapsed();
    let bytes: u64 = std::fs::read_dir(tmp.path().join("bench/data/registry"))
        .unwrap()
        .flatten()
        .map(|e| e.metadata().unwrap().len())
        .sum();
    // A fresh engine on the same data dir: load from disk.
    let again = device(
        "bench",
        tmp.path(),
        tmp.path().join("f"),
        false,
        None,
        Duration::from_secs(60),
    );
    let t = std::time::Instant::now();
    let (roots, rows) = again.engine.registry_size();
    let loaded = t.elapsed();
    let mut times = Vec::new();
    let by_hash = format!(
        "sha:{}",
        &architect::blob::Sha256::of(b"70/1234").to_hex()[..12]
    );
    for q in [
        "lead 2006",
        "song 7 rpp-bak",
        "artist 3 lead",
        "tombrooksmusic project 70 lead",
        &by_hash,
        "zzzz-no-such",
    ] {
        let t = std::time::Instant::now();
        let hits = again.engine.registry_search(q, 200).await;
        times.push(format!("{q:?}: {} hits in {:?}", hits.len(), t.elapsed()));
    }
    println!(
        "files {n}, roots {roots}, rows {rows}; built+persisted {built:?}; {} MB on disk; loaded {loaded:?}",
        bytes >> 20
    );
    for t in times {
        println!("  {t}");
    }
}

/// A prover that never answers: an offline holder.
struct Gone;

impl architect_files::sync::Prover for Gone {
    fn prove(
        &self,
        _hash: architect::blob::Sha256,
        _index: u64,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<architect_files::sync::Proof, architect_files::sync::ProveError>,
    > {
        Box::pin(async {
            Err(architect_files::sync::ProveError::Unreachable(
                "offline".into(),
            ))
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_holder_ages_but_never_fails_and_a_restart_re_signs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    project(&tree);
    let node = device(
        "node",
        tmp.path(),
        tree.clone(),
        true,
        None,
        Duration::from_secs(60),
    );
    passes(&node).await;
    let b = device(
        "desk",
        tmp.path(),
        tmp.path().join("desk-files"),
        false,
        Some(&node.engine.store),
        Duration::from_secs(60),
    );
    exchange(&node, &b);
    b.engine.pass_all().await;
    let hash = b.engine.search("lead").await[0].content.clone().unwrap();

    // The node scrubs (a Merkle root to check against), then drops off.
    node.engine.scrub(None, u64::MAX).await;
    exchange(&node, &b);
    b.engine.add_prover(node.engine.holder_id(), Arc::new(Gone));
    let c = b.engine.challenge(8, None).await;
    assert_eq!(
        (c.challenged, c.failed.len()),
        (0, 0),
        "no answer is no evidence"
    );
    assert_eq!(b.engine.locations(&hash).await.verified, 1);

    // The node restarts on the same data: it knows its ledgers and
    // signs nothing again.
    let before: Vec<i64> = {
        let loc = b.engine.locations(&hash).await;
        loc.copies.iter().map(|c| c.verified_at_us).collect()
    };
    let ledger_ids: Vec<Uuid> = node
        .engine
        .ledgers()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let versions = |d: &Device| -> Vec<Vec<u8>> {
        ledger_ids
            .iter()
            .map(|id| d.docs.lock().unwrap().get(id).unwrap().oplog_vv().encode())
            .collect()
    };
    let was = versions(&node);
    let source = node.docs.clone();
    let local = LocalFs::new(tmp.path().join("node").join("blobs"));
    let again = Engine::new(
        EngineConfig {
            settle: Duration::ZERO,
            scrub_budget: 0,
            challenges: 0,
            ..EngineConfig::new("node", tmp.path().join("node").join("data"), tree, true)
        },
        TreeStore::new(local.clone()),
        Fetcher::new(local),
        Arc::new(move |id| {
            let doc = source.lock().unwrap().entry(id).or_default().clone();
            Box::pin(async move { Some(doc) })
        }),
    );
    assert_eq!(again.ledgers().len(), ledger_ids.len());
    again.pass_all().await;
    assert_eq!(versions(&node), was, "a restart rewrote attestations");
    let after: Vec<i64> = b
        .engine
        .locations(&hash)
        .await
        .copies
        .iter()
        .map(|c| c.verified_at_us)
        .collect();
    assert_eq!(before, after);
}

#[tokio::test(flavor = "multi_thread")]
async fn evicting_one_cached_blob_is_refused_under_policy_and_withdrawn_otherwise() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    project(&tree);
    let window = Duration::from_secs(60);
    let node = device("node", tmp.path(), tree, true, None, window);
    passes(&node).await;
    let b = device(
        "desk",
        tmp.path(),
        tmp.path().join("desk-files"),
        false,
        Some(&node.engine.store),
        window,
    );
    exchange(&node, &b);
    b.engine.pass_all().await;
    let hash = b.engine.search("lead").await[0].content.clone().unwrap();
    let sha = architect::blob::Sha256::from_hex(&hash).unwrap();
    // Fetched into the desk's cache (what the drive's `Cache::begin` does).
    b.engine.fetcher.fetch(sha).await.unwrap();
    b.engine.note_verified(&hash);
    b.engine.attest(&[sha]).await;
    assert_eq!(b.engine.locations(&hash).await.verified, 2);

    // The default policy wants two verified copies: the desk's may not go.
    let refused = b.engine.evict_blob(sha).await;
    assert!(
        matches!(refused, Err(EngineError::BelowPolicy(_))),
        "{refused:?}"
    );
    assert!(b.engine.store.local.path_for(&sha).is_file());

    // One copy is enough: it goes, and so does its attestation.
    b.engine
        .set_policy(
            "tombrooksmusic",
            Some(Policy {
                min_verified: 1,
                min_sites: 1,
            }),
        )
        .await
        .unwrap();
    b.engine.evict_blob(sha).await.unwrap();
    assert!(!b.engine.store.local.path_for(&sha).is_file());
    let loc = b.engine.locations(&hash).await;
    assert!(loc.copies.iter().all(|c| !c.here), "{loc:#?}");
    assert_eq!(loc.verified, 1);
}

/// A real S3 server in process (`s3s-fs`), as `architect`'s `blob_s3` tests use.
async fn bucket(root: &Path) -> architect::blob::S3Store {
    const AK: &str = "AKIATESTTESTTESTTEST";
    const SK: &str = "testsecrettestsecrettestsecrettestsecret";
    std::fs::create_dir_all(root.join("blobs")).unwrap();
    let fs = s3s_fs::FileSystem::new(root).unwrap();
    let mut builder = s3s::service::S3ServiceBuilder::new(fs);
    builder.set_auth(s3s::auth::SimpleAuth::from_single(AK, SK));
    let service = builder.build();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service.clone();
            tokio::spawn(async move {
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
            });
        }
    });
    architect::blob::S3Store::open(&architect::blob::S3Config {
        endpoint: Some(format!("http://{addr}")),
        region: "us-east-1".into(),
        bucket: "blobs".into(),
        prefix: "architect".into(),
        access_key_id: AK.into(),
        secret_access_key: SK.into(),
        virtual_hosted: false,
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bucket_the_node_writes_to_is_a_holder_it_signs_for() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    project(&tree);
    let node = device(
        "node",
        tmp.path(),
        tree,
        true,
        None,
        Duration::from_secs(60),
    );
    passes(&node).await;
    let hash = node.engine.search("lead").await[0].content.clone().unwrap();
    let sha = architect::blob::Sha256::from_hex(&hash).unwrap();
    let s3 = bucket(&tmp.path().join("s3")).await;
    let holder = architect_files::sync::BucketHolder::new(
        node.engine.clone(),
        &s3,
        Some("http://garage.lan:3900"),
    );
    let s3 = s3.with_events(Arc::new(holder));

    // Replicated into the bucket: a second verified copy, at another site.
    let file = node.engine.store.file_for(&sha).unwrap();
    let body = tokio::fs::File::open(file).await.unwrap();
    s3.put_reader(Some(sha), Box::pin(body)).await.unwrap();
    let mut loc = node.engine.locations(&hash).await;
    for _ in 0..50 {
        if loc.verified == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        loc = node.engine.locations(&hash).await;
    }
    let copy = loc
        .copies
        .iter()
        .find(|c| c.holder.kind == architect_files::copies::HolderKind::S3)
        .expect("the bucket");
    assert_eq!(copy.holder.site, "garage.lan");
    assert_eq!(
        copy.holder.manager.as_deref(),
        Some(node.engine.holder_id().as_str())
    );
    assert_eq!((loc.verified, loc.sites), (2, 2), "{loc:#?}");

    // Deleted from the bucket: withdrawn.
    s3.delete(sha).await.unwrap();
    for _ in 0..50 {
        loc = node.engine.locations(&hash).await;
        if loc.verified == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        loc.copies
            .iter()
            .all(|c| c.holder.kind != architect_files::copies::HolderKind::S3),
        "{loc:#?}"
    );
}
