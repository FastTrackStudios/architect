//! The updater against a local fake of the GitHub releases API (#523):
//! a newer release is offered; Install writes the image and `version`
//! into the update dir and leaves `$APPIMAGE` alone, or replaces
//! `$APPIMAGE`; an unsigned release, a bad signature and a checksum
//! mismatch are refused with nothing written; a prerelease is ignored on
//! Stable; no `$APPIMAGE` reports only; a development build never asks.
//! Directories and env are parameters: no global env, no real network.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::significant_drop_tightening,
    clippy::option_if_let_else,
    clippy::as_conversions
)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::blob::Sha256;
use architect::release::{SIG_ASSET, SUMS_ASSET, trusted_comment};
use architect_updates::engine::{Engine, Source};
use architect_updates::model::{
    Channel, Env, LINUX_ASSET, Plan, Platform, ReportOnly, State, VERSION_FILE,
};
use axum::extract::{Request, State as AxState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blake2::Digest as _;
use ed25519_dalek::{Signer as _, SigningKey};

const OLD: &[u8] = b"old appimage bytes";
const NEW: &[u8] = b"new appimage bytes, version 1.3.0";
const SOURCE: Source = Source {
    app: "architect",
    repo: "FastTrackStudios/architect",
    key: None,
};

/// A throwaway minisign key: signs like `minisign -S -t <comment>`.
struct TestKey {
    sk: SigningKey,
    keynum: [u8; 8],
}

impl TestKey {
    fn new(seed: u8) -> Self {
        Self {
            sk: SigningKey::from_bytes(&[seed; 32]),
            keynum: [seed; 8],
        }
    }

    fn public(&self) -> String {
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&self.keynum);
        raw.extend_from_slice(self.sk.verifying_key().as_bytes());
        B64.encode(raw)
    }

    fn sign(&self, msg: &[u8], comment: &str) -> String {
        let prehash = blake2::Blake2b512::digest(msg);
        let sig = self.sk.sign(&prehash).to_bytes();
        let mut line = b"ED".to_vec();
        line.extend_from_slice(&self.keynum);
        line.extend_from_slice(&sig);
        let mut global = sig.to_vec();
        global.extend_from_slice(comment.as_bytes());
        let global = self.sk.sign(&global).to_bytes();
        format!(
            "untrusted comment: signature from minisign secret key\n{}\ntrusted comment: {comment}\n{}\n",
            B64.encode(line),
            B64.encode(global)
        )
    }
}

#[derive(Clone, Default)]
struct Fake {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    seen: Arc<Mutex<Vec<String>>>,
}

