//! The HTTP byte face, over a real socket: a browser reads a file by
//! range with a session or a read ticket (as a `<video>` does), a
//! rendition likewise, and uploads in resumable `PUT`s — every request's
//! caller resolved by the gate's own identity resolver, the org's
//! membership required and each path decided by role and grant, as on
//! the org's vox lane.

#![cfg(feature = "http")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::significant_drop_tightening
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use architect::Layer as _;
use architect::blob::{LocalFs, Sha256};
use architect::layer::{LayerRouter, handler_acceptor};
use architect::local::RawLaneCaller;
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_files::server::{FilesServer, TranscodeError, Transcoder, WebFiles};
use architect_files::web::{self, FilesWebHttpClient};
use architect_files::{
    FilesClient, MediaClass, OnConflict, PERMITS, RenditionKind, RenditionSet, RootId, RootPath,
    STREAM_PERMITS, Save, UploadSpec, content_of, files_service_rpc_service_descriptor,
    files_service_stream_service_descriptor, serve, stream_layer,
};
use architect_orgs::server::{
    BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver, confine,
};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{BoxIdentityFuture, IdentityResolver, Principal};
use futures_util::future::BoxFuture;

/// alice owns `fts`; bob views it; carol is a member; erin is in `hp`.
struct Identity;

impl Directory for Identity {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            Ok(match token {
                "alice" => vec![Org::new("fts", "FastTrackStudio", Role::Owner)],
                "bob" => vec![Org::new("fts", "FastTrackStudio", Role::Viewer)],
                "carol" => vec![Org::new("fts", "FastTrackStudio", Role::Member)],
                "erin" => vec![Org::new("hp", "Harbor & Pine", Role::Member)],
                _ => Vec::new(),
            })
        })
    }
}

/// Sessions: the token is the user id.
struct Sessions;

impl IdentityResolver for Sessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token.map_or(Principal::Anonymous, |t| Principal::User {
            user_id: t.to_owned(),
        });
        Box::pin(async move { who })
    }
}

/// Classifies by a prefix and renders the kind's name and the source's
/// hash.
struct Fake;

impl Transcoder for Fake {
    fn probe<'a>(&'a self, source: &'a Path) -> BoxFuture<'a, Result<MediaClass, TranscodeError>> {
        Box::pin(async move {
            let bytes = tokio::fs::read(source).await.unwrap();
            Ok(if bytes.starts_with(b"VIDEO") {
                MediaClass::Video
            } else {
                MediaClass::Other
            })
        })
    }

    fn render<'a>(
        &'a self,
        kind: RenditionKind,
        source: &'a Path,
        out: &'a Path,
    ) -> BoxFuture<'a, Result<(), TranscodeError>> {
        Box::pin(async move {
            let bytes = tokio::fs::read(source).await.unwrap();
            let made = format!("{kind}:{}", Sha256::of(&bytes).to_hex());
            tokio::fs::write(out, made).await.unwrap();
            Ok(())
        })
    }
}

struct World {
    _dir: tempfile::TempDir,
    members: Members,
    gate: Arc<PermissionsGate>,
    router: LayerRouter,
    base: String,
    http: reqwest::Client,
}

async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let members = Members::new();
    let files = FilesServer::new(
        dir.path(),
        LocalFs::new(dir.path().join("blobs")),
        members.clone(),
    )
    .with_renditions(Fake);
    let identity: Arc<dyn IdentityResolver> =
        Arc::new(MembersResolver::new(Sessions, Identity, members.clone()));
    let gate = PermissionsGate::new(
        Arc::new(ConfinedEngine::new(members.clone())),
        Arc::clone(&identity),
    )
    .unlisted(UnlistedPolicy::Deny)
    .permit(files_service_rpc_service_descriptor(), &PERMITS)
    .permit(files_service_stream_service_descriptor(), &STREAM_PERMITS);
    let router = LayerRouter::new()
        .with(files_service_rpc_service_descriptor(), serve(files.clone()))
        .merge(stream_layer(files.clone()));

    // The byte face, resolving callers with the gate's own resolver.
    let face = WebFiles::new(files, identity);
    let app = architect::layers![web::Service].provide_http(&face);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    World {
        _dir: dir,
        members,
        gate: Arc::new(gate),
        router,
        base: format!("http://{addr}"),
        http: reqwest::Client::new(),
    }
}

/// `who` on `org`'s vox lane.
async fn connect(world: &World, org: &str, who: &str) -> FilesClient {
    let handler = confine(
        OrgId::new(org),
        PermissionsGate::wrap_shared_with_bearer(
            Arc::clone(&world.gate),
            world.router.clone(),
            Some(who.to_owned()),
        ),
    );
    let (client_link, server_link) = vox::memory_link_pair(64);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(handler))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    let lane = vox::initiator_on(client_link)
        .establish::<RawLaneCaller>()
        .await
        .unwrap();
    std::mem::forget(lane.connection);
    FilesClient::new(lane.caller)
}

fn p(s: &str) -> RootPath {
    RootPath::parse(s).unwrap()
}

