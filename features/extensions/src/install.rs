//! Registries and what is installed from them (feature `host`).
//!
//! A [`Source`] is where packages come from: a registry's git repository
//! ([`GitRegistry`], cloned and pulled with the `git` program), a folder
//! with an `index.toml` ([`FolderRegistry`]: a registry being written, or
//! one on a shared drive), or a `ReaPack` repository ([`ReaPackSource`],
//! its `index.xml` and files fetched through a [`Fetch`] the app gives —
//! [`LocalFetch`] for `file://`, the app's HTTP client for the rest).
//!
//! [`Installs`] keeps what is installed, one folder an id under its root:
//! the package copied in, or — from a folder registry — a link to the
//! package where it is, so an author's edit there is what loads.
//!
//! A listing whose location is a signed `.archext` archive
//! ([`Location::Archive`]) is fetched as bytes ([`Fetched::Archive`]) and
//! opened by [`Installs`], never by the source: its signature must be by a
//! key [`Installs::trust`] or the source's [`Source::trust`] holds (the
//! org release key by default), its files must be the ones it signed, and
//! it must have a build for this platform. An archive from a file or a
//! peer goes through the same check ([`Installs::install_archive`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};

use semver::Version;
use serde::{Deserialize, Serialize};

pub use architect_extension_archive::{ArchiveError, Trust, Verified};

use crate::index::{Index, Listing, Location};
use crate::manifest::{MANIFEST, Manifest};

/// Why a source or an install failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The registry could not be reached or read.
    Source(String),
    /// `git` failed (its message), or is not there.
    Git(String),
    /// A file could not be read or written.
    Io(String),
    /// What came is not what was listed: another id or version, or no
    /// manifest.
    Mismatch(String),
    /// No source has it, at a version that fits.
    NotFound(String),
    /// The organization does not allow it.
    NotAllowed(String),
    /// Its archive was refused: unsigned, signed by a key not trusted
    /// here, changed after signing, or with no build for this platform.
    Archive(ArchiveError),
}

impl From<ArchiveError> for InstallError {
    fn from(e: ArchiveError) -> Self {
        Self::Archive(e)
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(e) => write!(f, "the registry: {e}"),
            Self::Git(e) => write!(f, "git: {e}"),
            Self::Io(e) | Self::NotAllowed(e) => f.write_str(e),
            Self::Mismatch(e) => write!(f, "not what the registry listed: {e}"),
            Self::NotFound(e) => write!(f, "no registry has {e}"),
            Self::Archive(e) => write!(f, "the archive is refused: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

fn io(what: &Path) -> impl Fn(std::io::Error) -> InstallError + '_ {
    move |e| InstallError::Io(format!("{}: {e}", what.display()))
}

/// How a source handed over a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// Its files are now in the folder it was given.
    Copied,
    /// It is used where it is (a folder registry's package).
    Linked(PathBuf),
    /// A signed archive's bytes, not yet opened: [`Installs`] checks its
    /// signature and writes its files.
    Archive(Vec<u8>),
}

/// Where packages come from.
pub trait Source: Send + Sync {
    /// What the person calls it (its index's name, or its URL).
    fn name(&self) -> String;

    /// Brings it up to date: pulls the repository, fetches the index.
    ///
    /// # Errors
    ///
    /// It cannot be reached.
    fn refresh(&self) -> Result<(), InstallError>;

    /// What it offers, as of the last refresh.
    ///
    /// # Errors
    ///
    /// The index cannot be read.
    fn index(&self) -> Result<Index, InstallError>;

    /// Puts `listing`'s files in `into` (an empty folder), or says where
    /// they are.
    ///
    /// # Errors
    ///
    /// They cannot be fetched.
    fn fetch(&self, listing: &Listing, into: &Path) -> Result<Fetched, InstallError>;

    /// The keys its archives may be signed by, past the ones the app
    /// trusts everywhere ([`Installs::trust`]): none unless it is told.
    fn trust(&self) -> Trust {
        Trust::none()
    }
}

/// Fetches a URL's bytes: what a `ReaPack` source downloads with.
pub trait Fetch: Send + Sync {
    /// # Errors
    ///
    /// It cannot be fetched.
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// `file://` URLs and plain paths.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFetch;

impl Fetch for LocalFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let path = url.strip_prefix("file://").unwrap_or(url);
        if path.contains("://") {
            return Err(format!(
                "{url}: only local files here; give the app's HTTP fetch"
            ));
        }
        std::fs::read(path).map_err(|e| format!("{url}: {e}"))
    }
}

