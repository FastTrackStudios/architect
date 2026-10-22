//! Signed self-update: replace a running binary with a newer GitHub
//! Release, and only one that proves itself.
//!
//! An [`Updater`] names an app's releases (the repo, the tag prefix, this
//! platform's asset); [`Updater::run`] then does what Task's
//! `task-server self-update` did:
//!
//! 1. ask the releases API for the newest `<prefix>X.Y.Z` release that is
//!    neither a draft nor a prerelease and carries this platform's asset
//!    and [`SUMS_ASSET`] (or exactly the version asked for);
//! 2. check [`SUMS_ASSET`] is signed by the release key
//!    ([`SIG_ASSET`], minisign), and that the signature's trusted comment
//!    names this release ([`trusted_comment`]), so a valid signature
//!    lifted from an older release is refused;
//! 3. download the asset into a staging file beside the binary, checking
//!    its sha256 against the signed sums as it lands;
//! 4. run the staged binary with `--version` and require it to answer
//!    `<app> <version>`;
//! 5. rename it over the binary (one atomic step: the staging file is in
//!    the same directory), then run the optional `--then` hook.
//!
//! Every failure before the rename leaves the installed binary
//! byte-identical and removes the staging file.
//!
//! # The release key
//!
//! The fleet signs every release's `SHA256SUMS` with one org-wide minisign
//! key, kept as a release-workflow secret; each app compiles in the public
//! half and passes it as `key: Option<&str>` (the base64 line of a
//! minisign `.pub` file). The signature is what makes the checksums mean
//! anything: whoever can write a release's assets can replace both the
//! binary and its sums, so a sha256 check alone is integrity, not
//! authenticity.
//!
//! `None` is the explicit "this build pins no key" (a dev build). It is
//! never a silent skip: checking for updates still works, and installing
//! fails with [`ReleaseError::NoSigningKey`] before anything is
//! downloaded.
//!
//! # Platforms
//!
//! Unix: the running process keeps the old inode until it restarts, so
//! renaming over its file is safe. A binary under `/nix/store` is never
//! touched ([`ReleaseError::NixStore`]); a Nix host moves its flake pin
//! instead. On Windows the rename over a running `.exe` fails, which
//! leaves the old binary in place. macOS `.app` bundles (code signature,
//! Gatekeeper) are not handled here; a bare binary is.
//!
//! Extracted from Task's `apps/server/src/self_update.rs` (STA-387, I19).

use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use facet::Facet;
use sha2::Digest as _;

use crate::blob::Sha256;

/// The checksum file every release carries beside its binaries, in the
/// `sha256sum` format (`<hex>  <name>`).
pub const SUMS_ASSET: &str = "SHA256SUMS";

/// The minisign signature of [`SUMS_ASSET`], made by the release workflow
/// with the release key and a [`trusted_comment`].
pub const SIG_ASSET: &str = "SHA256SUMS.minisig";

/// The public GitHub API.
pub const GITHUB_API: &str = "https://api.github.com";

/// How long the staged binary may take to answer `--version`.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why an update did not happen. Every variant except
/// [`ReleaseError::Hook`] means the installed binary is unchanged.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    /// The build pins no release key (`key: None`), so it cannot tell a
    /// real release from swapped assets and installs nothing.
    #[error(
        "this build pins no release signing key, so it cannot tell a real release from swapped \
         assets and installs nothing; install a newer release by hand"
    )]
    NoSigningKey,
    /// The release carries no [`SIG_ASSET`]; only signed releases install.
    #[error("{tag} carries no {SIG_ASSET}: only signed releases are installed; nothing changed")]
    Unsigned { tag: String },
    /// The pinned key is not a minisign public key.
    #[error("the pinned release key does not decode: {0}")]
    BadKey(String),
    /// The signature does not decode, is not by the key, or names
    /// another release.
    #[error("{tag}'s {SUMS_ASSET} is not signed for this release: {reason}; nothing changed")]
    BadSignature { tag: String, reason: String },
    /// The signed sums list no hash for this platform's asset.
    #[error("{tag}'s {SUMS_ASSET} lists no sha256 for {asset}; nothing changed")]
    NotListed { tag: String, asset: String },
    /// The downloaded asset does not match its signed sha256.
    #[error(
        "{asset} from {tag} does not match its sha256 (got {got}, release says {want}); nothing changed"
    )]
    ChecksumMismatch {
        tag: String,
        asset: String,
        got: Sha256,
        want: Sha256,
    },
    /// The staged binary did not answer `--version` with the release's
    /// version.
    #[error("the downloaded binary says `{said}`, not `{expected}`; nothing changed")]
    SmokeTest { said: String, expected: String },
    /// The binary to replace lives in the Nix store.
    #[error("{0} is in the Nix store: a Nix host updates by moving its flake pin, not in place")]
    NixStore(PathBuf),
    /// No release carries the asset asked for (`--to`).
    #[error("no release {tag} carries {asset} and {SUMS_ASSET}")]
    NotFound { tag: String, asset: String },
    /// Talking to the releases server failed.
    #[error("{what}: {source}")]
    Http {
        what: String,
        #[source]
        source: reqwest::Error,
    },
    /// The releases API answered something that is not a release list.
    #[error("the releases API answered something else: {0}")]
    Decode(String),
    /// A local file could not be created, written, run or renamed.
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: io::Error,
    },
    /// Bad command-line arguments.
    #[error("{0}")]
    Usage(String),
    /// The new binary IS installed; the `--then` hook failed after it.
    #[error("{installed} is installed, but `{command}` failed after it ({status})")]
    Hook {
        installed: String,
        command: String,
        status: String,
    },
}

