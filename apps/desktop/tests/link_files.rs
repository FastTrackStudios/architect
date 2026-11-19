//! A share link's files from wherever they are held (#490): a node that
//! answers a link without holding its recordings streams them from the
//! storage server over the nodes' own lane, or sends the browser to the
//! bucket; a link carries a project's sessions and exactly the files
//! they reference.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send,
    clippy::missing_const_for_fn,
    clippy::too_many_lines
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use architect_accounts::Issuer;
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_desktop::node::{self, Config, IrohMode, Running, Size};
use architect_spaces::{
    Audience, Download, GuestProjects, Listen, NewShare, ProjectAccess, Share, ShareAccess,
    ShareChange, ShareLinksClient, ShareProject, ShareScope,
};
use auth_server::{ServerConfig, server};
use reqwest::StatusCode;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

const AK: &str = "AKIATESTTESTTESTTEST";
const SK: &str = "testsecrettestsecrettestsecrettestsecret";

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

/// A session of `email`'s in a file under `dir`; the file and the token.
async fn session(origin: &str, dir: &Path, email: &str) -> (PathBuf, String) {
    std::fs::create_dir_all(dir).unwrap();
    let signed_in = Issuer::new(origin)
        .sign_in_password(email, auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    let file = dir.join(format!("session-{email}"));
    std::fs::write(&file, &signed_in.session.token).unwrap();
    (file, signed_in.session.token)
}

/// An S3 server in process, with one bucket.
async fn bucket(root: &Path) -> String {
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
    format!("http://{addr}")
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

/// Bytes that differ along the file, so a range is told from another.
fn audio(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| seed.wrapping_add(u8::try_from((i / 251) % 251).unwrap()))
        .collect()
}

/// A large file, each different from the next, written a block at a time.
fn big(path: &Path, seed: usize, len: usize) {
    use std::io::Write as _;
    let block: Vec<u8> = (0..1_usize << 20)
        .map(|i| u8::try_from((i.wrapping_mul(31).wrapping_add(seed)) % 251).unwrap())
        .collect();
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut left = len;
    while left > 0 {
        let take = left.min(block.len());
        file.write_all(&block[..take]).unwrap();
        left -= take;
    }
    file.flush().unwrap();
}

/// The storage server (everything, in place) and the hosted node (the
/// org's lane and links, the light part only), on one account.
struct World {
    storage: Running,
    hosted: Running,
    /// The storage server's vault and file tree; the hosted node's vault.
    vault: PathBuf,
    tree: PathBuf,
    hosted_vault: PathBuf,
    hosted_dir: PathBuf,
    /// Ada's session (the org's owner) and Grace's (an admin).
    ada: String,
    grace: String,
    _tmp: tempfile::TempDir,
}

/// What a world is made with.
#[derive(Default)]
struct Setup {
    /// An S3 bucket both nodes use.
    bucket: bool,
    /// How large the multitracks are, and how many extra ones.
    track_len: usize,
    tracks: usize,
    /// Multitracks in the vault, linked from the song's note (the load
    /// measurement), and whether the hosted node holds them too.
    vault_tracks: usize,
    hosted_holds: bool,
}

const PROJECT: &str = "acme/Projects/Album";

/// A session document that names `references`, as an Architect document.
async fn session_doc(references: &[&str]) -> Vec<u8> {
    let kind = crdt::DocKind::new("fts.session", "Session")
        .extension("fts-session")
        .schema(1, Vec::new());
    let memory = crdt::file::MemoryStorage::new();
    let file = crdt::file::DocFile::create(memory.clone(), &kind)
        .await
        .unwrap();
    let mut main = file.open_branch(crdt::file::MAIN).await.unwrap();
    main.transact("items", |doc| {
        let items = doc.get_map("items");
        for (n, reference) in references.iter().enumerate() {
            let item = items
                .insert_container(&format!("item-{n}"), crdt::loro::LoroMap::new())
                .unwrap();
            item.insert("kind", "audio").unwrap();
            item.insert("payload", *reference).unwrap();
        }
        doc.get_map("meta").insert("name", "Joy").unwrap();
    })
    .await
    .unwrap();
    memory.bytes()
}

async fn world(setup: Setup) -> World {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let len = if setup.track_len == 0 { 300_000 } else { setup.track_len };
    let endpoint = if setup.bucket {
        Some(bucket(&tmp.path().join("s3")).await)
    } else {
        None
    };
    let secret = tmp.path().join("s3-secret");
    std::fs::write(&secret, SK).unwrap();

    // The storage server: the vault with its recordings, the project.
    let storage_dir = tmp.path().join("storage");
    let vault = storage_dir.join("data/vaults/setlists");
    std::fs::create_dir_all(vault.join("Songs/Joy/Media")).unwrap();
    std::fs::write(
        vault.join("Songs/Joy/Joy.md"),
        "---\ntype: song\nmedia: reference.mp3\n---\n# Joy\n\n![[cover.png]]\n\n[[kick.wav]]\n",
    )
    .unwrap();
    std::fs::write(vault.join("Songs/Joy/cover.png"), audio(1, 3_000)).unwrap();
    std::fs::write(vault.join("Songs/Joy/reference.mp3"), audio(2, 40_000)).unwrap();
    std::fs::write(vault.join("Songs/Joy/Media/kick.wav"), audio(3, len)).unwrap();
    if setup.vault_tracks > 0 {
        let mut note = std::fs::read_to_string(vault.join("Songs/Joy/Joy.md")).unwrap();
        for n in 0..setup.vault_tracks {
            big(&vault.join(format!("Songs/Joy/Media/track-{n:03}.wav")), n, len);
            note.push_str(&format!("\n[[track-{n:03}.wav]]\n"));
        }
        std::fs::write(vault.join("Songs/Joy/Joy.md"), note).unwrap();
    }
    let tree = storage_dir.join("tree");
    let project = tree.join(PROJECT);
    std::fs::create_dir_all(project.join("songs/joy/Samples")).unwrap();
    std::fs::create_dir_all(project.join("songs/other")).unwrap();
    std::fs::write(project.join("songs/joy/Samples/bass.wav"), audio(4, len)).unwrap();
    std::fs::write(project.join("songs/joy/Samples/unused.wav"), audio(5, len)).unwrap();
    std::fs::write(project.join("songs/joy/notes.txt"), "private").unwrap();
    let mut references = vec!["Samples/bass.wav".to_owned(), "Samples/missing.wav".to_owned()];
    for n in 0..setup.tracks {
        let name = format!("Samples/track-{n:03}.wav");
        std::fs::write(
            project.join("songs/joy").join(&name),
            audio(u8::try_from(n % 200).unwrap().wrapping_add(10), len),
        )
        .unwrap();
        references.push(name);
    }
    let named: Vec<&str> = references.iter().map(String::as_str).collect();
    std::fs::write(
        project.join("songs/joy/Joy.fts-session"),
        session_doc(&named).await,
    )
    .unwrap();
    std::fs::write(
        project.join("songs/other/Other.fts-session"),
        session_doc(&["../joy/Samples/unused.wav"]).await,
    )
    .unwrap();

    let free_udp = || {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .to_string()
    };
    let base = |name: &str, dir: &Path, session: PathBuf, files: PathBuf| Config {
        name: name.to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        issuer: Some(origin.clone()),
        session_file: Some(session),
        iroh: IrohMode::Local,
        iroh_bind: Some(free_udp()),
        sweep_secs: 1,
        vaults: [("setlists".to_owned(), PathBuf::from("vaults/setlists"))].into(),
        vault_files: true,
        vault_orgs: [("setlists".to_owned(), "acme".to_owned())].into(),
        files_dir: Some(files),
        files_hold: true,
        files_secs: 1,
        files_store_max: Some(Size::Bytes(8 << 30)),
        files_keep_free: Some(Size::Bytes(0)),
        blob_s3_bucket: endpoint.as_ref().map(|_| "blobs".to_owned()),
        blob_s3_endpoint: endpoint.clone(),
        blob_s3_region: "us-east-1".to_owned(),
        blob_s3_access_key_id: endpoint.as_ref().map(|_| AK.to_owned()),
        blob_s3_secret_file: endpoint.as_ref().map(|_| secret.clone()),
        ..Config::default()
    };
    let (storage_session, _) = session(&origin, &storage_dir, "ada@local.test").await;
    let storage_cfg = base("storage", &storage_dir, storage_session, tree.clone());
    let storage = node::start(storage_cfg.clone()).await.unwrap();

    let hosted_dir = tmp.path().join("hosted");
    let (hosted_session, ada) = session(&origin, &hosted_dir, "ada@local.test").await;
    let mut hosted_cfg = base(
        "hosted",
        &hosted_dir,
        hosted_session,
        hosted_dir.join("files"),
    );
    hosted_cfg.host_org = Some("acme".to_owned());
    hosted_cfg.peers = vec![storage.dial().unwrap()];
    // The hosted node's rule: never a multitrack.
    if !setup.hosted_holds {
        hosted_cfg.files_skip_paths = vec!["**/Media/**".to_owned(), "**/Samples/**".to_owned()];
    }
    let hosted = node::start(hosted_cfg).await.unwrap();
    // The storage server dials the hosted node too (with n0 it finds it).
    let hosted_dial = hosted.dial().unwrap();
    storage.stop().await;
    let storage = node::start(Config {
        peers: vec![hosted_dial],
        ..storage_cfg
    })
    .await
    .unwrap();
    let (_, grace) = session(&origin, &tmp.path().join("grace"), "grace@local.test").await;
    let hosted_vault = hosted_dir.join("data/vaults/setlists");
    World {
        storage,
        hosted,
        vault,
        tree,
        hosted_vault,
        hosted_dir,
        ada,
        grace,
        _tmp: tmp,
    }
}

impl World {
    async fn links(&self, token: &str) -> ShareLinksClient {
        let at =
            architect::connect::Endpoint::url(format!("ws://{}/org/acme/vox", self.hosted.http))
                .with_bearer(Some(token.to_owned()));
        architect::connect::Pool::new().client(&at).await.unwrap()
    }

    async fn share(&self, listen: Listen) -> Share {
        self.links(&self.ada)
            .await
            .create(NewShare {
                vault: "setlists".to_owned(),
                scope: ShareScope::Vault,
                title: None,
                audience: Audience::Link,
                access: ShareAccess::Read,
                expires_at: None,
                domain: None,
                listen,
            })
            .await
            .unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.hosted.http)
    }

    /// Until the hosted node's link names `file` (a vault path).
    async fn listed(&self, share: &Share, file: &str) -> serde_json::Value {
        let mut found = serde_json::Value::Null;
        until(&format!("the link lists {file}"), async || {
            let json: serde_json::Value = reqwest::get(self.url(&format!("/s/{}/_files.json", share.code)))
                .await
                .unwrap()
                .json()
                .await
                .unwrap_or_default();
            let hit = json["files"]
                .as_array()
                .and_then(|files| files.iter().find(|f| f["path"] == file).cloned());
            if let Some(hit) = hit {
                found = hit;
                true
            } else {
                false
            }
        })
        .await;
        found
    }

    async fn projects(&self, share: &Share, query: &str) -> GuestProjects {
        let text = reqwest::get(self.url(&format!("/s/{}/_project.json{query}", share.code)))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        facet_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// GET `url`, a range of it when asked: the status, headers and bytes.
async fn get(
    url: &str,
    range: Option<(usize, usize)>,
) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
    let mut request = client().get(url);
    if let Some((first, last)) = range {
        request = request.header("range", format!("bytes={first}-{last}"));
    }
    let response = request.send().await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    (status, headers, response.bytes().await.unwrap().to_vec())
}

fn from(headers: &reqwest::header::HeaderMap) -> &str {
    headers
        .get("x-architect-from")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

/// #490: a recording the hosted node's rule leaves on the storage server
/// is on the link all the same, streamed from there by range, and never
/// lands on the hosted node; nothing of the vault is copied per build.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_streams_a_recording_its_node_does_not_hold_from_the_storage_server() {
    let w = world(Setup::default()).await;
    let share = w.share(Listen::Full).await;
    let base = format!("/s/{}", share.code);

    // The link names the multitrack with its size and hash, though the
    // hosted node holds none of it.
    let kick = w.listed(&share, "Songs/Joy/Media/kick.wav").await;
    assert_eq!(kick["size"], 300_000);
    assert_eq!(kick["sha256"].as_str().map(str::len), Some(64));
    assert!(!w.hosted_vault.join("Songs/Joy/Media/kick.wav").exists());
    let whole = std::fs::read(w.vault.join("Songs/Joy/Media/kick.wav")).unwrap();

    // A range from the middle, the start and the whole of it.
    let url = w.url(&format!("{base}/Songs/Joy/Media/kick.wav"));
    // (Once the link is built with it: a moment after it is listed.)
    until("the link serves the recording", async || {
        get(&url, Some((0, 0))).await.0 == StatusCode::PARTIAL_CONTENT
    })
    .await;
    let (status, headers, bytes) = get(&url, Some((100_000, 100_999))).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(from(&headers), "peer");
    assert_eq!(headers["content-range"], "bytes 100000-100999/300000");
    assert_eq!(headers["content-type"], "audio/wav");
    assert_eq!(bytes, whole[100_000..=100_999]);
    let (status, _, bytes) = get(&url, Some((0, 15))).await;
    assert_eq!((status, bytes.as_slice()), (StatusCode::PARTIAL_CONTENT, &whole[..16]));
    let (status, _, bytes) = get(&url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, whole);
    // Past its end: not satisfiable, as a file on disk answers.
    let (status, _, _) = get(&url, Some((300_000, 300_010))).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);

    // Relayed, never kept: the rule holds on the hosted node.
    assert!(!w.hosted_vault.join("Songs/Joy/Media/kick.wav").exists());
    let kept = |dir: &Path| {
        let mut big = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    stack.push(entry.path());
                } else if meta.len() >= 300_000 {
                    big.push(entry.path());
                }
            }
        }
        big
    };
    assert_eq!(kept(&w.hosted_dir), Vec::<PathBuf>::new());

    // The light files it holds are read from its own vault, and none is
    // copied into the link's build.
    until("the cover has synced to the hosted node", async || {
        std::fs::metadata(w.hosted_vault.join("Songs/Joy/cover.png")).is_ok_and(|m| m.len() == 3_000)
    })
    .await;
    let (status, headers, bytes) = get(&w.url(&format!("{base}/Songs/Joy/cover.png")), None).await;
    assert_eq!((status, bytes.len()), (StatusCode::OK, 3_000));
    assert_eq!(from(&headers), "");
    let built = w.hosted_dir.join("data/shares").join(&share.code).join("site");
    assert!(built.join("index.html").is_file() || built.join("songs").is_dir(), "built");
    assert!(!built.join("Songs/Joy/cover.png").exists());
    assert!(!built.join("Songs/Joy/Media/kick.wav").exists());

    // A file the link does not name is nowhere, held or not.
    for nowhere in ["Songs/Joy/Media/snare.wav", "Songs/Joy/Media/../../../../etc/passwd"] {
        let (status, _, _) = get(&w.url(&format!("{base}/{nowhere}")), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{nowhere}");
    }

    // The storage server gone: the link says so, and holds nobody up.
    w.storage.stop().await;
    let started = Instant::now();
    let (status, _, _) = get(&url, Some((0, 15))).await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT),
        "{status}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    w.hosted.stop().await;
}

