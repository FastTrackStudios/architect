//! The S3 face over a node's synced roots (feature `s3`), on a real
//! socket, signed with `SigV4` by a real S3 client (`object_store`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{Fetcher, LocalFs, Sha256};
use architect::http::axum::http::{Method, Uri};
use architect_files::sync::s3::{AsIssued, KeyHolder, S3Face, S3Key, S3Keys};
use architect_files::sync::{Engine, EngineConfig, TreeStore};
use architect_orgs::Role;
use loro::LoroDoc;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as Key;
use object_store::{ObjectStore as _, ObjectStoreExt as _, PutPayload, WriteMultipart};
use s3s::S3 as _;
use uuid::Uuid;

fn node(tmp: &Path) -> Engine {
    let files = tmp.join("tree");
    let p = files.join("fts/Projects/Right Here");
    std::fs::create_dir_all(p.join("Audio Files")).unwrap();
    std::fs::write(p.join("Right Here.RPP"), "<REAPER_PROJECT 0.1\n>\n").unwrap();
    std::fs::write(p.join("Audio Files/Bass_1.wav"), vec![7u8; 300_000]).unwrap();
    let lib = files.join("fts/Assets/Tracks/Praise");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("Praise.md"), "---\ntype: song\n---\n").unwrap();

    let docs: Arc<Mutex<HashMap<Uuid, LoroDoc>>> = Arc::default();
    let local = LocalFs::new(tmp.join("blobs"));
    Engine::new(
        EngineConfig {
            settle: Duration::ZERO,
            ..EngineConfig::new("node", tmp.join("data"), files, true)
        },
        TreeStore::new(local.clone()),
        Fetcher::new(local),
        Arc::new(move |id| {
            let doc = docs.lock().unwrap().entry(id).or_default().clone();
            Box::pin(async move { Some(doc) })
        }),
    )
}

fn client(addr: std::net::SocketAddr, key: &S3Key) -> AmazonS3 {
    AmazonS3Builder::new()
        .with_endpoint(format!("http://{addr}"))
        .with_allow_http(true)
        .with_region("us-east-1")
        .with_bucket_name("fts")
        .with_access_key_id(&key.access_key)
        .with_secret_access_key(&key.secret)
        .build()
        .unwrap()
}