/// The fetch a registry downloads `http(s)` archives with.
#[derive(Clone)]
struct Http(Option<Arc<dyn Fetch>>);

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Http(given)"
        } else {
            "Http(none)"
        })
    }
}

/// A registry in a folder: `index.toml` and the packages it lists by path.
/// Packages are linked, not copied: what is in the folder is what loads.
#[derive(Clone, Debug)]
pub struct FolderRegistry {
    root: PathBuf,
    trust: Trust,
    http: Http,
}

impl FolderRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            trust: Trust::none(),
            http: Http(None),
        }
    }

    /// Also trusts archives it lists signed by `key` (the base64 line of a
    /// minisign `.pub`), called `name`.
    #[must_use]
    pub fn trusting(mut self, name: impl Into<String>, key: impl Into<String>) -> Self {
        self.trust = self.trust.with(name, key);
        self
    }

    /// Fetches its `http(s)` archives with `fetch` (the app's HTTP
    /// client); without one only archives in the folder and `file://`
    /// ones are fetched.
    #[must_use]
    pub fn with_fetch(mut self, fetch: impl Fetch + 'static) -> Self {
        self.http = Http(Some(Arc::new(fetch)));
        self
    }

    /// An archive's bytes: a path in the registry, `file://`, or a URL
    /// through its fetch; its listed sha256 checked.
    fn archive(&self, url: &str, sha256: Option<&str>) -> Result<Fetched, InstallError> {
        let bytes = if url.contains("://") {
            match (&self.http.0, url.starts_with("file://")) {
                (_, true) => LocalFetch.get(url),
                (Some(fetch), false) => fetch.get(url),
                (None, false) => Err(format!(
                    "{url}: this registry has no HTTP fetch (`with_fetch`)"
                )),
            }
            .map_err(InstallError::Source)?
        } else {
            let path = self.package_dir(url)?;
            std::fs::read(&path).map_err(io(&path))?
        };
        check_sha256(url, &bytes, sha256)?;
        Ok(Fetched::Archive(bytes))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn read_index(&self) -> Result<Index, InstallError> {
        let path = self.root.join("index.toml");
        let text = std::fs::read_to_string(&path).map_err(io(&path))?;
        Index::parse(&text).map_err(|e| InstallError::Source(e.to_string()))
    }

    /// Where a listed path is in the folder, refusing one that leaves it.
    fn package_dir(&self, path: &str) -> Result<PathBuf, InstallError> {
        let rel = Path::new(path);
        if !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err(InstallError::Mismatch(format!(
                "{path} is outside the registry"
            )));
        }
        Ok(self.root.join(rel))
    }
}

impl Source for FolderRegistry {
    fn name(&self) -> String {
        self.read_index()
            .ok()
            .map(|i| i.name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    fn refresh(&self) -> Result<(), InstallError> {
        self.read_index().map(drop)
    }

    fn index(&self) -> Result<Index, InstallError> {
        self.read_index()
    }

    fn fetch(&self, listing: &Listing, into: &Path) -> Result<Fetched, InstallError> {
        match &listing.location {
            Location::Path(path) => Ok(Fetched::Linked(self.package_dir(path)?)),
            Location::Git { url, rev } => clone_at(url, rev, into).map(|()| Fetched::Copied),
            Location::Files(files) => fetch_files(&LocalFetch, files, into),
            Location::Archive { url, sha256 } => self.archive(url, sha256.as_deref()),
        }
    }

    fn trust(&self) -> Trust {
        self.trust.clone()
    }
}

/// Checks `bytes` are what the index pinned, when it pinned a hash.
fn check_sha256(url: &str, bytes: &[u8], want: Option<&str>) -> Result<(), InstallError> {
    use sha2::Digest as _;
    let Some(want) = want else { return Ok(()) };
    let got = hex::encode(sha2::Sha256::digest(bytes));
    if got.eq_ignore_ascii_case(want.trim()) {
        Ok(())
    } else {
        Err(InstallError::Mismatch(format!(
            "{url}'s sha256 is {got}, the index says {want}"
        )))
    }
}

/// A registry's git repository, cloned into a folder of the app's and
/// pulled on [`Source::refresh`]: what most registries are.
#[derive(Clone, Debug)]
pub struct GitRegistry {
    url: String,
    checkout: FolderRegistry,
}

impl GitRegistry {
    /// The registry at `url`, checked out in `checkout`.
    pub fn new(url: impl Into<String>, checkout: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            checkout: FolderRegistry::new(checkout),
        }
    }

    /// Also trusts archives it lists signed by `key` (the base64 line of a
    /// minisign `.pub`), called `name`.
    #[must_use]
    pub fn trusting(mut self, name: impl Into<String>, key: impl Into<String>) -> Self {
        self.checkout = self.checkout.trusting(name, key);
        self
    }

    /// Fetches its `http(s)` archives (release assets) with `fetch`.
    #[must_use]
    pub fn with_fetch(mut self, fetch: impl Fetch + 'static) -> Self {
        self.checkout = self.checkout.with_fetch(fetch);
        self
    }
}

impl Source for GitRegistry {
    fn name(&self) -> String {
        self.checkout
            .read_index()
            .ok()
            .map(|i| i.name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.url.clone())
    }

