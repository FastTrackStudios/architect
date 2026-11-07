//! The drive over a real FUSE mount (feature `drive`, Linux): a node holds
//! a project, a laptop mounts every root it knows, and plain file calls
//! through the mount browse, read on demand, write versions, save
//! atomically and pin by xattr. Skipped where FUSE cannot mount.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{Fetcher, LocalFs};
use architect_files::drive;
use architect_files::sync::{Engine, EngineConfig, Manifest, TreeStore};
use loro::{ExportMode, LoroDoc};
use uuid::Uuid;

type DocMap = Arc<Mutex<HashMap<Uuid, LoroDoc>>>;

struct Device {
    engine: Engine,
    docs: DocMap,
}

fn device(name: &str, tmp: &Path, files_dir: PathBuf, hold_all: bool) -> Device {
    let docs: DocMap = Arc::default();
    let local = LocalFs::new(tmp.join(name).join("blobs"));
    let source = docs.clone();
    let engine = Engine::new(
        EngineConfig {
            settle: Duration::ZERO,
            ..EngineConfig::new(name, tmp.join(name).join("data"), files_dir, hold_all)
        },
        TreeStore::new(local.clone()),
        Fetcher::new(local),
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

/// Run a blocking file call off the runtime the mount calls back into.
async fn fs<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

fn xattr(path: &Path, name: &str) -> String {
    let out = Command::new("getfattr")
        .args(["--only-values", "-n", name])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_drive_browses_reads_on_demand_writes_versions_and_pins_by_xattr() {
    if !Path::new("/dev/fuse").exists()
        || Command::new("getfattr").arg("--version").output().is_err()
    {
        eprintln!("no /dev/fuse or getfattr here: skipped");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    let held = tree.join("codywright/Projects/Right Here");
    std::fs::create_dir_all(held.join("Audio Files")).unwrap();
    std::fs::write(held.join("Right Here.RPP"), "<REAPER_PROJECT\n>\n").unwrap();
    let take: Vec<u8> = (0..3_000_000_u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    std::fs::write(held.join("Audio Files/Bass_1.wav"), &take).unwrap();

    let node = device("node", tmp.path(), tree.clone(), true);
    let laptop = device("laptop", tmp.path(), tmp.path().join("laptop-roots"), false);
    laptop
        .engine
        .fetcher
        .add_remote(Arc::new(node.engine.store.clone()));
    node.engine
        .fetcher
        .add_remote(Arc::new(laptop.engine.store.clone()));
    node.engine.pass_all().await;
    node.engine.pass_all().await;
    exchange(&node, &laptop);
    laptop.engine.pass_all().await;

    let mnt = tmp.path().join("Architect");
    let mounted = match drive::mount(
        laptop.engine.clone(),
        tokio::runtime::Handle::current(),
        &mnt,
        tmp.path().join("drive-tmp"),
        false,
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("FUSE did not mount here ({e}): skipped");
            return;
        }
    };
    let project = mnt.join("codywright/Projects/Right Here");

    // Browsed from the manifest, real sizes, nothing fetched.
    let p = project.clone();
    let (names, size) = fs(move || {
        let mut names: Vec<String> = std::fs::read_dir(&p)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        (
            names,
            std::fs::metadata(p.join("Audio Files/Bass_1.wav"))
                .unwrap()
                .len(),
        )
    })
    .await;
    assert_eq!(names, ["Audio Files", "Right Here.RPP"]);
    assert_eq!(size, 3_000_000);
    assert_eq!(
        xattr(&project.join("Audio Files/Bass_1.wav"), drive::XATTR_STATE),
        "online"
    );

    // Read on demand: the bytes are the node's, and they stay.
    let p = project.clone();
    let read = fs(move || std::fs::read(p.join("Audio Files/Bass_1.wav")).unwrap()).await;
    assert_eq!(read, take);
    // The background fetch lands it whole a moment after the last read.
    let mut state = String::new();
    for _ in 0..50 {
        state = xattr(&project.join("Audio Files/Bass_1.wav"), drive::XATTR_STATE);
        if state == "cached" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(state, "cached");

    // A new file, and a DAW's atomic save: written to a temp, renamed over.
    let p = project.clone();
    fs(move || {
        std::fs::write(p.join("Notes.txt"), "from the mount").unwrap();
        std::fs::write(
            p.join(".Right Here.RPP.tmp"),
            "<REAPER_PROJECT\n  saved\n>\n",
        )
        .unwrap();
        std::fs::rename(p.join(".Right Here.RPP.tmp"), p.join("Right Here.RPP")).unwrap();
    })
    .await;
    let doc_id = laptop
        .engine
        .resolve_root("codywright/Projects/Right Here")
        .unwrap();
    let doc = laptop.docs.lock().unwrap().get(&doc_id).unwrap().clone();
    let live = Manifest::new(&doc).tree().live();
    assert!(live.contains_key("Notes.txt"));
    assert!(!live.contains_key(".Right Here.RPP.tmp"));
    exchange(&node, &laptop);
    node.engine.pass_all().await;
    assert_eq!(
        std::fs::read_to_string(held.join("Notes.txt")).unwrap(),
        "from the mount"
    );
    assert!(
        std::fs::read_to_string(held.join("Right Here.RPP"))
            .unwrap()
            .contains("saved")
    );

    // Pin by xattr: a working copy appears, and the mount passes through to it.
    let st = Command::new("setfattr")
        .args(["-n", drive::XATTR_PINNED, "-v", "1"])
        .arg(&project)
        .status()
        .unwrap();
    assert!(st.success());
    let copy = tmp
        .path()
        .join("laptop-roots/codywright/Projects/Right Here");
    for _ in 0..100 {
        if copy.join("Audio Files/Bass_1.wav").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(xattr(&project, drive::XATTR_PINNED), "1");
    let p = project.clone();
    fs(move || std::fs::write(p.join("Right Here.RPP"), "<REAPER_PROJECT\n  pinned\n>\n").unwrap())
        .await;
    assert!(
        std::fs::read_to_string(copy.join("Right Here.RPP"))
            .unwrap()
            .contains("pinned")
    );

    mounted.unmount();
    assert!(!drive::is_mounted(&mnt));
}
