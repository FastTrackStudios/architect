//! `architect::blob` (STA-388 acceptance): an interrupted download
//! resumes from its `.part`, a wrong hash is removed and retried, the
//! `<file>.sha256` sidecar skips a file already here, the rename is
//! atomic, one transfer per file; and the content-addressed `LocalFs`
//! store puts by hash, streams a get and answers `has`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use architect::blob::{
    BlobError, BlobStore, LocalFs, Reader, ResumableFetch, Sha256, Source, join_under, read_sidecar,
};
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt as _, ReadBuf};

const SIZE: usize = 200_000;

fn data() -> Vec<u8> {
    (0..SIZE).map(|i| (i * 31 % 251) as u8).collect()
}

/// Serves `data[offset..]`, or stops early with an error, or flips a byte.
struct TestSource {
    data: Vec<u8>,
    /// Break off with an i/o error once this absolute offset is reached.
    fail_at: Option<usize>,
    /// The first `corrupt_calls` calls flip one byte.
    corrupt_calls: usize,
    /// Serve this many extra bytes past the end.
    extra: usize,
    /// End cleanly (no error) at this absolute offset.
    end_at: Option<usize>,
    calls: AtomicUsize,
    offsets: Mutex<Vec<u64>>,
}

impl TestSource {
    const fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            fail_at: None,
            corrupt_calls: 0,
            extra: 0,
            end_at: None,
            calls: AtomicUsize::new(0),
            offsets: Mutex::new(Vec::new()),
        }
    }

    fn offsets(&self) -> Vec<u64> {
        self.offsets.lock().unwrap().clone()
    }
}

/// Yields `bytes` in small reads, then an error if `fail`.
struct Chunked {
    bytes: Vec<u8>,
    pos: usize,
    fail: bool,
}

impl AsyncRead for Chunked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos >= self.bytes.len() {
            if self.fail {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "network went away",
                )));
            }
            return Poll::Ready(Ok(()));
        }
        let n = (self.bytes.len() - self.pos)
            .min(buf.remaining())
            .min(7_000);
        let start = self.pos;
        buf.put_slice(&self.bytes[start..start + n]);
        self.pos += n;
        Poll::Ready(Ok(()))
    }
}

impl Source for TestSource {
    fn read_from(&self, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.offsets.lock().unwrap().push(offset);
        let offset = offset as usize;
        let mut end = self.fail_at.or(self.end_at).unwrap_or(self.data.len());
        end = end.max(offset);
        let mut bytes = self.data[offset..end].to_vec();
        bytes.extend(std::iter::repeat_n(0xAA, self.extra));
        if call < self.corrupt_calls
            && let Some(b) = bytes.last_mut()
        {
            *b ^= 0xFF;
        }
        let fail = self.fail_at.is_some();
        Box::pin(async move {
            let reader: Reader = Box::pin(Chunked {
                bytes,
                pos: 0,
                fail,
            });
            Ok(reader)
        })
    }
}

fn fetch(dir: &tempfile::TempDir, data: &[u8]) -> ResumableFetch {
    ResumableFetch::new(dir.path().join("packs/kit.bin"), data.len() as u64)
        .sha256(Sha256::of(data))
}

#[tokio::test]
async fn interrupted_download_resumes_from_the_part() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);

    let mut broken = TestSource::new(data.clone());
    broken.fail_at = Some(70_000);
    let err = job.run(&broken, |_| {}).await.unwrap_err();
    assert!(matches!(err, BlobError::Source(_)), "{err:?}");
    assert_eq!(job.partial_len(), 70_000);
    assert!(!job.dest().exists(), "nothing in place before it is whole");
    assert!(!job.is_here());

    let good = TestSource::new(data.clone());
    let mut seen = Vec::new();
    let done = job.run(&good, |n| seen.push(n)).await.unwrap();
    assert_eq!(good.offsets(), vec![70_000], "asked only for the rest");
    assert_eq!(done.resumed_from, 70_000);
    assert!(!done.skipped);
    assert_eq!(done.sha256, Sha256::of(&data));
    assert_eq!(seen.last().copied(), Some(SIZE as u64));
    assert!(seen.iter().all(|n| *n > 70_000));
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
    assert!(
        !job.part_path().exists(),
        "the .part was renamed into place"
    );
    assert_eq!(
        std::fs::read_to_string(job.sidecar_path()).unwrap(),
        format!("{SIZE}:{}\n", Sha256::of(&data))
    );
}