    fn refresh(&self) -> Result<(), InstallError> {
        let dir = self.checkout.root();
        if dir.join(".git").exists() {
            git(Some(dir), &["fetch", "--depth", "1", "origin"])?;
            git(Some(dir), &["reset", "--hard", "FETCH_HEAD"])
        } else {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            let target = dir.display().to_string();
            git(None, &["clone", "--depth", "1", &self.url, &target])
        }
    }

    fn index(&self) -> Result<Index, InstallError> {
        self.checkout.read_index()
    }

    fn fetch(&self, listing: &Listing, into: &Path) -> Result<Fetched, InstallError> {
        match &listing.location {
            Location::Path(path) => {
                copy_dir(&self.checkout.package_dir(path)?, into)?;
                Ok(Fetched::Copied)
            }
            Location::Git { url, rev } => clone_at(url, rev, into).map(|()| Fetched::Copied),
            Location::Files(files) => fetch_files(&LocalFetch, files, into),
            Location::Archive { url, sha256 } => self.checkout.archive(url, sha256.as_deref()),
        }
    }

    fn trust(&self) -> Trust {
        self.checkout.trust()
    }
}

/// A `ReaPack` repository: its `index.xml` at a URL, its files fetched.
pub struct ReaPackSource {
    url: String,
    fetch: Arc<dyn Fetch>,
    index: Mutex<Option<Index>>,
}

impl std::fmt::Debug for ReaPackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaPackSource")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl ReaPackSource {
    /// The repository whose index is at `url`, fetched with `fetch`.
    pub fn new(url: impl Into<String>, fetch: impl Fetch + 'static) -> Self {
        Self {
            url: url.into(),
            fetch: Arc::new(fetch),
            index: Mutex::new(None),
        }
    }
}

impl Source for ReaPackSource {
    fn name(&self) -> String {
        self.index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|i| i.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.url.clone())
    }

    fn refresh(&self) -> Result<(), InstallError> {
        let bytes = self.fetch.get(&self.url).map_err(InstallError::Source)?;
        let text = String::from_utf8(bytes).map_err(|e| InstallError::Source(e.to_string()))?;
        let index =
            crate::reapack::parse(&text).map_err(|e| InstallError::Source(e.to_string()))?;
        *self.index.lock().unwrap_or_else(PoisonError::into_inner) = Some(index);
        Ok(())
    }

    fn index(&self) -> Result<Index, InstallError> {
        if self
            .index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
        {
            self.refresh()?;
        }
        Ok(self
            .index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_default())
    }

    fn fetch(&self, listing: &Listing, into: &Path) -> Result<Fetched, InstallError> {
        match &listing.location {
            Location::Files(files) => fetch_files(self.fetch.as_ref(), files, into),
            Location::Archive { url, sha256 } => {
                let bytes = self.fetch.get(url).map_err(InstallError::Source)?;
                check_sha256(url, &bytes, sha256.as_deref())?;
                Ok(Fetched::Archive(bytes))
            }
            other => Err(InstallError::Mismatch(format!(
                "a ReaPack package lists files, not {other:?}"
            ))),
        }
    }
}