impl ReleaseError {
    fn io(what: impl Into<String>) -> impl FnOnce(io::Error) -> Self {
        let what = what.into();
        move |source| Self::Io { what, source }
    }

    fn http(what: impl Into<String>) -> impl FnOnce(reqwest::Error) -> Self {
        let what = what.into();
        move |source| Self::Http { what, source }
    }
}

// ── Versions ────────────────────────────────────────────────────────

/// `X.Y.Z`, three dot-separated runs of ASCII digits.
#[must_use]
pub fn is_release_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `1.2.3` (a leading `v` and any `-pre`/`+build` tail ignored) as
/// numbers; an unparsable part counts as 0.
fn version_key(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Whether `candidate` is a higher version than `current`, compared as
/// numbers (`0.2.10` is newer than `0.2.9`).
#[must_use]
pub fn newer(candidate: &str, current: &str) -> bool {
    version_key(candidate) > version_key(current)
}

// ── Signature and sums ──────────────────────────────────────────────

/// The trusted comment a release's [`SIG_ASSET`] must carry.
///
/// The release workflow signs with `minisign -t "<app> <tag> SHA256SUMS"`,
/// e.g. `task-server v0.2.4 SHA256SUMS`. The signature covers it, so a
/// signature lifted onto another release names the wrong tag.
#[must_use]
pub fn trusted_comment(app: &str, tag: &str) -> String {
    format!("{app} {tag} {SUMS_ASSET}")
}

/// Check `sums` is what `key` signed (`sig`, a minisign signature file)
/// with the trusted comment `expected_comment`.
pub fn verify_sums(
    key: &str,
    sums: &[u8],
    sig: &str,
    expected_comment: &str,
) -> Result<(), ReleaseError> {
    let bad = |reason: String| ReleaseError::BadSignature {
        tag: expected_comment.to_owned(),
        reason,
    };
    let key = minisign_verify::PublicKey::from_base64(key.trim())
        .map_err(|e| ReleaseError::BadKey(e.to_string()))?;
    let signature = minisign_verify::Signature::decode(sig)
        .map_err(|e| bad(format!("not a minisign signature ({e})")))?;
    key.verify(sums, &signature, false)
        .map_err(|e| bad(format!("not by the release key ({e})")))?;
    if signature.trusted_comment() != expected_comment {
        return Err(bad(format!(
            "made for `{}`, not `{expected_comment}`",
            signature.trusted_comment()
        )));
    }
    Ok(())
}

/// The sha256 a `sha256sum`-format file lists for `asset` (a `*` before
/// the name, binary mode, is accepted).
#[must_use]
pub fn expected_sha256(sums: &str, asset: &str) -> Option<Sha256> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset)
            .then(|| Sha256::from_hex(hash).ok())
            .flatten()
    })
}

// ── Releases ────────────────────────────────────────────────────────

/// One release as the GitHub releases API lists it (the fields used).
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct GithubRelease {
    pub tag_name: String,
    #[facet(default)]
    pub draft: bool,
    #[facet(default)]
    pub prerelease: bool,
    #[facet(default)]
    pub assets: Vec<GithubAsset>,
}

