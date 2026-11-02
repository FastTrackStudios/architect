//! Renditions on a server: what a version's media yields, made off the
//! request path by a transcoder (a fake here, `ffmpeg` in the ignored
//! test at the end), kept across a restart and made again when missing.

#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::significant_drop_tightening
)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::blob::{LocalFs, Sha256};
use architect_files::server::{FilesServer, TranscodeError, Transcoder};
use architect_files::{
    FilesFault, FilesService as _, MediaClass, OnConflict, Relocation, RenditionKind, RenditionSet,
    RenditionState, RootId, RootPath, UploadSpec, content_of,
};
use architect_orgs::OrgId;
use architect_orgs::server::Members;
use architect_permissions::Principal;
use futures_util::future::BoxFuture;

/// Classifies by a prefix (`VIDEO`, `AUDIO`) and "renders" the kind's
/// name and the source's hash, so a rendition is reproducible. Records
/// every call; fails `fail` if set.
#[derive(Clone, Default)]
struct Fake {
    calls: Arc<Mutex<Vec<String>>>,
    fail: Option<RenditionKind>,
}

impl Fake {
    fn failing(kind: RenditionKind) -> Self {
        Self {
            fail: Some(kind),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Transcoder for Fake {
    fn probe<'a>(&'a self, source: &'a Path) -> BoxFuture<'a, Result<MediaClass, TranscodeError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("probe".into());
            let bytes = tokio::fs::read(source).await.unwrap();
            Ok(if bytes.starts_with(b"VIDEO") {
                MediaClass::Video
            } else if bytes.starts_with(b"AUDIO") {
                MediaClass::Audio
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
            self.calls.lock().unwrap().push(kind.tag().into());
            if self.fail == Some(kind) {
                return Err(TranscodeError::new("no sound to draw"));
            }
            let bytes = tokio::fs::read(source).await.unwrap();
            let made = format!("{kind}:{}", Sha256::of(&bytes).to_hex());
            tokio::fs::write(out, made).await.unwrap();
            Ok(())
        })
    }
}

fn server(dir: &Path, transcoder: impl Transcoder) -> FilesServer {
    FilesServer::new(dir, LocalFs::new(dir.join("blobs")), Members::new())
        .with_renditions(transcoder)
}

fn as_service(files: &FilesServer) -> architect_files::server::Acting {
    files.acting(
        OrgId::new("fts"),
        Principal::Service {
            name: "test".into(),
        },
    )
}

fn p(s: &str) -> RootPath {
    RootPath::parse(s).unwrap()
}

/// Save `bytes` at `path` in process, through the upload lane.
async fn save(files: &FilesServer, root: RootId, path: &str, bytes: &[u8]) {
    use futures_util::stream;
    let acting = as_service(files);
    let plan = acting
        .begin_upload(UploadSpec {
            root_id: root,
            path: p(path),
            size: bytes.len() as u64,
            content: Some(content_of(bytes)),
            expect: None,
        })
        .await
        .unwrap();
    if !plan.needed.is_empty() {
        acting
            .receive(plan.upload_id, stream::iter([Ok((0, bytes.to_vec()))]))
            .await
            .unwrap();
    }
    acting
        .complete_upload(plan.upload_id, OnConflict::Replace)
        .await
        .unwrap();
}

async fn settled(files: &FilesServer, root: RootId, path: &str) -> RenditionSet {
    let acting = as_service(files);
    for _ in 0..500 {
        let set = acting.renditions(root, p(path), None).await.unwrap();
        if set.settled() {
            return set;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the renditions of {path} never settled");
}

fn ready(set: &RenditionSet, kind: RenditionKind) -> bool {
    matches!(set.state(kind), Some(RenditionState::Ready { .. }))
}

#[tokio::test]
async fn a_video_gets_a_proxy_a_poster_and_peaks_and_audio_peaks_and_a_transcode() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let files = server(dir.path(), fake.clone());
    let root = as_service(&files)
        .ensure_root("review".into(), "Review".into())
        .await
        .unwrap()
        .id;

    save(&files, root, "cut 1.mov", b"VIDEO one").await;
    save(&files, root, "mix.wav", b"AUDIO two").await;
    save(&files, root, "notes.txt", b"just words").await;

    let video = settled(&files, root, "cut 1.mov").await;
    assert_eq!(video.class, Some(MediaClass::Video));
    let kinds: Vec<_> = video.kinds.iter().map(|k| k.kind).collect();
    assert_eq!(
        kinds,
        [
            RenditionKind::Proxy,
            RenditionKind::Poster,
            RenditionKind::Peaks
        ]
    );
    assert!(kinds.iter().all(|&k| ready(&video, k)));

    let audio = settled(&files, root, "mix.wav").await;
    assert_eq!(audio.class, Some(MediaClass::Audio));
    assert!(ready(&audio, RenditionKind::Peaks) && ready(&audio, RenditionKind::Audio));
    assert_eq!(audio.state(RenditionKind::Proxy), None);

    let text = settled(&files, root, "notes.txt").await;
    assert_eq!(text.class, Some(MediaClass::Other));
    assert!(text.kinds.is_empty());

    // The proxy's bytes are what the transcoder made of the version's.
    let acting = as_service(&files);
    let (blob, size) = acting
        .rendition(root, &p("cut 1.mov"), None, RenditionKind::Proxy)
        .await
        .unwrap();
    let made = std::fs::read(blob).unwrap();
    assert_eq!(made.len() as u64, size);
    assert_eq!(
        String::from_utf8(made).unwrap(),
        format!("proxy:{}", content_of(b"VIDEO one"))
    );

    // A kind the class does not yield is no rendition, not a wait.
    let err = acting
        .rendition(root, &p("mix.wav"), None, RenditionKind::Poster)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesFault::NoRendition { .. }), "{err:?}");
}

#[tokio::test]
async fn the_same_bytes_share_their_renditions_and_a_new_version_gets_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let files = server(dir.path(), fake.clone());
    let acting = as_service(&files);
    let root = acting
        .ensure_root("review".into(), "Review".into())
        .await
        .unwrap()
        .id;
    save(&files, root, "a.mov", b"VIDEO same").await;
    let first = settled(&files, root, "a.mov").await;
    let made = fake.calls().len();

