//! Synced roots between a node that holds a tree in place and a laptop
//! that pins one project (feature `sync`). The manifests travel as Loro
//! updates, as they do on the peer lanes; the bytes through a `Fetcher`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{Fetcher, LocalFs};
use architect_files::sync::{
    Engine, EngineConfig, EngineError, Manifest, Section, Settled, TreeStore,
};
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

/// Both ways, every document either side holds (what a live bridge does).
fn exchange(a: &Device, b: &Device) {
    let ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = a.docs.lock().unwrap().keys().copied().collect();
        ids.extend(b.docs.lock().unwrap().keys().copied());
        ids
    };
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

/// Two passes: the first sees a change, the second commits it.
async fn passes(d: &Device) {
    d.engine.pass_all().await;
    d.engine.pass_all().await;
}

fn project(files: &Path) -> PathBuf {
    let p = files.join("codywright/Projects/Right Here - Cody Wright");
    std::fs::create_dir_all(p.join("Right Here/Audio Files")).unwrap();
    std::fs::create_dir_all(p.join(".fts-files/store")).unwrap();
    std::fs::write(p.join(".fts-files/cadence.json"), "{}").unwrap();
    std::fs::write(
        p.join(".fts-root.json"),
        r#"{"id":"7392cd25-c5f0-487a-be42-df50d0666a36","name":"Right Here - Cody Wright"}"#,
    )
    .unwrap();
    std::fs::write(
        p.join("Right Here/Right Here.RPP"),
        "<REAPER_PROJECT 0.1\n>\n",
    )
    .unwrap();
    std::fs::write(
        p.join("Right Here/Audio Files/Bass_1.wav"),
        vec![7u8; 300_000],
    )
    .unwrap();
    p
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Wait for a file's mtime to be past the last index entry (coarse clocks).
async fn tick() {
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_project_syncs_both_ways_with_the_node_and_unpins_once_synced() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    let held = project(&tree);
    let node = device("node", tmp.path(), tree.clone(), true, None);
    let laptop = device(
        "laptop",
        tmp.path(),
        tmp.path().join("laptop-files"),
        false,
        Some(&node.engine.store),
    );
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));

    // The node imports the tree in place; Task's store is not touched.
    passes(&node).await;
    let roots = node.engine.status().await;
    let root = roots
        .iter()
        .find(|r| r.section == Section::Projects)
        .unwrap();
    assert_eq!(
        root.files, 2,
        "the session and its audio, not .fts-files: {root:?}"
    );
    let doc_id: Uuid = root.doc_id.parse().unwrap();
    assert_eq!(read(&held.join(".fts-files/cadence.json")), "{}");
    let checkpoints = node.engine.checkpoints(doc_id).await.unwrap();
    assert_eq!(
        checkpoints.first().map(|c| c.name.as_str()),
        Some("Imported")
    );

    // The laptop sees it without its bytes, then pins it.
    exchange(&node, &laptop);
    laptop.engine.pass_all().await;
    let seen = laptop.engine.root_status(doc_id).await.unwrap();
    assert!(!seen.pinned && seen.path.is_none());
    let copy = laptop.engine.pin(doc_id, None).await.unwrap();
    let session = copy.join("Right Here/Right Here.RPP");
    assert_eq!(read(&session), "<REAPER_PROJECT 0.1\n>\n");
    assert_eq!(
        std::fs::metadata(copy.join("Right Here/Audio Files/Bass_1.wav"))
            .unwrap()
            .len(),
        300_000
    );
    assert!(!copy.join(".fts-files").exists());

    // An edit on the laptop reaches the node, as a save point.
    tick().await;
    std::fs::write(&session, "<REAPER_PROJECT 0.2\n>\n").unwrap();
    passes(&laptop).await;
    exchange(&node, &laptop);
    node.engine.pass_all().await;
    assert_eq!(
        read(&held.join("Right Here/Right Here.RPP")),
        "<REAPER_PROJECT 0.2\n>\n"
    );
    let names: Vec<String> = node
        .engine
        .checkpoints(doc_id)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(
        names.contains(&"Saved Right Here.RPP".to_owned()),
        "{names:?}"
    );

    // An edit on the node reaches the laptop.
    tick().await;
    std::fs::write(held.join("Right Here/Notes.txt"), "from the node").unwrap();
    passes(&node).await;
    exchange(&node, &laptop);
    laptop.engine.pass_all().await;
    assert_eq!(read(&copy.join("Right Here/Notes.txt")), "from the node");

    // A change not synced yet keeps the copy; once synced it goes.
    tick().await;
    std::fs::write(&session, "<REAPER_PROJECT 0.3\n>\n").unwrap();
    assert!(
        matches!(laptop.engine.unpin(doc_id).await, Err(EngineError::NotSynced(p)) if p == ["Right Here/Right Here.RPP"])
    );
    passes(&laptop).await;
    laptop.engine.unpin(doc_id).await.unwrap();
    assert!(!session.exists());
    let after = laptop.engine.root_status(doc_id).await.unwrap();
    assert!(!after.pinned && after.files == 3, "{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_offline_saves_keep_both_and_a_restore_brings_a_version_back() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    let held = project(&tree);
    let node = device("node", tmp.path(), tree.clone(), true, None);
    let laptop = device(
        "laptop",
        tmp.path(),
        tmp.path().join("laptop-files"),
        false,
        Some(&node.engine.store),
    );
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));
    passes(&node).await;
    let doc_id: Uuid = node.engine.status().await[0].doc_id.parse().unwrap();
    exchange(&node, &laptop);
    let copy = laptop.engine.pin(doc_id, None).await.unwrap();

    // Both save the session while apart.
    tick().await;
    std::fs::write(held.join("Right Here/Right Here.RPP"), "node's mix").unwrap();
    tick().await;
    std::fs::write(copy.join("Right Here/Right Here.RPP"), "laptop's mix").unwrap();
    passes(&node).await;
    passes(&laptop).await;
    exchange(&node, &laptop);
    node.engine.pass_all().await;
    laptop.engine.pass_all().await;

    // The newer wins the path; the other sits beside it, on both.
    for dir in [&held, &copy] {
        assert_eq!(
            read(&dir.join("Right Here/Right Here.RPP")),
            "laptop's mix",
            "{}",
            dir.display()
        );
        assert_eq!(
            read(&dir.join("Right Here/Right Here (conflict node).RPP")),
            "node's mix"
        );
    }
    // The fork is closed: both closed it, with the same bytes, so it is settled.
    exchange(&node, &laptop);
    let doc = node.docs.lock().unwrap().get(&doc_id).unwrap().clone();
    assert!(matches!(
        Manifest::new(&doc)
            .tree()
            .settled("Right Here/Right Here.RPP"),
        Some(Settled::One(_))
    ));

    // Restore the first version of the session.
    let first = Manifest::new(&doc)
        .tree()
        .history("Right Here/Right Here.RPP")[0]
        .id;
    node.engine
        .restore(doc_id, "Right Here/Right Here.RPP", first)
        .await
        .unwrap();
    node.engine.pass_all().await;
    assert_eq!(
        read(&held.join("Right Here/Right Here.RPP")),
        "<REAPER_PROJECT 0.1\n>\n"
    );
    exchange(&node, &laptop);
    laptop.engine.pass_all().await;
    assert_eq!(
        read(&copy.join("Right Here/Right Here.RPP")),
        "<REAPER_PROJECT 0.1\n>\n"
    );
}