/// Where the bytes are in the bucket, the link sends the browser there:
/// a presigned read of that one object, for ten minutes. None pass
/// through the node; the URL reads nothing else and writes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_sends_the_browser_to_the_bucket_for_a_file_that_is_there() {
    let w = world(Setup {
        bucket: true,
        ..Setup::default()
    })
    .await;
    let share = w.share(Listen::Full).await;
    let kick = w.listed(&share, "Songs/Joy/Media/kick.wav").await;
    let sha = kick["sha256"].as_str().unwrap().to_owned();
    let url = w.url(&format!("/s/{}/Songs/Joy/Media/kick.wav", share.code));
    let whole = std::fs::read(w.vault.join("Songs/Joy/Media/kick.wav")).unwrap();

    // Once the storage server has copied it to the bucket.
    let mut location = String::new();
    until("the link redirects to the bucket", async || {
        let (status, headers, _) = get(&url, Some((0, 15))).await;
        if status == StatusCode::FOUND && from(&headers) == "bucket" {
            location = headers["location"].to_str().unwrap().to_owned();
            true
        } else {
            false
        }
    })
    .await;
    assert!(location.contains(&sha), "the one object, by its hash: {location}");
    assert!(location.contains("X-Amz-Expires=600"), "{location}");
    assert!(location.contains("X-Amz-Signature="), "{location}");
    assert!(!location.contains(SK), "never the secret");

    // The browser reads the range from the bucket itself.
    let (status, headers, bytes) = get(&location, Some((200_000, 200_099))).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT, "{headers:?}");
    assert_eq!(bytes, whole[200_000..=200_099]);
    // The URL is that object's alone: another object, a listing or a
    // write with its signature is refused.
    let other = w.listed(&share, "Songs/Joy/reference.mp3").await;
    let other_sha = other["sha256"].as_str().unwrap();
    let forged = location.replace(&sha, other_sha).replace(&sha[..2], &other_sha[..2]);
    assert_ne!(forged, location);
    let (status, _, _) = get(&forged, None).await;
    assert!(status.is_client_error(), "another object: {status}");
    let listing = location.split('?').next().unwrap().rsplit_once("/sha256/").unwrap().0;
    let query = location.split_once('?').unwrap().1;
    let (status, _, _) = get(&format!("{listing}/?{query}"), None).await;
    assert!(status.is_client_error(), "a listing: {status}");
    let put = client().put(&location).body("x").send().await.unwrap();
    assert!(put.status().is_client_error(), "a write: {}", put.status());

    // The link is what a guest keeps: revoked, it redirects nowhere.
    w.links(&w.ada).await.revoke(share.code.clone()).await.unwrap();
    let (status, _, _) = get(&url, Some((0, 15))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    w.storage.stop().await;
    w.hosted.stop().await;
}

