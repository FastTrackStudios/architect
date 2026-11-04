//! The `.archext` extension archive: how an extension travels from the
//! repository that builds it to a person's app, whichever way it comes (a
//! registry, a file, a peer).
//!
//! An archive is a gzipped tar (`tar tzf x.archext` lists it) of:
//!
//! - [`INDEX`] (`archext.json`), the content index ([`ContentIndex`]): the
//!   extension's id, version and kind, the platforms it has builds for, and
//!   every other file with its size and SHA-256;
//! - [`SIGNATURE`] (`archext.json.minisig`), a detached minisign signature
//!   over the index's bytes;
//! - the extension's files: `extension.toml`, its Luau, its `.wasm`
//!   component, its pages and assets, and for a process extension one
//!   program per platform under `bin/<target-triple>/`.
//!
//! [`open`] checks an archive before anything of it is written: the
//! signature is by a key the app trusts ([`Trust`], the org key by
//! default), the index is the one signed, every file is listed with its
//! hash and every listed file is there. An unsigned, tampered or
//! wrong-key archive is refused with an [`ArchiveError`] that says which.
//! [`Verified::extract`] then writes the files for this platform:
//! `bin/<host-triple>/x` lands at `bin/x`, the other platforms' builds
//! are left out, and an archive with builds but none for this platform is
//! refused ([`ArchiveError::NoBuild`]).
//!
//! [`pack`] makes the unsigned archive from a folder; [`sign::sign_archive`]
//! (feature `sign`) adds the signature. `cargo xtask ext-pack` and
//! `cargo xtask ext-sign` are those two, for the release workflow and for
//! people.

use std::io::{Read as _, Write as _};
use std::path::{Component, Path};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

pub mod registry;
#[cfg(feature = "sign")]
pub mod sign;

/// An archive's file extension.
pub const EXTENSION: &str = "archext";

/// The content index's name in an archive.
pub const INDEX: &str = "archext.json";

/// The signature's name in an archive: minisign, over [`INDEX`]'s bytes.
pub const SIGNATURE: &str = "archext.json.minisig";

/// The content index's format.
pub const FORMAT: u32 = 1;

/// The manifest's name (`architect_extensions::manifest::MANIFEST`).
pub const MANIFEST: &str = "extension.toml";

/// The `FastTrackStudios` org release key's public half: the base64 line of
/// its minisign `.pub`. Its secret half signs every FTS release (the
/// `release-signing` environments); [`Trust::org`] trusts it.
pub const ORG_KEY: &str = "RWRh9Gk9qaStQMhku4THMidEYrQsmAEsm2dDprUkZxShEtqfIcroM8Od";

/// The most an archive may unpack to.
pub const MAX_UNPACKED: u64 = 1 << 30;

/// The most files an archive may hold.
pub const MAX_FILES: usize = 20_000;

/// One file of an archive, as its index lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Its path in the package, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

/// `archext.json`: what the archive is and what is in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentIndex {
    /// [`FORMAT`].
    pub format: u32,
    pub id: String,
    pub version: String,
    /// `data`, `luau`, `wasm`, `process`.
    pub kind: String,
    /// The target triples it has builds for (`bin/<triple>/`), sorted;
    /// empty for a platform-neutral one (data, Luau, a WASI component).
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Every file but the index and the signature, sorted by path.
    pub files: Vec<FileEntry>,
}

impl ContentIndex {
    /// Its bytes, as signed: pretty JSON with a final newline.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec_pretty(self).unwrap_or_default();
        out.push(b'\n');
        out
    }
}

/// The trusted comment a signature carries: `archext <id> <version>`.
#[must_use]
pub fn trusted_comment(id: &str, version: &str) -> String {
    format!("{EXTENSION} {id} {version}")
}