    // A copy holds the same bytes: nothing is made again.
    acting
        .copy_paths(
            root,
            vec![Relocation {
                from: p("a.mov"),
                to: p("b.mov"),
            }],
            OnConflict::Fail,
        )
        .await
        .unwrap();
    assert_eq!(settled(&files, root, "b.mov").await, first);
    assert_eq!(fake.calls().len(), made);

    // A new version is new bytes: its own renditions, the old version's
    // kept and still addressed by its version.
    save(&files, root, "a.mov", b"VIDEO changed").await;
    let second = settled(&files, root, "a.mov").await;
    assert_ne!(second.content, first.content);
    let history = acting.history(root, p("a.mov")).await.unwrap();
    let old = acting
        .renditions(root, p("a.mov"), Some(history[0].id))
        .await
        .unwrap();
    assert_eq!(old, first);
}

#[tokio::test]
async fn renditions_survive_a_restart_and_are_rebuilt_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let root = {
        let files = server(dir.path(), Fake::default());
        let root = as_service(&files)
            .ensure_root("review".into(), "Review".into())
            .await
            .unwrap()
            .id;
        save(&files, root, "cut.mov", b"VIDEO kept").await;
        settled(&files, root, "cut.mov").await;
        root
    };

    // A new server on the same data: every rendition is there at once,
    // and the transcoder is not asked for anything.
    let fake = Fake::default();
    let files = server(dir.path(), fake.clone());
    let set = as_service(&files)
        .renditions(root, p("cut.mov"), None)
        .await
        .unwrap();
    assert!(set.settled());
    assert!(set.kinds.iter().all(|k| ready(&set, k.kind)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());

    // The proxy's blob goes (collected, a lost disk): it is missing, and
    // made again — it alone.
    let Some(RenditionState::Ready { content, .. }) = set.state(RenditionKind::Proxy) else {
        panic!("no proxy");
    };
    let blobs = LocalFs::new(dir.path().join("blobs"));
    std::fs::remove_file(blobs.path_for(&Sha256::from_hex(content.as_str()).unwrap())).unwrap();
    let err = as_service(&files)
        .rendition(root, &p("cut.mov"), None, RenditionKind::Proxy)
        .await
        .unwrap_err();
    assert_eq!(err, FilesFault::RenditionPending(RenditionKind::Proxy));
    let again = settled(&files, root, "cut.mov").await;
    assert!(ready(&again, RenditionKind::Proxy));
    assert_eq!(fake.calls(), ["proxy"]);
    assert_eq!(again, set);
}

