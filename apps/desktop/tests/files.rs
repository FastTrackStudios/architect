//! Projects between two nodes over iroh: the always-on one holds a tree
//! in place (Task's layout, its `.fts-files` left alone), a laptop pins
//! one project, and edits on either side reach the other.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send,
    clippy::missing_const_for_fn
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use architect_accounts::Issuer;
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_desktop::node::{self, Config, IrohMode, Running};
use architect_files::sync::Engine;
use auth_server::{ServerConfig, server};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

async fn issuer() -> String {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    assert!(
        auth_server::dev::seed(&auth, &config.database_url)
            .await
            .unwrap()
    );
    let app = server::app_router_with_db(&config, auth, db).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await.unwrap() });
    origin
}

async fn session(origin: &str, dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let signed_in = Issuer::new(origin)
        .sign_in_password("ada@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    let file = dir.join("session");
    std::fs::write(&file, signed_in.session.token).unwrap();
    file
}

fn free_udp() -> String {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().to_string()
}

fn config(
    name: &str,
    dir: &Path,
    origin: &str,
    session: PathBuf,
    files: PathBuf,
    hold: bool,
    iroh_bind: &str,
) -> Config {
    Config {
        name: name.to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        issuer: Some(origin.to_owned()),
        session_file: Some(session),
        iroh: IrohMode::Local,
        iroh_bind: Some(iroh_bind.to_owned()),
        sweep_secs: 2,
        files_dir: Some(files),
        files_hold: hold,
        files_secs: 1,
        ..Config::default()
    }
}

async fn until(what: &str, mut done: impl AsyncFnMut() -> bool) {
    for _ in 0..600 {
        if done().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out: {what}");
}

fn engine(node: &Running) -> &Engine {
    node.files.as_ref().unwrap()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

struct Pair {
    always: Running,
    laptop: Running,
    held: PathBuf,
    doc_id: uuid::Uuid,
    _tmp: tempfile::TempDir,
}

/// The always-on node holding a project, a laptop that knows it.
async fn pair() -> Pair {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("starcommand");
    let held = tree.join("codywright/Projects/Right Here - Cody Wright");
    std::fs::create_dir_all(held.join("Right Here/Audio Files")).unwrap();
    std::fs::create_dir_all(held.join(".fts-files/store")).unwrap();
    std::fs::write(
        held.join("Right Here/Right Here.RPP"),
        "<REAPER_PROJECT\n>\n",
    )
    .unwrap();
    std::fs::write(
        held.join("Right Here/Audio Files/Bass_1.wav"),
        vec![1u8; 2_000_000],
    )
    .unwrap();

    let (always_bind, laptop_bind) = (free_udp(), free_udp());
    let always_dir = tmp.path().join("always-on");
    let always_session = session(&origin, &always_dir).await;
    let always_cfg = config(
        "always-on",
        &always_dir,
        &origin,
        always_session,
        tree.clone(),
        true,
        &always_bind,
    );
    let always = node::start(always_cfg.clone()).await.unwrap();
    let laptop_dir = tmp.path().join("laptop");
    let mut laptop_cfg = config(
        "laptop",
        &laptop_dir,
        &origin,
        session(&origin, &laptop_dir).await,
        laptop_dir.join("Architect"),
        false,
        &laptop_bind,
    );
    laptop_cfg.peers = vec![always.dial().unwrap()];
    let laptop = node::start(laptop_cfg).await.unwrap();
    // The always-on node dials the laptop too (with n0 it finds it by id).
    let laptop_dial = laptop.dial().unwrap();
    always.stop().await;
    let always = node::start(Config {
        peers: vec![laptop_dial],
        ..always_cfg
    })
    .await
    .unwrap();

    // The laptop learns the project from the always-on node's catalogue.
    let mut doc_id = None;
    until("the laptop knows the project", async || {
        doc_id = engine(&laptop)
            .status()
            .await
            .into_iter()
            .find(|r| r.name == "Right Here - Cody Wright" && r.files == 2)
            .map(|r| r.doc_id.parse().unwrap());
        doc_id.is_some()
    })
    .await;
    let doc_id = doc_id.unwrap();
    Pair {
        always,
        laptop,
        held,
        doc_id,
        _tmp: tmp,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_held_on_the_always_on_node_is_pinned_on_a_laptop_and_syncs_both_ways() {
    let Pair {
        always,
        laptop,
        held,
        doc_id,
        _tmp,
    } = pair().await;

    // Pinned: a full copy, Task's store not among it.
    let copy = engine(&laptop).pin(doc_id, None).await.unwrap();
    let session_file = copy.join("Right Here/Right Here.RPP");
    until("the laptop has the project", async || {
        std::fs::metadata(copy.join("Right Here/Audio Files/Bass_1.wav"))
            .is_ok_and(|m| m.len() == 2_000_000)
            && read(&session_file) == "<REAPER_PROJECT\n>\n"
    })
    .await;
    assert!(!copy.join(".fts-files").exists());

    // A save on the laptop reaches the always-on node's tree.
    std::fs::write(&session_file, "<REAPER_PROJECT\n  laptop\n>\n").unwrap();
    until("the always-on node has the laptop's save", async || {
        read(&held.join("Right Here/Right Here.RPP")).contains("laptop")
    })
    .await;
    // A new take on the always-on node reaches the laptop.
    std::fs::write(
        held.join("Right Here/Audio Files/Gtr_2.wav"),
        vec![2u8; 500_000],
    )
    .unwrap();
    until("the laptop has the new take", async || {
        std::fs::metadata(copy.join("Right Here/Audio Files/Gtr_2.wav"))
            .is_ok_and(|m| m.len() == 500_000)
    })
    .await;
    let names: Vec<String> = engine(&always)
        .checkpoints(doc_id)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(
        names.contains(&"Imported".to_owned())
            && names.contains(&"Saved Right Here.RPP".to_owned()),
        "{names:?}"
    );

    laptop.stop().await;
    always.stop().await;
}

fn dav(laptop: &Running, rel: &str) -> String {
    let path: Vec<String> = rel.split('/').map(|s| s.replace(' ', "%20")).collect();
    format!("http://{}/dav/{}", laptop.http, path.join("/"))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpinned_project_browses_over_webdav_and_a_file_opens_on_demand() {
    let Pair {
        always,
        laptop,
        held,
        _tmp,
        ..
    } = pair().await;
    let http = reqwest::Client::new();
    let project = "codywright/Projects/Right Here - Cody Wright";

    // Browsed from the manifest: nothing of it is on the laptop.
    let listing = http
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            dav(&laptop, &format!("{project}/Right Here/Audio Files")),
        )
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status().as_u16(), 207);
    let xml = listing.text().await.unwrap();
    assert!(
        xml.contains("Bass_1.wav") && xml.contains("<D:getcontentlength>2000000<"),
        "{xml}"
    );
    let top = http
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            dav(&laptop, "codywright/Projects"),
        )
        .header("depth", "1")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(top.contains("Right Here - Cody Wright"), "{top}");

    // Opened: fetched from the always-on node, by range too, and kept.
    let wav = dav(
        &laptop,
        &format!("{project}/Right Here/Audio Files/Bass_1.wav"),
    );
    let part = http
        .get(&wav)
        .header("range", "bytes=10-19")
        .send()
        .await
        .unwrap();
    assert_eq!(part.status().as_u16(), 206);
    assert_eq!(part.bytes().await.unwrap().as_ref(), &[1u8; 10]);
    let whole = http.get(&wav).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(whole.len(), 2_000_000);
    let entries: serde_json::Value = http
        .post(format!("http://{}/files/browse", laptop.http))
        .json(&serde_json::json!({"root": project, "path": "Right Here/Audio Files"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(entries[0]["here"], serde_json::json!(true), "{entries}");

    // Written over WebDAV, unpinned: a version, in the always-on node's tree.
    let put = http
        .put(dav(&laptop, &format!("{project}/Right Here/Notes.txt")))
        .body("written over WebDAV")
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 201);
    until("the always-on node has the WebDAV write", async || {
        read(&held.join("Right Here/Notes.txt")) == "written over WebDAV"
    })
    .await;

    // Pinned and unpinned through the agent's HTTP face.
    let pinned: serde_json::Value = http
        .post(format!("http://{}/files/pin", laptop.http))
        .json(&serde_json::json!({"root": project, "path": null}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let copy = PathBuf::from(pinned.as_str().unwrap_or_else(|| panic!("{pinned}")));
    until("the pinned copy is whole", async || {
        read(&copy.join("Right Here/Notes.txt")) == "written over WebDAV"
            && std::fs::metadata(copy.join("Right Here/Audio Files/Bass_1.wav"))
                .is_ok_and(|m| m.len() == 2_000_000)
    })
    .await;
    let unpinned = http
        .post(format!("http://{}/files/unpin", laptop.http))
        .json(&serde_json::json!({"root": project}))
        .send()
        .await
        .unwrap();
    assert!(
        unpinned.status().is_success(),
        "{}",
        unpinned.text().await.unwrap()
    );
    assert!(!copy.join("Right Here/Notes.txt").exists());

    laptop.stop().await;
    always.stop().await;
}