/// Why an archive was not packed, signed or opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveError {
    /// It is not a gzipped tar, or it is cut short.
    NotAnArchive(String),
    /// An entry would land outside the package, or is not a plain file
    /// (a link, a device).
    Unsafe { path: String },
    /// An entry is there twice.
    Duplicate { path: String },
    /// It unpacks to more than [`MAX_UNPACKED`] or holds more than
    /// [`MAX_FILES`] files.
    TooLarge(String),
    /// It has no [`INDEX`].
    NoIndex,
    /// The index does not read, or is of another format.
    BadIndex(String),
    /// It carries no [`SIGNATURE`]: only signed archives install.
    Unsigned { id: String, version: String },
    /// The signature file does not decode.
    BadSignatureFile(String),
    /// The signature is by a key the app does not trust.
    UntrustedKey {
        /// The signer's key id (minisign's, hex).
        key_id: String,
        /// The names of the keys trusted here.
        trusted: Vec<String>,
    },
    /// The signature is by a trusted key but the index is not what it
    /// signed: the index was changed after signing.
    IndexTampered { signer: String },
    /// A file's bytes are not what the signed index says.
    FileTampered {
        path: String,
        want: String,
        got: String,
    },
    /// A file is in the archive but not in the signed index.
    Unlisted { path: String },
    /// The signed index lists a file the archive does not hold.
    Missing { path: String },
    /// It has builds, none for this platform.
    NoBuild { host: String, has: Vec<String> },
    /// Packing: the folder's manifest is missing or does not say who it is.
    Manifest(String),
    /// Signing: the key does not read, or is encrypted.
    Key(String),
    /// A file could not be read or written.
    Io(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnArchive(e) => write!(f, "not an .{EXTENSION} archive (a gzipped tar): {e}"),
            Self::Unsafe { path } => write!(
                f,
                "{path} is not a plain file inside the package: the archive is refused"
            ),
            Self::Duplicate { path } => write!(f, "{path} is in the archive twice"),
            Self::TooLarge(e) => write!(f, "the archive is too large: {e}"),
            Self::NoIndex => write!(f, "the archive has no {INDEX} (its content index)"),
            Self::BadIndex(e) => write!(f, "the archive's {INDEX} does not read: {e}"),
            Self::Unsigned { id, version } => write!(
                f,
                "{id} {version} is unsigned (no {SIGNATURE}): only signed archives install"
            ),
            Self::BadSignatureFile(e) => {
                write!(f, "the archive's {SIGNATURE} is not a minisign signature: {e}")
            }
            Self::UntrustedKey { key_id, trusted } => write!(
                f,
                "signed by key {key_id}, which this app does not trust (it trusts: {})",
                if trusted.is_empty() {
                    "no key".to_owned()
                } else {
                    trusted.join(", ")
                }
            ),
            Self::IndexTampered { signer } => write!(
                f,
                "the archive's {INDEX} is not what {signer} signed: it was changed after signing"
            ),
            Self::FileTampered { path, want, got } => write!(
                f,
                "{path} was changed after signing (its sha256 is {got}, the signed index says {want})"
            ),
            Self::Unlisted { path } => {
                write!(f, "{path} is in the archive but not in its signed index")
            }
            Self::Missing { path } => {
                write!(f, "the signed index lists {path}, which the archive lacks")
            }
            Self::NoBuild { host, has } => write!(
                f,
                "no build for this platform ({host}); it has builds for {}",
                has.join(", ")
            ),
            Self::Manifest(e) => write!(f, "{MANIFEST}: {e}"),
            Self::Key(e) => write!(f, "the signing key: {e}"),
            Self::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ArchiveError {}

fn io(what: &Path) -> impl Fn(std::io::Error) -> ArchiveError + '_ {
    move |e| ArchiveError::Io(format!("{}: {e}", what.display()))
}

/// A key the app trusts, and what it calls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedKey {
    pub name: String,
    /// The base64 line of a minisign `.pub`.
    pub key: String,
}

/// The keys an archive's signature may be by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trust {
    keys: Vec<TrustedKey>,
}

impl Trust {
    /// The org release key ([`ORG_KEY`]): what an app trusts unless told
    /// otherwise.
    #[must_use]
    pub fn org() -> Self {
        Self::none().with("the FastTrackStudios release key", ORG_KEY)
    }

    /// No key: every archive is refused.
    #[must_use]
    pub const fn none() -> Self {
        Self { keys: Vec::new() }
    }