fn fetch_files(
    fetch: &dyn Fetch,
    files: &[(String, String)],
    into: &Path,
) -> Result<Fetched, InstallError> {
    for (url, file) in files {
        let rel = Path::new(file);
        if !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err(InstallError::Mismatch(format!(
                "{file} is outside the package"
            )));
        }
        let bytes = fetch.get(url).map_err(InstallError::Source)?;
        let path = into.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&path, bytes).map_err(io(&path))?;
    }
    Ok(Fetched::Copied)
}

/// Runs `git` (in `dir`), never asking for a password.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<(), InstallError> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| InstallError::Git(format!("cannot run git: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(InstallError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }
}

/// Clones `url` into `into` at `rev` (a tag, a branch or a commit), and
/// drops its `.git`.
fn clone_at(url: &str, rev: &str, into: &Path) -> Result<(), InstallError> {
    let target = into.display().to_string();
    git(None, &["clone", "--quiet", url, &target])?;
    git(Some(into), &["checkout", "--quiet", rev])?;
    let dot_git = into.join(".git");
    std::fs::remove_dir_all(&dot_git).map_err(io(&dot_git))
}

/// Copies the folder `from` into `to`, leaving out `.git`.
fn copy_dir(from: &Path, to: &Path) -> Result<(), InstallError> {
    std::fs::create_dir_all(to).map_err(io(to))?;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        let path = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }
        let dest = to.join(entry.file_name());
        if entry.file_type().map_err(io(&path))?.is_dir() {
            copy_dir(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(io(&path))?;
        }
    }
    Ok(())
}

/// What `installed.toml` records of an install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    id: String,
    version: String,
    kind: String,
    /// Where it came from (the source's name).
    source: String,
    /// The package's folder, when it is linked rather than copied.
    #[serde(default)]
    link: Option<PathBuf>,
    /// The versions an update may take (a semver range), when pinned.
    #[serde(default)]
    pin: Option<String>,
}

const RECORD: &str = "installed.toml";

/// One installed package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub id: String,
    pub version: Version,
    pub kind: String,
    pub source: String,
    /// Where its files are: its own folder, or the folder it links to.
    pub dir: PathBuf,
    pub linked: bool,
    /// The versions an update may take (`=1.2.0`, `~1.2`), when pinned:
    /// `EXT_UPDATE` takes none past it.
    pub pin: Option<String>,
}

impl Install {
    /// Whether it is an extension the host loads (not a `ReaPack` package
    /// of the app's own kinds).
    #[must_use]
    pub fn is_extension(&self) -> bool {
        matches!(self.kind.as_str(), "data" | "luau" | "wasm" | "process")
    }
}

/// What is installed, one folder an id under `root`.
#[derive(Clone, Debug)]
pub struct Installs {
    root: PathBuf,
    trust: Trust,
}

/// One archive, offered as a source: how a file's or a peer's archive is
/// installed through the same path as a registry's.
struct OneArchive {
    from: String,
    bytes: Vec<u8>,
    trust: Trust,
}

impl Source for OneArchive {
    fn name(&self) -> String {
        self.from.clone()
    }

    fn refresh(&self) -> Result<(), InstallError> {
        Ok(())
    }

    fn index(&self) -> Result<Index, InstallError> {
        Ok(Index::default())
    }

    fn fetch(&self, _: &Listing, _: &Path) -> Result<Fetched, InstallError> {
        Ok(Fetched::Archive(self.bytes.clone()))
    }

    fn trust(&self) -> Trust {
        self.trust.clone()
    }
}

/// The listing an opened archive is: its index's id, version, kind and
/// platforms.
fn listing_of(v: &Verified, manifest: &Manifest, from: &str) -> Listing {
    Listing {
        id: v.index.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        kind: v.index.kind.clone(),
        description: manifest.description.clone(),
        category: String::new(),
        location: Location::Archive {
            url: from.to_owned(),
            sha256: None,
        },
        platforms: v.index.platforms.clone(),
        changelog: String::new(),
    }
}