#[tokio::test]
async fn sidecar_skips_a_file_already_here() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);
    job.run(&TestSource::new(data.clone()), |_| {})
        .await
        .unwrap();

    let again = TestSource::new(data.clone());
    let done = job.run(&again, |_| {}).await.unwrap();
    assert!(done.skipped);
    assert_eq!(done.sha256, Sha256::of(&data));
    assert_eq!(again.calls.load(Ordering::SeqCst), 0, "source not called");
    assert_eq!(read_sidecar(job.dest()), Some(Sha256::of(&data)));

    // A different expected hash is not "here", and neither is a file whose
    // size no longer matches its sidecar.
    let other = ResumableFetch::new(job.dest(), SIZE as u64).sha256(Sha256::of(b"other"));
    assert!(!other.is_here());
    std::fs::write(job.dest(), b"short").unwrap();
    assert_eq!(read_sidecar(job.dest()), None);
    assert!(!job.is_here());
    // ...and fetching again restores it.
    let done = job
        .run(&TestSource::new(data.clone()), |_| {})
        .await
        .unwrap();
    assert!(!done.skipped);
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
}

#[tokio::test]
async fn wrong_hash_is_removed_and_retried() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);

    let mut flaky = TestSource::new(data.clone());
    flaky.corrupt_calls = 1;
    let done = job.run(&flaky, |_| {}).await.unwrap();
    assert_eq!(flaky.offsets(), vec![0, 0], "the retry started from zero");
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
    assert_eq!(done.resumed_from, 0);
}

#[tokio::test]
async fn wrong_hash_with_no_attempts_left_removes_the_part() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data).attempts(1);

    let mut bad = TestSource::new(data.clone());
    bad.corrupt_calls = usize::MAX;
    let err = job.run(&bad, |_| {}).await.unwrap_err();
    match err {
        BlobError::HashMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, Sha256::of(&data));
            assert_ne!(actual, expected);
        }
        other => panic!("expected HashMismatch, got {other:?}"),
    }
    assert!(!job.part_path().exists(), "the corrupt .part was removed");
    assert!(!job.dest().exists());
    assert!(!job.sidecar_path().exists());

    // Retried later from a good source, it lands.
    job.run(&TestSource::new(data.clone()), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
}

#[tokio::test]
async fn a_corrupt_part_from_an_earlier_run_costs_one_retry() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);
    std::fs::create_dir_all(job.dest().parent().unwrap()).unwrap();
    std::fs::write(job.part_path(), vec![0u8; 1_000]).unwrap();

    let good = TestSource::new(data.clone());
    let done = job.run(&good, |_| {}).await.unwrap();
    assert_eq!(good.offsets(), vec![1_000, 0]);
    assert_eq!(done.resumed_from, 0);
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
}

#[tokio::test]
async fn short_source_keeps_the_part_and_long_source_drops_it() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);

    let mut short = TestSource::new(data.clone());
    short.end_at = Some(50_000);
    let err = job.run(&short, |_| {}).await.unwrap_err();
    assert!(
        matches!(err, BlobError::Incomplete { have: 50_000, .. }),
        "{err:?}"
    );
    assert_eq!(job.partial_len(), 50_000);

    let mut long = TestSource::new(data.clone());
    long.extra = 10;
    let err = job.run(&long, |_| {}).await.unwrap_err();
    assert!(matches!(err, BlobError::TooLong { .. }), "{err:?}");
    assert_eq!(long.offsets(), vec![50_000]);
    assert!(!job.part_path().exists());
    assert!(!job.dest().exists());

    // A stale .part longer than the file starts again.
    std::fs::write(job.part_path(), vec![1u8; SIZE + 5]).unwrap();
    let good = TestSource::new(data.clone());
    job.run(&good, |_| {}).await.unwrap();
    assert_eq!(good.offsets(), vec![0]);
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
}

/// Parks in `read_from` until released, so a second fetch can race it.
struct Gated {
    inner: TestSource,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl Source for Gated {
    fn read_from(&self, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            self.inner.read_from(offset).await
        })
    }
}

#[tokio::test]
async fn one_transfer_per_file() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = Arc::new(fetch(&dir, &data));
    let gated = Arc::new(Gated {
        inner: TestSource::new(data.clone()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });

    let first = {
        let (job, gated) = (job.clone(), gated.clone());
        tokio::spawn(async move { job.run(&*gated, |_| {}).await })
    };
    gated.entered.notified().await;

    let second = TestSource::new(data.clone());
    let err = job.run(&second, |_| {}).await.unwrap_err();
    assert!(matches!(err, BlobError::Busy(_)), "{err:?}");
    assert_eq!(second.calls.load(Ordering::SeqCst), 0);

    // A different file is not blocked.
    let other = ResumableFetch::new(dir.path().join("other.bin"), 3).sha256(Sha256::of(b"abc"));
    other
        .run(&TestSource::new(b"abc".to_vec()), |_| {})
        .await
        .unwrap();

    gated.release.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);

    // The slot is free again: the next run sees the sidecar.
    assert!(job.run(&second, |_| {}).await.unwrap().skipped);
}