/// One asset of a [`GithubRelease`].
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Parse a `GET /repos/<repo>/releases` answer.
pub fn parse_releases(body: &[u8]) -> Result<Vec<GithubRelease>, ReleaseError> {
    facet_json::from_slice(body).map_err(|e| ReleaseError::Decode(e.to_string()))
}

/// A release that carries this platform's binary and its sums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The full tag (`v0.2.4`, `desktop-v1.0.0`).
    pub tag: String,
    /// The version in it (`0.2.4`).
    pub version: String,
    pub binary_url: String,
    pub sums_url: String,
    /// [`SIG_ASSET`], when the release carries one. An unsigned release is
    /// still reported by a check, and never installed.
    pub sig_url: Option<String>,
}

/// From a release list: the newest `<tag_prefix>X.Y.Z` release that is
/// neither a draft nor a prerelease and carries both `asset` and
/// [`SUMS_ASSET`]; or exactly version `want` (a leading `v` ignored).
#[must_use]
pub fn pick(
    releases: &[GithubRelease],
    tag_prefix: &str,
    asset: &str,
    want: Option<&str>,
) -> Option<Candidate> {
    let want = want.map(|w| w.trim_start_matches('v'));
    releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| candidate(r, tag_prefix, asset))
        .filter(|c| want.is_none_or(|w| w == c.version))
        .reduce(|best, c| {
            if newer(&c.version, &best.version) {
                c
            } else {
                best
            }
        })
}

fn candidate(release: &GithubRelease, tag_prefix: &str, asset: &str) -> Option<Candidate> {
    let version = release
        .tag_name
        .strip_prefix(tag_prefix)
        .filter(|v| is_release_version(v))?;
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    Some(Candidate {
        tag: release.tag_name.clone(),
        version: version.to_owned(),
        binary_url: url(asset)?,
        sums_url: url(SUMS_ASSET)?,
        sig_url: url(SIG_ASSET),
    })
}

/// The release asset name `<app>-<os>-<arch>` for this build's platform
/// (`task-server-linux-x86_64`, `task-server-macos-aarch64`), from
/// `std::env::consts`.
#[must_use]
pub fn platform_asset(app: &str) -> String {
    format!("{app}-{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The running binary, symlinks resolved: the file [`Updater::run`]
/// replaces.
pub fn current_exe() -> Result<PathBuf, ReleaseError> {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(ReleaseError::io("find this binary"))
}

// ── Options ─────────────────────────────────────────────────────────

/// What a `self-update` run should do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    /// Report only; change nothing.
    pub check: bool,
    /// Install exactly this version (also a downgrade), not the newest.
    pub to: Option<String>,
    /// Run after a new binary is installed, and only then.
    pub then: Vec<String>,
}

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    Run(Options),
    /// `-h` / `--help`: print [`usage`] and stop.
    Help,
}

impl Options {
    /// Parse `[--check] [--to <X.Y.Z>] [--then <command> [args…]]`;
    /// everything after `--then` is the hook.
    pub fn parse(args: &[String]) -> Result<Args, ReleaseError> {
        let mut o = Self::default();
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--check" | "--dry-run" => o.check = true,
                "--to" => o.to = Some(parse_to(rest.next())?),
                "--then" => {
                    o.then = rest.cloned().collect();
                    if o.then.is_empty() {
                        return Err(ReleaseError::Usage("--then needs a command".into()));
                    }
                    break;
                }
                "-h" | "--help" | "help" => return Ok(Args::Help),
                other => return Err(ReleaseError::Usage(format!("unknown option `{other}`"))),
            }
        }
        Ok(Args::Run(o))
    }
}

fn parse_to(value: Option<&String>) -> Result<String, ReleaseError> {
    let v = value.ok_or_else(|| ReleaseError::Usage("--to needs a version".into()))?;
    let v = v.trim_start_matches('v');
    if !is_release_version(v) {
        return Err(ReleaseError::Usage(format!(
            "--to takes a version like 0.2.3, not `{v}`"
        )));
    }
    Ok(v.to_owned())
}

/// The `self-update` help text for `app`.
#[must_use]
pub fn usage(app: &str) -> String {
    format!(
        "usage: {app} self-update [--check] [--to <X.Y.Z>] [--then <command> [args…]]

  --check            say whether a newer release exists; change nothing
  --to <X.Y.Z>       install that release instead of the newest (also downgrades)
  --then <cmd…>      after installing a new binary, run <cmd…> (everything after --then)

Replaces this binary with the newest release's {app} for this platform, after
checking the release's {SUMS_ASSET} is signed by the release key pinned in this
binary, the download against those checksums, and that it reports the release's
version. Binaries in /nix/store are never replaced (move the flake pin)."
    )
}

