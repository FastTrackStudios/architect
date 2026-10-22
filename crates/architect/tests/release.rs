//! `architect::release` (STA-387 acceptance), against a local fake
//! releases server: a good update swaps the binary and runs the `--then`
//! hook; a bad signature, a bad sum or a failing `--version` each leave
//! the old binary byte-identical (and no staging file behind). Plus the
//! honest pieces: release picking, the sums format, option parsing.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::option_if_let_else,
    clippy::significant_drop_tightening
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::blob::Sha256;
use architect::release::{
    Args, Options, Outcome, ReleaseError, SIG_ASSET, SUMS_ASSET, Target, Updater, expected_sha256,
    newer, parse_releases, pick, trusted_comment, verify_sums,
};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blake2::Digest as _;
use ed25519_dalek::{Signer as _, SigningKey};

const APP: &str = "myapp";
const ASSET: &str = "myapp-linux-x86_64";
const REPO: &str = "FastTrackStudios/myapp";
const OLD: &[u8] = b"#!/bin/sh\necho 'myapp 1.0.0'\n";
const NEW: &[u8] = b"#!/bin/sh\necho 'myapp 1.2.0'\n";

// ── A minisign signer for tests ─────────────────────────────────────

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

    /// The `.pub` file's base64 line.
    fn public(&self) -> String {
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&self.keynum);
        raw.extend_from_slice(self.sk.verifying_key().as_bytes());
        B64.encode(raw)
    }

    /// A `.minisig` file over `msg` (prehashed, `ED`) with `comment`.
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

fn sums_for(bytes: &[u8]) -> String {
    format!(
        "{}  myapp-linux-aarch64\n{}  {ASSET}\n",
        "b".repeat(64),
        Sha256::of(bytes)
    )
}

// ── The fake releases server ────────────────────────────────────────

#[derive(Clone, Default)]
struct Fake {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// `(path, had an Authorization header)` for every request.
    seen: Arc<Mutex<Vec<(String, bool)>>>,
}

async fn serve_file(State(fake): State<Fake>, req: Request) -> Response {
    let path = req.uri().path().to_owned();
    let auth = req.headers().contains_key("authorization");
    fake.seen.lock().unwrap().push((path.clone(), auth));
    match fake.files.lock().unwrap().get(&path) {
        Some(body) => body.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// One release `v1.2.0` of [`APP`] carrying `binary`, `sums` and (if
/// given) `sig`, plus an older `v1.1.0` and an unrelated tag.
struct Release<'a> {
    binary: &'a [u8],
    sums: String,
    sig: Option<String>,
}

async fn start(release: Release<'_>) -> (String, Fake) {
    let fake = Fake::default();
    let app = axum::Router::new()
        .fallback(serve_file)
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let asset = |tag: &str, name: &str| {
        format!(
            r#"{{"name":"{name}","browser_download_url":"{base}/dl/{tag}/{name}","size":1,"uploader":{{"login":"x"}}}}"#
        )
    };
    let mut new_assets = vec![asset("v1.2.0", ASSET), asset("v1.2.0", SUMS_ASSET)];
    if release.sig.is_some() {
        new_assets.push(asset("v1.2.0", SIG_ASSET));
    }
    let list = format!(
        r#"[
          {{"tag_name":"desktop-v9.0.0","draft":false,"prerelease":false,"assets":[{}]}},
          {{"tag_name":"v1.2.0","name":"v1.2.0","draft":false,"prerelease":false,"html_url":"x","assets":[{}]}},
          {{"tag_name":"v1.1.0","draft":false,"prerelease":false,"assets":[{}]}}
        ]"#,
        asset("desktop-v9.0.0", ASSET),
        new_assets.join(","),
        asset("v1.1.0", ASSET),
    );
    {
        let mut files = fake.files.lock().unwrap();
        files.insert(format!("/api/repos/{REPO}/releases"), list.into_bytes());
        files.insert(format!("/dl/v1.2.0/{ASSET}"), release.binary.to_vec());
        files.insert(
            format!("/dl/v1.2.0/{SUMS_ASSET}"),
            release.sums.into_bytes(),
        );
        if let Some(sig) = release.sig {
            files.insert(format!("/dl/v1.2.0/{SIG_ASSET}"), sig.into_bytes());
        }
    }
    (base, fake)
}