    /// Also trusts `key` (the base64 line of a minisign `.pub`), called
    /// `name`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, key: impl Into<String>) -> Self {
        let key = key.into().trim().to_owned();
        if !self.keys.iter().any(|k| k.key == key) {
            self.keys.push(TrustedKey {
                name: name.into(),
                key,
            });
        }
        self
    }

    /// The keys of both.
    #[must_use]
    pub fn and(&self, other: &Self) -> Self {
        other
            .keys
            .iter()
            .fold(self.clone(), |t, k| t.with(k.name.clone(), k.key.clone()))
    }

    #[must_use]
    pub fn keys(&self) -> &[TrustedKey] {
        &self.keys
    }
}

/// minisign's key id of a public key or a signature line (bytes 2..10 of
/// the decoded line), as minisign prints it.
fn key_id(b64_line: &str) -> Option<String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64_line.trim())
        .ok()?;
    let id: [u8; 8] = raw.get(2..10)?.try_into().ok()?;
    Some(format!("{:016X}", u64::from_le_bytes(id)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Whether `path` is a plain relative path of normal components.
fn is_inside(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// The files of an archive, in its order, unchecked.
///
/// # Errors
///
/// It is not a gzipped tar, an entry is not a plain file inside the
/// package, an entry is there twice, or it is too large.
pub fn entries(archive: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
    let bad = |e: std::io::Error| ArchiveError::NotAnArchive(e.to_string());
    // The whole stream first, so its CRC is checked and nothing after the
    // tar's end goes unread; at most MAX_UNPACKED (and the tar's blocks).
    let mut tar_bytes = Vec::new();
    let limit = MAX_UNPACKED.saturating_add(MAX_UNPACKED / 8);
    flate2::read::GzDecoder::new(archive)
        .take(limit.saturating_add(1))
        .read_to_end(&mut tar_bytes)
        .map_err(bad)?;
    if u64::try_from(tar_bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(ArchiveError::TooLarge(format!(
            "more than {MAX_UNPACKED} bytes unpacked"
        )));
    }
    let mut tar = tar::Archive::new(tar_bytes.as_slice());
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total: u64 = 0;
    for entry in tar.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        let path = entry
            .path()
            .map_err(bad)?
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            if !is_inside(&path) {
                return Err(ArchiveError::Unsafe { path });
            }
            continue;
        }
        if !kind.is_file() || !is_inside(&path) {
            return Err(ArchiveError::Unsafe { path });
        }
        if out.iter().any(|(p, _)| *p == path) {
            return Err(ArchiveError::Duplicate { path });
        }
        if out.len() >= MAX_FILES {
            return Err(ArchiveError::TooLarge(format!("more than {MAX_FILES} files")));
        }
        let size = entry.header().size().map_err(bad)?;
        total = total.saturating_add(size);
        if total > MAX_UNPACKED {
            return Err(ArchiveError::TooLarge(format!(
                "more than {MAX_UNPACKED} bytes unpacked"
            )));
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(size)
            .read_to_end(&mut bytes)
            .map_err(bad)?;
        out.push((path, bytes));
    }
    Ok(out)
}

/// Whether `path` is a program for a platform: `bin/<triple>/…`.
fn platform_of(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("bin/")?;
    let (triple, file) = rest.split_once('/')?;
    (!file.is_empty() && triple.contains('-')).then_some(triple)
}

/// A gzipped tar of `files`, in order: plain files, mode 0644 (0755 under
/// `bin/`), owned by no one, dated 0, so the same files make the same
/// bytes.
///
/// # Errors
///
/// A path is not a plain relative path.
pub fn write_entries(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ArchiveError> {
    let gz = flate2::GzBuilder::new().write(Vec::new(), flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    let fail = |e: std::io::Error| ArchiveError::Io(format!("writing the archive: {e}"));
    for (path, bytes) in files {
        if !is_inside(path) {
            return Err(ArchiveError::Unsafe { path: path.clone() });
        }
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        header.set_mode(if path.starts_with("bin/") { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        tar.append_data(&mut header, path, bytes.as_slice())
            .map_err(fail)?;
    }
    let gz = tar.into_inner().map_err(fail)?;
    gz.finish().map_err(fail)
}

#[derive(Deserialize)]
struct ManifestHead {
    id: String,
    version: String,
    kind: String,
    #[serde(default)]
    entry: Option<EntryHead>,
}

#[derive(Deserialize)]
struct EntryHead {
    #[serde(default)]
    path: String,
}

/// Reads every file under `dir`, `/`-separated paths, sorted; leaving out
/// `.git`, `.DS_Store` and an index or signature already there.
fn read_tree(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), ArchiveError> {
        for entry in std::fs::read_dir(dir).map_err(io(dir))? {
            let entry = entry.map_err(io(dir))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                continue;
            }
            let path = entry.path();
            let kind = entry.file_type().map_err(io(&path))?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| ArchiveError::Io(e.to_string()))?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if rel == INDEX || rel == SIGNATURE {
                    continue;
                }
                out.push((rel, std::fs::read(&path).map_err(io(&path))?));
            } else {
                return Err(ArchiveError::Unsafe {
                    path: path.display().to_string(),
                });
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The content index of `files` (the manifest among them).
///
/// # Errors
///
/// No manifest, or it does not say its id, version and kind; a process
/// extension with builds lacks its program in one of them.
pub fn index_of(files: &[(String, Vec<u8>)]) -> Result<ContentIndex, ArchiveError> {
    let manifest = files
        .iter()
        .find(|(p, _)| p == MANIFEST)
        .ok_or_else(|| ArchiveError::Manifest("the package has none".to_owned()))?;
    let text = std::str::from_utf8(&manifest.1).map_err(|e| ArchiveError::Manifest(e.to_string()))?;
    let head: ManifestHead =
        toml::from_str(text).map_err(|e| ArchiveError::Manifest(e.to_string()))?;
    semver::Version::parse(&head.version)
        .map_err(|e| ArchiveError::Manifest(format!("version {}: {e}", head.version)))?;
    let mut platforms: Vec<String> = files
        .iter()
        .filter_map(|(p, _)| platform_of(p).map(str::to_owned))
        .collect();
    platforms.sort();
    platforms.dedup();
    // A process extension's program (`[entry] path = "bin/tool"`) is in
    // every platform's folder.
    if let Some(program) = head
        .entry
        .as_ref()
        .and_then(|e| e.path.strip_prefix("bin/"))
        .filter(|_| head.kind == "process")
    {
        for triple in &platforms {
            let want = format!("bin/{triple}/{program}");
            let alt = format!("{want}.exe");
            if !files.iter().any(|(p, _)| *p == want || *p == alt) {
                return Err(ArchiveError::Manifest(format!(
                    "[entry] path is bin/{program}, but the {triple} build has no {want}"
                )));
            }
        }
    }
    Ok(ContentIndex {
        format: FORMAT,
        id: head.id,
        version: head.version,
        kind: head.kind,
        platforms,
        files: files
            .iter()
            .map(|(path, bytes)| FileEntry {
                path: path.clone(),
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                sha256: sha256_hex(bytes),
            })
            .collect(),
    })
}

/// The unsigned archive of the package in `dir`: its index first, then
/// its files.
///
/// # Errors
///
/// It cannot be read, or [`index_of`] refuses it.
pub fn pack(dir: &Path) -> Result<(ContentIndex, Vec<u8>), ArchiveError> {
    let files = read_tree(dir)?;
    let index = index_of(&files)?;
    let mut all = vec![(INDEX.to_owned(), index.to_bytes())];
    all.extend(files);
    Ok((index, write_entries(&all)?))
}

/// The archive's index and its bytes, unchecked: what a signer signs.
///
/// # Errors
///
/// It is not an archive, or has no index that reads.
pub fn read_index(archive: &[u8]) -> Result<(ContentIndex, Vec<u8>), ArchiveError> {
    let files = entries(archive)?;
    let bytes = files
        .into_iter()
        .find(|(p, _)| p == INDEX)
        .map(|(_, b)| b)
        .ok_or(ArchiveError::NoIndex)?;
    let index = parse_index(&bytes)?;
    Ok((index, bytes))
}

fn parse_index(bytes: &[u8]) -> Result<ContentIndex, ArchiveError> {
    let index: ContentIndex =
        serde_json::from_slice(bytes).map_err(|e| ArchiveError::BadIndex(e.to_string()))?;
    if index.format != FORMAT {
        return Err(ArchiveError::BadIndex(format!(
            "format {}, this app reads {FORMAT}",
            index.format
        )));
    }
    Ok(index)
}

/// `archive` with `signature` (a `.minisig` file's text) as its
/// [`SIGNATURE`], in place of any there was.
///
/// # Errors
///
/// It is not an archive.
pub fn with_signature(archive: &[u8], signature: &str) -> Result<Vec<u8>, ArchiveError> {
    let mut files: Vec<(String, Vec<u8>)> = entries(archive)?
        .into_iter()
        .filter(|(p, _)| p != SIGNATURE)
        .collect();
    let at = files
        .iter()
        .position(|(p, _)| p == INDEX)
        .map_or(0, |i| i.saturating_add(1));
    files.insert(at, (SIGNATURE.to_owned(), signature.as_bytes().to_vec()));
    write_entries(&files)
}

/// An archive whose signature and files were checked.
#[derive(Clone, Debug)]
pub struct Verified {
    pub index: ContentIndex,
    /// The name of the trusted key that signed it.
    pub signer: String,
    files: Vec<(String, Vec<u8>)>,
}

/// Checks `archive` against `trust`: signed by a trusted key, the index
/// the one signed, every file listed with its hash, every listed file
/// there.
///
/// # Errors
///
/// What [`ArchiveError`] says: unsigned, untrusted key, tampered index or
/// file, unlisted or missing file, not an archive.
pub fn open(archive: &[u8], trust: &Trust) -> Result<Verified, ArchiveError> {
    let mut files = entries(archive)?;
    let take = |files: &mut Vec<(String, Vec<u8>)>, name: &str| {
        files
            .iter()
            .position(|(p, _)| p == name)
            .map(|i| files.remove(i).1)
    };
    let index_bytes = take(&mut files, INDEX).ok_or(ArchiveError::NoIndex)?;
    let Some(sig_bytes) = take(&mut files, SIGNATURE) else {
        let index = parse_index(&index_bytes)?;
        return Err(ArchiveError::Unsigned {
            id: index.id,
            version: index.version,
        });
    };
    let sig_text = String::from_utf8(sig_bytes)
        .map_err(|e| ArchiveError::BadSignatureFile(e.to_string()))?;
    let signature = minisign_verify::Signature::decode(&sig_text)
        .map_err(|e| ArchiveError::BadSignatureFile(e.to_string()))?;
    let signed_by = sig_text
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with("untrusted comment:"))
        .and_then(key_id)
        .unwrap_or_default();
    let signer = trust
        .keys()
        .iter()
        .find(|k| key_id(&k.key).as_deref() == Some(signed_by.as_str()))
        .ok_or_else(|| ArchiveError::UntrustedKey {
            key_id: signed_by.clone(),
            trusted: trust
                .keys()
                .iter()
                .map(|k| format!("{} ({})", k.name, key_id(&k.key).unwrap_or_default()))
                .collect(),
        })?;
    let key = minisign_verify::PublicKey::from_base64(&signer.key)
        .map_err(|e| ArchiveError::Key(format!("{}: {e}", signer.name)))?;
    key.verify(&index_bytes, &signature, false)
        .map_err(|_| ArchiveError::IndexTampered {
            signer: signer.name.clone(),
        })?;
    let index = parse_index(&index_bytes)?;
    for (path, bytes) in &files {
        let listed = index
            .files
            .iter()
            .find(|f| f.path == *path)
            .ok_or_else(|| ArchiveError::Unlisted { path: path.clone() })?;
        let got = sha256_hex(bytes);
        if got != listed.sha256 || u64::try_from(bytes.len()).ok() != Some(listed.size) {
            return Err(ArchiveError::FileTampered {
                path: path.clone(),
                want: listed.sha256.clone(),
                got,
            });
        }
    }
    if let Some(gone) = index
        .files
        .iter()
        .find(|f| !files.iter().any(|(p, _)| *p == f.path))
    {
        return Err(ArchiveError::Missing {
            path: gone.path.clone(),
        });
    }
    Ok(Verified {
        index,
        signer: signer.name.clone(),
        files,
    })
}

/// The target triples this platform runs, best first (`None` for one
/// with no builds yet).
#[must_use]
pub fn host_targets() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => &["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"],
        ("linux", "aarch64") => &["aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"],
        ("macos", "aarch64") => &["aarch64-apple-darwin"],
        ("macos", "x86_64") => &["x86_64-apple-darwin"],
        ("windows", "x86_64") => &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"],
        ("windows", "aarch64") => &["aarch64-pc-windows-msvc"],
        _ => &[],
    }
}

/// This platform's name, as an error names it.
#[must_use]
pub fn host_name() -> String {
    host_targets().first().map_or_else(
        || format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        |t| (*t).to_owned(),
    )
}

/// Which of `platforms` this platform runs (`targets`, best first): `None`
/// when it has builds and none fits; `Some(None)` when it has none (it is
/// platform-neutral).
#[must_use]
pub fn pick_target<'a>(platforms: &'a [String], targets: &[&str]) -> Option<Option<&'a str>> {
    if platforms.is_empty() {
        return Some(None);
    }
    targets
        .iter()
        .find_map(|t| platforms.iter().find(|p| p == t))
        .map(|p| Some(p.as_str()))
}

/// Why `platforms` cannot run here, in words; `None` when they can.
#[must_use]
pub fn cannot_run(platforms: &[String]) -> Option<String> {
    pick_target(platforms, host_targets()).is_none().then(|| {
        ArchiveError::NoBuild {
            host: host_name(),
            has: platforms.to_vec(),
        }
        .to_string()
    })
}

impl Verified {
    /// The text of its file `path`.
    #[must_use]
    pub fn text(&self, path: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, b)| std::str::from_utf8(b).ok())
    }

    /// Its manifest's text.
    #[must_use]
    pub fn manifest(&self) -> Option<&str> {
        self.text(MANIFEST)
    }

    /// Why it cannot run here, when it cannot: builds, none for this
    /// platform.
    #[must_use]
    pub fn cannot_run(&self) -> Option<String> {
        cannot_run(&self.index.platforms)
    }

    /// Writes its files for this platform into `into`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::NoBuild`], or a file could not be written.
    pub fn extract(&self, into: &Path) -> Result<(), ArchiveError> {
        self.extract_for(into, host_targets())
    }

    /// Writes its files for the first of `targets` it has a build for:
    /// `bin/<triple>/x` lands at `bin/x` (executable), other platforms'
    /// builds are left out; a platform-neutral one is written as it is.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::NoBuild`], or a file could not be written.
    pub fn extract_for(&self, into: &Path, targets: &[&str]) -> Result<(), ArchiveError> {
        let chosen = pick_target(&self.index.platforms, targets).ok_or_else(|| {
            ArchiveError::NoBuild {
                host: targets
                    .first()
                    .map_or_else(host_name, |t| (*t).to_owned()),
                has: self.index.platforms.clone(),
            }
        })?;
        std::fs::create_dir_all(into).map_err(io(into))?;
        for (path, bytes) in &self.files {
            let (rel, program) = match platform_of(path) {
                Some(triple) if Some(triple) == chosen => {
                    let file = path
                        .strip_prefix(&format!("bin/{triple}/"))
                        .unwrap_or(path);
                    (format!("bin/{file}"), true)
                }
                Some(_) => continue,
                None => (path.clone(), false),
            };
            let dest = into.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            let mut out = std::fs::File::create(&dest).map_err(io(&dest))?;
            out.write_all(bytes).map_err(io(&dest))?;
            #[cfg(unix)]
            if program {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                    .map_err(io(&dest))?;
            }
            #[cfg(not(unix))]
            let _ = program;
        }
        Ok(())
    }
}