// ── Decision ────────────────────────────────────────────────────────

/// What a run did. `Display` is a one-line report for a CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No release carries this platform's asset yet.
    NoRelease { current: String },
    /// The running version is the newest (or newer).
    Current { current: String, newest: String },
    /// `--to` names the running version.
    AlreadyInstalled { current: String },
    /// `--check`: a newer release exists; nothing changed.
    Available { current: String, newest: String },
    /// A new binary was verified and swapped in (and the hook, if any,
    /// succeeded).
    Installed { from: String, to: String },
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRelease { current } => write!(
                f,
                "{current}: no release carries this platform's binary yet"
            ),
            Self::Current { current, newest } => {
                write!(f, "{current} is current (newest release: {newest})")
            }
            Self::AlreadyInstalled { current } => write!(f, "{current} is already installed"),
            Self::Available { current, newest } => {
                write!(f, "update available: {current} -> {newest}")
            }
            Self::Installed { from, to } => write!(f, "updated {from} -> {to}"),
        }
    }
}

/// What to do with the release [`pick`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to install; this is the answer.
    Done(Outcome),
    /// Download, verify and swap in this release.
    Install(Candidate),
}

/// Decide from the release found, the running version and the options.
/// Honest: no I/O.
pub fn decide(
    found: Option<Candidate>,
    current: &str,
    opts: &Options,
    asset: &str,
) -> Result<Decision, ReleaseError> {
    let current = current.to_owned();
    let Some(found) = found else {
        return opts.to.as_ref().map_or_else(
            || Ok(Decision::Done(Outcome::NoRelease { current })),
            |v| {
                Err(ReleaseError::NotFound {
                    tag: v.clone(),
                    asset: asset.to_owned(),
                })
            },
        );
    };
    let newest = found.version.clone();
    Ok(Decision::Done(match &opts.to {
        None if !newer(&newest, &current) => Outcome::Current { current, newest },
        Some(v) if *v == current => Outcome::AlreadyInstalled { current },
        _ if opts.check => Outcome::Available { current, newest },
        _ => return Ok(Decision::Install(found)),
    }))
}

// ── Updater ─────────────────────────────────────────────────────────

/// What a run replaces, and with whose key.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// The running version (`env!("CARGO_PKG_VERSION")`).
    pub current: &'a str,
    /// The binary to replace ([`current_exe`]).
    pub exe: &'a Path,
    /// The release key's public half (base64), or `None` for a build that
    /// pins none: checks still work, installs fail with
    /// [`ReleaseError::NoSigningKey`].
    pub key: Option<&'a str>,
}

/// One app's releases: where they are and which asset is this platform's.
#[derive(Debug, Clone)]
pub struct Updater {
    app: String,
    repo: String,
    asset: String,
    tag_prefix: String,
    api_base: String,
    token: Option<String>,
    http: Option<reqwest::Client>,
}