fn good_release(key: &TestKey) -> Release<'static> {
    let sums = sums_for(NEW);
    let sig = key.sign(sums.as_bytes(), &trusted_comment(APP, "v1.2.0"));
    Release {
        binary: NEW,
        sums,
        sig: Some(sig),
    }
}

// ── The installed binary ────────────────────────────────────────────

struct Installed {
    dir: tempfile::TempDir,
    exe: PathBuf,
}

fn installed() -> Installed {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(APP);
    std::fs::write(&exe, OLD).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    Installed { dir, exe }
}

impl Installed {
    /// The binary is exactly the old one, and nothing else is in its
    /// directory (the staging file was removed).
    fn assert_untouched(&self) {
        assert_eq!(std::fs::read(&self.exe).unwrap(), OLD, "old binary changed");
        assert_eq!(
            entries(self.dir.path()),
            vec![APP.to_owned()],
            "staging file left behind"
        );
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn updater(base: &str) -> Updater {
    // The API under `/api`, assets under `/dl`, as GitHub splits them
    // across hosts: the token must reach the first only.
    Updater::new(APP, REPO, ASSET).api_base(format!("{base}/api"))
}

async fn update(
    base: &str,
    at: &Installed,
    key: Option<&str>,
    opts: &Options,
) -> Result<Outcome, ReleaseError> {
    let target = Target {
        current: "1.0.0",
        exe: &at.exe,
        key,
    };
    updater(base).run(target, opts).await
}

// ── The acceptance cases ────────────────────────────────────────────

#[tokio::test]
async fn a_good_update_swaps_the_binary_and_runs_the_hook() {
    let key = TestKey::new(7);
    let (base, fake) = start(good_release(&key)).await;
    let at = installed();
    let marker = at.dir.path().join("hook-ran");
    let opts = Options {
        then: vec!["touch".into(), marker.display().to_string()],
        ..Options::default()
    };
    let target = Target {
        current: "1.0.0",
        exe: &at.exe,
        key: Some(&key.public()),
    };
    let outcome = updater(&base)
        .token(Some("secret".into()))
        .run(target, &opts)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Installed {
            from: "1.0.0".into(),
            to: "1.2.0".into()
        }
    );
    assert_eq!(std::fs::read(&at.exe).unwrap(), NEW);
    assert!(marker.exists(), "the --then hook ran");
    assert_eq!(
        entries(at.dir.path()),
        vec!["hook-ran".to_owned(), APP.to_owned()]
    );
    // The token goes to the API only, never to an asset download.
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().any(|(p, auth)| p.starts_with("/api/") && *auth));
    assert!(
        seen.iter()
            .filter(|(p, _)| p.starts_with("/dl/"))
            .all(|(_, auth)| !auth)
    );
}

#[tokio::test]
async fn a_bad_signature_leaves_the_old_binary() {
    let key = TestKey::new(7);
    let impostor = TestKey::new(9);
    // Signed by a key that is not the pinned one.
    let (base, _) = start(good_release(&impostor)).await;
    let at = installed();
    let err = update(&base, &at, Some(&key.public()), &Options::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ReleaseError::BadSignature { .. }), "{err}");
    at.assert_untouched();
}

#[tokio::test]
async fn swapped_sums_under_a_real_signature_leave_the_old_binary() {
    let key = TestKey::new(7);
    let mut release = good_release(&key);
    // Someone who can write the release's assets, but not sign.
    release.sums = sums_for(b"#!/bin/sh\necho 'myapp 1.2.0'\nrm -rf ~\n");
    let (base, _) = start(release).await;
    let at = installed();
    let err = update(&base, &at, Some(&key.public()), &Options::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ReleaseError::BadSignature { .. }), "{err}");
    at.assert_untouched();
}