/// A link carries a project: its shared session documents and exactly
/// the files they reference, by hash, from the storage server; nothing
/// else of the root by any name; how far it plays, what it offers to
/// keep and who may save a session back are the link's permissions, and
/// its URL never moves.
#[tokio::test(flavor = "multi_thread")]
async fn a_project_link_carries_its_sessions_and_exactly_the_files_they_reference() {
    let w = world(Setup::default()).await;
    let share = w.share(Listen::Full).await;
    let links = w.links(&w.ada).await;
    let project = |sessions: &[&str], access: ProjectAccess| ShareProject {
        root: PROJECT.to_owned(),
        sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
        access,
    };
    let joy = "songs/joy/Joy.fts-session";
    let carried = links
        .update(
            share.code.clone(),
            ShareChange::Projects(vec![project(&[joy], ProjectAccess::Read)]),
        )
        .await
        .unwrap();
    assert_eq!(carried.code, share.code, "the same link, now with a project");
    // Not a project's root, or not a session: refused.
    for bad in [
        ShareProject { root: "acme/Assets/Tracks".to_owned(), ..project(&[], ProjectAccess::Read) },
        project(&["songs/joy/Samples/bass.wav"], ProjectAccess::Read),
    ] {
        assert!(
            links.update(share.code.clone(), ShareChange::Projects(vec![bad])).await.is_err()
        );
    }

    // Once the hosted node knows the root (from the storage server's
    // catalogue): the session, what it references, and nothing else.
    let mut found = None;
    until("the link names the session's files", async || {
        let listed = w.projects(&share, "").await;
        let ready = listed.projects.first().is_some_and(|p| !p.files.is_empty());
        found = Some(listed);
        ready
    })
    .await;
    let listed = found.unwrap();
    assert_eq!((listed.listen.as_str(), listed.max.as_str()), ("full", "full"));
    let album = &listed.projects[0];
    assert_eq!((album.root.as_str(), album.name.as_str()), (PROJECT, "Album"));
    assert_eq!(album.access, "read");
    let sessions: Vec<&str> = album.sessions.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(sessions, [joy]);
    let files: Vec<&str> = album.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(files, ["songs/joy/Samples/bass.wav"], "what the session names, no more");
    let session = &album.sessions[0];
    assert_eq!(session.save, None, "read-only");
    let named: Vec<(&str, &str)> = session
        .references
        .iter()
        .map(|r| (r.named.as_str(), r.sha256.as_str()))
        .collect();
    let bass = &album.files[0];
    assert_eq!(named, [("Samples/bass.wav", bass.sha256.as_str())]);
    assert_eq!(bass.size, 300_000);
    assert!(!bass.stream_only);

    // Its bytes, by range, from the storage server; the session whole.
    let whole = std::fs::read(w.tree.join(PROJECT).join("songs/joy/Samples/bass.wav")).unwrap();
    let (status, headers, bytes) = get(&w.url(&bass.url), Some((250_000, 250_063))).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(from(&headers), "peer");
    assert_eq!(headers["content-type"], "audio/wav");
    assert_eq!(bytes, whole[250_000..=250_063]);
    let on_disk = std::fs::read(w.tree.join(PROJECT).join(joy)).unwrap();
    let (status, _, bytes) = get(&w.url(&session.url), None).await;
    assert_eq!((status, bytes), (StatusCode::OK, on_disk));

    // Nothing else of the root, by any name: a file no shared session
    // names, a session that is not shared, a text file beside them.
    let hash_of = |rel: &str| {
        architect::blob::Sha256::of(&std::fs::read(w.tree.join(PROJECT).join(rel)).unwrap())
            .to_hex()
    };
    let blob = |sha: &str, name: &str| w.url(&format!("/s/{}/_blob/{sha}/{name}", share.code));
    for (rel, name) in [
        ("songs/joy/Samples/unused.wav", "unused.wav"),
        ("songs/other/Other.fts-session", "Other.fts-session"),
        ("songs/joy/notes.txt", "notes.txt"),
    ] {
        let (status, _, body) = get(&blob(&hash_of(rel), name), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{rel}");
        assert!(body.len() < 200, "{rel}: no bytes of it");
    }
    // Nor by the name of one it does serve, or a path in the root.
    let (status, _, _) = get(&blob(&hash_of("songs/joy/Samples/unused.wav"), "bass.wav"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for path in ["_blob/", "_blob/songs/joy/Samples/unused.wav", "_project/0/songs/joy/notes.txt"] {
        let (status, _, _) = get(&w.url(&format!("/s/{}/{path}", share.code)), None).await;
        assert!(status.is_client_error(), "{path}: {status}");
    }

    // How far the link plays holds here too: below Full a guest is told
    // of no file, and none is served; the session is the Reference layer.
    let reference = w.projects(&share, "?listen=reference").await;
    assert_eq!(reference.listen, "reference");
    assert_eq!(reference.projects[0].sessions.len(), 1);
    assert!(reference.projects[0].files.is_empty());
    assert!(reference.projects[0].sessions[0].references.is_empty());
    assert!(w.projects(&share, "?listen=page").await.projects[0].sessions.is_empty());
    links.update(share.code.clone(), ShareChange::Listen(Listen::Reference)).await.unwrap();
    let lowered = w.projects(&share, "").await;
    assert_eq!(lowered.max, "reference");
    assert!(lowered.projects[0].files.is_empty());
    let (status, _, _) = get(&w.url(&bass.url), Some((0, 15))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a stem on a link lowered to Reference");
    assert_eq!(get(&w.url(&session.url), None).await.0, StatusCode::OK);
    links.update(share.code.clone(), ShareChange::Listen(Listen::Full)).await.unwrap();

    // Played, not offered: the multitrack is marked and sent inline.
    links.update(share.code.clone(), ShareChange::Download(Download::NotMedia)).await.unwrap();
    let playing = w.projects(&share, "").await;
    assert!(playing.projects[0].files[0].stream_only);
    let (status, headers, _) = get(&w.url(&bass.url), Some((0, 15))).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-disposition"], "inline");

    // Read-only: nobody saves a session through the link.
    let save = w.url(&format!("/s/{}/_project/0/{joy}", share.code));
    let edited = session_doc(&["Samples/bass.wav", "Samples/unused.wav"]).await;
    let put = |token: Option<&str>, to: &str, body: Vec<u8>| {
        let mut request = client().put(to).body(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        async move { request.send().await.unwrap().status() }
    };
    assert_eq!(put(Some(&w.grace), &save, edited.clone()).await, StatusCode::FORBIDDEN);

    // With edit: a signed-in person the link lets in saves it; nobody
    // anonymous does, and only a session the link shares.
    links
        .update(
            share.code.clone(),
            ShareChange::Projects(vec![project(&[joy], ProjectAccess::Edit)]),
        )
        .await
        .unwrap();
    let editable = w.projects(&share, "").await;
    assert_eq!(editable.projects[0].access, "edit");
    assert_eq!(
        editable.projects[0].sessions[0].save.as_deref(),
        Some(format!("/s/{}/_project/0/songs/joy/Joy.fts-session", share.code).as_str())
    );
    assert_eq!(put(None, &save, edited.clone()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(put(Some("not-a-session"), &save, edited.clone()).await, StatusCode::UNAUTHORIZED);
    let other = w.url(&format!("/s/{}/_project/0/songs/other/Other.fts-session", share.code));
    assert_eq!(put(Some(&w.grace), &other, edited.clone()).await, StatusCode::NOT_FOUND);
    let stem = w.url(&format!("/s/{}/_project/0/songs/joy/Samples/bass.wav", share.code));
    assert_eq!(put(Some(&w.grace), &stem, edited.clone()).await, StatusCode::NOT_FOUND);
    assert_eq!(put(Some(&w.grace), &save, edited.clone()).await, StatusCode::OK);

    // The saved session is the project's now, on the storage server too,
    // and what it references is what the link serves: the URL the same.
    until("the storage server has the saved session", async || {
        std::fs::read(w.tree.join(PROJECT).join(joy)).is_ok_and(|b| b == edited)
    })
    .await;
    until("the link serves what the session now names", async || {
        w.projects(&share, "").await.projects[0].files.len() == 2
    })
    .await;
    let after = w.projects(&share, "").await;
    let files: Vec<&str> = after.projects[0].files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(files, ["songs/joy/Samples/bass.wav", "songs/joy/Samples/unused.wav"]);
    assert_ne!(after.projects[0].sessions[0].sha256, session.sha256);
    let (status, _, _) = get(&blob(&hash_of("songs/joy/Samples/unused.wav"), "unused.wav"), Some((0, 9))).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT, "named now, so served now");
    assert_eq!(links.shares().await.unwrap().len(), 1);
    assert_eq!(links.shares().await.unwrap()[0].code, share.code);

    // No session named: every session document of the root.
    links
        .update(share.code.clone(), ShareChange::Projects(vec![project(&[], ProjectAccess::Read)]))
        .await
        .unwrap();
    let all = w.projects(&share, "").await;
    let sessions: Vec<&str> = all.projects[0].sessions.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(sessions, [joy, "songs/other/Other.fts-session"]);
    // An audience's link is a slice of the notes: it carries no project.
    w.storage.stop().await;
    w.hosted.stop().await;
}

/// The load measurement (#490): a song of many large tracks, opened by a
/// client that wants the start of every track at once. Time to the first
/// 256 KiB of each of N tracks, all asked together, through each path:
/// the answering node's own disk, a relay from the storage server, and a
/// redirect to the bucket. Run by hand:
///
/// ```sh
/// LINK_BENCH_TRACKS=60 LINK_BENCH_MB=64 cargo nextest run -p architect-desktop \
///   --no-default-features --features vox --test link_files --run-ignored only \
///   -E 'test(time_to_first_audio)' --no-capture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement: gigabytes of files, run by hand"]
async fn time_to_first_audio_through_each_path() {
    let env = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let tracks = env("LINK_BENCH_TRACKS", 60);
    let len = env("LINK_BENCH_MB", 64) << 20;
    const FIRST: usize = 256 << 10;
    let mut report = format!(
        "\n{tracks} tracks of {} MiB; the first {} KiB of each, all asked at once\n",
        len >> 20,
        FIRST >> 10
    );
    for (path, bucket, hosted_holds) in [
        ("here", false, true),
        ("peer", false, false),
        ("bucket", true, false),
    ] {
        let w = world(Setup {
            bucket,
            track_len: len,
            vault_tracks: tracks,
            hosted_holds,
            ..Setup::default()
        })
        .await;
        eprintln!("phase {path}: world up");
        let share = w.share(Listen::Full).await;
        let last = format!("Songs/Joy/Media/track-{:03}.wav", tracks - 1);
        w.listed(&share, &last).await;
        let url = |n: usize| {
            w.url(&format!(
                "/s/{}/Songs/Joy/Media/track-{:03}.wav",
                share.code,
                n % tracks
            ))
        };
        // Ready when every track answers through the path measured.
        let following = reqwest::Client::new();
        for _ in 0..3000 {
            let mut ready = 0;
            for n in 0..tracks {
                let (status, headers, _) = get(&url(n), Some((0, 0))).await;
                let via = from(&headers).to_owned();
                let ok = match path {
                    "here" => status == StatusCode::PARTIAL_CONTENT && via.is_empty(),
                    "peer" => status == StatusCode::PARTIAL_CONTENT && via == "peer",
                    _ => status == StatusCode::FOUND && via == "bucket",
                };
                ready += usize::from(ok);
            }
            eprintln!("phase {path}: {ready}/{tracks} ready");
            if ready == tracks {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        for asked in [tracks, 200] {
            for (what, offset) in [("start", 0), ("middle", len / 2)] {
                let began = Instant::now();
                let mut waits = Vec::new();
                for n in 0..asked {
                    let (client, url) = (following.clone(), url(n));
                    waits.push(tokio::spawn(async move {
                        let at = Instant::now();
                        let response = client
                            .get(url)
                            .header("range", format!("bytes={offset}-{}", offset + FIRST - 1))
                            .send()
                            .await
                            .unwrap();
                        let status = response.status();
                        let got = response.bytes().await.map(|b| b.len()).unwrap_or(0);
                        (status, got, at.elapsed())
                    }));
                }
                let mut times = Vec::new();
                let mut failed = 0;
                let mut why = std::collections::BTreeMap::<String, usize>::new();
                for wait in waits {
                    let (status, got, took) = wait.await.unwrap();
                    if status == StatusCode::PARTIAL_CONTENT && got == FIRST {
                        times.push(took);
                    } else {
                        failed += 1;
                        *why.entry(format!("{status} {got}")).or_default() += 1;
                    }
                }
                if failed > 0 {
                    report.push_str(&format!("        failures: {why:?}\n"));
                }
                times.sort();
                let ms = |d: Option<&Duration>| d.map_or(0, |d| d.as_millis());
                report.push_str(&format!(
                    "{path:>6}  {asked:>3} tracks  {what:<6}  all in {:>5} ms   median {:>5} ms   slowest {:>5} ms   failed {failed}\n",
                    began.elapsed().as_millis(),
                    ms(times.get(times.len() / 2)),
                    ms(times.last()),
                ));
            }
        }
        w.storage.stop().await;
        w.hosted.stop().await;
    }
    println!("{report}");
}