impl Updater {
    /// `app` is the binary's name, as its `--version` prints it and as the
    /// trusted comment names it; `repo` is `owner/name`; `asset` is this
    /// platform's release asset (usually [`platform_asset`]). Tags are
    /// `v<X.Y.Z>`, and the API is [`GITHUB_API`], unless set.
    #[must_use]
    pub fn new(app: impl Into<String>, repo: impl Into<String>, asset: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            repo: repo.into(),
            asset: asset.into(),
            tag_prefix: "v".into(),
            api_base: GITHUB_API.into(),
            token: None,
            http: None,
        }
    }

    /// Release tags are `<prefix>X.Y.Z` (default `v`; the Task desktop
    /// uses `desktop-v`). Other tags are ignored.
    #[must_use]
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    /// The releases API base (default [`GITHUB_API`]); a test points this
    /// at a local server.
    #[must_use]
    pub fn api_base(mut self, base: impl Into<String>) -> Self {
        base.into()
            .trim_end_matches('/')
            .clone_into(&mut self.api_base);
        self
    }

    /// A bearer token for the API (`GITHUB_TOKEN`, for the rate limit).
    /// It is only sent to URLs under the API base, never to asset
    /// downloads. Blank is none.
    #[must_use]
    pub fn token(mut self, token: Option<String>) -> Self {
        self.token = token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        self
    }

    /// Use this client instead of the default (30 s connect, 60 s between
    /// reads, 30 min overall).
    #[must_use]
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// The asset this updater installs.
    #[must_use]
    pub fn asset(&self) -> &str {
        &self.asset
    }

    /// Check for and (unless `opts.check`) install an update to
    /// `target.exe`, then run `opts.then` if a new binary went in.
    pub async fn run(&self, target: Target<'_>, opts: &Options) -> Result<Outcome, ReleaseError> {
        let http = self.http_client()?;
        let releases = self.releases(&http).await?;
        let found = pick(&releases, &self.tag_prefix, &self.asset, opts.to.as_deref());
        let candidate = match decide(found, target.current, opts, &self.asset)? {
            Decision::Done(outcome) => return Ok(outcome),
            Decision::Install(candidate) => candidate,
        };
        self.install(&http, &candidate, target).await?;
        run_hook(&opts.then, &candidate.tag).await?;
        Ok(Outcome::Installed {
            from: target.current.to_owned(),
            to: candidate.version,
        })
    }

    /// Verify `candidate` and swap it in over `target.exe`. On any error
    /// the binary there is unchanged.
    pub async fn install(
        &self,
        http: &reqwest::Client,
        candidate: &Candidate,
        target: Target<'_>,
    ) -> Result<(), ReleaseError> {
        let key = target.key.ok_or(ReleaseError::NoSigningKey)?;
        if target.exe.starts_with("/nix/store") {
            return Err(ReleaseError::NixStore(target.exe.to_owned()));
        }
        let tag = &candidate.tag;
        let sig_url = candidate
            .sig_url
            .as_deref()
            .ok_or_else(|| ReleaseError::Unsigned { tag: tag.clone() })?;
        let sums = self
            .get_bytes(http, &candidate.sums_url, SUMS_ASSET)
            .await?;
        let sig = self.get_bytes(http, sig_url, SIG_ASSET).await?;
        let sig = String::from_utf8_lossy(&sig);
        verify_sums(key, &sums, &sig, &trusted_comment(&self.app, tag))?;
        let want =
            expected_sha256(&String::from_utf8_lossy(&sums), &self.asset).ok_or_else(|| {
                ReleaseError::NotListed {
                    tag: tag.clone(),
                    asset: self.asset.clone(),
                }
            })?;

        let mut staged = staging_file(target.exe)?;
        let got = self
            .download(http, &candidate.binary_url, &mut staged)
            .await?;
        if got != want {
            return Err(ReleaseError::ChecksumMismatch {
                tag: tag.clone(),
                asset: self.asset.clone(),
                got,
                want,
            });
        }
        let expected = format!("{} {}", self.app, candidate.version);
        swap_in(staged, target.exe, &expected).await
    }

    fn http_client(&self) -> Result<reqwest::Client, ReleaseError> {
        if let Some(http) = &self.http {
            return Ok(http.clone());
        }
        reqwest::Client::builder()
            .user_agent(format!("{}-self-update", self.app))
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .timeout(Duration::from_mins(30))
            .build()
            .map_err(ReleaseError::http("build the HTTP client"))
    }

    async fn releases(&self, http: &reqwest::Client) -> Result<Vec<GithubRelease>, ReleaseError> {
        let url = format!("{}/repos/{}/releases?per_page=30", self.api_base, self.repo);
        parse_releases(&self.get_bytes(http, &url, "the release list").await?)
    }

    async fn get(
        &self,
        http: &reqwest::Client,
        url: &str,
        what: &str,
    ) -> Result<reqwest::Response, ReleaseError> {
        let mut req = http.get(url);
        if let Some(token) = self
            .token
            .as_ref()
            .filter(|_| url.starts_with(&self.api_base))
        {
            req = req.bearer_auth(token);
        }
        req.send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(ReleaseError::http(format!("download {what}")))
    }

    async fn get_bytes(
        &self,
        http: &reqwest::Client,
        url: &str,
        what: &str,
    ) -> Result<Vec<u8>, ReleaseError> {
        let response = self.get(http, url, what).await?;
        let body = response
            .bytes()
            .await
            .map_err(ReleaseError::http(format!("download {what}")))?;
        Ok(body.to_vec())
    }

    /// Stream the asset into `staged`, hashing it as it lands (the binary
    /// is never held in memory whole), and sync it.
    async fn download(
        &self,
        http: &reqwest::Client,
        url: &str,
        staged: &mut tempfile::NamedTempFile,
    ) -> Result<Sha256, ReleaseError> {
        let what = format!("download {}", self.asset);
        let mut response = self.get(http, url, &self.asset).await?;
        let mut hasher = sha2::Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(ReleaseError::http(&what))? {
            hasher.update(&chunk);
            staged.write_all(&chunk).map_err(ReleaseError::io(&what))?;
        }
        staged
            .as_file()
            .sync_all()
            .map_err(ReleaseError::io(&what))?;
        Ok(Sha256::from_bytes(hasher.finalize().into()))
    }
}