#[tokio::test]
async fn a_signature_lifted_from_another_release_leaves_the_old_binary() {
    let key = TestKey::new(7);
    let mut release = good_release(&key);
    release.sig = Some(key.sign(release.sums.as_bytes(), &trusted_comment(APP, "v1.1.0")));
    let (base, _) = start(release).await;
    let at = installed();
    let err = update(&base, &at, Some(&key.public()), &Options::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("made for `myapp v1.1.0"), "{err}");
    at.assert_untouched();
}

#[tokio::test]
async fn a_bad_sum_leaves_the_old_binary() {
    let key = TestKey::new(7);
    let mut release = good_release(&key);
    // The signed sums are right; the asset served is not what they list.
    release.binary = b"#!/bin/sh\necho 'myapp 1.2.0'\n# tampered\n";
    let (base, _) = start(release).await;
    let at = installed();
    let err = update(&base, &at, Some(&key.public()), &Options::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReleaseError::ChecksumMismatch { .. }),
        "{err}"
    );
    at.assert_untouched();
}

#[tokio::test]
async fn a_failing_version_check_leaves_the_old_binary() {
    let key = TestKey::new(7);
    for binary in [
        &b"#!/bin/sh\necho 'myapp 0.0.1'\n"[..], // lies about its version
        &b"#!/bin/sh\necho 'myapp 1.2.0'\nexit 3\n"[..], // fails
    ] {
        let sums = sums_for(binary);
        let sig = key.sign(sums.as_bytes(), &trusted_comment(APP, "v1.2.0"));
        let (base, _) = start(Release {
            binary,
            sums,
            sig: Some(sig),
        })
        .await;
        let at = installed();
        let err = update(&base, &at, Some(&key.public()), &Options::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ReleaseError::SmokeTest { .. }), "{err}");
        at.assert_untouched();
    }
}

#[tokio::test]
async fn no_key_checks_but_never_installs() {
    let key = TestKey::new(7);
    let (base, fake) = start(good_release(&key)).await;
    let at = installed();
    let check = Options {
        check: true,
        ..Options::default()
    };
    assert_eq!(
        update(&base, &at, None, &check).await.unwrap(),
        Outcome::Available {
            current: "1.0.0".into(),
            newest: "1.2.0".into()
        }
    );
    let err = update(&base, &at, None, &Options::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ReleaseError::NoSigningKey), "{err}");
    at.assert_untouched();
    let seen = fake.seen.lock().unwrap().clone();
    assert!(
        seen.iter().all(|(p, _)| !p.starts_with("/dl/")),
        "nothing downloaded"
    );
}

#[tokio::test]
async fn an_unsigned_release_is_reported_but_not_installed() {
    let key = TestKey::new(7);
    let mut release = good_release(&key);
    release.sig = None;
    let (base, _) = start(release).await;
    let at = installed();
    let err = update(&base, &at, Some(&key.public()), &Options::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ReleaseError::Unsigned { .. }), "{err}");
    at.assert_untouched();
}

#[tokio::test]
async fn current_and_pinned_versions_change_nothing() {
    let key = TestKey::new(7);
    let (base, _) = start(good_release(&key)).await;
    let at = installed();
    let target = |current| Target {
        current,
        exe: &at.exe,
        key: None,
    };
    let up = updater(&base);
    assert!(matches!(
        up.run(target("1.2.0"), &Options::default()).await.unwrap(),
        Outcome::Current { .. }
    ));
    let pin = |v: &str| Options {
        to: Some(v.into()),
        ..Options::default()
    };
    assert!(matches!(
        up.run(target("1.2.0"), &pin("1.2.0")).await.unwrap(),
        Outcome::AlreadyInstalled { .. }
    ));
    assert!(matches!(
        up.run(target("1.2.0"), &pin("3.0.0")).await.unwrap_err(),
        ReleaseError::NotFound { .. }
    ));
    at.assert_untouched();
}

// ── The honest pieces ───────────────────────────────────────────────