fn wav(samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    for chunk in [
        &16u32.to_le_bytes()[..],
        &1u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &8000u32.to_le_bytes(),
        &16000u32.to_le_bytes(),
        &2u16.to_le_bytes(),
        &16u16.to_le_bytes(),
    ] {
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out.extend(data);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_library_s_assets_browse_search_and_draw_from_a_laptop_that_holds_none_of_it() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    let tracks = tree.join("days-to-praise/Assets/Tracks/Praise - Elevation Worship");
    std::fs::create_dir_all(tracks.join("audio/wav")).unwrap();
    std::fs::write(
        tracks.join("Praise - Elevation Worship.md"),
        "---\ntype: song\nartist: Elevation Worship\nkey: A\nstems:\n  - name: \"Click\"\n    path: \"audio/wav/01 - Click.wav\"\n---\n# Praise\n",
    )
    .unwrap();
    let click: Vec<i16> = (0..16000)
        .map(|i| if i % 800 < 40 { 20000 } else { 0 })
        .collect();
    std::fs::write(tracks.join("audio/wav/01 - Click.wav"), wav(&click)).unwrap();
    let setlists = tree.join("days-to-praise/Assets/Setlists");
    std::fs::create_dir_all(&setlists).unwrap();
    std::fs::write(
        setlists.join("Testing.md"),
        "---\ntype: setlist\ntitle: Testing\n---\n[[Praise - Elevation Worship]]\n",
    )
    .unwrap();

    let node = device("node", tmp.path(), tree.clone(), true, None);
    let laptop = device(
        "laptop",
        tmp.path(),
        tmp.path().join("laptop-files"),
        false,
        Some(&node.engine.store),
    );
    passes(&node).await;
    exchange(&node, &laptop);
    laptop.engine.pass_all().await;

    let library = laptop
        .engine
        .resolve_root("days-to-praise/Assets/Tracks")
        .unwrap();
    let assets = laptop.engine.assets(library).await.unwrap();
    assert_eq!(assets.len(), 1);
    let song = &assets[0];
    assert_eq!(song.properties["artist"], "Elevation Worship");
    assert_eq!(song.media[0].name, "Click");
    let setlist = laptop
        .engine
        .resolve_root("days-to-praise/Assets/Setlists")
        .unwrap();
    assert_eq!(
        laptop.engine.assets(setlist).await.unwrap()[0].links,
        vec!["Praise - Elevation Worship"]
    );

    let hits = laptop.engine.search("elevation song").await;
    assert!(
        hits.iter()
            .any(|h| h.what == "song" && h.name == "Praise - Elevation Worship"),
        "{hits:?}"
    );

    let peaks = laptop
        .engine
        .peaks(library, &song.media[0].path)
        .await
        .unwrap();
    assert!(peaks.length > 0 && peaks.data.iter().copied().max() == Some(20000));
}