async fn serve(AxState(fake): AxState<Fake>, req: Request) -> Response {
    let path = req.uri().path().to_owned();
    fake.seen.lock().unwrap().push(path.clone());
    match fake.files.lock().unwrap().get(&path) {
        Some(body) => body.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// What release v1.3.0 carries.
struct Release {
    image: Vec<u8>,
    sums: String,
    sig: Option<String>,
}

impl Release {
    /// Signed by `key` for v1.3.0, listing `image`'s sha256.
    fn signed(key: &TestKey) -> Self {
        let sums = sums_for(NEW);
        let sig = key.sign(sums.as_bytes(), &trusted_comment("architect", "v1.3.0"));
        Self {
            image: NEW.to_vec(),
            sums,
            sig: Some(sig),
        }
    }
}

fn sums_for(bytes: &[u8]) -> String {
    format!(
        "{}  Architect-macos-arm64.dmg\n{}  {LINUX_ASSET}\n",
        "b".repeat(64),
        Sha256::of(bytes)
    )
}

/// The releases: v1.3.0 as given, a newer v2.0.0 prerelease, older
/// v1.2.0 (the running one).
async fn start(release: Release) -> (String, Fake) {
    let fake = Fake::default();
    let app = axum::Router::new().fallback(serve).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let asset = |tag: &str, name: &str| {
        format!(r#"{{"name":"{name}","browser_download_url":"{base}/dl/{tag}/{name}"}}"#)
    };
    let mut assets = vec![asset("v1.3.0", LINUX_ASSET), asset("v1.3.0", SUMS_ASSET)];
    if release.sig.is_some() {
        assets.push(asset("v1.3.0", SIG_ASSET));
    }
    let pre = [
        asset("v2.0.0", LINUX_ASSET),
        asset("v2.0.0", SUMS_ASSET),
        asset("v2.0.0", SIG_ASSET),
    ];
    let list = format!(
        r#"[
          {{"tag_name":"v2.0.0","draft":false,"prerelease":true,"html_url":"{base}/r/v2.0.0","assets":[{}]}},
          {{"tag_name":"v1.3.0","draft":false,"prerelease":false,"html_url":"{base}/r/v1.3.0","assets":[{}]}},
          {{"tag_name":"v1.2.0","draft":false,"prerelease":false,"assets":[]}}
        ]"#,
        pre.join(","),
        assets.join(",")
    );
    {
        let mut files = fake.files.lock().unwrap();
        files.insert(
            "/repos/FastTrackStudios/architect/releases".into(),
            list.into_bytes(),
        );
        files.insert(format!("/dl/v1.3.0/{LINUX_ASSET}"), release.image);
        files.insert(
            format!("/dl/v1.3.0/{SUMS_ASSET}"),
            release.sums.into_bytes(),
        );
        if let Some(sig) = release.sig {
            files.insert(format!("/dl/v1.3.0/{SIG_ASSET}"), sig.into_bytes());
        }
    }
    (base, fake)
}

fn linux() -> Platform {
    Platform::new("linux", "x86_64")
}

fn engine(base: &str, key: &TestKey) -> Engine {
    Engine::new(&SOURCE, Some("1.2.0".into()), linux())
        .api_base(base)
        .key(Some(key.public()))
}

/// An `AppImage` at `<tmp>/apps/Architect.AppImage` holding [`OLD`].
fn appimage(tmp: &Path) -> PathBuf {
    let dir = tmp.join("apps");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Architect.AppImage");
    std::fs::write(&path, OLD).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

async fn install(
    engine: &Engine,
    plan: &Plan,
) -> (Result<State, architect_updates::engine::Failure>, u64) {
    let mut last = 0;
    let done = engine
        .install("1.3.0", plan, &mut |received, _| last = received)
        .await;
    (done, last)
}

#[tokio::test]
async fn a_newer_release_is_available() {
    let key = TestKey::new(7);
    let (base, _) = start(Release::signed(&key)).await;
    let env = Env {
        update_dir: Some("/tmp/never-written".into()),
        ..Env::default()
    };
    let state = engine(&base, &key).check(Channel::Stable, &env).await;
    assert_eq!(
        state,
        State::Available {
            version: "1.3.0".into(),
            notes_url: format!("{base}/r/v1.3.0"),
            signed: true,
            plan: Plan::UpdateDir {
                dir: "/tmp/never-written".into()
            },
        },
        "v2.0.0 is a prerelease: not on Stable"
    );
}

#[tokio::test]
async fn the_update_dir_gets_the_image_then_its_version_and_appimage_is_untouched() {
    let key = TestKey::new(7);
    let (base, _) = start(Release::signed(&key)).await;
    let tmp = tempfile::tempdir().unwrap();
    let image = appimage(tmp.path());
    let dir = tmp.path().join("data/architect/updates");
    let env = Env {
        update_dir: Some(dir.clone()),
        appimage: Some(image.clone()),
        appimage_writable: true,
    };
    let engine = engine(&base, &key);
    let plan = engine.plan(&env);
    assert_eq!(plan, Plan::UpdateDir { dir: dir.clone() });

    let (done, received) = install(&engine, &plan).await;
    assert_eq!(
        done.unwrap(),
        State::Ready {
            version: "1.3.0".into(),
            plan
        }
    );
    assert_eq!(received, NEW.len() as u64, "progress heard every byte");
    assert_eq!(std::fs::read(dir.join(LINUX_ASSET)).unwrap(), NEW);
    assert_eq!(mode(&dir.join(LINUX_ASSET)), 0o755);
    assert_eq!(
        std::fs::read_to_string(dir.join(VERSION_FILE)).unwrap(),
        "1.3.0\n"
    );
    assert_eq!(mode(&dir), 0o700, "the dir is made owner-only");
    assert_eq!(
        names(&dir),
        vec![LINUX_ASSET.to_owned(), VERSION_FILE.to_owned()]
    );
    assert_eq!(std::fs::read(&image).unwrap(), OLD, "$APPIMAGE untouched");
}

#[tokio::test]
async fn without_an_update_dir_the_appimage_is_replaced() {
    let key = TestKey::new(7);
    let (base, _) = start(Release::signed(&key)).await;
    let tmp = tempfile::tempdir().unwrap();
    let image = appimage(tmp.path());
    let env = Env {
        update_dir: None,
        appimage: Some(image.clone()),
        appimage_writable: true,
    };
    let engine = engine(&base, &key);
    let plan = engine.plan(&env);
    assert_eq!(
        plan,
        Plan::InPlace {
            appimage: image.clone()
        }
    );
    let (done, _) = install(&engine, &plan).await;
    assert!(matches!(done.unwrap(), State::Ready { .. }));
    assert_eq!(std::fs::read(&image).unwrap(), NEW);
    assert_eq!(mode(&image), 0o755);
    assert_eq!(names(image.parent().unwrap()), vec!["Architect.AppImage"]);
}

/// Each refusal leaves the update dir without an image or a version.
async fn refused(release: Release, key: &TestKey, want: &str) {
    let (base, _) = start(release).await;
    let tmp = tempfile::tempdir().unwrap();
    let image = appimage(tmp.path());
    let dir = tmp.path().join("updates");
    let env = Env {
        update_dir: Some(dir.clone()),
        appimage: Some(image.clone()),
        appimage_writable: true,
    };
    let engine = engine(&base, key);
    let plan = engine.plan(&env);
    let failure = install(&engine, &plan).await.0.unwrap_err();
    assert!(
        failure.reason.contains(want),
        "`{}` does not say `{want}`",
        failure.reason
    );
    assert!(failure.refused, "a verification refusal");
    assert_eq!(names(&dir), Vec::<String>::new(), "nothing written");
    assert_eq!(std::fs::read(&image).unwrap(), OLD);
}

#[tokio::test]
async fn an_unsigned_release_is_reported_and_refused() {
    let key = TestKey::new(7);
    let unsigned = Release {
        sig: None,
        ..Release::signed(&key)
    };
    let (base, _) = start(Release {
        sig: None,
        ..Release::signed(&key)
    })
    .await;
    let state = engine(&base, &key)
        .check(Channel::Stable, &Env::default())
        .await;
    assert!(
        matches!(state, State::Available { signed: false, .. }),
        "{state:?}"
    );
    refused(unsigned, &key, "v1.3.0 is not signed").await;
}

#[tokio::test]
async fn a_bad_signature_is_refused() {
    let key = TestKey::new(7);
    let other = TestKey::new(9);
    // Signed, but by another key than the one the app pins.
    refused(Release::signed(&other), &key, "not signed for this release").await;
}

#[tokio::test]
async fn a_checksum_mismatch_is_refused() {
    let key = TestKey::new(7);
    let release = Release {
        image: b"something else".to_vec(),
        ..Release::signed(&key)
    };
    refused(release, &key, "does not match its sha256").await;
}

#[tokio::test]
async fn a_prerelease_is_not_offered_on_stable() {
    let key = TestKey::new(7);
    let (base, _) = start(Release::signed(&key)).await;
    // Running 1.3.0: only the v2.0.0 prerelease is newer.
    let state = Engine::new(&SOURCE, Some("1.3.0".into()), linux())
        .api_base(&base)
        .check(Channel::Stable, &Env::default())
        .await;
    assert_eq!(
        state,
        State::UpToDate {
            newest: Some("1.3.0".into())
        }
    );
}

#[tokio::test]
async fn nightly_is_refused_until_it_exists() {
    let key = TestKey::new(7);
    let (base, fake) = start(Release::signed(&key)).await;
    let state = engine(&base, &key)
        .check(Channel::Nightly, &Env::default())
        .await;
    let State::Failed { reason } = state else {
        panic!("{state:?}");
    };
    assert!(reason.contains("not available yet"), "{reason}");
    assert!(fake.seen.lock().unwrap().is_empty(), "nothing asked");
}

#[tokio::test]
async fn without_an_appimage_the_update_is_reported_only() {
    let key = TestKey::new(7);
    let (base, fake) = start(Release::signed(&key)).await;
    let engine = engine(&base, &key);
    let state = engine.check(Channel::Stable, &Env::default()).await;
    assert!(
        matches!(
            &state,
            State::Available {
                plan: Plan::ReportOnly(ReportOnly::NoAppImage),
                ..
            }
        ),
        "{state:?}"
    );
    let plan = engine.plan(&Env::default());
    let asked = fake.seen.lock().unwrap().len();
    let failure = install(&engine, &plan).await.0.unwrap_err();
    assert!(
        failure.reason.contains("not running from an AppImage"),
        "{}",
        failure.reason
    );
    assert_eq!(fake.seen.lock().unwrap().len(), asked, "nothing downloaded");
}

#[tokio::test]
async fn a_development_build_never_asks_or_installs() {
    let key = TestKey::new(7);
    let (base, fake) = start(Release::signed(&key)).await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("updates");
    let dev = Engine::new(&SOURCE, None, linux())
        .api_base(&base)
        .key(Some(key.public()));
    let env = Env {
        update_dir: Some(dir.clone()),
        ..Env::default()
    };
    assert_eq!(
        dev.check(Channel::Stable, &env).await,
        State::DevelopmentBuild
    );
    assert_eq!(
        dev.plan(&env),
        Plan::ReportOnly(ReportOnly::DevelopmentBuild)
    );
    let failure = install(&dev, &Plan::UpdateDir { dir: dir.clone() })
        .await
        .0
        .unwrap_err();
    assert!(
        failure.reason.contains("development build"),
        "{}",
        failure.reason
    );
    assert!(fake.seen.lock().unwrap().is_empty(), "never asked the API");
    assert!(!dir.exists());
}