fn request<T>(input: T, key: &S3Key) -> s3s::S3Request<T> {
    s3s::S3Request {
        input,
        method: Method::GET,
        uri: Uri::from_static("/"),
        headers: architect::http::axum::http::HeaderMap::new(),
        extensions: architect::http::axum::http::Extensions::new(),
        credentials: Some(s3s::auth::Credentials {
            access_key: key.access_key.clone(),
            secret_key: key.secret.clone().into(),
        }),
        region: None,
        service: None,
        trailing_headers: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_org_is_a_bucket_and_s3_clients_read_write_and_list_versions_by_role() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = node(tmp.path());
    engine.pass_all().await;
    engine.pass_all().await;

    let keys = S3Keys::in_dir(&tmp.path().join("data"));
    let member = keys
        .issue(
            "fts",
            KeyHolder::User("cody".into()),
            Role::Member,
            "laptop",
        )
        .unwrap();
    let viewer = keys
        .issue("fts", KeyHolder::Device("ep1".into()), Role::Viewer, "tv")
        .unwrap();
    let face = S3Face::new(engine.clone(), keys.clone(), AsIssued);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(face.clone().serve(listener));
    let s3 = client(addr, &member);

    // Listing: the org's sections, then a project's folder.
    let top = s3.list_with_delimiter(None).await.unwrap();
    let mut sections: Vec<String> = top
        .common_prefixes
        .iter()
        .map(ToString::to_string)
        .collect();
    sections.sort();
    assert_eq!(sections, vec!["Assets", "Projects"]);
    let project = s3
        .list_with_delimiter(Some(&Key::from("Projects/Right Here")))
        .await
        .unwrap();
    assert_eq!(project.objects.len(), 1, "the session file");
    assert_eq!(project.common_prefixes.len(), 1, "Audio Files/");
    let all: Vec<_> = futures::TryStreamExt::try_collect::<Vec<_>>(s3.list(None))
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    // A ranged read and a head, with the content's sha-256 as the ETag.
    let wav = Key::from("Projects/Right Here/Audio Files/Bass_1.wav");
    let head = s3.head(&wav).await.unwrap();
    assert_eq!(head.size, 300_000);
    assert_eq!(
        head.e_tag.as_deref().map(|e| e.trim_matches('"').len()),
        Some(64)
    );
    let ten = s3.get_range(&wav, 1_000..1_010).await.unwrap();
    assert_eq!(&ten[..], &[7u8; 10]);

    // A put is a version; the working copy follows.
    let rpp = Key::from("Projects/Right Here/Right Here.RPP");
    s3.put(&rpp, PutPayload::from_static(b"<REAPER_PROJECT 0.2\n>\n"))
        .await
        .unwrap();
    let got = s3.get(&rpp).await.unwrap().bytes().await.unwrap();
    assert_eq!(&got[..], b"<REAPER_PROJECT 0.2\n>\n");

    // A multipart upload of a large take: one version, whole.
    let take = Key::from("Projects/Right Here/Audio Files/Take_2.wav");
    let big: Vec<u8> = (0..12_000_000u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let mut upload =
        WriteMultipart::new_with_chunk_size(s3.put_multipart(&take).await.unwrap(), 5 << 20);
    upload.write(&big);
    upload.finish().await.unwrap();
    let back = s3.get(&take).await.unwrap().bytes().await.unwrap();
    assert_eq!(back.len(), big.len());
    assert_eq!(Sha256::of(&back), Sha256::of(&big));

    // A copy into the library moves no bytes; a delete is a version.
    let copied = Key::from("Assets/Tracks/Praise/Bass.wav");
    s3.copy(&wav, &copied).await.unwrap();
    assert_eq!(s3.head(&copied).await.unwrap().e_tag, head.e_tag);
    s3.delete(&copied).await.unwrap();
    assert!(s3.head(&copied).await.is_err());

    // Versions: the session file's two, newest first; the copy's delete marker.
    let versions = face
        .list_object_versions(request(
            s3s::dto::ListObjectVersionsInput {
                bucket: "fts".into(),
                prefix: Some("Projects/Right Here/Right Here.RPP".into()),
                ..Default::default()
            },
            &member,
        ))
        .await
        .unwrap()
        .output;
    let v = versions.versions.unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].is_latest, Some(true));
    let old = v[1].version_id.clone().unwrap();
    let past = face
        .get_object(request(
            s3s::dto::GetObjectInput {
                bucket: "fts".into(),
                key: "Projects/Right Here/Right Here.RPP".into(),
                version_id: Some(old),
                ..Default::default()
            },
            &member,
        ))
        .await
        .unwrap()
        .output;
    assert_eq!(past.content_length, Some(22));
    let markers = face
        .list_object_versions(request(
            s3s::dto::ListObjectVersionsInput {
                bucket: "fts".into(),
                prefix: Some("Assets/".into()),
                ..Default::default()
            },
            &member,
        ))
        .await
        .unwrap()
        .output
        .delete_markers
        .unwrap();
    assert_eq!(markers.len(), 1);

    // A viewer reads and may not write; another org is no bucket; a
    // revoked or unknown key is refused.
    let tv = client(addr, &viewer);
    assert!(tv.head(&wav).await.is_ok());
    assert!(tv.put(&rpp, PutPayload::from_static(b"x")).await.is_err());
    let elsewhere = keys
        .issue("other", KeyHolder::User("cody".into()), Role::Owner, "x")
        .unwrap();
    assert!(client(addr, &elsewhere).head(&wav).await.is_err());
    keys.revoke(&viewer.access_key).unwrap();
    assert!(tv.head(&wav).await.is_err());
    let forged = S3Key {
        secret: "wrong".repeat(8),
        ..member.clone()
    };
    assert!(client(addr, &forged).head(&wav).await.is_err());

    // The working copy on the node has the new session file and the take.
    tokio::time::sleep(Duration::from_millis(300)).await;
    engine.pass_all().await;
    let dir = tmp.path().join("tree/fts/Projects/Right Here");
    assert_eq!(
        std::fs::read_to_string(dir.join("Right Here.RPP")).unwrap(),
        "<REAPER_PROJECT 0.2\n>\n"
    );
    assert_eq!(
        std::fs::metadata(dir.join("Audio Files/Take_2.wav"))
            .unwrap()
            .len(),
        12_000_000
    );
}