#[tokio::test]
async fn a_dropped_fetch_pauses_and_frees_the_slot() {
    let dir = tempfile::tempdir().unwrap();
    let data = data();
    let job = fetch(&dir, &data);
    let gated = Gated {
        inner: TestSource::new(data.clone()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    };
    // Drop the fetch while it waits on the source.
    let timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        job.run(&gated, |_| {}),
    )
    .await;
    assert!(timed_out.is_err());
    let done = job
        .run(&TestSource::new(data.clone()), |_| {})
        .await
        .unwrap();
    assert!(!done.skipped);
    assert_eq!(std::fs::read(job.dest()).unwrap(), data);
}

#[tokio::test]
async fn local_fs_puts_by_hash_streams_gets_and_answers_has() {
    let dir = tempfile::tempdir().unwrap();
    let fs = LocalFs::new(dir.path().join("blobs"));
    let store: &dyn BlobStore = &fs;
    let data = data();
    let hash = Sha256::of(&data);

    assert!(!store.has(hash).await.unwrap());
    assert!(matches!(
        store.get(hash).await.unwrap_err(),
        BlobError::NotFound(h) if h == hash
    ));

    let body: Reader = Box::pin(std::io::Cursor::new(data.clone()));
    assert_eq!(store.put(None, body).await.unwrap(), hash);
    assert!(store.has(hash).await.unwrap());
    let hex = hash.to_hex();
    assert_eq!(
        fs.path_for(&hash),
        dir.path()
            .join("blobs")
            .join(hex.get(..2).unwrap())
            .join(&hex)
    );

    let mut blob = store.get(hash).await.unwrap();
    assert_eq!(blob.len, SIZE as u64);
    let mut back = Vec::new();
    blob.reader.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, data);

    // Same bytes again: same hash, still one file.
    assert_eq!(fs.put_bytes(&data).await.unwrap(), hash);
    // Known hash already stored: the body is not even read.
    let unread: Reader = Box::pin(Chunked {
        bytes: Vec::new(),
        pos: 0,
        fail: true,
    });
    assert_eq!(store.put(Some(hash), unread).await.unwrap(), hash);
    let tmp = dir.path().join("blobs/.tmp");
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
}

#[tokio::test]
async fn local_fs_refuses_bytes_that_do_not_match_the_expected_hash() {
    let dir = tempfile::tempdir().unwrap();
    let fs = LocalFs::new(dir.path());
    let claimed = Sha256::of(b"what the caller said");
    let body: Reader = Box::pin(std::io::Cursor::new(b"what arrived".to_vec()));
    let err = fs.put(Some(claimed), body).await.unwrap_err();
    assert!(
        matches!(err, BlobError::HashMismatch { expected, .. } if expected == claimed),
        "{err:?}"
    );
    assert!(!fs.has(claimed).await.unwrap());
    assert!(!fs.has(Sha256::of(b"what arrived")).await.unwrap());
    assert_eq!(
        std::fs::read_dir(dir.path().join(".tmp")).unwrap().count(),
        0
    );

    // A body that breaks off stores nothing either.
    let broken: Reader = Box::pin(Chunked {
        bytes: b"half".to_vec(),
        pos: 0,
        fail: true,
    });
    assert!(matches!(
        fs.put(None, broken).await.unwrap_err(),
        BlobError::Source(_)
    ));
    assert_eq!(
        std::fs::read_dir(dir.path().join(".tmp")).unwrap().count(),
        0
    );
}

#[test]
fn sha256_hex_round_trips_and_rejects_junk() {
    let h = Sha256::of(b"abc");
    assert_eq!(
        h.to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(h.to_hex().parse::<Sha256>().unwrap(), h);
    assert_eq!(Sha256::from_hex(&h.to_hex().to_uppercase()).unwrap(), h);
    assert_eq!(Sha256::from_bytes(*h.as_bytes()), h);
    let not_hex = "g".repeat(64);
    let signed = format!("+{}", h.to_hex().get(1..).unwrap());
    for junk in ["", "abc", not_hex.as_str(), signed.as_str()] {
        assert!(
            matches!(Sha256::from_hex(junk), Err(BlobError::BadHash(_))),
            "{junk}"
        );
    }
}

#[test]
fn join_under_keeps_host_paths_inside_the_root() {
    let root = std::path::Path::new("/lib");
    assert_eq!(
        join_under(root, "packs/kit.bin").unwrap(),
        root.join("packs/kit.bin")
    );
    for bad in ["../x", "/etc/passwd", "a//b", "a/./b", "a/..", "", "a/"] {
        assert!(
            matches!(join_under(root, bad), Err(BlobError::BadPath(_))),
            "{bad}"
        );
    }
}