fn fts() -> OrgId {
    OrgId::new("fts")
}

/// 3 MiB of a pattern, starting `VIDEO` so the fake calls it a video.
fn clip() -> Vec<u8> {
    let mut bytes = b"VIDEO".to_vec();
    bytes.extend((0..3u32 << 20).map(|i| (i % 251) as u8));
    bytes
}

impl World {
    fn content(&self, root: RootId, path: &str) -> String {
        web::content_url(&self.base, &fts(), root, &p(path), None, None)
    }

    async fn status(&self, url: &str, bearer: Option<&str>) -> u16 {
        let mut request = self.http.get(url);
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        request.send().await.unwrap().status().as_u16()
    }
}

#[tokio::test]
async fn a_browser_reads_a_file_by_range_as_the_session_it_presents() {
    let world = world().await;
    let alice = connect(&world, "fts", "alice").await;
    let root = alice.ensure_root("review", "Review").await.unwrap().id;
    let data = clip();
    alice
        .put(root, "Cuts/cut 1.mp4", &data, Save::create_only())
        .await
        .unwrap();
    let url = world.content(root, "Cuts/cut 1.mp4");

    // The whole file, as a video, offering ranges.
    let response = world
        .http
        .get(&url)
        .bearer_auth("alice")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert_eq!(response.bytes().await.unwrap().as_ref(), data.as_slice());

    // What a `<video>` asks as it seeks: a window, 206.
    let response = world
        .http
        .get(&url)
        .bearer_auth("alice")
        .header("range", "bytes=1000000-1000999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 1000000-1000999/{}", data.len()).as_str()
    );
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        &data[1_000_000..1_001_000]
    );

    // A viewer reads; nobody, a stranger and another org's member do not.
    assert_eq!(world.status(&url, Some("bob")).await, 200);
    assert_eq!(world.status(&url, None).await, 401);
    assert_eq!(world.status(&url, Some("mallory")).await, 403);
    assert_eq!(world.status(&url, Some("erin")).await, 403);
    // A path that is not there, and the query's token in place of the
    // header.
    assert_eq!(
        world
            .status(&world.content(root, "Cuts/none.mp4"), Some("alice"))
            .await,
        404
    );
    assert_eq!(world.status(&format!("{url}?token=bob"), None).await, 200);

    // A past version, by its id; a viewer may read history.
    alice
        .put(root, "Cuts/cut 1.mp4", b"VIDEO v2", Save::overwrite())
        .await
        .unwrap();
    let history = alice
        .service
        .history(root, p("Cuts/cut 1.mp4"))
        .await
        .unwrap();
    let old = web::content_url(
        &world.base,
        &fts(),
        root,
        &p("Cuts/cut 1.mp4"),
        Some(history[0].id),
        None,
    );
    let response = world
        .http
        .get(&old)
        .bearer_auth("bob")
        .send()
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().len(), data.len());
    let now = world
        .http
        .get(&url)
        .bearer_auth("bob")
        .send()
        .await
        .unwrap();
    assert_eq!(now.bytes().await.unwrap().as_ref(), b"VIDEO v2");
}

