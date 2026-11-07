//! `S3Store` against a real S3 server in process (`s3s-fs`, `SigV4`):
//! content addressing and dedup, multipart puts, ranged reads, a stable
//! holder id, verify, delete, the holder events, and a `Fetcher` that
//! fills itself from the bucket.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use architect::blob::{
    BlobError, BlobStore as _, Fetcher, HolderEvents, LocalFs, Remote as _, S3Config, S3Store,
    Sha256,
};
use tokio::io::AsyncReadExt as _;

const AK: &str = "AKIATESTTESTTESTTEST";
const SK: &str = "testsecrettestsecrettestsecrettestsecret";

async fn server(root: &std::path::Path) -> String {
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

fn config(endpoint: &str) -> S3Config {
    S3Config {
        endpoint: Some(endpoint.to_owned()),
        region: "us-east-1".into(),
        bucket: "blobs".into(),
        prefix: "architect".into(),
        access_key_id: AK.into(),
        secret_access_key: SK.into(),
        virtual_hosted: false,
    }
}

#[derive(Default)]
struct Seen(Mutex<Vec<String>>);

impl HolderEvents for Seen {
    fn held(&self, holder: &str, hash: Sha256, size: u64) {
        self.0
            .lock()
            .unwrap()
            .push(format!("held {holder} {hash} {size}"));
    }
    fn dropped(&self, holder: &str, hash: Sha256) {
        self.0
            .lock()
            .unwrap()
            .push(format!("dropped {holder} {hash}"));
    }
    fn verified(&self, holder: &str, hash: Sha256, ok: bool) {
        self.0
            .lock()
            .unwrap()
            .push(format!("verified {holder} {hash} {ok}"));
    }
}

async fn read_all(blob: architect::blob::Blob) -> Vec<u8> {
    let mut out = Vec::new();
    let mut reader = blob.reader;
    reader.read_to_end(&mut out).await.unwrap();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bucket_holds_blobs_by_hash_once_with_a_stable_holder_id() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = server(&tmp.path().join("s3")).await;
    let seen = Arc::new(Seen::default());
    let store = S3Store::open(&config(&endpoint))
        .await
        .unwrap()
        .with_events(seen.clone());
    let holder = store.holder_id().to_owned();
    assert!(holder.starts_with("s3:"));
    // Another node opening the same bucket is the same holder.
    let other = S3Store::open(&config(&endpoint)).await.unwrap();
    assert_eq!(other.holder_id(), holder);
    assert!(!format!("{:?}", config(&endpoint)).contains(SK));

    // A large blob goes up in parts and reads back by range.
    let big: Vec<u8> = (0..20_000_000u32)
        .map(|i| u8::try_from(i % 241).unwrap())
        .collect();
    let hash = store
        .put(None, Box::pin(std::io::Cursor::new(big.clone())))
        .await
        .unwrap();
    assert_eq!(hash, Sha256::of(&big));
    assert_eq!(
        store.key_for(&hash),
        format!(
            "architect/sha256/{}/{hash}",
            hash.to_hex().get(..2).unwrap()
        )
    );
    let tail = read_all(store.get_from(hash, 19_999_990).await.unwrap()).await;
    assert_eq!(tail, big[19_999_990..]);
    assert_eq!(other.size(hash).await.unwrap(), Some(20_000_000));

    // The same bytes from another node: nothing stored twice.
    let again = other
        .put(Some(hash), Box::pin(std::io::Cursor::new(big.clone())))
        .await
        .unwrap();
    assert_eq!(again, hash);
    let small = store.put(None, Box::pin(&b"take one"[..])).await.unwrap();
    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 2, "one object per hash: {listed:?}");

    // Bytes that do not hash to what was expected are not stored.
    let wrong = store
        .put(Some(Sha256::of(b"else")), Box::pin(&b"take two"[..]))
        .await;
    assert!(matches!(wrong, Err(BlobError::HashMismatch { .. })));
    assert_eq!(store.list().await.unwrap().len(), 2);

    // A scrub: whole bytes verify; corrupt ones do not.
    assert!(store.verify(hash).await.unwrap());
    let on_disk = tmp.path().join("s3/blobs").join(store.key_for(&small));
    std::fs::write(&on_disk, b"take 1!!").unwrap();
    assert!(!store.verify(small).await.unwrap());

    // A node missing a blob fetches it from the bucket, as from a peer.
    let local = LocalFs::new(tmp.path().join("local"));
    let fetcher = Fetcher::new(local.clone()).with_remote(Arc::new(other.clone()));
    fetcher.fetch(hash).await.unwrap();
    assert_eq!(
        std::fs::read(local.path_for(&hash)).unwrap().len(),
        big.len()
    );

    store.delete(small).await.unwrap();
    assert!(!store.has(small).await.unwrap());

    let events = seen.0.lock().unwrap().clone();
    assert!(events.contains(&format!("held {holder} {hash} 20000000")));
    assert!(events.contains(&format!("verified {holder} {hash} true")));
    assert!(events.contains(&format!("verified {holder} {small} false")));
    assert!(events.contains(&format!("dropped {holder} {small}")));
}