// ── Staging and swap ────────────────────────────────────────────────

/// A new, owner-only, randomly named file beside `exe` to download into.
///
/// In the same directory so the final rename is one atomic step on one
/// filesystem; under a random name created
/// exclusively (`O_EXCL`), so a file or symlink planted at a guessable
/// name fails the create instead of redirecting the write; owner-only
/// until it has proved itself. Removed when dropped, so every early
/// return cleans up.
pub fn staging_file(exe: &Path) -> Result<tempfile::NamedTempFile, ReleaseError> {
    let dir = exe.parent().ok_or_else(|| ReleaseError::Io {
        what: format!("{} has no directory", exe.display()),
        source: io::ErrorKind::InvalidInput.into(),
    })?;
    let name = exe
        .file_name()
        .map_or_else(|| "binary".into(), |n| n.to_string_lossy());
    let prefix = format!(".{name}.update-");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempfile_in(dir).map_err(ReleaseError::io(format!(
        "create a staging file in {} (is it writable by this user?)",
        dir.display()
    )))
}

/// Prove the staged binary answers `--version` with exactly `expected`
/// (`<app> <version>`), then rename it over `exe`. The staged file is
/// removed on any failure.
pub async fn swap_in(
    staged: tempfile::NamedTempFile,
    exe: &Path,
    expected: &str,
) -> Result<(), ReleaseError> {
    // Close the write handle first: Linux refuses to exec a file that is
    // open for writing (ETXTBSY). The path still deletes itself on drop.
    let staged = staged.into_temp_path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(ReleaseError::io("make the staged binary executable"))?;
    }
    let said = version_output(&staged).await?;
    if said.as_deref() != Some(expected) {
        return Err(ReleaseError::SmokeTest {
            said: said.unwrap_or_else(|| "nothing (it failed)".into()),
            expected: expected.to_owned(),
        });
    }
    staged.persist(exe).map_err(|e| ReleaseError::Io {
        what: format!("replace {}", exe.display()),
        source: e.error,
    })
}

/// What `<path> --version` prints (trimmed), or `None` if it exits
/// unsuccessfully or does not finish within [`SMOKE_TIMEOUT`].
///
/// A freshly closed file can still be "busy" for a moment while another
/// thread's fork holds its write descriptor (ETXTBSY), so that one spawn
/// error is retried briefly.
async fn version_output(path: &Path) -> Result<Option<String>, ReleaseError> {
    let mut busy_retries = 10_u32;
    loop {
        let child = tokio::process::Command::new(path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(SMOKE_TIMEOUT, child).await {
            Err(_elapsed) => return Ok(None),
            Ok(Ok(out)) => {
                return Ok(out
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()));
            }
            Ok(Err(e)) if e.kind() == io::ErrorKind::ExecutableFileBusy && busy_retries > 0 => {
                busy_retries = busy_retries.saturating_sub(1);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(Err(source)) => {
                return Err(ReleaseError::Io {
                    what: "run the downloaded binary".into(),
                    source,
                });
            }
        }
    }
}

/// Run the `--then` hook (if any) after `installed` went in.
async fn run_hook(then: &[String], installed: &str) -> Result<(), ReleaseError> {
    let Some((program, args)) = then.split_first() else {
        return Ok(());
    };
    let command = then.join(" ");
    let status = tokio::process::Command::new(program)
        .args(args)
        .status()
        .await
        .map_err(|e| ReleaseError::Hook {
            installed: installed.to_owned(),
            command: command.clone(),
            status: e.to_string(),
        })?;
    hook_result(status, installed, command)
}

fn hook_result(status: ExitStatus, installed: &str, command: String) -> Result<(), ReleaseError> {
    if status.success() {
        return Ok(());
    }
    Err(ReleaseError::Hook {
        installed: installed.to_owned(),
        command,
        status: status.to_string(),
    })
}