#[tokio::test]
async fn a_ticket_reads_its_folder_for_an_element_that_cannot_send_a_header() {
    let world = world().await;
    let alice = connect(&world, "fts", "alice").await;
    let root = alice.ensure_root("review", "Review").await.unwrap().id;
    alice
        .put(root, "Song/stem.wav", b"stem bytes", Save::create_only())
        .await
        .unwrap();
    alice
        .put(root, "Other/secret.wav", b"not yours", Save::create_only())
        .await
        .unwrap();

    // carol mints a ticket for the song's folder over her lane.
    let carol = connect(&world, "fts", "carol").await;
    let ticket = carol.ticket(root, "Song").await.unwrap();
    let stem = web::content_url(
        &world.base,
        &fts(),
        root,
        &p("Song/stem.wav"),
        None,
        Some(&ticket.token),
    );
    let response = world
        .http
        .get(&stem)
        .header("range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"stem");

    // It reads its folder and nothing else, and writes nothing.
    let other = web::content_url(
        &world.base,
        &fts(),
        root,
        &p("Other/secret.wav"),
        None,
        Some(&ticket.token),
    );
    assert_eq!(world.status(&other, None).await, 401);
    let begin = FilesWebHttpClient::at(world.base.clone())
        .begin_upload(
            "fts".into(),
            UploadSpec {
                root_id: root,
                path: p("Song/new.wav"),
                size: 3,
                content: None,
                expect: None,
            },
            ticket.token.clone(),
        )
        .await;
    assert!(begin.is_err());
    // A ticket altered by a byte does not open. Change a character in the
    // middle to a different one: the last one may already be 'A', and in
    // unpadded base64 it also carries spare bits that decode the same.
    let mut chars: Vec<char> = ticket.token.chars().collect();
    let mid = chars.len() / 2;
    chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
    let forged: String = chars.into_iter().collect();
    let forged_url = web::content_url(
        &world.base,
        &fts(),
        root,
        &p("Song/stem.wav"),
        None,
        Some(&forged),
    );
    assert_eq!(world.status(&forged_url, None).await, 401);

    // What carol may read is asked on every use: out of the org, her
    // ticket reads nothing.
    world.members.set(
        &Principal::User {
            user_id: "carol".into(),
        },
        Vec::new(),
    );
    assert_eq!(world.status(&stem, None).await, 403);

    // A viewer may mint one too (a ticket only reads), and someone in
    // another org cannot reach this org's lane to ask.
    let bob = connect(&world, "fts", "bob").await;
    assert!(bob.ticket(root, "Song").await.is_ok());
    let erin = connect(&world, "fts", "erin").await;
    assert!(erin.ticket(root, "Song").await.is_err());
}

#[tokio::test]
async fn an_upload_resumes_over_plain_puts_and_lands_as_a_version() {
    let world = world().await;
    let alice = connect(&world, "fts", "alice").await;
    let root = alice.ensure_root("review", "Review").await.unwrap().id;
    let data = clip();
    let client = FilesWebHttpClient::at(world.base.clone());

    let plan = client
        .begin_upload(
            "fts".into(),
            UploadSpec {
                root_id: root,
                path: p("Cuts/upload.mp4"),
                size: data.len() as u64,
                content: Some(content_of(&data)),
                expect: None,
            },
            "alice".into(),
        )
        .await
        .unwrap();
    assert_eq!(plan.needed.len(), 1);
    let put = format!("{}/files/fts/uploads/{}", world.base, plan.upload_id);

    // The first half, then the connection "drops".
    let half = data.len() / 2;
    let response = world
        .http
        .put(format!("{put}?offset=0"))
        .bearer_auth("alice")
        .body(data[..half].to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Where to resume is the server's account; the rest goes from there.
    let progress = client
        .upload_progress("fts".into(), plan.upload_id, "alice".into())
        .await
        .unwrap();
    assert_eq!(progress.received, half as u64);
    let resume = progress.needed[0].start;
    assert_eq!(resume, half as u64);
    let response = world
        .http
        .put(format!("{put}?offset={resume}"))
        .bearer_auth("alice")
        .body(data[half..].to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Someone else cannot land alice's upload; she can.
    assert!(
        client
            .complete_upload(
                "fts".into(),
                plan.upload_id,
                OnConflict::Fail,
                "carol".into()
            )
            .await
            .is_err()
    );
    let entry = client
        .complete_upload(
            "fts".into(),
            plan.upload_id,
            OnConflict::Fail,
            "alice".into(),
        )
        .await
        .unwrap();
    assert_eq!(entry.content, Some(content_of(&data)));
    assert_eq!(alice.get(root, "Cuts/upload.mp4").await.unwrap(), data);

    // A viewer may not upload at all.
    let refused = client
        .begin_upload(
            "fts".into(),
            UploadSpec {
                root_id: root,
                path: p("Cuts/bob.mp4"),
                size: 1,
                content: None,
                expect: None,
            },
            "bob".into(),
        )
        .await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn a_video_version_gets_its_renditions_served_ranged() {
    let world = world().await;
    let alice = connect(&world, "fts", "alice").await;
    let root = alice.ensure_root("review", "Review").await.unwrap().id;
    let data = clip();
    alice
        .put(root, "Cuts/cut.mov", &data, Save::create_only())
        .await
        .unwrap();
    let ticket = alice.ticket(root, "Cuts").await.unwrap();

    // Made off the request path: ask until they are.
    let status = format!(
        "{}/files/fts/{root}/renditions/Cuts/cut.mov?token={}",
        world.base, ticket.token
    );
    let mut set: Option<RenditionSet> = None;
    for _ in 0..500 {
        let text = world
            .http
            .get(&status)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let now: RenditionSet = facet_json::from_str(&text).unwrap();
        if now.settled() {
            set = Some(now);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let set = set.expect("the renditions settle");
    assert_eq!(set.class, Some(MediaClass::Video));

    // The proxy and the poster, ranged, as their types.
    let proxy = web::rendition_url(
        &world.base,
        &fts(),
        root,
        RenditionKind::Proxy,
        &p("Cuts/cut.mov"),
        None,
        Some(&ticket.token),
    );
    let response = world
        .http
        .get(&proxy)
        .header("range", "bytes=0-5")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"proxy:");
    let poster = web::rendition_url(
        &world.base,
        &fts(),
        root,
        RenditionKind::Poster,
        &p("Cuts/cut.mov"),
        None,
        Some(&ticket.token),
    );
    let response = world.http.get(&poster).send().await.unwrap();
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(
        response.text().await.unwrap(),
        format!("poster:{}", content_of(&data))
    );

    // A kind a video does not yield is 404, with the code a client reads.
    let audio = web::rendition_url(
        &world.base,
        &fts(),
        root,
        RenditionKind::Audio,
        &p("Cuts/cut.mov"),
        None,
        Some(&ticket.token),
    );
    let response = world.http.get(&audio).send().await.unwrap();
    assert_eq!(response.status(), 404);
    assert!(response.text().await.unwrap().contains("no_rendition"));
}