impl Installs {
    /// Installs under `root`, trusting archives signed by the org release
    /// key ([`Trust::org`]).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            trust: Trust::org(),
        }
    }

    /// Trusts `trust` (in place of the org key alone) for every archive,
    /// whichever source it comes from.
    #[must_use]
    pub fn with_trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// The keys an archive may be signed by, from any source.
    #[must_use]
    pub const fn trust(&self) -> &Trust {
        &self.trust
    }

    /// Opens `bytes` as an archive from `source` (its own keys trusted
    /// too): signed by a trusted key, its files the ones signed.
    ///
    /// # Errors
    ///
    /// [`InstallError::Archive`] with why it was refused.
    pub fn open_archive(
        &self,
        bytes: &[u8],
        source: &dyn Source,
    ) -> Result<Verified, InstallError> {
        Ok(architect_extension_archive::open(
            bytes,
            &self.trust.and(&source.trust()),
        )?)
    }

    /// Opens an archive that came as a file or from a peer, and the
    /// listing it is: what [`Self::install_archive`] would install.
    ///
    /// # Errors
    ///
    /// It is refused ([`InstallError::Archive`]), or its manifest is.
    pub fn read_archive(
        &self,
        bytes: &[u8],
        from: &str,
    ) -> Result<(Verified, Manifest, Listing), InstallError> {
        self.read_archive_trusting(bytes, from, &Trust::none())
    }

    /// [`Self::read_archive`], trusting `publishers` too (the keys the
    /// app's registries carry, `crate::registries`).
    ///
    /// # Errors
    ///
    /// It is refused ([`InstallError::Archive`]), or its manifest is.
    pub fn read_archive_trusting(
        &self,
        bytes: &[u8],
        from: &str,
        publishers: &Trust,
    ) -> Result<(Verified, Manifest, Listing), InstallError> {
        let v = self.open_archive(
            bytes,
            &OneArchive {
                from: from.to_owned(),
                bytes: Vec::new(),
                trust: publishers.clone(),
            },
        )?;
        let text = v
            .manifest()
            .ok_or_else(|| InstallError::Mismatch(format!("the archive has no {MANIFEST}")))?;
        let manifest = Manifest::parse(text).map_err(|e| InstallError::Mismatch(e.to_string()))?;
        let listing = listing_of(&v, &manifest, from);
        Ok((v, manifest, listing))
    }

    /// Installs an archive that came as a file or from a peer (`from`
    /// says which), checked as a registry's is: signed by a key
    /// [`Self::trust`] holds.
    ///
    /// # Errors
    ///
    /// It is refused, has no build for this platform, or cannot be
    /// written.
    pub fn install_archive(&self, bytes: Vec<u8>, from: &str) -> Result<Install, InstallError> {
        self.install_archive_trusting(bytes, from, &Trust::none())
    }

    /// [`Self::install_archive`], trusting `publishers` too.
    ///
    /// # Errors
    ///
    /// As [`Self::install_archive`].
    pub fn install_archive_trusting(
        &self,
        bytes: Vec<u8>,
        from: &str,
        publishers: &Trust,
    ) -> Result<Install, InstallError> {
        let (_, _, listing) = self.read_archive_trusting(&bytes, from, publishers)?;
        self.install(
            &OneArchive {
                from: from.to_owned(),
                bytes,
                trust: publishers.clone(),
            },
            &listing,
        )
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of `id`'s install.
    fn slot(&self, id: &str) -> PathBuf {
        self.root.join(slug(id))
    }

    /// Everything installed, by id.
    #[must_use]
    pub fn list(&self) -> Vec<Install> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out: Vec<Install> = entries
            .filter_map(Result::ok)
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| read_record(&e.path()))
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// `id`'s install.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Install> {
        read_record(&self.slot(id))
    }

    /// Installs `listing` from `source`, in place of the version there
    /// was. An extension's manifest must say the id and version listed.
    ///
    /// # Errors
    ///
    /// It cannot be fetched or written, or is not what was listed.
    pub fn install(&self, source: &dyn Source, listing: &Listing) -> Result<Install, InstallError> {
        std::fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let slot = self.slot(&listing.id);
        let incoming = self.root.join(format!(
            ".incoming-{}",
            slot.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        if incoming.exists() {
            std::fs::remove_dir_all(&incoming).map_err(io(&incoming))?;
        }
        let result = self.receive(source, listing, &incoming, &slot);
        if incoming.exists() {
            let _ = std::fs::remove_dir_all(&incoming);
        }
        result
    }

    fn receive(
        &self,
        source: &dyn Source,
        listing: &Listing,
        incoming: &Path,
        slot: &Path,
    ) -> Result<Install, InstallError> {
        let fetched = source.fetch(listing, incoming)?;
        let link = match fetched {
            Fetched::Copied => None,
            Fetched::Linked(dir) => Some(dir),
            Fetched::Archive(bytes) => {
                let v = self.open_archive(&bytes, source)?;
                if v.index.id != listing.id || v.index.version != listing.version.to_string() {
                    return Err(InstallError::Mismatch(format!(
                        "listed {} {}, its archive is {} {}",
                        listing.id, listing.version, v.index.id, v.index.version
                    )));
                }
                v.extract(incoming)?;
                None
            }
        };
        let files = link.clone().unwrap_or_else(|| incoming.to_path_buf());
        let is_extension = matches!(listing.kind.as_str(), "data" | "luau" | "wasm" | "process");
        if is_extension {
            let path = files.join(MANIFEST);
            let text = std::fs::read_to_string(&path).map_err(io(&path))?;
            let manifest =
                Manifest::parse(&text).map_err(|e| InstallError::Mismatch(e.to_string()))?;
            if manifest.id != listing.id || manifest.version != listing.version {
                return Err(InstallError::Mismatch(format!(
                    "listed {} {}, its manifest says {} {}",
                    listing.id, listing.version, manifest.id, manifest.version
                )));
            }
        }
        std::fs::create_dir_all(incoming).map_err(io(incoming))?;
        // A pin outlives the version it was made on.
        let pin = read_record(slot).and_then(|i| i.pin);
        let record = Record {
            id: listing.id.clone(),
            version: listing.version.to_string(),
            kind: listing.kind.clone(),
            source: source.name(),
            link,
            pin,
        };
        let text = toml::to_string(&record).map_err(|e| InstallError::Io(e.to_string()))?;
        let at = incoming.join(RECORD);
        std::fs::write(&at, text).map_err(io(&at))?;
        if slot.exists() {
            std::fs::remove_dir_all(slot).map_err(io(slot))?;
        }
        std::fs::rename(incoming, slot).map_err(io(slot))?;
        read_record(slot)
            .ok_or_else(|| InstallError::Io(format!("{} was not kept", slot.display())))
    }

    /// Pins `id`'s install to `versions` (a semver range), or unpins it.
    ///
    /// # Errors
    ///
    /// It is not installed, `versions` is not a range, or the record could
    /// not be written.
    pub fn set_pin(&self, id: &str, versions: Option<&str>) -> Result<Install, InstallError> {
        let slot = self.slot(id);
        let at = slot.join(RECORD);
        let text = std::fs::read_to_string(&at)
            .map_err(|_| InstallError::NotFound(format!("{id} is not installed")))?;
        let mut record: Record =
            toml::from_str(&text).map_err(|e| InstallError::Io(e.to_string()))?;
        if let Some(v) = versions {
            semver::VersionReq::parse(v)
                .map_err(|e| InstallError::NotFound(format!("version {v}: {e}")))?;
        }
        record.pin = versions.map(str::to_owned);
        let text = toml::to_string(&record).map_err(|e| InstallError::Io(e.to_string()))?;
        std::fs::write(&at, text).map_err(io(&at))?;
        read_record(&slot).ok_or_else(|| InstallError::Io(format!("{} was not kept", at.display())))
    }

    /// Removes `id`'s install: false when there was none. A linked one's
    /// folder is left where it is.
    ///
    /// # Errors
    ///
    /// It cannot be removed.
    pub fn uninstall(&self, id: &str) -> Result<bool, InstallError> {
        let slot = self.slot(id);
        if !slot.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&slot).map_err(io(&slot))?;
        Ok(true)
    }
}

/// `id` as a folder's name: what is not a letter, digit, `.`, `-` or `_`
/// is `_`.
pub(crate) fn slug(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn read_record(slot: &Path) -> Option<Install> {
    let text = std::fs::read_to_string(slot.join(RECORD)).ok()?;
    let record: Record = toml::from_str(&text).ok()?;
    Some(Install {
        version: Version::parse(&record.version).ok()?,
        linked: record.link.is_some(),
        dir: record.link.unwrap_or_else(|| slot.to_path_buf()),
        id: record.id,
        kind: record.kind,
        source: record.source,
        pin: record.pin,
    })
}