#[test]
fn picks_the_newest_release_with_the_binary_and_its_sums() {
    let rel = |tag: &str, names: &[&str], draft: bool| {
        let assets: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"name":"{n}","browser_download_url":"https://x/{tag}/{n}"}}"#))
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":false,"assets":[{}]}}"#,
            assets.join(",")
        )
    };
    let body = format!(
        "[{}]",
        [
            rel("desktop-v9.9.9", &[ASSET, SUMS_ASSET], false),
            rel("v0.2.10", &[ASSET, SUMS_ASSET], false),
            rel("v0.2.9", &[ASSET, SUMS_ASSET, SIG_ASSET], false),
            rel("v0.3.0", &[ASSET], false),               // no sums
            rel("v0.4.0", &["other", SUMS_ASSET], false), // other platform
            rel("v1.0.0-rc.1", &[ASSET, SUMS_ASSET], false),
            rel("v5.0.0", &[ASSET, SUMS_ASSET], true), // draft
        ]
        .join(",")
    );
    let releases = parse_releases(body.as_bytes()).unwrap();
    let c = pick(&releases, "v", ASSET, None).unwrap();
    assert_eq!((c.tag.as_str(), c.version.as_str()), ("v0.2.10", "0.2.10"));
    assert_eq!(c.sig_url, None, "an unsigned release is still found");
    let c = pick(&releases, "v", ASSET, Some("v0.2.9")).unwrap();
    assert_eq!(
        c.sig_url.as_deref(),
        Some("https://x/v0.2.9/SHA256SUMS.minisig")
    );
    assert!(pick(&releases, "v", ASSET, Some("0.3.0")).is_none());
    assert_eq!(
        pick(&releases, "desktop-v", ASSET, None).unwrap().version,
        "9.9.9"
    );
    assert!(newer("0.2.10", "0.2.9") && !newer("0.2.3", "0.2.3"));
}

#[test]
fn reads_the_sha256sum_format() {
    let h = "a".repeat(64);
    let sums = format!(
        "{}  x-aarch64\n{h}  {ASSET}\n{} *x-macos\n",
        "b".repeat(64),
        "C".repeat(64)
    );
    assert_eq!(expected_sha256(&sums, ASSET).unwrap().to_hex(), h);
    assert_eq!(
        expected_sha256(&sums, "x-macos").unwrap().to_hex(),
        "c".repeat(64)
    );
    assert!(expected_sha256(&sums, "x-windows").is_none());
    assert!(expected_sha256(&format!("nothex  {ASSET}"), ASSET).is_none());
}

#[test]
fn verify_sums_checks_key_body_and_comment() {
    let key = TestKey::new(3);
    let sums = sums_for(NEW);
    let comment = trusted_comment(APP, "v1.2.0");
    assert_eq!(comment, "myapp v1.2.0 SHA256SUMS");
    let sig = key.sign(sums.as_bytes(), &comment);
    verify_sums(&key.public(), sums.as_bytes(), &sig, &comment).unwrap();
    assert!(verify_sums(&key.public(), b"other", &sig, &comment).is_err());
    assert!(verify_sums(&TestKey::new(4).public(), sums.as_bytes(), &sig, &comment).is_err());
    // The trusted comment edited to match: the global signature covers it.
    let lifted = key.sign(sums.as_bytes(), &trusted_comment(APP, "v1.1.0"));
    let edited = lifted.replace("v1.1.0", "v1.2.0");
    assert!(verify_sums(&key.public(), sums.as_bytes(), &edited, &comment).is_err());
    assert!(matches!(
        verify_sums("not a key", sums.as_bytes(), &sig, &comment),
        Err(ReleaseError::BadKey(_))
    ));
}

#[test]
fn options_parse_and_then_takes_the_rest() {
    let a = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        Options::parse(&a(&[
            "--check",
            "--to",
            "v0.2.4",
            "--then",
            "systemctl",
            "try-restart",
            "--x"
        ]))
        .unwrap(),
        Args::Run(Options {
            check: true,
            to: Some("0.2.4".into()),
            then: a(&["systemctl", "try-restart", "--x"]),
        })
    );
    assert_eq!(Options::parse(&a(&["--help"])).unwrap(), Args::Help);
    assert!(Options::parse(&a(&["--to", "latest"])).is_err());
    assert!(Options::parse(&a(&["--to"])).is_err());
    assert!(Options::parse(&a(&["--then"])).is_err());
    assert!(Options::parse(&a(&["--force"])).is_err());
}