#[tokio::test]
async fn a_kind_the_transcoder_fails_at_is_reported_and_not_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::failing(RenditionKind::Peaks);
    let files = server(dir.path(), fake.clone());
    let acting = as_service(&files);
    let root = acting
        .ensure_root("review".into(), "Review".into())
        .await
        .unwrap()
        .id;
    save(&files, root, "silent.mov", b"VIDEO no sound").await;
    let set = settled(&files, root, "silent.mov").await;
    assert!(ready(&set, RenditionKind::Proxy) && ready(&set, RenditionKind::Poster));
    assert_eq!(
        set.state(RenditionKind::Peaks),
        Some(&RenditionState::Failed {
            reason: "no sound to draw".into()
        })
    );
    let err = acting
        .rendition(root, &p("silent.mov"), None, RenditionKind::Peaks)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesFault::NoRendition { .. }), "{err:?}");
    let asked = fake.calls().len();
    settled(&files, root, "silent.mov").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fake.calls().len(), asked);
}

#[tokio::test]
async fn a_server_with_no_transcoder_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let files = FilesServer::new(
        dir.path(),
        LocalFs::new(dir.path().join("blobs")),
        Members::new(),
    );
    let root = as_service(&files)
        .ensure_root("review".into(), "Review".into())
        .await
        .unwrap()
        .id;
    save(&files, root, "cut.mov", b"VIDEO").await;
    let set = as_service(&files)
        .renditions(root, p("cut.mov"), None)
        .await
        .unwrap();
    assert!(set.settled() && set.problem.is_some());
    let err = as_service(&files)
        .rendition(root, &p("cut.mov"), None, RenditionKind::Proxy)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesFault::NoRendition { .. }), "{err:?}");
}

/// The real thing: `ffmpeg` makes a proxy, a poster and peaks of a test
/// video, and peaks and AAC of a tone. Needs `ffmpeg`/`ffprobe` on PATH
/// (`nix shell nixpkgs#ffmpeg -c cargo test --workspace -- --ignored ffmpeg`).
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe on PATH"]
async fn ffmpeg_makes_a_proxy_a_poster_and_peaks() {
    use architect_files::server::FfmpegTranscoder;
    let ffmpeg = FfmpegTranscoder::new();
    assert!(ffmpeg.available().await, "ffmpeg is not on PATH");
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    let make = |args: &[&str], out: &str| {
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .arg(media.join(out))
            .status()
            .unwrap();
        assert!(status.success(), "making {out}");
        std::fs::read(media.join(out)).unwrap()
    };
    let video = make(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=1280x960:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "pcm_s16le",
            "-shortest",
        ],
        "clip.mov",
    );
    let tone = make(
        &["-f", "lavfi", "-i", "sine=frequency=220:duration=3"],
        "tone.wav",
    );

    let files = server(dir.path(), ffmpeg);
    let root = as_service(&files)
        .ensure_root("review".into(), "Review".into())
        .await
        .unwrap()
        .id;
    save(&files, root, "clip.mov", &video).await;
    save(&files, root, "tone.wav", &tone).await;

    let wait = |path: &'static str| {
        let files = files.clone();
        async move {
            for _ in 0..600 {
                let set = as_service(&files)
                    .renditions(root, p(path), None)
                    .await
                    .unwrap();
                if set.settled() {
                    return set;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("{path} never settled");
        }
    };
    let set = wait("clip.mov").await;
    assert_eq!(set.class, Some(MediaClass::Video), "{set:?}");
    assert!(set.kinds.iter().all(|k| ready(&set, k.kind)), "{set:?}");
    let read = |kind| {
        let files = files.clone();
        async move {
            let (blob, _) = as_service(&files)
                .rendition(
                    root,
                    &p(if kind == RenditionKind::Audio {
                        "tone.wav"
                    } else {
                        "clip.mov"
                    }),
                    None,
                    kind,
                )
                .await
                .unwrap();
            std::fs::read(blob).unwrap()
        }
    };
    let proxy = read(RenditionKind::Proxy).await;
    assert_eq!(&proxy[4..8], b"ftyp", "an MP4");
    let poster = read(RenditionKind::Poster).await;
    assert_eq!(&poster[..2], &[0xFF, 0xD8], "a JPEG");
    let peaks: architect_files::Peaks =
        facet_json::from_str(&String::from_utf8(read(RenditionKind::Peaks).await).unwrap())
            .unwrap();
    assert_eq!(peaks.length, 200, "two seconds at 100 a second");
    assert!(peaks.data.iter().any(|&s| s > 1000), "a tone is loud");

    let tone = wait("tone.wav").await;
    assert_eq!(tone.class, Some(MediaClass::Audio), "{tone:?}");
    let aac = read(RenditionKind::Audio).await;
    assert_eq!(&aac[4..8], b"ftyp", "an MP4 of sound");
}
